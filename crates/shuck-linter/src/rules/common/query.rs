use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BinaryCommand, BinaryOp, BuiltinCommand, Command, CompoundCommand, ConditionalExpr,
    DeclOperand, FunctionDef, ParameterOp, Pattern, PatternPart, Redirect, Span, Stmt, StmtSeq,
    Subscript, VarRef, Word, WordPart, WordPartNode,
};
use shuck_parser::parser::Parser;

use super::expansion::ExpansionContext;
use super::word::static_word_text;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkContext {
    pub loop_depth: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandWalkOptions {
    pub descend_nested_word_commands: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandVisit<'a> {
    pub stmt: &'a Stmt,
    pub command: &'a Command,
    pub redirects: &'a [Redirect],
    pub context: WalkContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionKind {
    Command,
    ProcessInput,
    ProcessOutput,
}

#[derive(Debug, Clone)]
pub struct NestedCommandSubstitution {
    pub commands: StmtSeq,
    pub span: Span,
    pub kind: CommandSubstitutionKind,
}

pub fn walk_commands(
    commands: &StmtSeq,
    options: CommandWalkOptions,
    visitor: &mut impl FnMut(CommandVisit<'_>),
) {
    CommandWalker { options, visitor }.walk_commands(commands, WalkContext::default());
}

pub fn iter_commands<'a>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
) -> impl Iterator<Item = CommandVisit<'a>> {
    let mut visits = Vec::new();
    collect_command_visits(commands, options, WalkContext::default(), &mut visits);
    visits.into_iter()
}

pub fn pipeline_segments(command: &Command) -> Option<Vec<&Stmt>> {
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

pub fn walk_words(
    commands: &StmtSeq,
    options: CommandWalkOptions,
    visitor: &mut impl FnMut(&Word),
) {
    WordWalker { options, visitor }.walk_commands(commands);
}

pub fn command_assignments(command: &Command) -> &[Assignment] {
    match command {
        Command::Simple(command) => &command.assignments,
        Command::Builtin(command) => builtin_assignments(command),
        Command::Decl(command) => &command.assignments,
        Command::Binary(_) | Command::Compound(_) | Command::Function(_) => &[],
    }
}

pub fn declaration_operands(command: &Command) -> &[DeclOperand] {
    match command {
        Command::Decl(command) => &command.operands,
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_) => &[],
    }
}

pub fn command_redirects(visit: CommandVisit<'_>) -> &[Redirect] {
    visit.redirects
}

pub fn iter_command_words(visit: CommandVisit<'_>, source: &str) -> impl Iterator<Item = Word> {
    let mut words = Vec::new();
    collect_command_words(visit.command, visit.redirects, source, &mut words);
    words.into_iter()
}

pub fn iter_word_command_substitutions(
    word: &Word,
) -> impl Iterator<Item = NestedCommandSubstitution> + '_ {
    let mut substitutions = Vec::new();
    collect_word_command_substitutions(&word.parts, &mut substitutions);
    substitutions.into_iter()
}

pub fn iter_command_substitutions(
    visit: CommandVisit<'_>,
    source: &str,
) -> impl Iterator<Item = NestedCommandSubstitution> {
    let mut substitutions = Vec::new();
    for word in iter_command_words(visit, source) {
        substitutions.extend(iter_word_command_substitutions(&word));
    }
    substitutions.into_iter()
}

pub fn visit_arithmetic_words(expression: &ArithmeticExprNode, visitor: &mut impl FnMut(&Word)) {
    let mut words = Vec::new();
    collect_arithmetic_words(expression, &mut words);
    for word in words {
        visitor(word);
    }
}

pub fn visit_var_ref_subscript_words(reference: &VarRef, visitor: &mut impl FnMut(&Word)) {
    let mut words = Vec::new();
    collect_var_ref_subscript_words(reference, &mut words);
    for word in words {
        visitor(word);
    }
}

pub fn visit_var_ref_subscript_words_with_source(
    reference: &VarRef,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    visit_subscript_words(reference.subscript.as_ref(), source, visitor);
}

