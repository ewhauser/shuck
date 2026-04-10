use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BinaryCommand, BinaryOp, BuiltinCommand, Command, CompoundCommand, ConditionalExpr,
    DeclOperand, FunctionDef, Pattern, PatternPart, Redirect, Stmt, StmtSeq, Subscript, VarRef,
    Word, WordPart, WordPartNode, ZshGlobSegment,
};
use shuck_parser::parser::Parser;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WalkContext {
    pub(crate) loop_depth: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CommandWalkOptions {
    pub(crate) descend_nested_word_commands: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommandVisit<'a> {
    pub(crate) stmt: &'a Stmt,
    pub(crate) command: &'a Command,
    pub(crate) redirects: &'a [Redirect],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionKind {
    Command,
    ProcessInput,
    ProcessOutput,
}

// Structural traversal helpers stay crate-visible so `facts.rs` owns repeated
// AST walks. Rule implementations should consume `Checker::facts()` instead of
// calling these walkers directly.
pub(crate) fn iter_commands<'a>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
) -> impl Iterator<Item = CommandVisit<'a>> {
    let mut visits = Vec::new();
    collect_command_visits(commands, options, WalkContext::default(), &mut visits);
    visits.into_iter()
}

pub(crate) fn pipeline_segments(command: &Command) -> Option<Vec<&Stmt>> {
    let Command::Binary(command) = command else {
        return None;
    };
    if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
        return None;
    }

    let mut segments = Vec::new();
    collect_pipeline_segments(command, &mut segments);
    Some(segments)
}

fn zsh_glob_patterns(glob: &shuck_ast::ZshQualifiedGlob) -> impl Iterator<Item = &Pattern> + '_ {
    glob.segments.iter().filter_map(|segment| match segment {
        ZshGlobSegment::Pattern(pattern) => Some(pattern),
        ZshGlobSegment::InlineControl(_) => None,
    })
}

pub(crate) fn command_assignments(command: &Command) -> &[Assignment] {
    match command {
        Command::Simple(command) => &command.assignments,
        Command::Builtin(command) => builtin_assignments(command),
        Command::Decl(command) => &command.assignments,
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => &[],
    }
}

pub(crate) fn declaration_operands(command: &Command) -> &[DeclOperand] {
    match command {
        Command::Decl(command) => &command.operands,
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => &[],
    }
}

pub(crate) fn visit_arithmetic_words(
    expression: &ArithmeticExprNode,
    visitor: &mut impl FnMut(&Word),
) {
    visit_arithmetic_words_in_expr(expression, visitor);
}

pub(crate) fn visit_var_ref_subscript_words(reference: &VarRef, visitor: &mut impl FnMut(&Word)) {
    let mut words = Vec::new();
    collect_var_ref_subscript_words(reference, &mut words);
    for word in words {
        visitor(word);
    }
}

pub(crate) fn visit_var_ref_subscript_words_with_source(
    reference: &VarRef,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    visit_subscript_words(reference.subscript.as_ref(), source, visitor);
}

pub(crate) fn visit_subscript_words(
    subscript: Option<&Subscript>,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    let mut words = Vec::new();
    collect_subscript_words(subscript, source, &mut words);
    for word in words {
        visitor(&word);
    }
}

fn visit_arithmetic_words_in_expr(
    expression: &ArithmeticExprNode,
    visitor: &mut impl FnMut(&Word),
) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => visit_arithmetic_words_in_expr(index, visitor),
        ArithmeticExpr::ShellWord(word) => visitor(word),
        ArithmeticExpr::Parenthesized { expression } => {
            visit_arithmetic_words_in_expr(expression, visitor)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visit_arithmetic_words_in_expr(expr, visitor)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visit_arithmetic_words_in_expr(left, visitor);
            visit_arithmetic_words_in_expr(right, visitor);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arithmetic_words_in_expr(condition, visitor);
            visit_arithmetic_words_in_expr(then_expr, visitor);
            visit_arithmetic_words_in_expr(else_expr, visitor);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue_words(target, visitor);
            visit_arithmetic_words_in_expr(value, visitor);
        }
    }
}

