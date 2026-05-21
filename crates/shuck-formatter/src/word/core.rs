use super::arithmetic::*;
use super::command_substitution::*;
use super::parameter::*;
use super::raw_rewrites::*;
use super::*;

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

pub(super) fn source_fragment_is_line_continuation_padding(fragment: &str) -> bool {
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

pub(super) fn word_part_nodes_any(
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

pub(super) fn render_word_syntax_internal(
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
pub(super) fn word_has_escaped_command_substitution(word: &Word, source: &str) -> bool {
    if raw_word_source_slice(word, source)
        .is_some_and(|raw| raw.contains("\\$(") || raw.contains("\\`"))
    {
        return true;
    }

    word_part_nodes_any(&word.parts, &mut |part| {
        word_part_has_escaped_command_substitution(&part.kind, part.span, source)
    })
}

pub(super) fn word_part_has_escaped_command_substitution(
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

pub(super) fn raw_escaped_multiline_double_quoted_word(raw: &str) -> bool {
    raw.strip_prefix('$').unwrap_or(raw).starts_with("\"\\\n")
        || raw.strip_prefix('$').unwrap_or(raw).starts_with("\"\\\r\n")
}

pub(super) fn raw_single_line_escaped_quote_command_substitution_should_preserve(
    raw: &str,
) -> bool {
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

pub(super) fn word_needs_special_rendering(word: &Word) -> bool {
    word_part_nodes_any(&word.parts, &mut |part| {
        part_needs_special_rendering(&part.kind)
    })
}

pub(super) fn word_needs_formatter_rendering(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    word_part_nodes_any(&word.parts, &mut |part| {
        word_part_needs_formatter_rendering(part, source, options)
    })
}

pub(super) fn word_part_needs_formatter_rendering(
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

pub(super) fn word_part_has_parameter_raw_subscript_needs_compaction(
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

pub(super) fn word_part_has_parameter_command_redirect_spacing_needs_normalization(
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

pub(super) fn word_part_has_arithmetic_expansion_source_needs_trim(
    part: &WordPart,
    source: &str,
) -> bool {
    match part {
        WordPart::ArithmeticExpansion { expression, .. } => {
            let raw = expression.slice(source);
            raw.trim_matches([' ', '\t', '\r']).len() != raw.len()
        }
        _ => false,
    }
}

pub(super) fn word_has_multiline_double_quoted_source(word: &Word, source: &str) -> bool {
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
        .is_some_and(|body| !classify_sequence_contains_multiline_literal_source(body, source))
}

pub(crate) fn word_is_quoted_formattable_command_substitution_only_with_facts(
    word: &Word,
    facts: &FormatterFacts<'_>,
) -> bool {
    quoted_command_substitution_only_body(word)
        .is_some_and(|body| !facts.sequence_contains_multiline_literal_source(body))
}

pub(crate) fn word_is_quoted_command_substitution_only(word: &Word) -> bool {
    quoted_command_substitution_only_body(word).is_some()
}

pub(super) fn quoted_command_substitution_only_body(word: &Word) -> Option<&StmtSeq> {
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

pub(super) fn part_needs_special_rendering(part: &WordPart) -> bool {
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

pub(super) fn render_word_parts(
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
pub(super) struct WordPartRenderContext {
    allow_source_indented_inline_command_substitution: bool,
    source_indented_inline_command_substitution: bool,
}

pub(super) fn render_word_part(
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
                    facts,
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
                } else if stmt_seq_contains_comments(facts, body) {
                    let fallback = RawCommandSubstitutionCommentFallback {
                        raw,
                        body,
                        source,
                        span_start: span.start.offset,
                        options,
                        facts,
                    };
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
                                rendered, fallback, false,
                            );
                        }
                    } else {
                        push_raw_command_substitution_comment_fallback(rendered, fallback, true);
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
                    facts,
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
                if stmt_seq_contains_comments(facts, body) {
                    if process_substitution_source_opens_to_body_line(raw)
                        && !stmt_seq_has_heredoc(facts, body)
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

pub(super) fn push_braced_var_ref(
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

pub(super) fn literal_ends_with_line_indent_for_word_part(literal: &str) -> bool {
    let Some((_, suffix)) = literal.rsplit_once('\n') else {
        return false;
    };
    suffix.chars().all(|ch| matches!(ch, ' ' | '\t'))
}

pub(super) fn preferred_raw_word_part_source<'a>(
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

pub(super) fn parameter_raw_subscript_needs_compaction(
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

pub(super) fn parameter_bourne_subscript(
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

pub(super) fn push_unquoted_literal(rendered: &mut String, literal: &str) {
    if !literal.contains("\\\n") && !literal.contains("\\\r\n") {
        rendered.push_str(literal);
        return;
    }

    let mut chars = literal.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(skipped_indent) = consume_escaped_newline_indent(&mut chars)
        {
            if skipped_indent {
                rendered.push(' ');
            }
            continue;
        }
        rendered.push(ch);
    }
}

pub(super) fn render_double_quoted_literal(rendered: &mut String, text: &str) {
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
#[derive(Clone, Copy)]
pub(super) struct WordRenderEnv<'source, 'a> {
    pub(super) source: &'source str,
    pub(super) options: &'a ResolvedShellFormatOptions,
    pub(super) source_map: Option<&'a SourceMap<'source>>,
    pub(super) facts: Option<&'a FormatterFacts<'source>>,
}

impl<'source, 'a> WordRenderEnv<'source, 'a> {
    pub(super) fn new(
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
