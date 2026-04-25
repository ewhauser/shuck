use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BourneParameterExpansion, BuiltinCommand, CaseItem, Command, CompoundCommand, ConditionalExpr,
    DeclOperand, File, FunctionDef, HeredocBodyPartNode, ParameterExpansion,
    ParameterExpansionSyntax, Pattern, PatternPart, Redirect, Stmt, StmtSeq, TextRange, TextSize,
    VarRef, Word, WordPart, WordPartNode, ZshExpansionTarget, ZshGlobSegment,
};
use shuck_indexer::{CommentIndex, IndexedComment};

use crate::{Rule, code_to_rule};

use super::ShellCheckCodeMap;

/// A parsed suppression directive from a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionDirective {
    /// The action: disable, disable-file, or ignore.
    pub action: SuppressionAction,
    /// Which directive syntax produced this.
    pub source: SuppressionSource,
    /// Rule codes this directive applies to.
    pub codes: Vec<Rule>,
    /// The comment's source range.
    pub range: TextRange,
    /// 1-based line number of the directive.
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionAction {
    Disable,
    DisableFile,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionSource {
    Shuck,
    ShellCheck,
}

/// Parse all suppression directives from a file's comments.
pub fn parse_directives(
    source: &str,
    file: &File,
    comment_index: &CommentIndex,
    shellcheck_map: &ShellCheckCodeMap,
) -> Vec<SuppressionDirective> {
    let mut directives = Vec::new();

    for comment in comment_index.comments() {
        let Some(comment) = normalized_comment(source, comment) else {
            continue;
        };

        if let Some(directive) = parse_shuck_directive(source, &comment, file, shellcheck_map) {
            directives.push(directive);
        } else if let Some(directive) =
            parse_shellcheck_directive(source, &comment, file, shellcheck_map)
        {
            directives.push(directive);
        }
    }

    directives.sort_by_key(|directive| {
        (
            directive.line,
            directive.range.start().to_u32(),
            directive.range.end().to_u32(),
        )
    });
    directives
}

#[derive(Debug, Clone, Copy)]
struct NormalizedComment<'a> {
    text: &'a str,
    range: TextRange,
    line: u32,
    is_own_line: bool,
}

fn normalized_comment<'a>(
    source: &'a str,
    comment: &IndexedComment,
) -> Option<NormalizedComment<'a>> {
    let start = usize::from(comment.range.start()).min(source.len());
    let end = usize::from(comment.range.end()).min(source.len());
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |index| end + index);
    let line = &source[line_start..line_end];
    let relative_start = start.saturating_sub(line_start).min(line.len());
    let relative_end = end.saturating_sub(line_start).min(line.len());
    let marker = find_comment_marker(line, relative_start, relative_end)?;
    let comment_start = line_start + marker;

    Some(NormalizedComment {
        text: &source[comment_start..line_end],
        range: TextRange::new(
            TextSize::new(comment_start as u32),
            TextSize::new(line_end as u32),
        ),
        line: u32::try_from(comment.line).ok()?,
        is_own_line: is_horizontal_whitespace(&source[line_start..comment_start]),
    })
}

fn find_comment_marker(line: &str, start: usize, end: usize) -> Option<usize> {
    line.match_indices('#')
        .min_by_key(|(index, _)| {
            (
                index.abs_diff(start),
                usize::from(*index < start),
                index.abs_diff(end),
            )
        })
        .map(|(index, _)| index)
}

fn is_horizontal_whitespace(text: &str) -> bool {
    text.chars().all(|ch| matches!(ch, ' ' | '\t' | '\r'))
}

fn parse_shuck_directive(
    source: &str,
    comment: &NormalizedComment<'_>,
    file: &File,
    shellcheck_map: &ShellCheckCodeMap,
) -> Option<SuppressionDirective> {
    let body = strip_comment_prefix(comment.text);
    let remainder = strip_prefix_ignore_ascii_case(body, "shuck:")?;
    let remainder = remainder
        .split_once('#')
        .map_or(remainder, |(before, _)| before);
    let (action, codes) = remainder.split_once('=')?;
    let action = parse_shuck_action(action.trim())?;
    let codes = parse_codes(codes, |code| resolve_suppression_code(code, shellcheck_map));
    if codes.is_empty() {
        return None;
    }

    if action == SuppressionAction::Disable
        && !comment.is_own_line
        && !shellcheck_directive_can_apply_to_following_command(source, file, comment.range)
        && !is_case_label_directive(comment, file)
    {
        return None;
    }

    Some(SuppressionDirective {
        action,
        source: SuppressionSource::Shuck,
        codes,
        range: comment.range,
        line: comment.line,
    })
}

