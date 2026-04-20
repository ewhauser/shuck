fn collect_array_assignment_use_replacement_expansion_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_use_replacement_expansion_spans(&word.parts, &mut spans);
    sort_and_dedup_spans(&mut spans);
    spans
}

fn collect_use_replacement_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { .. }
            | WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::Parameter(parameter) if parameter_uses_replacement_operator(parameter) => {
                spans.push(part.span);
            }
            WordPart::ParameterExpansion { operator, .. }
            | WordPart::IndirectExpansion {
                operator: Some(operator),
                ..
            } if matches!(operator, ParameterOp::UseReplacement) => spans.push(part.span),
            WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. } => {}
        }
    }
}

fn parameter_uses_replacement_operator(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Indirect {
            operator: Some(operator),
            ..
        }
        | BourneParameterExpansion::Operation { operator, .. } => {
            matches!(operator, ParameterOp::UseReplacement)
        }
        BourneParameterExpansion::Access { .. }
        | BourneParameterExpansion::Length { .. }
        | BourneParameterExpansion::Indices { .. }
        | BourneParameterExpansion::PrefixMatch { .. }
        | BourneParameterExpansion::Slice { .. }
        | BourneParameterExpansion::Transformation { .. }
        | BourneParameterExpansion::Indirect { operator: None, .. } => false,
    }
}


fn collect_broken_assoc_key_spans(command: &Command, source: &str, spans: &mut Vec<Span>) {
    for assignment in query::command_assignments(command) {
        collect_broken_assoc_key_spans_in_assignment(assignment, source, spans);
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        collect_broken_assoc_key_spans_in_assignment(assignment, source, spans);
    }
}

fn collect_broken_assoc_key_spans_in_assignment(
    assignment: &Assignment,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let AssignmentValue::Compound(array) = &assignment.value else {
        return;
    };
    if array.kind == ArrayKind::Indexed {
        return;
    }

    for element in &array.elements {
        let ArrayElem::Sequential(word) = element else {
            continue;
        };
        if has_unclosed_assoc_key_prefix(word, source) {
            spans.push(word.span);
        }
    }
}

fn has_unclosed_assoc_key_prefix(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    if !text.starts_with('[') {
        return false;
    }

    let mut excluded = expansion_part_spans(word);
    excluded.sort_by_key(|span| span.start.offset);
    let mut excluded = excluded.into_iter().peekable();

    let mut bracket_depth = 0_i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut saw_equals = false;

    for (offset, ch) in text.char_indices() {
        let absolute_offset = word.span.start.offset + offset;
        while matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.end.offset
        ) {
            excluded.next();
        }
        if matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.start.offset && absolute_offset < span.end.offset
        ) {
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => {
                escaped = true;
                continue;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                continue;
            }
            '"' if !in_single => {
                in_double = !in_double;
                continue;
            }
            _ => {}
        }

        if in_single || in_double {
            continue;
        }

        match ch {
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => {
                bracket_depth -= 1;
                if bracket_depth == 0 {
                    return false;
                }
            }
            '=' if bracket_depth > 0 => saw_equals = true,
            _ => {}
        }
    }

    saw_equals
}

fn collect_comma_array_assignment_spans(command: &Command, source: &str, spans: &mut Vec<Span>) {
    for assignment in query::command_assignments(command) {
        if let Some(span) = comma_array_assignment_span(assignment, source) {
            spans.push(span);
        }
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        if let Some(span) = comma_array_assignment_span(assignment, source) {
            spans.push(span);
        }
    }
}

fn collect_ifs_literal_backslash_assignment_value_spans(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for assignment in query::command_assignments(command) {
        if let Some(span) = ifs_literal_backslash_assignment_value_span(assignment, source) {
            spans.push(span);
        }
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        if let Some(span) = ifs_literal_backslash_assignment_value_span(assignment, source) {
            spans.push(span);
        }
    }
}

fn ifs_literal_backslash_assignment_value_span(
    assignment: &Assignment,
    source: &str,
) -> Option<Span> {
    if assignment.target.name.as_str() != "IFS" {
        return None;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    if word.span.slice(source).starts_with("$'") || word.span.slice(source).starts_with("$\"") {
        return None;
    }

    static_word_text(word, source)
        .is_some_and(|text| text.contains('\\'))
        .then_some(word.span)
}

fn comma_array_assignment_span(assignment: &Assignment, source: &str) -> Option<Span> {
    let AssignmentValue::Compound(array) = &assignment.value else {
        return None;
    };
    if !array_value_has_unquoted_comma(array, source) {
        return None;
    }

    compound_assignment_paren_span(assignment, source)
}

fn array_value_has_unquoted_comma(array: &shuck_ast::ArrayExpr, source: &str) -> bool {
    let _ = source;
    array
        .elements
        .iter()
        .any(|element| element.value().has_top_level_unquoted_comma())
}

fn compound_assignment_paren_span(assignment: &Assignment, source: &str) -> Option<Span> {
    let AssignmentValue::Compound(_) = &assignment.value else {
        return None;
    };

    let text = assignment.span.slice(source);
    let equals = text.find('=')?;
    let open = text[equals + 1..].find('(')? + equals + 1;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }

    let start = assignment.span.start.advanced_by(&text[..open]);
    let end = assignment
        .span
        .start
        .advanced_by(&text[..close + ')'.len_utf8()]);
    Some(Span::from_positions(start, end))
}

