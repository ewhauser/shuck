use super::arithmetic::*;
use super::command_substitution::*;
use super::core::WordRenderEnv;
use super::parameter::*;
use super::raw_rewrites::*;
use super::*;

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
pub(super) fn render_heredoc_body_part(
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
                facts,
                raw,
            )
            .is_none()
            {
                let layout = command_substitution_layout(
                    raw,
                    body,
                    Some(facts),
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

pub(super) fn escaped_heredoc_expansion_source(
    span: shuck_ast::Span,
    source: &str,
) -> Option<&str> {
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

pub(super) fn render_heredoc_body_command_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    upper_bound: usize,
    source: &str,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
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
        Some(facts),
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

pub(super) fn heredoc_command_substitution_body_prefix(
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

pub(super) fn heredoc_command_substitution_close_prefix(
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