fn parse_shellcheck_directive(
    source: &str,
    comment: &NormalizedComment<'_>,
    file: &File,
    shellcheck_map: &ShellCheckCodeMap,
) -> Option<SuppressionDirective> {
    let remainder = shellcheck_comment_remainder(comment.text)?;

    let mut codes = Vec::new();
    for part in remainder.split_ascii_whitespace() {
        if let Some(group) = strip_prefix_ignore_ascii_case(part, "disable=") {
            for code in group.split(',') {
                let code = code.trim();
                if code.is_empty() {
                    continue;
                }
                if code.eq_ignore_ascii_case("all") {
                    codes.extend(Rule::iter());
                } else {
                    codes.extend(resolve_suppression_code(code, shellcheck_map));
                }
            }
        }
    }

    if codes.is_empty() {
        return None;
    }

    if !comment.is_own_line
        && !shellcheck_directive_can_apply_to_following_command(source, file, comment.range)
        && !is_case_label_directive(comment, file)
    {
        return None;
    }

    Some(SuppressionDirective {
        action: SuppressionAction::Disable,
        source: SuppressionSource::ShellCheck,
        codes,
        range: comment.range,
        line: comment.line,
    })
}

pub(crate) fn shellcheck_directive_can_apply_to_following_command(
    source: &str,
    file: &File,
    comment_range: TextRange,
) -> bool {
    let Some(context) = inline_directive_context(source, comment_range) else {
        return false;
    };

    if context.prefix.trim_end().ends_with(';') {
        return true;
    }

    command_visits(&file.body)
        .into_iter()
        .any(|visit| match visit.command {
            Command::Compound(CompoundCommand::If(command)) => {
                let if_header = command.condition.first().is_some_and(|stmt| {
                    next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                        && command_start_segment_matches(
                            source,
                            visit.stmt.span.start.offset,
                            context.comment_start,
                            "if",
                        )
                });

                let then_header = body_header_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.then_branch,
                    &context,
                    "then",
                ) || body_opener_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.then_branch,
                    &context,
                    '{',
                );

                let mut previous_branch_end = command.then_branch.span.end.offset;
                let mut elif_header = false;
                let mut elif_body_header = false;
                for (condition, branch) in &command.elif_branches {
                    elif_header |= condition.first().is_some_and(|stmt| {
                        next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                            && gap_segment_ends_with_keyword(
                                source,
                                previous_branch_end,
                                context.comment_start,
                                "elif",
                            )
                    });
                    elif_body_header |= body_header_matches(
                        source,
                        condition.span.end.offset,
                        branch,
                        &context,
                        "then",
                    ) || body_opener_matches(
                        source,
                        condition.span.end.offset,
                        branch,
                        &context,
                        '{',
                    );
                    previous_branch_end = branch.span.end.offset;
                }

                let else_header = command.else_branch.as_ref().is_some_and(|branch| {
                    branch.first().is_some_and(|stmt| {
                        next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                            && (gap_segment_ends_with_keyword(
                                source,
                                previous_branch_end,
                                context.comment_start,
                                "else",
                            ) || gap_segment_ends_with_char(
                                source,
                                previous_branch_end,
                                context.comment_start,
                                '{',
                            ))
                    })
                });

                if_header || then_header || elif_header || elif_body_header || else_header
            }
            Command::Compound(CompoundCommand::For(command)) => {
                body_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::Repeat(command)) => {
                body_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::Foreach(command)) => {
                body_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::ArithmeticFor(command)) => {
                body_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::While(command)) => {
                loop_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.condition,
                    context,
                    "while",
                ) || body_header_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::Until(command)) => {
                loop_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.condition,
                    context,
                    "until",
                ) || body_header_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::Select(command)) => {
                body_header_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    "do",
                ) || body_opener_matches(
                    source,
                    visit.stmt.span.start.offset,
                    &command.body,
                    &context,
                    '{',
                )
            }
            Command::Compound(CompoundCommand::Subshell(body)) => {
                body.first().is_some_and(|stmt| {
                    next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                        && context.prefix.trim_end().ends_with('(')
                })
            }
            Command::Compound(CompoundCommand::BraceGroup(body)) => {
                body.first().is_some_and(|stmt| {
                    next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                        && context.prefix.trim_end().ends_with('{')
                })
            }
            _ => false,
        })
}

