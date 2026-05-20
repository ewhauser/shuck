use crate::comments::SourceMap;
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::scan::{
    branch_keyword_offset, close_suffix_comment_offsets, last_shell_keyword_start,
    last_uncommented_shell_keyword_before, leading_shell_indent, matching_done_close_start,
    matching_if_close_start, normalized_close_keyword_span, refine_common_indent,
    shell_comment_can_start, skip_double_quoted, skip_single_quoted,
};
use crate::word::{
    matching_raw_command_substitution_close, normalize_raw_pipeline_continuations,
    render_arithmetic_expr_to_buf, render_word_syntax_to_buf, render_word_syntax_with_facts_to_buf,
};
use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticExprNode, ArrayElem, Assignment, AssignmentValue,
    BackgroundOperator, BinaryCommand, BinaryOp, BuiltinCommand, CaseItem, CaseTerminator, Command,
    CompoundCommand, DeclClause, DeclOperand, ForSyntax, ForeachSyntax, FunctionDef, IfCommand,
    IfSyntax, Redirect, RedirectKind, RepeatSyntax, SimpleCommand, SourceText, Span, Stmt, StmtSeq,
    StmtTerminator, Subscript, VarRef, Word, WordPart,
};

pub(crate) fn array_elem_parts(element: &ArrayElem) -> (Option<&Subscript>, &Word, &'static str) {
    match element {
        ArrayElem::Sequential(word) => (None, word, ""),
        ArrayElem::Keyed { key, value } => (Some(key), value, "="),
        ArrayElem::KeyedAppend { key, value } => (Some(key), value, "+="),
    }
}

pub(crate) fn array_elem_value_word_mut(element: &mut ArrayElem) -> &mut Word {
    match element {
        ArrayElem::Sequential(word)
        | ArrayElem::Keyed { value: word, .. }
        | ArrayElem::KeyedAppend { value: word, .. } => word,
    }
}

pub(crate) fn format_arithmetic_command_source(raw: &str) -> String {
    raw.strip_prefix("((")
        .and_then(|body| body.strip_suffix("))"))
        .map(|body| {
            if body.contains('\n') {
                format_multiline_arithmetic_command_body(body)
            } else {
                format!("(({}))", format_arithmetic_for_init_source(body.trim()))
            }
        })
        .unwrap_or_else(|| raw.to_string())
}

pub(crate) fn format_arithmetic_for_init_source(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(',') {
        return raw.to_string();
    }
    let Some((lhs, op, rhs)) = split_simple_arithmetic_assignment(trimmed) else {
        return raw.to_string();
    };
    if lhs.is_empty() || rhs.is_empty() {
        return raw.to_string();
    }
    format!("{} {} {}", lhs.trim(), op, rhs.trim())
}

pub(crate) fn format_arithmetic_for_clause_source(
    raw: &str,
    ast: Option<&ArithmeticExprNode>,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }
    if let Some(ast) = ast {
        let mut rendered = String::new();
        render_arithmetic_expr_to_buf(&mut rendered, ast, source, options);
        rendered
    } else {
        format_arithmetic_for_init_source(raw)
    }
}

fn split_simple_arithmetic_assignment(raw: &str) -> Option<(&str, &str, &str)> {
    for op in [
        "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "=",
    ] {
        let Some(index) = raw.find(op) else {
            continue;
        };
        if byte_index_inside_braced_parameter(raw, index) {
            continue;
        }
        if op == "=" {
            let previous = raw[..index].chars().next_back();
            let next = raw[index + op.len()..].chars().next();
            if previous.is_some_and(|ch| matches!(ch, '!' | '<' | '>' | '=')) || next == Some('=') {
                continue;
            }
        }
        return Some((&raw[..index], op, &raw[index + op.len()..]));
    }
    None
}

fn byte_index_inside_braced_parameter(raw: &str, target: usize) -> bool {
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < raw.len() {
        if index >= target {
            return depth > 0;
        }
        let rest = &raw[index..];
        if rest.starts_with("${") {
            depth += 1;
            index += 2;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '}' && depth > 0 {
            depth -= 1;
        }
        index += ch.len_utf8();
    }
    false
}

fn format_multiline_arithmetic_command_body(body: &str) -> String {
    let mut lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return "(( ))".to_string();
    }

    let mut index = 1;
    while index < lines.len() {
        if let Some(rest) = lines[index].strip_prefix('+') {
            let rest = rest.trim_start().to_string();
            if let Some(previous) = lines.get_mut(index - 1) {
                previous.push_str(" +");
            }
            lines[index] = rest;
        }
        index += 1;
    }

    let mut rendered = String::from("((\\\n");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(line);
        if index + 1 < lines.len() {
            rendered.push_str(" \\");
        } else {
            rendered.push_str("))\n");
        }
    }
    rendered
}

pub(crate) fn render_assignment_with_facts_to_buf(
    assignment: &Assignment,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
    rendered: &mut String,
) {
    render_assignment_inner(
        assignment,
        source,
        options,
        Some(source_map),
        Some(facts),
        rendered,
    );
}

fn render_assignment_inner(
    assignment: &Assignment,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    rendered: &mut String,
) {
    let start = rendered.len();
    render_assignment_head_to_buf(assignment, source, rendered);
    match &assignment.value {
        AssignmentValue::Scalar(value) => {
            render_word_syntax_with_optional_facts_to_buf(
                value, source, options, source_map, facts, rendered,
            );
        }
        AssignmentValue::Compound(array) => {
            rendered.push('(');
            for (index, value) in array.elements.iter().enumerate() {
                if index > 0 {
                    rendered.push(' ');
                }
                render_array_elem_to_buf(value, source, options, source_map, facts, rendered);
            }
            rendered.push(')');
        }
    }
    trim_unescaped_trailing_whitespace_in_place(rendered, start);
}

