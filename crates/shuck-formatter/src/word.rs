use std::fmt::Write as _;

use shuck_ast::{
    ArithmeticAssignOp, ArithmeticBinaryOp, ArithmeticExpansionSyntax, ArithmeticExpr,
    ArithmeticExprNode, ArithmeticLvalue, ArithmeticPostfixOp, ArithmeticUnaryOp,
    BourneParameterExpansion, Command, CommandSubstitutionSyntax, CompoundCommand, HeredocBody,
    HeredocBodyPart, ParameterOp, Pattern, Stmt, StmtSeq, Subscript, SubscriptSelector, VarRef,
    Word, WordPart,
};
use shuck_format::IndentStyle;
use shuck_format::{FormatResult, text, write};
use shuck_parser::parser::Parser;

use crate::FormatNodeRule;
use crate::command::stmt_seq_has_heredoc;
use crate::comments::SourceMap;
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::prelude::ShellFormatter;
use crate::streaming::format_stmt_sequence_streaming_to_buf;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatWord;

impl FormatNodeRule<Word> for FormatWord {
    fn fmt(&self, word: &Word, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let rendered = render_word_syntax(
            word,
            formatter.context().source(),
            formatter.context().options(),
        );
        write!(formatter, [text(rendered)])
    }
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
    render_word_syntax_internal(word, source, options, None, None, rendered);
}

pub(crate) fn render_word_syntax_with_facts_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
    rendered: &mut String,
) {
    render_word_syntax_internal(
        word,
        source,
        options,
        Some(source_map),
        Some(facts),
        rendered,
    );
}

pub(crate) fn render_heredoc_body_to_buf(
    body: &HeredocBody,
    source: &str,
    options: &ResolvedShellFormatOptions,
    _facts: &FormatterFacts<'_>,
    rendered: &mut String,
) {
    for part in &body.parts {
        if render_heredoc_body_part(rendered, &part.kind, part.span, source, options, _facts)
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
    rendered: &mut String,
) {
    if word_has_escaped_backtick_substitution(word, source)
        && let Some(raw) = raw_word_source_slice(word, source)
    {
        rendered.push_str(raw);
        return;
    }

    if let Some(raw) = raw_word_source_slice(word, source)
        && let Some(without_continuation) = raw_simple_expansion_without_line_continuation(raw)
    {
        rendered.push_str(without_continuation);
        return;
    }

    if let Some(raw) = raw_word_source_slice(word, source)
        && let Some(formatted) = format_parameter_default_arithmetic_in_raw_word(raw, options)
    {
        rendered.push_str(&formatted);
        return;
    }

    if word_needs_special_rendering(word, source) {
        if render_word_parts(
            word.parts.as_slice(),
            source,
            options,
            source_map,
            facts,
            rendered,
        )
        .is_err()
        {
            unreachable!("writing into a String should not fail");
        }
        return;
    }

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_word_source_slice(word, source)
        && could_need_preserve_raw_syntax(slice)
        && !simple_expansion_word_has_line_continuation_suffix(word, slice)
    {
        let start = rendered.len();
        word.render_syntax_to_buf(source, rendered);
        if should_preserve_raw_syntax(slice, &rendered[start..]) {
            rendered.truncate(start);
            rendered.push_str(slice);
        }
        return;
    }

    word.render_syntax_to_buf(source, rendered);
}

fn raw_simple_expansion_without_line_continuation(raw: &str) -> Option<&str> {
    if !raw.contains('\n') {
        return None;
    }
    let trimmed = raw.trim_end_matches([' ', '\t', '\r', '\n']);
    let without_backslash = trimmed.strip_suffix('\\')?.trim_end();
    if without_backslash.starts_with("${") && without_backslash.ends_with('}') {
        Some(without_backslash)
    } else {
        None
    }
}

fn simple_expansion_word_has_line_continuation_suffix(word: &Word, raw: &str) -> bool {
    raw.contains('\n')
        && raw
            .trim_end_matches([' ', '\t', '\r', '\n'])
            .ends_with('\\')
        && word.parts.iter().all(|part| {
            matches!(
                part.kind,
                WordPart::Parameter(_)
                    | WordPart::ParameterExpansion { .. }
                    | WordPart::ArrayAccess(_)
                    | WordPart::ArrayLength(_)
                    | WordPart::ArrayIndices(_)
                    | WordPart::Length(_)
                    | WordPart::Variable(_)
            )
        })
}

fn format_parameter_default_arithmetic_in_raw_word(
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if !(raw.contains(":-$((")
        || raw.contains(":=$((")
        || raw.contains(":+$((")
        || raw.contains(":?$((")
        || raw.contains("-$((")
        || raw.contains("=$((")
        || raw.contains("+$((")
        || raw.contains("?$(("))
    {
        return None;
    }

    let mut formatted = String::new();
    let mut index = 0;
    let mut changed = false;
    while let Some(relative) = raw[index..].find("$((") {
        let start = index + relative;
        let operator_prefix = &raw[..start];
        if !parameter_default_operator_precedes_operand(operator_prefix) {
            formatted.push_str(&raw[index..start + "$(".len()]);
            index = start + "$(".len();
            continue;
        }
        let Some(end) = raw[start + "$((".len()..].find("))") else {
            break;
        };
        let end = start + "$((".len() + end + "))".len();
        formatted.push_str(&raw[index..start]);
        if let Some(arithmetic) = format_arithmetic_expansion_operand(&raw[start..end], options) {
            changed |= arithmetic != raw[start..end];
            formatted.push_str(&arithmetic);
        } else {
            formatted.push_str(&raw[start..end]);
        }
        index = end;
    }
    formatted.push_str(&raw[index..]);
    changed.then_some(formatted)
}

fn parameter_default_operator_precedes_operand(prefix: &str) -> bool {
    if [":-", ":=", ":+", ":?"]
        .iter()
        .any(|operator| prefix.ends_with(operator))
    {
        return true;
    }

    let Some((_, parameter_body_prefix)) = prefix.rsplit_once("${") else {
        return false;
    };
    !parameter_body_prefix.contains(':')
        && ["-", "=", "+", "?"]
            .iter()
            .any(|operator| parameter_body_prefix.ends_with(operator))
}

/// Returns `true` when a word contains a backtick command-substitution node
/// whose raw source starts with `\`, indicating the parser misinterpreted an
/// escaped literal backtick (`\``) as a command-substitution delimiter.
/// In that case the word's raw source must be preserved verbatim.
fn word_has_escaped_backtick_substitution(word: &Word, source: &str) -> bool {
    word.parts.iter().any(|part| {
        matches!(
            part.kind,
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::Backtick,
                ..
            }
        ) && raw_source_slice(part.span, source).is_some_and(|raw| raw.starts_with('\\'))
    })
}

