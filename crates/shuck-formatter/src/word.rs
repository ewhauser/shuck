use std::fmt::Write as _;

use shuck_ast::{
    ArithmeticAssignOp, ArithmeticBinaryOp, ArithmeticExpansionSyntax, ArithmeticExpr,
    ArithmeticExprNode, ArithmeticLvalue, ArithmeticPostfixOp, ArithmeticUnaryOp, Assignment,
    AssignmentValue, BinaryOp, BourneParameterExpansion, BuiltinCommand, Command,
    CommandSubstitutionSyntax, CompoundCommand, ConditionalExpr, HeredocBody, HeredocBodyPart,
    ParameterOp, Pattern, PatternPart, Redirect, Stmt, StmtSeq, SubscriptSelector, VarRef, Word,
    WordPart, WordPartNode,
};
use shuck_format::IndentStyle;

use crate::command::{
    array_elem_parts, builtin_like_parts, compound_contains_child, stmt_seq_has_heredoc,
    trim_unescaped_trailing_whitespace,
};
use crate::comments::SourceMap;
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::scan::{
    common_nonempty_shell_indent, heredoc_start, leading_shell_indent as line_leading_shell_indent,
    line_indent_before_offset as line_indent_before_source_offset,
    line_without_continuation_backslash, redirect_operator_end, refine_common_indent,
    shell_comment_can_start,
};
use crate::streaming::format_stmt_sequence_streaming_to_buf;

pub(crate) fn word_gap_end_before_trailing_continuation(word: &Word, source: &str) -> usize {
    let span_end = word.span.end.offset;
    let part_end = word
        .parts
        .iter()
        .map(|part| part.span.end.offset)
        .max()
        .unwrap_or(span_end);
    if part_end >= span_end {
        return span_end;
    }
    let Some(last_part) = word.parts.iter().max_by_key(|part| part.span.end.offset) else {
        return span_end;
    };
    if !matches!(
        last_part.kind,
        WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
    ) {
        return span_end;
    }
    let Some(trailing) = source.get(part_end..span_end) else {
        return span_end;
    };
    if source_fragment_is_line_continuation_padding(trailing) {
        part_end
    } else {
        span_end
    }
}

fn source_fragment_is_line_continuation_padding(fragment: &str) -> bool {
    let fragment = fragment.trim_start_matches([' ', '\t']);
    let Some(after_backslash) = fragment.strip_prefix('\\') else {
        return false;
    };
    let Some(after_newline) = after_backslash
        .strip_prefix("\r\n")
        .or_else(|| after_backslash.strip_prefix('\n'))
    else {
        return false;
    };
    after_newline.chars().all(|ch| matches!(ch, ' ' | '\t'))
}

fn word_part_nodes_any(
    parts: &[WordPartNode],
    predicate: &mut impl FnMut(&WordPartNode) -> bool,
) -> bool {
    parts.iter().any(|part| {
        predicate(part)
            || matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if word_part_nodes_any(parts.as_slice(), predicate)
            )
    })
}

pub(crate) fn render_word_syntax(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let mut rendered = String::new();
    render_word_syntax_to_buf(word, source, options, &mut rendered);
    rendered
}

pub(crate) fn render_word_syntax_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    rendered: &mut String,
) {
    render_word_syntax_internal(word, source, options, None, None, true, rendered);
}

pub(crate) fn render_word_syntax_with_facts_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
    rendered: &mut String,
) {
    let source_map = Some(source_map);
    let facts = Some(facts);
    render_word_syntax_internal(word, source, options, source_map, facts, true, rendered);
}

pub(crate) fn render_escaped_multiline_word_syntax_with_facts_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
    rendered: &mut String,
) {
    let source_map = Some(source_map);
    let facts = Some(facts);
    render_word_syntax_internal(word, source, options, source_map, facts, false, rendered);
}

pub(crate) fn word_has_multiline_literal_source(word: &Word, source: &str) -> bool {
    if raw_word_source_slice(word, source).is_some_and(|raw| {
        raw.contains("\\\n")
            && word_has_multiline_double_quoted_source(word, source)
            && !word_is_quoted_command_substitution_only(word)
    }) {
        return true;
    }

    word_part_nodes_any(&word.parts, &mut |part| {
        word_part_has_multiline_literal_source(&part.kind, part.span, source)
    })
}

fn word_part_has_multiline_literal_source(
    part: &WordPart,
    span: shuck_ast::Span,
    source: &str,
) -> bool {
    match part {
        WordPart::Literal(text) => text.as_str(source, span).contains('\n'),
        WordPart::SingleQuoted { value, dollar } => {
            if *dollar {
                raw_source_slice(span, source).is_some_and(|raw| raw.contains('\n'))
            } else {
                value.slice(source).contains('\n')
            }
        }
        WordPart::CommandSubstitution { body, .. } => {
            stmt_seq_has_multiline_literal_source(body, source)
                || (stmt_seq_contains_comments(body)
                    && raw_source_slice(span, source).is_some_and(|raw| {
                        raw.contains('\n')
                            && !command_substitution_source_starts_with_body_line(raw)
                    }))
        }
        WordPart::ProcessSubstitution { body, .. } => {
            stmt_seq_has_multiline_literal_source(body, source)
                || (stmt_seq_contains_comments(body)
                    && raw_source_slice(span, source).is_some_and(|raw| raw.contains('\n')))
        }
        _ => false,
    }
}

fn stmt_seq_has_multiline_literal_source(sequence: &StmtSeq, source: &str) -> bool {
    sequence
        .iter()
        .any(|stmt| stmt_has_multiline_literal_source(stmt, source))
}

fn stmt_has_multiline_literal_source(stmt: &Stmt, source: &str) -> bool {
    command_has_multiline_literal_source(&stmt.command, source)
        || stmt
            .redirects
            .iter()
            .any(|redirect| redirect_has_multiline_literal_source(redirect, source))
}

fn words_have_multiline_literal_source(words: &[Word], source: &str) -> bool {
    words
        .iter()
        .any(|word| word_has_multiline_literal_source(word, source))
}

fn assignments_have_multiline_literal_source(assignments: &[Assignment], source: &str) -> bool {
    assignments
        .iter()
        .any(|assignment| assignment_has_multiline_literal_source(assignment, source))
}

fn command_has_multiline_literal_source(command: &Command, source: &str) -> bool {
    match command {
        Command::Simple(command) => {
            word_has_multiline_literal_source(&command.name, source)
                || words_have_multiline_literal_source(&command.args, source)
                || assignments_have_multiline_literal_source(&command.assignments, source)
        }
        Command::Builtin(command) => builtin_has_multiline_literal_source(command, source),
        Command::Decl(command) => {
            assignments_have_multiline_literal_source(&command.assignments, source)
                || command.operands.iter().any(|operand| match operand {
                    shuck_ast::DeclOperand::Flag(word) | shuck_ast::DeclOperand::Dynamic(word) => {
                        word_has_multiline_literal_source(word, source)
                    }
                    shuck_ast::DeclOperand::Assignment(assignment) => {
                        assignment_has_multiline_literal_source(assignment, source)
                    }
                    shuck_ast::DeclOperand::Name(_) => false,
                })
        }
        Command::Binary(command) => {
            stmt_has_multiline_literal_source(&command.left, source)
                || stmt_has_multiline_literal_source(&command.right, source)
        }
        Command::Compound(command) => compound_has_multiline_literal_source(command, source),
        Command::Function(command) => stmt_has_multiline_literal_source(&command.body, source),
        Command::AnonymousFunction(command) => {
            words_have_multiline_literal_source(&command.args, source)
                || stmt_has_multiline_literal_source(&command.body, source)
        }
    }
}

fn builtin_has_multiline_literal_source(command: &BuiltinCommand, source: &str) -> bool {
    let (_, _, assignments, primary, extra_args) = builtin_like_parts(command);
    builtin_args_have_multiline_literal_source(primary, extra_args, assignments, source)
}

fn builtin_args_have_multiline_literal_source(
    primary: Option<&Word>,
    extra_args: &[Word],
    assignments: &[Assignment],
    source: &str,
) -> bool {
    primary.is_some_and(|word| word_has_multiline_literal_source(word, source))
        || words_have_multiline_literal_source(extra_args, source)
        || assignments_have_multiline_literal_source(assignments, source)
}

fn compound_has_multiline_literal_source(command: &CompoundCommand, source: &str) -> bool {
    compound_words_have_multiline_literal_source(command, source)
        || compound_contains_child(
            command,
            |stmt| stmt_has_multiline_literal_source(stmt, source),
            |sequence| stmt_seq_has_multiline_literal_source(sequence, source),
        )
}

fn compound_words_have_multiline_literal_source(command: &CompoundCommand, source: &str) -> bool {
    match command {
        CompoundCommand::For(command) => {
            command
                .targets
                .iter()
                .any(|target| word_has_multiline_literal_source(&target.word, source))
                || command
                    .words
                    .as_deref()
                    .is_some_and(|words| words_have_multiline_literal_source(words, source))
        }
        CompoundCommand::Repeat(command) => {
            word_has_multiline_literal_source(&command.count, source)
        }
        CompoundCommand::Foreach(command) => {
            words_have_multiline_literal_source(&command.words, source)
        }
        CompoundCommand::Case(command) => word_has_multiline_literal_source(&command.word, source),
        CompoundCommand::Select(command) => {
            words_have_multiline_literal_source(&command.words, source)
        }
        CompoundCommand::Conditional(command) => {
            conditional_expr_has_multiline_literal_source(&command.expression, source)
        }
        _ => false,
    }
}

fn conditional_expr_has_multiline_literal_source(expr: &ConditionalExpr, source: &str) -> bool {
    match expr {
        ConditionalExpr::Binary(expr) => {
            conditional_expr_has_multiline_literal_source(&expr.left, source)
                || conditional_expr_has_multiline_literal_source(&expr.right, source)
        }
        ConditionalExpr::Unary(expr) => {
            conditional_expr_has_multiline_literal_source(&expr.expr, source)
        }
        ConditionalExpr::Parenthesized(expr) => {
            conditional_expr_has_multiline_literal_source(&expr.expr, source)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            word_has_multiline_literal_source(word, source)
        }
        ConditionalExpr::Pattern(_) | ConditionalExpr::VarRef(_) => false,
    }
}

fn redirect_has_multiline_literal_source(redirect: &Redirect, source: &str) -> bool {
    redirect
        .word_target()
        .is_some_and(|word| word_has_multiline_literal_source(word, source))
        || redirect.heredoc().is_some_and(|heredoc| {
            word_has_multiline_literal_source(&heredoc.delimiter.raw, source)
        })
}

fn assignment_has_multiline_literal_source(assignment: &Assignment, source: &str) -> bool {
    assignment_value_has_multiline_literal_source(assignment, source)
        || matches!(&assignment.value, AssignmentValue::Scalar(_))
            && assignment_has_raw_backslash_continuation_literal(assignment, source)
}

pub(crate) fn assignment_value_has_multiline_literal_source(
    assignment: &Assignment,
    source: &str,
) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_has_multiline_literal_source(word, source),
        AssignmentValue::Compound(array) => array
            .elements
            .iter()
            .any(|element| word_has_multiline_literal_source(array_elem_parts(element).1, source)),
    }
}

fn assignment_has_raw_backslash_continuation_literal(
    assignment: &Assignment,
    source: &str,
) -> bool {
    let raw = assignment.span.slice(source);
    raw.contains("\\\n")
        && !raw.contains("$(")
        && !raw.contains('`')
        && !raw.contains("<(")
        && !raw.contains(">(")
}

pub(crate) fn render_heredoc_body_to_buf(
    body: &HeredocBody,
    source: &str,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
    embedded_command_indent_levels: usize,
    rendered: &mut String,
) {
    for part in &body.parts {
        if render_heredoc_body_part(
            rendered,
            &part.kind,
            part.span,
            source,
            options,
            facts,
            embedded_command_indent_levels,
        )
        .is_err()
        {
            unreachable!("writing into a String should not fail");
        }
    }
}

fn render_word_syntax_internal(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    preserve_escaped_multiline_words: bool,
    rendered: &mut String,
) {
    let preserve_raw = !options.simplify() && !options.minify();

    if preserve_raw
        && !word_is_single_quoted_only(word)
        && let Some(raw) = raw_word_source_slice(word, source)
        && let Some(normalized) = normalize_raw_command_substitution_padding(raw)
    {
        let normalized = normalize_raw_arithmetic_command_substitution_padding(&normalized)
            .or_else(|| normalize_raw_arithmetic_expansion_padding(&normalized))
            .unwrap_or(normalized);
        if !raw_command_substitution_needs_structural_spacing(&normalized) {
            push_raw_shell_text_with_normalized_redirect_spacing(rendered, &normalized);
            return;
        }
    }

    if preserve_escaped_multiline_words
        && word_has_escaped_command_substitution(word, source)
        && let Some(raw) = raw_word_source_slice(word, source)
    {
        rendered.push_str(raw);
        return;
    }

    if preserve_raw
        && let Some(raw) = raw_word_source_slice(word, source)
        && raw_single_line_escaped_quote_command_substitution_should_preserve(raw)
    {
        rendered.push_str(raw);
        return;
    }

    if preserve_raw
        && let Some(raw) = raw_word_source_slice(word, source)
        && let Some(normalized) = normalize_raw_compound_assignment_word_continuations(raw)
    {
        rendered.push_str(&normalized);
        return;
    }

    if preserve_raw
        && !word_needs_special_rendering(word)
        && let Some(raw) = raw_word_source_slice(word, source)
        && let Some(normalized) = normalize_raw_unquoted_word_continuations(raw)
    {
        rendered.push_str(&normalized);
        return;
    }

    if preserve_raw
        && let Some(raw) = raw_word_source_slice(word, source)
        && let Some(normalized) = normalize_raw_empty_parameter_replacement_delimiters(raw)
    {
        rendered.push_str(&normalized);
        return;
    }

    if preserve_raw
        && let Some(raw) = raw_word_source_slice(word, source)
        && (word_has_multiline_double_quoted_source(word, source)
            || (raw.starts_with('"') && raw.contains("\\\n")))
        && !word_is_quoted_formattable_command_substitution_only(word, source)
        && (preserve_escaped_multiline_words || !raw_escaped_multiline_double_quoted_word(raw))
        && could_need_preserve_raw_syntax(raw)
    {
        push_raw_word_with_normalized_command_redirect_spacing(
            rendered, word, raw, source, options,
        );
        return;
    }

    if word_needs_formatter_rendering(word, source, options) {
        let start = rendered.len();
        let env = WordRenderEnv::new(source, options, source_map, facts);
        if render_word_parts(
            word.parts.as_slice(),
            env,
            preserve_escaped_multiline_words,
            rendered,
        )
        .is_err()
        {
            unreachable!("writing into a String should not fail");
        }
        if preserve_raw
            && let Some(slice) = raw_word_source_slice(word, source)
            && should_preserve_special_rendered_raw_syntax(slice, &rendered[start..])
        {
            rendered.truncate(start);
            push_preserved_raw_word_source(rendered, word, slice, source, options);
        }
        return;
    }

    if preserve_raw
        && let Some(slice) = raw_word_source_slice(word, source)
        && could_need_preserve_raw_syntax(slice)
    {
        let start = rendered.len();
        word.render_syntax_to_buf(source, rendered);
        if should_preserve_raw_syntax(slice, &rendered[start..]) {
            rendered.truncate(start);
            push_preserved_raw_word_source(rendered, word, slice, source, options);
        }
        return;
    }

    word.render_syntax_to_buf(source, rendered);
}

/// Returns `true` when a word contains a command-substitution node whose raw
/// source was escaped in the original word, indicating the parser
/// misinterpreted a literal prompt fragment as a command-substitution delimiter.
/// In that case the word's raw source must be preserved verbatim.
fn word_has_escaped_command_substitution(word: &Word, source: &str) -> bool {
    if raw_word_source_slice(word, source)
        .is_some_and(|raw| raw.contains("\\$(") || raw.contains("\\`"))
    {
        return true;
    }

    word_part_nodes_any(&word.parts, &mut |part| {
        word_part_has_escaped_command_substitution(&part.kind, part.span, source)
    })
}

fn word_part_has_escaped_command_substitution(
    part: &WordPart,
    span: shuck_ast::Span,
    source: &str,
) -> bool {
    match part {
        WordPart::CommandSubstitution { syntax, .. } => match syntax {
            CommandSubstitutionSyntax::Backtick => {
                raw_source_slice(span, source).is_some_and(|raw| raw.starts_with('\\'))
            }
            CommandSubstitutionSyntax::DollarParen => {
                raw_source_slice(span, source).is_some_and(|raw| raw.starts_with("\\$("))
                    || source
                        .get(..span.start.offset)
                        .is_some_and(|prefix| prefix.ends_with('\\'))
            }
        },
        _ => false,
    }
}

fn raw_escaped_multiline_double_quoted_word(raw: &str) -> bool {
    raw.strip_prefix('$').unwrap_or(raw).starts_with("\"\\\n")
        || raw.strip_prefix('$').unwrap_or(raw).starts_with("\"\\\r\n")
}

fn raw_single_line_escaped_quote_command_substitution_should_preserve(raw: &str) -> bool {
    let Some(escaped_quote) = raw.find("\\\"") else {
        return false;
    };
    let Some(command_substitution) = raw.find("$(") else {
        return false;
    };

    !raw.contains('\n')
        && raw.starts_with('"')
        && raw.ends_with('"')
        && escaped_quote < command_substitution
}

fn word_needs_special_rendering(word: &Word) -> bool {
    word_part_nodes_any(&word.parts, &mut |part| {
        part_needs_special_rendering(&part.kind)
    })
}

fn word_needs_formatter_rendering(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    word_part_nodes_any(&word.parts, &mut |part| {
        word_part_needs_formatter_rendering(part, source, options)
    })
}

fn word_part_needs_formatter_rendering(
    part: &WordPartNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    part_needs_special_rendering(&part.kind)
        || word_part_has_parameter_raw_subscript_needs_compaction(&part.kind, source, options)
        || word_part_has_parameter_command_redirect_spacing_needs_normalization(
            &part.kind, part.span, source,
        )
        || word_part_has_arithmetic_expansion_source_needs_trim(&part.kind, source)
}

fn word_part_has_parameter_raw_subscript_needs_compaction(
    part: &WordPart,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    match part {
        WordPart::Parameter(parameter) => {
            parameter_raw_subscript_needs_compaction(parameter, source, options)
        }
        _ => false,
    }
}

fn word_part_has_parameter_command_redirect_spacing_needs_normalization(
    part: &WordPart,
    span: shuck_ast::Span,
    source: &str,
) -> bool {
    match part {
        WordPart::Parameter(_) | WordPart::ParameterExpansion { .. } => {
            raw_source_slice(span, source).is_some_and(raw_parameter_command_spacing_would_change)
        }
        _ => false,
    }
}

fn word_part_has_arithmetic_expansion_source_needs_trim(part: &WordPart, source: &str) -> bool {
    match part {
        WordPart::ArithmeticExpansion { expression, .. } => {
            let raw = expression.slice(source);
            raw.trim_matches([' ', '\t', '\r']).len() != raw.len()
        }
        _ => false,
    }
}

fn word_has_multiline_double_quoted_source(word: &Word, source: &str) -> bool {
    word_part_nodes_any(&word.parts, &mut |part| {
        matches!(&part.kind, WordPart::DoubleQuoted { .. })
            && raw_source_slice(part.span, source).is_some_and(|raw| raw.contains('\n'))
    })
}

pub(crate) fn word_is_quoted_formattable_command_substitution_only(
    word: &Word,
    source: &str,
) -> bool {
    quoted_command_substitution_only_body(word)
        .is_some_and(|body| !stmt_seq_has_multiline_literal_source(body, source))
}

pub(crate) fn word_is_quoted_command_substitution_only(word: &Word) -> bool {
    quoted_command_substitution_only_body(word).is_some()
}

fn quoted_command_substitution_only_body(word: &Word) -> Option<&StmtSeq> {
    let [
        shuck_ast::WordPartNode {
            kind:
                WordPart::DoubleQuoted {
                    parts,
                    dollar: false,
                },
            ..
        },
    ] = word.parts.as_slice()
    else {
        return None;
    };

    let mut substitution_body = None;
    for part in parts {
        match &part.kind {
            WordPart::CommandSubstitution { body, .. } if substitution_body.is_none() => {
                substitution_body = Some(body);
            }
            WordPart::Literal(text) if text.is_empty() => {}
            _ => return None,
        }
    }

    substitution_body
}

fn part_needs_special_rendering(part: &WordPart) -> bool {
    match part {
        WordPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_some(),
        WordPart::Parameter(parameter) => parameter_needs_special_rendering(parameter),
        WordPart::ParameterExpansion { operator, .. } => matches!(
            operator.as_ref(),
            ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
        ),
        WordPart::SingleQuoted { dollar: true, .. }
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ProcessSubstitution { .. } => true,
        _ => false,
    }
}

