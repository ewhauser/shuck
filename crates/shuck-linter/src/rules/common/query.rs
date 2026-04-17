use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BinaryCommand, BinaryOp, BourneParameterExpansion, BuiltinCommand, Command, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, HeredocBodyPartNode, ParameterExpansion,
    ParameterExpansionSyntax, Pattern, PatternPart, Redirect, Stmt, StmtSeq, Subscript, VarRef,
    Word, WordPart, WordPartNode, ZshExpansionTarget, ZshGlobSegment,
};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WalkContext {
    pub(crate) loop_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConditionKind {
    If,
    Elif,
    While,
    Until,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CommandTraversalContext {
    pub(crate) walk: WalkContext,
    pub(crate) nested_word_command: bool,
    pub(crate) condition_kind: Option<ConditionKind>,
    pub(crate) in_if_condition: bool,
    pub(crate) in_elif_condition: bool,
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct TraversedCommandVisit<'a> {
    pub(crate) visit: CommandVisit<'a>,
    pub(crate) context: CommandTraversalContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionKind {
    Command,
    ProcessInput,
    ProcessOutput,
}

// Structural traversal helpers stay crate-visible so `facts.rs` owns repeated
// AST walks. Rule implementations should consume `Checker::facts()` instead of
// calling these walkers directly, while `facts.rs` can opt into the streaming
// walker when it needs traversal context without a second whole-file pass.
pub(crate) fn walk_commands<'a, F>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    collect_command_visits(
        commands,
        options,
        CommandTraversalContext::default(),
        visitor,
    );
}

pub(crate) fn iter_commands_with_context<'a>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
) -> impl Iterator<Item = TraversedCommandVisit<'a>> {
    let mut visits = Vec::new();
    walk_commands(commands, options, &mut |visit, context| {
        visits.push(TraversedCommandVisit { visit, context });
    });
    visits.into_iter()
}

pub(crate) fn iter_commands<'a>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
) -> impl Iterator<Item = CommandVisit<'a>> {
    iter_commands_with_context(commands, options).map(|visit| visit.visit)
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

fn collect_subscript_words(subscript: Option<&Subscript>, _source: &str, words: &mut Vec<Word>) {
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

    if let Some(word) = subscript.word_ast() {
        words.push(word.clone());
        return;
    }

    debug_assert!(
        subscript.word_ast().is_some(),
        "ordinary subscripts should always carry a word AST"
    );
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

fn collect_command_visits<'a, F>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for stmt in commands.iter() {
        collect_command_visit(stmt, options, context, visitor);
    }
}

fn collect_command_visit<'a, F>(
    stmt: &'a Stmt,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    visitor(
        CommandVisit {
            stmt,
            command: &stmt.command,
            redirects: &stmt.redirects,
        },
        context,
    );

    match &stmt.command {
        Command::Simple(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            collect_word_visits(&command.name, options, context, visitor);
            collect_word_slice_visits(&command.args, options, context, visitor);
        }
        Command::Builtin(command) => collect_builtin_visits(command, options, context, visitor),
        Command::Decl(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word_visits(word, options, context, visitor);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment_visit(assignment, options, context, visitor);
                    }
                }
            }
        }
        Command::Binary(command) => {
            collect_command_visit(&command.left, options, context, visitor);
            collect_command_visit(&command.right, options, context, visitor);
        }
        Command::Compound(command) => {
            collect_compound_visits(command, options, context, visitor);
        }
        Command::Function(FunctionDef { header, body, .. }) => {
            for entry in &header.entries {
                collect_word_visits(&entry.word, options, context, visitor);
            }
            collect_command_visit(body, options, context, visitor);
        }
        Command::AnonymousFunction(function) => {
            collect_word_slice_visits(&function.args, options, context, visitor);
            collect_command_visit(&function.body, options, context, visitor);
        }
    }

    collect_redirect_visits(&stmt.redirects, options, context, visitor);
}

