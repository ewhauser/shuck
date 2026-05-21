use super::*;

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

pub(crate) fn render_assignment_to_buf(
    assignment: &Assignment,
    context: RenderContext<'_, '_>,
    rendered: &mut String,
) {
    let start = rendered.len();
    let source = context.source;
    render_assignment_head_to_buf(assignment, source, rendered);
    match &assignment.value {
        AssignmentValue::Scalar(value) => {
            render_word_syntax_to_buf(value, context, rendered);
        }
        AssignmentValue::Compound(array) => {
            rendered.push('(');
            for (index, value) in array.elements.iter().enumerate() {
                if index > 0 {
                    rendered.push(' ');
                }
                render_array_elem_to_buf(value, context, rendered);
            }
            rendered.push(')');
        }
    }
    let end = start + trim_unescaped_trailing_whitespace(&rendered[start..]).len();
    rendered.truncate(end);
}

fn render_array_elem_to_buf(
    element: &ArrayElem,
    context: RenderContext<'_, '_>,
    rendered: &mut String,
) {
    let (key, value, op) = array_elem_parts(element);
    if let Some(key) = key {
        render_keyed_array_elem_to_buf(key, value, context, op, rendered);
    } else {
        render_word_syntax_to_buf(value, context, rendered);
    }
}

fn render_keyed_array_elem_to_buf(
    key: &Subscript,
    value: &Word,
    context: RenderContext<'_, '_>,
    operator: &str,
    rendered: &mut String,
) {
    rendered.push('[');
    render_subscript_to_buf(key, context.source, rendered);
    rendered.push(']');
    rendered.push_str(operator);
    render_word_syntax_to_buf(value, context, rendered);
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