fn render_word_parts(
    parts: &[shuck_ast::WordPartNode],
    env: WordRenderEnv<'_, '_>,
    allow_source_indented_inline_command_substitution: bool,
    rendered: &mut String,
) -> Result<(), std::fmt::Error> {
    for part in parts {
        render_word_part(
            rendered,
            &part.kind,
            part.span,
            env,
            WordPartRenderContext {
                allow_source_indented_inline_command_substitution,
                ..WordPartRenderContext::default()
            },
        )?;
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy)]
struct WordPartRenderContext {
    allow_source_indented_inline_command_substitution: bool,
    source_indented_inline_command_substitution: bool,
}

fn render_heredoc_body_part(
    rendered: &mut String,
    part: &HeredocBodyPart,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
    embedded_command_indent_levels: usize,
) -> Result<(), std::fmt::Error> {
    match part {
        HeredocBodyPart::Literal(text) => {
            let raw = span.slice(source);
            let cooked = text.as_str(source, span);
            if raw != cooked && (raw.contains("\\$") || raw.contains("\\`") || raw.contains("\\\\"))
            {
                rendered.push_str(raw);
            } else {
                rendered.push_str(cooked);
            }
        }
        HeredocBodyPart::Variable(name) => {
            if let Some(raw) = escaped_heredoc_expansion_source(span, source) {
                rendered.push_str(raw);
            } else {
                std::write!(rendered, "${name}")?;
            }
        }
        HeredocBodyPart::CommandSubstitution { body, syntax } => {
            let raw = raw_source_slice(span, source);
            if let Some(raw) = escaped_heredoc_expansion_source(span, source) {
                rendered.push_str(raw);
            } else if render_heredoc_body_command_substitution(
                rendered,
                body,
                span.end.offset,
                source,
                options,
                raw,
            )
            .is_none()
            {
                let layout = command_substitution_layout(
                    raw,
                    body,
                    source,
                    options.dialect(),
                    raw.is_none() && *syntax == CommandSubstitutionSyntax::DollarParen,
                    false,
                );

                if render_command_substitution(
                    rendered,
                    body,
                    span.end.offset,
                    source,
                    options,
                    layout,
                    embedded_command_indent_levels,
                    raw,
                    Some(facts.source_map()),
                    Some(facts),
                )
                .is_none()
                {
                    if let Some(raw) = raw {
                        rendered.push_str(raw);
                    } else {
                        std::write!(rendered, "$({body:?})")?;
                    }
                }
            }
        }
        HeredocBodyPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => {
            if let Some(raw) = escaped_heredoc_expansion_source(span, source) {
                rendered.push_str(raw);
            } else {
                push_arithmetic_expansion(
                    rendered,
                    expression,
                    expression_ast.as_ref(),
                    *syntax,
                    WordRenderEnv::new(source, options, Some(facts.source_map()), Some(facts)),
                );
            }
        }
        HeredocBodyPart::Parameter(parameter) => {
            if let Some(raw) = escaped_heredoc_expansion_source(span, source) {
                rendered.push_str(raw);
            } else {
                push_parameter_word(rendered, parameter, source, options)?;
            }
        }
    }

    Ok(())
}

fn escaped_heredoc_expansion_source(span: shuck_ast::Span, source: &str) -> Option<&str> {
    let raw = span.slice(source);
    if raw.starts_with(['\\', '\x00']) {
        return Some(raw);
    }

    let start = span.start.offset;
    if start > 0
        && source
            .as_bytes()
            .get(start - 1)
            .is_some_and(|byte| *byte == b'\\')
    {
        return source.get(start - 1..span.end.offset);
    }

    None
}

fn push_raw_word_with_normalized_command_redirect_spacing(
    rendered: &mut String,
    word: &Word,
    raw: &str,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let mut spans = Vec::new();
    collect_raw_command_substitution_spans(word.parts.as_slice(), &mut spans);
    spans.sort_by_key(|span| span.start.offset);
    let mut cursor = word.span.start.offset;
    let word_end = word.span.end.offset.min(source.len());
    let mut wrote_span = false;
    for span in spans {
        let start = span.start.offset;
        let end = span.end.offset;
        if start < cursor || end > word_end || start >= end {
            continue;
        }
        if let Some(prefix) = source.get(cursor..start) {
            rendered.push_str(prefix);
        }
        if let Some(command) = source.get(start..end) {
            push_raw_command_substitution_with_normalized_spacing(
                rendered, command, source, start, options,
            );
            wrote_span = true;
        }
        cursor = end;
    }
    if wrote_span {
        if let Some(suffix) = source.get(cursor..word_end) {
            rendered.push_str(suffix);
        }
    } else {
        rendered.push_str(raw);
    }
}

fn push_raw_command_substitution_with_normalized_spacing(
    target: &mut String,
    raw: &str,
    source: &str,
    start_offset: usize,
    options: &ResolvedShellFormatOptions,
) {
    if let Some(normalized) = normalize_raw_backtick_command_substitution(raw) {
        target.push_str(&normalized);
        return;
    }
    if !raw.contains('\n') {
        push_raw_shell_text_with_normalized_redirect_spacing(target, raw);
        return;
    }
    let normalized_pipeline = normalize_raw_pipeline_continuations(raw);
    let raw = normalized_pipeline.as_deref().unwrap_or(raw);
    let normalized_close_continuations =
        normalize_continuations_before_substitution_close_lines(raw);
    let raw = normalized_close_continuations.as_deref().unwrap_or(raw);
    let outer_indent = line_indent_before_source_offset(source, start_offset).unwrap_or("");
    let mut quote = RawShellQuoteState::default();
    let raw_lines = raw.split('\n').collect::<Vec<_>>();
    let Some((first, lines)) = raw_lines.split_first() else {
        return;
    };
    target.push_str(first);
    quote.scan_line(first);
    let mut previous_pipeline_indent: Option<String> = None;
    let mut continuation_pipeline_stage_indent: Option<String> = None;
    let mut compound_indents = RawCompoundIndentState::default();
    let outer_shell_indent = normalized_raw_shell_indent(outer_indent, options);
    let mut continuation_indent: Option<String> = line_without_continuation_backslash(first)
        .and_then(|continued| {
            let starts_command_substitution =
                first.trim_start_matches([' ', '\t']).starts_with("$(");
            (starts_command_substitution && !continued.contains(')'))
                .then(|| source_indent_plus_one_unit(&outer_shell_indent, options))
        });
    let mut literal_exit_continuation_indent: Option<String> = None;
    for (line_index, line) in lines.iter().enumerate() {
        let line = *line;
        target.push('\n');
        if quote.in_multiline_literal() {
            let line_continues = line_without_continuation_backslash(line).is_some();
            if let Some(previous_indent) = continuation_indent.as_deref() {
                let stripped = line
                    .strip_prefix(outer_indent)
                    .unwrap_or_else(|| strip_one_indent_unit(line, options));
                let content = stripped.trim_start_matches([' ', '\t']);
                target.push_str(previous_indent);
                target.push_str(content);
            } else {
                target.push_str(line);
            }
            quote.scan_line(line);
            continuation_indent = if line_continues {
                if quote.in_multiline_literal() {
                    continuation_indent.clone()
                } else {
                    continuation_indent
                        .clone()
                        .or_else(|| literal_exit_continuation_indent.take())
                        .or_else(|| Some(source_indent_plus_one_unit("", options)))
                }
            } else {
                if !quote.in_multiline_literal() {
                    literal_exit_continuation_indent = None;
                }
                None
            };
            continue;
        } else {
            let mut line = strip_outer_indent_or_one_unit(line, outer_indent, options).to_string();
            let source_indent_for_compound_shift = line_leading_shell_indent(&line).to_string();
            if let Some(shifted) = compound_indents.shifted_line(&line, options) {
                line = shifted;
            }
            let (indent, content) = raw_line_parts(&line);
            let carried_pipeline_indent = previous_pipeline_indent.clone();
            if let Some(previous_indent) = carried_pipeline_indent.as_deref()
                && !content.trim().is_empty()
                && raw_indent_units(indent, options) < raw_indent_units(previous_indent, options)
            {
                line = format!("{previous_indent}{content}");
            }
            let (indent, content) = raw_line_parts(&line);
            let closes_substitution_wrapper = raw_line_closes_substitution_wrapper(content)
                && raw_block_line_is_outer_substitution_close(lines, line_index);
            if let Some(previous_indent) = continuation_indent.as_deref()
                && !content.trim().is_empty()
                && !content.starts_with('#')
                && !closes_substitution_wrapper
                && normalized_raw_shell_indent(indent, options) != previous_indent
            {
                line = format!("{previous_indent}{content}");
            }
            let (indent, content) = raw_line_parts(&line);
            if let Some(child_indent) =
                compound_indents.child_indent_if_underindented(indent, content, options)
            {
                line = format!("{child_indent}{content}");
            }
            let (indent, content) = raw_line_parts(&line);
            let used_continuation_indent = continuation_indent.is_some();
            let rendered_indent = if closes_substitution_wrapper {
                push_raw_shell_line_with_rendered_indent(target, &line, options, "");
                String::new()
            } else {
                push_raw_shell_line_with_normalized_source_indent(target, &line, options, None);
                rendered_raw_shell_indent_for_line(indent, content, None, options)
            };
            let line_closes_pipeline_stage_compound =
                compound_indents.closes_pipeline_stage(content);
            let line_is_pipeline_continuation_stage = carried_pipeline_indent.is_some();
            let continued_pipeline_stage_indent = continuation_pipeline_stage_indent.clone();
            previous_pipeline_indent = if content.trim().is_empty() {
                None
            } else if content.starts_with('#') {
                carried_pipeline_indent
            } else if line_ends_with_raw_continuation_operator(&line) {
                carried_pipeline_indent.or_else(|| {
                    let indent = line_leading_shell_indent(&line);
                    Some(
                        if content.starts_with('-') || line_closes_pipeline_stage_compound {
                            if raw_line_closes_inline_brace_group_before_pipeline(content) {
                                continued_pipeline_stage_indent.unwrap_or_else(|| {
                                    source_indent_minus_one_unit(indent, options)
                                })
                            } else {
                                indent.to_string()
                            }
                        } else {
                            source_indent_plus_one_unit(indent, options)
                        },
                    )
                })
            } else {
                None
            };
            let line_continues = line_without_continuation_backslash(&line).is_some();
            let line_indent = line_leading_shell_indent(&line).to_string();
            quote.scan_line(&line);
            compound_indents.update_line(
                content,
                &source_indent_for_compound_shift,
                &rendered_indent,
                indent,
                line_is_pipeline_continuation_stage,
                options,
            );
            if line_continues {
                if line_is_pipeline_continuation_stage && !used_continuation_indent {
                    continuation_pipeline_stage_indent = Some(line_indent.clone());
                }
            } else if used_continuation_indent {
                continuation_pipeline_stage_indent = None;
            }
            if quote.in_multiline_literal() && used_continuation_indent {
                literal_exit_continuation_indent = Some(line_indent.clone());
            }
            continuation_indent = if line_continues {
                Some(
                    if quote.in_multiline_literal() || used_continuation_indent {
                        line_indent
                    } else {
                        source_indent_plus_one_unit(&line_indent, options)
                    },
                )
            } else {
                None
            };
            continue;
        }
    }
}

fn collect_raw_command_substitution_spans(
    parts: &[shuck_ast::WordPartNode],
    spans: &mut Vec<shuck_ast::Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::CommandSubstitution { .. } => spans.push(part.span),
            WordPart::DoubleQuoted { parts, .. } => {
                collect_raw_command_substitution_spans(parts.as_slice(), spans);
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct RawShellQuoteState {
    quote: Option<char>,
}

impl RawShellQuoteState {
    fn in_multiline_literal(&self) -> bool {
        self.quote.is_some()
    }

    fn scan_line(&mut self, line: &str) {
        let mut escaped = false;
        for (index, ch) in line.char_indices() {
            match self.quote {
                Some('\'') => {
                    if ch == '\'' {
                        self.quote = None;
                    }
                }
                Some('"') => {
                    if ch == '"' && !escaped {
                        self.quote = None;
                    }
                }
                _ => {
                    if ch == '#' && shell_comment_can_start(line, index) {
                        break;
                    }
                    if ch == '\'' || (ch == '"' && !escaped) {
                        self.quote = Some(ch);
                    }
                }
            }

            escaped = ch == '\\' && !escaped;
            if ch != '\\' {
                escaped = false;
            }
        }
    }
}

fn render_word_part(
    rendered: &mut String,
    part: &WordPart,
    span: shuck_ast::Span,
    env: WordRenderEnv<'_, '_>,
    context: WordPartRenderContext,
) -> Result<(), std::fmt::Error> {
    let source = env.source;
    let options = env.options;
    let source_map = env.source_map;
    let facts = env.facts;

    if let Some(raw) = preferred_raw_word_part_source(part, span, source, options) {
        rendered.push_str(raw);
        return Ok(());
    }

    match part {
        WordPart::Literal(text) => {
            push_unquoted_literal(rendered, text.syntax_str(source, span));
        }
        WordPart::SingleQuoted { value, dollar } => {
            if *dollar {
                rendered.push('$');
            }
            rendered.push('\'');
            rendered.push_str(value.slice(source));
            rendered.push('\'');
        }
        WordPart::DoubleQuoted { parts, dollar } => {
            if *dollar {
                rendered.push('$');
            }
            rendered.push('"');
            let mut inner = String::new();
            let mut follows_line_indent_literal = false;
            for part in parts {
                match &part.kind {
                    WordPart::Literal(text) => {
                        let literal = if text.is_source_backed() {
                            text.syntax_str(source, part.span)
                        } else {
                            text.as_str(source, part.span)
                        };
                        if text.is_source_backed() {
                            inner.push_str(literal);
                        } else {
                            render_double_quoted_literal(&mut inner, literal);
                        }
                        follows_line_indent_literal =
                            literal_ends_with_line_indent_for_word_part(literal);
                    }
                    other => {
                        render_word_part(
                            &mut inner,
                            other,
                            part.span,
                            env,
                            WordPartRenderContext {
                                allow_source_indented_inline_command_substitution: context
                                    .allow_source_indented_inline_command_substitution,
                                source_indented_inline_command_substitution: context
                                    .allow_source_indented_inline_command_substitution
                                    && follows_line_indent_literal,
                            },
                        )?;
                        follows_line_indent_literal = false;
                    }
                }
            }
            if let Some(normalized) = normalize_raw_arithmetic_expansion_padding(&inner) {
                rendered.push_str(&normalized);
            } else {
                rendered.push_str(&inner);
            }
            rendered.push('"');
        }
        WordPart::Variable(name) => {
            std::write!(rendered, "${name}")?;
        }
        WordPart::CommandSubstitution { body, syntax } => {
            if let Some(raw) = raw_source_slice(span, source) {
                let raw = raw_dollar_command_substitution_slice(raw).unwrap_or(raw);
                let layout = command_substitution_layout(
                    Some(raw),
                    body,
                    source,
                    options.dialect(),
                    false,
                    context.source_indented_inline_command_substitution,
                );
                if raw_dollar_command_substitution_body(raw)
                    .is_some_and(raw_body_contains_pipeline_multistatement_brace_group)
                    && let Some(block) =
                        render_inline_raw_command_substitution_as_block(raw, options)
                {
                    rendered.push_str(&block);
                } else if stmt_seq_contains_comments(body) {
                    if commented_command_substitution_can_use_structural_formatter(body) {
                        let rendered_start = rendered.len();
                        if render_command_substitution(
                            rendered,
                            body,
                            span.end.offset,
                            source,
                            options,
                            layout,
                            1,
                            Some(raw),
                            source_map,
                            None,
                        )
                        .is_some()
                        {
                            restore_raw_case_terminator_suffix_comments(
                                rendered,
                                rendered_start,
                                raw,
                            );
                        } else {
                            push_raw_command_substitution_comment_fallback(
                                rendered,
                                raw,
                                body,
                                source,
                                span.start.offset,
                                options,
                                false,
                            );
                        }
                    } else {
                        push_raw_command_substitution_comment_fallback(
                            rendered,
                            raw,
                            body,
                            source,
                            span.start.offset,
                            options,
                            true,
                        );
                    }
                } else if let Some(block) =
                    render_inline_raw_command_substitution_as_block(raw, options)
                {
                    rendered.push_str(&block);
                } else if render_command_substitution(
                    rendered,
                    body,
                    span.end.offset,
                    source,
                    options,
                    layout,
                    1,
                    Some(raw),
                    source_map,
                    facts,
                )
                .is_some()
                {
                } else {
                    push_raw_shell_text_with_normalized_redirect_spacing(rendered, raw);
                }
            } else if render_command_substitution(
                rendered,
                body,
                span.end.offset,
                source,
                options,
                command_substitution_layout(
                    None,
                    body,
                    source,
                    options.dialect(),
                    *syntax == CommandSubstitutionSyntax::DollarParen,
                    false,
                ),
                1,
                None,
                source_map,
                facts,
            )
            .is_some()
            {
            } else {
                std::write!(rendered, "$({body:?})")?;
            }
        }
        WordPart::ProcessSubstitution { body, is_input } => {
            if let Some(raw) = raw_source_slice(span, source) {
                if stmt_seq_contains_comments(body) {
                    if process_substitution_source_opens_to_body_line(raw)
                        && !stmt_seq_has_heredoc(body)
                    {
                        push_raw_block_command_substitution_without_outer_indent(
                            rendered,
                            raw,
                            source,
                            span.start.offset,
                            options,
                        );
                    } else {
                        rendered.push_str(raw);
                    }
                } else if render_process_substitution(
                    rendered,
                    body,
                    *is_input,
                    span,
                    source,
                    options,
                    raw.contains('\n'),
                    Some(raw),
                    facts,
                )
                .is_some()
                {
                } else {
                    rendered.push_str(raw);
                }
            } else if render_process_substitution(
                rendered, body, *is_input, span, source, options, false, None, facts,
            )
            .is_some()
            {
            } else {
                let prefix = if *is_input { "<" } else { ">" };
                std::write!(rendered, "{}({body:?})", prefix)?;
            }
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => push_arithmetic_expansion(
            rendered,
            expression,
            expression_ast.as_deref(),
            *syntax,
            env,
        ),
        WordPart::Parameter(parameter) => {
            push_parameter_word(rendered, parameter, source, options)?;
        }
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => render_parameter_expansion(
            rendered,
            reference,
            operator.as_ref(),
            operand.as_ref(),
            *colon_variant,
            Some(span),
            env,
        )?,
        WordPart::Length(reference) | WordPart::ArrayLength(reference) => {
            push_braced_var_ref(rendered, "#", reference, source, options);
        }
        WordPart::ArrayAccess(reference) => {
            push_braced_var_ref(rendered, "", reference, source, options);
        }
        WordPart::ArrayIndices(reference) => {
            push_braced_var_ref(rendered, "!", reference, source, options);
        }
        WordPart::Substring {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push(':');
            push_parameter_slice_offset(rendered, offset, offset_ast.as_deref(), source, options);
            if let Some(length) = length {
                rendered.push(':');
                push_arithmetic_source_text(
                    rendered,
                    length,
                    length_ast.as_deref(),
                    source,
                    options,
                );
            }
            rendered.push('}');
        }
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
            if let Some(operator) = operator {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(parameter_defaulting_operator(operator.as_ref()));
                if let Some(operand) = operand {
                    push_parameter_operand(rendered, operand, source, options);
                }
            }
            rendered.push('}');
        }
        WordPart::PrefixMatch { prefix, kind } => {
            std::write!(rendered, "${{!{}{}}}", prefix, kind.as_char())?;
        }
        WordPart::Transformation { .. } | WordPart::ZshQualifiedGlob(_) => {
            rendered.push_str(span.slice(source));
        }
    }

    Ok(())
}

fn push_braced_var_ref(
    rendered: &mut String,
    prefix: &str,
    reference: &VarRef,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    rendered.push_str("${");
    rendered.push_str(prefix);
    push_var_ref(rendered, reference, source, options);
    rendered.push('}');
}

fn literal_ends_with_line_indent_for_word_part(literal: &str) -> bool {
    let Some((_, suffix)) = literal.rsplit_once('\n') else {
        return false;
    };
    suffix.chars().all(|ch| matches!(ch, ' ' | '\t'))
}

fn preferred_raw_word_part_source<'a>(
    part: &WordPart,
    span: shuck_ast::Span,
    source: &'a str,
    options: &ResolvedShellFormatOptions,
) -> Option<&'a str> {
    if options.simplify() || options.minify() {
        return None;
    }

    match part {
        WordPart::SingleQuoted { .. } => raw_source_slice(span, source),
        WordPart::DoubleQuoted { parts, .. } => {
            let raw = raw_source_slice(span, source)?;
            let has_formattable_parts = word_part_nodes_any(parts, &mut |part| {
                word_part_needs_formatter_rendering(part, source, options)
            });
            (!has_formattable_parts).then_some(raw)
        }
        WordPart::Parameter(parameter) => {
            let raw = raw_source_slice(span, source)?;
            (parameter_prefers_raw_source(parameter, span, source)
                && !parameter_raw_subscript_needs_compaction(parameter, source, options)
                && !raw_parameter_command_spacing_would_change(raw))
            .then_some(raw)
        }
        WordPart::ParameterExpansion { .. } => {
            let raw = raw_source_slice(span, source)?;
            (!raw_parameter_command_spacing_would_change(raw)).then_some(raw)
        }
        WordPart::Substring {
            offset_ast,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            offset_ast,
            length_ast,
            ..
        } => (!(offset_ast.is_some() || length_ast.is_some()))
            .then(|| raw_source_slice(span, source))
            .flatten(),
        _ => None,
    }
}