fn is_case_label_directive(comment: &NormalizedComment<'_>, file: &File) -> bool {
    command_visits(&file.body).into_iter().any(|visit| {
        let Command::Compound(CompoundCommand::Case(command)) = visit.command else {
            return false;
        };

        command
            .cases
            .iter()
            .any(|case| case_label_directive(case, comment))
    })
}

#[derive(Clone, Copy)]
struct DirectiveCommandVisit<'a> {
    stmt: &'a Stmt,
    command: &'a Command,
}

fn command_visits(commands: &StmtSeq) -> Vec<DirectiveCommandVisit<'_>> {
    let mut visits = Vec::new();
    collect_command_visits(commands, &mut visits);
    visits
}

fn collect_command_visits<'a>(commands: &'a StmtSeq, visits: &mut Vec<DirectiveCommandVisit<'a>>) {
    for stmt in commands.iter() {
        collect_command_visit(stmt, visits);
    }
}

fn collect_command_visit<'a>(stmt: &'a Stmt, visits: &mut Vec<DirectiveCommandVisit<'a>>) {
    visits.push(DirectiveCommandVisit {
        stmt,
        command: &stmt.command,
    });

    match &stmt.command {
        Command::Simple(command) => {
            collect_assignment_visits(&command.assignments, visits);
            collect_word_visits(&command.name, visits);
            collect_word_slice_visits(&command.args, visits);
        }
        Command::Builtin(command) => collect_builtin_visits(command, visits),
        Command::Decl(command) => {
            collect_assignment_visits(&command.assignments, visits);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word_visits(word, visits);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment_visit(assignment, visits);
                    }
                }
            }
        }
        Command::Binary(command) => {
            collect_command_visit(&command.left, visits);
            collect_command_visit(&command.right, visits);
        }
        Command::Compound(command) => collect_compound_visits(command, visits),
        Command::Function(FunctionDef { header, body, .. }) => {
            for entry in &header.entries {
                collect_word_visits(&entry.word, visits);
            }
            collect_command_visit(body, visits);
        }
        Command::AnonymousFunction(function) => {
            collect_word_slice_visits(&function.args, visits);
            collect_command_visit(&function.body, visits);
        }
    }

    collect_redirect_visits(&stmt.redirects, visits);
}

fn collect_builtin_visits<'a>(
    command: &'a BuiltinCommand,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignment_visits(&command.assignments, visits);
            if let Some(word) = &command.depth {
                collect_word_visits(word, visits);
            }
            collect_word_slice_visits(&command.extra_args, visits);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignment_visits(&command.assignments, visits);
            if let Some(word) = &command.depth {
                collect_word_visits(word, visits);
            }
            collect_word_slice_visits(&command.extra_args, visits);
        }
        BuiltinCommand::Return(command) => {
            collect_assignment_visits(&command.assignments, visits);
            if let Some(word) = &command.code {
                collect_word_visits(word, visits);
            }
            collect_word_slice_visits(&command.extra_args, visits);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignment_visits(&command.assignments, visits);
            if let Some(word) = &command.code {
                collect_word_visits(word, visits);
            }
            collect_word_slice_visits(&command.extra_args, visits);
        }
    }
}

fn collect_compound_visits<'a>(
    command: &'a CompoundCommand,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_command_visits(&command.condition, visits);
            collect_command_visits(&command.then_branch, visits);
            for (condition, body) in &command.elif_branches {
                collect_command_visits(condition, visits);
                collect_command_visits(body, visits);
            }
            if let Some(body) = &command.else_branch {
                collect_command_visits(body, visits);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_word_slice_visits(words, visits);
            }
            collect_command_visits(&command.body, visits);
        }
        CompoundCommand::Repeat(command) => {
            collect_word_visits(&command.count, visits);
            collect_command_visits(&command.body, visits);
        }
        CompoundCommand::Foreach(command) => {
            collect_word_slice_visits(&command.words, visits);
            collect_command_visits(&command.body, visits);
        }
        CompoundCommand::ArithmeticFor(command) => collect_command_visits(&command.body, visits),
        CompoundCommand::While(command) => {
            collect_command_visits(&command.condition, visits);
            collect_command_visits(&command.body, visits);
        }
        CompoundCommand::Until(command) => {
            collect_command_visits(&command.condition, visits);
            collect_command_visits(&command.body, visits);
        }
        CompoundCommand::Case(command) => {
            collect_word_visits(&command.word, visits);
            for case in &command.cases {
                collect_pattern_slice_visits(&case.patterns, visits);
                collect_command_visits(&case.body, visits);
            }
        }
        CompoundCommand::Select(command) => {
            collect_word_slice_visits(&command.words, visits);
            collect_command_visits(&command.body, visits);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_command_visits(commands, visits);
        }
        CompoundCommand::Always(command) => {
            collect_command_visits(&command.body, visits);
            collect_command_visits(&command.always_body, visits);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command_visit(command, visits);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_visits(&command.expression, visits);
        }
        CompoundCommand::Coproc(command) => collect_command_visit(&command.body, visits),
    }
}