pub fn visit_subscript_words(
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

fn visit_optional_arithmetic_words(
    expression: Option<&ArithmeticExprNode>,
    visitor: &mut impl FnMut(&Word),
) {
    let mut words = Vec::new();
    collect_optional_arithmetic_words(expression, &mut words);
    for word in words {
        visitor(word);
    }
}

pub fn visit_command_words(visit: CommandVisit<'_>, source: &str, visitor: &mut impl FnMut(&Word)) {
    for word in iter_command_words(visit, source) {
        visitor(&word);
    }
}

pub fn visit_argument_words(command: &Command, visitor: &mut impl FnMut(&Word)) {
    match command {
        Command::Simple(command) => {
            for word in &command.args {
                visitor(word);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
        },
        _ => {}
    }
}

pub fn iter_expansion_words<'a>(
    visit: CommandVisit<'a>,
    source: &str,
) -> impl Iterator<Item = (Word, ExpansionContext)> {
    let mut words = Vec::new();
    collect_expansion_words(visit, source, &mut words);
    words.into_iter()
}

pub fn visit_expansion_words(
    visit: CommandVisit<'_>,
    source: &str,
    visitor: &mut impl FnMut(&Word, ExpansionContext),
) {
    for (word, context) in iter_expansion_words(visit, source) {
        visitor(&word, context);
    }
}

pub fn visit_command_redirects(visit: CommandVisit<'_>, visitor: &mut impl FnMut(&Redirect)) {
    for redirect in command_redirects(visit) {
        visitor(redirect);
    }
}

fn collect_expansion_words<'a>(
    visit: CommandVisit<'a>,
    source: &str,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    let command = visit.command;
    collect_command_name_context_words(command, source, words);

    collect_argument_context_words(command, source, words);

    collect_expansion_assignment_value_words(command, source, words);

    match command {
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(items) = &command.words {
                    for word in items {
                        words.push((word.clone(), ExpansionContext::ForList));
                    }
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    words.push((word.clone(), ExpansionContext::SelectList));
                }
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    for pattern in &case.patterns {
                        collect_pattern_context_words(
                            pattern,
                            ExpansionContext::CasePattern,
                            words,
                        );
                    }
                }
            }
            CompoundCommand::Conditional(command) => {
                collect_conditional_expansion_words(&command.expression, source, words);
            }
            CompoundCommand::If(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Time(_) => {}
        },
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Function(_) => {}
    }

    for redirect in command_redirects(visit) {
        let Some(context) = ExpansionContext::from_redirect_kind(redirect.kind) else {
            continue;
        };
        let word = redirect
            .word_target()
            .expect("expected non-heredoc redirect target");
        words.push((word.clone(), context));
    }

    if let Some(action) = trap_action_word(command, source) {
        words.push((action.clone(), ExpansionContext::TrapAction));
    }

    for word in iter_command_words(visit, source) {
        collect_word_parameter_patterns(&word, words);
    }
}

fn collect_command_name_context_words(
    command: &Command,
    source: &str,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    if let Command::Simple(command) = command
        && static_word_text(&command.name, source).is_none()
    {
        words.push((command.name.clone(), ExpansionContext::CommandName));
    }
}

fn collect_argument_context_words(
    command: &Command,
    source: &str,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    match command {
        Command::Simple(command) => {
            if static_word_text(&command.name, source).is_some_and(|name| name == "trap") {
                return;
            }
            for word in &command.args {
                words.push((word.clone(), ExpansionContext::CommandArgument));
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                if let Some(word) = &command.depth {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
                for word in &command.extra_args {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(word) = &command.depth {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
                for word in &command.extra_args {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
                for word in &command.extra_args {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
                for word in &command.extra_args {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
            }
        },
        Command::Decl(command) => {
            for operand in &command.operands {
                if let DeclOperand::Dynamic(word) = operand {
                    words.push((word.clone(), ExpansionContext::CommandArgument));
                }
            }
        }
        Command::Binary(_) | Command::Compound(_) | Command::Function(_) => {}
    }
}

fn collect_expansion_assignment_value_words(
    command: &Command,
    source: &str,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    for assignment in command_assignments(command) {
        collect_expansion_assignment_words(
            assignment,
            source,
            ExpansionContext::AssignmentValue,
            words,
        );
    }

    for operand in declaration_operands(command) {
        match operand {
            DeclOperand::Name(reference) => collect_var_ref_subscript_context_words(
                reference,
                source,
                ExpansionContext::DeclarationAssignmentValue,
                words,
            ),
            DeclOperand::Assignment(assignment) => {
                collect_expansion_assignment_words(
                    assignment,
                    source,
                    ExpansionContext::DeclarationAssignmentValue,
                    words,
                );
            }
            DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => {}
        }
    }
}

fn collect_expansion_assignment_words(
    assignment: &Assignment,
    source: &str,
    context: ExpansionContext,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    collect_var_ref_subscript_context_words(&assignment.target, source, context, words);

    match &assignment.value {
        AssignmentValue::Scalar(word) => words.push((word.clone(), context)),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => words.push((word.clone(), context)),
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        collect_subscript_context_words(Some(key), source, context, words);
                        words.push((value.clone(), context))
                    }
                }
            }
        }
    }
}

fn collect_var_ref_subscript_context_words(
    reference: &VarRef,
    source: &str,
    context: ExpansionContext,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    collect_subscript_context_words(reference.subscript.as_ref(), source, context, words);
}

fn collect_subscript_context_words(
    subscript: Option<&Subscript>,
    source: &str,
    context: ExpansionContext,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    let mut subscript_words = Vec::new();
    collect_subscript_words(subscript, source, &mut subscript_words);
    words.extend(subscript_words.into_iter().map(|word| (word, context)));
}

fn collect_pattern_context_words(
    pattern: &Pattern,
    context: ExpansionContext,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_pattern_context_words(pattern, context, words);
                }
            }
            PatternPart::Word(word) => words.push((word.clone(), context)),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_conditional_expansion_words(
    expression: &ConditionalExpr,
    source: &str,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_expansion_words(&expr.left, source, words);
            collect_conditional_expansion_words(&expr.right, source, words);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_expansion_words(&expr.expr, source, words)
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_expansion_words(&expr.expr, source, words)
        }
        ConditionalExpr::Word(word) => {
            words.push((word.clone(), ExpansionContext::StringTestOperand))
        }
        ConditionalExpr::Regex(word) => words.push((word.clone(), ExpansionContext::RegexOperand)),
        ConditionalExpr::Pattern(_) => {}
        ConditionalExpr::VarRef(reference) => collect_var_ref_subscript_context_words(
            reference,
            source,
            ExpansionContext::ConditionalVarRefSubscript,
            words,
        ),
    }
}