fn parameter_raw_subscript_needs_compaction(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if parameter_bourne_operand_needs_subscript_compaction(parameter, source) {
        return true;
    }
    if let Some(subscript) = parameter_bourne_subscript(parameter) {
        let syntax = subscript.syntax_text(source);
        if let Some(ast) = subscript.arithmetic_ast.as_ref()
            && arithmetic_subscript_prefers_spaced_expression(syntax)
        {
            let mut rendered = String::new();
            render_arithmetic_subscript_expr_to_buf(&mut rendered, ast, source, options, false);
            return rendered != syntax;
        }
        return compact_dynamic_arithmetic_subscript(syntax) != syntax;
    }
    if parameter.bourne().is_some() {
        return false;
    }
    let raw = parameter.raw_body.slice(source);
    compact_raw_parameter_subscript(raw) != raw
}

fn parameter_bourne_subscript(
    parameter: &shuck_ast::ParameterExpansion,
) -> Option<&shuck_ast::Subscript> {
    let reference = match parameter.bourne()? {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Length { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Indirect { reference, .. }
        | BourneParameterExpansion::Slice { reference, .. }
        | BourneParameterExpansion::Operation { reference, .. }
        | BourneParameterExpansion::Transformation { reference, .. } => reference,
        BourneParameterExpansion::PrefixMatch { .. } => return None,
    };
    reference.subscript.as_deref()
}

fn push_unquoted_literal(rendered: &mut String, literal: &str) {
    if !literal.contains("\\\n") && !literal.contains("\\\r\n") {
        rendered.push_str(literal);
        return;
    }

    let mut chars = literal.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(skipped_indent) = consume_escaped_newline_indent(&mut chars) {
                if skipped_indent {
                    rendered.push(' ');
                }
                continue;
            }
        }
        rendered.push(ch);
    }
}

pub(crate) fn normalize_raw_unquoted_word_continuations(raw: &str) -> Option<String> {
    if !raw.contains("\\\n") && !raw.contains("\\\r\n") {
        return None;
    }

    let mut normalized = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut changed = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_double_quotes {
            in_single_quotes = !in_single_quotes;
            normalized.push(ch);
            continue;
        }
        if ch == '"' && !in_single_quotes {
            in_double_quotes = !in_double_quotes;
            normalized.push(ch);
            continue;
        }
        if ch == '\\' && !in_single_quotes && !in_double_quotes {
            if let Some(skipped_indent) = consume_escaped_newline_indent(&mut chars) {
                changed = true;
                if chars
                    .peek()
                    .is_some_and(|next| matches!(next, '|' | '&' | ';' | '<' | '>' | '(' | ')'))
                {
                    return None;
                }
                if skipped_indent {
                    normalized.push(' ');
                }
                continue;
            }
        }
        normalized.push(ch);
    }

    changed.then_some(normalized)
}

fn consume_escaped_newline_indent(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Option<bool> {
    let mut probe = chars.clone();
    let newline_len = match probe.next() {
        Some('\n') => 1,
        Some('\r') if probe.next().is_some_and(|next| next == '\n') => 2,
        _ => return None,
    };

    for _ in 0..newline_len {
        chars.next();
    }
    let mut skipped_indent = false;
    while chars.peek().is_some_and(|next| matches!(next, ' ' | '\t')) {
        skipped_indent = true;
        chars.next();
    }
    Some(skipped_indent)
}

fn normalize_raw_compound_assignment_word_continuations(raw: &str) -> Option<String> {
    if (!raw.contains("\\\n") && !raw.contains("\\\r\n"))
        || raw.contains("$(")
        || raw.contains('`')
        || raw.contains("<(")
        || raw.contains(">(")
    {
        return None;
    }

    let open = raw.find("=(").or_else(|| raw.find("+=("))?;
    let open_paren = open + raw[open..].find('(')?;
    let head = raw.get(..=open_paren)?;
    if !raw_compound_assignment_head_is_simple(head) {
        return None;
    }
    let close = raw.rfind(')')?;
    if close <= open_paren {
        return None;
    }

    let body = raw.get(open_paren + 1..close)?;
    let tail = raw.get(close..)?;
    let body_lines = body
        .lines()
        .map(|line| {
            line_without_continuation_backslash(line)
                .unwrap_or_else(|| line.trim_end_matches([' ', '\t', '\r']))
        })
        .collect::<Vec<_>>();
    if body_lines.len() < 2 {
        return None;
    }

    let common_indent =
        common_nonempty_shell_indent(body_lines.get(1..).unwrap_or_default().iter().copied());
    let mut normalized = String::with_capacity(raw.len());
    normalized.push_str(head);
    normalized.push_str(body_lines[0].trim_start_matches([' ', '\t']));
    for line in &body_lines[1..] {
        normalized.push('\n');
        if line.trim().is_empty() {
            continue;
        }
        normalized.push('\t');
        normalized.push_str(
            line.strip_prefix(&common_indent)
                .unwrap_or_else(|| line.trim_start_matches([' ', '\t'])),
        );
    }
    normalized.push_str(tail);
    Some(normalized)
}

fn raw_compound_assignment_head_is_simple(head: &str) -> bool {
    let Some(name) = head.strip_suffix("+=(").or_else(|| head.strip_suffix("=(")) else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    matches!(first, '_' | 'a'..='z' | 'A'..='Z')
        && chars.all(|ch| matches!(ch, '_' | 'a'..='z' | 'A'..='Z' | '0'..='9'))
}

fn parameter_bourne_operand_needs_subscript_compaction(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
) -> bool {
    let operand = match parameter.bourne() {
        Some(
            BourneParameterExpansion::Indirect {
                operand: Some(operand),
                ..
            }
            | BourneParameterExpansion::Operation {
                operand: Some(operand),
                ..
            },
        ) => operand.slice(source),
        _ => return false,
    };
    compact_parameter_operand_subscripts(operand) != operand
}

fn parameter_needs_special_rendering(parameter: &shuck_ast::ParameterExpansion) -> bool {
    parameter.bourne().is_some_and(|syntax| match syntax {
        BourneParameterExpansion::Operation { operator, .. } => {
            matches!(
                operator.as_ref(),
                ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
            )
        }
        BourneParameterExpansion::Slice { .. } => true,
        _ => false,
    })
}

fn parameter_prefers_raw_source(
    parameter: &shuck_ast::ParameterExpansion,
    span: shuck_ast::Span,
    source: &str,
) -> bool {
    parameter.bourne().is_none_or(|syntax| match syntax {
        BourneParameterExpansion::Operation { operator, .. } => match operator.as_ref() {
            ParameterOp::ReplaceFirst { replacement, .. }
            | ParameterOp::ReplaceAll { replacement, .. } => {
                !replacement.slice(source).is_empty()
                    || raw_source_slice(span, source).is_some_and(|raw| raw.ends_with("/}"))
            }
            _ => true,
        },
        BourneParameterExpansion::Slice {
            offset_ast,
            length_ast,
            ..
        } => offset_ast.is_none() && length_ast.is_none(),
        _ => true,
    })
}

fn stmt_seq_contains_comments(sequence: &StmtSeq) -> bool {
    !sequence.leading_comments.is_empty()
        || !sequence.trailing_comments.is_empty()
        || sequence.iter().any(stmt_contains_comments)
}

fn stmt_contains_comments(stmt: &Stmt) -> bool {
    !stmt.leading_comments.is_empty()
        || stmt.inline_comment.is_some()
        || command_contains_comments(&stmt.command)
}

fn command_contains_comments(command: &Command) -> bool {
    match command {
        Command::Binary(command) => {
            stmt_contains_comments(&command.left) || stmt_contains_comments(&command.right)
        }
        Command::Compound(command) => compound_contains_comments(command),
        Command::Function(function) => stmt_contains_comments(&function.body),
        Command::AnonymousFunction(function) => stmt_contains_comments(&function.body),
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => false,
    }
}

fn compound_contains_comments(command: &CompoundCommand) -> bool {
    compound_contains_child(command, stmt_contains_comments, stmt_seq_contains_comments)
}

fn push_raw_command_substitution_comment_fallback(
    rendered: &mut String,
    raw: &str,
    body: &shuck_ast::StmtSeq,
    source: &str,
    span_start: usize,
    options: &ResolvedShellFormatOptions,
    try_normalized_body: bool,
) {
    if push_inline_raw_command_substitution_as_block(rendered, raw, options) {
        return;
    }
    if command_substitution_source_starts_with_body_line(raw) && !stmt_seq_has_heredoc(body) {
        push_raw_block_command_substitution_without_outer_indent(
            rendered, raw, source, span_start, options,
        );
        return;
    }
    if try_normalized_body
        && push_inline_raw_command_substitution_with_normalized_body(rendered, raw, options)
    {
        return;
    }
    push_raw_shell_text_with_normalized_redirect_spacing(rendered, raw);
}

#[allow(clippy::too_many_arguments)]
fn render_command_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    upper_bound: usize,
    source: &str,
    options: &ResolvedShellFormatOptions,
    layout: CommandSubstitutionLayout,
    inline_continuation_indent_levels: usize,
    raw: Option<&str>,
    _source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> Option<()> {
    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(
        source,
        body,
        options,
        facts,
        Some(upper_bound),
        &mut nested,
    )?;

    let trimmed = trim_trailing_line_endings(&nested);
    let normalized_backtick_body;
    let trimmed = if raw.is_some_and(|raw| raw.starts_with('`')) && trimmed.contains("\\\\$") {
        normalized_backtick_body = normalize_backtick_body_escaped_dollars(trimmed);
        normalized_backtick_body.as_str()
    } else {
        trimmed
    };
    if trimmed.is_empty() {
        if raw
            .and_then(raw_dollar_command_substitution_body)
            .is_some_and(|body| !body.trim_matches([' ', '\t', '\r', '\n']).is_empty())
        {
            return None;
        }
        rendered.push_str("$()");
        return Some(());
    }
    let normalized_close_continuation = trim_rendered_close_line_continuation(trimmed);
    let trimmed = normalized_close_continuation.as_deref().unwrap_or(trimmed);
    let trailing_escaped_whitespace = raw
        .and_then(raw_command_substitution_trailing_escaped_horizontal_whitespace)
        .or_else(|| {
            source_trailing_escaped_horizontal_whitespace_before_offset(source, upper_bound)
        });

    match layout {
        CommandSubstitutionLayout::Inline | CommandSubstitutionLayout::InlineContinued => {
            rendered.push_str("$(");
            let trimmed = trim_inline_command_substitution_padding(trimmed);
            if let Some(body) =
                restore_trailing_escaped_horizontal_whitespace(trimmed, trailing_escaped_whitespace)
            {
                push_command_substitution_inline_body(
                    rendered,
                    &body,
                    options,
                    inline_continuation_indent_levels,
                );
            } else {
                push_command_substitution_inline_body(
                    rendered,
                    trimmed,
                    options,
                    inline_continuation_indent_levels,
                );
            }
            rendered.push(')');
        }
        CommandSubstitutionLayout::InlineSourceIndented => {
            rendered.push_str("$(");
            push_source_indented_inline_command_substitution(rendered, trimmed, raw?, options);
            rendered.push(')');
        }
        CommandSubstitutionLayout::Block => {
            rendered.push_str("$(\n");
            push_indented_rendered_block(rendered, trimmed, options, 1);
            rendered.push_str("\n)");
        }
    }

    Some(())
}

fn format_nested_stmt_sequence_to_buf(
    source: &str,
    body: &StmtSeq,
    options: &ResolvedShellFormatOptions,
    facts: Option<&FormatterFacts<'_>>,
    upper_bound: Option<usize>,
    rendered: &mut String,
) -> Option<()> {
    let owned_facts;
    let facts = match facts {
        Some(facts) => facts,
        None => {
            let file = shuck_ast::File {
                body: body.clone(),
                span: body.span,
            };
            owned_facts = FormatterFacts::build(source, &file, options);
            &owned_facts
        }
    };
    format_stmt_sequence_streaming_to_buf(source, body, options, facts, upper_bound, rendered).ok()
}

fn restore_trailing_escaped_horizontal_whitespace(
    body: &str,
    escaped_whitespace: Option<char>,
) -> Option<String> {
    let whitespace = escaped_whitespace?;
    body.ends_with('\\').then(|| {
        let mut restored = body.to_string();
        restored.push(whitespace);
        restored
    })
}

fn raw_command_substitution_trailing_escaped_horizontal_whitespace(raw: &str) -> Option<char> {
    let body = raw_dollar_command_substitution_body(raw)?;
    trailing_escaped_horizontal_whitespace(body)
}

fn source_trailing_escaped_horizontal_whitespace_before_offset(
    source: &str,
    upper_bound: usize,
) -> Option<char> {
    let close_offset = upper_bound.checked_sub(1)?;
    if source.as_bytes().get(close_offset) != Some(&b')') {
        return None;
    }
    trailing_escaped_horizontal_whitespace(source.get(..close_offset)?)
}

fn trailing_escaped_horizontal_whitespace(body: &str) -> Option<char> {
    let (whitespace_start, whitespace) = body.char_indices().next_back()?;
    if !matches!(whitespace, ' ' | '\t') {
        return None;
    }
    let backslash_count = body.as_bytes()[..whitespace_start]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    (backslash_count % 2 == 1).then_some(whitespace)
}

fn trim_rendered_close_line_continuation(rendered: &str) -> Option<String> {
    let trimmed = rendered.trim_end_matches([' ', '\t']);
    if let Some((before_close, close_line)) = trimmed.rsplit_once('\n')
        && close_line.trim_matches([' ', '\t', '\r']) == ")"
    {
        let before_close = before_close.trim_end_matches([' ', '\t']);
        return has_odd_trailing_backslashes(before_close).then(|| {
            before_close[..before_close.len().saturating_sub(1)]
                .trim_end_matches([' ', '\t'])
                .to_string()
        });
    }
    has_odd_trailing_backslashes(trimmed).then(|| {
        trimmed[..trimmed.len().saturating_sub(1)]
            .trim_end_matches([' ', '\t'])
            .to_string()
    })
}

fn has_odd_trailing_backslashes(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn commented_command_substitution_can_use_structural_formatter(body: &StmtSeq) -> bool {
    let [stmt] = body.as_slice() else {
        return false;
    };
    !stmt.negated
        && stmt.redirects.is_empty()
        && stmt.terminator.is_none()
        && (matches!(
            &stmt.command,
            Command::Compound(CompoundCommand::Case(_) | CompoundCommand::If(_))
        ) || command_is_pipeline_of_compound_groups(&stmt.command))
}

fn command_is_pipeline_of_compound_groups(command: &Command) -> bool {
    let Command::Binary(binary) = command else {
        return false;
    };
    matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        && stmt_is_compound_group_pipeline_operand(&binary.left)
        && stmt_is_compound_group_pipeline_operand(&binary.right)
}

fn stmt_is_compound_group_pipeline_operand(stmt: &Stmt) -> bool {
    if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
        return false;
    }
    match &stmt.command {
        Command::Binary(_) => command_is_pipeline_of_compound_groups(&stmt.command),
        Command::Compound(CompoundCommand::BraceGroup(_) | CompoundCommand::Subshell(_)) => true,
        _ => false,
    }
}

fn restore_raw_case_terminator_suffix_comments(
    rendered: &mut String,
    rendered_start: usize,
    raw: &str,
) {
    let comments = raw_case_terminator_suffix_comments_by_line(raw);
    if comments.iter().all(Option::is_none) || rendered_start >= rendered.len() {
        return;
    }

    let mut body = rendered[rendered_start..].to_string();
    let mut search_start = 0usize;
    for comment in comments {
        let Some((line_start, line_end)) =
            next_uncommented_case_terminator_line(&body, search_start)
        else {
            break;
        };
        if let Some(comment) = comment {
            let insert_at = line_end;
            body.insert_str(insert_at, &format!(" {comment}"));
            search_start = line_start + (line_end - line_start) + comment.len() + 1;
        } else {
            search_start = line_end.saturating_add(1);
        }
    }

    rendered.truncate(rendered_start);
    rendered.push_str(&body);
}

fn raw_case_terminator_suffix_comments_by_line(raw: &str) -> Vec<Option<String>> {
    raw.lines()
        .filter_map(|line| {
            if !case_terminator_text_appears(line) {
                return None;
            }
            let comment = line.find('#').and_then(|comment_start| {
                let before_comment = line.get(..comment_start)?;
                if !case_terminator_text_appears(before_comment) {
                    return None;
                }
                Some(
                    line.get(comment_start..)?
                        .trim_end_matches([' ', '\t', '\r'])
                        .to_string(),
                )
            });
            Some(comment)
        })
        .collect()
}

fn next_uncommented_case_terminator_line(body: &str, start: usize) -> Option<(usize, usize)> {
    let mut offset = start.min(body.len());
    while offset < body.len() {
        let relative_end = body[offset..]
            .find('\n')
            .unwrap_or(body.len().saturating_sub(offset));
        let line_end = offset + relative_end;
        let line = body.get(offset..line_end)?;
        if case_terminator_text_appears(line) && !line.contains('#') {
            return Some((offset, line_end));
        }
        offset = line_end.saturating_add(1);
    }
    None
}

fn case_terminator_text_appears(text: &str) -> bool {
    text.contains(";;") || text.contains(";&") || text.contains(";;&")
}

fn normalize_backtick_body_escaped_dollars(body: &str) -> String {
    let mut normalized = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'\\') {
            chars.next();
            if chars.peek() == Some(&'$') {
                normalized.push('\\');
                normalized.push('$');
                chars.next();
                continue;
            }
            normalized.push('\\');
            normalized.push('\\');
            continue;
        }
        normalized.push(ch);
    }
    normalized
}

fn render_heredoc_body_command_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    upper_bound: usize,
    source: &str,
    options: &ResolvedShellFormatOptions,
    raw: Option<&str>,
) -> Option<()> {
    let raw = raw?;
    if !command_substitution_source_starts_with_body_line(raw) {
        return None;
    }

    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(
        source,
        body,
        options,
        None,
        Some(upper_bound),
        &mut nested,
    )?;
    let trimmed = trim_trailing_line_endings(&nested);
    if trimmed.is_empty() {
        rendered.push_str("$()");
        return Some(());
    }

    let body_prefix = heredoc_command_substitution_body_prefix(raw, options);
    let close_prefix = heredoc_command_substitution_close_prefix(&body_prefix, options);

    rendered.push_str("$(\n");
    for (index, line) in trimmed.lines().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        if !line.is_empty() {
            rendered.push_str(&body_prefix);
        }
        rendered.push_str(line);
    }
    rendered.push('\n');
    rendered.push_str(&close_prefix);
    rendered.push(')');
    Some(())
}

fn heredoc_command_substitution_body_prefix(
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let indent = raw
        .lines()
        .skip(1)
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty() && trimmed != ")").then(|| line_leading_shell_indent(line))
        })
        .unwrap_or("");

    match options.indent_style() {
        IndentStyle::Tab => indent.chars().map(|_| '\t').collect(),
        IndentStyle::Space => {
            let mut prefix = String::new();
            for ch in indent.chars() {
                if ch == '\t' {
                    prefix.push_str(&" ".repeat(usize::from(options.indent_width())));
                } else {
                    prefix.push(' ');
                }
            }
            prefix
        }
    }
}

fn heredoc_command_substitution_close_prefix(
    body_prefix: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let mut prefix = body_prefix.to_string();
    match options.indent_style() {
        IndentStyle::Tab => {
            prefix.pop();
        }
        IndentStyle::Space => {
            let width = usize::from(options.indent_width());
            for _ in 0..width {
                if !prefix.ends_with(' ') {
                    break;
                }
                prefix.pop();
            }
        }
    }
    prefix
}

fn push_command_substitution_inline_body(
    target: &mut String,
    body: &str,
    options: &ResolvedShellFormatOptions,
    inline_continuation_indent_levels: usize,
) {
    let expanded_pipeline_brace_group = expand_inline_pipeline_brace_group_body(body, options);
    let body = expanded_pipeline_brace_group.as_deref().unwrap_or(body);
    let adjusted_body = indent_inline_case_command_body(body, options).or_else(|| {
        indent_inline_pipeline_continuations(body, options, inline_continuation_indent_levels)
    });
    let body = adjusted_body.as_deref().unwrap_or(body);
    if body.starts_with('(') {
        target.push(' ');
    }
    if options.space_redirects() {
        target.push_str(body);
    } else {
        push_raw_shell_text_with_normalized_redirect_spacing(target, body);
    }
}

fn expand_inline_pipeline_brace_group_body(
    body: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if body.contains('\n') || !raw_body_contains_pipeline_multistatement_brace_group(body) {
        return None;
    }

    let parsed = shuck_parser::parser::Parser::with_dialect(body, options.dialect()).parse();
    if parsed.is_err() {
        return None;
    }

    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(body, &parsed.file.body, options, None, None, &mut nested)?;
    let trimmed = trim_trailing_line_endings(&nested);
    trimmed.contains('\n').then(|| trimmed.to_string())
}

