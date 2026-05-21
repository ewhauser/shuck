use super::command_substitution::*;
use super::parameter::*;
use super::*;

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
        if ch == '\\'
            && !in_single_quotes
            && !in_double_quotes
            && let Some(skipped_indent) = consume_escaped_newline_indent(&mut chars)
        {
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
        normalized.push(ch);
    }

    changed.then_some(normalized)
}

pub(super) fn consume_escaped_newline_indent(
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

pub(super) fn normalize_raw_compound_assignment_word_continuations(raw: &str) -> Option<String> {
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

pub(super) fn raw_compound_assignment_head_is_simple(head: &str) -> bool {
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

pub(super) fn parameter_bourne_operand_needs_subscript_compaction(
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

pub(super) fn parameter_needs_special_rendering(parameter: &shuck_ast::ParameterExpansion) -> bool {
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

pub(super) fn parameter_prefers_raw_source(
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

pub(super) fn stmt_seq_contains_comments(
    facts: Option<&FormatterFacts<'_>>,
    sequence: &StmtSeq,
) -> bool {
    facts.map_or_else(
        || classify_sequence_contains_comments(sequence),
        |facts| facts.sequence_contains_comments(sequence),
    )
}

pub(super) fn stmt_seq_has_heredoc(facts: Option<&FormatterFacts<'_>>, sequence: &StmtSeq) -> bool {
    facts.map_or_else(
        || classify_sequence_contains_heredoc(sequence),
        |facts| facts.sequence_contains_heredoc(sequence),
    )
}

pub(super) fn normalize_raw_backtick_command_substitution(raw: &str) -> Option<String> {
    let body = raw.strip_prefix('`')?.strip_suffix('`')?;
    let body = normalize_backtick_body_escaped_dollars(body);
    Some(format!("$({body})"))
}

pub(super) fn raw_block_line_is_outer_substitution_close(lines: &[&str], index: usize) -> bool {
    lines
        .get(index.saturating_add(1)..)
        .is_none_or(|remaining| {
            remaining
                .iter()
                .all(|line| line.trim_matches([' ', '\t', '\r']).is_empty())
        })
}

pub(super) fn normalize_continuations_before_comment_lines(text: &str) -> Option<String> {
    normalize_continuations_before_matching_lines(text, false, |next| next.starts_with('#'))
}

pub(super) fn normalize_continuations_before_substitution_close_lines(
    text: &str,
) -> Option<String> {
    normalize_continuations_before_matching_lines(text, true, raw_line_closes_substitution_wrapper)
}

pub(super) fn normalize_continuations_before_matching_lines(
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
pub(super) struct RawCompoundIndentShift {
    pub(super) source_indent: String,
    pub(super) extra_units: usize,
    pub(super) close_keyword: &'static str,
}

pub(super) struct RawCompoundCommentIndent {
    pub(super) child_indent: String,
    pub(super) close_keyword: &'static str,
    pub(super) pipeline_continuation: bool,
}

#[derive(Default)]
pub(super) struct RawCompoundIndentState {
    pub(super) shifts: Vec<RawCompoundIndentShift>,
    pub(super) comments: Vec<RawCompoundCommentIndent>,
}

impl RawCompoundIndentState {
    pub(super) fn shifted_line(
        &self,
        line: &str,
        options: &ResolvedShellFormatOptions,
    ) -> Option<String> {
        let shift = self.shifts.last()?;
        raw_line_indent_matches_shift(line, shift)
            .then(|| add_raw_indent_units(line, shift.extra_units, options))
    }

    pub(super) fn in_body(&self, content: &str) -> bool {
        self.comments.last().is_some_and(|compound| {
            !content.trim().is_empty()
                && !raw_line_closes_compound(content, compound.close_keyword)
                && !raw_line_is_compound_mid_keyword(content)
        })
    }

    pub(super) fn child_indent_if_underindented<'a>(
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

    pub(super) fn closes_last(&self, content: &str) -> bool {
        self.comments
            .last()
            .is_some_and(|compound| raw_line_closes_compound(content, compound.close_keyword))
    }

    pub(super) fn closes_pipeline_stage(&self, content: &str) -> bool {
        self.comments.last().is_some_and(|compound| {
            compound.pipeline_continuation
                && raw_line_closes_compound(content, compound.close_keyword)
        })
    }

    pub(super) fn update_line(
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

pub(super) fn raw_line_indent_matches_shift(line: &str, shift: &RawCompoundIndentShift) -> bool {
    let (indent, content) = raw_line_parts(line);
    !content.trim().is_empty() && raw_indent_starts_with(indent, &shift.source_indent)
}

pub(super) fn raw_line_parts(line: &str) -> (&str, &str) {
    let indent = line_leading_shell_indent(line);
    (indent, &line[indent.len()..])
}

pub(super) fn raw_indent_starts_with(indent: &str, prefix: &str) -> bool {
    indent == prefix || indent.starts_with(prefix)
}

pub(super) fn add_raw_indent_units(
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

pub(super) fn push_raw_shell_line_with_normalized_source_indent(
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

pub(super) fn push_raw_shell_line_with_rendered_indent(
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

pub(super) fn rendered_raw_shell_indent_for_line(
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

pub(super) fn strip_semicolon_before_trailing_comment(line: &str) -> Option<String> {
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

pub(super) fn normalize_padding_before_trailing_comment(line: &str) -> Option<String> {
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

pub(super) fn trailing_comment_start(line: &str) -> Option<usize> {
    RawShellScanner::new(line).find_comment(0, line.len())
}

pub(super) fn raw_line_closes_substitution_wrapper(content: &str) -> bool {
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

pub(super) fn push_raw_shell_text_with_normalized_redirect_spacing(
    target: &mut String,
    text: &str,
) {
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

pub(super) fn push_raw_shell_line_content_with_normalized_spacing(
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

pub(super) fn expand_inline_raw_command_substitutions_in_line(
    target: &mut String,
    line: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if !line.contains("$(") {
        return false;
    }

    let mut changed = false;
    let mut last = 0usize;
    let mut index = 0usize;
    let mut quote = QuoteState::default();
    let scanner = RawShellScanner::new(line);
    let bytes = line.as_bytes();

    while index < line.len() {
        let Some(ch) = line[index..].chars().next() else {
            break;
        };
        let next_index = index + ch.len_utf8();
        if quote.consume_raw_char(ch, false) {
            index = next_index;
            continue;
        }
        if ch == '#' && scanner.find_comment(index, next_index).is_some() {
            break;
        }
        if ch == '$'
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

        index = next_index;
    }

    if changed {
        push_raw_shell_line_with_normalized_redirect_spacing(target, &line[last..]);
    }
    changed
}

pub(super) fn render_inline_raw_command_substitution_as_block(
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
    let parsed_facts = FormatterFacts::build(body, &parsed.file, options);
    let inline_multiline = parsed_facts
        .sequence_contains_multistatement_pipeline_brace_group(&parsed.file.body)
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

pub(super) fn raw_body_contains_pipeline_multistatement_brace_group(body: &str) -> bool {
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

pub(super) fn raw_brace_group_has_multiple_commands(body_after_open: &str) -> bool {
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

pub(super) fn raw_command_redirect_spacing_would_change(raw: &str) -> bool {
    if !(raw.contains('<') || raw.contains('>')) {
        return false;
    }
    let mut normalized = String::with_capacity(raw.len());
    push_raw_shell_text_with_normalized_redirect_spacing(&mut normalized, raw);
    normalized != raw
}

pub(super) fn push_preserved_raw_word_source(
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

pub(super) fn raw_parameter_command_spacing_would_change(raw: &str) -> bool {
    raw_command_redirect_spacing_would_change(raw)
        || raw_command_substitution_needs_structural_spacing(raw)
}

pub(super) fn raw_command_substitution_needs_structural_spacing(raw: &str) -> bool {
    let mut index = 0usize;

    while let Some((open_offset, close_offset)) = next_raw_command_substitution(raw, index) {
        if raw_shell_body_needs_structural_spacing(&raw[open_offset + 2..close_offset]) {
            return true;
        }
        index = close_offset + 1;
    }

    false
}

pub(super) fn raw_shell_body_needs_structural_spacing(body: &str) -> bool {
    let body = body.trim_matches([' ', '\t']);
    if raw_body_contains_pipeline_multistatement_brace_group(body) {
        return true;
    }
    let mut quote = QuoteState::default();
    let mut horizontal_run = 0usize;
    let mut index = 0usize;

    while index < body.len() {
        let rest = &body[index..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        let next_index = index + ch.len_utf8();

        if quote.consume_raw_char(ch, true) {
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

pub(super) fn normalize_raw_command_substitution_padding(raw: &str) -> Option<String> {
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

pub(super) fn trim_raw_command_substitution_horizontal_padding(body: &str) -> &str {
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
    let mut in_single_quotes = false;
    let mut escaped = false;

    while index + 1 < bytes.len() {
        let ch = raw[index..].chars().next()?;
        let next_index = index + ch.len_utf8();
        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            index = next_index;
            continue;
        }
        if escaped {
            escaped = false;
            index = next_index;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index = next_index;
            continue;
        }
        if ch == '\'' {
            in_single_quotes = true;
            index = next_index;
            continue;
        }

        if bytes[index] == b'$'
            && bytes[index + 1] == b'{'
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
        index = next_index;
    }

    finish_raw_rewrite(rendered, raw, cursor, changed)
}

pub(super) fn matching_raw_parameter_expansion_close(
    raw: &str,
    body_start: usize,
) -> Option<usize> {
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

pub(super) fn raw_parameter_replacement_needs_empty_delimiter(body: &str) -> bool {
    let Some(after_operator) = raw_parameter_replacement_body_after_operator(body) else {
        return false;
    };
    let (_, replacement) = split_raw_parameter_replacement(after_operator);
    if replacement.is_empty() {
        return !raw_has_final_replacement_delimiter(after_operator);
    }

    replacement_ends_with_ambiguous_quote(replacement)
}

pub(super) fn raw_parameter_replacement_body_after_operator(body: &str) -> Option<&str> {
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

pub(super) fn raw_parameter_subscript_end(body: &str, open: usize) -> Option<usize> {
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

pub(super) fn raw_has_final_replacement_delimiter(after_operator: &str) -> bool {
    let Some((last_index, _)) = after_operator.char_indices().next_back() else {
        return false;
    };
    after_operator[last_index..].starts_with('/')
        && !RawShellScanner::new(after_operator).is_escaped(last_index)
}

pub(super) fn replacement_ends_with_ambiguous_quote(replacement: &str) -> bool {
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

pub(super) fn normalize_raw_arithmetic_command_substitution_padding(raw: &str) -> Option<String> {
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

pub(super) fn normalize_raw_arithmetic_expansion_padding(raw: &str) -> Option<String> {
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

pub(super) fn matching_raw_arithmetic_expansion_close(
    raw: &str,
    body_start: usize,
) -> Option<usize> {
    let mut quote = QuoteState::default();
    let mut paren_depth = 0usize;
    let mut index = body_start;

    while index < raw.len() {
        let rest = &raw[index..];
        let ch = rest.chars().next()?;
        let next_index = index + ch.len_utf8();
        if quote.consume_raw_char(ch, true) {
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

pub(super) fn push_raw_shell_line_with_normalized_redirect_spacing(
    target: &mut String,
    line: &str,
) {
    let mut last = 0;
    let mut index = 0;
    let mut quote = QuoteState::default();
    let scanner = RawShellScanner::new(line);
    let bytes = line.as_bytes();

    while index < line.len() {
        let Some(ch) = line[index..].chars().next() else {
            break;
        };
        let next_index = index + ch.len_utf8();
        if quote.consume_shell_word_char(ch) {
            index = next_index;
            continue;
        }
        if !quote.in_quotes() && ch == '#' && scanner.find_comment(index, next_index).is_some() {
            break;
        }

        if !quote.in_single_quotes()
            && ch == '$'
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
            continue;
        }

        if !quote.in_quotes() && matches!(bytes[index], b' ' | b'\t' | b'\r') {
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
                continue;
            }
        }

        if !quote.in_quotes() && bytes[index].is_ascii_digit() {
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
                    continue;
                }
            }
            index = fd_start;
        }

        if !quote.in_quotes()
            && matches!(bytes[index], b'<' | b'>')
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
                continue;
            }
        }

        if !quote.in_quotes() && bytes.get(index..index + 3) == Some(b"<<<") {
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
                continue;
            }
        }

        index = next_index;
    }

    target.push_str(&line[last..]);
}

pub(super) fn raw_semicolon_can_attach_to_previous_word(
    bytes: &[u8],
    whitespace_start: usize,
) -> bool {
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

pub(super) fn raw_semicolon_is_single_terminator(bytes: &[u8], semicolon_start: usize) -> bool {
    !matches!(
        bytes.get(semicolon_start + 1).copied(),
        Some(b';' | b'&' | b'|')
    )
}

pub(super) fn raw_compound_close_keyword(content: &str) -> Option<&'static str> {
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

pub(super) fn raw_line_closes_compound(content: &str, close_keyword: &str) -> bool {
    raw_line_starts_with_keyword(content.trim_start_matches([' ', '\t', '\r']), close_keyword)
}

pub(super) fn raw_line_is_compound_mid_keyword(content: &str) -> bool {
    let content = content.trim_start_matches([' ', '\t', '\r']);
    raw_line_starts_with_keyword(content, "else")
        || raw_line_starts_with_keyword(content, "elif")
        || raw_line_starts_with_keyword(content, "then")
        || raw_line_starts_with_keyword(content, "do")
}

pub(super) fn raw_line_starts_with_keyword(line: &str, keyword: &str) -> bool {
    let Some(rest) = line.strip_prefix(keyword) else {
        return false;
    };
    rest.is_empty()
        || rest
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b';' | b'|' | b'&'))
}

pub(super) fn raw_line_ends_with_keyword(line: &str, keyword: &str) -> bool {
    let Some(prefix) = line.strip_suffix(keyword) else {
        return false;
    };
    prefix.is_empty()
        || prefix
            .as_bytes()
            .last()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b';' | b'|'))
}

pub(super) fn raw_line_closes_inline_brace_group_before_pipeline(content: &str) -> bool {
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

pub(super) fn raw_redirect_target_spacing_can_be_stripped(
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

pub(super) fn source_indent_units_before_offset(
    source: &str,
    offset: usize,
    options: &ResolvedShellFormatOptions,
) -> usize {
    let Some(indent) = line_indent_before_source_offset(source, offset) else {
        return 0;
    };
    raw_indent_units(indent, options)
}

pub(super) fn raw_indent_units(indent: &str, options: &ResolvedShellFormatOptions) -> usize {
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

pub(super) fn strip_one_indent_unit<'a>(
    line: &'a str,
    options: &ResolvedShellFormatOptions,
) -> &'a str {
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

pub(super) fn strip_outer_indent_or_one_unit<'a>(
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

pub(super) fn source_indent_minus_one_unit(
    indent: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
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

pub(super) fn source_indent_plus_one_unit(
    indent: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
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
pub(super) fn raw_word_source_slice<'a>(word: &Word, source: &'a str) -> Option<&'a str> {
    raw_source_slice(word.span, source)
}

pub(super) fn word_is_single_quoted_only(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [shuck_ast::WordPartNode {
            kind: WordPart::SingleQuoted { .. },
            ..
        }]
    )
}

pub(super) fn raw_pattern_source_slice<'a>(pattern: &Pattern, source: &'a str) -> Option<&'a str> {
    raw_source_slice(pattern.span, source)
}

pub(super) fn raw_source_slice(span: shuck_ast::Span, source: &str) -> Option<&str> {
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

pub(super) fn should_preserve_raw_syntax(raw: &str, rendered: &str) -> bool {
    raw != rendered && could_need_preserve_raw_syntax(raw)
}

pub(super) fn should_preserve_special_rendered_raw_syntax(raw: &str, rendered: &str) -> bool {
    raw != rendered
        && !raw.contains('\n')
        && !raw_command_substitution_needs_structural_spacing(raw)
        && could_need_preserve_raw_syntax_beyond_line_continuations(raw)
}

pub(super) fn could_need_preserve_raw_syntax(raw: &str) -> bool {
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

pub(super) fn could_need_preserve_raw_syntax_beyond_line_continuations(raw: &str) -> bool {
    raw.starts_with('\\')
        || raw.starts_with('&')
        || raw.starts_with("$'")
        || raw_contains_escaped_horizontal_whitespace(raw)
        || raw.contains("\\\"")
        || raw.contains("\\`")
        || raw.contains("[^ ]")
}

pub(super) fn raw_contains_escaped_horizontal_whitespace(raw: &str) -> bool {
    raw.contains("\\ ") || raw.contains("\\\t")
}

pub(super) fn raw_contains_double_backslash_outside_single_quotes(raw: &str) -> bool {
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