fn word_needs_special_rendering(word: &Word, source: &str) -> bool {
    word.parts
        .iter()
        .any(|part| part_needs_special_rendering(&part.kind, source))
}

fn part_needs_special_rendering(part: &WordPart, source: &str) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_needs_special_rendering(&part.kind, source)),
        WordPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_some(),
        WordPart::Parameter(parameter) => parameter_needs_special_rendering(parameter, source),
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            ..
        } => {
            var_ref_needs_special_rendering(reference, source)
                || operand.as_ref().is_some_and(|operand| {
                    parameter_default_operand_needs_special_rendering(
                        operator.as_ref(),
                        operand.slice(source),
                    )
                })
        }
        WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Length(_) => true,
        WordPart::Substring { .. } | WordPart::ArraySlice { .. } => true,
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => true,
        _ => false,
    }
}

fn render_word_parts(
    parts: &[shuck_ast::WordPartNode],
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    rendered: &mut String,
) -> Result<(), std::fmt::Error> {
    for part in parts {
        render_word_part(
            rendered, &part.kind, part.span, source, options, source_map, facts,
        )?;
    }
    Ok(())
}

fn double_quoted_multiline_can_preserve_raw(parts: &[shuck_ast::WordPartNode]) -> bool {
    !parts.iter().any(|part| {
        matches!(
            part.kind,
            WordPart::CommandSubstitution { .. } | WordPart::ArithmeticExpansion { .. }
        )
    })
}