fn indent_inline_case_command_body(
    body: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if !body.contains('\n') || !body.trim_start_matches([' ', '\t']).starts_with("case ") {
        return None;
    }

    let prefix = options.indent_prefix(1);
    let mut rendered = String::with_capacity(body.len() + prefix.len());
    let mut changed = false;
    for (index, line) in body.split('\n').enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        if index > 0 && !line.trim().is_empty() {
            rendered.push_str(&prefix);
            changed = true;
        }
        rendered.push_str(line);
    }
    changed.then_some(rendered)
}

fn trim_inline_command_substitution_padding(body: &str) -> &str {
    body.trim_matches([' ', '\t'])
}

fn indent_inline_pipeline_continuations(
    body: &str,
    options: &ResolvedShellFormatOptions,
    indent_levels: usize,
) -> Option<String> {
    if !body.contains('\n') {
        return None;
    }

    let unit = options.indent_prefix(1);
    let prefix = unit.repeat(indent_levels.max(1));
    let mut rendered = String::with_capacity(body.len() + prefix.len());
    let mut changed = false;
    let mut previous_ends_pipeline = false;
    let mut pipeline_comment_continuation = false;
    let mut continuation_indent: Option<String> = None;
    let mut quote = RawShellQuoteState::default();

    for (index, line) in body.split('\n').enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        let mut rendered_line = String::new();
        let used_continuation_indent = if let Some(indent) = continuation_indent.take()
            && !line.trim().is_empty()
        {
            rendered_line.push_str(&indent);
            rendered_line.push_str(line.trim_start_matches([' ', '\t']));
            changed = true;
            true
        } else {
            false
        };
        let continues_pipeline_operand = previous_ends_pipeline || pipeline_comment_continuation;
        if !used_continuation_indent
            && continues_pipeline_operand
            && !line.is_empty()
            && !line.starts_with([' ', '\t'])
        {
            rendered_line.push_str(&prefix);
            rendered_line.push_str(line);
            changed = true;
        } else if !used_continuation_indent
            && continues_pipeline_operand
            && indent_levels > 1
            && !line.trim().is_empty()
            && line_leading_shell_indent(line) != prefix
        {
            rendered_line.push_str(&prefix);
            rendered_line.push_str(line.trim_start_matches([' ', '\t']));
            changed = true;
        } else if !used_continuation_indent {
            rendered_line.push_str(line);
        }

        rendered.push_str(&rendered_line);
        let line_is_pipeline_comment = continues_pipeline_operand
            && rendered_line
                .trim_start_matches([' ', '\t'])
                .starts_with('#');
        let line_continues = line_without_continuation_backslash(&rendered_line).is_some();
        quote.scan_line(&rendered_line);
        previous_ends_pipeline = line_ends_with_raw_continuation_operator(&rendered_line);
        pipeline_comment_continuation = line_is_pipeline_comment;
        continuation_indent = line_continues.then(|| {
            let indent = line_leading_shell_indent(&rendered_line);
            if quote.in_multiline_literal() || used_continuation_indent {
                indent.to_string()
            } else {
                source_indent_plus_one_unit(indent, options)
            }
        });
    }

    changed.then_some(rendered)
}

fn line_ends_with_pipeline_operator(line: &str) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    trimmed.ends_with("|&") || (trimmed.ends_with('|') && !trimmed.ends_with("||"))
}

fn line_ends_with_raw_continuation_operator(line: &str) -> bool {
    let code = trailing_comment_start(line)
        .map(|comment_start| &line[..comment_start])
        .unwrap_or(line);
    let trimmed = code.trim_end_matches([' ', '\t', '\r']);
    line_ends_with_pipeline_operator(trimmed) || trimmed.ends_with("&&") || trimmed.ends_with("||")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandSubstitutionLayout {
    Inline,
    InlineContinued,
    InlineSourceIndented,
    Block,
}

fn command_substitution_layout(
    raw: Option<&str>,
    body: &shuck_ast::StmtSeq,
    source: &str,
    dialect: shuck_parser::ShellDialect,
    force_block: bool,
    allow_source_indented_inline: bool,
) -> CommandSubstitutionLayout {
    if force_block {
        return CommandSubstitutionLayout::Block;
    }

    if stmt_seq_has_heredoc(body) {
        return CommandSubstitutionLayout::Block;
    }

    if let Some(raw) = raw {
        if command_substitution_source_starts_with_body_line(raw) {
            return CommandSubstitutionLayout::Block;
        }
        if command_substitution_source_closes_on_own_line(raw) {
            return CommandSubstitutionLayout::Block;
        }
        if command_substitution_source_parses_as_multiple_statements(raw, dialect) {
            return CommandSubstitutionLayout::Block;
        }
        if command_substitution_source_prefers_continued_inline_body(raw) {
            return CommandSubstitutionLayout::InlineContinued;
        }
        if allow_source_indented_inline && raw.contains('\n') {
            return CommandSubstitutionLayout::InlineSourceIndented;
        }
    }

    if body.len() > 1
        || body
            .span
            .slice(source)
            .trim_start_matches([' ', '\t', '\r'])
            .starts_with('\n')
    {
        CommandSubstitutionLayout::Block
    } else {
        CommandSubstitutionLayout::Inline
    }
}

fn command_substitution_source_parses_as_multiple_statements(
    raw: &str,
    dialect: shuck_parser::ShellDialect,
) -> bool {
    if raw.contains('\n') || !raw.contains(';') {
        return false;
    }

    let Some(body) = raw_dollar_command_substitution_body(raw) else {
        return false;
    };
    let body = body.trim();
    if body.is_empty() {
        return false;
    }

    let parsed = shuck_parser::parser::Parser::with_dialect(body, dialect).parse();
    !parsed.is_err() && parsed.file.body.len() > 1
}

fn raw_dollar_command_substitution_body(raw: &str) -> Option<&str> {
    raw.strip_prefix("$(")?;
    let close_offset = matching_raw_command_substitution_close(raw, 2)?;
    raw.get(2..close_offset)
}

fn raw_dollar_command_substitution_slice(raw: &str) -> Option<&str> {
    raw.strip_prefix("$(")?;
    let close_offset = matching_raw_command_substitution_close(raw, 2)?;
    raw.get(..close_offset + 1)
}

fn command_substitution_source_starts_with_body_line(raw: &str) -> bool {
    if raw.starts_with(['\n', '\r']) {
        return true;
    }
    raw.strip_prefix("$(")
        .is_some_and(|after_open| after_open.starts_with(['\n', '\r']))
}

fn command_substitution_source_closes_on_own_line(raw: &str) -> bool {
    substitution_source_closes_on_own_line(raw)
}

fn push_inline_raw_command_substitution_as_block(
    target: &mut String,
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    let Some(after_open) = raw.strip_prefix("$(") else {
        return false;
    };
    if after_open.starts_with(['\n', '\r']) || !command_substitution_source_closes_on_own_line(raw)
    {
        return false;
    }

    let Some(close_offset) = raw.rfind(')') else {
        return false;
    };
    let Some(close_line_start) = raw[..close_offset].rfind('\n').map(|index| index + 1) else {
        return false;
    };
    let Some(body_source) = raw.get(2..close_line_start) else {
        return false;
    };
    let body_source = body_source.trim_end_matches(['\n', '\r']);
    if body_source.trim().is_empty() {
        target.push_str("$()");
        return true;
    }

    let nested = normalize_inline_raw_command_substitution_body(body_source, options);
    target.push_str("$(\n");
    push_indented_rendered_block(target, &nested, options, 1);
    target.push_str("\n)");
    true
}

fn push_inline_raw_command_substitution_with_normalized_body(
    target: &mut String,
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if command_substitution_source_starts_with_body_line(raw)
        || command_substitution_source_closes_on_own_line(raw)
    {
        return false;
    }
    let Some(body_source) = raw_dollar_command_substitution_body(raw) else {
        return false;
    };
    if !body_source.contains('\n') {
        return false;
    }

    let body_source = body_source.trim_start_matches([' ', '\t', '\r']);
    if !body_source.starts_with('(') {
        return false;
    }

    let nested = normalize_inline_raw_command_substitution_body_preserving_nested_comments(
        body_source,
        options,
    );
    target.push_str("$(");
    if nested.starts_with('(') {
        target.push(' ');
    }
    target.push_str(&nested);
    target.push(')');
    true
}

fn normalize_inline_raw_command_substitution_body(
    body_source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    normalize_inline_raw_command_substitution_body_with_options(body_source, options, false)
}

fn normalize_inline_raw_command_substitution_body_preserving_nested_comments(
    body_source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    normalize_inline_raw_command_substitution_body_with_options(body_source, options, true)
}

fn normalize_inline_raw_command_substitution_body_with_options(
    body_source: &str,
    options: &ResolvedShellFormatOptions,
    preserve_nested_comment_indent: bool,
) -> String {
    let normalized = normalize_raw_pipeline_continuations(body_source);
    let normalized_pipeline_continuation = normalized.is_some();
    let body_source = normalized.as_deref().unwrap_or(body_source);
    let normalized_comment_continuations =
        normalize_continuations_before_comment_lines(body_source);
    let body_source = normalized_comment_continuations
        .as_deref()
        .unwrap_or(body_source);
    let normalized_close_continuations =
        normalize_continuations_before_substitution_close_lines(body_source);
    let body_source = normalized_close_continuations
        .as_deref()
        .unwrap_or(body_source);
    let lines = body_source.lines().map(str::to_string).collect::<Vec<_>>();
    let source_base_indent = inline_raw_body_source_base_indent(&lines);

    let mut rendered = String::new();
    let mut previous_pipeline_indent_units: Option<usize> = None;
    let mut continuation_indent_units: Option<usize> = None;
    let mut pipeline_compounds = Vec::<InlinePipelineCompound>::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        let content = line.trim_start_matches([' ', '\t']);
        if content.trim().is_empty() {
            previous_pipeline_indent_units = None;
            continuation_indent_units = None;
            continue;
        }

        let carried_pipeline_indent = previous_pipeline_indent_units;
        let pipeline_base_units = pipeline_compounds
            .last()
            .map(|compound| compound.base_units)
            .unwrap_or(0);
        let relative_source_indent =
            inline_raw_body_relative_source_indent(line, index, source_base_indent.as_deref());
        let relative_indent = if content.starts_with('#')
            && carried_pipeline_indent.is_none()
            && pipeline_compounds.is_empty()
            && (!preserve_nested_comment_indent || relative_source_indent.is_empty())
        {
            ""
        } else {
            relative_source_indent
        };
        let mut indent_units = pipeline_base_units + raw_indent_units(relative_indent, options);
        if let Some(previous_units) = carried_pipeline_indent {
            let extra_units = usize::from(!normalized_pipeline_continuation);
            indent_units = indent_units.max(previous_units + extra_units);
        }
        let mut used_continuation_indent = false;
        if let Some(units) = continuation_indent_units.take()
            && !content.starts_with('#')
        {
            indent_units = units;
            used_continuation_indent = true;
        }

        rendered.extend(std::iter::repeat_n('\t', indent_units));
        push_raw_shell_line_with_normalized_redirect_spacing(&mut rendered, content);
        let line_is_pipeline_continuation_stage = carried_pipeline_indent.is_some();
        if content.starts_with('#') {
            previous_pipeline_indent_units = carried_pipeline_indent;
        } else {
            previous_pipeline_indent_units =
                line_ends_with_raw_continuation_operator(content).then_some(indent_units);
            if line_without_continuation_backslash(content).is_some() {
                continuation_indent_units = Some(if used_continuation_indent {
                    indent_units
                } else {
                    indent_units + 1
                });
            } else {
                continuation_indent_units = None;
            }
        }
        if let Some(close_keyword) = raw_compound_close_keyword(content)
            && (line_is_pipeline_continuation_stage || !pipeline_compounds.is_empty())
        {
            pipeline_compounds.push(InlinePipelineCompound {
                close_keyword,
                base_units: if line_is_pipeline_continuation_stage {
                    indent_units
                } else {
                    pipeline_base_units
                },
            });
        }
        if pipeline_compounds
            .last()
            .is_some_and(|compound| raw_line_closes_compound(content, compound.close_keyword))
        {
            pipeline_compounds.pop();
        }
    }

    rendered
}

struct InlinePipelineCompound {
    close_keyword: &'static str,
    base_units: usize,
}

fn inline_raw_body_source_base_indent(lines: &[String]) -> Option<String> {
    let mut common: Option<String> = None;
    for line in lines.iter().skip(1) {
        if line.trim_matches([' ', '\t', '\r']).is_empty() {
            continue;
        }
        let indent = line_leading_shell_indent(line);
        if refine_common_indent(&mut common, indent) {
            return None;
        }
    }
    common
}

fn inline_raw_body_relative_source_indent<'a>(
    line: &'a str,
    index: usize,
    source_base_indent: Option<&str>,
) -> &'a str {
    let indent = line_leading_shell_indent(line);
    if index == 0 {
        return indent;
    }
    let Some(source_base_indent) = source_base_indent else {
        return indent;
    };
    indent.strip_prefix(source_base_indent).unwrap_or("")
}

fn command_substitution_source_prefers_continued_inline_body(raw: &str) -> bool {
    let Some(after_open) = raw.strip_prefix("$(") else {
        return false;
    };
    if after_open.starts_with(['\n', '\r']) {
        return false;
    }

    raw.lines()
        .any(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with('\\'))
}

fn push_raw_block_command_substitution_without_outer_indent(
    target: &mut String,
    raw: &str,
    source: &str,
    start_offset: usize,
    options: &ResolvedShellFormatOptions,
) {
    let normalized_pipeline = normalize_raw_pipeline_continuations(raw);
    let normalized_pipeline_continuation = normalized_pipeline.is_some();
    let raw = normalized_pipeline.as_deref().unwrap_or(raw);
    let normalized_comment_continuations = normalize_continuations_before_comment_lines(raw);
    let raw = normalized_comment_continuations.as_deref().unwrap_or(raw);
    let normalized_close_continuations =
        normalize_continuations_before_substitution_close_lines(raw);
    let raw = normalized_close_continuations.as_deref().unwrap_or(raw);
    let outer_indent = line_indent_before_source_offset(source, start_offset).unwrap_or("");
    let raw_lines = raw.split('\n').collect::<Vec<_>>();
    let Some((first, lines)) = raw_lines.split_first() else {
        return;
    };
    target.push_str(first);
    let mut body_indent: Option<String> = None;
    let mut previous_pipeline_indent: Option<String> = None;
    let mut continuation_indent: Option<String> = None;
    let mut compound_indents = RawCompoundIndentState::default();
    let mut quote = RawShellQuoteState::default();
    for (line_index, line) in lines.iter().enumerate() {
        let line = *line;
        target.push('\n');
        if quote.in_multiline_literal() {
            target.push_str(line);
            quote.scan_line(line);
            let (indent, content) = raw_line_parts(line);
            previous_pipeline_indent = if content.trim().is_empty() {
                None
            } else if line_ends_with_raw_continuation_operator(line) {
                Some(indent.to_string())
            } else {
                None
            };
            continue;
        }

        let mut line = strip_outer_indent_or_one_unit(line, outer_indent, options).to_string();
        let source_indent_for_compound_shift = line_leading_shell_indent(&line).to_string();
        if let Some(shifted) = compound_indents.shifted_line(&line, options) {
            line = shifted;
        }
        let carried_pipeline_indent = previous_pipeline_indent.clone();
        let mut force_preserve_line_indent = false;
        let (indent, content) = raw_line_parts(&line);
        if let Some(previous_indent) = previous_pipeline_indent.as_deref()
            && !content.trim().is_empty()
            && !raw_line_closes_substitution_wrapper(content)
        {
            let desired_indent = if normalized_pipeline_continuation {
                previous_indent.to_string()
            } else {
                source_indent_plus_one_unit(previous_indent, options)
            };
            if raw_indent_units(indent, options) < raw_indent_units(&desired_indent, options) {
                line = format!("{desired_indent}{content}");
                force_preserve_line_indent = true;
            }
        }
        let (indent, content) = raw_line_parts(&line);
        let in_compound_body = compound_indents.in_body(content);
        if let Some(child_indent) =
            compound_indents.child_indent_if_underindented(indent, content, options)
        {
            line = format!("{child_indent}{content}");
            force_preserve_line_indent = true;
        }
        let (indent, content) = raw_line_parts(&line);
        let mut forced_rendered_indent = None;
        let mut used_continuation_indent = false;
        if let Some(previous_indent) = continuation_indent.take()
            && !content.trim().is_empty()
            && !content.starts_with('#')
            && !raw_line_closes_substitution_wrapper(content)
        {
            forced_rendered_indent = Some(previous_indent);
            force_preserve_line_indent = true;
            used_continuation_indent = true;
        }
        if compound_indents.comments.len() > 1 && compound_indents.closes_last(content) {
            force_preserve_line_indent = true;
        }
        let closes_substitution_wrapper = raw_line_closes_substitution_wrapper(content)
            && raw_block_line_is_outer_substitution_close(lines, line_index);
        let leading_block_comment = body_indent.is_none() && content.starts_with('#');
        if body_indent.is_none()
            && !content.trim().is_empty()
            && !content.starts_with('#')
            && !closes_substitution_wrapper
        {
            body_indent = Some(indent.to_string());
        }
        let is_pipeline_continuation =
            carried_pipeline_indent.is_some() && !content.trim().is_empty();
        let body_indent_for_line =
            if force_preserve_line_indent || is_pipeline_continuation || in_compound_body {
                None
            } else if leading_block_comment {
                Some("")
            } else {
                body_indent.as_deref()
            };
        let rendered_indent = if closes_substitution_wrapper {
            push_raw_shell_line_with_rendered_indent(target, &line, options, "");
            String::new()
        } else if let Some(rendered_indent) = forced_rendered_indent.as_deref() {
            push_raw_shell_line_with_rendered_indent(target, &line, options, rendered_indent);
            rendered_indent.to_string()
        } else {
            push_raw_shell_line_with_normalized_source_indent(
                target,
                &line,
                options,
                body_indent_for_line,
            );
            rendered_raw_shell_indent_for_line(indent, content, body_indent_for_line, options)
        };
        let line_is_pipeline_continuation_stage = carried_pipeline_indent.is_some();
        previous_pipeline_indent = if content.trim().is_empty() {
            None
        } else if content.starts_with('#') {
            carried_pipeline_indent
        } else if line_ends_with_raw_continuation_operator(&line) {
            carried_pipeline_indent.or_else(|| Some(rendered_indent.clone()))
        } else {
            None
        };
        let line_continues = line_without_continuation_backslash(&line).is_some();
        quote.scan_line(&line);
        continuation_indent = if line_continues && !content.starts_with('#') {
            Some(if used_continuation_indent {
                rendered_indent.clone()
            } else {
                source_indent_plus_one_unit(&rendered_indent, options)
            })
        } else {
            None
        };
        compound_indents.update_line(
            content,
            &source_indent_for_compound_shift,
            &rendered_indent,
            indent,
            line_is_pipeline_continuation_stage,
            options,
        );
    }
}

fn normalize_raw_backtick_command_substitution(raw: &str) -> Option<String> {
    let body = raw.strip_prefix('`')?.strip_suffix('`')?;
    let body = normalize_backtick_body_escaped_dollars(body);
    Some(format!("$({body})"))
}

fn raw_block_line_is_outer_substitution_close(lines: &[&str], index: usize) -> bool {
    lines
        .get(index.saturating_add(1)..)
        .is_none_or(|remaining| {
            remaining
                .iter()
                .all(|line| line.trim_matches([' ', '\t', '\r']).is_empty())
        })
}

fn normalize_continuations_before_comment_lines(text: &str) -> Option<String> {
    normalize_continuations_before_matching_lines(text, false, |next| next.starts_with('#'))
}

fn normalize_continuations_before_substitution_close_lines(text: &str) -> Option<String> {
    normalize_continuations_before_matching_lines(text, true, raw_line_closes_substitution_wrapper)
}

fn normalize_continuations_before_matching_lines(
    text: &str,
    trim_prefix: bool,
    next_line_matches: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let mut changed = false;

    for index in 0..lines.len().saturating_sub(1) {
        let next_content = lines[index + 1].trim_start_matches([' ', '\t']);
        if next_line_matches(next_content)
            && let Some(prefix) = line_without_continuation_backslash(&lines[index])
        {
            lines[index] = if trim_prefix {
                prefix.trim_end_matches([' ', '\t']).to_string()
            } else {
                prefix.to_string()
            };
            changed = true;
        }
    }

    changed.then(|| lines.join("\n"))
}

#[derive(Debug)]
struct RawCompoundIndentShift {
    source_indent: String,
    extra_units: usize,
    close_keyword: &'static str,
}

struct RawCompoundCommentIndent {
    child_indent: String,
    close_keyword: &'static str,
    pipeline_continuation: bool,
}

#[derive(Default)]
struct RawCompoundIndentState {
    shifts: Vec<RawCompoundIndentShift>,
    comments: Vec<RawCompoundCommentIndent>,
}

