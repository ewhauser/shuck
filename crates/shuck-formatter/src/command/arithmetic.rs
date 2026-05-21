use super::*;

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
    context: RenderContext<'_, '_>,
) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }
    if let Some(ast) = ast {
        let mut rendered = String::new();
        render_arithmetic_expr_to_buf(&mut rendered, ast, context);
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