fn render_heredoc_body_part(
    rendered: &mut String,
    part: &HeredocBodyPart,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
    _facts: &FormatterFacts<'_>,
) -> Result<(), std::fmt::Error> {
    match part {
        HeredocBodyPart::Literal(text) => rendered.push_str(text.as_str(source, span)),
        HeredocBodyPart::Variable(name) => {
            std::write!(rendered, "${name}")?;
        }
        HeredocBodyPart::CommandSubstitution { body, syntax } => {
            let raw = raw_source_slice(span, source);
            let multiline = raw.is_some_and(command_substitution_opens_on_own_line)
                || body.as_slice().len() > 1
                || stmt_seq_has_heredoc(body)
                || raw.is_none() && *syntax == CommandSubstitutionSyntax::DollarParen;

            if render_command_substitution(
                rendered,
                body,
                span.end.offset,
                source,
                options,
                multiline,
                None,
                None,
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
        HeredocBodyPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => {
            if matches!(syntax, ArithmeticExpansionSyntax::LegacyBracket) {
                push_trimmed_arithmetic_expansion_source(
                    rendered,
                    expression.slice(source),
                    *syntax,
                );
            } else if let Some(expression_ast) = expression_ast {
                if !expression.is_source_backed() {
                    push_trimmed_arithmetic_expansion_source(
                        rendered,
                        expression.slice(source),
                        *syntax,
                    );
                } else {
                    match syntax {
                        ArithmeticExpansionSyntax::DollarParenParen => {
                            rendered.push_str("$((");
                            push_arithmetic_expr(
                                rendered,
                                expression_ast,
                                ArithmeticContext::TopLevel,
                                source,
                                options,
                            );
                            rendered.push_str("))");
                        }
                        ArithmeticExpansionSyntax::LegacyBracket => {
                            rendered.push_str("$[");
                            push_arithmetic_expr(
                                rendered,
                                expression_ast,
                                ArithmeticContext::TopLevel,
                                source,
                                options,
                            );
                            rendered.push(']');
                        }
                    }
                }
            } else {
                match syntax {
                    ArithmeticExpansionSyntax::DollarParenParen => {
                        std::write!(rendered, "$(({}))", expression.slice(source))?;
                    }
                    ArithmeticExpansionSyntax::LegacyBracket => {
                        std::write!(rendered, "$[{}]", expression.slice(source))?;
                    }
                }
            }
        }
        HeredocBodyPart::Parameter(parameter) => {
            push_parameter_word(rendered, parameter, source, options)?;
        }
    }

    Ok(())
}

fn render_word_part(
    rendered: &mut String,
    part: &WordPart,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> Result<(), std::fmt::Error> {
    if let Some(raw) = preferred_raw_word_part_source(part, span, source, options) {
        rendered.push_str(raw);
        return Ok(());
    }

    match part {
        WordPart::Literal(text) => rendered.push_str(text.as_str(source, span)),
        WordPart::SingleQuoted { value, dollar } => {
            if *dollar {
                rendered.push('$');
            }
            rendered.push('\'');
            rendered.push_str(value.slice(source));
            rendered.push('\'');
        }
        WordPart::DoubleQuoted { parts, dollar } => {
            if let Some(raw) = raw_source_slice(span, source)
                && (raw.contains("\\\"")
                    || raw.contains('\n') && double_quoted_multiline_can_preserve_raw(parts))
            {
                rendered.push_str(raw);
                return Ok(());
            }
            if *dollar {
                rendered.push('$');
            }
            rendered.push('"');
            for part in parts {
                match &part.kind {
                    WordPart::Literal(text) if text.is_source_backed() => {
                        rendered.push_str(text.syntax_str(source, part.span));
                    }
                    WordPart::Literal(text) => {
                        render_double_quoted_literal(rendered, text.as_str(source, part.span));
                    }
                    other => render_word_part(
                        rendered, other, part.span, source, options, source_map, facts,
                    )?,
                }
            }
            rendered.push('"');
        }
        WordPart::Variable(name) => {
            std::write!(rendered, "${name}")?;
        }
        WordPart::CommandSubstitution { body, syntax } => {
            if let Some(raw) = raw_source_slice(span, source) {
                if stmt_seq_contains_comments(body) {
                    rendered.push_str(raw);
                } else if render_command_substitution(
                    rendered,
                    body,
                    span.end.offset,
                    source,
                    options,
                    command_substitution_opens_on_own_line(raw)
                        || command_substitution_has_line_continuation(raw)
                            && !command_substitution_starts_with_inline_brace_group(raw)
                        || body.as_slice().len() > 1
                        || stmt_seq_has_heredoc(body),
                    source_map,
                    facts,
                )
                .is_some()
                {
                } else {
                    rendered.push_str(raw);
                }
            } else if render_command_substitution(
                rendered,
                body,
                span.end.offset,
                source,
                options,
                *syntax == CommandSubstitutionSyntax::DollarParen,
                source_map,
                facts,
            )
            .is_some()
            {
            } else {
                std::write!(rendered, "$({body:?})")?;
            }
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => {
            if matches!(syntax, ArithmeticExpansionSyntax::LegacyBracket) {
                push_trimmed_arithmetic_expansion_source(
                    rendered,
                    expression.slice(source),
                    *syntax,
                );
            } else if let Some(expression_ast) = expression_ast {
                if !expression.is_source_backed() {
                    push_trimmed_arithmetic_expansion_source(
                        rendered,
                        expression.slice(source),
                        *syntax,
                    );
                } else {
                    match syntax {
                        ArithmeticExpansionSyntax::DollarParenParen => {
                            rendered.push_str("$((");
                            push_arithmetic_expr(
                                rendered,
                                expression_ast,
                                ArithmeticContext::TopLevel,
                                source,
                                options,
                            );
                            rendered.push_str("))");
                        }
                        ArithmeticExpansionSyntax::LegacyBracket => {
                            rendered.push_str("$[");
                            push_arithmetic_expr(
                                rendered,
                                expression_ast,
                                ArithmeticContext::TopLevel,
                                source,
                                options,
                            );
                            rendered.push(']');
                        }
                    }
                }
            } else {
                match syntax {
                    ArithmeticExpansionSyntax::DollarParenParen => {
                        std::write!(rendered, "$(({}))", expression.slice(source))?;
                    }
                    ArithmeticExpansionSyntax::LegacyBracket => {
                        std::write!(rendered, "$[{}]", expression.slice(source))?;
                    }
                }
            }
        }
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
            source,
            options,
        )?,
        WordPart::Length(reference) => {
            rendered.push_str("${#");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        WordPart::ArrayAccess(reference) => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        WordPart::ArrayLength(reference) => {
            rendered.push_str("${#");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        WordPart::ArrayIndices(reference) => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
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
            push_parameter_slice_arithmetic(
                rendered,
                offset,
                offset_ast.as_deref(),
                source,
                options,
            );
            if let Some(length) = length {
                rendered.push(':');
                push_parameter_slice_arithmetic(
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
                    rendered.push_str(operand.slice(source));
                }
            }
            rendered.push('}');
        }
        WordPart::PrefixMatch { prefix, kind } => {
            std::write!(rendered, "${{!{}{}}}", prefix, kind.as_char())?;
        }
        WordPart::ProcessSubstitution { body, is_input } => {
            if let Some(facts) = facts {
                let mut body_rendered = String::new();
                if format_stmt_sequence_streaming_to_buf(
                    source,
                    body,
                    options,
                    facts,
                    None,
                    &mut body_rendered,
                )
                .is_ok()
                {
                    rendered.push(if *is_input { '<' } else { '>' });
                    rendered.push('(');
                    rendered.push_str(body_rendered.trim_end());
                    rendered.push(')');
                    return Ok(());
                }
            }
            rendered.push_str(span.slice(source));
        }
        WordPart::Transformation { .. } | WordPart::ZshQualifiedGlob(_) => {
            rendered.push_str(span.slice(source));
        }
    }

    Ok(())
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
        WordPart::Parameter(parameter) => parameter_prefers_raw_source(parameter, span, source)
            .then(|| raw_source_slice(span, source))
            .flatten(),
        WordPart::ParameterExpansion { .. } => raw_source_slice(span, source),
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

fn parameter_needs_special_rendering(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
) -> bool {
    parameter.bourne().is_some_and(|syntax| match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Length { reference }
        | BourneParameterExpansion::Indices { reference } => {
            var_ref_needs_special_rendering(reference, source)
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            ..
        } => {
            var_ref_needs_special_rendering(reference, source)
                || matches!(
                    operator.as_ref(),
                    ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
                )
                || operand.as_ref().is_some_and(|operand| {
                    parameter_default_operand_needs_special_rendering(
                        operator.as_ref(),
                        operand.slice(source),
                    )
                })
        }
        BourneParameterExpansion::Slice { .. } => true,
        _ => false,
    })
}

fn parameter_default_operand_needs_special_rendering(operator: &ParameterOp, raw: &str) -> bool {
    matches!(
        operator,
        ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
    ) && raw.trim().starts_with("$((")
        && raw.trim().ends_with("))")
}

fn var_ref_needs_special_rendering(reference: &VarRef, source: &str) -> bool {
    reference.subscript.as_deref().is_some_and(|subscript| {
        subscript.arithmetic_ast.is_some()
            && subscript_prefers_structured_arithmetic(subscript, source)
            || subscript
                .word_ast()
                .is_some_and(word_contains_structured_arithmetic_expansion)
    })
}

fn subscript_prefers_structured_arithmetic(subscript: &Subscript, source: &str) -> bool {
    subscript
        .syntax_text(source)
        .trim_start()
        .starts_with("$((")
}

fn parameter_prefers_raw_source(
    parameter: &shuck_ast::ParameterExpansion,
    span: shuck_ast::Span,
    source: &str,
) -> bool {
    parameter.bourne().is_none_or(|syntax| match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Length { reference }
        | BourneParameterExpansion::Indices { reference }
            if var_ref_needs_special_rendering(reference, source) =>
        {
            false
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        } if var_ref_needs_special_rendering(reference, source) => false,
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
    match command {
        CompoundCommand::If(command) => {
            stmt_seq_contains_comments(&command.condition)
                || stmt_seq_contains_comments(&command.then_branch)
                || command.elif_branches.iter().any(|(condition, body)| {
                    stmt_seq_contains_comments(condition) || stmt_seq_contains_comments(body)
                })
                || command
                    .else_branch
                    .as_ref()
                    .is_some_and(stmt_seq_contains_comments)
        }
        CompoundCommand::For(command) => stmt_seq_contains_comments(&command.body),
        CompoundCommand::Repeat(command) => stmt_seq_contains_comments(&command.body),
        CompoundCommand::Foreach(command) => stmt_seq_contains_comments(&command.body),
        CompoundCommand::ArithmeticFor(command) => stmt_seq_contains_comments(&command.body),
        CompoundCommand::While(command) => {
            stmt_seq_contains_comments(&command.condition)
                || stmt_seq_contains_comments(&command.body)
        }
        CompoundCommand::Until(command) => {
            stmt_seq_contains_comments(&command.condition)
                || stmt_seq_contains_comments(&command.body)
        }
        CompoundCommand::Case(command) => command
            .cases
            .iter()
            .any(|case| stmt_seq_contains_comments(&case.body)),
        CompoundCommand::Select(command) => stmt_seq_contains_comments(&command.body),
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            stmt_seq_contains_comments(body)
        }
        CompoundCommand::Always(command) => {
            stmt_seq_contains_comments(&command.body)
                || stmt_seq_contains_comments(&command.always_body)
        }
        CompoundCommand::Time(command) => command
            .command
            .as_ref()
            .is_some_and(|stmt| stmt_contains_comments(stmt)),
        CompoundCommand::Coproc(command) => stmt_contains_comments(&command.body),
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_command_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    upper_bound: usize,
    source: &str,
    options: &ResolvedShellFormatOptions,
    multiline: bool,
    _source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> Option<()> {
    let mut nested = String::new();
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
    format_stmt_sequence_streaming_to_buf(
        source,
        body,
        options,
        facts,
        Some(upper_bound),
        &mut nested,
    )
    .ok()?;

    let trimmed = trim_trailing_line_endings(&nested);
    if trimmed.is_empty() {
        rendered.push_str("$()");
        return Some(());
    }

    if multiline {
        rendered.push_str("$(\n");
        push_indented_rendered_block(rendered, trimmed, options, 1);
        rendered.push_str("\n)");
    } else {
        let compacted_brace_group =
            compact_inline_brace_group_command_substitution(trimmed, options);
        let trimmed = compacted_brace_group.as_deref().unwrap_or(trimmed);
        rendered.push_str("$(");
        if trimmed.starts_with('(') {
            rendered.push(' ');
        }
        rendered.push_str(trimmed);
        rendered.push(')');
    }

    Some(())
}

fn compact_inline_brace_group_command_substitution(
    rendered: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    let body = rendered.strip_prefix("{\n")?;
    let close_offset = body.rfind("\n}")?;
    let group_body = &body[..close_offset];
    let suffix = &body[close_offset + 2..];
    let lines = group_body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let (first, rest) = lines.split_first()?;

    let mut compacted = String::new();
    compacted.push_str("{ ");
    compacted.push_str(first);
    let indent = command_substitution_indent_prefix(options, 1);
    for line in rest {
        compacted.push('\n');
        compacted.push_str(&indent);
        compacted.push_str(line);
    }
    if !compacted.trim_end().ends_with(';') {
        compacted.push(';');
    }
    compacted.push_str(" }");
    compacted.push_str(suffix);
    Some(compacted)
}

fn trim_trailing_line_endings(rendered: &str) -> &str {
    rendered.trim_end_matches(&['\r', '\n'][..])
}

fn command_substitution_opens_on_own_line(raw: &str) -> bool {
    raw.strip_prefix("$(")
        .is_some_and(|body| body.starts_with(['\n', '\r']))
}

fn command_substitution_has_line_continuation(raw: &str) -> bool {
    raw.contains("\\\n") || raw.contains("\\\r\n")
}

fn command_substitution_starts_with_inline_brace_group(raw: &str) -> bool {
    raw.starts_with("$({")
}

fn push_indented_rendered_block(
    target: &mut String,
    rendered: &str,
    options: &ResolvedShellFormatOptions,
    levels: usize,
) {
    let prefix = command_substitution_indent_prefix(options, levels);
    let mut pending_heredocs = Vec::new();
    let mut active_heredoc: Option<RenderedHeredocDelimiter> = None;

    for (index, line) in rendered.lines().enumerate() {
        if index > 0 {
            target.push('\n');
        }

        if let Some(delimiter) = active_heredoc.as_ref() {
            target.push_str(line);
            if delimiter.matches_line(line) {
                active_heredoc = pop_next_heredoc_delimiter(&mut pending_heredocs);
            }
            continue;
        }

        if line_needs_command_substitution_indent(line, options) {
            target.push_str(&prefix);
        }
        target.push_str(line);

        pending_heredocs.extend(heredoc_delimiters_in_rendered_line(line));
        active_heredoc = pop_next_heredoc_delimiter(&mut pending_heredocs);
    }
}

fn command_substitution_indent_prefix(
    options: &ResolvedShellFormatOptions,
    levels: usize,
) -> String {
    match options.indent_style() {
        IndentStyle::Tab => "\t".repeat(levels),
        IndentStyle::Space => " ".repeat(levels * usize::from(options.indent_width())),
    }
}

fn pop_next_heredoc_delimiter(
    pending: &mut Vec<RenderedHeredocDelimiter>,
) -> Option<RenderedHeredocDelimiter> {
    (!pending.is_empty()).then(|| pending.remove(0))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedHeredocDelimiter {
    pub(crate) delimiter: String,
    pub(crate) strip_tabs: bool,
}

impl RenderedHeredocDelimiter {
    pub(crate) fn matches_line(&self, line: &str) -> bool {
        line == self.delimiter || self.strip_tabs && line.trim_start_matches('\t') == self.delimiter
    }
}

pub(crate) fn heredoc_delimiters_in_rendered_line(line: &str) -> Vec<RenderedHeredocDelimiter> {
    let mut delimiters = Vec::new();
    let mut search_start = 0;
    while let Some(offset) = line[search_start..].find("<<") {
        let operator_start = search_start + offset;
        let mut delimiter_start = operator_start + "<<".len();
        let rest = &line[delimiter_start..];
        if rest.starts_with('<') {
            search_start = delimiter_start + '<'.len_utf8();
            continue;
        }
        if heredoc_operator_is_in_non_redirection_context(line, operator_start) {
            search_start = delimiter_start;
            continue;
        }

        let mut rest = rest;
        let strip_tabs = rest.starts_with('-');
        if rest.starts_with('-') {
            delimiter_start += '-'.len_utf8();
            rest = &line[delimiter_start..];
        }
        let trimmed = rest.trim_start();
        delimiter_start += rest.len() - trimmed.len();
        rest = trimmed;
        let token_end = rest
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '&' | '|'))
            .unwrap_or(rest.len());
        if token_end == 0 {
            search_start = delimiter_start;
            continue;
        }

        let token = &rest[..token_end];
        delimiters.push(RenderedHeredocDelimiter {
            delimiter: unquote_heredoc_delimiter(token),
            strip_tabs,
        });
        search_start = delimiter_start + token_end;
    }
    delimiters
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderedLineQuote {
    Single,
    Double,
}

fn heredoc_operator_is_in_non_redirection_context(line: &str, operator_start: usize) -> bool {
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut arithmetic_depth = 0usize;
    let mut conditional_depth = 0usize;
    let mut double_quoted_command_substitution_depth = 0usize;

    while index < operator_start {
        let rest = &line[index..operator_start];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if escaped {
            escaped = false;
            index += ch_len;
            continue;
        }

        match quote {
            Some(RenderedLineQuote::Single) => {
                if ch == '\'' {
                    quote = None;
                }
                index += ch_len;
                continue;
            }
            Some(RenderedLineQuote::Double) => {
                if ch == '\\' {
                    escaped = true;
                    index += ch_len;
                    continue;
                }
                if rest.starts_with("$((") {
                    arithmetic_depth += 1;
                    index += "$((".len();
                    continue;
                }
                if arithmetic_depth > 0 && rest.starts_with("))") {
                    arithmetic_depth -= 1;
                    index += "))".len();
                    continue;
                }
                if rest.starts_with("$(") {
                    double_quoted_command_substitution_depth += 1;
                    index += "$(".len();
                    continue;
                }
                if double_quoted_command_substitution_depth > 0 && ch == ')' {
                    double_quoted_command_substitution_depth -= 1;
                    index += ch_len;
                    continue;
                }
                if ch == '"' && double_quoted_command_substitution_depth == 0 {
                    quote = None;
                }
                index += ch_len;
                continue;
            }
            None => {}
        }

        if ch == '\\' {
            escaped = true;
            index += ch_len;
            continue;
        }
        if ch == '\'' {
            quote = Some(RenderedLineQuote::Single);
            index += ch_len;
            continue;
        }
        if ch == '"' {
            quote = Some(RenderedLineQuote::Double);
            index += ch_len;
            continue;
        }
        if rest.starts_with("$((") {
            arithmetic_depth += 1;
            index += "$((".len();
            continue;
        }
        if arithmetic_depth > 0 && rest.starts_with("))") {
            arithmetic_depth -= 1;
            index += "))".len();
            continue;
        }
        if rest.starts_with("((") && shell_word_starts_at_boundary(line, index) {
            arithmetic_depth += 1;
            index += "((".len();
            continue;
        }
        if rest.starts_with("[[") && shell_word_starts_at_boundary(line, index) {
            conditional_depth += 1;
            index += "[[".len();
            continue;
        }
        if conditional_depth > 0 && rest.starts_with("]]") {
            conditional_depth -= 1;
            index += "]]".len();
            continue;
        }

        index += ch_len;
    }

    escaped
        || arithmetic_depth > 0
        || conditional_depth > 0
        || matches!(quote, Some(RenderedLineQuote::Single))
        || matches!(quote, Some(RenderedLineQuote::Double))
            && double_quoted_command_substitution_depth == 0
}

fn shell_word_starts_at_boundary(line: &str, index: usize) -> bool {
    line[..index].chars().next_back().is_none_or(|ch| {
        ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '{' | '}')
    })
}

fn unquote_heredoc_delimiter(token: &str) -> String {
    let mut delimiter = String::with_capacity(token.len());
    let mut chars = token.chars().peekable();
    let mut quote = None;
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(active), current) if active == current => quote = None,
            (_, '\\') => {
                if let Some(next) = chars.next() {
                    delimiter.push(next);
                }
            }
            _ => delimiter.push(ch),
        }
    }
    delimiter
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
    push_arithmetic_expr(rendered, expr, ArithmeticContext::TopLevel, source, options);
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

fn push_arithmetic_expr(
    rendered: &mut String,
    expr: &ArithmeticExprNode,
    context: ArithmeticContext,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let needs_parentheses = arithmetic_needs_parentheses(expr, context);
    if needs_parentheses {
        rendered.push('(');
    }

    match &expr.kind {
        ArithmeticExpr::Number(number) => rendered.push_str(number.slice(source)),
        ArithmeticExpr::Variable(name) => rendered.push_str(name),
        ArithmeticExpr::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(
                rendered,
                index,
                ArithmeticContext::Subscript,
                source,
                options,
            );
            rendered.push(']');
        }
        ArithmeticExpr::ShellWord(word) => {
            rendered.push_str(&render_arithmetic_shell_word(word, source, options));
        }
        ArithmeticExpr::Parenthesized { expression } => {
            rendered.push('(');
            push_arithmetic_expr(
                rendered,
                expression,
                ArithmeticContext::TopLevel,
                source,
                options,
            );
            rendered.push(')');
        }
        ArithmeticExpr::Unary { op, expr } => {
            rendered.push_str(arithmetic_unary_operator(*op));
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Unary, source, options);
        }
        ArithmeticExpr::Postfix { expr, op } => {
            push_arithmetic_expr(rendered, expr, ArithmeticContext::Postfix, source, options);
            rendered.push_str(arithmetic_postfix_operator(*op));
        }
        ArithmeticExpr::Binary { left, op, right } => {
            push_arithmetic_expr(
                rendered,
                left,
                ArithmeticContext::Binary(*op),
                source,
                options,
            );
            if matches!(op, ArithmeticBinaryOp::Comma) {
                rendered.push_str(", ");
            } else {
                rendered.push(' ');
                rendered.push_str(arithmetic_binary_operator(*op));
                rendered.push(' ');
            }
            push_arithmetic_expr(
                rendered,
                right,
                ArithmeticContext::Binary(*op),
                source,
                options,
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
                source,
                options,
            );
            rendered.push_str(" ? ");
            push_arithmetic_expr(
                rendered,
                then_expr,
                ArithmeticContext::ConditionalBranch,
                source,
                options,
            );
            rendered.push_str(" : ");
            push_arithmetic_expr(
                rendered,
                else_expr,
                ArithmeticContext::ConditionalBranch,
                source,
                options,
            );
        }
        ArithmeticExpr::Assignment { target, op, value } => {
            push_arithmetic_lvalue(rendered, target, source, options);
            rendered.push(' ');
            rendered.push_str(arithmetic_assign_operator(*op));
            rendered.push(' ');
            push_arithmetic_expr(
                rendered,
                value,
                ArithmeticContext::Assignment,
                source,
                options,
            );
        }
    }

    if needs_parentheses {
        rendered.push(')');
    }
}

fn render_arithmetic_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    if options.simplify() || options.minify() {
        let [part] = word.parts.as_slice() else {
            return render_word_syntax(word, source, options);
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
            _ => render_word_syntax(word, source, options),
        };
    }

    render_word_syntax(word, source, options)
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
        ArithmeticContext::Unary => {
            expr_prec < arithmetic_precedence_value(ArithmeticBinaryOp::Power)
        }
        ArithmeticContext::Postfix => {
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
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    match target {
        ArithmeticLvalue::Variable(name) => rendered.push_str(name),
        ArithmeticLvalue::Indexed { name, index } => {
            rendered.push_str(name);
            rendered.push('[');
            push_arithmetic_expr(
                rendered,
                index,
                ArithmeticContext::Subscript,
                source,
                options,
            );
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
            _ => render_arithmetic_expr_to_buf(rendered, ast, source, options),
        }
    } else {
        rendered.push_str(text.slice(source));
    }
}

fn push_parameter_slice_arithmetic(
    rendered: &mut String,
    text: &shuck_ast::SourceText,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let start = rendered.len();
    push_arithmetic_source_text(rendered, text, ast, source, options);
    if text.slice(source).trim_start().starts_with('(') {
        let trimmed = rendered[start..].trim_start().to_string();
        rendered.truncate(start);
        rendered.push_str(&trimmed);
    }
}

fn render_arithmetic_slice_shell_word(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let trim_leading_parenthesized_offset = word.span.slice(source).trim_start().starts_with('(');
    let [part] = word.parts.as_slice() else {
        let rendered = render_word_syntax(word, source, options);
        return if trim_leading_parenthesized_offset {
            rendered.trim_start().to_string()
        } else {
            rendered
        };
    };

    match &part.kind {
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } => match syntax {
            ArithmeticExpansionSyntax::DollarParenParen => {
                let mut rendered = String::from("$((");
                if let Some(ast) = expression_ast.as_deref() {
                    render_arithmetic_expr_to_buf(&mut rendered, ast, source, options);
                } else {
                    rendered.push_str(expression.slice(source).trim());
                }
                rendered.push_str("))");
                rendered
            }
            ArithmeticExpansionSyntax::LegacyBracket => {
                let mut rendered = String::from("$[");
                if let Some(ast) = expression_ast.as_deref() {
                    render_arithmetic_expr_to_buf(&mut rendered, ast, source, options);
                } else {
                    rendered.push_str(expression.slice(source).trim());
                }
                rendered.push(']');
                rendered
            }
        },
        _ => {
            let rendered = render_word_syntax(word, source, options);
            if trim_leading_parenthesized_offset {
                rendered.trim_start().to_string()
            } else {
                rendered
            }
        }
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
        } else if let Some(ast) = subscript.arithmetic_ast.as_ref()
            && subscript_prefers_structured_arithmetic(subscript, source)
        {
            render_arithmetic_expr_to_buf(rendered, ast, source, options);
        } else if let Some(word) = subscript.word_ast()
            && word_contains_structured_arithmetic_expansion(word)
        {
            render_word_syntax_to_buf(word, source, options, rendered);
        } else {
            rendered.push_str(subscript.syntax_text(source));
        }
        rendered.push(']');
    }
}