impl RawCompoundIndentState {
    fn shifted_line(&self, line: &str, options: &ResolvedShellFormatOptions) -> Option<String> {
        let shift = self.shifts.last()?;
        raw_line_indent_matches_shift(line, shift)
            .then(|| add_raw_indent_units(line, shift.extra_units, options))
    }

    fn in_body(&self, content: &str) -> bool {
        self.comments.last().is_some_and(|compound| {
            !content.trim().is_empty()
                && !raw_line_closes_compound(content, compound.close_keyword)
                && !raw_line_is_compound_mid_keyword(content)
        })
    }

    fn child_indent_if_underindented<'a>(
        &'a self,
        indent: &str,
        content: &str,
        options: &ResolvedShellFormatOptions,
    ) -> Option<&'a str> {
        let compound = self.comments.last()?;
        (self.in_body(content)
            && raw_indent_units(indent, options)
                < raw_indent_units(&compound.child_indent, options))
        .then_some(compound.child_indent.as_str())
    }

    fn closes_last(&self, content: &str) -> bool {
        self.comments
            .last()
            .is_some_and(|compound| raw_line_closes_compound(content, compound.close_keyword))
    }

    fn closes_pipeline_stage(&self, content: &str) -> bool {
        self.comments.last().is_some_and(|compound| {
            compound.pipeline_continuation
                && raw_line_closes_compound(content, compound.close_keyword)
        })
    }

    fn update_line(
        &mut self,
        content: &str,
        source_indent: &str,
        rendered_indent: &str,
        shifted_indent: &str,
        pipeline_continuation: bool,
        options: &ResolvedShellFormatOptions,
    ) {
        if let Some(close_keyword) = raw_compound_close_keyword(content) {
            self.comments.push(RawCompoundCommentIndent {
                child_indent: source_indent_plus_one_unit(rendered_indent, options),
                close_keyword,
                pipeline_continuation,
            });
            let before_units = raw_indent_units(source_indent, options);
            let after_units = raw_indent_units(shifted_indent, options);
            if after_units > before_units {
                self.shifts.push(RawCompoundIndentShift {
                    source_indent: source_indent.to_string(),
                    extra_units: after_units - before_units,
                    close_keyword,
                });
            }
        }
        if self
            .shifts
            .last()
            .is_some_and(|shift| raw_line_closes_compound(content, shift.close_keyword))
        {
            self.shifts.pop();
        }
        if self.closes_last(content) {
            self.comments.pop();
        }
    }
}

fn raw_line_indent_matches_shift(line: &str, shift: &RawCompoundIndentShift) -> bool {
    let (indent, content) = raw_line_parts(line);
    !content.trim().is_empty() && raw_indent_starts_with(indent, &shift.source_indent)
}

fn raw_line_parts(line: &str) -> (&str, &str) {
    let indent = line_leading_shell_indent(line);
    (indent, &line[indent.len()..])
}

fn raw_indent_starts_with(indent: &str, prefix: &str) -> bool {
    indent == prefix || indent.starts_with(prefix)
}

fn add_raw_indent_units(
    line: &str,
    extra_units: usize,
    options: &ResolvedShellFormatOptions,
) -> String {
    let (indent, content) = raw_line_parts(line);
    let mut shifted = indent.to_string();
    for _ in 0..extra_units {
        shifted = source_indent_plus_one_unit(&shifted, options);
    }
    format!("{shifted}{content}")
}

fn push_raw_shell_line_with_normalized_source_indent(
    target: &mut String,
    line: &str,
    options: &ResolvedShellFormatOptions,
    body_indent: Option<&str>,
) {
    let (mut indent, content) = raw_line_parts(line);
    if content.starts_with('#')
        && let Some(body_indent) = body_indent
        && indent.len() > body_indent.len()
    {
        indent = body_indent;
    }
    let trimmed_content = content.trim_matches([' ', '\t', '\r']);
    let mut rendered_indent = String::new();
    if body_indent == Some("")
        && !trimmed_content.is_empty()
        && !raw_line_closes_substitution_wrapper(trimmed_content)
    {
        options.push_indent_units(&mut rendered_indent, 1);
    } else {
        rendered_indent.push_str(&normalized_raw_shell_indent(indent, options));
    }
    target.push_str(&rendered_indent);
    let normalized_content;
    let content = {
        normalized_content = body_indent
            .is_some()
            .then(|| strip_semicolon_before_trailing_comment(content))
            .flatten()
            .or_else(|| normalize_padding_before_trailing_comment(content));
        normalized_content.as_deref().unwrap_or(content)
    };
    push_raw_shell_line_content_with_normalized_spacing(target, content, options, &rendered_indent);
}

fn push_raw_shell_line_with_rendered_indent(
    target: &mut String,
    line: &str,
    options: &ResolvedShellFormatOptions,
    rendered_indent: &str,
) {
    let (_, content) = raw_line_parts(line);
    target.push_str(rendered_indent);
    let normalized_content = normalize_padding_before_trailing_comment(content);
    let content = normalized_content.as_deref().unwrap_or(content);
    push_raw_shell_line_content_with_normalized_spacing(target, content, options, rendered_indent);
}

fn rendered_raw_shell_indent_for_line(
    indent: &str,
    content: &str,
    body_indent: Option<&str>,
    options: &ResolvedShellFormatOptions,
) -> String {
    let trimmed_content = content.trim_matches([' ', '\t', '\r']);
    if body_indent == Some("")
        && !trimmed_content.is_empty()
        && !raw_line_closes_substitution_wrapper(trimmed_content)
    {
        let mut rendered = String::new();
        options.push_indent_units(&mut rendered, 1);
        rendered
    } else {
        normalized_raw_shell_indent(indent, options)
    }
}

fn strip_semicolon_before_trailing_comment(line: &str) -> Option<String> {
    let comment_start = trailing_comment_start(line)?;
    let before_comment = line[..comment_start].trim_end_matches([' ', '\t', '\r']);
    let before_semicolon = before_comment.strip_suffix(';')?;
    if before_semicolon.ends_with(';') {
        return None;
    }

    let mut rendered = String::with_capacity(line.len().saturating_sub(1));
    rendered.push_str(before_semicolon.trim_end_matches([' ', '\t', '\r']));
    rendered.push(' ');
    rendered.push_str(&line[comment_start..]);
    Some(rendered)
}

fn normalize_padding_before_trailing_comment(line: &str) -> Option<String> {
    let comment_start = trailing_comment_start(line)?;
    let before_comment = &line[..comment_start];
    let code = before_comment.trim_end_matches([' ', '\t', '\r']);
    if code.is_empty()
        || code.len() == before_comment.len()
        || before_comment[code.len()..].chars().count() == 1
    {
        return None;
    }

    let mut rendered = String::with_capacity(line.len());
    rendered.push_str(code);
    rendered.push(' ');
    rendered.push_str(&line[comment_start..]);
    Some(rendered)
}

fn trailing_comment_start(line: &str) -> Option<usize> {
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;

    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single_quotes => escaped = true,
            '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
            '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
            '#' if !in_single_quotes && !in_double_quotes => return Some(index),
            _ => {}
        }
    }

    None
}

fn raw_line_closes_substitution_wrapper(content: &str) -> bool {
    let Some(rest) = content.trim_matches([' ', '\t', '\r']).strip_prefix(')') else {
        return false;
    };
    let rest = rest.trim_matches([' ', '\t', '\r']);
    rest.is_empty()
        || rest == "\\"
        || rest == "|"
        || rest == "|&"
        || rest.starts_with("#")
        || rest.starts_with("\\ ")
        || rest.starts_with("| ")
        || rest.starts_with("|& ")
}

fn push_raw_shell_text_with_normalized_redirect_spacing(target: &mut String, text: &str) {
    let normalized_pipeline = normalize_raw_pipeline_continuations(text);
    let text = normalized_pipeline.as_deref().unwrap_or(text);
    let mut lines = text.split('\n');
    if let Some(first) = lines.next() {
        push_raw_shell_line_with_normalized_redirect_spacing(target, first);
    }
    for line in lines {
        target.push('\n');
        push_raw_shell_line_with_normalized_redirect_spacing(target, line);
    }
}

fn push_raw_shell_line_content_with_normalized_spacing(
    target: &mut String,
    line: &str,
    options: &ResolvedShellFormatOptions,
    line_indent: &str,
) {
    let mut rendered = String::new();
    if expand_inline_raw_command_substitutions_in_line(&mut rendered, line, options) {
        let mut lines = rendered.split('\n');
        if let Some(first) = lines.next() {
            target.push_str(first);
        }
        for line in lines {
            target.push('\n');
            target.push_str(line_indent);
            target.push_str(line);
        }
    } else {
        push_raw_shell_line_with_normalized_redirect_spacing(target, line);
    }
}

pub(crate) fn normalize_raw_pipeline_continuations(text: &str) -> Option<String> {
    let trailing = normalize_raw_trailing_pipe_continuations(text);
    let leading = normalize_raw_leading_pipe_continuations(trailing.as_deref().unwrap_or(text));
    leading.or(trailing)
}

fn normalize_raw_trailing_pipe_continuations(text: &str) -> Option<String> {
    let mut lines = text
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut changed = false;

    for line in &mut lines {
        let Some(prefix) = line_without_trailing_pipe_continuation(line) else {
            continue;
        };
        *line = prefix.to_string();
        changed = true;
    }

    changed.then(|| lines.join("\n"))
}

fn normalize_raw_leading_pipe_continuations(text: &str) -> Option<String> {
    let mut lines = text
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut changed = false;

    for index in 0..lines.len().saturating_sub(1) {
        let Some(prefix) = line_without_continuation_backslash(&lines[index]).map(str::to_string)
        else {
            continue;
        };
        let Some((indent, operator, rest)) =
            leading_pipe_continuation(&lines[index + 1]).map(|(indent, operator, rest)| {
                (
                    indent.to_string(),
                    operator,
                    rest.trim_start_matches([' ', '\t', '\r']).to_string(),
                )
            })
        else {
            continue;
        };

        lines[index] = format!("{prefix} {operator}");
        lines[index + 1] = format!("{indent}{rest}");
        changed = true;
    }

    changed.then(|| lines.join("\n"))
}

fn line_without_trailing_pipe_continuation(line: &str) -> Option<&str> {
    let prefix = line_without_continuation_backslash(line)?;
    line_ends_with_raw_continuation_operator(prefix).then_some(prefix)
}

fn leading_pipe_continuation(line: &str) -> Option<(&str, &'static str, &str)> {
    let content_start = line
        .char_indices()
        .find_map(|(index, ch)| (!matches!(ch, ' ' | '\t')).then_some(index))
        .unwrap_or(line.len());
    let indent = &line[..content_start];
    let rest = &line[content_start..];
    if let Some(remainder) = rest.strip_prefix("|&") {
        Some((indent, "|&", remainder))
    } else if let Some(remainder) = rest.strip_prefix("||") {
        Some((indent, "||", remainder))
    } else if let Some(remainder) = rest.strip_prefix("&&") {
        Some((indent, "&&", remainder))
    } else {
        rest.strip_prefix('|')
            .map(|remainder| (indent, "|", remainder))
    }
}

fn expand_inline_raw_command_substitutions_in_line(
    target: &mut String,
    line: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if !line.contains("$(") {
        return false;
    }

    let bytes = line.as_bytes();
    let mut changed = false;
    let mut last = 0usize;
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\'' && !in_double_quotes {
            in_single_quotes = !in_single_quotes;
            index += 1;
            continue;
        }
        if byte == b'"' && !in_single_quotes {
            in_double_quotes = !in_double_quotes;
            index += 1;
            continue;
        }
        if !in_single_quotes && !in_double_quotes && byte == b'#' {
            break;
        }
        if !in_single_quotes
            && !in_double_quotes
            && byte == b'$'
            && bytes.get(index + 1) == Some(&b'(')
            && bytes.get(index + 2) != Some(&b'(')
            && let Some(close_offset) = matching_raw_command_substitution_close(line, index + 2)
        {
            let raw = &line[index..=close_offset];
            if let Some(block) = render_inline_raw_command_substitution_as_block(raw, options) {
                push_raw_shell_line_with_normalized_redirect_spacing(target, &line[last..index]);
                target.push_str(&block);
                last = close_offset + 1;
                changed = true;
            }
            index = close_offset + 1;
            continue;
        }

        escaped = byte == b'\\';
        index += 1;
    }

    if changed {
        push_raw_shell_line_with_normalized_redirect_spacing(target, &line[last..]);
    }
    changed
}

fn render_inline_raw_command_substitution_as_block(
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if raw.contains('\n') {
        return None;
    }

    let body = raw_dollar_command_substitution_body(raw)?.trim_matches([' ', '\t', '\r']);
    if body.is_empty() {
        return None;
    }

    let parsed = shuck_parser::parser::Parser::with_dialect(body, options.dialect()).parse();
    if parsed.is_err() {
        return None;
    }
    let inline_multiline = stmt_seq_contains_multistatement_pipeline_brace_group(&parsed.file.body)
        || raw_body_contains_pipeline_multistatement_brace_group(body);
    if parsed.file.body.len() <= 1 && !inline_multiline {
        return None;
    }

    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(body, &parsed.file.body, options, None, None, &mut nested)?;
    let trimmed = trim_trailing_line_endings(&nested);
    if trimmed.is_empty() {
        return Some("$()".to_string());
    }

    let mut rendered = String::new();
    if inline_multiline && parsed.file.body.len() == 1 {
        rendered.push_str("$(");
        push_command_substitution_inline_body(
            &mut rendered,
            trim_inline_command_substitution_padding(trimmed),
            options,
            1,
        );
        rendered.push(')');
    } else {
        rendered.push_str("$(\n");
        push_indented_rendered_block(&mut rendered, trimmed, options, 1);
        rendered.push_str("\n)");
    }
    Some(rendered)
}

fn stmt_seq_contains_multistatement_pipeline_brace_group(statements: &StmtSeq) -> bool {
    statements
        .iter()
        .any(stmt_contains_multistatement_pipeline_brace_group)
}

fn stmt_contains_multistatement_pipeline_brace_group(stmt: &Stmt) -> bool {
    match &stmt.command {
        Command::Binary(binary) if matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            stmt_contains_multistatement_pipeline_brace_group(&binary.left)
                || stmt_contains_multistatement_pipeline_brace_group(&binary.right)
        }
        Command::Compound(CompoundCommand::BraceGroup(body)) => body.len() > 1,
        _ => false,
    }
}

fn raw_body_contains_pipeline_multistatement_brace_group(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match byte {
            b'\\' if !in_single_quotes => {
                escaped = true;
                index += 1;
                continue;
            }
            b'\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
                index += 1;
                continue;
            }
            b'"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
                index += 1;
                continue;
            }
            b'|' if !in_single_quotes
                && !in_double_quotes
                && bytes.get(index + 1) != Some(&b'|') =>
            {
                let mut group_start = index + 1;
                if bytes.get(group_start) == Some(&b'&') {
                    group_start += 1;
                }
                while bytes
                    .get(group_start)
                    .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
                {
                    group_start += 1;
                }
                if bytes.get(group_start) == Some(&b'{')
                    && raw_brace_group_has_multiple_commands(&body[group_start + 1..])
                {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }

    false
}

fn raw_brace_group_has_multiple_commands(body_after_open: &str) -> bool {
    let bytes = body_after_open.as_bytes();
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;
    let mut saw_separator = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match byte {
            b'\\' if !in_single_quotes => {
                escaped = true;
            }
            b'\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
            }
            b'"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
            }
            b'}' if !in_single_quotes && !in_double_quotes => return false,
            b';' | b'\n' if !in_single_quotes && !in_double_quotes => {
                saw_separator = true;
            }
            _ if saw_separator
                && !in_single_quotes
                && !in_double_quotes
                && !matches!(byte, b' ' | b'\t' | b'\r') =>
            {
                return true;
            }
            _ => {}
        }
        index += 1;
    }

    false
}

fn raw_command_redirect_spacing_would_change(raw: &str) -> bool {
    if !(raw.contains('<') || raw.contains('>')) {
        return false;
    }
    let mut normalized = String::with_capacity(raw.len());
    push_raw_shell_text_with_normalized_redirect_spacing(&mut normalized, raw);
    normalized != raw
}

fn push_preserved_raw_word_source(
    rendered: &mut String,
    word: &Word,
    raw: &str,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    if raw.contains('<') || raw.contains('>') || raw.contains('`') {
        push_raw_word_with_normalized_command_redirect_spacing(
            rendered, word, raw, source, options,
        );
    } else {
        rendered.push_str(raw);
    }
}

fn raw_parameter_command_spacing_would_change(raw: &str) -> bool {
    raw_command_redirect_spacing_would_change(raw)
        || raw_command_substitution_needs_structural_spacing(raw)
}

fn raw_command_substitution_needs_structural_spacing(raw: &str) -> bool {
    let mut index = 0usize;

    while let Some((open_offset, close_offset)) = next_raw_command_substitution(raw, index) {
        if raw_shell_body_needs_structural_spacing(&raw[open_offset + 2..close_offset]) {
            return true;
        }
        index = close_offset + 1;
    }

    false
}

#[derive(Default)]
struct RawQuoteState {
    quote: Option<char>,
    escaped: bool,
}

impl RawQuoteState {
    fn consume(&mut self, ch: char, include_backticks: bool) -> bool {
        if self.escaped {
            self.escaped = false;
            return true;
        }

        match self.quote {
            Some('\'') => {
                if ch == '\'' {
                    self.quote = None;
                }
                true
            }
            Some('"') => {
                if ch == '"' {
                    self.quote = None;
                } else if ch == '\\' {
                    self.escaped = true;
                }
                true
            }
            Some('`') if include_backticks => {
                if ch == '`' {
                    self.quote = None;
                } else if ch == '\\' {
                    self.escaped = true;
                }
                true
            }
            _ if ch == '\\' => {
                self.escaped = true;
                true
            }
            _ if ch == '\'' || ch == '"' || (include_backticks && ch == '`') => {
                self.quote = Some(ch);
                true
            }
            _ => false,
        }
    }
}

fn raw_shell_body_needs_structural_spacing(body: &str) -> bool {
    let body = body.trim_matches([' ', '\t']);
    if raw_body_contains_pipeline_multistatement_brace_group(body) {
        return true;
    }
    let mut quote = RawQuoteState::default();
    let mut horizontal_run = 0usize;
    let mut index = 0usize;

    while index < body.len() {
        let rest = &body[index..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        let next_index = index + ch.len_utf8();

        if quote.consume(ch, true) {
            horizontal_run = 0;
            index = next_index;
            continue;
        }

        if rest.starts_with("$(")
            && !rest.starts_with("$((")
            && let Some(close_offset) = matching_raw_command_substitution_close(body, index + 2)
        {
            if raw_shell_body_needs_structural_spacing(&body[index + 2..close_offset]) {
                return true;
            }
            horizontal_run = 0;
            index = close_offset + 1;
            continue;
        }

        match ch {
            ' ' | '\t' | '\r' => {
                if ch != ' ' {
                    return true;
                }
                horizontal_run += 1;
                if horizontal_run > 1 {
                    return true;
                }
            }
            '|' if !rest.starts_with("||") => {
                let op_len = if rest.starts_with("|&") { 2 } else { 1 };
                let previous_is_space = body[..index]
                    .chars()
                    .next_back()
                    .is_some_and(|previous| matches!(previous, ' ' | '\t' | '\r'));
                let next_is_space = body[index + op_len..]
                    .chars()
                    .next()
                    .is_some_and(|next| matches!(next, ' ' | '\t' | '\r'));
                if !previous_is_space || !next_is_space {
                    return true;
                }
                horizontal_run = 0;
            }
            ';' if !rest.starts_with(";;") => return true,
            _ => horizontal_run = 0,
        }

        index = next_index;
    }

    false
}

fn normalize_raw_command_substitution_padding(raw: &str) -> Option<String> {
    let mut rendered = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    let mut changed = false;

    while let Some((open_offset, close_offset)) = next_raw_command_substitution(raw, index) {
        let body = &raw[open_offset + 2..close_offset];
        if !body.contains('\n') {
            let trimmed = trim_raw_command_substitution_horizontal_padding(body);
            let normalized_body = normalize_raw_command_substitution_padding(trimmed)
                .unwrap_or_else(|| trimmed.to_string());
            if trimmed.len() != body.len() || normalized_body != trimmed {
                rendered.push_str(&raw[cursor..open_offset]);
                rendered.push_str("$(");
                if normalized_body.starts_with('(') {
                    rendered.push(' ');
                }
                rendered.push_str(&normalized_body);
                rendered.push(')');
                cursor = close_offset + 1;
                changed = true;
            }
        }
        index = close_offset + 1;
    }

    finish_raw_rewrite(rendered, raw, cursor, changed)
}

fn trim_raw_command_substitution_horizontal_padding(body: &str) -> &str {
    trim_unescaped_trailing_whitespace(body.trim_start_matches([' ', '\t']))
}

pub(crate) fn normalize_raw_empty_parameter_replacement_delimiters(raw: &str) -> Option<String> {
    if !raw.contains("${") {
        return None;
    }

    let bytes = raw.as_bytes();
    let mut rendered = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    let mut changed = false;

    while index + 1 < bytes.len() {
        if bytes[index] == b'$'
            && bytes[index + 1] == b'{'
            && index
                .checked_sub(1)
                .and_then(|previous| bytes.get(previous))
                .is_none_or(|byte| *byte != b'\\')
            && let Some(close_offset) = matching_raw_parameter_expansion_close(raw, index + 2)
        {
            let body = &raw[index + 2..close_offset];
            if raw_parameter_replacement_needs_empty_delimiter(body) {
                rendered.push_str(&raw[cursor..close_offset]);
                rendered.push('/');
                cursor = close_offset;
                changed = true;
            }
            index = close_offset + 1;
            continue;
        }
        index += 1;
    }

    finish_raw_rewrite(rendered, raw, cursor, changed)
}

fn matching_raw_parameter_expansion_close(raw: &str, body_start: usize) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut depth = 1usize;
    let mut escaped = false;
    let mut index = body_start;

    while index < bytes.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }

        match bytes[index] {
            b'\\' => escaped = true,
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }

    None
}