fn collect_assignment_visits<'a>(
    assignments: &'a [Assignment],
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    for assignment in assignments {
        collect_assignment_visit(assignment, visits);
    }
}

fn collect_assignment_visit<'a>(
    assignment: &'a Assignment,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word_visits(word, visits),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => collect_word_visits(word, visits),
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        collect_word_visits(value, visits);
                    }
                }
            }
        }
    }
}

fn collect_word_slice_visits<'a>(words: &'a [Word], visits: &mut Vec<DirectiveCommandVisit<'a>>) {
    for word in words {
        collect_word_visits(word, visits);
    }
}

fn collect_pattern_slice_visits<'a>(
    patterns: &'a [Pattern],
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    for pattern in patterns {
        collect_pattern_visits(pattern, visits);
    }
}

fn collect_pattern_visits<'a>(pattern: &'a Pattern, visits: &mut Vec<DirectiveCommandVisit<'a>>) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => collect_pattern_slice_visits(patterns, visits),
            PatternPart::Word(word) => collect_word_visits(word, visits),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_word_visits<'a>(word: &'a Word, visits: &mut Vec<DirectiveCommandVisit<'a>>) {
    collect_word_part_visits(&word.parts, visits);
}

fn collect_word_part_visits<'a>(
    parts: &'a [WordPartNode],
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    for part in parts {
        match &part.kind {
            WordPart::ZshQualifiedGlob(glob) => {
                for pattern in zsh_glob_patterns(glob) {
                    collect_pattern_visits(pattern, visits);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => collect_word_part_visits(parts, visits),
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    visit_arithmetic_words(expression_ast, &mut |word| {
                        collect_word_visits(word, visits);
                    });
                }
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => collect_command_visits(body, visits),
            WordPart::Parameter(parameter) => collect_parameter_expansion_visits(parameter, visits),
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
                collect_var_ref_word_visits(reference, visits);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, visits);
                }
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => {
                collect_var_ref_word_visits(reference, visits);
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
                collect_var_ref_word_visits(reference, visits);
                if let Some(expression) = offset_ast.as_ref() {
                    visit_arithmetic_words(expression, &mut |word| {
                        collect_word_visits(word, visits);
                    });
                } else {
                    collect_word_visits(offset_word_ast, visits);
                }
                if let Some(expression) = length_ast.as_ref() {
                    visit_arithmetic_words(expression, &mut |word| {
                        collect_word_visits(word, visits);
                    });
                } else if let Some(word) = length_word_ast.as_ref() {
                    collect_word_visits(word, visits);
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. } => {}
        }
    }
}

fn zsh_glob_patterns(glob: &shuck_ast::ZshQualifiedGlob) -> impl Iterator<Item = &Pattern> + '_ {
    glob.segments.iter().filter_map(|segment| match segment {
        ZshGlobSegment::Pattern(pattern) => Some(pattern),
        ZshGlobSegment::InlineControl(_) => None,
    })
}

fn collect_var_ref_word_visits<'a>(
    reference: &'a VarRef,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    let Some(subscript) = reference.subscript.as_deref() else {
        return;
    };
    if subscript.selector().is_some() {
        return;
    }
    if let Some(expression) = subscript.arithmetic_ast.as_ref() {
        visit_arithmetic_words(expression, &mut |word| {
            collect_word_visits(word, visits);
        });
    } else if let Some(word) = subscript.word_ast() {
        collect_word_visits(word, visits);
    }
}