fn collect_var_ref_subscript_words<'a>(reference: &'a VarRef, words: &mut Vec<&'a Word>) {
    collect_optional_arithmetic_words(
        reference
            .subscript
            .as_ref()
            .and_then(|subscript| subscript.arithmetic_ast.as_ref()),
        words,
    );
}

fn collect_subscript_words(subscript: Option<&Subscript>, source: &str, words: &mut Vec<Word>) {
    let Some(subscript) = subscript else {
        return;
    };
    if subscript.selector().is_some() {
        return;
    }
    if let Some(expression) = subscript.arithmetic_ast.as_ref() {
        let mut arithmetic_words = Vec::new();
        collect_arithmetic_words(expression, &mut arithmetic_words);
        words.extend(arithmetic_words.into_iter().cloned());
        return;
    }

    let text = subscript.syntax_source_text();
    words.push(Parser::parse_word_fragment(
        source,
        text.slice(source),
        text.span(),
    ));
}

fn collect_optional_arithmetic_words<'a>(
    expression: Option<&'a ArithmeticExprNode>,
    words: &mut Vec<&'a Word>,
) {
    if let Some(expression) = expression {
        collect_arithmetic_words(expression, words);
    }
}

fn collect_arithmetic_lvalue_words<'a>(target: &'a ArithmeticLvalue, words: &mut Vec<&'a Word>) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => collect_arithmetic_words(index, words),
    }
}

fn collect_arithmetic_words<'a>(expression: &'a ArithmeticExprNode, words: &mut Vec<&'a Word>) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => collect_arithmetic_words(index, words),
        ArithmeticExpr::ShellWord(word) => words.push(word),
        ArithmeticExpr::Parenthesized { expression } => collect_arithmetic_words(expression, words),
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            collect_arithmetic_words(expr, words)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_words(left, words);
            collect_arithmetic_words(right, words);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_words(condition, words);
            collect_arithmetic_words(then_expr, words);
            collect_arithmetic_words(else_expr, words);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_arithmetic_lvalue_words(target, words);
            collect_arithmetic_words(value, words);
        }
    }
}

fn visit_arithmetic_lvalue_words(target: &ArithmeticLvalue, visitor: &mut impl FnMut(&Word)) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => visit_arithmetic_words_in_expr(index, visitor),
    }
}

fn collect_command_visits<'a>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for stmt in commands.iter() {
        collect_command_visit(stmt, options, context, visits);
    }
}

fn collect_command_visit<'a>(
    stmt: &'a Stmt,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    visits.push(CommandVisit {
        stmt,
        command: &stmt.command,
        redirects: &stmt.redirects,
    });

    match &stmt.command {
        Command::Simple(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            collect_word_visits(&command.name, options, context, visits);
            collect_word_slice_visits(&command.args, options, context, visits);
        }
        Command::Builtin(command) => collect_builtin_visits(command, options, context, visits),
        Command::Decl(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word_visits(word, options, context, visits);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment_visit(assignment, options, context, visits);
                    }
                }
            }
        }
        Command::Binary(command) => {
            collect_command_visit(&command.left, options, context, visits);
            collect_command_visit(&command.right, options, context, visits);
        }
        Command::Compound(command) => {
            collect_compound_visits(command, options, context, visits);
        }
        Command::Function(FunctionDef { header, body, .. }) => {
            for entry in &header.entries {
                collect_word_visits(&entry.word, options, context, visits);
            }
            collect_command_visit(body, options, context, visits);
        }
        Command::AnonymousFunction(function) => {
            collect_word_slice_visits(&function.args, options, context, visits);
            collect_command_visit(&function.body, options, context, visits);
        }
    }

    collect_redirect_visits(&stmt.redirects, options, context, visits);
}