fn word_contains_structured_arithmetic_expansion(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| part_contains_structured_arithmetic_expansion(&part.kind))
}

fn part_contains_structured_arithmetic_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_structured_arithmetic_expansion(&part.kind)),
        WordPart::ArithmeticExpansion {
            expression_ast: Some(_),
            ..
        } => true,
        _ => false,
    }
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
        rendered.push_str(raw);
        rendered.push('}');
        return Ok(());
    };

    match syntax {
        BourneParameterExpansion::Access { reference } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        BourneParameterExpansion::Length { reference } => {
            rendered.push_str("${#");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
        }
        BourneParameterExpansion::Indices { reference } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, source, options);
            rendered.push('}');
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
            push_parameter_slice_arithmetic(
                rendered,
                offset,
                offset_ast.as_deref(),
                source,
                options,
            );
            if let Some(length) = length {
                rendered.push(':');
                push_parameter_slice_arithmetic(
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
                source,
                options,
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

fn render_parameter_expansion(
    rendered: &mut String,
    reference: &VarRef,
    operator: &ParameterOp,
    operand: Option<&shuck_ast::SourceText>,
    colon_variant: bool,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> Result<(), std::fmt::Error> {
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
                push_parameter_default_operand(rendered, operand, source, options);
            }
        }
        ParameterOp::RemovePrefixShort { pattern } => {
            rendered.push('#');
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
        }
        ParameterOp::RemovePrefixLong { pattern } => {
            rendered.push_str("##");
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
        }
        ParameterOp::RemoveSuffixShort { pattern } => {
            rendered.push('%');
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
        }
        ParameterOp::RemoveSuffixLong { pattern } => {
            rendered.push_str("%%");
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
            ..
        } => {
            rendered.push('/');
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
            rendered.push('/');
            rendered.push_str(replacement.slice(source));
        }
        ParameterOp::ReplaceAll {
            pattern,
            replacement,
            ..
        } => {
            rendered.push_str("//");
            render_pattern_syntax_to_buf(pattern, source, options, rendered);
            rendered.push('/');
            rendered.push_str(replacement.slice(source));
        }
        ParameterOp::UpperFirst => rendered.push('^'),
        ParameterOp::UpperAll => rendered.push_str("^^"),
        ParameterOp::LowerFirst => rendered.push(','),
        ParameterOp::LowerAll => rendered.push_str(",,"),
    }
    rendered.push('}');
    Ok(())
}