fn collect_parameter_expansion_visits<'a>(
    parameter: &'a ParameterExpansion,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_var_ref_word_visits(reference, visits);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, visits);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, visits);
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, visits);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, visits);
                }
                if let Some(word) = operator.replacement_word_ast() {
                    collect_word_visits(word, visits);
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
                collect_var_ref_word_visits(reference, visits);
                if let Some(expression) = offset_ast.as_ref() {
                    visit_arithmetic_words(expression, &mut |word| {
                        collect_word_visits(word, visits);
                    });
                } else {
                    collect_word_visits(offset_word_ast, visits);
                }
                if let Some(expression) = length_ast.as_ref() {
                    visit_arithmetic_words(expression, &mut |word| {
                        collect_word_visits(word, visits);
                    });
                } else if let Some(word) = length_word_ast.as_ref() {
                    collect_word_visits(word, visits);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_zsh_target_visits(&syntax.target, visits);

            for modifier in &syntax.modifiers {
                if let Some(word) = modifier.argument_word_ast() {
                    collect_word_visits(word, visits);
                }
            }

            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::PatternOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Unknown { .. } => {
                        if let Some(word) = operation.operand_word_ast() {
                            collect_word_visits(word, visits);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::ReplacementOperation { .. } => {
                        if let Some(word) = operation.pattern_word_ast() {
                            collect_word_visits(word, visits);
                        }
                        if let Some(word) = operation.replacement_word_ast() {
                            collect_word_visits(word, visits);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::Slice { .. } => {
                        if let Some(word) = operation.offset_word_ast() {
                            collect_word_visits(word, visits);
                        }
                        if let Some(word) = operation.length_word_ast() {
                            collect_word_visits(word, visits);
                        }
                    }
                }
            }
        }
    }
}

fn collect_zsh_target_visits<'a>(
    target: &'a ZshExpansionTarget,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    match target {
        ZshExpansionTarget::Reference(reference) => collect_var_ref_word_visits(reference, visits),
        ZshExpansionTarget::Nested(parameter) => {
            collect_parameter_expansion_visits(parameter, visits);
        }
        ZshExpansionTarget::Word(word) => collect_word_visits(word, visits),
        ZshExpansionTarget::Empty => {}
    }
}

fn collect_redirect_visits<'a>(
    redirects: &'a [Redirect],
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    for redirect in redirects {
        if let Some(word) = redirect.word_target() {
            collect_word_visits(word, visits);
        } else if let Some(heredoc) = redirect.heredoc()
            && heredoc.delimiter.expands_body
        {
            collect_heredoc_body_part_visits(&heredoc.body.parts, visits);
        }
    }
}

fn collect_heredoc_body_part_visits<'a>(
    parts: &'a [HeredocBodyPartNode],
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    visit_arithmetic_words(expression_ast, &mut |word| {
                        collect_word_visits(word, visits);
                    });
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                collect_command_visits(body, visits);
            }
            shuck_ast::HeredocBodyPart::Literal(_)
            | shuck_ast::HeredocBodyPart::Variable(_)
            | shuck_ast::HeredocBodyPart::Parameter(_) => {}
        }
    }
}

fn collect_conditional_visits<'a>(
    expression: &'a ConditionalExpr,
    visits: &mut Vec<DirectiveCommandVisit<'a>>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_visits(&expr.left, visits);
            collect_conditional_visits(&expr.right, visits);
        }
        ConditionalExpr::Unary(expr) => collect_conditional_visits(&expr.expr, visits),
        ConditionalExpr::Parenthesized(expr) => collect_conditional_visits(&expr.expr, visits),
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_word_visits(word, visits);
        }
        ConditionalExpr::Pattern(pattern) => collect_pattern_visits(pattern, visits),
        ConditionalExpr::VarRef(_) => {}
    }
}

fn visit_arithmetic_words<'a>(
    expression: &'a ArithmeticExprNode,
    visitor: &mut impl FnMut(&'a Word),
) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => visit_arithmetic_words(index, visitor),
        ArithmeticExpr::ShellWord(word) => visitor(word),
        ArithmeticExpr::Parenthesized { expression } => {
            visit_arithmetic_words(expression, visitor);
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visit_arithmetic_words(expr, visitor);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visit_arithmetic_words(left, visitor);
            visit_arithmetic_words(right, visitor);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arithmetic_words(condition, visitor);
            visit_arithmetic_words(then_expr, visitor);
            visit_arithmetic_words(else_expr, visitor);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue_words(target, visitor);
            visit_arithmetic_words(value, visitor);
        }
    }
}

fn visit_arithmetic_lvalue_words<'a>(
    target: &'a ArithmeticLvalue,
    visitor: &mut impl FnMut(&'a Word),
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => visit_arithmetic_words(index, visitor),
    }
}

fn case_label_directive(case: &CaseItem, comment: &NormalizedComment<'_>) -> bool {
    let Some(pattern) = case.patterns.last() else {
        return false;
    };

    let Some(pattern_line) = u32::try_from(pattern.span.end.line).ok() else {
        return false;
    };
    if pattern_line != comment.line || usize::from(comment.range.start()) <= pattern.span.end.offset
    {
        return false;
    }

    case.body
        .first()
        .is_none_or(|stmt| u32::try_from(stmt.span.start.line).ok() != Some(comment.line))
}

