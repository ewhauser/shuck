use super::arithmetic::*;
use super::command_substitution::*;
use super::core::*;
use super::raw_rewrites::*;
use super::*;

pub(super) fn push_parameter_word(
    rendered: &mut String,
    parameter: &shuck_ast::ParameterExpansion,
    context: RenderContext<'_, '_>,
) -> Result<(), std::fmt::Error> {
    let source = context.source;
    let Some(syntax) = parameter.bourne() else {
        let raw = parameter.raw_body.slice(source);
        rendered.push_str("${");
        rendered.push_str(&compact_raw_parameter_subscript(raw));
        rendered.push('}');
        return Ok(());
    };

    match syntax {
        BourneParameterExpansion::Access { reference } => {
            push_braced_var_ref(rendered, "", reference, context);
        }
        BourneParameterExpansion::Length { reference } => {
            push_braced_var_ref(rendered, "#", reference, context);
        }
        BourneParameterExpansion::Indices { reference } => {
            push_braced_var_ref(rendered, "!", reference, context);
        }
        BourneParameterExpansion::Indirect {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            rendered.push_str("${!");
            push_var_ref(rendered, reference, context);
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
            push_var_ref(rendered, reference, context);
            rendered.push(':');
            push_parameter_slice_offset(rendered, offset, offset_ast.as_deref(), context);
            if let Some(length) = length {
                rendered.push(':');
                push_arithmetic_source_text(rendered, length, length_ast.as_deref(), context);
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
                context,
            )?;
        }
        BourneParameterExpansion::Transformation {
            reference,
            operator,
        } => {
            rendered.push_str("${");
            push_var_ref(rendered, reference, context);
            rendered.push('@');
            std::write!(rendered, "{operator}")?;
            rendered.push('}');
        }
    }

    Ok(())
}

pub(super) fn push_parameter_operand(
    rendered: &mut String,
    operand: &shuck_ast::SourceText,
    context: RenderContext<'_, '_>,
) {
    let operand = compact_parameter_operand_subscripts(operand.slice(context.source));
    if operand.contains("$(") || operand.contains('`') {
        let mut normalized = String::new();
        push_raw_shell_text_with_normalized_redirect_spacing(&mut normalized, &operand);
        if let Some(command_normalized) =
            normalize_inline_command_substitutions_in_parameter_operand(
                &normalized,
                context.options,
            )
        {
            rendered.push_str(&command_normalized);
        } else {
            rendered.push_str(&normalized);
        }
    } else {
        rendered.push_str(&operand);
    }
}

pub(super) fn normalize_inline_command_substitutions_in_parameter_operand(
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

pub(super) fn next_raw_command_substitution(raw: &str, index: usize) -> Option<(usize, usize)> {
    RawShellText::new(raw).next_command_substitution(index)
}

pub(super) fn finish_raw_rewrite(
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

pub(super) fn normalize_inline_parameter_command_substitution_body(
    body: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    let trimmed = body.trim_matches([' ', '\t', '\r']);
    if trimmed.is_empty() {
        return None;
    }

    let fragment = FragmentFormatter::parse(trimmed, options)?;
    let nested = fragment.format_body(None).ok()?;
    let formatted = trim_trailing_line_endings(&nested);
    (!formatted.is_empty() && !formatted.contains('\n')).then(|| formatted.to_string())
}

pub(super) fn compact_raw_parameter_subscript(raw: &str) -> String {
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

pub(super) fn compact_parameter_operand_subscripts(text: &str) -> String {
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

pub(super) fn render_parameter_expansion(
    rendered: &mut String,
    reference: &VarRef,
    operator: &ParameterOp,
    operand: Option<&shuck_ast::SourceText>,
    colon_variant: bool,
    raw_parameter_span: Option<shuck_ast::Span>,
    env: RenderContext<'_, '_>,
) -> Result<(), std::fmt::Error> {
    let (source, options) = (env.source, env.options);

    rendered.push_str("${");
    push_var_ref(rendered, reference, env);
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
                push_parameter_operand(rendered, operand, env);
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
            render_pattern_syntax_to_buf(pattern, env, rendered);
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
                render_parameter_replacement_pattern(rendered, pattern, env);
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

pub(super) fn raw_parameter_replacement_parts<'a>(
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

pub(super) fn split_raw_parameter_replacement(raw: &str) -> (&str, &str) {
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

pub(super) fn render_parameter_replacement_pattern(
    rendered: &mut String,
    pattern: &Pattern,
    context: RenderContext<'_, '_>,
) {
    let source = context.source;
    let options = context.options;
    if !options.simplify()
        && !options.minify()
        && let Some(raw) = raw_pattern_source_slice(pattern, source)
    {
        rendered.push_str(raw);
        return;
    }

    render_pattern_syntax_to_buf(pattern, context, rendered);
}

pub(super) fn push_parameter_replacement_text(
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

pub(crate) fn parameter_defaulting_operator(operator: &ParameterOp) -> &'static str {
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
    context: RenderContext<'_, '_>,
    rendered: &mut String,
) {
    let source = context.source;
    let options = context.options;
    if pattern_needs_formatter_rendering(pattern) {
        render_pattern_parts_syntax_to_buf(pattern, context, rendered);
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

pub(super) fn pattern_needs_formatter_rendering(pattern: &Pattern) -> bool {
    pattern.parts.iter().any(|part| match &part.kind {
        PatternPart::Word(word) => word_needs_special_rendering(word),
        PatternPart::Group { patterns, .. } => {
            patterns.iter().any(pattern_needs_formatter_rendering)
        }
        _ => false,
    })
}

pub(super) fn render_pattern_parts_syntax_to_buf(
    pattern: &Pattern,
    context: RenderContext<'_, '_>,
    rendered: &mut String,
) {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Word(word) => {
                render_word_syntax_to_buf(word, context, rendered);
            }
            PatternPart::Group { kind, patterns } => {
                let _ = std::write!(rendered, "{}(", kind.prefix());
                for (index, pattern) in patterns.iter().enumerate() {
                    if index > 0 {
                        rendered.push('|');
                    }
                    render_pattern_syntax_to_buf(pattern, context, rendered);
                }
                rendered.push(')');
            }
            _ => {
                let single = Pattern {
                    parts: vec![part.clone()],
                    span: part.span,
                };
                single.render_syntax_to_buf(context.source, rendered);
            }
        }
    }
}