fn raw_parameter_replacement_needs_empty_delimiter(body: &str) -> bool {
    let Some(after_operator) = raw_parameter_replacement_body_after_operator(body) else {
        return false;
    };
    let (_, replacement) = split_raw_parameter_replacement(after_operator);
    if replacement.is_empty() {
        return !raw_has_final_replacement_delimiter(after_operator);
    }

    replacement_ends_with_ambiguous_quote(replacement)
}

fn raw_parameter_replacement_body_after_operator(body: &str) -> Option<&str> {
    let mut index = body.strip_prefix('!').map_or(0, |_| 1);
    let bytes = body.as_bytes();
    if index >= bytes.len() {
        return None;
    }

    if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
        index += 1;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
    } else if bytes[index].is_ascii_digit()
        || matches!(
            bytes[index],
            b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!' | b'0'
        )
    {
        index += 1;
    } else {
        return None;
    }

    if bytes.get(index) == Some(&b'[') {
        index = raw_parameter_subscript_end(body, index)?;
    }

    body.get(index..)
        .and_then(|rest| rest.strip_prefix("//").or_else(|| rest.strip_prefix('/')))
}

fn raw_parameter_subscript_end(body: &str, open: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut depth = 1usize;
    let mut escaped = false;
    let mut index = open + 1;
    while index < bytes.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\\' => escaped = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn raw_has_final_replacement_delimiter(after_operator: &str) -> bool {
    let Some((last_index, _)) = after_operator.char_indices().next_back() else {
        return false;
    };
    after_operator[last_index..].starts_with('/')
        && !raw_char_is_escaped(after_operator, last_index)
}

fn raw_char_is_escaped(raw: &str, index: usize) -> bool {
    let mut backslashes = 0usize;
    for ch in raw[..index].chars().rev() {
        if ch == '\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

fn replacement_ends_with_ambiguous_quote(replacement: &str) -> bool {
    if replacement.ends_with('\'') {
        return replacement[..replacement.len() - '\''.len_utf8()].ends_with('\\');
    }
    if replacement.ends_with('"') {
        let quote_index = replacement.len() - '"'.len_utf8();
        let backslashes = replacement[..quote_index]
            .chars()
            .rev()
            .take_while(|ch| *ch == '\\')
            .count();
        return backslashes > 0 && backslashes % 2 == 0;
    }
    false
}

fn normalize_raw_arithmetic_command_substitution_padding(raw: &str) -> Option<String> {
    let (open, close) = if raw.starts_with("$((") && raw.ends_with("))") {
        ("$((", "))")
    } else if raw.starts_with("$[") && raw.ends_with(']') {
        ("$[", "]")
    } else {
        return None;
    };
    let body_start = open.len();
    let body_end = raw.len().saturating_sub(close.len());
    let body = raw.get(body_start..body_end)?;
    if !(body.contains("$(") || body.contains('`')) {
        return None;
    }
    let trimmed = body.trim_matches([' ', '\t', '\r']);
    if trimmed.len() == body.len() {
        return None;
    }

    let mut rendered = String::with_capacity(raw.len());
    rendered.push_str(open);
    rendered.push_str(trimmed);
    rendered.push_str(close);
    Some(rendered)
}

fn normalize_raw_arithmetic_expansion_padding(raw: &str) -> Option<String> {
    let mut rendered = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    let mut changed = false;

    while index + 2 < raw.len() {
        let rest = &raw[index..];
        if rest.starts_with("$((")
            && index
                .checked_sub(1)
                .and_then(|previous| raw.as_bytes().get(previous))
                .is_none_or(|byte| *byte != b'\\')
            && let Some(close_start) = matching_raw_arithmetic_expansion_close(raw, index + 3)
        {
            let body = &raw[index + 3..close_start];
            let trimmed = body.trim_matches([' ', '\t', '\r']);
            if trimmed.len() != body.len() {
                rendered.push_str(&raw[cursor..index]);
                rendered.push_str("$((");
                rendered.push_str(trimmed);
                rendered.push_str("))");
                cursor = close_start + 2;
                changed = true;
            }
            index = close_start + 2;
            continue;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        index += ch.len_utf8();
    }

    finish_raw_rewrite(rendered, raw, cursor, changed)
}

fn matching_raw_arithmetic_expansion_close(raw: &str, body_start: usize) -> Option<usize> {
    let mut quote = RawQuoteState::default();
    let mut paren_depth = 0usize;
    let mut index = body_start;

    while index < raw.len() {
        let rest = &raw[index..];
        let ch = rest.chars().next()?;
        let next_index = index + ch.len_utf8();
        if quote.consume(ch, true) {
            index = next_index;
            continue;
        }

        if rest.starts_with("$(")
            && !rest.starts_with("$((")
            && let Some(close_offset) = matching_raw_command_substitution_close(raw, index + 2)
        {
            index = close_offset + 1;
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' if rest.starts_with("))") && paren_depth == 0 => return Some(index),
            ')' if paren_depth > 0 => paren_depth -= 1,
            _ => {}
        }

        index = next_index;
    }

    None
}

pub(crate) fn matching_raw_command_substitution_close(
    raw: &str,
    body_start: usize,
) -> Option<usize> {
    let mut quote = RawQuoteState::default();
    let mut paren_depth = 0usize;
    let mut index = body_start;

    while index < raw.len() {
        let ch = raw[index..].chars().next()?;
        let next_index = index + ch.len_utf8();
        if quote.consume(ch, false) {
            index = next_index;
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth == 0 {
                    return Some(index);
                }
                paren_depth -= 1;
            }
            _ => {}
        }

        index = next_index;
    }

    None
}

fn push_raw_shell_line_with_normalized_redirect_spacing(target: &mut String, line: &str) {
    let mut last = 0;
    let mut index = 0;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;
    let bytes = line.as_bytes();

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\'' && !in_double_quotes && !escaped {
            in_single_quotes = !in_single_quotes;
            index += 1;
            continue;
        }
        if byte == b'"' && !in_single_quotes && !escaped {
            in_double_quotes = !in_double_quotes;
            index += 1;
            continue;
        }
        if !in_single_quotes && !in_double_quotes && byte == b'#' {
            break;
        }

        if !in_single_quotes
            && !escaped
            && byte == b'$'
            && bytes.get(index + 1) == Some(&b'(')
            && bytes.get(index + 2) != Some(&b'(')
            && let Some(close_offset) = matching_raw_command_substitution_close(line, index + 2)
        {
            target.push_str(&line[last..index]);
            target.push_str("$(");
            push_raw_shell_text_with_normalized_redirect_spacing(
                target,
                &line[index + 2..close_offset],
            );
            target.push(')');
            last = close_offset + 1;
            index = close_offset + 1;
            escaped = false;
            continue;
        }

        if !in_single_quotes && !in_double_quotes && matches!(byte, b' ' | b'\t' | b'\r') {
            let whitespace_start = index;
            let mut semicolon_start = index + 1;
            while semicolon_start < bytes.len()
                && matches!(bytes[semicolon_start], b' ' | b'\t' | b'\r')
            {
                semicolon_start += 1;
            }
            if bytes.get(semicolon_start) == Some(&b';')
                && raw_semicolon_can_attach_to_previous_word(bytes, whitespace_start)
                && raw_semicolon_is_single_terminator(bytes, semicolon_start)
            {
                target.push_str(&line[last..whitespace_start]);
                last = semicolon_start;
                index = semicolon_start;
                escaped = false;
                continue;
            }
        }

        if !in_single_quotes && !in_double_quotes && byte.is_ascii_digit() {
            let fd_start = index;
            let mut operator_start = index + 1;
            while operator_start < bytes.len() && bytes[operator_start].is_ascii_digit() {
                operator_start += 1;
            }
            if let Some(operator_end) = redirect_operator_end(bytes, operator_start) {
                let mut target_start = operator_end;
                while target_start < bytes.len()
                    && matches!(bytes[target_start], b' ' | b'\t' | b'\r')
                {
                    target_start += 1;
                }
                if target_start > operator_end && target_start < bytes.len() {
                    target.push_str(&line[last..operator_end]);
                    last = target_start;
                    index = target_start;
                    escaped = false;
                    continue;
                }
            }
            index = fd_start;
        }

        if !in_single_quotes
            && !in_double_quotes
            && matches!(byte, b'<' | b'>')
            && let Some(operator_end) = redirect_operator_end(bytes, index)
        {
            let mut target_start = operator_end;
            while target_start < bytes.len() && matches!(bytes[target_start], b' ' | b'\t' | b'\r')
            {
                target_start += 1;
            }
            if target_start > operator_end
                && target_start < bytes.len()
                && raw_redirect_target_spacing_can_be_stripped(bytes, index, target_start)
            {
                target.push_str(&line[last..operator_end]);
                last = target_start;
                index = target_start;
                escaped = false;
                continue;
            }
        }

        if !in_single_quotes && !in_double_quotes && bytes.get(index..index + 3) == Some(b"<<<") {
            let operator_end = index + 3;
            let mut target_start = operator_end;
            while target_start < bytes.len() && matches!(bytes[target_start], b' ' | b'\t' | b'\r')
            {
                target_start += 1;
            }
            if target_start > operator_end && target_start < bytes.len() {
                target.push_str(&line[last..operator_end]);
                last = target_start;
                index = target_start;
                escaped = false;
                continue;
            }
        }

        escaped = !in_single_quotes && byte == b'\\' && !escaped;
        if byte != b'\\' {
            escaped = false;
        }
        index += 1;
    }

    target.push_str(&line[last..]);
}

fn raw_semicolon_can_attach_to_previous_word(bytes: &[u8], whitespace_start: usize) -> bool {
    bytes
        .get(..whitespace_start)
        .and_then(|prefix| {
            prefix
                .iter()
                .rev()
                .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r'))
                .copied()
        })
        .is_some_and(|byte| !matches!(byte, b';' | b'('))
}

fn raw_semicolon_is_single_terminator(bytes: &[u8], semicolon_start: usize) -> bool {
    !matches!(
        bytes.get(semicolon_start + 1).copied(),
        Some(b';' | b'&' | b'|')
    )
}

fn raw_compound_close_keyword(content: &str) -> Option<&'static str> {
    let trimmed = content.trim_end_matches([' ', '\t', '\r']);
    if trimmed == "{" || trimmed.ends_with(" {") || trimmed.ends_with("; {") {
        return Some("}");
    }
    if raw_line_starts_with_keyword(trimmed, "for")
        || raw_line_starts_with_keyword(trimmed, "select")
        || raw_line_starts_with_keyword(trimmed, "while")
        || raw_line_starts_with_keyword(trimmed, "until")
    {
        return raw_line_ends_with_keyword(trimmed, "do").then_some("done");
    }
    if raw_line_starts_with_keyword(trimmed, "if") {
        return raw_line_ends_with_keyword(trimmed, "then").then_some("fi");
    }
    if raw_line_starts_with_keyword(trimmed, "case") {
        return raw_line_ends_with_keyword(trimmed, "in").then_some("esac");
    }
    None
}

fn raw_line_closes_compound(content: &str, close_keyword: &str) -> bool {
    raw_line_starts_with_keyword(content.trim_start_matches([' ', '\t', '\r']), close_keyword)
}

fn raw_line_is_compound_mid_keyword(content: &str) -> bool {
    let content = content.trim_start_matches([' ', '\t', '\r']);
    raw_line_starts_with_keyword(content, "else")
        || raw_line_starts_with_keyword(content, "elif")
        || raw_line_starts_with_keyword(content, "then")
        || raw_line_starts_with_keyword(content, "do")
}

fn raw_line_starts_with_keyword(line: &str, keyword: &str) -> bool {
    let Some(rest) = line.strip_prefix(keyword) else {
        return false;
    };
    rest.is_empty()
        || rest
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b';' | b'|' | b'&'))
}

fn raw_line_ends_with_keyword(line: &str, keyword: &str) -> bool {
    let Some(prefix) = line.strip_suffix(keyword) else {
        return false;
    };
    prefix.is_empty()
        || prefix
            .as_bytes()
            .last()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b';' | b'|'))
}

fn raw_line_closes_inline_brace_group_before_pipeline(content: &str) -> bool {
    let trimmed = content.trim_end_matches([' ', '\t', '\r']);
    let before_operator = if let Some(prefix) = trimmed.strip_suffix("|&") {
        prefix
    } else if let Some(prefix) = trimmed.strip_suffix('|') {
        prefix
    } else {
        return false;
    };
    before_operator
        .trim_end_matches([' ', '\t', '\r'])
        .ends_with('}')
}

fn raw_redirect_target_spacing_can_be_stripped(
    bytes: &[u8],
    operator_start: usize,
    target_start: usize,
) -> bool {
    if !matches!(bytes.get(operator_start), Some(b'<' | b'>')) {
        return true;
    }
    if bytes.get(operator_start) == bytes.get(target_start)
        && bytes.get(target_start + 1) == Some(&b'(')
    {
        return false;
    }
    !bytes
        .get(target_start)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn source_indent_units_before_offset(
    source: &str,
    offset: usize,
    options: &ResolvedShellFormatOptions,
) -> usize {
    let Some(indent) = line_indent_before_source_offset(source, offset) else {
        return 0;
    };
    raw_indent_units(indent, options)
}

fn raw_indent_units(indent: &str, options: &ResolvedShellFormatOptions) -> usize {
    let normalized = normalized_raw_shell_indent(indent, options);
    let width = usize::from(options.indent_width()).max(1);
    match options.indent_style() {
        IndentStyle::Tab => {
            normalized.chars().filter(|ch| *ch == '\t').count()
                + normalized.chars().filter(|ch| *ch == ' ').count() / width
        }
        IndentStyle::Space => normalized.len() / width,
    }
}

fn strip_one_indent_unit<'a>(line: &'a str, options: &ResolvedShellFormatOptions) -> &'a str {
    match options.indent_style() {
        IndentStyle::Tab => line.strip_prefix('\t').unwrap_or_else(|| {
            line.strip_prefix(&" ".repeat(usize::from(options.indent_width())))
                .unwrap_or(line)
        }),
        IndentStyle::Space => line
            .strip_prefix(&" ".repeat(usize::from(options.indent_width())))
            .unwrap_or(line),
    }
}

fn strip_outer_indent_or_one_unit<'a>(
    line: &'a str,
    outer_indent: &str,
    options: &ResolvedShellFormatOptions,
) -> &'a str {
    if outer_indent.is_empty() {
        return strip_one_indent_unit(line, options);
    }
    line.strip_prefix(outer_indent)
        .unwrap_or_else(|| strip_one_indent_unit(line, options))
}

fn source_indent_minus_one_unit(indent: &str, options: &ResolvedShellFormatOptions) -> String {
    match options.indent_style() {
        IndentStyle::Tab if indent.ends_with('\t') => {
            let mut shortened = indent.to_string();
            shortened.pop();
            shortened
        }
        _ => {
            let width = usize::from(options.indent_width()).max(1);
            if indent.ends_with(&" ".repeat(width)) {
                indent[..indent.len().saturating_sub(width)].to_string()
            } else if indent.ends_with('\t') {
                let mut shortened = indent.to_string();
                shortened.pop();
                shortened
            } else {
                indent.to_string()
            }
        }
    }
}

fn source_indent_plus_one_unit(indent: &str, options: &ResolvedShellFormatOptions) -> String {
    if indent.chars().all(|ch| ch == '\t') {
        let mut extended = indent.to_string();
        extended.push('\t');
        extended
    } else {
        let width = match options.indent_style() {
            IndentStyle::Tab => usize::from(options.indent_width()).clamp(1, 4),
            IndentStyle::Space => usize::from(options.indent_width()),
        };
        let mut extended = indent.to_string();
        extended.push_str(&" ".repeat(width));
        extended
    }
}

#[allow(clippy::too_many_arguments)]
fn render_process_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    is_input: bool,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
    multiline: bool,
    raw: Option<&str>,
    facts: Option<&FormatterFacts<'_>>,
) -> Option<()> {
    let has_heredoc = stmt_seq_has_heredoc(body);
    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(
        source,
        body,
        options,
        facts,
        span.end.offset.checked_sub(1),
        &mut nested,
    )?;

    let prefix = if is_input { '<' } else { '>' };
    let trimmed = trim_trailing_line_endings(&nested);
    if trimmed.is_empty() {
        rendered.push(prefix);
        rendered.push_str("()");
        return Some(());
    }

    let rendered_multiline = trimmed.contains('\n');
    if multiline || has_heredoc || rendered_multiline {
        if rendered_multiline
            && !has_heredoc
            && raw.is_some_and(process_substitution_source_starts_with_inline_brace_group)
        {
            rendered.push(prefix);
            rendered.push('(');
            rendered.push_str(trimmed);
            rendered.push(')');
        } else if let Some(raw) = raw
            && process_substitution_source_starts_with_body_line(raw)
            && raw.contains('\n')
            && !substitution_source_closes_on_own_line(raw)
        {
            rendered.push(prefix);
            rendered.push('(');
            push_source_indented_inline_command_substitution(rendered, trimmed, raw, options);
            rendered.push(')');
        } else {
            let outer_levels =
                source_indent_units_before_offset(source, span.start.offset, options);
            rendered.push(prefix);
            rendered.push_str("(\n");
            push_indented_rendered_block(rendered, trimmed, options, outer_levels + 1);
            rendered.push('\n');
            options.push_indent_units(rendered, outer_levels);
            rendered.push(')');
        }
    } else {
        rendered.push(prefix);
        rendered.push('(');
        rendered.push_str(trimmed);
        rendered.push(')');
    }

    Some(())
}

fn process_substitution_source_starts_with_inline_brace_group(raw: &str) -> bool {
    raw.get(2..).is_some_and(|body| {
        (raw.starts_with("<(") || raw.starts_with(">("))
            && !body.starts_with(['\n', '\r'])
            && body.trim_start_matches([' ', '\t']).starts_with('{')
    })
}

fn process_substitution_source_starts_with_body_line(raw: &str) -> bool {
    raw.get(2..).is_some_and(|body| {
        (raw.starts_with("<(") || raw.starts_with(">(")) && !body.starts_with('\n')
    })
}

fn process_substitution_source_opens_to_body_line(raw: &str) -> bool {
    raw.get(2..).is_some_and(|body| {
        (raw.starts_with("<(") || raw.starts_with(">(")) && body.starts_with(['\n', '\r'])
    })
}

fn substitution_source_closes_on_own_line(raw: &str) -> bool {
    let Some(close_offset) = raw.rfind(')') else {
        return false;
    };
    let line_start = raw[..close_offset]
        .rfind('\n')
        .map_or(0, |newline| newline.saturating_add(1));
    line_start > 0 && raw[line_start..close_offset].trim().is_empty()
}

fn trim_trailing_line_endings(rendered: &str) -> &str {
    rendered.trim_end_matches(&['\r', '\n'][..])
}

fn push_source_indented_inline_command_substitution(
    target: &mut String,
    rendered: &str,
    raw: &str,
    options: &ResolvedShellFormatOptions,
) {
    let raw_indents = raw
        .lines()
        .skip(1)
        .map(line_leading_shell_indent)
        .map(|indent| normalized_source_inline_indent(indent, options))
        .collect::<Vec<_>>();
    let fallback_indent = raw_indents.first().map(String::as_str).unwrap_or("");
    for (index, line) in rendered.lines().enumerate() {
        if index > 0 {
            target.push('\n');
            let indent = raw_indents
                .get(index - 1)
                .map(String::as_str)
                .unwrap_or(fallback_indent);
            target.push_str(indent);
        }
        if index == 0 {
            target.push_str(line);
        } else {
            target.push_str(line.trim_start_matches([' ', '\t']));
        }
    }
}

fn normalized_source_inline_indent(indent: &str, options: &ResolvedShellFormatOptions) -> String {
    match options.indent_style() {
        IndentStyle::Tab if indent.chars().all(|ch| ch == ' ') => {
            let unit = usize::from(options.indent_width()).clamp(1, 4);
            if indent.len().is_multiple_of(unit) {
                "\t".repeat(indent.len() / unit)
            } else {
                indent.to_string()
            }
        }
        IndentStyle::Space if indent.chars().all(|ch| ch == '\t') => {
            " ".repeat(indent.len() * usize::from(options.indent_width()))
        }
        _ => indent.to_string(),
    }
}