fn push_parameter_default_operand(
    rendered: &mut String,
    operand: &shuck_ast::SourceText,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let raw = operand.slice(source);
    if let Some(formatted) = format_arithmetic_expansion_operand(raw, options) {
        rendered.push_str(&formatted);
    } else {
        rendered.push_str(raw);
    }
}

fn format_arithmetic_expansion_operand(
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix("$((")?.strip_suffix("))")?;
    let spaced_inner = format_simple_arithmetic_operand_spacing(inner);
    let synthetic = format!("(( {inner} ))");
    let parsed = Parser::new(&synthetic).parse();
    if parsed.is_err() {
        return (spaced_inner != inner.trim()).then(|| format!("$(({spaced_inner}))"));
    }
    let statement = parsed.file.body.first()?;
    let Command::Compound(CompoundCommand::Arithmetic(command)) = &statement.command else {
        return None;
    };
    let expr = command.expr_ast.as_ref()?;
    let mut formatted = String::from("$((");
    render_arithmetic_expr_to_buf(&mut formatted, expr, &synthetic, options);
    if formatted == "$((" && spaced_inner != inner.trim() {
        formatted.push_str(&spaced_inner);
        formatted.push_str("))");
        return Some(formatted);
    }
    formatted.push_str("))");
    if formatted == trimmed && spaced_inner != inner.trim() {
        return Some(format!("$(({spaced_inner}))"));
    }
    Some(formatted)
}