fn strip_comment_prefix(text: &str) -> &str {
    text.trim_start().trim_start_matches('#').trim_start()
}

fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = text.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn shellcheck_comment_remainder(comment_text: &str) -> Option<&str> {
    let body = strip_comment_prefix(comment_text);
    let remainder = strip_prefix_ignore_ascii_case(body, "shellcheck")?;
    if let Some(first) = remainder.chars().next()
        && !first.is_ascii_whitespace()
    {
        return None;
    }
    Some(remainder)
}

#[derive(Debug, Clone, Copy)]
struct InlineDirectiveContext<'a> {
    prefix: &'a str,
    line_end: usize,
    comment_start: usize,
}

fn inline_directive_context<'a>(
    source: &'a str,
    comment_range: TextRange,
) -> Option<InlineDirectiveContext<'a>> {
    let comment_start = usize::from(comment_range.start()).min(source.len());
    let line_start = source[..comment_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[comment_start..]
        .find('\n')
        .map_or(source.len(), |index| comment_start + index);

    Some(InlineDirectiveContext {
        prefix: &source[line_start..comment_start],
        line_end,
        comment_start,
    })
}

fn next_command_starts_after_comment_line(
    next_command_start: usize,
    context: &InlineDirectiveContext<'_>,
) -> bool {
    next_command_start >= context.line_end
}

fn command_start_segment_matches(
    source: &str,
    command_start: usize,
    comment_start: usize,
    keyword: &str,
) -> bool {
    segment_ends_with_keyword(source, command_start, comment_start, keyword)
}

fn gap_segment_ends_with_keyword(
    source: &str,
    gap_start: usize,
    comment_start: usize,
    keyword: &str,
) -> bool {
    segment_ends_with_keyword(source, gap_start.min(comment_start), comment_start, keyword)
}

fn gap_segment_ends_with_char(
    source: &str,
    gap_start: usize,
    comment_start: usize,
    ch: char,
) -> bool {
    segment_ends_with_char(source, gap_start.min(comment_start), comment_start, ch)
}

fn segment_ends_with_keyword(source: &str, start: usize, end: usize, keyword: &str) -> bool {
    let Some(segment) = source.get(start.min(end)..end) else {
        return false;
    };
    let trimmed = segment.trim_end_matches([' ', '\t', '\r']);
    let Some(prefix) = trimmed.strip_suffix(keyword) else {
        return false;
    };

    prefix
        .chars()
        .next_back()
        .is_none_or(|ch| ch.is_ascii_whitespace())
}

fn segment_ends_with_char(source: &str, start: usize, end: usize, ch: char) -> bool {
    let Some(segment) = source.get(start.min(end)..end) else {
        return false;
    };

    segment.trim_end_matches([' ', '\t', '\r']).ends_with(ch)
}

fn loop_header_matches(
    source: &str,
    command_start: usize,
    condition: &StmtSeq,
    context: InlineDirectiveContext<'_>,
    keyword: &str,
) -> bool {
    condition.first().is_some_and(|stmt| {
        next_command_starts_after_comment_line(stmt.span.start.offset, &context)
            && command_start_segment_matches(source, command_start, context.comment_start, keyword)
    })
}

fn body_header_matches(
    source: &str,
    header_end: usize,
    body: &StmtSeq,
    context: &InlineDirectiveContext<'_>,
    keyword: &str,
) -> bool {
    body.first().is_some_and(|stmt| {
        next_command_starts_after_comment_line(stmt.span.start.offset, context)
            && gap_segment_ends_with_keyword(source, header_end, context.comment_start, keyword)
    })
}

fn body_opener_matches(
    source: &str,
    header_end: usize,
    body: &StmtSeq,
    context: &InlineDirectiveContext<'_>,
    opener: char,
) -> bool {
    body.first().is_some_and(|stmt| {
        next_command_starts_after_comment_line(stmt.span.start.offset, context)
            && gap_segment_ends_with_char(source, header_end, context.comment_start, opener)
    })
}

fn parse_shuck_action(value: &str) -> Option<SuppressionAction> {
    if value.eq_ignore_ascii_case("disable") {
        Some(SuppressionAction::Disable)
    } else if value.eq_ignore_ascii_case("disable-file") {
        Some(SuppressionAction::DisableFile)
    } else if value.eq_ignore_ascii_case("ignore") {
        Some(SuppressionAction::Ignore)
    } else {
        None
    }
}