fn collect_word_parameter_patterns(word: &Word, words: &mut Vec<(Word, ExpansionContext)>) {
    collect_word_part_parameter_patterns(&word.parts, words);
}

fn collect_word_part_parameter_patterns(
    parts: &[WordPartNode],
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_part_parameter_patterns(parts, words)
            }
            WordPart::ParameterExpansion { operator, .. } => {
                collect_parameter_operator_patterns(operator, words)
            }
            WordPart::IndirectExpansion {
                operator: Some(operator),
                ..
            } => collect_parameter_operator_patterns(operator, words),
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { operator: None, .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
        }
    }
}

fn collect_parameter_operator_patterns(
    operator: &ParameterOp,
    words: &mut Vec<(Word, ExpansionContext)>,
) {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern }
        | ParameterOp::ReplaceFirst { pattern, .. }
        | ParameterOp::ReplaceAll { pattern, .. } => {
            collect_pattern_context_words(pattern, ExpansionContext::ParameterPattern, words);
        }
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

fn trap_action_word<'a>(command: &'a Command, source: &str) -> Option<&'a Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    if static_word_text(&command.name, source).as_deref() != Some("trap") {
        return None;
    }

    let mut start = 0usize;

    if let Some(first) = command
        .args
        .first()
        .and_then(|word| static_word_text(word, source))
    {
        match first.as_str() {
            "-p" | "-l" => return None,
            "--" => start = 1,
            _ => {}
        }
    }

    let action = command.args.get(start)?;
    command.args.get(start + 1)?;
    Some(action)
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
        context,
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
        Command::Function(FunctionDef { body, .. }) => {
            collect_command_visit(body, options, context, visits);
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

struct CommandWalker<'a, F> {
    options: CommandWalkOptions,
    visitor: &'a mut F,
}

impl<F: FnMut(CommandVisit<'_>)> CommandWalker<'_, F> {
    fn walk_commands(&mut self, commands: &StmtSeq, context: WalkContext) {
        for stmt in commands.iter() {
            self.walk_command(stmt, context);
        }
    }

    fn walk_command(&mut self, stmt: &Stmt, context: WalkContext) {
        (self.visitor)(CommandVisit {
            stmt,
            command: &stmt.command,
            redirects: &stmt.redirects,
            context,
        });

        match &stmt.command {
            Command::Simple(command) => {
                self.walk_assignments(&command.assignments, context);
                self.walk_word(&command.name, context);
                self.walk_words(&command.args, context);
            }
            Command::Builtin(command) => self.walk_builtin(command, context),
            Command::Decl(command) => {
                self.walk_assignments(&command.assignments, context);
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            self.walk_word(word, context);
                        }
                        DeclOperand::Name(_) => {}
                        DeclOperand::Assignment(assignment) => {
                            self.walk_assignment(assignment, context);
                        }
                    }
                }
            }
            Command::Binary(command) => {
                self.walk_command(&command.left, context);
                self.walk_command(&command.right, context);
            }
            Command::Compound(command) => {
                self.walk_compound(command, context);
            }
            Command::Function(FunctionDef { body, .. }) => self.walk_command(body, context),
        }

        self.walk_redirects(&stmt.redirects, context);
    }

    fn walk_builtin(&mut self, command: &BuiltinCommand, context: WalkContext) {
        match command {
            BuiltinCommand::Break(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.depth {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
            }
            BuiltinCommand::Continue(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.depth {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
            }
            BuiltinCommand::Return(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.code {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
            }
            BuiltinCommand::Exit(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.code {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
            }
        }
    }

    fn walk_compound(&mut self, command: &CompoundCommand, context: WalkContext) {
        match command {
            CompoundCommand::If(command) => {
                self.walk_commands(&command.condition, context);
                self.walk_commands(&command.then_branch, context);
                for (condition, body) in &command.elif_branches {
                    self.walk_commands(condition, context);
                    self.walk_commands(body, context);
                }
                if let Some(body) = &command.else_branch {
                    self.walk_commands(body, context);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    self.walk_words(words, context);
                }
                self.walk_commands(
                    &command.body,
                    WalkContext {
                        loop_depth: context.loop_depth + 1,
                    },
                );
            }
            CompoundCommand::ArithmeticFor(command) => self.walk_commands(
                &command.body,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
            ),
            CompoundCommand::While(command) => {
                let loop_context = WalkContext {
                    loop_depth: context.loop_depth + 1,
                };
                self.walk_commands(&command.condition, loop_context);
                self.walk_commands(&command.body, loop_context);
            }
            CompoundCommand::Until(command) => {
                let loop_context = WalkContext {
                    loop_depth: context.loop_depth + 1,
                };
                self.walk_commands(&command.condition, loop_context);
                self.walk_commands(&command.body, loop_context);
            }
            CompoundCommand::Case(command) => {
                self.walk_word(&command.word, context);
                for case in &command.cases {
                    self.walk_patterns(&case.patterns, context);
                    self.walk_commands(&case.body, context);
                }
            }
            CompoundCommand::Select(command) => {
                self.walk_words(&command.words, context);
                self.walk_commands(
                    &command.body,
                    WalkContext {
                        loop_depth: context.loop_depth + 1,
                    },
                );
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.walk_commands(commands, context);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.walk_command(command, context);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.walk_conditional_expr(&command.expression, context);
            }
            CompoundCommand::Coproc(command) => self.walk_command(&command.body, context),
        }
    }

    fn walk_assignments(&mut self, assignments: &[Assignment], context: WalkContext) {
        for assignment in assignments {
            self.walk_assignment(assignment, context);
        }
    }

    fn walk_assignment(&mut self, assignment: &Assignment, context: WalkContext) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.walk_word(word, context),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.walk_word(word, context),
                        ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                            self.walk_word(value, context)
                        }
                    }
                }
            }
        }
    }

    fn walk_words(&mut self, words: &[Word], context: WalkContext) {
        for word in words {
            self.walk_word(word, context);
        }
    }

    fn walk_patterns(&mut self, patterns: &[Pattern], context: WalkContext) {
        for pattern in patterns {
            self.walk_pattern(pattern, context);
        }
    }

    fn walk_word(&mut self, word: &Word, context: WalkContext) {
        if !self.options.descend_nested_word_commands {
            return;
        }

        self.walk_word_parts(&word.parts, context);
    }

    fn walk_pattern(&mut self, pattern: &Pattern, context: WalkContext) {
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => self.walk_patterns(patterns, context),
                PatternPart::Word(word) => self.walk_word(word, context),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn walk_word_parts(&mut self, parts: &[WordPartNode], context: WalkContext) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => self.walk_word_parts(parts, context),
                WordPart::ArithmeticExpansion { expression_ast, .. } => {
                    visit_optional_arithmetic_words(expression_ast.as_ref(), &mut |word| {
                        self.walk_word(word, context);
                    });
                }
                WordPart::CommandSubstitution { body, .. }
                | WordPart::ProcessSubstitution { body, .. } => {
                    self.walk_commands(body, context);
                }
                _ => {}
            }
        }
    }

    fn walk_conditional_expr(&mut self, expression: &ConditionalExpr, context: WalkContext) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.walk_conditional_expr(&expr.left, context);
                self.walk_conditional_expr(&expr.right, context);
            }
            ConditionalExpr::Unary(expr) => self.walk_conditional_expr(&expr.expr, context),
            ConditionalExpr::Parenthesized(expr) => self.walk_conditional_expr(&expr.expr, context),
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.walk_word(word, context)
            }
            ConditionalExpr::Pattern(pattern) => self.walk_pattern(pattern, context),
            ConditionalExpr::VarRef(reference) => {
                visit_var_ref_subscript_words(reference, &mut |word| {
                    self.walk_word(word, context);
                });
            }
        }
    }

    fn walk_redirects(&mut self, redirects: &[Redirect], context: WalkContext) {
        for redirect in redirects {
            self.walk_word(redirect_walk_word(redirect), context);
        }
    }
}