fn collect_pipeline_segments<'a>(command: &'a BinaryCommand, segments: &mut Vec<&'a Stmt>) {
    match &command.left.command {
        Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(left, segments);
        }
        _ => segments.push(&command.left),
    }

    match &command.right.command {
        Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(right, segments);
        }
        _ => segments.push(&command.right),
    }
}

fn collect_builtin_visits<'a>(
    command: &'a BuiltinCommand,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
        }
        BuiltinCommand::Return(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
        }
    }
}

fn collect_compound_visits<'a>(
    command: &'a CompoundCommand,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_command_visits(&command.condition, options, context, visits);
            collect_command_visits(&command.then_branch, options, context, visits);
            for (condition, body) in &command.elif_branches {
                collect_command_visits(condition, options, context, visits);
                collect_command_visits(body, options, context, visits);
            }
            if let Some(body) = &command.else_branch {
                collect_command_visits(body, options, context, visits);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_word_slice_visits(words, options, context, visits);
            }
            collect_command_visits(
                &command.body,
                options,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
                visits,
            );
        }
        CompoundCommand::Repeat(command) => {
            collect_word_visits(&command.count, options, context, visits);
            collect_command_visits(
                &command.body,
                options,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
                visits,
            );
        }
        CompoundCommand::Foreach(command) => {
            collect_word_slice_visits(&command.words, options, context, visits);
            collect_command_visits(
                &command.body,
                options,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
                visits,
            );
        }
        CompoundCommand::ArithmeticFor(command) => collect_command_visits(
            &command.body,
            options,
            WalkContext {
                loop_depth: context.loop_depth + 1,
            },
            visits,
        ),
        CompoundCommand::While(command) => {
            let loop_context = WalkContext {
                loop_depth: context.loop_depth + 1,
            };
            collect_command_visits(&command.condition, options, loop_context, visits);
            collect_command_visits(&command.body, options, loop_context, visits);
        }
        CompoundCommand::Until(command) => {
            let loop_context = WalkContext {
                loop_depth: context.loop_depth + 1,
            };
            collect_command_visits(&command.condition, options, loop_context, visits);
            collect_command_visits(&command.body, options, loop_context, visits);
        }
        CompoundCommand::Case(command) => {
            collect_word_visits(&command.word, options, context, visits);
            for case in &command.cases {
                collect_pattern_slice_visits(&case.patterns, options, context, visits);
                collect_command_visits(&case.body, options, context, visits);
            }
        }
        CompoundCommand::Select(command) => {
            collect_word_slice_visits(&command.words, options, context, visits);
            collect_command_visits(
                &command.body,
                options,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
                visits,
            );
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_command_visits(commands, options, context, visits);
        }
        CompoundCommand::Always(command) => {
            collect_command_visits(&command.body, options, context, visits);
            collect_command_visits(&command.always_body, options, context, visits);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command_visit(command, options, context, visits);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_visits(&command.expression, options, context, visits);
        }
        CompoundCommand::Coproc(command) => {
            collect_command_visit(&command.body, options, context, visits);
        }
    }
}

fn collect_assignment_visits<'a>(
    assignments: &'a [Assignment],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for assignment in assignments {
        collect_assignment_visit(assignment, options, context, visits);
    }
}

fn collect_assignment_visit<'a>(
    assignment: &'a Assignment,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word_visits(word, options, context, visits),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_word_visits(word, options, context, visits);
                    }
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        collect_word_visits(value, options, context, visits);
                    }
                }
            }
        }
    }
}

fn collect_word_slice_visits<'a>(
    words: &'a [Word],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for word in words {
        collect_word_visits(word, options, context, visits);
    }
}

fn collect_pattern_slice_visits<'a>(
    patterns: &'a [Pattern],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for pattern in patterns {
        collect_pattern_visits(pattern, options, context, visits);
    }
}

fn collect_word_visits<'a>(
    word: &'a Word,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    if !options.descend_nested_word_commands {
        return;
    }

    collect_word_part_visits(&word.parts, options, context, visits);
}