fn render_array_elem_to_buf(
    element: &ArrayElem,
    source: &str,
    options: &crate::options::ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    rendered: &mut String,
) {
    let (key, value, op) = array_elem_parts(element);
    if let Some(key) = key {
        render_keyed_array_elem_to_buf(
            key, value, source, options, source_map, facts, op, rendered,
        );
    } else {
        render_word_syntax_with_optional_facts_to_buf(
            value, source, options, source_map, facts, rendered,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_keyed_array_elem_to_buf(
    key: &Subscript,
    value: &Word,
    source: &str,
    options: &crate::options::ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    operator: &str,
    rendered: &mut String,
) {
    rendered.push('[');
    render_subscript_to_buf(key, source, rendered);
    rendered.push(']');
    rendered.push_str(operator);
    render_word_syntax_with_optional_facts_to_buf(
        value, source, options, source_map, facts, rendered,
    );
}

pub(crate) fn render_assignment_head_to_buf(
    assignment: &Assignment,
    source: &str,
    rendered: &mut String,
) {
    rendered.push_str(assignment.target.name.as_str());
    if let Some(index) = &assignment.target.subscript {
        rendered.push('[');
        render_subscript_to_buf(index, source, rendered);
        rendered.push(']');
    }
    if assignment.append {
        rendered.push_str("+=");
    } else {
        rendered.push('=');
    }
}

fn render_word_syntax_with_optional_facts_to_buf(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
    source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
    rendered: &mut String,
) {
    match (source_map, facts) {
        (Some(source_map), Some(facts)) => {
            render_word_syntax_with_facts_to_buf(word, source, options, source_map, facts, rendered)
        }
        _ => render_word_syntax_to_buf(word, source, options, rendered),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultilineCompoundAssignmentLayout {
    pub(crate) lines: Vec<String>,
    pub(crate) open_inline: bool,
    pub(crate) close_inline: bool,
}

pub(crate) fn multiline_compound_assignment_layout(
    assignment: &Assignment,
    source: &str,
) -> Option<MultilineCompoundAssignmentLayout> {
    let AssignmentValue::Compound(_) = &assignment.value else {
        return None;
    };

    let slice = assignment.span.slice(source);
    if !slice.contains('\n') {
        return None;
    }

    let open = slice.find('(')?;
    let close = slice.rfind(')')?;
    if close <= open {
        return None;
    }

    let body = &slice[open + 1..close];
    let open_line = body.split_once('\n').map_or(body, |(line, _)| line);
    let close_line = body.rsplit_once('\n').map_or(body, |(_, line)| line);
    let mut raw_lines = body
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t', '\r']))
        .collect::<Vec<_>>();
    if open_line.trim().is_empty() && raw_lines.first().is_some_and(|line| line.is_empty()) {
        raw_lines.remove(0);
    }
    if close_line.trim().is_empty()
        && raw_lines.last().is_some_and(|line| line.is_empty())
        && !body.ends_with('\n')
    {
        raw_lines.pop();
    }
    let common_indent =
        multiline_compound_assignment_common_body_indent(&raw_lines, !open_line.trim().is_empty());
    let residual_space_indent_width = multiline_compound_assignment_residual_space_indent_width(
        &raw_lines,
        &common_indent,
        !open_line.trim().is_empty(),
    );
    let command_substitution_body_lines =
        multiline_compound_assignment_command_substitution_body_lines(&raw_lines);
    let mut lines = raw_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            normalize_multiline_compound_assignment_line(
                line,
                &common_indent,
                residual_space_indent_width,
                index == 0 && !open_line.trim().is_empty(),
                command_substitution_body_lines[index],
            )
        })
        .collect::<Vec<_>>();
    for line in &mut lines {
        *line = trim_inline_command_substitution_open_padding(line);
    }
    let lines = normalize_multiline_compound_assignment_command_substitutions(lines);

    (!lines.is_empty()).then_some(MultilineCompoundAssignmentLayout {
        lines,
        open_inline: !open_line.trim().is_empty(),
        close_inline: !close_line.trim().is_empty(),
    })
}

fn trim_inline_command_substitution_open_padding(line: &str) -> String {
    let mut rendered = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find("$(") {
        rendered.push_str(&rest[..open + 2]);
        rest = &rest[open + 2..];
        rest = rest.trim_start_matches([' ', '\t']);
    }
    rendered.push_str(rest);
    rendered
}

fn normalize_multiline_compound_assignment_command_substitutions(
    mut lines: Vec<String>,
) -> Vec<String> {
    let mut index = 0;
    while index < lines.len() {
        let Some((close_index, body_end, close_line_is_standalone)) =
            multiline_compound_assignment_command_substitution_range(&lines, index)
        else {
            index += 1;
            continue;
        };

        normalize_multiline_compound_assignment_command_substitution_pipeline_continuations(
            &mut lines,
            index,
            close_index + 1,
        );

        let body_prefix =
            multiline_compound_assignment_command_substitution_body_prefix(&lines[index]);
        if body_prefix.is_empty() {
            index = close_index + 1;
            continue;
        }

        let common_indent = common_command_substitution_body_indent(&lines[index + 1..body_end]);
        for line in &mut lines[index + 1..body_end] {
            if line.trim().is_empty() {
                continue;
            }
            let stripped = if common_indent.is_empty() {
                line.trim_start_matches([' ', '\t'])
            } else {
                line.strip_prefix(&common_indent)
                    .unwrap_or_else(|| line.trim_start_matches([' ', '\t']))
            };
            *line = format!("{body_prefix}{stripped}");
        }
        if close_line_is_standalone {
            lines[close_index] = lines[close_index]
                .trim_start_matches([' ', '\t'])
                .to_string();
        }
        index = close_index + 1;
    }
    lines
}

fn multiline_compound_assignment_command_substitution_range<T: AsRef<str>>(
    lines: &[T],
    open_index: usize,
) -> Option<(usize, usize, bool)> {
    if !line_has_unclosed_command_substitution_open(lines.get(open_index)?.as_ref()) {
        return None;
    }
    let close_index =
        multiline_compound_assignment_command_substitution_close_index(lines, open_index)?;
    let close_line_is_standalone = lines[close_index]
        .as_ref()
        .trim_start_matches([' ', '\t'])
        .starts_with(')');
    let body_end = if close_line_is_standalone {
        close_index
    } else {
        close_index + 1
    };
    Some((close_index, body_end, close_line_is_standalone))
}

fn multiline_compound_assignment_command_substitution_close_index<T: AsRef<str>>(
    lines: &[T],
    open_index: usize,
) -> Option<usize> {
    let mut tail = String::new();
    for (index, line) in lines.get(open_index..)?.iter().enumerate() {
        if index > 0 {
            tail.push('\n');
        }
        tail.push_str(line.as_ref());
    }
    let open = tail.find("$(")?;
    let close = matching_raw_command_substitution_close(&tail, open + 2)?;
    Some(open_index + tail[..close].bytes().filter(|byte| *byte == b'\n').count())
}

fn multiline_compound_assignment_command_substitution_body_lines(raw_lines: &[&str]) -> Vec<bool> {
    let mut body_lines = vec![false; raw_lines.len()];
    let mut index = 0;
    while index < raw_lines.len() {
        let Some((close_index, body_end, _)) =
            multiline_compound_assignment_command_substitution_range(raw_lines, index)
        else {
            index += 1;
            continue;
        };

        for body_line in &mut body_lines[index + 1..body_end] {
            *body_line = true;
        }
        index = close_index + 1;
    }
    body_lines
}

fn normalize_multiline_compound_assignment_command_substitution_pipeline_continuations(
    lines: &mut [String],
    body_start: usize,
    body_end: usize,
) {
    if body_start >= body_end {
        return;
    }

    let body = lines[body_start..body_end].join("\n");
    let Some(normalized) = normalize_raw_pipeline_continuations(&body) else {
        return;
    };
    let normalized_lines = normalized
        .lines()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if normalized_lines.len() != body_end - body_start {
        return;
    }

    for (line, normalized) in lines[body_start..body_end].iter_mut().zip(normalized_lines) {
        *line = normalized;
    }
}

pub(crate) fn multiline_compound_assignment_command_substitution_body_prefix(
    open_line: &str,
) -> &'static str {
    let Some(open) = open_line.find("$(") else {
        return "";
    };
    let prefix = open_line[..open].trim_start_matches([' ', '\t']);
    let inline_body = !open_line[open + 2..]
        .trim_matches([' ', '\t', '\r', '\\'])
        .is_empty();
    if prefix.is_empty() && inline_body {
        return "\t";
    }
    if prefix.is_empty() || prefix.contains("$(") || prefix.contains('(') || prefix.contains('=') {
        ""
    } else {
        "\t"
    }
}

pub(crate) fn line_has_unclosed_command_substitution_open(line: &str) -> bool {
    let Some(open) = line.find("$(") else {
        return false;
    };
    !line[open + 2..].contains(')')
}

fn common_command_substitution_body_indent(lines: &[String]) -> String {
    let mut common: Option<String> = None;
    for line in lines {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() {
            continue;
        }
        let indent = leading_shell_indent(line);
        if indent.is_empty() {
            return String::new();
        }
        if refine_common_indent(&mut common, indent) {
            return String::new();
        }
    }
    common.unwrap_or_default()
}

fn normalize_multiline_compound_assignment_line(
    line: &str,
    common_indent: &str,
    residual_space_indent_width: usize,
    open_inline_line: bool,
    preserve_line_continuation: bool,
) -> String {
    let trimmed_start = line.trim_start_matches([' ', '\t']);
    let trimmed = if preserve_line_continuation {
        trimmed_start.trim_end_matches([' ', '\t'])
    } else {
        trim_multiline_compound_assignment_line_continuation(trimmed_start)
    };
    if trimmed.is_empty() {
        return String::new();
    }
    if open_inline_line {
        return normalize_multiline_compound_assignment_spacing(trimmed);
    }
    if trimmed.starts_with(')') {
        return trimmed.to_string();
    }
    if trimmed.starts_with('[') {
        return normalize_multiline_compound_assignment_spacing(trimmed);
    }
    let stripped = line
        .strip_prefix(common_indent)
        .map(|line| {
            if preserve_line_continuation {
                line.trim_end_matches([' ', '\t'])
            } else {
                trim_multiline_compound_assignment_line_continuation(line)
            }
        })
        .unwrap_or(trimmed);
    let normalized =
        if preserve_line_continuation && line_starts_with_redirect_continuation(stripped) {
            format!("\t{}", stripped.trim_start_matches([' ', '\t']))
        } else if preserve_line_continuation {
            canonicalize_multiline_compound_assignment_residual_indent(
                stripped,
                residual_space_indent_width,
            )
        } else {
            stripped.trim_start_matches([' ', '\t']).to_string()
        };
    normalize_multiline_compound_assignment_spacing(&normalized)
}

fn line_starts_with_redirect_continuation(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let bytes = trimmed.as_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'<' | b'>') => true,
        Some(b'&') => bytes.get(index + 1) == Some(&b'>'),
        _ => false,
    }
}