fn normalized_raw_shell_indent(indent: &str, options: &ResolvedShellFormatOptions) -> String {
    match options.indent_style() {
        IndentStyle::Tab if !indent.is_empty() && indent.chars().all(|ch| ch == ' ') => {
            let unit = usize::from(options.indent_width()).clamp(1, 4);
            "\t".repeat(indent.len().div_ceil(unit))
        }
        _ => normalized_source_inline_indent(indent, options),
    }
}

fn push_indented_rendered_block(
    target: &mut String,
    rendered: &str,
    options: &ResolvedShellFormatOptions,
    levels: usize,
) {
    let prefix = options.indent_prefix(levels);
    let normalized_literal_continuations =
        normalize_literal_continuation_indent_for_block(rendered);
    let rendered = normalized_literal_continuations
        .as_deref()
        .unwrap_or(rendered);
    let common_source_indent = common_rendered_block_indent(rendered, options);

    let mut active_heredoc: Option<CommandSubstitutionHeredocIndent> = None;
    for (index, line) in rendered.lines().enumerate() {
        if index > 0 {
            target.push('\n');
        }

        if let Some(heredoc) = active_heredoc.as_ref() {
            let closes = heredoc_line_closes_command_substitution_heredoc(line, heredoc);
            if heredoc.strip_tabs {
                if closes {
                    target.push_str(&prefix);
                    target.push_str(&heredoc.command_indent);
                    target.push_str(line.trim_start_matches('\t'));
                    active_heredoc = None;
                    continue;
                }
                if line_needs_command_substitution_indent(line, options) {
                    target.push_str(&prefix);
                }
            }
            target.push_str(line);
            if closes {
                active_heredoc = None;
            }
            continue;
        }

        let line = strip_common_rendered_block_indent(line, &common_source_indent);
        if line_needs_command_substitution_indent(line, options) {
            target.push_str(&prefix);
        }
        target.push_str(line);
        active_heredoc = command_substitution_heredoc_indent(line);
    }
}

fn normalize_literal_continuation_indent_for_block(rendered: &str) -> Option<String> {
    if !rendered.contains('\n') {
        return None;
    }

    let mut quote = RawShellQuoteState::default();
    let mut continuation_indent: Option<String> = None;
    let mut normalized = String::with_capacity(rendered.len());
    let mut changed = false;

    for (index, line) in rendered.split('\n').enumerate() {
        if index > 0 {
            normalized.push('\n');
        }

        let mut line = line.to_string();
        if let Some(indent) = continuation_indent.take()
            && !line.trim().is_empty()
        {
            let content = line.trim_start_matches([' ', '\t']);
            if line_leading_shell_indent(&line) != indent {
                line = format!("{indent}{content}");
                changed = true;
            }
        }

        let line_continues = line_without_continuation_backslash(&line).is_some();
        quote.scan_line(&line);
        continuation_indent = (line_continues && quote.in_multiline_literal())
            .then(|| line_leading_shell_indent(&line).to_string());
        normalized.push_str(&line);
    }

    changed.then_some(normalized)
}

fn common_rendered_block_indent(rendered: &str, options: &ResolvedShellFormatOptions) -> String {
    let mut active_heredoc: Option<CommandSubstitutionHeredocIndent> = None;
    let mut common: Option<String> = None;

    for line in rendered.lines() {
        if let Some(heredoc) = active_heredoc.as_ref() {
            if heredoc_line_closes_command_substitution_heredoc(line, heredoc) {
                active_heredoc = None;
            }
            continue;
        }

        if line_needs_command_substitution_indent(line, options) {
            let indent = line_leading_shell_indent(line);
            if indent.is_empty() {
                return String::new();
            }
            if refine_common_indent(&mut common, indent) {
                return String::new();
            }
        }

        active_heredoc = command_substitution_heredoc_indent(line);
    }

    common.unwrap_or_default()
}

fn strip_common_rendered_block_indent<'a>(line: &'a str, common_indent: &str) -> &'a str {
    if common_indent.is_empty() {
        line
    } else {
        line.strip_prefix(common_indent).unwrap_or(line)
    }
}

#[derive(Debug, Clone)]
struct CommandSubstitutionHeredocIndent {
    delimiter: String,
    strip_tabs: bool,
    command_indent: String,
}

fn command_substitution_heredoc_indent(line: &str) -> Option<CommandSubstitutionHeredocIndent> {
    let start = heredoc_start(line)?;
    Some(CommandSubstitutionHeredocIndent {
        delimiter: start.delimiter.to_string(),
        strip_tabs: start.strip_tabs,
        command_indent: line_leading_shell_indent(line).to_string(),
    })
}

fn heredoc_line_closes_command_substitution_heredoc(
    line: &str,
    heredoc: &CommandSubstitutionHeredocIndent,
) -> bool {
    if heredoc.strip_tabs {
        line.trim_start_matches('\t') == heredoc.delimiter
    } else {
        line == heredoc.delimiter
    }
}

fn line_needs_command_substitution_indent(
    line: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if line.is_empty() {
        return false;
    }

    match options.indent_style() {
        // Leave literal multiline string continuation lines alone. Formatter-
        // produced shell indentation already uses tabs in this mode.
        IndentStyle::Tab => !line.starts_with(' '),
        IndentStyle::Space => true,
    }
}

fn render_double_quoted_literal(rendered: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '"' | '\\' | '$' | '`' => {
                rendered.push('\\');
                rendered.push(ch);
            }
            _ => rendered.push(ch),
        }
    }
}

pub(crate) fn render_arithmetic_expr_to_buf(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    push_arithmetic_expr(
        rendered,
        expr,
        ArithmeticContext::TopLevel,
        false,
        WordRenderEnv::new(source, options, None, None),
    );
}

fn render_arithmetic_subscript_expr_to_buf(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    source: &str,
    options: &ResolvedShellFormatOptions,
    compact: bool,
) {
    push_arithmetic_expr(
        rendered,
        expr,
        ArithmeticContext::Subscript,
        compact,
        WordRenderEnv::new(source, options, None, None),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArithmeticContext {
    TopLevel,
    Unary,
    Postfix,
    Binary(ArithmeticBinaryOp),
    Assignment,
    ConditionalCondition,
    ConditionalBranch,
    Subscript,
}

#[derive(Clone, Copy)]
struct WordRenderEnv<'source, 'a> {
    source: &'source str,
    options: &'a ResolvedShellFormatOptions,
    source_map: Option<&'a SourceMap<'source>>,
    facts: Option<&'a FormatterFacts<'source>>,
}

impl<'source, 'a> WordRenderEnv<'source, 'a> {
    fn new(
        source: &'source str,
        options: &'a ResolvedShellFormatOptions,
        source_map: Option<&'a SourceMap<'source>>,
        facts: Option<&'a FormatterFacts<'source>>,
    ) -> Self {
        Self {
            source,
            options,
            source_map,
            facts,
        }
    }
}

fn push_arithmetic_expr(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    context: ArithmeticContext,
    compact: bool,
    env: WordRenderEnv<'_, '_>,
) {
    let needs_parentheses = arithmetic_needs_parentheses(expr, context);
    if needs_parentheses {
        rendered.push('(');
    }

    match &expr.kind {
        ArithmeticExpr::Number(number) => rendered.push_str(number.slice(env.source)),
        ArithmeticExpr::Variable(name) => rendered.push_str(name),
        ArithmeticExpr::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(rendered, index, ArithmeticContext::Subscript, true, env);
            rendered.push(']');
        }
        ArithmeticExpr::ShellWord(word) => {
            let word = render_arithmetic_shell_word(
                word,
                env.source,
                env.options,
                env.source_map,
                env.facts,
            );
            if compact {
                rendered.push_str(&compact_dynamic_arithmetic_subscript(
                    word.trim_matches([' ', '\t', '\r']),
                ));
            } else {
                rendered.push_str(&word);
            }
        }
        ArithmeticExpr::Parenthesized { expression } => {
            rendered.push('(');
            push_arithmetic_expr(
                rendered,
                expression,
                ArithmeticContext::TopLevel,
                compact,
                env,
            );
            rendered.push(')');
        }
        ArithmeticExpr::Unary { op, expr } => {
            rendered.push_str(arithmetic_unary_operator(*op));
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Unary, compact, env);
        }
        ArithmeticExpr::Postfix { expr, op } => {
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Postfix, compact, env);
            rendered.push_str(arithmetic_postfix_operator(*op));
        }
        ArithmeticExpr::Binary { left, op, right } => {
            push_arithmetic_expr(rendered, left, ArithmeticContext::Binary(*op), compact, env);
            if !compact {
                rendered.push(' ');
            }
            rendered.push_str(arithmetic_binary_operator(*op));
            if !compact {
                rendered.push(' ');
            }
            push_arithmetic_expr(
                rendered,
                right,
                ArithmeticContext::Binary(*op),
                compact,
                env,
            );
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            push_arithmetic_expr(
                rendered,
                condition,
                ArithmeticContext::ConditionalCondition,
                compact,
                env,
            );
            rendered.push_str(if compact { "?" } else { " ? " });
            push_arithmetic_expr(
                rendered,
                then_expr,
                ArithmeticContext::ConditionalBranch,
                compact,
                env,
            );
            rendered.push_str(if compact { ":" } else { " : " });
            push_arithmetic_expr(
                rendered,
                else_expr,
                ArithmeticContext::ConditionalBranch,
                compact,
                env,
            );
        }
        ArithmeticExpr::Assignment { target, op, value } => {
            push_arithmetic_lvalue(rendered, target, env);
            if !compact {
                rendered.push(' ');
            }
            rendered.push_str(arithmetic_assign_operator(*op));
            if !compact {
                rendered.push(' ');
            }
            push_arithmetic_expr(rendered, value, ArithmeticContext::Assignment, compact, env);
        }
    }

    if needs_parentheses {
        rendered.push(')');
    }
}

fn push_arithmetic_expansion_body(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    env: WordRenderEnv<'_, '_>,
) {
    let mut body = String::new();
    push_arithmetic_expr(&mut body, expr, ArithmeticContext::TopLevel, false, env);
    if body.contains("$(")
        || body.contains('`')
        || arithmetic_expr_contains_command_substitution(expr)
    {
        rendered.push_str(body.trim_matches([' ', '\t', '\r']));
    } else {
        rendered.push_str(&body);
    }
}

fn arithmetic_expr_contains_command_substitution(expr: &ArithmeticExprNode) -> bool {
    match &expr.kind {
        ArithmeticExpr::ShellWord(word) => word.parts.iter().any(|part| {
            matches!(
                part.kind,
                WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. }
            )
        }),
        ArithmeticExpr::Indexed { index, .. } => {
            arithmetic_expr_contains_command_substitution(index)
        }
        ArithmeticExpr::Parenthesized { expression } => {
            arithmetic_expr_contains_command_substitution(expression)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            arithmetic_expr_contains_command_substitution(expr)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            arithmetic_expr_contains_command_substitution(left)
                || arithmetic_expr_contains_command_substitution(right)
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            arithmetic_expr_contains_command_substitution(condition)
                || arithmetic_expr_contains_command_substitution(then_expr)
                || arithmetic_expr_contains_command_substitution(else_expr)
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            arithmetic_lvalue_contains_command_substitution(target)
                || arithmetic_expr_contains_command_substitution(value)
        }
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => false,
    }
}

fn arithmetic_lvalue_contains_command_substitution(target: &ArithmeticLvalue) -> bool {
    match target {
        ArithmeticLvalue::Variable(_) => false,
        ArithmeticLvalue::Indexed { index, .. } => {
            arithmetic_expr_contains_command_substitution(index)
        }
    }
}

fn render_arithmetic_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> String {
    let render_with_context = || {
        let mut rendered = String::new();
        render_word_syntax_internal(
            word,
            source,
            options,
            source_map,
            facts,
            true,
            &mut rendered,
        );
        rendered
    };

    if options.simplify() || options.minify() {
        let [part] = word.parts.as_slice() else {
            return render_with_context();
        };

        return match &part.kind {
            WordPart::Variable(name) => name.to_string(),
            WordPart::ArrayAccess(reference) if reference.subscript.is_none() => {
                reference.name.to_string()
            }
            WordPart::Parameter(parameter)
                if is_plain_arithmetic_identifier(parameter.raw_body.slice(source)) =>
            {
                parameter.raw_body.slice(source).to_string()
            }
            _ => render_with_context(),
        };
    }

    render_with_context()
}

fn is_plain_arithmetic_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn arithmetic_needs_parentheses(expr: &ArithmeticExprNode, context: ArithmeticContext) -> bool {
    let expr_prec = arithmetic_precedence(expr);
    match context {
        ArithmeticContext::TopLevel
        | ArithmeticContext::Subscript
        | ArithmeticContext::ConditionalCondition => false,
        ArithmeticContext::Unary | ArithmeticContext::Postfix => {
            expr_prec < arithmetic_precedence_value(ArithmeticBinaryOp::Power)
        }
        ArithmeticContext::Assignment => expr_prec <= 2,
        ArithmeticContext::ConditionalBranch => expr_prec <= 1,
        ArithmeticContext::Binary(parent_op) => {
            let parent_prec = arithmetic_precedence_value(parent_op);
            expr_prec < parent_prec
                || matches!(
                    expr.kind,
                    ArithmeticExpr::Assignment { .. } | ArithmeticExpr::Conditional { .. }
                ) && expr_prec == parent_prec
        }
    }
}

fn arithmetic_precedence(expr: &ArithmeticExprNode) -> u8 {
    match &expr.kind {
        ArithmeticExpr::Number(_)
        | ArithmeticExpr::Variable(_)
        | ArithmeticExpr::Indexed { .. }
        | ArithmeticExpr::ShellWord(_)
        | ArithmeticExpr::Parenthesized { .. } => 100,
        ArithmeticExpr::Postfix { .. } => 90,
        ArithmeticExpr::Unary { .. } => 80,
        ArithmeticExpr::Binary { op, .. } => arithmetic_precedence_value(*op),
        ArithmeticExpr::Conditional { .. } => 2,
        ArithmeticExpr::Assignment { .. } => 1,
    }
}

fn arithmetic_precedence_value(op: ArithmeticBinaryOp) -> u8 {
    match op {
        ArithmeticBinaryOp::Comma => 0,
        ArithmeticBinaryOp::LogicalOr => 10,
        ArithmeticBinaryOp::LogicalAnd => 20,
        ArithmeticBinaryOp::BitwiseOr => 30,
        ArithmeticBinaryOp::BitwiseXor => 40,
        ArithmeticBinaryOp::BitwiseAnd => 50,
        ArithmeticBinaryOp::Equal | ArithmeticBinaryOp::NotEqual => 60,
        ArithmeticBinaryOp::LessThan
        | ArithmeticBinaryOp::LessThanOrEqual
        | ArithmeticBinaryOp::GreaterThan
        | ArithmeticBinaryOp::GreaterThanOrEqual => 70,
        ArithmeticBinaryOp::ShiftLeft | ArithmeticBinaryOp::ShiftRight => 75,
        ArithmeticBinaryOp::Add | ArithmeticBinaryOp::Subtract => 80,
        ArithmeticBinaryOp::Multiply | ArithmeticBinaryOp::Divide | ArithmeticBinaryOp::Modulo => {
            85
        }
        ArithmeticBinaryOp::Power => 95,
    }
}

fn arithmetic_unary_operator(op: ArithmeticUnaryOp) -> &'static str {
    match op {
        ArithmeticUnaryOp::PreIncrement => "++",
        ArithmeticUnaryOp::PreDecrement => "--",
        ArithmeticUnaryOp::Plus => "+",
        ArithmeticUnaryOp::Minus => "-",
        ArithmeticUnaryOp::LogicalNot => "!",
        ArithmeticUnaryOp::BitwiseNot => "~",
    }
}

fn arithmetic_postfix_operator(op: ArithmeticPostfixOp) -> &'static str {
    match op {
        ArithmeticPostfixOp::Increment => "++",
        ArithmeticPostfixOp::Decrement => "--",
    }
}

fn arithmetic_binary_operator(op: ArithmeticBinaryOp) -> &'static str {
    match op {
        ArithmeticBinaryOp::Comma => ",",
        ArithmeticBinaryOp::Power => "**",
        ArithmeticBinaryOp::Multiply => "*",
        ArithmeticBinaryOp::Divide => "/",
        ArithmeticBinaryOp::Modulo => "%",
        ArithmeticBinaryOp::Add => "+",
        ArithmeticBinaryOp::Subtract => "-",
        ArithmeticBinaryOp::ShiftLeft => "<<",
        ArithmeticBinaryOp::ShiftRight => ">>",
        ArithmeticBinaryOp::LessThan => "<",
        ArithmeticBinaryOp::LessThanOrEqual => "<=",
        ArithmeticBinaryOp::GreaterThan => ">",
        ArithmeticBinaryOp::GreaterThanOrEqual => ">=",
        ArithmeticBinaryOp::Equal => "==",
        ArithmeticBinaryOp::NotEqual => "!=",
        ArithmeticBinaryOp::BitwiseAnd => "&",
        ArithmeticBinaryOp::BitwiseXor => "^",
        ArithmeticBinaryOp::BitwiseOr => "|",
        ArithmeticBinaryOp::LogicalAnd => "&&",
        ArithmeticBinaryOp::LogicalOr => "||",
    }
}

fn arithmetic_assign_operator(op: ArithmeticAssignOp) -> &'static str {
    match op {
        ArithmeticAssignOp::Assign => "=",
        ArithmeticAssignOp::AddAssign => "+=",
        ArithmeticAssignOp::SubAssign => "-=",
        ArithmeticAssignOp::MulAssign => "*=",
        ArithmeticAssignOp::DivAssign => "/=",
        ArithmeticAssignOp::ModAssign => "%=",
        ArithmeticAssignOp::ShiftLeftAssign => "<<=",
        ArithmeticAssignOp::ShiftRightAssign => ">>=",
        ArithmeticAssignOp::AndAssign => "&=",
        ArithmeticAssignOp::XorAssign => "^=",
        ArithmeticAssignOp::OrAssign => "|=",
    }
}

fn push_arithmetic_lvalue(
    rendered: &mut String,
    target: &ArithmeticLvalue,
    env: WordRenderEnv<'_, '_>,
) {
    match target {
        ArithmeticLvalue::Variable(name) => rendered.push_str(name),
        ArithmeticLvalue::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(rendered, index, ArithmeticContext::Subscript, true, env);
            rendered.push(']');
        }
    }
}

fn push_arithmetic_source_text(
    rendered: &mut String,
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    if let Some(ast) = ast {
        match &ast.kind {
            ArithmeticExpr::ShellWord(word) if !options.simplify() && !options.minify() => {
                rendered.push_str(&render_arithmetic_slice_shell_word(word, source, options));
            }
            _ => render_arithmetic_subscript_expr_to_buf(rendered, ast, source, options, true),
        }
    } else {
        rendered.push_str(text.slice(source));
    }
}

fn push_parameter_slice_offset(
    rendered: &mut String,
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let mut offset = String::new();
    push_arithmetic_source_text(&mut offset, text, ast, source, options);
    if offset.starts_with('-') {
        rendered.push(' ');
    }
    rendered.push_str(&offset);
}

fn render_arithmetic_slice_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let [part] = word.parts.as_slice() else {
        return render_word_syntax(word, source, options);
    };

    match &part.kind {
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => {
            let mut body = String::new();
            if let Some(ast) = expression_ast.as_deref() {
                render_arithmetic_expr_to_buf(&mut body, ast, source, options);
            } else {
                body.push_str(expression.slice(source).trim());
            }
            let (open, close) = match syntax {
                ArithmeticExpansionSyntax::DollarParenParen => ("$((", "))"),
                ArithmeticExpansionSyntax::LegacyBracket => ("$[", "]"),
            };
            format!("{open}{body}{close}")
        }
        _ => render_word_syntax(word, source, options),
    }
}

fn push_arithmetic_expansion(
    rendered: &mut String,
    expression: &shuck_ast::SourceText,
    expression_ast: Option<&ArithmeticExprNode>,
    syntax: ArithmeticExpansionSyntax,
    env: WordRenderEnv<'_, '_>,
) {
    let expression_source = expression.slice(env.source);
    if matches!(syntax, ArithmeticExpansionSyntax::LegacyBracket) {
        push_trimmed_arithmetic_expansion_source(rendered, expression_source, syntax);
    } else if let Some(formatted) = format_multiline_arithmetic_expansion_source(
        expression_source,
        syntax,
        expression_ast,
        env.source,
        env.options,
    ) {
        rendered.push_str(&formatted);
    } else if arithmetic_expression_prefers_raw_source(expression_source)
        || !expression.is_source_backed()
    {
        push_trimmed_arithmetic_expansion_source(rendered, expression_source, syntax);
    } else if let Some(expression_ast) = expression_ast {
        match syntax {
            ArithmeticExpansionSyntax::DollarParenParen => {
                rendered.push_str("$((");
                push_arithmetic_expansion_body(rendered, expression_ast, env);
                rendered.push_str("))");
            }
            ArithmeticExpansionSyntax::LegacyBracket => unreachable!("handled above"),
        }
    } else {
        push_trimmed_arithmetic_expansion_source(rendered, expression_source, syntax);
    }
}