struct WordWalker<'a, F> {
    options: CommandWalkOptions,
    visitor: &'a mut F,
}

impl<F: FnMut(&Word)> WordWalker<'_, F> {
    fn walk_commands(&mut self, commands: &StmtSeq) {
        for stmt in commands.iter() {
            self.walk_command(stmt);
        }
    }

    fn walk_command(&mut self, stmt: &Stmt) {
        match &stmt.command {
            Command::Simple(command) => {
                self.walk_assignments(&command.assignments);
                self.walk_word(&command.name);
                self.walk_words(&command.args);
            }
            Command::Builtin(command) => self.walk_builtin(command),
            Command::Decl(command) => {
                self.walk_assignments(&command.assignments);
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            self.walk_word(word);
                        }
                        DeclOperand::Name(_) => {}
                        DeclOperand::Assignment(assignment) => self.walk_assignment(assignment),
                    }
                }
            }
            Command::Binary(command) => {
                self.walk_command(&command.left);
                self.walk_command(&command.right);
            }
            Command::Compound(command) => {
                self.walk_compound(command);
            }
            Command::Function(FunctionDef { body, .. }) => self.walk_command(body),
        }

        self.walk_redirects(&stmt.redirects);
    }

    fn walk_builtin(&mut self, command: &BuiltinCommand) {
        match command {
            BuiltinCommand::Break(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.depth {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
            }
            BuiltinCommand::Continue(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.depth {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
            }
            BuiltinCommand::Return(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.code {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
            }
            BuiltinCommand::Exit(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.code {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
            }
        }
    }

    fn walk_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.then_branch);
                for (condition, body) in &command.elif_branches {
                    self.walk_commands(condition);
                    self.walk_commands(body);
                }
                if let Some(body) = &command.else_branch {
                    self.walk_commands(body);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    self.walk_words(words);
                }
                self.walk_commands(&command.body);
            }
            CompoundCommand::ArithmeticFor(command) => self.walk_commands(&command.body),
            CompoundCommand::While(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Until(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Case(command) => {
                self.walk_word(&command.word);
                for case in &command.cases {
                    self.walk_patterns(&case.patterns);
                    self.walk_commands(&case.body);
                }
            }
            CompoundCommand::Select(command) => {
                self.walk_words(&command.words);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.walk_commands(commands);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.walk_command(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.walk_conditional_expr(&command.expression);
            }
            CompoundCommand::Coproc(command) => self.walk_command(&command.body),
        }
    }

    fn walk_assignments(&mut self, assignments: &[Assignment]) {
        for assignment in assignments {
            self.walk_assignment(assignment);
        }
    }

    fn walk_assignment(&mut self, assignment: &Assignment) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.walk_word(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.walk_word(word),
                        ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                            self.walk_word(value)
                        }
                    }
                }
            }
        }
    }

    fn walk_words(&mut self, words: &[Word]) {
        for word in words {
            self.walk_word(word);
        }
    }

    fn walk_patterns(&mut self, patterns: &[Pattern]) {
        for pattern in patterns {
            self.walk_pattern(pattern);
        }
    }

    fn walk_word(&mut self, word: &Word) {
        (self.visitor)(word);

        if !self.options.descend_nested_word_commands {
            return;
        }

        self.walk_word_parts(&word.parts);
    }

    fn walk_pattern(&mut self, pattern: &Pattern) {
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => self.walk_patterns(patterns),
                PatternPart::Word(word) => self.walk_word(word),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn walk_word_parts(&mut self, parts: &[WordPartNode]) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => self.walk_word_parts(parts),
                WordPart::ArithmeticExpansion { expression_ast, .. } => {
                    visit_optional_arithmetic_words(expression_ast.as_ref(), &mut |word| {
                        self.walk_word(word);
                    });
                }
                WordPart::CommandSubstitution { body, .. }
                | WordPart::ProcessSubstitution { body, .. } => self.walk_commands(body),
                _ => {}
            }
        }
    }

    fn walk_redirects(&mut self, redirects: &[Redirect]) {
        for redirect in redirects {
            self.walk_word(redirect_walk_word(redirect));
        }
    }

    fn walk_conditional_expr(&mut self, expression: &ConditionalExpr) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.walk_conditional_expr(&expr.left);
                self.walk_conditional_expr(&expr.right);
            }
            ConditionalExpr::Unary(expr) => self.walk_conditional_expr(&expr.expr),
            ConditionalExpr::Parenthesized(expr) => self.walk_conditional_expr(&expr.expr),
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => self.walk_word(word),
            ConditionalExpr::Pattern(pattern) => self.walk_pattern(pattern),
            ConditionalExpr::VarRef(reference) => {
                visit_var_ref_subscript_words(reference, &mut |word| {
                    self.walk_word(word);
                });
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

fn collect_command_words(
    command: &Command,
    redirects: &[Redirect],
    source: &str,
    words: &mut Vec<Word>,
) {
    match command {
        Command::Simple(command) => {
            collect_assignments_words(&command.assignments, source, words);
            words.push(command.name.clone());
            collect_words(&command.args, words);
        }
        Command::Builtin(command) => collect_builtin_words(command, source, words),
        Command::Decl(command) => {
            collect_assignments_words(&command.assignments, source, words);
            for operand in &command.operands {
                collect_decl_operand_words(operand, source, words);
            }
        }
        Command::Binary(_) | Command::Function(_) => {}
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(command_words) = &command.words {
                    collect_words(command_words, words);
                }
            }
            CompoundCommand::Case(command) => {
                words.push(command.word.clone());
                for case in &command.cases {
                    collect_pattern_words(&case.patterns, words);
                }
            }
            CompoundCommand::Select(command) => collect_words(&command.words, words),
            CompoundCommand::Conditional(command) => {
                collect_conditional_words(&command.expression, source, words);
            }
            CompoundCommand::If(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Time(_)
            | CompoundCommand::Coproc(_) => {}
        },
    }

    collect_redirect_target_words(redirects, words);
}