fn trim_multiline_compound_assignment_line_continuation(line: &str) -> &str {
    let trimmed = line.trim_end_matches([' ', '\t']);
    let trailing_backslashes = trimmed
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    if trailing_backslashes % 2 == 1 && !line_continuation_is_inside_unclosed_substitution(trimmed)
    {
        trimmed[..trimmed.len().saturating_sub(1)].trim_end_matches([' ', '\t'])
    } else {
        trimmed
    }
}

fn line_continuation_is_inside_unclosed_substitution(line: &str) -> bool {
    let Some(before_continuation) = line.strip_suffix('\\') else {
        return false;
    };

    let mut depth = 0usize;
    let mut chars = before_continuation.chars().peekable();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            chars.next();
            continue;
        }
        if ch == '\'' && !in_double_quotes {
            in_single_quotes = !in_single_quotes;
            continue;
        }
        if ch == '"' && !in_single_quotes {
            in_double_quotes = !in_double_quotes;
            continue;
        }
        if in_single_quotes {
            continue;
        }
        match ch {
            '$' | '<' | '>' if chars.peek().is_some_and(|next| *next == '(') => {
                chars.next();
                depth += 1;
            }
            ')' if depth > 0 => depth -= 1,
            _ => {}
        }
    }

    depth > 0
}

fn multiline_compound_assignment_common_body_indent(lines: &[&str], open_inline: bool) -> String {
    let mut common: Option<String> = None;
    for (index, line) in lines.iter().enumerate() {
        if index == 0 && open_inline {
            continue;
        }
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(')') || trimmed.starts_with('#') {
            continue;
        }
        let indent = leading_shell_indent(line);
        if indent.is_empty() {
            continue;
        }
        if refine_common_indent(&mut common, indent) {
            return String::new();
        }
    }
    common.unwrap_or_default()
}

fn multiline_compound_assignment_residual_space_indent_width(
    lines: &[&str],
    common_indent: &str,
    open_inline: bool,
) -> usize {
    let mut width = None::<usize>;
    for (index, line) in lines.iter().enumerate() {
        if index == 0 && open_inline {
            continue;
        }
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(')') || trimmed.starts_with('#') {
            continue;
        }
        let stripped = line.strip_prefix(common_indent).unwrap_or(trimmed);
        let indent = leading_shell_indent(stripped);
        if indent.is_empty() || indent.contains('\t') {
            continue;
        }
        width = Some(width.map_or(indent.len(), |current| current.min(indent.len())));
    }
    width.unwrap_or(1).max(1)
}

fn canonicalize_multiline_compound_assignment_residual_indent(
    line: &str,
    residual_space_indent_width: usize,
) -> String {
    let indent = leading_shell_indent(line);
    if indent.is_empty() {
        return line.to_string();
    }
    let body = &line[indent.len()..];
    let mut indent_units = 0usize;
    let mut pending_spaces = 0usize;
    for ch in indent.chars() {
        if ch == '\t' {
            indent_units += 1;
            pending_spaces = 0;
        } else {
            pending_spaces += 1;
            if pending_spaces == residual_space_indent_width {
                indent_units += 1;
                pending_spaces = 0;
            }
        }
    }
    if pending_spaces > 0 {
        indent_units += 1;
    }

    let mut rendered = "\t".repeat(indent_units);
    rendered.push_str(body);
    rendered
}

fn normalize_multiline_compound_assignment_spacing(line: &str) -> String {
    let indent = leading_shell_indent(line);
    let body = &line[indent.len()..];
    if body.is_empty() || body.starts_with('#') {
        return line.to_string();
    }

    let mut rendered = String::with_capacity(line.len());
    rendered.push_str(indent);
    let mut chars = body.chars().peekable();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;
    let mut changed = false;
    let mut previous_was_space = false;

    while let Some(ch) = chars.next() {
        if escaped {
            rendered.push(ch);
            escaped = false;
            previous_was_space = false;
            continue;
        }
        if ch == '\\' && !in_single_quotes {
            rendered.push(ch);
            escaped = true;
            previous_was_space = false;
            continue;
        }
        if ch == '\'' && !in_double_quotes {
            in_single_quotes = !in_single_quotes;
            rendered.push(ch);
            previous_was_space = false;
            continue;
        }
        if ch == '"' && !in_single_quotes {
            in_double_quotes = !in_double_quotes;
            rendered.push(ch);
            previous_was_space = false;
            continue;
        }
        if !in_single_quotes && !in_double_quotes && matches!(ch, ' ' | '\t') {
            while chars.peek().is_some_and(|next| matches!(next, ' ' | '\t')) {
                chars.next();
                changed = true;
            }
            if !previous_was_space && chars.peek().is_some() {
                rendered.push(' ');
                previous_was_space = true;
            } else {
                changed = true;
            }
            continue;
        }
        rendered.push(ch);
        previous_was_space = false;
    }

    if changed { rendered } else { line.to_string() }
}

pub(crate) fn multiline_compound_assignment_lines(
    assignment: &Assignment,
    source: &str,
) -> Option<Vec<String>> {
    multiline_compound_assignment_layout(assignment, source).map(|layout| layout.lines)
}

pub(crate) fn render_var_ref_to_buf(reference: &VarRef, source: &str, rendered: &mut String) {
    rendered.push_str(reference.name.as_str());
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        render_subscript_to_buf(subscript, source, rendered);
        rendered.push(']');
    }
}

pub(crate) fn render_subscript_to_buf(subscript: &Subscript, source: &str, rendered: &mut String) {
    if let Some(selector) = subscript.selector() {
        rendered.push(selector.as_char());
        return;
    }

    render_source_text_to_buf(subscript.syntax_source_text(), source, rendered);
}

pub(crate) fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
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

pub(crate) fn render_source_text_to_buf(text: &SourceText, source: &str, rendered: &mut String) {
    if !text.is_source_backed() || text.span().end.offset <= source.len() {
        rendered.push_str(text.slice(source));
    }
}

fn trim_unescaped_trailing_whitespace_in_place(text: &mut String, start: usize) {
    let end = start + trim_unescaped_trailing_whitespace(&text[start..]).len();
    text.truncate(end);
}

pub(crate) fn has_heredoc(stmt: &Stmt) -> bool {
    stmt.redirects.iter().any(is_heredoc)
        || match &stmt.command {
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => false,
            Command::Binary(command) => has_heredoc(&command.left) || has_heredoc(&command.right),
            Command::Compound(command) => compound_has_heredoc(command),
            Command::Function(function) => has_heredoc(&function.body),
            Command::AnonymousFunction(function) => has_heredoc(&function.body),
        }
}

fn compound_has_heredoc(command: &CompoundCommand) -> bool {
    compound_contains_child(command, has_heredoc, stmt_seq_has_heredoc)
}

pub(crate) fn compound_contains_child(
    command: &CompoundCommand,
    mut stmt_predicate: impl FnMut(&Stmt) -> bool,
    mut seq_predicate: impl FnMut(&StmtSeq) -> bool,
) -> bool {
    let mut found = false;
    for_each_compound_child(command, |child| {
        if found {
            return;
        }
        found = match child {
            CompoundChild::Stmt(stmt) => stmt_predicate(stmt),
            CompoundChild::Sequence(sequence) => seq_predicate(sequence),
        };
    });
    found
}

