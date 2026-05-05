pub(super) fn is_scannable_simple_arithmetic_subscript_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && (is_shell_variable_name(trimmed) || trimmed.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(super) fn is_simple_arithmetic_reference_subscript(subscript: &Subscript, source: &str) -> bool {
    subscript.selector().is_none()
        && !subscript.syntax_text(source).contains('$')
        && matches!(
            subscript.arithmetic_ast.as_ref().map(|expr| &expr.kind),
            Some(ArithmeticExpr::Variable(_) | ArithmeticExpr::Number(_))
        )
}

pub(super) fn is_arithmetic_variable_reference_word(word: &Word, source: &str) -> bool {
    matches!(word.parts.as_slice(), [part] if match &part.kind {
        WordPart::Variable(name) => is_shell_variable_name(name.as_str()),
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if is_shell_variable_name(reference.name.as_str())
                    && reference
                        .subscript
                        .as_ref()
                        .is_none_or(|subscript| {
                            is_simple_arithmetic_reference_subscript(subscript, source)
                        })
        ),
        _ => false,
    })
}

pub(super) fn collect_arithmetic_command_spans(
    expression: &ArithmeticExprNode,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    visit_arithmetic_words(expression, &mut |word| {
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            true,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

pub(super) fn collect_slice_arithmetic_expression_spans(
    expression: &ArithmeticExprNode,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    visit_arithmetic_words(expression, &mut |word| {
        collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
            &word.parts,
            source,
            dollar_spans,
        );
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            false,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

pub(super) fn collect_arithmetic_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    let Some(text) = text else {
        return;
    };
    if !text.slice(source).contains('$') {
        return;
    }

    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        return;
    };
    collect_arithmetic_expansion_spans_from_parts(
        &word.parts,
        source,
        collect_dollar_spans,
        dollar_spans,
        command_substitution_spans,
    );
}

pub(super) fn collect_dollar_prefixed_arithmetic_variable_spans(
    span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }

        let Some(next) = bytes.get(index + 1).copied() else {
            break;
        };

        let match_end = if next == b'{' {
            let name_start = index + 2;
            let Some(first) = bytes.get(name_start).copied() else {
                index += 1;
                continue;
            };
            if !(first == b'_' || first.is_ascii_alphabetic()) {
                index += 1;
                continue;
            }

            let mut name_end = name_start + 1;
            while let Some(byte) = bytes.get(name_end).copied() {
                if byte == b'_' || byte.is_ascii_alphanumeric() {
                    name_end += 1;
                } else {
                    break;
                }
            }

            match bytes.get(name_end).copied() {
                Some(b'}') => name_end + 1,
                Some(b'[') => {
                    let subscript_start = name_end + 1;
                    let Some(subscript_end_rel) = text[subscript_start..].find(']') else {
                        index += 1;
                        continue;
                    };
                    let subscript_end = subscript_start + subscript_end_rel;
                    if bytes.get(subscript_end + 1) != Some(&b'}')
                        || !is_scannable_simple_arithmetic_subscript_text(
                            &text[subscript_start..subscript_end],
                        )
                    {
                        index += 1;
                        continue;
                    }

                    subscript_end + 2
                }
                _ => {
                    index += 1;
                    continue;
                }
            }
        } else if next == b'_' || next.is_ascii_alphabetic() {
            let mut name_end = index + 2;
            while let Some(byte) = bytes.get(name_end).copied() {
                if byte == b'_' || byte.is_ascii_alphanumeric() {
                    name_end += 1;
                } else {
                    break;
                }
            }
            name_end
        } else {
            index += 1;
            continue;
        };

        let start = span.start.advanced_by(&text[..index]);
        let end = start.advanced_by(&text[index..match_end]);
        spans.push(Span::from_positions(start, end));
        index = match_end;
    }
}

pub(super) fn collect_dollar_prefixed_indexed_subscript_word_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in &word.parts {
        match &part.kind {
            WordPart::Variable(name) if is_shell_variable_name(name.as_str()) => {
                spans.push(part.span);
            }
            WordPart::Variable(_) => {}
            WordPart::Parameter(parameter) => {
                if matches!(
                    parameter.bourne(),
                    Some(BourneParameterExpansion::Access { reference })
                        if is_shell_variable_name(reference.name.as_str())
                            && reference
                                .subscript
                                .as_ref()
                                .is_none_or(|subscript| {
                                    is_simple_arithmetic_reference_subscript(subscript, source)
                                })
                ) {
                    spans.push(part.span);
                }
            }
            WordPart::Literal(_)
            | WordPart::DoubleQuoted { .. }
            | WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

pub(super) fn collect_wrapped_arithmetic_spans_in_word(
    word: &Word,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    let text = word.span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 2 < bytes.len() {
        if !is_unescaped_dollar(bytes, index)
            || bytes[index + 1] != b'('
            || bytes[index + 2] != b'('
        {
            index += 1;
            continue;
        }

        let mut depth = 1usize;
        let mut cursor = index + 3;
        let mut matched = false;

        while cursor < bytes.len() {
            if cursor + 2 < bytes.len()
                && bytes[cursor] == b'$'
                && bytes[cursor + 1] == b'('
                && bytes[cursor + 2] == b'('
            {
                depth += 1;
                cursor += 3;
                continue;
            }

            match bytes[cursor] {
                b'(' => {
                    depth += 1;
                    cursor += 1;
                }
                b')' => {
                    if depth == 1 && cursor + 1 < bytes.len() && bytes[cursor + 1] == b')' {
                        let expr_start = index + 3;
                        let expr_end = cursor;
                        let start = word.span.start.advanced_by(&text[..expr_start]);
                        let end = start.advanced_by(&text[expr_start..expr_end]);
                        let expression_span = Span::from_positions(start, end);
                        collect_dollar_prefixed_arithmetic_variable_spans(
                            expression_span,
                            source,
                            dollar_spans,
                        );
                        collect_wrapped_arithmetic_command_substitution_spans(
                            expression_span,
                            source,
                            command_substitution_spans,
                        );
                        index = cursor + 2;
                        matched = true;
                        break;
                    }

                    depth = depth.saturating_sub(1);
                    cursor += 1;
                }
                _ => {
                    cursor += 1;
                }
            }
        }

        if !matched {
            break;
        }
    }
}

pub(super) fn collect_wrapped_arithmetic_command_substitution_spans(
    span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if !is_unescaped_dollar(bytes, index)
            || bytes[index + 1] != b'('
            || bytes.get(index + 2) == Some(&b'(')
        {
            index += 1;
            continue;
        }

        let Some(end) = find_command_substitution_end(text, index) else {
            break;
        };

        let start = span.start.advanced_by(&text[..index]);
        let end_pos = start.advanced_by(&text[index..end]);
        spans.push(Span::from_positions(start, end_pos));
        index = end;
    }
}

pub(super) fn is_unescaped_dollar(bytes: &[u8], index: usize) -> bool {
    if bytes.get(index) != Some(&b'$') {
        return false;
    }

    let mut backslash_count = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslash_count += 1;
        cursor -= 1;
    }

    backslash_count.is_multiple_of(2)
}

pub(super) fn find_command_substitution_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut paren_depth = 0usize;
    let mut cursor = start + 2;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'{'
        {
            cursor = find_runtime_parameter_closing_brace(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && matches!(bytes[cursor], b'<' | b'>')
            && bytes[cursor + 1] == b'('
        {
            cursor = find_process_substitution_end(text, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(text, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 => return Some(cursor + 1),
            b')' => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

pub(super) fn find_wrapped_arithmetic_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut paren_depth = 0usize;
    let mut cursor = start + 3;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'{'
        {
            cursor = find_runtime_parameter_closing_brace(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && matches!(bytes[cursor], b'<' | b'>')
            && bytes[cursor + 1] == b'('
        {
            cursor = find_process_substitution_end(text, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(text, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 && cursor + 1 < bytes.len() && bytes[cursor + 1] == b')' => {
                return Some(cursor + 2);
            }
            b')' if paren_depth > 0 => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

pub(super) fn find_process_substitution_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut paren_depth = 0usize;
    let mut cursor = start + 2;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'{'
        {
            cursor = find_runtime_parameter_closing_brace(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && matches!(bytes[cursor], b'<' | b'>')
            && bytes[cursor + 1] == b'('
        {
            cursor = find_process_substitution_end(text, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(text, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 => return Some(cursor + 1),
            b')' => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

pub(super) fn skip_single_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\'' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

pub(super) fn skip_double_quoted(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut cursor = start;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(text, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'"' => return Some(cursor + 1),
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            _ => cursor += 1,
        }
    }

    None
}

pub(super) fn skip_backticks(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }
        if bytes[cursor] == b'`' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

pub(super) fn word_needs_wrapped_arithmetic_fallback(word: &Word, source: &str) -> bool {
    parts_need_wrapped_arithmetic_fallback(&word.parts, source)
}

pub(super) fn parts_need_wrapped_arithmetic_fallback(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            parts_need_wrapped_arithmetic_fallback(parts, source)
        }
        WordPart::Substring {
            offset_ast: None,
            offset,
            ..
        }
        | WordPart::ArraySlice {
            offset_ast: None,
            offset,
            ..
        } => offset.is_source_backed() && offset.slice(source).starts_with("$(("),
        WordPart::Parameter(parameter) => {
            parameter_needs_wrapped_arithmetic_fallback(parameter, source)
        }
        _ => false,
    })
}

pub(super) fn parameter_needs_wrapped_arithmetic_fallback(
    parameter: &ParameterExpansion,
    source: &str,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
            offset_ast: None,
            offset,
            ..
        }) => offset.is_source_backed() && offset.slice(source).starts_with("$(("),
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => {
                parameter_needs_wrapped_arithmetic_fallback(parameter, source)
            }
            ZshExpansionTarget::Word(word) => word_needs_wrapped_arithmetic_fallback(word, source),
            ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Empty => false,
        },
        _ => false,
    }
}

pub(super) fn collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
    parts: &[WordPartNode],
    source: &str,
    dollar_spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                    parts,
                    source,
                    dollar_spans,
                )
            }
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                let mut ignored_command_substitution_spans = Vec::new();
                if let Some(expression) = expression_ast {
                    visit_arithmetic_words(expression, &mut |word| {
                        collect_arithmetic_context_spans_in_word(
                            word,
                            source,
                            true,
                            dollar_spans,
                            &mut ignored_command_substitution_spans,
                        );
                    });
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &expression_word_ast.parts,
                        source,
                        true,
                        dollar_spans,
                        &mut ignored_command_substitution_spans,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

pub(super) fn collect_arithmetic_context_spans_in_word(
    word: &Word,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    if collect_dollar_spans && is_arithmetic_variable_reference_word(word, source) {
        dollar_spans.push(word.span);
    }

    for part in &word.parts {
        if let WordPart::CommandSubstitution { .. } = &part.kind {
            command_substitution_spans.push(part.span);
        }
    }

    collect_arithmetic_expansion_spans_from_parts(
        &word.parts,
        source,
        collect_dollar_spans,
        dollar_spans,
        command_substitution_spans,
    );
}

pub(super) fn collect_arithmetic_spans_in_parameter_operator(
    operator: &ParameterOp,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    match operator {
        ParameterOp::ReplaceFirst {
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            replacement_word_ast,
            ..
        } => collect_arithmetic_expansion_spans_from_parts(
            &replacement_word_ast.parts,
            source,
            collect_dollar_spans,
            dollar_spans,
            command_substitution_spans,
        ),
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::RemovePrefixShort { .. }
        | ParameterOp::RemovePrefixLong { .. }
        | ParameterOp::RemoveSuffixShort { .. }
        | ParameterOp::RemoveSuffixLong { .. }
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

pub(super) fn collect_arithmetic_expansion_spans_from_parts(
    parts: &[WordPartNode],
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_arithmetic_expansion_spans_from_parts(
                parts,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast {
                    visit_arithmetic_words(expression, &mut |word| {
                        collect_arithmetic_context_spans_in_word(
                            word,
                            source,
                            collect_dollar_spans,
                            dollar_spans,
                            command_substitution_spans,
                        );
                    });
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &expression_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            WordPart::Parameter(parameter) => collect_arithmetic_spans_in_parameter_expansion(
                parameter,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_parameter_operator(
                    operator,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => collect_arithmetic_spans_in_var_ref(
                reference,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                if let Some(expression) = offset_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &offset_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &length_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

pub(super) fn collect_arithmetic_update_operator_spans_from_parts(
    parts: &[WordPartNode],
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_from_parts_impl(
        parts, semantic, None, source, spans, false,
    );
}

pub(super) fn collect_arithmetic_update_operator_spans_from_parts_with_nested_commands(
    parts: &[WordPartNode],
    semantic: &SemanticModel,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_from_parts_impl(
        parts,
        semantic,
        Some(semantic_artifacts),
        source,
        spans,
        true,
    );
}

pub(super) fn collect_arithmetic_update_operator_spans_from_parts_impl(
    parts: &[WordPartNode],
    semantic: &SemanticModel,
    semantic_artifacts: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_arithmetic_update_operator_spans_from_parts_impl(
                    parts,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                )
            }
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &expression_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            WordPart::Parameter(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
                    parameter,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                )
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
                collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
                    operator,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                )
            }
            WordPart::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(expression) = offset_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &offset_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &length_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                if include_nested_commands {
                    let semantic_artifacts = semantic_artifacts
                        .expect("nested command arithmetic scans require semantic artifacts");
                    collect_arithmetic_update_operator_spans_in_nested_command_body(
                        body,
                        semantic_artifacts,
                        semantic,
                        source,
                        spans,
                    );
                }
            }
        }
    }
}

pub(super) fn collect_arithmetic_update_operator_spans_in_var_ref(
    reference: &VarRef,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_in_var_ref_impl(
        reference, semantic, None, source, spans, false,
    );
}

pub(super) fn collect_arithmetic_update_operator_spans_in_var_ref_impl(
    reference: &VarRef,
    semantic: &SemanticModel,
    semantic_artifacts: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    if !var_ref_subscript_has_assoc_semantics(reference, semantic) {
        collect_arithmetic_update_operator_spans_in_subscript(
            reference.subscript.as_deref(),
            source,
            spans,
        );
    }
    visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_arithmetic_update_operator_spans_from_parts_impl(
            &word.parts,
            semantic,
            semantic_artifacts,
            source,
            spans,
            include_nested_commands,
        );
    });
}

pub(super) fn collect_arithmetic_update_operator_spans_in_parameter_expansion_with_nested_commands(
    parameter: &ParameterExpansion,
    semantic: &SemanticModel,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
        parameter,
        semantic,
        Some(semantic_artifacts),
        source,
        spans,
        true,
    );
}

pub(super) fn collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
    parameter: &ParameterExpansion,
    semantic: &SemanticModel,
    semantic_artifacts: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(operator) = operator.as_ref() {
                    collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
                        operator,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
                if let Some(operand_word_ast) = operand_word_ast.as_ref() {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &operand_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
                collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
                    operator,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(operand_word_ast) = operand_word_ast.as_ref() {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &operand_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(expression) = offset_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &offset_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &length_word_ast.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            ZshExpansionTarget::Nested(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
                    parameter,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            ZshExpansionTarget::Word(word) => {
                collect_arithmetic_update_operator_spans_from_parts_impl(
                    &word.parts,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            ZshExpansionTarget::Empty => {}
        },
    }
}

pub(super) fn collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
    operator: &ParameterOp,
    semantic: &SemanticModel,
    semantic_artifacts: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    match operator {
        ParameterOp::ReplaceFirst {
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            replacement_word_ast,
            ..
        } => collect_arithmetic_update_operator_spans_from_parts_impl(
            &replacement_word_ast.parts,
            semantic,
            semantic_artifacts,
            source,
            spans,
            include_nested_commands,
        ),
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::RemovePrefixShort { .. }
        | ParameterOp::RemovePrefixLong { .. }
        | ParameterOp::RemoveSuffixShort { .. }
        | ParameterOp::RemoveSuffixLong { .. }
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

pub(super) fn collect_arithmetic_spans_in_var_ref(
    reference: &VarRef,
    source: &str,
    _collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
            &word.parts,
            source,
            dollar_spans,
        );
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            false,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

pub(super) fn collect_arithmetic_spans_in_parameter_expansion(
    parameter: &ParameterExpansion,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_fragment(
                    operand_word_ast.as_deref(),
                    operand.as_ref(),
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_fragment(
                    operand_word_ast.as_deref(),
                    operand.as_ref(),
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_parameter_operator(
                    operator,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                if let Some(expression) = offset_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &offset_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &length_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => collect_arithmetic_spans_in_var_ref(
                reference,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            ZshExpansionTarget::Nested(parameter) => {
                collect_arithmetic_spans_in_parameter_expansion(
                    parameter,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                )
            }
            ZshExpansionTarget::Word(word) => collect_arithmetic_expansion_spans_from_parts(
                &word.parts,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            ZshExpansionTarget::Empty => {}
        },
    }
}