fn collect_assignments_words(assignments: &[Assignment], source: &str, words: &mut Vec<Word>) {
    for assignment in assignments {
        collect_assignment_words(assignment, source, words);
    }
}

fn collect_assignment_words(assignment: &Assignment, source: &str, words: &mut Vec<Word>) {
    collect_subscript_words(assignment.target.subscript.as_ref(), source, words);

    match &assignment.value {
        AssignmentValue::Scalar(word) => words.push(word.clone()),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => words.push(word.clone()),
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        collect_subscript_words(Some(key), source, words);
                        words.push(value.clone());
                    }
                }
            }
        }
    }
}

fn collect_builtin_words(command: &BuiltinCommand, source: &str, words: &mut Vec<Word>) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignments_words(&command.assignments, source, words);
            if let Some(word) = &command.depth {
                words.push(word.clone());
            }
            collect_words(&command.extra_args, words);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignments_words(&command.assignments, source, words);
            if let Some(word) = &command.depth {
                words.push(word.clone());
            }
            collect_words(&command.extra_args, words);
        }
        BuiltinCommand::Return(command) => {
            collect_assignments_words(&command.assignments, source, words);
            if let Some(word) = &command.code {
                words.push(word.clone());
            }
            collect_words(&command.extra_args, words);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignments_words(&command.assignments, source, words);
            if let Some(word) = &command.code {
                words.push(word.clone());
            }
            collect_words(&command.extra_args, words);
        }
    }
}