enum CompoundChild<'a> {
    Stmt(&'a Stmt),
    Sequence(&'a StmtSeq),
}

fn for_each_compound_child(command: &CompoundCommand, mut visitor: impl FnMut(CompoundChild<'_>)) {
    match command {
        CompoundCommand::If(command) => {
            visitor(CompoundChild::Sequence(&command.condition));
            visitor(CompoundChild::Sequence(&command.then_branch));
            for (condition, body) in &command.elif_branches {
                visitor(CompoundChild::Sequence(condition));
                visitor(CompoundChild::Sequence(body));
            }
            if let Some(body) = &command.else_branch {
                visitor(CompoundChild::Sequence(body));
            }
        }
        CompoundCommand::For(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::Repeat(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::Foreach(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::ArithmeticFor(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::While(command) => {
            visitor(CompoundChild::Sequence(&command.condition));
            visitor(CompoundChild::Sequence(&command.body));
        }
        CompoundCommand::Until(command) => {
            visitor(CompoundChild::Sequence(&command.condition));
            visitor(CompoundChild::Sequence(&command.body));
        }
        CompoundCommand::Case(command) => {
            for item in &command.cases {
                visitor(CompoundChild::Sequence(&item.body));
            }
        }
        CompoundCommand::Select(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            visitor(CompoundChild::Sequence(commands));
        }
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = command.command.as_deref() {
                visitor(CompoundChild::Stmt(command));
            }
        }
        CompoundCommand::Coproc(command) => visitor(CompoundChild::Stmt(&command.body)),
        CompoundCommand::Always(command) => {
            visitor(CompoundChild::Sequence(&command.body));
            visitor(CompoundChild::Sequence(&command.always_body));
        }
    }
}

pub(crate) fn stmt_seq_has_heredoc(commands: &StmtSeq) -> bool {
    commands.iter().any(has_heredoc)
}

fn is_heredoc(redirect: &shuck_ast::Redirect) -> bool {
    matches!(
        redirect.kind,
        RedirectKind::HereDoc | RedirectKind::HereDocStrip
    )
}

pub(crate) fn stmt_verbatim_span_with_source_map(stmt: &Stmt, source_map: &SourceMap<'_>) -> Span {
    stmt_verbatim_span_impl(stmt, source_map.source(), Some(source_map))
}

fn stmt_verbatim_span_impl(stmt: &Stmt, source: &str, source_map: Option<&SourceMap<'_>>) -> Span {
    let command_span = if let Command::Simple(command) = &stmt.command
        && simple_command_uses_synthetic_words(command, source)
    {
        synthetic_simple_command_verbatim_span(command, source, source_map)
    } else {
        command_verbatim_span(&stmt.command, source, source_map)
    };
    let mut span = merge_redirect_heredoc_spans(command_span, &stmt.redirects, source);
    if stmt.negated {
        span = merge_non_empty_span(stmt.span, span);
    }
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    if span == Span::new() {
        stmt_span(stmt)
    } else {
        span
    }
}

fn command_verbatim_span(
    command: &Command,
    source: &str,
    source_map: Option<&SourceMap<'_>>,
) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => builtin_like_parts(command).0,
        Command::Decl(command) => command.span,
        Command::Binary(command) => stmt_verbatim_span_impl(&command.left, source, source_map)
            .merge(stmt_verbatim_span_impl(&command.right, source, source_map)),
        Command::Compound(command) => compound_verbatim_span(command, source, source_map),
        Command::Function(command) => function_header_span(command).merge(
            function_body_verbatim_span(&command.body, source, source_map),
        ),
        Command::AnonymousFunction(command) => anonymous_function_header_span(command)
            .merge(function_body_verbatim_span(
                &command.body,
                source,
                source_map,
            ))
            .merge(words_span(&command.args)),
    }
}

fn function_body_verbatim_span(
    body: &Stmt,
    source: &str,
    source_map: Option<&SourceMap<'_>>,
) -> Span {
    let mut span = stmt_verbatim_span_impl(body, source, source_map);
    let Some(source_map) = source_map else {
        return span;
    };
    if let Some(group_span) = command_group_attachment_span(&body.command, source_map) {
        span = merge_non_empty_span(span, group_span);
    }
    span
}

pub(crate) fn command_group_commands(command: &Command) -> Option<(&StmtSeq, char)> {
    match command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => Some((commands, '{')),
        Command::Compound(CompoundCommand::Subshell(commands)) => Some((commands, '(')),
        _ => None,
    }
}

pub(crate) fn branch_open_keyword_start(
    sequence: &StmtSeq,
    source: &str,
    keyword: &str,
) -> Option<usize> {
    let first = sequence.first()?;
    last_uncommented_shell_keyword_before(source, stmt_span(first).start.offset, keyword)
}

pub(crate) fn if_next_branch_region_with_body_end(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    mut branch_body_end: impl FnMut(&StmtSeq) -> usize,
) -> Option<(usize, usize)> {
    let current_branch_end = if branch_index == 0 {
        branch_body_end(&command.then_branch)
    } else {
        command
            .elif_branches
            .get(branch_index - 1)
            .map(|(_, body)| branch_body_end(body))
            .unwrap_or_else(|| branch_body_end(&command.then_branch))
    };

    if let Some((condition, _)) = command.elif_branches.get(branch_index) {
        let keyword = branch_keyword_offset(
            source,
            current_branch_end,
            condition.span.start.offset,
            "elif",
        )
        .unwrap_or(condition.span.start.offset);
        Some((current_branch_end, keyword))
    } else if branch_index == command.elif_branches.len() {
        command.else_branch.as_ref().map(|body| {
            let keyword =
                branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
                    .unwrap_or(body.span.start.offset);
            (current_branch_end, keyword)
        })
    } else {
        None
    }
}

pub(crate) fn collect_pipeline_parts<'a, T>(
    command: &'a BinaryCommand,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<T>,
    operator_for: &impl Fn(&BinaryCommand) -> T,
) {
    collect_pipeline_stmt_parts(command.left.as_ref(), statements, operators, operator_for);
    operators.push(operator_for(command));
    collect_pipeline_stmt_parts(command.right.as_ref(), statements, operators, operator_for);
}

fn collect_pipeline_stmt_parts<'a, T>(
    stmt: &'a Stmt,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<T>,
    operator_for: &impl Fn(&BinaryCommand) -> T,
) {
    if let Some(binary) = stmt_plain_pipeline_binary(stmt) {
        collect_pipeline_parts(binary, statements, operators, operator_for);
    } else {
        statements.push(stmt);
    }
}

fn stmt_plain_pipeline_binary(stmt: &Stmt) -> Option<&BinaryCommand> {
    if let Command::Binary(binary) = &stmt.command
        && stmt.redirects.is_empty()
        && !stmt.negated
        && stmt.terminator.is_none()
        && matches!(
            binary.op,
            shuck_ast::BinaryOp::Pipe | shuck_ast::BinaryOp::PipeAll
        )
    {
        Some(binary)
    } else {
        None
    }
}

pub(crate) fn collect_binary_list_first<'a, T>(
    command: &'a BinaryCommand,
    rest: &mut Vec<T>,
    item_for: &impl Fn(&'a BinaryCommand) -> T,
) -> &'a Stmt {
    if let Command::Binary(left_binary) = &command.left.command
        && command.left.redirects.is_empty()
        && !command.left.negated
        && command.left.terminator.is_none()
        && matches!(left_binary.op, BinaryOp::And | BinaryOp::Or)
    {
        let first = collect_binary_list_first(left_binary, rest, item_for);
        rest.push(item_for(command));
        return first;
    }

    let first = command.left.as_ref();
    rest.push(item_for(command));
    first
}

fn command_group_attachment_span(
    command: &Command,
    source_map: &crate::comments::SourceMap<'_>,
) -> Option<Span> {
    let (commands, open) = command_group_commands(command)?;
    group_attachment_span(
        commands.as_slice(),
        source_map,
        open,
        matching_group_close(open),
    )
}