fn format_simple_arithmetic_operand_spacing(inner: &str) -> String {
    let mut formatted = String::new();
    let chars = inner.trim().chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut parameter_depth = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '$' && chars.get(index + 1) == Some(&'{') {
            parameter_depth += 1;
            formatted.push(ch);
            index += 1;
            continue;
        }
        if ch == '}' && parameter_depth > 0 {
            parameter_depth -= 1;
            formatted.push(ch);
            index += 1;
            continue;
        }
        if parameter_depth == 0 && matches!(ch, '+' | '-' | '*' | '/' | '%') {
            while formatted.ends_with(' ') {
                formatted.pop();
            }
            formatted.push(' ');
            formatted.push(ch);
            formatted.push(' ');
            index += 1;
            while chars.get(index).is_some_and(|next| next.is_whitespace()) {
                index += 1;
            }
            continue;
        }
        formatted.push(ch);
        index += 1;
    }
    formatted
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

pub(crate) fn render_pattern_syntax(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let mut rendered = String::new();
    render_pattern_syntax_to_buf(pattern, source, options, &mut rendered);
    rendered
}

pub(crate) fn render_pattern_syntax_to_buf(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
    rendered: &mut String,
) {
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

fn raw_word_source_slice<'a>(word: &Word, source: &'a str) -> Option<&'a str> {
    raw_source_slice(word.span, source)
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