fn condition_context(
    context: CommandTraversalContext,
    kind: ConditionKind,
) -> CommandTraversalContext {
    let (in_if_condition, in_elif_condition) = match kind {
        ConditionKind::If => (true, context.in_elif_condition),
        ConditionKind::Elif => (true, true),
        ConditionKind::While | ConditionKind::Until => {
            (context.in_if_condition, context.in_elif_condition)
        }
    };

    CommandTraversalContext {
        condition_kind: Some(kind),
        in_if_condition,
        in_elif_condition,
        ..context
    }
}

fn loop_context(context: CommandTraversalContext) -> CommandTraversalContext {
    CommandTraversalContext {
        walk: WalkContext {
            loop_depth: context.walk.loop_depth + 1,
        },
        ..context
    }
}

fn nested_word_context(context: CommandTraversalContext) -> CommandTraversalContext {
    CommandTraversalContext {
        nested_word_command: true,
        ..context
    }
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

fn collect_builtin_visits<'a, F>(
    command: &'a BuiltinCommand,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
        BuiltinCommand::Return(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
    }
}

fn collect_compound_visits<'a, F>(
    command: &'a CompoundCommand,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match command {
        CompoundCommand::If(command) => {
            collect_command_visits(
                &command.condition,
                options,
                condition_context(context, ConditionKind::If),
                visitor,
            );
            collect_command_visits(&command.then_branch, options, context, visitor);
            for (condition, body) in &command.elif_branches {
                collect_command_visits(
                    condition,
                    options,
                    condition_context(context, ConditionKind::Elif),
                    visitor,
                );
                collect_command_visits(body, options, context, visitor);
            }
            if let Some(body) = &command.else_branch {
                collect_command_visits(body, options, context, visitor);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_word_slice_visits(words, options, context, visitor);
            }
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::Repeat(command) => {
            collect_word_visits(&command.count, options, context, visitor);
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::Foreach(command) => {
            collect_word_slice_visits(&command.words, options, context, visitor);
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::ArithmeticFor(command) => {
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::While(command) => {
            let loop_context = loop_context(context);
            collect_command_visits(
                &command.condition,
                options,
                condition_context(loop_context, ConditionKind::While),
                visitor,
            );
            collect_command_visits(&command.body, options, loop_context, visitor);
        }
        CompoundCommand::Until(command) => {
            let loop_context = loop_context(context);
            collect_command_visits(
                &command.condition,
                options,
                condition_context(loop_context, ConditionKind::Until),
                visitor,
            );
            collect_command_visits(&command.body, options, loop_context, visitor);
        }
        CompoundCommand::Case(command) => {
            collect_word_visits(&command.word, options, context, visitor);
            for case in &command.cases {
                collect_pattern_slice_visits(&case.patterns, options, context, visitor);
                collect_command_visits(&case.body, options, context, visitor);
            }
        }
        CompoundCommand::Select(command) => {
            collect_word_slice_visits(&command.words, options, context, visitor);
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_command_visits(commands, options, context, visitor);
        }
        CompoundCommand::Always(command) => {
            collect_command_visits(&command.body, options, context, visitor);
            collect_command_visits(&command.always_body, options, context, visitor);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command_visit(command, options, context, visitor);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_visits(&command.expression, options, context, visitor);
        }
        CompoundCommand::Coproc(command) => {
            collect_command_visit(&command.body, options, context, visitor);
        }
    }
}

fn collect_assignment_visits<'a, F>(
    assignments: &'a [Assignment],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for assignment in assignments {
        collect_assignment_visit(assignment, options, context, visitor);
    }
}

fn collect_assignment_visit<'a, F>(
    assignment: &'a Assignment,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word_visits(word, options, context, visitor),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_word_visits(word, options, context, visitor);
                    }
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        collect_word_visits(value, options, context, visitor);
                    }
                }
            }
        }
    }
}

fn collect_word_slice_visits<'a, F>(
    words: &'a [Word],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for word in words {
        collect_word_visits(word, options, context, visitor);
    }
}

fn collect_pattern_slice_visits<'a, F>(
    patterns: &'a [Pattern],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for pattern in patterns {
        collect_pattern_visits(pattern, options, context, visitor);
    }
}

fn collect_word_visits<'a, F>(
    word: &'a Word,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    if !options.descend_nested_word_commands {
        return;
    }

    collect_word_part_visits(&word.parts, options, nested_word_context(context), visitor);
}