fn collect_word_command_substitutions(
    parts: &[WordPartNode],
    substitutions: &mut Vec<NestedCommandSubstitution>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_command_substitutions(parts, substitutions);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                let mut arithmetic_words = Vec::new();
                collect_optional_arithmetic_words(expression_ast.as_ref(), &mut arithmetic_words);
                for word in arithmetic_words {
                    collect_word_command_substitutions(&word.parts, substitutions);
                }
            }
            WordPart::CommandSubstitution { body, .. } => {
                substitutions.push(NestedCommandSubstitution {
                    commands: body.clone(),
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                });
            }
            WordPart::ProcessSubstitution { body, is_input } => {
                substitutions.push(NestedCommandSubstitution {
                    commands: body.clone(),
                    span: part.span,
                    kind: if *is_input {
                        CommandSubstitutionKind::ProcessInput
                    } else {
                        CommandSubstitutionKind::ProcessOutput
                    },
                });
            }
            _ => {}
        }
    }
}

fn collect_decl_operand_words(operand: &DeclOperand, source: &str, words: &mut Vec<Word>) {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => words.push(word.clone()),
        DeclOperand::Name(reference) => {
            collect_subscript_words(reference.subscript.as_ref(), source, words)
        }
        DeclOperand::Assignment(assignment) => collect_assignment_words(assignment, source, words),
    }
}

fn collect_words(command_words: &[Word], words: &mut Vec<Word>) {
    words.extend(command_words.iter().cloned());
}

fn collect_pattern_words(patterns: &[Pattern], words: &mut Vec<Word>) {
    for pattern in patterns {
        collect_pattern_words_from_pattern(pattern, words);
    }
}

fn collect_redirect_target_words(redirects: &[Redirect], words: &mut Vec<Word>) {
    for redirect in redirects {
        words.push(redirect_walk_word(redirect).clone());
    }
}

fn redirect_walk_word(redirect: &Redirect) -> &Word {
    match redirect.word_target() {
        Some(word) => word,
        None => &redirect.heredoc().expect("expected heredoc redirect").body,
    }
}

fn collect_conditional_words(expression: &ConditionalExpr, source: &str, words: &mut Vec<Word>) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_words(&expr.left, source, words);
            collect_conditional_words(&expr.right, source, words);
        }
        ConditionalExpr::Unary(expr) => collect_conditional_words(&expr.expr, source, words),
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_words(&expr.expr, source, words)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => words.push(word.clone()),
        ConditionalExpr::Pattern(pattern) => collect_pattern_words_from_pattern(pattern, words),
        ConditionalExpr::VarRef(reference) => {
            collect_subscript_words(reference.subscript.as_ref(), source, words);
        }
    }
}