fn stmt_group_attachment_or_verbatim_span(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> Option<Span> {
    let (commands, open) = command_group_commands(&stmt.command)?;
    Some(
        group_attachment_span(
            commands.as_slice(),
            source_map,
            open,
            matching_group_close(open),
        )
        .unwrap_or_else(|| stmt_verbatim_span_with_source_map(stmt, source_map)),
    )
}

fn stmt_group_base_span(
    stmt: &Stmt,
    commands: &StmtSeq,
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
) -> Span {
    group_attachment_span(
        commands.as_slice(),
        source_map,
        open,
        matching_group_close(open),
    )
    .unwrap_or_else(|| stmt_span(stmt))
}

fn merge_redirect_heredoc_spans(mut span: Span, redirects: &[Redirect], source: &str) -> Span {
    for redirect in redirects {
        span = merge_non_empty_span(span, redirect.span);
        if let Some(heredoc) = redirect.heredoc() {
            span = span.merge(extend_heredoc_body_span(heredoc.body.span, source));
        }
    }
    span
}

pub(crate) fn stmt_span(stmt: &Stmt) -> Span {
    let mut span = stmt.span;
    for redirect in &stmt.redirects {
        span = merge_non_empty_span(span, redirect.span);
    }
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    span
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(stmt_span)
            .reduce(Span::merge)
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}

fn compound_verbatim_span(
    command: &CompoundCommand,
    source: &str,
    source_map: Option<&SourceMap<'_>>,
) -> Span {
    match command {
        CompoundCommand::Subshell(commands) => {
            group_verbatim_span_impl(commands.as_slice(), source, source_map, '(', ')')
        }
        CompoundCommand::BraceGroup(commands) => {
            group_verbatim_span_impl(commands.as_slice(), source, source_map, '{', '}')
        }
        _ => compound_verbatim_span_from_children(command, source, source_map),
    }
}

fn compound_verbatim_span_from_children(
    command: &CompoundCommand,
    source: &str,
    source_map: Option<&SourceMap<'_>>,
) -> Span {
    let mut span = compound_span(command);
    for_each_compound_child(command, |child| {
        span = match child {
            CompoundChild::Stmt(stmt) => {
                span.merge(stmt_verbatim_span_impl(stmt, source, source_map))
            }
            CompoundChild::Sequence(sequence) => {
                merge_stmt_sequence_verbatim_span(span, sequence, source, source_map)
            }
        };
    });
    span
}

fn merge_stmt_sequence_verbatim_span(
    mut span: Span,
    commands: &StmtSeq,
    source: &str,
    source_map: Option<&SourceMap<'_>>,
) -> Span {
    for command in commands.iter() {
        span = merge_non_empty_span(span, stmt_verbatim_span_impl(command, source, source_map));
    }
    span
}

#[cfg(test)]
fn group_verbatim_span(commands: &[Stmt], source: &str, open: char, close: char) -> Span {
    group_verbatim_span_impl(commands, source, None, open, close)
}

fn group_verbatim_span_impl(
    commands: &[Stmt],
    source: &str,
    source_map: Option<&SourceMap<'_>>,
    open: char,
    close: char,
) -> Span {
    let inner = commands
        .iter()
        .map(|command| stmt_verbatim_span_impl(command, source, source_map))
        .reduce(Span::merge)
        .unwrap_or_default();
    if inner == Span::new() {
        return inner;
    }

    let Some(open_offset) = source[..inner.start.offset].rfind(open) else {
        return inner;
    };
    let wrapper_prefix_start = open_offset + open.len_utf8();
    if !source[wrapper_prefix_start..inner.start.offset].contains('#') {
        return inner;
    }
    let Some(close_offset) = find_group_close_offset(source, inner.end.offset, close) else {
        return inner;
    };

    span_for_offsets(
        source,
        source_map,
        open_offset,
        close_offset + close.len_utf8(),
    )
}

pub(crate) fn group_open_suffix<'a>(
    commands: &[Stmt],
    source_map: &'a crate::comments::SourceMap<'a>,
    open: char,
) -> Option<(Span, &'a str)> {
    let source = source_map.source();
    let first = commands.first()?;
    let first_start = stmt_group_attachment_start_offset(first, source_map);
    let open_offset = find_group_open_offset_before_stmt(source, first_start, open)?;
    let line_end = source[open_offset..]
        .find('\n')
        .map(|offset| open_offset + offset)
        .unwrap_or(source.len());
    let suffix_start = open_offset + open.len_utf8();
    let suffix = source.get(suffix_start..line_end)?;
    suffix
        .trim_start_matches(char::is_whitespace)
        .starts_with('#')
        .then(|| (source_map.span_for_offsets(suffix_start, line_end), suffix))
}

pub(crate) fn group_attachment_span(
    commands: &[Stmt],
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
) -> Option<Span> {
    let source = source_map.source();
    let first = commands.first()?;
    let open_offset = find_group_open_offset_before_stmt(
        source,
        stmt_group_attachment_start_offset(first, source_map),
        open,
    )?;
    let sequence_end = commands
        .iter()
        .map(|command| stmt_group_attachment_end_offset(command, source_map))
        .max()
        .unwrap_or(0);
    let end = find_group_close_offset(source, sequence_end, close)
        .map(|offset| offset + close.len_utf8())
        .unwrap_or(sequence_end);
    Some(source_map.span_for_offsets(open_offset, end))
}

pub(crate) fn stmt_start_after_operator(
    stmt: &Stmt,
    operator_end: usize,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            group_open_offset_after_operator(
                stmt,
                commands.as_slice(),
                operator_end,
                source,
                source_map,
                '{',
                '}',
            )
        }
        Command::Compound(CompoundCommand::Subshell(commands)) => group_open_offset_after_operator(
            stmt,
            commands.as_slice(),
            operator_end,
            source,
            source_map,
            '(',
            ')',
        ),
        _ => command_format_span(&stmt.command).start.offset,
    }
}

fn group_open_offset_after_operator(
    stmt: &Stmt,
    commands: &[Stmt],
    operator_end: usize,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
) -> usize {
    let search_end = commands
        .first()
        .map(|first| stmt_group_attachment_start_offset(first, source_map))
        .unwrap_or_else(|| stmt_span(stmt).end.offset);

    find_group_open_offset_between(source, operator_end, search_end, open)
        .or_else(|| {
            group_attachment_span(commands, source_map, open, close).map(|span| span.start.offset)
        })
        .unwrap_or_else(|| command_format_span(&stmt.command).start.offset)
}

