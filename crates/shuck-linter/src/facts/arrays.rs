use super::*;

pub(crate) fn build_plain_unindexed_array_reference_facts(
    facts: &LinterFacts<'_>,
) -> Vec<PlainUnindexedArrayReferenceFact> {
    let candidate_references = facts
        .words
        .plain_unindexed_reference_spans
        .iter()
        .copied()
        .flat_map(|span| {
            facts
                .semantic
                .references_in_span(span)
                .filter(move |reference| reference.span == span)
        })
        .collect::<Vec<_>>();
    let mut context = PlainUnindexedArrayReferenceContext::new(facts);

    candidate_references
        .into_iter()
        .filter_map(|reference| context.classify_reference(reference))
        .collect()
}

pub(crate) struct PlainUnindexedArrayReferenceContext<'a, 'src> {
    facts: &'a LinterFacts<'src>,
    semantic: &'a SemanticModel,
    simple_command_ancestors_by_offset: FxHashMap<usize, Vec<SimpleCommandAncestor>>,
    same_command_writers_by_name: FxHashMap<Name, Vec<BindingId>>,
    presence_test_ends_by_name_binding: FxHashMap<Name, FxHashMap<Option<BindingId>, Vec<usize>>>,
    resolved_binding_ids: FxHashMap<ReferenceId, Option<BindingId>>,
}

impl<'a, 'src> PlainUnindexedArrayReferenceContext<'a, 'src> {
    fn new(facts: &'a LinterFacts<'src>) -> Self {
        Self {
            facts,
            semantic: facts.semantic,
            simple_command_ancestors_by_offset: FxHashMap::default(),
            same_command_writers_by_name: FxHashMap::default(),
            presence_test_ends_by_name_binding: FxHashMap::default(),
            resolved_binding_ids: FxHashMap::default(),
        }
    }

    fn classify_reference(
        &mut self,
        reference: &Reference,
    ) -> Option<PlainUnindexedArrayReferenceFact> {
        if self.semantic.is_guarded_parameter_reference(reference.id)
            || self.reference_is_zsh_conditional_operand(reference)
            || self.reference_is_zsh_presence_test(reference)
            || self.reference_has_prior_presence_test(reference)
            || self.reference_reads_into_same_name_array_writer(reference)
        {
            return None;
        }

        let policy = self.semantic.reference_array_use_kind(reference.id)?;
        if self.reference_is_zsh_array_assignment_list_value(reference, policy) {
            return None;
        }

        Some(match policy {
            shuck_semantic::ArrayReferencePolicy::RequiresExplicitSelector => {
                PlainUnindexedArrayReferenceFact::SelectorRequired(
                    SelectorRequiredArrayReference::new(reference.id, reference.span),
                )
            }
            shuck_semantic::ArrayReferencePolicy::NativeZshScalar => {
                PlainUnindexedArrayReferenceFact::NativeZshScalar(
                    NativeZshScalarArrayReference::new(reference.id, reference.span),
                )
            }
            shuck_semantic::ArrayReferencePolicy::Ambiguous => {
                PlainUnindexedArrayReferenceFact::Ambiguous(AmbiguousArrayReference::new(
                    reference.id,
                    reference.span,
                ))
            }
        })
    }

    fn reference_is_zsh_array_assignment_list_value(
        &mut self,
        reference: &Reference,
        policy: shuck_semantic::ArrayReferencePolicy,
    ) -> bool {
        if self.facts.source_facts.shell != ShellDialect::Zsh
            || matches!(
                policy,
                shuck_semantic::ArrayReferencePolicy::RequiresExplicitSelector
            )
        {
            return false;
        }

        self.facts.word_facts().any(|word| {
            self.facts.is_compound_assignment_value_word(word)
                && self
                    .semantic
                    .references_in_command_span(
                        self.facts.command(word.command_id()).span(),
                        word.span(),
                    )
                    .any(|direct_reference| direct_reference.id == reference.id)
        })
    }

    fn array_reference_policy(
        &self,
        reference: &Reference,
    ) -> shuck_semantic::ArrayReferencePolicy {
        if self.facts.source_facts.shell != ShellDialect::Zsh {
            return shuck_semantic::ArrayReferencePolicy::RequiresExplicitSelector;
        }

        self.semantic
            .shell_behavior_at(reference.span.start.offset)
            .array_reference_policy()
    }

    fn reference_reads_into_same_name_array_writer(&mut self, reference: &Reference) -> bool {
        let candidate_bindings = self
            .same_command_candidate_writer_bindings(&reference.name)
            .to_vec();
        candidate_bindings.into_iter().any(|binding_id| {
            let binding = self.semantic.binding(binding_id);
            binding.span.start.offset <= reference.span.start.offset
                && self
                    .same_simple_command_is_assignment_only(binding.span, reference.span)
                    .is_some_and(|assignment_only| {
                        binding_suppresses_same_command_array_read(binding, assignment_only)
                    })
        })
    }

