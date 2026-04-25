use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PresenceTestReferenceFact {
    command_span: Span,
    reference_id: ReferenceId,
}

impl PresenceTestReferenceFact {
    pub(crate) fn command_span(&self) -> Span {
        self.command_span
    }

    pub(crate) fn reference_id(&self) -> ReferenceId {
        self.reference_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PresenceTestNameFact {
    command_span: Span,
    tested_span: Span,
}

impl PresenceTestNameFact {
    pub(crate) fn command_span(&self) -> Span {
        self.command_span
    }

    pub(crate) fn tested_span(&self) -> Span {
        self.tested_span
    }
}

#[derive(Debug, Default)]
pub(super) struct PresenceTestedNames {
    pub(super) global_names: FxHashSet<Name>,
    pub(super) nested_command_spans_by_name: FxHashMap<Name, Vec<Span>>,
    pub(super) c006_global_names: FxHashSet<Name>,
    pub(super) c006_nested_command_spans_by_name: FxHashMap<Name, Vec<Span>>,
    pub(super) references_by_name: FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    pub(super) names_by_name: FxHashMap<Name, Vec<PresenceTestNameFact>>,
}

pub(super) fn build_presence_tested_names(
    commands: &[CommandFact<'_>],
    source: &str,
    semantic: &SemanticModel,
) -> PresenceTestedNames {
    let mut global_names = FxHashSet::default();
    let mut nested_command_spans_by_name = FxHashMap::<Name, Vec<Span>>::default();
    let mut c006_global_names = FxHashSet::default();
    let mut c006_nested_command_spans_by_name = FxHashMap::<Name, Vec<Span>>::default();
    let mut references_by_name = FxHashMap::<Name, Vec<PresenceTestReferenceFact>>::default();
    let mut names_by_name = FxHashMap::<Name, Vec<PresenceTestNameFact>>::default();
    let outermost_nested_scopes = build_outermost_nested_presence_scopes(commands);
    let sorted_reference_indices = sorted_presence_reference_indices(semantic.references());

    for command in commands {
        let mut command_names = FxHashSet::default();
        let mut c006_command_names = FxHashSet::default();
        let mut command_reference_ids = FxHashSet::default();
        let mut command_name_spans = Vec::<(Name, Span)>::new();

        if let Some(simple_test) = command.simple_test() {
            let mut simple_test_names = FxHashSet::default();
            collect_presence_tested_names_from_simple_test_operands(
                simple_test.operands(),
                source,
                semantic.references(),
                &sorted_reference_indices,
                &mut simple_test_names,
                &mut command_reference_ids,
                &mut command_name_spans,
            );
            if simple_test.syntax() == SimpleTestSyntax::Bracket {
                c006_command_names.extend(simple_test_names.iter().cloned());
            }
            command_names.extend(simple_test_names);
        }

        if let Some(conditional) = command.conditional() {
            let mut conditional_names = FxHashSet::default();
            collect_presence_tested_names_from_conditional_expr(
                conditional.root().expression(),
                source,
                semantic.references(),
                &sorted_reference_indices,
                &mut conditional_names,
                &mut command_reference_ids,
                &mut command_name_spans,
            );
            c006_command_names.extend(conditional_names.iter().cloned());
            command_names.extend(conditional_names);
        }

        for reference_id in command_reference_ids {
            let reference = semantic.reference(reference_id);
            references_by_name
                .entry(reference.name.clone())
                .or_default()
                .push(PresenceTestReferenceFact {
                    command_span: command.span(),
                    reference_id,
                });
        }

        for (name, tested_span) in command_name_spans {
            names_by_name
                .entry(name)
                .or_default()
                .push(PresenceTestNameFact {
                    command_span: command.span(),
                    tested_span,
                });
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
            for name in c006_command_names {
                c006_nested_command_spans_by_name
                    .entry(name)
                    .or_default()
                    .push(span);
            }
        } else {
            global_names.extend(command_names);
            c006_global_names.extend(c006_command_names);
        }
    }

    for spans in nested_command_spans_by_name.values_mut() {
        spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
        spans.dedup();
    }
    for spans in c006_nested_command_spans_by_name.values_mut() {
        spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
        spans.dedup();
    }

    for references in references_by_name.values_mut() {
        references.sort_unstable_by_key(|fact| {
            let reference = semantic.reference(fact.reference_id());
            (
                fact.command_span().start.offset,
                fact.command_span().end.offset,
                reference.span.start.offset,
                reference.span.end.offset,
            )
        });
        references.dedup();
    }

    for names in names_by_name.values_mut() {
        names.sort_unstable_by_key(|fact| {
            (
                fact.command_span().start.offset,
                fact.command_span().end.offset,
                fact.tested_span().start.offset,
                fact.tested_span().end.offset,
            )
        });
        names.dedup();
    }

    PresenceTestedNames {
        global_names,
        nested_command_spans_by_name,
        c006_global_names,
        c006_nested_command_spans_by_name,
        references_by_name,
        names_by_name,
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
    references: &[Reference],
    sorted_reference_indices: &[usize],
    names: &mut FxHashSet<Name>,
    reference_ids: &mut FxHashSet<ReferenceId>,
    name_spans: &mut Vec<(Name, Span)>,
) {
    let mut index = 0;
    while index < operands.len() {
        if is_simple_test_logical_operator(operands[index], source) {
            index += 1;
            continue;
        }

        let consumed = collect_presence_tested_names_from_simple_test_leaf(
            &operands[index..],
            source,
            references,
            sorted_reference_indices,
            names,
            reference_ids,
            name_spans,
        );
        if consumed == 0 {
            break;
        }
        index += consumed;
    }
}

fn collect_presence_tested_names_from_simple_test_leaf(
    operands: &[&Word],
    source: &str,
    references: &[Reference],
    sorted_reference_indices: &[usize],
    names: &mut FxHashSet<Name>,
    reference_ids: &mut FxHashSet<ReferenceId>,
    name_spans: &mut Vec<(Name, Span)>,
) -> usize {
    let Some(first) = operands.first().copied() else {
        return 0;
    };

    if static_word_text(first, source).as_deref() == Some("!") {
        return 1 + collect_presence_tested_names_from_simple_test_leaf(
            &operands[1..],
            source,
            references,
            sorted_reference_indices,
            names,
            reference_ids,
            name_spans,
        );
    }

    if static_word_text(first, source).as_deref() == Some("-v") {
        if let Some(word) = operands.get(1).copied() {
            record_presence_tested_name_from_variable_set_word(word, source, names, name_spans);
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
            record_presence_test_word(
                word,
                references,
                sorted_reference_indices,
                names,
                reference_ids,
            );
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
        record_presence_test_word(
            first,
            references,
            sorted_reference_indices,
            names,
            reference_ids,
        );
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
    references: &[Reference],
    sorted_reference_indices: &[usize],
    names: &mut FxHashSet<Name>,
    reference_ids: &mut FxHashSet<ReferenceId>,
    name_spans: &mut Vec<(Name, Span)>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    match expression {
        ConditionalExpr::Word(word) => record_presence_test_word(
            word,
            references,
            sorted_reference_indices,
            names,
            reference_ids,
        ),
        ConditionalExpr::Unary(unary) if unary.op == ConditionalUnaryOp::VariableSet => {
            collect_presence_tested_name_from_conditional_variable_set_operand(
                &unary.expr,
                source,
                names,
                name_spans,
            );
        }
        ConditionalExpr::Unary(unary) if unary.op == ConditionalUnaryOp::Not => {
            collect_presence_tested_names_from_conditional_expr(
                &unary.expr,
                source,
                references,
                sorted_reference_indices,
                names,
                reference_ids,
                name_spans,
            );
        }
        ConditionalExpr::Unary(unary)
            if conditional_unary_operator_family(unary.op)
                == ConditionalOperatorFamily::StringUnary =>
        {
            collect_presence_tested_names_from_conditional_operand(
                &unary.expr,
                references,
                sorted_reference_indices,
                names,
                reference_ids,
            );
        }
        ConditionalExpr::Binary(binary)
            if conditional_binary_operator_family(binary.op)
                == ConditionalOperatorFamily::Logical =>
        {
            collect_presence_tested_names_from_conditional_expr(
                &binary.left,
                source,
                references,
                sorted_reference_indices,
                names,
                reference_ids,
                name_spans,
            );
            collect_presence_tested_names_from_conditional_expr(
                &binary.right,
                source,
                references,
                sorted_reference_indices,
                names,
                reference_ids,
                name_spans,
            );
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
    name_spans: &mut Vec<(Name, Span)>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    match expression {
        ConditionalExpr::VarRef(reference) => {
            names.insert(reference.name.clone());
            name_spans.push((reference.name.clone(), reference.span));
        }
        ConditionalExpr::Word(word) => {
            record_presence_tested_name_from_variable_set_word(word, source, names, name_spans);
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

fn record_presence_tested_name_from_variable_set_word(
    word: &Word,
    source: &str,
    names: &mut FxHashSet<Name>,
    name_spans: &mut Vec<(Name, Span)>,
) {
    if let Some(name) = presence_tested_name_from_variable_set_word(word, source) {
        names.insert(name.clone());
        name_spans.push((name, word.span));
    }
}

fn presence_tested_name_from_variable_set_word(word: &Word, source: &str) -> Option<Name> {
    static_word_text(word, source).and_then(|text| {
        let base_name = text.split_once('[').map_or(text.as_ref(), |(name, _)| name);
        is_shell_variable_name(base_name).then(|| Name::from(base_name))
    })
}

fn collect_presence_tested_names_from_conditional_operand(
    expression: &ConditionalExpr,
    references: &[Reference],
    sorted_reference_indices: &[usize],
    names: &mut FxHashSet<Name>,
    reference_ids: &mut FxHashSet<ReferenceId>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    if let ConditionalExpr::Word(word) = expression {
        record_presence_test_word(
            word,
            references,
            sorted_reference_indices,
            names,
            reference_ids,
        );
    }
}

fn record_presence_test_word(
    word: &Word,
    references: &[Reference],
    sorted_reference_indices: &[usize],
    names: &mut FxHashSet<Name>,
    reference_ids: &mut FxHashSet<ReferenceId>,
) {
    collect_presence_tested_names_from_word(word, names);
    collect_presence_test_reference_ids_from_word(
        &word.parts,
        references,
        sorted_reference_indices,
        reference_ids,
    );
}

fn sorted_presence_reference_indices(references: &[Reference]) -> Vec<usize> {
    let mut indices = references
        .iter()
        .enumerate()
        .filter(|(_, reference)| {
            !matches!(
                reference.kind,
                ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
            )
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    indices.sort_unstable_by_key(|&index| references[index].span.start.offset);
    indices
}

fn collect_presence_test_reference_ids_from_word(
    parts: &[WordPartNode],
    references: &[Reference],
    sorted_reference_indices: &[usize],
    reference_ids: &mut FxHashSet<ReferenceId>,
) {
    collect_presence_test_reference_ids_from_word_parts(
        parts,
        references,
        sorted_reference_indices,
        reference_ids,
    );
}

fn collect_presence_test_reference_ids_from_word_parts(
    parts: &[WordPartNode],
    references: &[Reference],
    sorted_reference_indices: &[usize],
    reference_ids: &mut FxHashSet<ReferenceId>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_presence_test_reference_ids_from_word(
                parts,
                references,
                sorted_reference_indices,
                reference_ids,
            ),
            WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {
                collect_presence_test_reference_ids_in_span(
                    part.span,
                    references,
                    sorted_reference_indices,
                    reference_ids,
                );
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Substring { reference, .. }
            | WordPart::ArraySlice { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                collect_presence_test_reference_ids_in_span(
                    reference.span,
                    references,
                    sorted_reference_indices,
                    reference_ids,
                );
            }
            WordPart::Parameter(parameter) => collect_presence_test_reference_ids_from_parameter(
                parameter,
                references,
                sorted_reference_indices,
                reference_ids,
            ),
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_presence_test_reference_ids_from_parameter(
    parameter: &shuck_ast::ParameterExpansion,
    references: &[Reference],
    sorted_reference_indices: &[usize],
    reference_ids: &mut FxHashSet<ReferenceId>,
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
                collect_presence_test_reference_ids_in_span(
                    reference.span,
                    references,
                    sorted_reference_indices,
                    reference_ids,
                );
            }
            BourneParameterExpansion::PrefixMatch { .. } => {
                collect_presence_test_reference_ids_in_span(
                    parameter.span,
                    references,
                    sorted_reference_indices,
                    reference_ids,
                );
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            shuck_ast::ZshExpansionTarget::Reference(reference) => {
                collect_presence_test_reference_ids_in_span(
                    reference.span,
                    references,
                    sorted_reference_indices,
                    reference_ids,
                );
            }
            shuck_ast::ZshExpansionTarget::Nested(parameter) => {
                collect_presence_test_reference_ids_from_parameter(
                    parameter,
                    references,
                    sorted_reference_indices,
                    reference_ids,
                );
            }
            shuck_ast::ZshExpansionTarget::Word(_) | shuck_ast::ZshExpansionTarget::Empty => {}
        },
    }
}

fn collect_presence_test_reference_ids_in_span(
    span: Span,
    references: &[Reference],
    sorted_reference_indices: &[usize],
    reference_ids: &mut FxHashSet<ReferenceId>,
) {
    let first_reference = sorted_reference_indices
        .partition_point(|&index| references[index].span.start.offset < span.start.offset);

    for &index in &sorted_reference_indices[first_reference..] {
        let reference = &references[index];
        if reference.span.start.offset > span.end.offset {
            break;
        }
        if contains_span(span, reference.span) {
            reference_ids.insert(reference.id);
        }
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
