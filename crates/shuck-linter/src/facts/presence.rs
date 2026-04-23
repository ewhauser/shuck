use super::*;

#[derive(Debug, Default)]
pub(super) struct PresenceTestedNames {
    pub(super) global_names: FxHashSet<Name>,
    pub(super) nested_command_spans_by_name: FxHashMap<Name, Vec<Span>>,
}

pub(super) fn build_presence_tested_names(
    commands: &[CommandFact<'_>],
    source: &str,
) -> PresenceTestedNames {
    let mut global_names = FxHashSet::default();
    let mut nested_command_spans_by_name = FxHashMap::<Name, Vec<Span>>::default();
    let outermost_nested_scopes = build_outermost_nested_presence_scopes(commands);

    for command in commands {
        let mut command_names = FxHashSet::default();

        if let Some(simple_test) = command.simple_test() {
            collect_presence_tested_names_from_simple_test_operands(
                simple_test.operands(),
                source,
                &mut command_names,
            );
        }

        if let Some(conditional) = command.conditional() {
            collect_presence_tested_names_from_conditional_expr(
                conditional.root().expression(),
                source,
                &mut command_names,
            );
        }

        if command.is_nested_word_command() {
            let span =
                outermost_nested_scopes[command.id().index()].unwrap_or_else(|| command.span());
            for name in command_names {
                nested_command_spans_by_name
                    .entry(name)
                    .or_default()
                    .push(span);
            }
        } else {
            global_names.extend(command_names);
        }
    }

    for spans in nested_command_spans_by_name.values_mut() {
        spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
        spans.dedup();
    }

    PresenceTestedNames {
        global_names,
        nested_command_spans_by_name,
    }
}

fn build_outermost_nested_presence_scopes(commands: &[CommandFact<'_>]) -> Vec<Option<Span>> {
    let mut ordered_commands = commands
        .iter()
        .map(|command| {
            (
                command.span(),
                command.id(),
                command.is_nested_word_command(),
            )
        })
        .collect::<Vec<_>>();
    ordered_commands.sort_unstable_by(|left, right| {
        compare_command_offset_entries((left.0, left.1), (right.0, right.1))
    });

    let mut outermost_scopes = vec![None; commands.len()];
    let mut active_nested_scopes = Vec::<Span>::new();
    for (span, id, is_nested) in ordered_commands {
        pop_finished_nested_presence_scopes(&mut active_nested_scopes, span.start.offset);
        outermost_scopes[id.index()] = active_nested_scopes
            .first()
            .copied()
            .or_else(|| is_nested.then_some(span));
        if is_nested {
            active_nested_scopes.push(span);
        }
    }

    outermost_scopes
}

fn pop_finished_nested_presence_scopes(active_nested_scopes: &mut Vec<Span>, offset: usize) {
    while active_nested_scopes
        .last()
        .is_some_and(|span| span.end.offset <= offset)
    {
        active_nested_scopes.pop();
    }
}

fn collect_presence_tested_names_from_simple_test_operands(
    operands: &[&Word],
    source: &str,
    names: &mut FxHashSet<Name>,
) {
    let mut index = 0;
    while index < operands.len() {
        if is_simple_test_logical_operator(operands[index], source) {
            index += 1;
            continue;
        }

        let consumed =
            collect_presence_tested_names_from_simple_test_leaf(&operands[index..], source, names);
        if consumed == 0 {
            break;
        }
        index += consumed;
    }
}

fn collect_presence_tested_names_from_simple_test_leaf(
    operands: &[&Word],
    source: &str,
    names: &mut FxHashSet<Name>,
) -> usize {
    let Some(first) = operands.first().copied() else {
        return 0;
    };

    if static_word_text(first, source).as_deref() == Some("!") {
        return 1 + collect_presence_tested_names_from_simple_test_leaf(
            &operands[1..],
            source,
            names,
        );
    }

    if static_word_text(first, source).as_deref() == Some("-v") {
        if let Some(word) = operands.get(1).copied() {
            collect_presence_tested_name_from_variable_set_word(word, source, names);
            return 2;
        }
        return 1;
    }

    if static_word_text(first, source)
        .as_deref()
        .is_some_and(|operator| {
            simple_test_unary_operator_family(operator) == SimpleTestOperatorFamily::StringUnary
        })
    {
        if let Some(word) = operands.get(1).copied() {
            collect_presence_tested_names_from_word(word, names);
            return 2;
        }
        return 1;
    }

    if operands.len() == 1
        || operands
            .get(1)
            .copied()
            .is_some_and(|word| is_simple_test_logical_operator(word, source))
    {
        collect_presence_tested_names_from_word(first, names);
        return 1;
    }

    operands
        .iter()
        .skip(1)
        .position(|word| is_simple_test_logical_operator(word, source))
        .map_or(operands.len(), |offset| offset + 1)
}

fn is_simple_test_logical_operator(word: &Word, source: &str) -> bool {
    matches!(static_word_text(word, source).as_deref(), Some("-a" | "-o"))
}

fn collect_presence_tested_names_from_conditional_expr(
    expression: &ConditionalExpr,
    source: &str,
    names: &mut FxHashSet<Name>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    match expression {
        ConditionalExpr::Word(word) => collect_presence_tested_names_from_word(word, names),
        ConditionalExpr::Unary(unary) if unary.op == ConditionalUnaryOp::VariableSet => {
            collect_presence_tested_name_from_conditional_variable_set_operand(
                &unary.expr,
                source,
                names,
            );
        }
        ConditionalExpr::Unary(unary) if unary.op == ConditionalUnaryOp::Not => {
            collect_presence_tested_names_from_conditional_expr(&unary.expr, source, names);
        }
        ConditionalExpr::Unary(unary)
            if conditional_unary_operator_family(unary.op)
                == ConditionalOperatorFamily::StringUnary =>
        {
            collect_presence_tested_names_from_conditional_operand(&unary.expr, names);
        }
        ConditionalExpr::Binary(binary)
            if conditional_binary_operator_family(binary.op)
                == ConditionalOperatorFamily::Logical =>
        {
            collect_presence_tested_names_from_conditional_expr(&binary.left, source, names);
            collect_presence_tested_names_from_conditional_expr(&binary.right, source, names);
        }
        ConditionalExpr::Unary(_)
        | ConditionalExpr::Binary(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => {}
        ConditionalExpr::Parenthesized(_) => {
            unreachable!("parentheses should be stripped before collecting presence tests")
        }
    }
}

fn collect_presence_tested_name_from_conditional_variable_set_operand(
    expression: &ConditionalExpr,
    source: &str,
    names: &mut FxHashSet<Name>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    match expression {
        ConditionalExpr::VarRef(reference) => {
            names.insert(reference.name.clone());
        }
        ConditionalExpr::Word(word) => {
            collect_presence_tested_name_from_variable_set_word(word, source, names);
        }
        ConditionalExpr::Parenthesized(_) => {
            unreachable!("parentheses should be stripped before collecting presence tests")
        }
        ConditionalExpr::Unary(_)
        | ConditionalExpr::Binary(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_) => {}
    }
}

fn collect_presence_tested_name_from_variable_set_word(
    word: &Word,
    source: &str,
    names: &mut FxHashSet<Name>,
) {
    if let Some(name) = static_word_text(word, source).and_then(|text| {
        let base_name = text.split_once('[').map_or(text.as_ref(), |(name, _)| name);
        is_shell_variable_name(base_name).then(|| Name::from(base_name))
    }) {
        names.insert(name);
    }
}

fn collect_presence_tested_names_from_conditional_operand(
    expression: &ConditionalExpr,
    names: &mut FxHashSet<Name>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    if let ConditionalExpr::Word(word) = expression {
        collect_presence_tested_names_from_word(word, names);
    }
}

fn collect_presence_tested_names_from_word(word: &Word, names: &mut FxHashSet<Name>) {
    collect_presence_tested_names_from_word_parts(&word.parts, names);
}

fn collect_presence_tested_names_from_word_parts(
    parts: &[WordPartNode],
    names: &mut FxHashSet<Name>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_presence_tested_names_from_word_parts(parts, names);
            }
            WordPart::Variable(name) | WordPart::PrefixMatch { prefix: name, .. } => {
                names.insert(name.clone());
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Substring { reference, .. }
            | WordPart::ArraySlice { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                names.insert(reference.name.clone());
            }
            WordPart::Parameter(parameter) => {
                collect_presence_tested_names_from_parameter_expansion(parameter, names);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_presence_tested_names_from_parameter_expansion(
    parameter: &shuck_ast::ParameterExpansion,
    names: &mut FxHashSet<Name>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Indirect { reference, .. }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                names.insert(reference.name.clone());
            }
            BourneParameterExpansion::PrefixMatch { prefix: name, .. } => {
                names.insert(name.clone());
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            shuck_ast::ZshExpansionTarget::Reference(reference) => {
                names.insert(reference.name.clone());
            }
            shuck_ast::ZshExpansionTarget::Word(_) => {}
            shuck_ast::ZshExpansionTarget::Nested(parameter) => {
                collect_presence_tested_names_from_parameter_expansion(parameter, names);
            }
            shuck_ast::ZshExpansionTarget::Empty => {}
        },
    }
}