fn parse_codes(value: &str, mut resolve: impl FnMut(&str) -> Vec<Rule>) -> Vec<Rule> {
    value
        .split(',')
        .flat_map(|code| {
            let code = code.trim();
            if code.is_empty() {
                Vec::new()
            } else {
                resolve(code)
            }
        })
        .collect()
}

fn resolve_suppression_code(code: &str, shellcheck_map: &ShellCheckCodeMap) -> Vec<Rule> {
    let mut rules = resolve_rule_code(code).into_iter().collect::<Vec<_>>();
    for rule in shellcheck_map.resolve_all(code) {
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    rules
}

fn resolve_rule_code(code: &str) -> Option<Rule> {
    code_to_rule(code).or_else(|| {
        let upper = code.to_ascii_uppercase();
        (upper != code).then(|| code_to_rule(&upper)).flatten()
    })
}

#[cfg(test)]
mod tests {
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect};

    use super::*;

    fn directives(source: &str) -> Vec<SuppressionDirective> {
        directives_with_dialect(source, ShellDialect::Bash)
    }

    fn directives_with_dialect(source: &str, dialect: ShellDialect) -> Vec<SuppressionDirective> {
        let output = Parser::with_dialect(source, dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        parse_directives(
            source,
            &output.file,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        )
    }

    #[test]
    fn parses_shuck_directives_and_strips_reasons() {
        let directives = directives("# shuck: disable=C006,S001 # legacy code\n");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].source, SuppressionSource::Shuck);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UndefinedVariable, Rule::UnquotedExpansion]
        );
        assert_eq!(directives[0].line, 1);
    }

    #[test]
    fn parses_shuck_ignore_directives_on_inline_lines() {
        let directives = directives("echo $foo # shuck: ignore=C006,S001 # legacy\n");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].action, SuppressionAction::Ignore);
        assert_eq!(directives[0].source, SuppressionSource::Shuck);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UndefinedVariable, Rule::UnquotedExpansion]
        );
        assert_eq!(directives[0].line, 1);
    }

    #[test]
    fn parses_dead_code_rule_aliases() {
        let directives = directives(
            "\
# shuck: disable=C124
# shuck: disable=SH-293
",
        );

        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].codes, vec![Rule::UnreachableAfterExit]);
        assert_eq!(directives[1].codes, vec![Rule::UnreachableAfterExit]);
    }

    #[test]
    fn parses_shellcheck_codes_in_shuck_directives() {
        let directives = directives(
            "\
# shuck: disable=SC2086,2154
# shuck: disable-file=SC2268
",
        );

        assert_eq!(directives.len(), 2);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UnquotedExpansion, Rule::UndefinedVariable]
        );
        assert_eq!(directives[1].action, SuppressionAction::DisableFile);
        assert_eq!(
            directives[1].codes,
            vec![Rule::XPrefixInTest, Rule::BackslashBeforeCommand]
        );
    }

    #[test]
    fn parses_shellcheck_directives_on_own_line_and_after_code() {
        let source = "\
# shellcheck disable=SC2086 disable=SC2034 disable=SC7777
echo $foo
case $x in
  on) # shellcheck disable=SC2034
    value=1
    ;;