fn collect_word_part_visits<'a, F>(
    parts: &'a [WordPartNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for part in parts {
        match &part.kind {
            WordPart::ZshQualifiedGlob(glob) => {
                for pattern in zsh_glob_patterns(glob) {
                    collect_pattern_visits(pattern, options, context, visitor);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_part_visits(parts, options, context, visitor);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression_ast), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                }
            }
            WordPart::Parameter(parameter) => {
                collect_parameter_expansion_visits(parameter, options, context, visitor);
            }
            WordPart::ParameterExpansion {
                reference,
                operand_word_ast,
                ..
            }
            | WordPart::IndirectExpansion {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
            }
            WordPart::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(expression) = offset_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else {
                    collect_word_visits(offset_word_ast, options, context, visitor);
                }
                if let Some(expression) = length_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else if let Some(word) = length_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                collect_command_visits(body, options, context, visitor);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. }
            => {}
        }
    }
}

fn collect_var_ref_word_visits<'a, F>(
    reference: &'a VarRef,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    let mut words = Vec::new();
    collect_var_ref_subscript_words(reference, &mut words);
    for word in words {
        collect_word_visits(word, options, context, visitor);
    }
}

fn collect_parameter_expansion_visits<'a, F>(
    parameter: &'a ParameterExpansion,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansion::Operation {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(expression) = offset_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else {
                    collect_word_visits(offset_word_ast, options, context, visitor);
                }
                if let Some(expression) = length_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else if let Some(word) = length_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_zsh_target_visits(&syntax.target, options, context, visitor);

            for modifier in &syntax.modifiers {
                if let Some(word) = modifier.argument_word_ast() {
                    collect_word_visits(word, options, context, visitor);
                }
            }

            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::PatternOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Unknown { .. } => {
                        if let Some(word) = operation.operand_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::ReplacementOperation { .. } => {
                        if let Some(word) = operation.pattern_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                        if let Some(word) = operation.replacement_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::Slice { .. } => {
                        if let Some(word) = operation.offset_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                        if let Some(word) = operation.length_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                    }
                }
            }
        }
    }
}

fn collect_zsh_target_visits<'a, F>(
    target: &'a ZshExpansionTarget,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match target {
        ZshExpansionTarget::Reference(reference) => {
            collect_var_ref_word_visits(reference, options, context, visitor);
        }
        ZshExpansionTarget::Nested(parameter) => {
            collect_parameter_expansion_visits(parameter, options, context, visitor);
        }
        ZshExpansionTarget::Word(word) => {
            collect_word_visits(word, options, context, visitor);
        }
        ZshExpansionTarget::Empty => {}
    }
}

fn collect_pattern_visits<'a, F>(
    pattern: &'a Pattern,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                collect_pattern_slice_visits(patterns, options, context, visitor);
            }
            PatternPart::Word(word) => collect_word_visits(word, options, context, visitor),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_redirect_visits<'a, F>(
    redirects: &'a [Redirect],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for redirect in redirects {
        if let Some(word) = redirect.word_target() {
            collect_word_visits(word, options, context, visitor);
        } else if let Some(heredoc) = redirect.heredoc()
            && heredoc.delimiter.expands_body
        {
            collect_heredoc_body_part_visits(&heredoc.body.parts, options, context, visitor);
        }
    }
}