fn find_group_open_offset_between(
    source: &str,
    search_start: usize,
    search_end: usize,
    open: char,
) -> Option<usize> {
    let mut offset = search_start.min(source.len());
    let upper = search_end.min(source.len());

    while offset < upper {
        let tail = &source[offset..upper];
        let ch = tail.chars().next()?;
        match ch {
            '\\' => {
                offset += ch.len_utf8();
                if let Some(escaped) = source[offset..upper].chars().next() {
                    offset += escaped.len_utf8();
                }
                continue;
            }
            '\'' => {
                offset = skip_single_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '"' => {
                offset = skip_double_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '#' if shell_comment_can_start(source, offset) => {
                offset = tail
                    .find('\n')
                    .map_or(upper, |newline| offset + newline + 1);
                continue;
            }
            _ => {}
        }

        if ch == open {
            return Some(offset);
        }

        offset += ch.len_utf8();
    }

    None
}

fn find_group_open_offset_before_stmt(
    source: &str,
    search_end: usize,
    open: char,
) -> Option<usize> {
    let mut line_end = search_end.min(source.len());

    loop {
        let line_start = source[..line_end]
            .rfind('\n')
            .map(|offset| offset + 1)
            .unwrap_or(0);
        if let Some(open_offset) =
            find_group_open_offset_on_line(source, line_start, line_end, open)
        {
            return Some(open_offset);
        }

        if line_start == 0 {
            break;
        }
        line_end = line_start - 1;
    }

    None
}

fn find_group_open_offset_on_line(
    source: &str,
    line_start: usize,
    line_end: usize,
    open: char,
) -> Option<usize> {
    let mut last_open = None;
    let mut offset = line_start;

    while offset < line_end {
        let ch = source[offset..].chars().next()?;

        match ch {
            '\\' => {
                offset += ch.len_utf8();
                if let Some(escaped) = source[offset..line_end].chars().next() {
                    offset += escaped.len_utf8();
                }
                continue;
            }
            '\'' => {
                offset = skip_single_quoted(source, offset + ch.len_utf8(), line_end);
                continue;
            }
            '"' => {
                offset = skip_double_quoted(source, offset + ch.len_utf8(), line_end);
                continue;
            }
            '#' if shell_comment_can_start(source, offset) => break,
            _ => {}
        }

        if ch == open {
            last_open = Some(offset);
        }
        offset += ch.len_utf8();
    }

    last_open
}

fn stmt_group_attachment_start_offset(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    stmt_group_attachment_or_verbatim_span(stmt, source_map)
        .unwrap_or_else(|| stmt_verbatim_span_with_source_map(stmt, source_map))
        .start
        .offset
}

fn stmt_group_attachment_end_offset(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    if let Some(span) = stmt_group_attachment_or_verbatim_span(stmt, source_map) {
        return span.end.offset;
    }

    match &stmt.command {
        Command::Function(_) | Command::AnonymousFunction(_) => stmt_span(stmt).end.offset,
        _ if has_heredoc(stmt) => {
            stmt_verbatim_span_with_source_map(stmt, source_map)
                .end
                .offset
        }
        _ => stmt_span(stmt).end.offset,
    }
}

fn find_group_close_offset(source: &str, sequence_end: usize, close: char) -> Option<usize> {
    let close_len = close.len_utf8();
    let capped_end = sequence_end.min(source.len());
    if let Some(offset) = find_group_close_offset_after_sequence(source, capped_end, close) {
        return Some(offset);
    }

    let trimmed_end = source[..capped_end]
        .trim_end_matches(char::is_whitespace)
        .len();
    if trimmed_end >= close_len
        && source
            .get(trimmed_end - close_len..trimmed_end)
            .is_some_and(|slice| slice.starts_with(close))
    {
        return Some(trimmed_end - close_len);
    }

    None
}

fn find_group_close_offset_after_sequence(
    source: &str,
    sequence_end: usize,
    close: char,
) -> Option<usize> {
    let mut offset = sequence_end.min(source.len());
    while offset < source.len() {
        let tail = &source[offset..];
        if tail.starts_with("\\\n") {
            offset += "\\\n".len();
            continue;
        }
        let ch = tail.chars().next()?;
        if ch.is_whitespace() {
            offset += ch.len_utf8();
            continue;
        }
        if ch == ';' {
            offset += ch.len_utf8();
            continue;
        }
        if ch == '#' {
            offset = tail
                .find('\n')
                .map(|newline| offset + newline + 1)
                .unwrap_or(source.len());
            continue;
        }
        return (ch == close).then_some(offset);
    }

    None
}

pub(crate) fn group_was_inline_in_source(
    commands: &[Stmt],
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
) -> bool {
    group_attachment_span(commands, source_map, open, close)
        .map(|span| !span.slice(source_map.source()).contains('\n'))
        .unwrap_or(false)
}

pub(crate) fn command_format_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => simple_command_format_span(command),
        Command::Builtin(command) => {
            let (span, name, assignments, primary, extra_args) = builtin_like_parts(command);
            builtin_like_span(span.start, name, assignments, primary, extra_args)
        }
        Command::Decl(command) => decl_clause_format_span(command),
        Command::Binary(command) => {
            stmt_format_span(&command.left).merge(stmt_format_span(&command.right))
        }
        Command::Compound(command) => compound_format_span(command),
        Command::Function(command) => function_attachment_span(command),
        Command::AnonymousFunction(command) => anonymous_function_attachment_span(command),
    }
}

fn simple_command_format_span(command: &SimpleCommand) -> Span {
    let mut span = Span::new();
    for assignment in &command.assignments {
        span = merge_non_empty_span(span, assignment.span);
    }
    if !command.name.parts.is_empty() {
        span = merge_non_empty_span(span, command.name.span);
    }
    for argument in &command.args {
        span = merge_non_empty_span(span, argument.span);
    }
    if span == Span::new() {
        command.span
    } else {
        span
    }
}

fn builtin_like_span(
    start: shuck_ast::Position,
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
) -> Span {
    let mut span = Span::from_positions(start, start.advanced_by(name));
    for assignment in assignments {
        span = merge_non_empty_span(span, assignment.span);
    }
    if let Some(primary) = primary {
        span = merge_non_empty_span(span, primary.span);
    }
    for argument in extra_args {
        span = merge_non_empty_span(span, argument.span);
    }
    span
}

pub(crate) fn builtin_like_parts(
    command: &BuiltinCommand,
) -> (
    Span,
    &'static str,
    &[Assignment],
    Option<&shuck_ast::Word>,
    &[shuck_ast::Word],
) {
    match command {
        BuiltinCommand::Break(command) => (
            command.span,
            "break",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
        ),
        BuiltinCommand::Continue(command) => (
            command.span,
            "continue",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
        ),
        BuiltinCommand::Return(command) => (
            command.span,
            "return",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
        ),
        BuiltinCommand::Exit(command) => (
            command.span,
            "exit",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
        ),
    }
}

fn decl_clause_format_span(command: &DeclClause) -> Span {
    let mut span = command.variant_span;
    for assignment in &command.assignments {
        span = merge_non_empty_span(span, assignment.span);
    }
    for operand in &command.operands {
        let operand_span = match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
            DeclOperand::Name(name) => name.span,
            DeclOperand::Assignment(assignment) => assignment.span,
        };
        span = merge_non_empty_span(span, operand_span);
    }
    span
}

pub(crate) fn function_attachment_span(command: &FunctionDef) -> Span {
    function_header_span(command).merge(stmt_span(&command.body))
}

pub(crate) fn anonymous_function_attachment_span(command: &AnonymousFunctionCommand) -> Span {
    anonymous_function_header_span(command)
        .merge(stmt_span(&command.body))
        .merge(words_span(&command.args))
}

pub(crate) fn function_header_span(command: &FunctionDef) -> Span {
    command.header.span()
}

pub(crate) fn anonymous_function_header_span(command: &AnonymousFunctionCommand) -> Span {
    match command.surface {
        shuck_ast::AnonymousFunctionSurface::FunctionKeyword {
            function_keyword_span,
        } => function_keyword_span,
        shuck_ast::AnonymousFunctionSurface::Parens { parens_span } => parens_span,
    }
}

pub(crate) fn words_span(words: &[shuck_ast::Word]) -> Span {
    words.iter().fold(Span::new(), |span, word| {
        merge_non_empty_span(span, word.span)
    })
}

pub(crate) fn compound_format_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(stmt_format_span)
            .reduce(Span::merge)
            .unwrap_or_default(),
        _ => compound_span(command),
    }
}

pub(crate) fn stmt_format_span(stmt: &Stmt) -> Span {
    let mut span = if stmt.negated {
        stmt.span
    } else {
        command_format_span(&stmt.command)
    };
    for redirect in &stmt.redirects {
        span = merge_non_empty_span(span, redirect.span);
    }
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    if span == Span::new() {
        stmt_span(stmt)
    } else {
        span
    }
}

pub(crate) fn merge_non_empty_span(current: Span, next: Span) -> Span {
    if current == Span::new() {
        next
    } else if next == Span::new() {
        current
    } else {
        current.merge(next)
    }
}

fn span_for_offsets(
    source: &str,
    source_map: Option<&SourceMap<'_>>,
    start: usize,
    end: usize,
) -> Span {
    if let Some(source_map) = source_map {
        source_map.span_for_offsets(start, end)
    } else {
        crate::comments::SourceMap::new(source).span_for_offsets(start, end)
    }
}

pub(crate) fn line_gap_break_count(current_line: usize, next_line: usize) -> usize {
    next_line.saturating_sub(current_line).clamp(1, 2)
}

pub(crate) fn rendered_stmt_end_line(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    match &stmt.command {
        Command::Function(_) | Command::AnonymousFunction(_) => {
            span_render_end_line(stmt_span(stmt), source, source_map)
        }
        _ if has_heredoc(stmt) => span_render_end_line(
            stmt_verbatim_span_with_source_map(stmt, source_map),
            source,
            source_map,
        ),
        Command::Binary(command) => rendered_stmt_end_line(&command.right, source, source_map),
        _ => {
            if let Some((commands, open)) = command_group_commands(&stmt.command) {
                let mut span = stmt_group_base_span(stmt, commands, source_map, open);
                for redirect in &stmt.redirects {
                    span = merge_non_empty_span(span, redirect.span);
                }
                if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
                    && let Some(terminator_span) = stmt.terminator_span
                {
                    span = merge_non_empty_span(span, terminator_span);
                }
                span_render_end_line(span, source, source_map)
            } else {
                span_render_end_line(stmt_format_span(stmt), source, source_map)
            }
        }
    }
}

pub(crate) fn span_render_end_line(
    span: Span,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    let mut end = span.end.offset.min(source.len());
    while end > span.start.offset
        && source
            .as_bytes()
            .get(end - 1)
            .is_some_and(u8::is_ascii_whitespace)
    {
        end -= 1;
    }

    if end == span.start.offset {
        span.start.line
    } else {
        source_map.line_number_for_offset(end - 1)
    }
}