fn push_trimmed_arithmetic_expansion_source(
    rendered: &mut String,
    expression_source: &str,
    syntax: ArithmeticExpansionSyntax,
) {
    match syntax {
        ArithmeticExpansionSyntax::DollarParenParen => {
            rendered.push_str("$((");
            rendered.push_str(expression_source.trim());
            rendered.push_str("))");
        }
        ArithmeticExpansionSyntax::LegacyBracket => {
            rendered.push_str("$[");
            rendered.push_str(expression_source.trim());
            rendered.push(']');
        }
    }
}

fn format_multiline_arithmetic_expansion_source(
    expression_source: &str,
    syntax: ArithmeticExpansionSyntax,
    expression_ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if !matches!(syntax, ArithmeticExpansionSyntax::DollarParenParen)
        || !expression_source.contains('\n')
        || expression_source.contains('`')
    {
        return None;
    }

    let operators = multiline_arithmetic_source_trailing_operators(expression_source)?;
    let mut rendered_body = String::new();
    if let Some(expression_ast) = expression_ast {
        render_arithmetic_expr_to_buf(&mut rendered_body, expression_ast, source, options);
    } else {
        rendered_body.push_str(expression_source.trim());
    }
    let lines = split_rendered_arithmetic_body_at_source_operators(&rendered_body, &operators)?;
    let mut continuation_indent = String::new();
    options.push_indent_units(&mut continuation_indent, 1);
    let lines = lines
        .into_iter()
        .map(|line| format!("{continuation_indent}{line}"))
        .collect::<Vec<_>>();
    Some(format!("$((\\\n{}))", lines.join(" \\\n")))
}

fn multiline_arithmetic_source_trailing_operators(expression_source: &str) -> Option<Vec<&str>> {
    let lines = expression_source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return None;
    }

    let mut operators = Vec::with_capacity(lines.len().saturating_sub(1));
    for line in &lines[..lines.len() - 1] {
        operators.push(arithmetic_source_line_trailing_operator(line)?);
    }
    Some(operators)
}

fn arithmetic_source_line_trailing_operator(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    [
        "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "**", "+", "-", "*", "/", "%", "<", ">",
        "&", "^", "|",
    ]
    .into_iter()
    .find(|operator| trimmed.ends_with(operator))
}

fn split_rendered_arithmetic_body_at_source_operators(
    rendered_body: &str,
    operators: &[&str],
) -> Option<Vec<String>> {
    let mut lines = Vec::with_capacity(operators.len() + 1);
    let mut rest = rendered_body.trim();
    for operator in operators {
        let needle = format!(" {operator} ");
        let index = rest.find(&needle)?;
        let end = index + 1 + operator.len();
        lines.push(rest[..end].trim().to_string());
        rest = rest[index + needle.len()..].trim_start();
    }
    if rest.is_empty() {
        return None;
    }
    lines.push(rest.trim().to_string());
    Some(lines)
}

fn arithmetic_expression_prefers_raw_source(expression_source: &str) -> bool {
    expression_source.contains('`')
}

fn push_var_ref(
    rendered: &mut String,
    reference: &VarRef,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    rendered.push_str(reference.name.as_ref());
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        if let Some(selector) = subscript.selector() {
            rendered.push(match selector {
                SubscriptSelector::At => '@',
                SubscriptSelector::Star => '*',
            });
        } else if let Some(ast) = subscript.arithmetic_ast.as_ref() {
            let compact =
                !arithmetic_subscript_prefers_spaced_expression(subscript.syntax_text(source));
            render_arithmetic_subscript_expr_to_buf(rendered, ast, source, options, compact);
        } else {
            rendered.push_str(&compact_dynamic_arithmetic_subscript(
                subscript.syntax_text(source),
            ));
        }
        rendered.push(']');
    }
}

fn arithmetic_subscript_prefers_spaced_expression(text: &str) -> bool {
    let text = text.trim_start_matches([' ', '\t', '\r']);
    text.starts_with("$((") || text.starts_with('(')
}

fn compact_dynamic_arithmetic_subscript(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut dollar_paren_depth = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek().is_some_and(|next| *next == '(') {
            rendered.push(ch);
            rendered.push(chars.next().expect("peeked '('"));
            dollar_paren_depth = dollar_paren_depth.saturating_add(1);
            continue;
        }
        if dollar_paren_depth > 0 {
            if ch == '(' {
                dollar_paren_depth = dollar_paren_depth.saturating_add(1);
            } else if ch == ')' {
                dollar_paren_depth = dollar_paren_depth.saturating_sub(1);
            }
            rendered.push(ch);
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\r')
            && next_is_additive_operator_before_operand(chars.clone())
            && rendered
                .chars()
                .last()
                .is_some_and(|previous| !matches!(previous, ' ' | '\t' | '\r'))
        {
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\r')
            && chars.clone().next().is_some_and(|next| next == '%')
            && rendered
                .chars()
                .last()
                .is_some_and(|previous| !matches!(previous, ' ' | '\t' | '\r'))
        {
            continue;
        }
        if matches!(ch, '+' | '-')
            && chars
                .clone()
                .find(|next| !matches!(next, ' ' | '\t' | '\r'))
                .is_some_and(is_arithmetic_subscript_operand_start)
        {
            rendered.push(ch);
            while chars
                .peek()
                .is_some_and(|next| matches!(next, ' ' | '\t' | '\r'))
            {
                chars.next();
            }
            continue;
        }
        if ch == '%' {
            rendered.push(ch);
            while chars
                .peek()
                .is_some_and(|next| matches!(next, ' ' | '\t' | '\r'))
            {
                chars.next();
            }
            continue;
        }
        rendered.push(ch);
    }
    rendered
}

fn next_is_additive_operator_before_operand(
    mut chars: std::iter::Peekable<std::str::Chars<'_>>,
) -> bool {
    let Some(operator) = chars.next() else {
        return false;
    };
    if !matches!(operator, '+' | '-') {
        return false;
    }
    chars
        .find(|next| !matches!(next, ' ' | '\t' | '\r'))
        .is_some_and(is_arithmetic_subscript_operand_start)
}

fn is_arithmetic_subscript_operand_start(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '(' | '{')
}

fn push_parameter_word(
    rendered: &mut String,
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Result<(), std::fmt::Error> {
    let Some(syntax) = parameter.bourne() else {
        let raw = parameter.raw_body.slice(source);
        rendered.push_str("${");
        rendered.push_str(&compact_raw_parameter_subscript(raw));
        rendered.push('}');
        return Ok(());
    };

    match syntax {
        BourneParameterExpansion::Access { reference } => {
            push_braced_var_ref(rendered, "", reference, source, options);
        }
        BourneParameterExpansion::Length { reference } => {
            push_braced_var_ref(rendered, "#", reference, source, options);
        }
        BourneParameterExpansion::Indices { reference } => {
            push_braced_var_ref(rendered, "!", reference, source, options);
        }
        BourneParameterExpansion::Indirect {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
            if let Some(operator) = operator {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(parameter_defaulting_operator(operator.as_ref()));
                if let Some(operand) = operand {
                    rendered.push_str(operand.slice(source));
                }
            }
            rendered.push('}');
        }
        BourneParameterExpansion::PrefixMatch { prefix, kind } => {
            rendered.push_str("${!");
            rendered.push_str(prefix);
            rendered.push(kind.as_char());
            rendered.push('}');
        }
        BourneParameterExpansion::Slice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push(':');
            push_parameter_slice_offset(rendered, offset, offset_ast.as_deref(), source, options);
            if let Some(length) = length {
                rendered.push(':');
                push_arithmetic_source_text(
                    rendered,
                    length,
                    length_ast.as_deref(),
                    source,
                    options,
                );
            }
            rendered.push('}');
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            render_parameter_expansion(
                rendered,
                reference,
                operator.as_ref(),
                operand.as_ref(),
                *colon_variant,
                Some(parameter.span),
                WordRenderEnv::new(source, options, None, None),
            )?;
        }
        BourneParameterExpansion::Transformation {
            reference,
            operator,
        } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push('@');
            std::write!(rendered, "{operator}")?;
            rendered.push('}');
        }
    }

    Ok(())
}

fn push_parameter_operand(
    rendered: &mut String,
    operand: &shuck_ast::SourceText,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let operand = compact_parameter_operand_subscripts(operand.slice(source));
    if operand.contains("$(") || operand.contains('`') {
        let mut normalized = String::new();
        push_raw_shell_text_with_normalized_redirect_spacing(&mut normalized, &operand);
        if let Some(command_normalized) =
            normalize_inline_command_substitutions_in_parameter_operand(&normalized, options)
        {
            rendered.push_str(&command_normalized);
        } else {
            rendered.push_str(&normalized);
        }
    } else {
        rendered.push_str(&operand);
    }
}

fn normalize_inline_command_substitutions_in_parameter_operand(
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    let mut rendered = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    let mut changed = false;

    while let Some((open_offset, close_offset)) = next_raw_command_substitution(raw, index) {
        let body = &raw[open_offset + 2..close_offset];
        if !body.contains('\n')
            && let Some(normalized_body) =
                normalize_inline_parameter_command_substitution_body(body, options)
            && normalized_body != body
        {
            rendered.push_str(&raw[cursor..open_offset]);
            rendered.push_str("$(");
            rendered.push_str(&normalized_body);
            rendered.push(')');
            cursor = close_offset + 1;
            changed = true;
        }
        index = close_offset + 1;
    }

    finish_raw_rewrite(rendered, raw, cursor, changed)
}

fn next_raw_command_substitution(raw: &str, mut index: usize) -> Option<(usize, usize)> {
    let bytes = raw.as_bytes();

    while index + 1 < bytes.len() {
        if bytes[index] == b'$'
            && bytes[index + 1] == b'('
            && index
                .checked_sub(1)
                .and_then(|previous| bytes.get(previous))
                .is_none_or(|byte| *byte != b'\\')
            && bytes.get(index + 2).is_none_or(|byte| *byte != b'(')
            && let Some(close_offset) = matching_raw_command_substitution_close(raw, index + 2)
        {
            return Some((index, close_offset));
        }
        index += 1;
    }

    None
}

fn finish_raw_rewrite(
    mut rendered: String,
    raw: &str,
    cursor: usize,
    changed: bool,
) -> Option<String> {
    changed.then(|| {
        rendered.push_str(&raw[cursor..]);
        rendered
    })
}

fn normalize_inline_parameter_command_substitution_body(
    body: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    let trimmed = body.trim_matches([' ', '\t', '\r']);
    if trimmed.is_empty() {
        return None;
    }

    let parsed = shuck_parser::parser::Parser::with_dialect(trimmed, options.dialect()).parse();
    if parsed.is_err() {
        return None;
    }

    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(
        trimmed,
        &parsed.file.body,
        options,
        None,
        None,
        &mut nested,
    )?;
    let formatted = trim_trailing_line_endings(&nested);
    (!formatted.is_empty() && !formatted.contains('\n')).then(|| formatted.to_string())
}

fn compact_raw_parameter_subscript(raw: &str) -> String {
    let Some(open) = raw.find('[') else {
        return raw.to_string();
    };
    let Some(close) = raw.rfind(']') else {
        return raw.to_string();
    };
    if close <= open {
        return raw.to_string();
    }
    let mut rendered = String::with_capacity(raw.len());
    rendered.push_str(&raw[..=open]);
    rendered.push_str(&compact_dynamic_arithmetic_subscript(&raw[open + 1..close]));
    rendered.push_str(&raw[close..]);
    rendered
}

fn compact_parameter_operand_subscripts(text: &str) -> String {
    let Some(body) = text
        .strip_prefix("${")
        .and_then(|body| body.strip_suffix('}'))
    else {
        return text.to_string();
    };
    let compacted = compact_raw_parameter_subscript(body);
    if compacted == body {
        return text.to_string();
    }
    let mut rendered = String::with_capacity(text.len());
    rendered.push_str("${");
    rendered.push_str(&compacted);
    rendered.push('}');
    rendered
}

fn render_parameter_expansion(
    rendered: &mut String,
    reference: &VarRef,
    operator: &ParameterOp,
    operand: Option<&shuck_ast::SourceText>,
    colon_variant: bool,
    raw_parameter_span: Option<shuck_ast::Span>,
    env: WordRenderEnv<'_, '_>,
) -> Result<(), std::fmt::Error> {
    let (source, options) = (env.source, env.options);

    rendered.push_str("${");
    push_var_ref(rendered, reference, source, options);
    match operator {
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error => {
            if colon_variant {
                rendered.push(':');
            }
            rendered.push_str(parameter_defaulting_operator(operator));
            if let Some(operand) = operand {
                push_parameter_operand(rendered, operand, source, options);
            }
        }
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => {
            let removal_operator = match operator {
                ParameterOp::RemovePrefixShort { .. } => "#",
                ParameterOp::RemovePrefixLong { .. } => "##",
                ParameterOp::RemoveSuffixShort { .. } => "%",
                ParameterOp::RemoveSuffixLong { .. } => "%%",
                _ => unreachable!(),
            };
            rendered.push_str(removal_operator);
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
            ..
        }
        | ParameterOp::ReplaceAll {
            pattern,
            replacement,
            ..
        } => {
            let replace_all = matches!(operator, ParameterOp::ReplaceAll { .. });
            rendered.push('/');
            if replace_all {
                rendered.push('/');
            }
            if let Some((raw_pattern, raw_replacement)) = raw_parameter_replacement_parts(
                raw_parameter_span,
                reference,
                replace_all,
                source,
                options,
            ) {
                rendered.push_str(raw_pattern);
                rendered.push('/');
                rendered.push_str(raw_replacement);
            } else {
                render_parameter_replacement_pattern(rendered, pattern, source, options);
                rendered.push('/');
                push_parameter_replacement_text(rendered, replacement, source);
            }
        }
        ParameterOp::UpperFirst => rendered.push('^'),
        ParameterOp::UpperAll => rendered.push_str("^^"),
        ParameterOp::LowerFirst => rendered.push(','),
        ParameterOp::LowerAll => rendered.push_str(",,"),
    }
    rendered.push('}');
    Ok(())
}

fn raw_parameter_replacement_parts<'a>(
    raw_parameter_span: Option<shuck_ast::Span>,
    reference: &VarRef,
    replace_all: bool,
    source: &'a str,
    options: &ResolvedShellFormatOptions,
) -> Option<(&'a str, &'a str)> {
    if options.simplify() || options.minify() {
        return None;
    }

    let span = raw_parameter_span?;
    let raw_parameter = source.get(span.start.offset..span.end.offset)?;
    let raw = raw_parameter.strip_prefix("${")?.strip_suffix('}')?;
    let raw_body_start = span.start.offset.checked_add("${".len())?;
    let reference_end = reference.name_span.end.offset.checked_sub(raw_body_start)?;
    let operator = if replace_all { "//" } else { "/" };
    let after_operator = raw.get(reference_end..)?.strip_prefix(operator)?;
    Some(split_raw_parameter_replacement(after_operator))
}

fn split_raw_parameter_replacement(raw: &str) -> (&str, &str) {
    let mut escaped = false;
    let mut parameter_depth = 0usize;
    let mut chars = raw.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '$' if chars.peek().is_some_and(|(_, next)| *next == '{') => {
                chars.next();
                parameter_depth += 1;
            }
            '}' if parameter_depth > 0 => parameter_depth -= 1,
            '/' if parameter_depth == 0 => {
                return (&raw[..index], &raw[index + '/'.len_utf8()..]);
            }
            _ => {}
        }
    }

    (raw, "")
}

fn render_parameter_replacement_pattern(
    rendered: &mut String,
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    if !options.simplify()
        && !options.minify()
        && let Some(raw) = raw_pattern_source_slice(pattern, source)
    {
        rendered.push_str(raw);
        return;
    }

    render_pattern_syntax_to_buf(pattern, source, options, rendered);
}

fn push_parameter_replacement_text(
    rendered: &mut String,
    replacement: &shuck_ast::SourceText,
    source: &str,
) {
    if let Some(raw) = raw_source_slice(replacement.span(), source) {
        rendered.push_str(raw);
    } else {
        rendered.push_str(replacement.slice(source));
    }
}

fn parameter_defaulting_operator(operator: &ParameterOp) -> &'static str {
    match operator {
        ParameterOp::UseDefault => "-",
        ParameterOp::AssignDefault => "=",
        ParameterOp::UseReplacement => "+",
        ParameterOp::Error => "?",
        _ => "",
    }
}

pub(crate) fn render_pattern_syntax_to_buf(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
    rendered: &mut String,
) {
    if pattern_needs_formatter_rendering(pattern) {
        render_pattern_parts_syntax_to_buf(pattern, source, options, rendered);
        return;
    }

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_pattern_source_slice(pattern, source)
        && could_need_preserve_raw_syntax(slice)
    {
        let start = rendered.len();
        pattern.render_syntax_to_buf(source, rendered);
        if should_preserve_raw_syntax(slice, &rendered[start..]) {
            rendered.truncate(start);
            rendered.push_str(slice);
        }
        return;
    }

    pattern.render_syntax_to_buf(source, rendered);
}

fn pattern_needs_formatter_rendering(pattern: &Pattern) -> bool {
    pattern.parts.iter().any(|part| match &part.kind {
        PatternPart::Word(word) => word_needs_special_rendering(word),
        PatternPart::Group { patterns, .. } => {
            patterns.iter().any(pattern_needs_formatter_rendering)
        }
        _ => false,
    })
}

fn render_pattern_parts_syntax_to_buf(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
    rendered: &mut String,
) {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Word(word) => {
                render_word_syntax_to_buf(word, source, options, rendered);
            }
            PatternPart::Group { kind, patterns } => {
                let _ = std::write!(rendered, "{}(", kind.prefix());
                for (index, pattern) in patterns.iter().enumerate() {
                    if index > 0 {
                        rendered.push('|');
                    }
                    render_pattern_syntax_to_buf(pattern, source, options, rendered);
                }
                rendered.push(')');
            }
            _ => {
                let single = Pattern {
                    parts: vec![part.clone()],
                    span: part.span,
                };
                single.render_syntax_to_buf(source, rendered);
            }
        }
    }
}

fn raw_word_source_slice<'a>(word: &Word, source: &'a str) -> Option<&'a str> {
    raw_source_slice(word.span, source)
}

fn word_is_single_quoted_only(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [shuck_ast::WordPartNode {
            kind: WordPart::SingleQuoted { .. },
            ..
        }]
    )
}

fn raw_pattern_source_slice<'a>(pattern: &Pattern, source: &'a str) -> Option<&'a str> {
    raw_source_slice(pattern.span, source)
}

fn raw_source_slice(span: shuck_ast::Span, source: &str) -> Option<&str> {
    if span.start.offset >= span.end.offset || span.end.offset > source.len() {
        return None;
    }

    let slice = span.slice(source);
    if slice.contains('\n') {
        Some(slice)
    } else {
        Some(trim_unescaped_trailing_whitespace(slice))
    }
}

fn should_preserve_raw_syntax(raw: &str, rendered: &str) -> bool {
    raw != rendered && could_need_preserve_raw_syntax(raw)
}

fn should_preserve_special_rendered_raw_syntax(raw: &str, rendered: &str) -> bool {
    raw != rendered
        && !raw.contains('\n')
        && !raw_command_substitution_needs_structural_spacing(raw)
        && could_need_preserve_raw_syntax_beyond_line_continuations(raw)
}

fn could_need_preserve_raw_syntax(raw: &str) -> bool {
    raw.starts_with('\\')
        || raw.starts_with('&')
        || raw.starts_with("$'")
        || raw_contains_escaped_horizontal_whitespace(raw)
        || raw.contains("\\\n")
        || raw.contains("\\\"")
        || raw.contains("\\`")
        || raw_contains_double_backslash_outside_single_quotes(raw)
        || raw.contains("[^ ]")
}

fn could_need_preserve_raw_syntax_beyond_line_continuations(raw: &str) -> bool {
    raw.starts_with('\\')
        || raw.starts_with('&')
        || raw.starts_with("$'")
        || raw_contains_escaped_horizontal_whitespace(raw)
        || raw.contains("\\\"")
        || raw.contains("\\`")
        || raw.contains("[^ ]")
}

fn raw_contains_escaped_horizontal_whitespace(raw: &str) -> bool {
    raw.contains("\\ ") || raw.contains("\\\t")
}

fn raw_contains_double_backslash_outside_single_quotes(raw: &str) -> bool {
    let mut in_single_quotes = false;
    let mut previous_was_backslash = false;
    let mut chars = raw.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' && !previous_was_backslash {
            in_single_quotes = !in_single_quotes;
        }

        if !in_single_quotes && ch == '\\' && chars.peek().is_some_and(|(_, next)| *next == '\\') {
            return true;
        }

        previous_was_backslash = ch == '\\'
            && raw
                .get(index + ch.len_utf8()..)
                .is_some_and(|rest| !rest.starts_with('\\'));
    }

    false
}