fn collect_conditional_visits<'a, F>(
    expression: &'a ConditionalExpr,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_visits(&expr.left, options, context, visitor);
            collect_conditional_visits(&expr.right, options, context, visitor);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visitor)
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visitor);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_word_visits(word, options, context, visitor)
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_pattern_visits(pattern, options, context, visitor)
        }
        ConditionalExpr::VarRef(reference) => {
            let mut subscript_words = Vec::new();
            collect_var_ref_subscript_words(reference, &mut subscript_words);
            for word in subscript_words {
                collect_word_visits(word, options, context, visitor);
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

fn collect_heredoc_body_part_visits<'a, F>(
    parts: &'a [HeredocBodyPartNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    if !options.descend_nested_word_commands {
        return;
    }

    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression_ast), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                collect_command_visits(body, options, context, visitor);
            }
            shuck_ast::HeredocBodyPart::Literal(_)
            | shuck_ast::HeredocBodyPart::Variable(_)
            | shuck_ast::HeredocBodyPart::Parameter(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{
        BourneParameterExpansion, Command, ParameterExpansionSyntax, StmtSeq, VarRef, Word,
        WordPart,
    };
    use shuck_parser::parser::Parser;

    use super::{
        CommandWalkOptions, ConditionKind, iter_commands, pipeline_segments,
        visit_var_ref_subscript_words_with_source, walk_commands,
    };

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

    fn parameter_access_reference(word: &Word) -> &VarRef {
        let [part] = word.parts.as_slice() else {
            panic!("expected single parameter part");
        };
        let WordPart::Parameter(parameter) = &part.kind else {
            panic!("expected parameter part, got {:?}", part.kind);
        };
        let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) =
            &parameter.syntax
        else {
            panic!("expected access expansion, got {:?}", parameter.syntax);
        };
        reference
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
    fn walk_commands_tracks_nested_word_and_condition_context() {
        let source = "\
if foo \"$(bar)\"; then
  :
elif while baz \"$(qux)\"; do :; done; then
  :
fi
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_text(&command.name, source) else {
                    return;
                };
                if name == ":" {
                    return;
                }
                visits.push((
                    name,
                    context.nested_word_command,
                    context.condition_kind,
                    context.in_if_condition,
                    context.in_elif_condition,
                ));
            },
        );

        assert_eq!(
            visits,
            vec![
                (
                    "foo".to_owned(),
                    false,
                    Some(ConditionKind::If),
                    true,
                    false
                ),
                ("bar".to_owned(), true, Some(ConditionKind::If), true, false),
                (
                    "baz".to_owned(),
                    false,
                    Some(ConditionKind::While),
                    true,
                    true
                ),
                (
                    "qux".to_owned(),
                    true,
                    Some(ConditionKind::While),
                    true,
                    true
                ),
            ]
        );
    }

    #[test]
    fn walk_commands_prefers_nested_if_and_elif_condition_kinds_inside_loops() {
        let source = "\
while if foo; then bar; elif baz; then qux; fi; do
  :
done
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_text(&command.name, source) else {
                    return;
                };
                if name == ":" {
                    return;
                }
                visits.push((
                    name,
                    context.condition_kind,
                    context.in_if_condition,
                    context.in_elif_condition,
                ));
            },
        );

        assert_eq!(
            visits,
            vec![
                ("foo".to_owned(), Some(ConditionKind::If), true, false),
                ("bar".to_owned(), Some(ConditionKind::While), false, false),
                ("baz".to_owned(), Some(ConditionKind::Elif), true, true),
                ("qux".to_owned(), Some(ConditionKind::While), false, false),
            ]
        );
    }

    #[test]
    fn walk_commands_descends_into_parameter_expansion_operands() {
        let source = "\
printf '%s\\n' ${value:-$(expr $(nproc) + 1)}
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_text(&command.name, source) else {
                    return;
                };
                visits.push((name, context.nested_word_command));
            },
        );

        assert_eq!(
            visits,
            vec![
                ("printf".to_owned(), false),
                ("expr".to_owned(), true),
                ("nproc".to_owned(), true),
            ]
        );
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

    #[test]
    fn visit_var_ref_subscript_words_uses_parser_backed_subscript_words() {
        let source = "echo ${map[$key]} ${map[@]}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands.stmts[0].command else {
            panic!("expected simple command");
        };

        let ordinary = parameter_access_reference(&command.args[0]);
        let selector = parameter_access_reference(&command.args[1]);

        let mut ordinary_words = Vec::new();
        visit_var_ref_subscript_words_with_source(ordinary, source, &mut |word| {
            ordinary_words.push(word.render_syntax(source));
        });

        let mut selector_words = Vec::new();
        visit_var_ref_subscript_words_with_source(selector, source, &mut |word| {
            selector_words.push(word.render_syntax(source));
        });

        assert_eq!(ordinary_words, vec!["$key"]);
        assert!(selector_words.is_empty());
    }
}