fn collect_word_part_visits<'a>(
    parts: &'a [WordPartNode],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for part in parts {
        match &part.kind {
            WordPart::ZshQualifiedGlob(glob) => {
                for pattern in zsh_glob_patterns(glob) {
                    collect_pattern_visits(pattern, options, context, visits);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_part_visits(parts, options, context, visits);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression_ast), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visits);
                    }
                }
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                collect_command_visits(body, options, context, visits);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. } => {}
        }
    }
}

fn collect_pattern_visits<'a>(
    pattern: &'a Pattern,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                collect_pattern_slice_visits(patterns, options, context, visits);
            }
            PatternPart::Word(word) => collect_word_visits(word, options, context, visits),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_redirect_visits<'a>(
    redirects: &'a [Redirect],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for redirect in redirects {
        collect_word_visits(redirect_walk_word(redirect), options, context, visits);
    }
}

fn collect_conditional_visits<'a>(
    expression: &'a ConditionalExpr,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_visits(&expr.left, options, context, visits);
            collect_conditional_visits(&expr.right, options, context, visits);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visits)
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visits);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_word_visits(word, options, context, visits)
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_pattern_visits(pattern, options, context, visits)
        }
        ConditionalExpr::VarRef(reference) => {
            let mut subscript_words = Vec::new();
            collect_var_ref_subscript_words(reference, &mut subscript_words);
            for word in subscript_words {
                collect_word_visits(word, options, context, visits);
            }
        }
    }
}

fn builtin_assignments(command: &BuiltinCommand) -> &[Assignment] {
    match command {
        BuiltinCommand::Break(command) => &command.assignments,
        BuiltinCommand::Continue(command) => &command.assignments,
        BuiltinCommand::Return(command) => &command.assignments,
        BuiltinCommand::Exit(command) => &command.assignments,
    }
}

fn redirect_walk_word(redirect: &Redirect) -> &Word {
    match redirect.word_target() {
        Some(word) => word,
        None => &redirect.heredoc().expect("expected heredoc redirect").body,
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, StmtSeq, Word, WordPart};
    use shuck_parser::parser::Parser;

    use super::{CommandWalkOptions, iter_commands, pipeline_segments};

    fn parse_commands(source: &str) -> StmtSeq {
        let output = Parser::new(source).parse().unwrap();
        output.file.body
    }

    fn static_word_text(word: &Word, source: &str) -> Option<String> {
        let mut result = String::new();
        for (part, span) in word.parts_with_spans() {
            match part {
                WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
                _ => return None,
            }
        }
        Some(result)
    }

    #[test]
    fn iter_commands_can_ignore_or_follow_nested_word_commands() {
        let source = "echo \"$(printf x)\"\n";
        let commands = parse_commands(source);

        let structural = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: false,
            },
        )
        .filter_map(|visit| {
            let Command::Simple(command) = visit.command else {
                return None;
            };

            static_word_text(&command.name, source)
        })
        .collect::<Vec<_>>();

        let nested = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        )
        .filter_map(|visit| {
            let Command::Simple(command) = visit.command else {
                return None;
            };

            static_word_text(&command.name, source)
        })
        .collect::<Vec<_>>();

        assert_eq!(structural, vec!["echo"]);
        assert_eq!(nested, vec!["echo", "printf"]);
    }

    #[test]
    fn pipeline_segments_flattens_pipe_chains() {
        let source = "printf '%s\\n' a | command kill 0 | tee out.txt\n";
        let commands = parse_commands(source);
        let Command::Binary(command) = &commands[0].command else {
            panic!("expected binary command");
        };

        let segments = pipeline_segments(&Command::Binary(command.clone()))
            .expect("expected pipeline segments")
            .into_iter()
            .map(|stmt| match &stmt.command {
                Command::Simple(command) => static_word_text(&command.name, source).unwrap(),
                _ => "<non-simple>".to_owned(),
            })
            .collect::<Vec<_>>();

        assert_eq!(segments, vec!["printf", "command", "tee"]);
    }
}