esac
";
        let directives = directives(source);

        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UnquotedExpansion, Rule::UnusedAssignment]
        );
        assert_eq!(directives[1].action, SuppressionAction::Disable);
        assert_eq!(directives[1].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[1].codes, vec![Rule::UnusedAssignment]);
    }

    #[test]
    fn parses_shuck_codes_in_shellcheck_directives() {
        let directives = directives("# shellcheck disable=S001,SH-039,C124\n");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(
            directives[0].codes,
            vec![
                Rule::UnquotedExpansion,
                Rule::UndefinedVariable,
                Rule::UnreachableAfterExit,
            ]
        );
    }

    #[test]
    fn rejects_shellcheck_directives_after_regular_code() {
        let source = "\
value=1 # shellcheck disable=SC2086
echo $foo
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn rejects_shuck_disable_directives_after_regular_code() {
        let source = "\
value=1 # shuck: disable=C006
echo $foo
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn rejects_case_label_directives_after_same_line_commands() {
        let source = "\
case $x in
  on) echo \"$x\" # shellcheck disable=SC2086
      ;;
esac
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn parses_shuck_disable_directives_after_control_flow_headers_and_group_openers() {
        let source = "\
if # shuck: disable=SC2086
  echo $foo
then
  :
fi
if true; then { # shuck: disable=SC2086
  echo $foo
}; fi
while # shuck: disable=SC2086
  echo $foo
do
  :
done
{ # shuck: disable=SC2086
  echo $foo
}
";
        let directives = directives(source);

        assert_eq!(directives.len(), 4);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::Shuck
                && directive.action == SuppressionAction::Disable
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shuck_disable_directives_after_case_labels() {
        let source = "\
case $x in
  on) # shuck: disable=SC2086
    echo $foo
    ;;
esac
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::Shuck);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
    }

    #[test]
    fn rejects_shellcheck_directives_after_keyword_like_arguments() {
        let source = "\
echo if # shellcheck disable=SC2086
echo $foo
echo { # shellcheck disable=SC2086
echo $bar
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn parses_shellcheck_directives_after_control_flow_headers_and_group_openers() {
        let source = "\
if # shellcheck disable=SC2086
  echo $foo
then # shellcheck disable=SC2086
  echo $foo
elif # shellcheck disable=SC2086
  echo $bar
then
  :
else # shellcheck disable=SC2086
  echo $baz
fi
while # shellcheck disable=SC2086
  echo $foo
do # shellcheck disable=SC2086
  echo $foo
done
until # shellcheck disable=SC2086
  echo $foo
do
  :
done
{ # shellcheck disable=SC2086
  echo $foo
}
( # shellcheck disable=SC2086
  echo $foo
)
";
        let directives = directives(source);

        assert_eq!(directives.len(), 9);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::ShellCheck
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shellcheck_directives_after_elif_then_header() {
        let source = "\
if false; then
  :
elif true; then # shellcheck disable=SC2086
  echo $foo
fi
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 3);
    }

    #[test]
    fn parses_shellcheck_directives_after_for_do_header() {
        let source = "\
for item in 1; do # shellcheck disable=SC2086
  echo $foo
done
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 1);
    }

    #[test]
    fn rejects_shellcheck_directives_after_keyword_suffixes_inside_words() {
        let source = "\
for item in to-do # shellcheck disable=SC2086
do
  echo $foo
done
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn parses_shellcheck_directives_after_then_inline_group_openers() {
        let source = "\
if true; then { # shellcheck disable=SC2086
  echo $foo
}; fi
if true; then ( # shellcheck disable=SC2086
  echo $bar
); fi
";
        let directives = directives(source);

        assert_eq!(directives.len(), 2);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::ShellCheck
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shellcheck_directives_after_zsh_brace_if_headers() {
        let source = "\
if [[ -n $foo ]] { # shellcheck disable=SC2086
  echo $foo
} elif [[ -n $bar ]] { # shellcheck disable=SC2086
  echo $bar
} else { # shellcheck disable=SC2086
  echo $baz
}
";
        let directives = directives_with_dialect(source, ShellDialect::Zsh);

        assert_eq!(directives.len(), 3);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::ShellCheck
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shellcheck_directives_after_zsh_brace_loop_headers() {
        let source = "\
for item in 1; { # shellcheck disable=SC2086
  echo $foo
}
repeat 2 { # shellcheck disable=SC2086
  echo $bar
}
foreach item (1 2) { # shellcheck disable=SC2086
  echo $baz
}
";
        let directives = directives_with_dialect(source, ShellDialect::Zsh);

        assert_eq!(directives.len(), 3);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::ShellCheck
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shellcheck_disable_all() {
        let directives = directives("# shellcheck disable=all\n");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes.len(), Rule::COUNT);
        assert!(directives[0].codes.contains(&Rule::CompoundTestOperator));
        assert!(directives[0].codes.contains(&Rule::UnquotedExpansion));
    }

    #[test]
    fn ignores_malformed_and_unknown_directives() {
        let source = "\
# shuck: disable=
# shuck: ignore=
# shuck: foobar=C001
# shuck disable=C001
# shuck: enable=SH-039
# shuck: disable=SH-039
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].codes, vec![Rule::UndefinedVariable]);
    }

    #[test]
    fn parses_shellcheck_directives_inside_command_substitutions() {
        let source = "\
value=\"$(
  # shellcheck disable=SC2086
  echo $foo
)\"
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 2);
    }

    #[test]
    fn parses_case_label_directives_inside_command_substitution_arguments() {
        let source = "\
printf '%s\\n' \"$(
  case $x in
    on) # shellcheck disable=SC2086
      echo $foo
      ;;
  esac
)\"\n";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 3);
    }
}