pub(crate) fn stmt_has_trailing_comment(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let raw = stmt_span(stmt);
    let formatted = stmt_format_span(stmt);
    raw.end.offset > formatted.end.offset
        && source_map.contains_comment_between(formatted.end.offset, raw.end.offset)
}

pub(crate) fn should_render_verbatim(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> bool {
    (!options.simplify()
        && matches!(&stmt.command, Command::Simple(command) if simple_command_uses_synthetic_words(command, source_map.source())))
        || (options.keep_padding() && stmt_has_alignment_sensitive_padding(stmt, source_map))
        || (has_heredoc(stmt)
            && !matches!(stmt.command, Command::Binary(_))
            && stmt_has_trailing_comment(stmt, source_map))
}

pub(crate) fn simple_command_uses_synthetic_words(command: &SimpleCommand, source: &str) -> bool {
    word_uses_synthetic_source(&command.name, source)
}

fn synthetic_simple_command_verbatim_span(
    command: &SimpleCommand,
    source: &str,
    source_map: Option<&SourceMap<'_>>,
) -> Span {
    let start = command.span.start.offset.min(source.len());
    let end = command.span.end.offset.min(source.len());
    let Some(raw) = source.get(start..end) else {
        return command.span;
    };
    let leading_padding = raw.len() - raw.trim_start_matches([' ', '\t']).len();
    let candidate = &raw[leading_padding..];
    let command_start = if let Some(operator_len) = candidate
        .starts_with("&&")
        .then_some(2)
        .or_else(|| candidate.starts_with("||").then_some(2))
    {
        let after_operator = leading_padding + operator_len;
        let rest = &raw[after_operator..];
        let operator_padding = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        start + after_operator + operator_padding
    } else {
        start
    };
    let command_end =
        trim_synthetic_simple_command_trailing_case_terminator(source, command_start, end);
    span_for_offsets(source, source_map, command_start, command_end)
}

fn trim_synthetic_simple_command_trailing_case_terminator(
    source: &str,
    start: usize,
    end: usize,
) -> usize {
    let Some(raw) = source.get(start..end) else {
        return end;
    };
    let trimmed_end = raw.trim_end_matches([' ', '\t', '\r', '\n']).len();
    let trimmed = &raw[..trimmed_end];
    let terminator_len = [";;&", ";;", ";&", ";|"]
        .into_iter()
        .find_map(|terminator| trimmed.ends_with(terminator).then_some(terminator.len()));
    let Some(terminator_len) = terminator_len else {
        return start + trimmed_end;
    };
    let before_terminator = trimmed[..trimmed.len() - terminator_len]
        .trim_end_matches([' ', '\t'])
        .len();
    start + before_terminator
}

fn word_uses_synthetic_source(word: &Word, source: &str) -> bool {
    if !word
        .parts_with_spans()
        .any(|(part, _)| word_part_uses_synthetic_source(part))
    {
        return false;
    }
    let rendered = word.render_syntax(source);
    let raw = source
        .get(word.span.start.offset.min(source.len())..word.span.end.offset.min(source.len()));
    raw.is_none_or(|raw| raw != rendered)
}

fn word_part_uses_synthetic_source(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(text) => !text.is_source_backed(),
        WordPart::SingleQuoted { value, .. } => !value.is_source_backed(),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_uses_synthetic_source(&part.kind)),
        _ => false,
    }
}