    fn reference_is_zsh_conditional_operand(&self, reference: &Reference) -> bool {
        !matches!(
            self.array_reference_policy(reference),
            shuck_semantic::ArrayReferencePolicy::RequiresExplicitSelector
        ) && matches!(
            reference.kind,
            shuck_semantic::ReferenceKind::ConditionalOperand
        )
    }

    fn reference_is_zsh_presence_test(&self, reference: &Reference) -> bool {
        !matches!(
            self.array_reference_policy(reference),
            shuck_semantic::ArrayReferencePolicy::RequiresExplicitSelector
        ) && self
            .facts
            .presence_test_references(&reference.name)
            .iter()
            .any(|test| test.reference_id() == reference.id)
    }

    fn reference_has_prior_presence_test(&mut self, reference: &Reference) -> bool {
        if loop_header_word_quote(self.facts, reference.span)
            .is_some_and(|quote| quote != WordQuote::Unquoted)
        {
            return false;
        }

        let reference_binding = self.resolved_binding_id(reference.id);
        self.presence_test_ends_by_binding(&reference.name)
            .get(&reference_binding)
            .is_some_and(|ends| ends.partition_point(|end| *end < reference.span.start.offset) > 0)
    }

    fn presence_test_ends_by_binding(
        &mut self,
        name: &Name,
    ) -> &FxHashMap<Option<BindingId>, Vec<usize>> {
        if !self.presence_test_ends_by_name_binding.contains_key(name) {
            let mut by_binding = FxHashMap::<Option<BindingId>, Vec<usize>>::default();

            for test in self.facts.presence_test_references(name) {
                let binding_id = self.resolved_binding_id(test.reference_id());
                by_binding
                    .entry(binding_id)
                    .or_default()
                    .push(test.command_span().end.offset);
            }

            for test in self.facts.presence_test_names(name) {
                let binding_id = self
                    .semantic
                    .visible_binding(name, test.tested_span())
                    .map(|binding| binding.id);
                by_binding
                    .entry(binding_id)
                    .or_default()
                    .push(test.command_span().end.offset);
            }

            for ends in by_binding.values_mut() {
                ends.sort_unstable();
                ends.dedup();
            }

            self.presence_test_ends_by_name_binding
                .insert(name.clone(), by_binding);
        }

        self.presence_test_ends_by_name_binding
            .get(name)
            .expect("presence-test bindings should be cached")
    }

    fn resolved_binding_id(&mut self, reference_id: ReferenceId) -> Option<BindingId> {
        *self
            .resolved_binding_ids
            .entry(reference_id)
            .or_insert_with(|| {
                self.semantic
                    .resolved_binding(reference_id)
                    .map(|binding| binding.id)
            })
    }

    fn same_command_candidate_writer_bindings(&mut self, name: &Name) -> &[BindingId] {
        self.same_command_writers_by_name
            .entry(name.clone())
            .or_insert_with(|| {
                let mut bindings = self
                    .semantic
                    .bindings_for(name)
                    .iter()
                    .copied()
                    .filter(|binding_id| {
                        let binding = self.semantic.binding(*binding_id);
                        matches!(
                            binding.kind,
                            BindingKind::ArrayAssignment
                                | BindingKind::MapfileTarget
                                | BindingKind::ReadTarget
                        )
                    })
                    .collect::<Vec<_>>();
                bindings.sort_unstable_by_key(|binding_id| {
                    self.semantic.binding(*binding_id).span.start.offset
                });
                bindings
            })
    }

    fn simple_command_ancestors(&mut self, offset: usize) -> &[SimpleCommandAncestor] {
        self.simple_command_ancestors_by_offset
            .entry(offset)
            .or_insert_with(|| {
                let mut ancestors = Vec::new();
                let mut current = self.facts.innermost_command_id_containing_offset(offset);
                while let Some(command_id) = current {
                    let command = self.facts.command(command_id);
                    if matches!(command.command(), Command::Simple(_)) {
                        ancestors.push(SimpleCommandAncestor {
                            id: command_id,
                            assignment_only: command.literal_name() == Some(""),
                        });
                    }
                    current = self.facts.command_parent_id(command_id);
                }
                ancestors
            })
    }

    fn same_simple_command_is_assignment_only(
        &mut self,
        binding_span: Span,
        reference_span: Span,
    ) -> Option<bool> {
        let binding_ancestors = self
            .simple_command_ancestors(binding_span.start.offset)
            .to_vec();
        let reference_ancestors = self
            .simple_command_ancestors(reference_span.start.offset)
            .to_vec();

        for reference_ancestor in reference_ancestors {
            if let Some(binding_ancestor) = binding_ancestors
                .iter()
                .find(|binding_ancestor| binding_ancestor.id == reference_ancestor.id)
            {
                return Some(binding_ancestor.assignment_only);
            }
        }

        None
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SimpleCommandAncestor {
    id: CommandId,
    assignment_only: bool,
}

pub(crate) fn span_is_within(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

pub(crate) fn loop_header_word_quote(facts: &LinterFacts<'_>, span: Span) -> Option<WordQuote> {
    facts
        .for_headers()
        .iter()
        .flat_map(|header| header.words().iter())
        .chain(
            facts
                .select_headers()
                .iter()
                .flat_map(|header| header.words().iter()),
        )
        .find(|word| span_is_within(word.span(), span))
        .map(|word| word.classification().quote)
}

pub(crate) fn binding_suppresses_same_command_array_read(
    binding: &Binding,
    assignment_only: bool,
) -> bool {
    matches!(binding.kind, BindingKind::MapfileTarget)
        || (matches!(binding.kind, BindingKind::ReadTarget)
            && binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::ArrayAssignment) && assignment_only)
}

pub(crate) fn collect_use_replacement_expansion_spans(
    parts: &[WordPartNode],
    spans: &mut Vec<Span>,
) {
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
            } if matches!(operator.as_ref(), ParameterOp::UseReplacement) => spans.push(part.span),
            WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. } => {}
        }
    }
}