fn collect_pattern_words_from_pattern(pattern: &Pattern, words: &mut Vec<Word>) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => collect_pattern_words(patterns, words),
            PatternPart::Word(word) => words.push(word.clone()),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, RedirectKind, StmtSeq, Word, WordPart};
    use shuck_parser::parser::Parser;

    use super::{
        CommandSubstitutionKind, CommandVisit, CommandWalkOptions, command_assignments,
        command_redirects, declaration_operands, iter_command_substitutions, iter_command_words,
        iter_commands, iter_expansion_words, iter_word_command_substitutions, pipeline_segments,
        visit_expansion_words,
    };
    use crate::rules::common::expansion::ExpansionContext;

    fn parse_commands(source: &str) -> StmtSeq {
        let output = Parser::new(source).parse().unwrap();
        output.file.body
    }

    fn top_level_visits(commands: &StmtSeq) -> Vec<CommandVisit<'_>> {
        commands
            .iter()
            .map(|stmt| CommandVisit {
                stmt,
                command: &stmt.command,
                redirects: &stmt.redirects,
                context: Default::default(),
            })
            .collect()
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
    fn iter_commands_tracks_loop_depth_across_for_while_and_select() {
        let source = "for item in \"$(printf for-word)\"; do\n    printf for-body\n\
done\nwhile printf while-cond; do\n    printf while-body\n\
done\nselect item in \"$(printf select-word)\"; do\n    printf select-body\n\
done\n";
        let commands = parse_commands(source);
        let seen = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        )
        .filter_map(|visit| {
            let Command::Simple(command) = visit.command else {
                return None;
            };
            let name = static_word_text(&command.name, source)?;
            if name != "printf" {
                return None;
            }

            let label = command
                .args
                .first()
                .and_then(|word| static_word_text(word, source))
                .unwrap();
            Some((label, visit.context.loop_depth))
        })
        .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                ("for-word".to_owned(), 0),
                ("for-body".to_owned(), 1),
                ("while-cond".to_owned(), 1),
                ("while-body".to_owned(), 1),
                ("select-word".to_owned(), 0),
                ("select-body".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn common_iterators_cover_command_shapes() {
        let source = "foo=1 printf bar >simple\nfoo=1 break 2 bar >builtin\n\
foo=1 export foo=1 >decl\nfor item in foo bar; do :; done >compound\n";
        let commands = parse_commands(source);
        let visits = top_level_visits(&commands);

        let assignment_names = visits
            .iter()
            .map(|visit| {
                command_assignments(visit.command)
                    .iter()
                    .map(|assignment| assignment.target.name.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let operand_counts = visits
            .iter()
            .map(|visit| declaration_operands(visit.command).len())
            .collect::<Vec<_>>();

        let word_lists: Vec<Vec<String>> = visits
            .iter()
            .map(|visit| {
                iter_command_words(*visit, source)
                    .map(|word| static_word_text(&word, source).unwrap())
                    .collect()
            })
            .collect();

        let redirect_lists: Vec<Vec<String>> = visits
            .iter()
            .map(|visit| {
                command_redirects(*visit)
                    .iter()
                    .map(|redirect| {
                        static_word_text(
                            redirect
                                .word_target()
                                .expect("expected non-heredoc redirect target"),
                            source,
                        )
                        .unwrap()
                    })
                    .collect()
            })
            .collect();

        assert_eq!(
            assignment_names,
            vec![
                vec!["foo".to_owned()],
                vec!["foo".to_owned()],
                vec!["foo".to_owned()],
                Vec::<String>::new(),
            ]
        );
        assert_eq!(operand_counts, vec![0, 0, 1, 0]);
        assert_eq!(
            word_lists,
            vec![
                vec![
                    "1".to_owned(),
                    "printf".to_owned(),
                    "bar".to_owned(),
                    "simple".to_owned(),
                ],
                vec![
                    "1".to_owned(),
                    "2".to_owned(),
                    "bar".to_owned(),
                    "builtin".to_owned(),
                ],
                vec!["1".to_owned(), "1".to_owned(), "decl".to_owned()],
                vec!["foo".to_owned(), "bar".to_owned(), "compound".to_owned(),],
            ]
        );
        assert_eq!(
            redirect_lists,
            vec![
                vec!["simple".to_owned()],
                vec!["builtin".to_owned()],
                vec!["decl".to_owned()],
                vec!["compound".to_owned()],
            ]
        );
    }

    #[test]
    fn word_command_substitution_iterator_reports_kind_and_span() {
        let source = "echo \"$(printf cmd)\" <(printf in) >(printf out)\n";
        let commands = parse_commands(source);
        let substitutions = iter_command_substitutions(top_level_visits(&commands)[0], source)
            .map(|substitution| {
                let label = substitution
                    .commands
                    .first()
                    .and_then(|command| match &command.command {
                        Command::Simple(command) => command
                            .args
                            .first()
                            .and_then(|word| static_word_text(word, source)),
                        _ => None,
                    })
                    .unwrap();
                (label, substitution.kind, substitution.span)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            substitutions
                .iter()
                .map(|(label, kind, _)| (label.clone(), *kind))
                .collect::<Vec<_>>(),
            vec![
                ("cmd".to_owned(), CommandSubstitutionKind::Command),
                ("in".to_owned(), CommandSubstitutionKind::ProcessInput),
                ("out".to_owned(), CommandSubstitutionKind::ProcessOutput),
            ]
        );
        assert!(
            substitutions
                .iter()
                .all(|(_, _, span)| span.start.offset < span.end.offset)
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
    fn command_substitution_iterators_reach_assignments_here_strings_conditionals_wrappers_and_compounds()
     {
        let source = "value=\"$(printf assign)\"\n\
command printf \"$(printf wrapped)\"\n\
cat <<< \"$(printf here)\"\n\
if [[ \"$(printf lhs)\" = \"$(printf rhs)\" ]]; then :; fi\n\
for item in \"$(printf loop)\"; do :; done\n";
        let commands = parse_commands(source);

        let substitution_labels = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        )
        .map(|visit| {
            iter_command_substitutions(visit, source)
                .map(|substitution| {
                    substitution
                        .commands
                        .first()
                        .and_then(|nested_command| match &nested_command.command {
                            Command::Simple(command) => command
                                .args
                                .first()
                                .and_then(|word| static_word_text(word, source)),
                            _ => None,
                        })
                        .unwrap()
                })
                .collect::<Vec<_>>()
        })
        .filter(|labels| !labels.is_empty())
        .collect::<Vec<_>>();

        assert_eq!(
            substitution_labels,
            vec![
                vec!["assign".to_owned()],
                vec!["wrapped".to_owned()],
                vec!["here".to_owned()],
                vec!["lhs".to_owned(), "rhs".to_owned()],
                vec!["loop".to_owned()],
            ]
        );

        let wrapper_word_substitutions = match &commands[1].command {
            Command::Simple(command) => iter_word_command_substitutions(&command.args[1])
                .map(|substitution| substitution.kind)
                .collect::<Vec<_>>(),
            _ => panic!("expected simple command"),
        };
        assert_eq!(
            wrapper_word_substitutions,
            vec![CommandSubstitutionKind::Command]
        );
    }

    #[test]
    fn command_substitution_iterators_reach_non_arithmetic_subscripts_and_patterns() {
        let source = "\
declare arr[$(printf decl-subscript)]=1
name[$(printf assign-subscript)]=1
declare -A map=([\"$(printf keyed-subscript)\"]=1)
[[ $lhs == \"$(printf pattern-substitution)\" ]]
[[ -v assoc[\"$(printf conditional-subscript)\"] ]]
";
        let commands = parse_commands(source);
        let seen = top_level_visits(&commands)
            .into_iter()
            .flat_map(|visit| iter_command_substitutions(visit, source))
            .map(|substitution| {
                let Command::Simple(command) = &substitution.commands[0].command else {
                    panic!("expected simple command in substitution");
                };
                static_word_text(&command.args[0], source)
                    .expect("expected static substitution argument")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                "decl-subscript".to_owned(),
                "assign-subscript".to_owned(),
                "keyed-subscript".to_owned(),
                "pattern-substitution".to_owned(),
                "conditional-subscript".to_owned(),
            ]
        );
    }

    #[test]
    fn visit_expansion_words_covers_all_project_one_contexts() {
        let source = "\
value=$assign
export declared=$decl
$cmd '%s\\n' $arg >$out >&$dup <<< $here
for item in $first; do :; done
select item in $choice; do :; done
case $subject in
  $case_pat) : ;;
esac
[[ $left < $right ]]
[[ $text =~ $regex ]]
trimmed=${value%$suffix}
trap -- \"echo $trap_body\" EXIT
[[ -v assoc[$(( $idx + 1 ))] ]]
";
        let commands = parse_commands(source);
        let mut seen = Vec::new();

        for visit in top_level_visits(&commands) {
            visit_expansion_words(visit, source, &mut |word, context| {
                seen.push((context, word.span.slice(source).to_owned()));
            });
        }

        assert_eq!(
            seen,
            vec![
                (ExpansionContext::AssignmentValue, "$assign".to_owned()),
                (
                    ExpansionContext::DeclarationAssignmentValue,
                    "$decl".to_owned()
                ),
                (ExpansionContext::CommandName, "$cmd".to_owned()),
                (ExpansionContext::CommandArgument, "'%s\\n'".to_owned()),
                (ExpansionContext::CommandArgument, "$arg".to_owned()),
                (
                    ExpansionContext::RedirectTarget(RedirectKind::Output),
                    "$out".to_owned(),
                ),
                (
                    ExpansionContext::DescriptorDupTarget(RedirectKind::DupOutput),
                    "$dup".to_owned(),
                ),
                (ExpansionContext::HereString, "$here".to_owned()),
                (ExpansionContext::ForList, "$first".to_owned()),
                (ExpansionContext::SelectList, "$choice".to_owned()),
                (ExpansionContext::CasePattern, "$case_pat".to_owned()),
                (ExpansionContext::StringTestOperand, "$left".to_owned()),
                (ExpansionContext::StringTestOperand, "$right".to_owned()),
                (ExpansionContext::StringTestOperand, "$text".to_owned()),
                (ExpansionContext::RegexOperand, "$regex".to_owned()),
                (
                    ExpansionContext::AssignmentValue,
                    "${value%$suffix}".to_owned()
                ),
                (ExpansionContext::ParameterPattern, "$suffix".to_owned()),
                (
                    ExpansionContext::TrapAction,
                    "\"echo $trap_body\"".to_owned()
                ),
                (
                    ExpansionContext::ConditionalVarRefSubscript,
                    "$(( $idx + 1 ))".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn visit_expansion_words_covers_non_arithmetic_subscripts() {
        let source = "\
declare arr[$(printf decl-subscript)]=1
name[$(printf assign-subscript)]=1
declare -A map=([\"$(printf keyed-subscript)\"]=1)
[[ $lhs == \"$(printf pattern-substitution)\" ]]
[[ -v assoc[\"$(printf conditional-subscript)\"] ]]
";
        let commands = parse_commands(source);
        let seen = top_level_visits(&commands)
            .into_iter()
            .flat_map(|visit| {
                let mut words = Vec::new();
                visit_expansion_words(visit, source, &mut |word, context| {
                    let text = word.span.slice(source);
                    if text.contains("printf") {
                        words.push((context, text.to_owned()));
                    }
                });
                words
            })
            .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                (
                    ExpansionContext::DeclarationAssignmentValue,
                    "$(printf decl-subscript)".to_owned(),
                ),
                (
                    ExpansionContext::AssignmentValue,
                    "$(printf assign-subscript)".to_owned(),
                ),
                (
                    ExpansionContext::DeclarationAssignmentValue,
                    "\"$(printf keyed-subscript)\"".to_owned(),
                ),
                (
                    ExpansionContext::ConditionalVarRefSubscript,
                    "\"$(printf conditional-subscript)\"".to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn iter_expansion_words_distinguishes_same_word_by_shell_context() {
        let source = "\
$same '%s\\n' $same >$same
case $value in
  $same) : ;;
esac
[[ $text =~ $same ]]
";
        let commands = parse_commands(source);
        let seen = top_level_visits(&commands)
            .into_iter()
            .flat_map(|visit| {
                iter_expansion_words(visit, source)
                    .filter(|(word, _)| word.span.slice(source) == "$same")
                    .map(|(_, context)| context)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                ExpansionContext::CommandName,
                ExpansionContext::CommandArgument,
                ExpansionContext::RedirectTarget(RedirectKind::Output),
                ExpansionContext::CasePattern,
                ExpansionContext::RegexOperand,
            ]
        );
    }

    #[test]
    fn iter_command_substitutions_descends_into_arithmetic_and_var_ref_subscripts() {
        let source = "\
echo $(( $(printf outer) + 1 ))
[[ -v assoc[$(( $(printf inner) ))] ]]
";
        let commands = parse_commands(source);
        let seen = top_level_visits(&commands)
            .into_iter()
            .flat_map(|visit| iter_command_substitutions(visit, source))
            .map(|substitution| {
                let Command::Simple(command) = &substitution.commands[0].command else {
                    panic!("expected simple command in substitution");
                };
                (
                    substitution.kind,
                    static_word_text(&command.name, source).expect("expected literal command name"),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                (CommandSubstitutionKind::Command, "printf".to_owned()),
                (CommandSubstitutionKind::Command, "printf".to_owned()),
            ]
        );
    }
}
