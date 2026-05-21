use super::*;

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
        *line = RawShellText::new(line).trim_command_substitution_open_padding();
    }
    let lines = normalize_multiline_compound_assignment_command_substitutions(lines);

    (!lines.is_empty()).then_some(MultilineCompoundAssignmentLayout {
        lines,
        open_inline: !open_line.trim().is_empty(),
        close_inline: !close_line.trim().is_empty(),
    })
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

        let common_indent =
            common_nonempty_shell_indent(lines[index + 1..body_end].iter().map(String::as_str));
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
    let open = RawShellText::new(lines.get(open_index)?.as_ref())
        .unclosed_command_substitution_body_start()?;
    let mut tail = String::new();
    for (index, line) in lines.get(open_index..)?.iter().enumerate() {
        if index > 0 {
            tail.push('\n');
        }
        tail.push_str(line.as_ref());
    }
    let close = matching_raw_command_substitution_close(&tail, open)?;
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
    let normalized = if preserve_line_continuation
        && RawShellText::new(stripped).starts_with_redirect_continuation()
    {
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

fn trim_multiline_compound_assignment_line_continuation(line: &str) -> &str {
    let trimmed = line.trim_end_matches([' ', '\t']);
    let trailing_backslashes = trimmed
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    if trailing_backslashes % 2 == 1
        && !RawShellText::new(trimmed).continuation_inside_unclosed_substitution()
    {
        trimmed[..trimmed.len().saturating_sub(1)].trim_end_matches([' ', '\t'])
    } else {
        trimmed
    }
}

fn multiline_compound_assignment_common_body_indent(lines: &[&str], open_inline: bool) -> String {
    let mut common: Option<String> = None;
    for line in multiline_compound_assignment_body_lines(lines, open_inline) {
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
    for line in multiline_compound_assignment_body_lines(lines, open_inline) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let stripped = line.strip_prefix(common_indent).unwrap_or(trimmed);
        let indent = leading_shell_indent(stripped);
        if indent.is_empty() || indent.contains('\t') {
            continue;
        }
        width = Some(width.map_or(indent.len(), |current| current.min(indent.len())));
    }
    width.unwrap_or(1).max(1)
}

fn multiline_compound_assignment_body_lines<'a>(
    lines: &'a [&'a str],
    open_inline: bool,
) -> impl Iterator<Item = &'a str> + 'a {
    lines.iter().enumerate().filter_map(move |(index, line)| {
        if index == 0 && open_inline {
            return None;
        }
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with(')') || trimmed.starts_with('#') {
            return None;
        }
        Some(*line)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_assignment_command_substitution_matches_unclosed_opener() {
        let raw_lines = [") $(closed) $(open", "echo body", ")"];

        let body_lines = multiline_compound_assignment_command_substitution_body_lines(&raw_lines);

        assert_eq!(body_lines, vec![false, true, false]);
    }
}