pub(crate) fn stmt_attachment_span(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> Span {
    let span = if should_render_verbatim(stmt, source_map, options) {
        stmt_verbatim_span_with_source_map(stmt, source_map)
    } else if let Command::Function(command) = &stmt.command {
        function_attachment_span(command)
    } else if let Command::AnonymousFunction(command) = &stmt.command {
        anonymous_function_attachment_span(command)
    } else if let Some((commands, open)) = command_group_commands(&stmt.command) {
        stmt.redirects.iter().fold(
            stmt_group_base_span(stmt, commands, source_map, open),
            |span, redirect| span.merge(redirect.span),
        )
    } else {
        let mut span = command_attachment_span(&stmt.command, source, source_map, options);
        for redirect in &stmt.redirects {
            span = merge_non_empty_span(span, redirect.span);
        }
        if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
            && let Some(terminator_span) = stmt.terminator_span
        {
            span = merge_non_empty_span(span, terminator_span);
        }
        if span == Span::new() {
            stmt_span(stmt)
        } else {
            span
        }
    };
    extend_compound_close_suffix_attachment_span(span, stmt, source, source_map)
}

fn extend_compound_close_suffix_attachment_span(
    span: Span,
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> Span {
    let Some(close_span) = stmt_compound_close_span(stmt, source, source_map) else {
        return span;
    };
    close_suffix_comment_span(source, source_map, close_span).map_or(span, |comment_span| {
        merge_non_empty_span(span, comment_span)
    })
}

fn stmt_compound_close_span(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> Option<Span> {
    let Command::Compound(command) = &stmt.command else {
        return None;
    };
    match command {
        CompoundCommand::If(command) => Some(if_close_span(command, source, source_map)),
        CompoundCommand::For(command) => match command.syntax {
            ForSyntax::InDoDone { done_span, .. } | ForSyntax::ParenDoDone { done_span, .. } => {
                done_close_span(source, source_map, command.span, Some(done_span))
            }
            ForSyntax::InBrace {
                right_brace_span, ..
            }
            | ForSyntax::ParenBrace {
                right_brace_span, ..
            } => Some(normalized_close_keyword_span(
                source,
                source_map,
                right_brace_span,
                "}",
            )),
            ForSyntax::InDirect { .. } | ForSyntax::ParenDirect { .. } => None,
        },
        CompoundCommand::Repeat(command) => match command.syntax {
            RepeatSyntax::DoDone { done_span, .. } => {
                done_close_span(source, source_map, command.span, Some(done_span))
            }
            RepeatSyntax::Brace {
                right_brace_span, ..
            } => Some(normalized_close_keyword_span(
                source,
                source_map,
                right_brace_span,
                "}",
            )),
            RepeatSyntax::Direct => None,
        },
        CompoundCommand::Foreach(command) => match command.syntax {
            ForeachSyntax::InDoDone { done_span, .. } => {
                done_close_span(source, source_map, command.span, Some(done_span))
            }
            ForeachSyntax::ParenBrace {
                right_brace_span, ..
            } => Some(normalized_close_keyword_span(
                source,
                source_map,
                right_brace_span,
                "}",
            )),
        },
        CompoundCommand::ArithmeticFor(command) => {
            done_close_span(source, source_map, command.span, None)
        }
        CompoundCommand::While(command) => done_close_span(source, source_map, command.span, None),
        CompoundCommand::Until(command) => done_close_span(source, source_map, command.span, None),
        CompoundCommand::Select(command) => done_close_span(source, source_map, command.span, None),
        CompoundCommand::Case(command) => last_shell_keyword_start(source, command.span, "esac")
            .map(|start| source_map.span_for_offsets(start, start + "esac".len())),
        _ => None,
    }
}

fn close_suffix_comment_span(
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    close_span: Span,
) -> Option<Span> {
    let (comment_start, comment_end) = close_suffix_comment_offsets(source, close_span)?;
    Some(source_map.span_for_offsets(comment_start, comment_end))
}

pub(crate) fn if_close_span(
    command: &IfCommand,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> Span {
    let (syntax_close, keyword) = match command.syntax {
        IfSyntax::ThenFi { fi_span, .. } => (fi_span, "fi"),
        IfSyntax::Brace {
            right_brace_span, ..
        } => (right_brace_span, "}"),
    };
    let syntax_close = normalized_close_keyword_span(source, source_map, syntax_close, keyword);
    matching_if_close_start(source, command.span)
        .map(|start| source_map.span_for_offsets(start, start + keyword.len()))
        .unwrap_or(syntax_close)
}

pub(crate) fn done_close_span(
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    span: Span,
    fallback: Option<Span>,
) -> Option<Span> {
    matching_done_close_start(source, span)
        .map(|start| source_map.span_for_offsets(start, start + "done".len()))
        .or_else(|| {
            fallback.map(|span| normalized_close_keyword_span(source, source_map, span, "done"))
        })
}

fn command_attachment_span(
    command: &Command,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> Span {
    match command {
        Command::Binary(command) => {
            stmt_attachment_span(&command.left, source, source_map, options).merge(
                stmt_attachment_span(&command.right, source, source_map, options),
            )
        }
        _ => command_format_span(command),
    }
}

pub(crate) fn stmt_render_start_line(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> usize {
    if let Some((commands, open)) = command_group_commands(&stmt.command) {
        group_render_start_line(stmt, commands.as_slice(), source, source_map, open, options)
    } else {
        stmt_attachment_span(stmt, source, source_map, options)
            .start
            .line
    }
}

fn group_render_start_line(
    stmt: &Stmt,
    commands: &[Stmt],
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    options: &crate::options::ResolvedShellFormatOptions,
) -> usize {
    group_attachment_span(commands, source_map, open, matching_group_close(open))
        .map(|span| span.start.line)
        .or_else(|| {
            find_empty_group_open_offset(source, stmt_span(stmt).start.offset, open)
                .map(|offset| source_map.line_number_for_offset(offset))
        })
        .unwrap_or_else(|| {
            stmt_attachment_span(stmt, source, source_map, options)
                .start
                .line
        })
}

pub(crate) fn matching_group_close(open: char) -> char {
    match open {
        '{' => '}',
        '(' => ')',
        other => other,
    }
}

fn find_empty_group_open_offset(
    source: &str,
    mut close_offset: usize,
    open: char,
) -> Option<usize> {
    close_offset = close_offset.min(source.len());
    while close_offset > 0 {
        let ch = source[..close_offset].chars().next_back()?;
        close_offset -= ch.len_utf8();
        if ch.is_whitespace() {
            continue;
        }
        return (ch == open).then_some(close_offset);
    }
    None
}

fn stmt_has_alignment_sensitive_padding(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let mut spans = stmt_token_spans(stmt);
    spans.retain(|span| span != &Span::new() && span.start.offset < span.end.offset);
    spans.sort_by_key(|span| span.start.offset);
    spans.windows(2).any(|window| {
        let [left, right] = window else {
            return false;
        };
        if right.start.offset <= left.end.offset {
            return false;
        }
        source_map.has_alignment_padding_between(left.end.offset, right.start.offset)
    })
}

pub(crate) fn case_item_was_inline_in_source(item: &CaseItem) -> bool {
    let Some(stmt) = item.body.first() else {
        return false;
    };

    item.patterns
        .last()
        .is_some_and(|pattern| pattern.span.end.line == stmt_span(stmt).start.line)
        && item
            .terminator_span
            .is_some_and(|span| span.start.line == stmt_format_span(stmt).end.line)
}

pub(crate) fn case_item_body_upper_bound(item: &CaseItem, fallback: usize) -> Option<usize> {
    Some(
        item.terminator_span
            .map(|span| span.start.offset)
            .unwrap_or(fallback),
    )
}

fn command_token_spans(command: &Command) -> Vec<Span> {
    match command {
        Command::Simple(command) => {
            let mut spans = command
                .assignments
                .iter()
                .map(|assignment| assignment.span)
                .collect::<Vec<_>>();
            if !command.name.parts.is_empty() {
                spans.push(command.name.span);
            }
            spans.extend(command.args.iter().map(|word| word.span));
            spans
        }
        Command::Builtin(command) => {
            let (span, name, assignments, primary, extra_args) = builtin_like_parts(command);
            builtin_like_token_spans(span.start, name, assignments, primary, extra_args)
        }
        Command::Decl(command) => {
            let mut spans = command
                .assignments
                .iter()
                .map(|assignment| assignment.span)
                .collect::<Vec<_>>();
            spans.push(command.variant_span);
            spans.extend(command.operands.iter().map(|operand| match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
                DeclOperand::Name(name) => name.span,
                DeclOperand::Assignment(assignment) => assignment.span,
            }));
            spans
        }
        Command::Binary(command) => vec![command.span],
        Command::Compound(command) => vec![compound_format_span(command)],
        Command::Function(command) => vec![
            function_header_span(command),
            stmt_format_span(&command.body),
        ],
        Command::AnonymousFunction(command) => {
            let mut spans = vec![
                anonymous_function_header_span(command),
                stmt_format_span(&command.body),
            ];
            spans.extend(command.args.iter().map(|argument| argument.span));
            spans
        }
    }
}

fn builtin_like_token_spans(
    start: shuck_ast::Position,
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
) -> Vec<Span> {
    let mut spans = assignments
        .iter()
        .map(|assignment| assignment.span)
        .collect::<Vec<_>>();
    spans.push(Span::from_positions(start, start.advanced_by(name)));
    if let Some(primary) = primary {
        spans.push(primary.span);
    }
    spans.extend(extra_args.iter().map(|argument| argument.span));
    spans
}

fn stmt_token_spans(stmt: &Stmt) -> Vec<Span> {
    let mut spans = if stmt.negated {
        vec![Span::from_positions(
            stmt.span.start,
            stmt.span.start.advanced_by("!"),
        )]
    } else {
        Vec::new()
    };
    spans.extend(command_token_spans(&stmt.command));
    spans.extend(stmt.redirects.iter().map(|redirect| redirect.span));
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        spans.push(terminator_span);
    }
    spans
}

pub(crate) fn render_background_operator(operator: BackgroundOperator) -> &'static str {
    match operator {
        BackgroundOperator::Plain => "&",
        BackgroundOperator::Pipe => "&|",
        BackgroundOperator::Bang => "&!",
    }
}

pub(crate) fn case_terminator(terminator: CaseTerminator) -> &'static str {
    match terminator {
        CaseTerminator::Break => ";;",
        CaseTerminator::FallThrough => ";&",
        CaseTerminator::Continue => ";;&",
        CaseTerminator::ContinueMatching => ";|",
    }
}

pub(crate) fn binary_operator(operator: &shuck_ast::BinaryOp) -> &'static str {
    match operator {
        shuck_ast::BinaryOp::And => "&&",
        shuck_ast::BinaryOp::Or => "||",
        shuck_ast::BinaryOp::Pipe => "|",
        shuck_ast::BinaryOp::PipeAll => "|&",
    }
}

pub(crate) fn slice_span(source: &str, span: Option<Span>) -> &str {
    span.and_then(|span| source.get(span.start.offset..span.end.offset))
        .unwrap_or("")
}

pub(crate) fn extend_heredoc_body_span(span: Span, source: &str) -> Span {
    let mut end = span.end.offset;
    while end < source.len() {
        let byte = source.as_bytes()[end];
        end += 1;
        if byte == b'\n' {
            break;
        }
    }
    let end_position = span.start.advanced_by(&source[span.start.offset..end]);
    Span::from_positions(span.start, end_position)
}

#[cfg(test)]
mod tests {
    use super::*;

    use shuck_parser::parser::Parser;

    fn parse(source: &str) -> shuck_ast::File {
        Parser::new(source).parse().unwrap().file
    }

    #[test]
    fn group_verbatim_span_keeps_wrapper_comments_with_semicolon_terminated_body() {
        let source = "{ # note\n  echo ok; # inside\n}\n";
        let file = parse(source);
        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };

        let span = group_verbatim_span(brace_group.as_slice(), source, '{', '}');

        assert_eq!(span.slice(source), "{ # note\n  echo ok; # inside\n}");
    }

    #[test]
    fn group_verbatim_span_keeps_wrapper_comments_around_heredoc_bodies() {
        let source = "{ # note\n  cat <<EOF\npayload\nEOF\n}\n";
        let file = parse(source);
        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };

        let span = group_verbatim_span(brace_group.as_slice(), source, '{', '}');

        assert_eq!(span.slice(source), "{ # note\n  cat <<EOF\npayload\nEOF\n}");
    }

    #[test]
    fn group_verbatim_span_keeps_wrapper_comments_across_line_continuations() {
        let source = "{ # note\n  echo ok; \\\n}\n";
        let file = parse(source);
        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };

        let span = group_verbatim_span(brace_group.as_slice(), source, '{', '}');

        assert_eq!(span.slice(source), "{ # note\n  echo ok; \\\n}");
    }
}