pub(crate) fn parameter_uses_replacement_operator(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Indirect {
            operator: Some(operator),
            ..
        }
        | BourneParameterExpansion::Operation { operator, .. } => {
            matches!(operator.as_ref(), ParameterOp::UseReplacement)
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

pub(crate) fn collect_broken_assoc_key_spans(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for assignment in command_assignments(command) {
        collect_broken_assoc_key_spans_in_assignment(assignment, source, spans);
    }

    for operand in declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        collect_broken_assoc_key_spans_in_assignment(assignment, source, spans);
    }
}

pub(crate) fn collect_broken_assoc_key_spans_in_assignment(
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

pub(crate) fn has_unclosed_assoc_key_prefix(word: &Word, source: &str) -> bool {
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

pub(crate) fn collect_comma_array_assignment_spans(
    command: &Command,
    source: &str,
    shell: ShellDialect,
    semantic: &SemanticModel,
    spans: &mut Vec<Span>,
) {
    for assignment in command_assignments(command) {
        if let Some(span) = comma_array_assignment_span(assignment, source, shell, semantic) {
            spans.push(span);
        }
    }

    for operand in declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        if let Some(span) = comma_array_assignment_span(assignment, source, shell, semantic) {
            spans.push(span);
        }
    }
}

pub(crate) fn collect_ifs_literal_backslash_assignment_value_spans(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for assignment in command_assignments(command) {
        if let Some(span) = ifs_literal_backslash_assignment_value_span(assignment, source) {
            spans.push(span);
        }
    }

    for operand in declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        if let Some(span) = ifs_literal_backslash_assignment_value_span(assignment, source) {
            spans.push(span);
        }
    }
}

pub(crate) fn ifs_literal_backslash_assignment_value_span(
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

pub(crate) fn comma_array_assignment_span(
    assignment: &Assignment,
    source: &str,
    shell: ShellDialect,
    semantic: &SemanticModel,
) -> Option<Span> {
    let AssignmentValue::Compound(array) = &assignment.value else {
        return None;
    };
    if !array_value_has_unquoted_comma(assignment, array, source, shell, semantic) {
        return None;
    }

    compound_assignment_paren_span(assignment, source)
}

pub(crate) fn array_value_has_unquoted_comma(
    assignment: &Assignment,
    array: &shuck_ast::ArrayExpr,
    source: &str,
    shell: ShellDialect,
    semantic: &SemanticModel,
) -> bool {
    let allow_zsh_option_map_values =
        shell == ShellDialect::Zsh && assignment_target_has_assoc_context(assignment, semantic);

    array.elements.iter().any(|element| {
        let value = element.value();
        value.has_top_level_unquoted_comma()
            && !(allow_zsh_option_map_values && zsh_option_map_value_allows_comma(value, source))
    })
}

pub(crate) fn assignment_target_has_assoc_context(
    assignment: &Assignment,
    semantic: &SemanticModel,
) -> bool {
    semantic
        .binding_for_definition_span(assignment.target.name_span)
        .is_some_and(|binding| {
            semantic
                .binding(binding)
                .attributes
                .contains(BindingAttributes::ASSOC)
        })
        || semantic
            .previous_visible_binding(
                &assignment.target.name,
                assignment.target.name_span,
                Some(assignment.target.name_span),
            )
            .is_some_and(|binding| {
                binding.attributes.contains(BindingAttributes::ASSOC)
                    && !semantic.binding_cleared_before(binding.id, assignment.target.name_span)
            })
}

pub(crate) fn zsh_option_map_value_allows_comma(
    value: &shuck_ast::ArrayValueWord,
    source: &str,
) -> bool {
    let Some(text) = static_word_text(value, source) else {
        return false;
    };
    let aliases = text.split(':').next().unwrap_or_default();
    let Some(rest) = aliases.strip_prefix("opt_") else {
        return false;
    };

    let parts = rest.split(',').collect::<Vec<_>>();
    parts.len() > 1 && parts.iter().copied().all(zsh_option_alias_part)
}

pub(crate) fn zsh_option_alias_part(part: &str) -> bool {
    part.starts_with('-')
        && part.len() > 1
        && part
            .bytes()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(crate) fn compound_assignment_paren_span(
    assignment: &Assignment,
    source: &str,
) -> Option<Span> {
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