fn could_need_preserve_raw_syntax(raw: &str) -> bool {
    raw.starts_with('\\')
        || raw.starts_with('&')
        || raw.starts_with("$'")
        || raw.contains("\\\"")
        || raw.contains("\\`")
        || raw.contains("\\\\")
        || raw.contains("[^ ]")
}

fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text.as_bytes()[..whitespace_start]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}

#[cfg(test)]
mod tests {
    use crate::ShellFormatOptions;

    use super::*;

    #[test]
    fn indented_rendered_block_ignores_arithmetic_shift_as_heredoc() {
        let options = ShellFormatOptions::default().resolve("", None);
        let mut rendered = String::new();

        push_indented_rendered_block(&mut rendered, "echo $((1 << 2))\necho done", &options, 1);

        assert_eq!(rendered, "\techo $((1 << 2))\n\techo done");
    }

    #[test]
    fn indented_rendered_block_keeps_heredoc_body_verbatim() {
        let options = ShellFormatOptions::default().resolve("", None);
        let mut rendered = String::new();

        push_indented_rendered_block(
            &mut rendered,
            "cat <<EOF\nbody\nEOF\necho done",
            &options,
            1,
        );

        assert_eq!(rendered, "\tcat <<EOF\nbody\nEOF\n\techo done");
    }

    #[test]
    fn indented_rendered_block_closes_tab_stripped_heredocs() {
        let options = ShellFormatOptions::default().resolve("", None);
        let mut rendered = String::new();

        push_indented_rendered_block(
            &mut rendered,
            "cat <<-EOF\n\tbody\n\tEOF\necho done",
            &options,
            1,
        );

        assert_eq!(rendered, "\tcat <<-EOF\n\tbody\n\tEOF\n\techo done");
    }
}
