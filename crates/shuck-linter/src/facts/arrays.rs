fn build_plain_unindexed_array_reference_facts(
    facts: &LinterFacts<'_>,
) -> Vec<PlainUnindexedArrayReferenceFact> {
    let candidate_references = facts
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

struct PlainUnindexedArrayReferenceContext<'a, 'src> {
    facts: &'a LinterFacts<'src>,
    semantic: &'a SemanticModel,
    local_declarations: LocalDeclarationIndex,
    simple_command_ancestors_by_offset: FxHashMap<usize, Vec<SimpleCommandAncestor>>,
    same_command_writers_by_name: FxHashMap<Name, Vec<BindingId>>,
    presence_test_ends_by_name_binding: FxHashMap<Name, FxHashMap<Option<BindingId>, Vec<usize>>>,
    resolved_binding_ids: FxHashMap<ReferenceId, Option<BindingId>>,
    binding_inherits_indexed_array_type_cache: FxHashMap<BindingId, bool>,
    binding_has_prior_local_barrier_cache: FxHashMap<BindingId, bool>,
    binding_is_append_declaration_cache: FxHashMap<BindingId, bool>,
    binding_reset_by_name_only_before_cache: FxHashMap<(BindingId, usize), bool>,
}

impl<'a, 'src> PlainUnindexedArrayReferenceContext<'a, 'src> {
    fn new(facts: &'a LinterFacts<'src>) -> Self {
        Self {
            facts,
            semantic: facts.semantic,
            local_declarations: LocalDeclarationIndex::build(facts.semantic),
            simple_command_ancestors_by_offset: FxHashMap::default(),
            same_command_writers_by_name: FxHashMap::default(),
            presence_test_ends_by_name_binding: FxHashMap::default(),
            resolved_binding_ids: FxHashMap::default(),
            binding_inherits_indexed_array_type_cache: FxHashMap::default(),
            binding_has_prior_local_barrier_cache: FxHashMap::default(),
            binding_is_append_declaration_cache: FxHashMap::default(),
            binding_reset_by_name_only_before_cache: FxHashMap::default(),
        }
    }

    fn classify_reference(
        &mut self,
        reference: &Reference,
    ) -> Option<PlainUnindexedArrayReferenceFact> {
        if self.semantic.is_guarded_parameter_reference(reference.id)
            || self.reference_has_prior_presence_test(reference)
            || self.reference_reads_into_same_name_array_writer(reference)
            || self.reference_has_prior_zsh_scalar_local_barrier(reference)
        {
            return None;
        }

        if let Some(binding) = self.semantic.resolved_binding(reference.id)
            && self.semantic.binding_visible_at(binding.id, reference.span)
            && !binding_is_array_like(binding)
            && !self.binding_inherits_indexed_array_type(binding)
            && (binding_resets_indexed_array_type(binding)
                || self.binding_has_prior_local_barrier(binding)
                || (self.facts.shell == ShellDialect::Zsh
                    && binding_is_initialized_scalar_declaration(binding)))
        {
            return None;
        }

        let array_like = if is_bash_runtime_array_name(reference.name.as_str()) {
            true
        } else {
            let mut binding_ids = Vec::new();
            let mut seen = FxHashSet::default();
            if let Some(binding) = self.semantic.resolved_binding(reference.id)
                && !binding_is_array_like(binding)
                && seen.insert(binding.id)
            {
                binding_ids.push(binding.id);
            }
            for binding_id in self
                .semantic
                .visible_candidate_bindings_for_reference(reference)
            {
                if seen.insert(binding_id) {
                    binding_ids.push(binding_id);
                }
            }

            binding_ids.into_iter().any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                !self.binding_reset_by_name_only_declaration_before(binding, reference.span)
                    && (binding_is_array_like(binding)
                        || self.binding_inherits_indexed_array_type(binding))
            })
        };
        if !array_like {
            return None;
        }

        if is_bash_runtime_array_name(reference.name.as_str()) {
            return Some(PlainUnindexedArrayReferenceFact::SelectorRequired(
                SelectorRequiredArrayReference::new(reference.id, reference.span),
            ));
        }

        Some(match self.array_reference_policy(reference) {
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

    fn array_reference_policy(
        &self,
        reference: &Reference,
    ) -> shuck_semantic::ArrayReferencePolicy {
        if self.facts.shell != ShellDialect::Zsh {
            return shuck_semantic::ArrayReferencePolicy::RequiresExplicitSelector;
        }

        self.semantic
            .shell_behavior_at(reference.span.start.offset)
            .array_reference_policy()
    }

    fn binding_inherits_indexed_array_type(&mut self, binding: &Binding) -> bool {
        if let Some(cached) = self
            .binding_inherits_indexed_array_type_cache
            .get(&binding.id)
            .copied()
        {
            return cached;
        }

        let inherited = if binding_resets_indexed_array_type(binding) {
            false
        } else {
            let initialized_scalar_declaration =
                matches!(binding.kind, BindingKind::Declaration(_))
                    && binding
                        .attributes
                        .contains(BindingAttributes::DECLARATION_INITIALIZED)
                    && !binding
                        .attributes
                        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
            let append_declaration = self.binding_is_append_declaration(binding);
            let prior_local_barrier = self.binding_has_prior_local_barrier(binding);
            let prior_bindings = self
                .semantic
                .bindings_for(&binding.name)
                .iter()
                .copied()
                .filter(|candidate_id| {
                    let candidate = self.semantic.binding(*candidate_id);
                    let same_scope_candidate_allowed = !initialized_scalar_declaration
                        || append_declaration
                        || self.facts.shell != ShellDialect::Zsh;
                    candidate.span.start.offset < binding.span.start.offset
                        && ((candidate.scope != binding.scope && !prior_local_barrier)
                            || same_scope_candidate_allowed)
                        && !self
                            .binding_reset_by_name_only_declaration_before(candidate, binding.span)
                })
                .collect::<Vec<_>>();

            let mut inherited = false;
            for candidate_id in prior_bindings.into_iter().rev() {
                let candidate = self.semantic.binding(candidate_id);
                if binding_resets_indexed_array_type(candidate) {
                    inherited = false;
                    break;
                }
                if binding_is_sticky_indexed_array(candidate) {
                    inherited = true;
                    break;
                }
            }
            inherited
        };

        self.binding_inherits_indexed_array_type_cache
            .insert(binding.id, inherited);
        inherited
    }

    fn reference_has_prior_zsh_scalar_local_barrier(&self, reference: &Reference) -> bool {
        if self.facts.shell != ShellDialect::Zsh {
            return false;
        }

        let latest_barrier = self
            .semantic
            .ancestor_scopes(self.semantic.scope_at(reference.span.start.offset))
            .flat_map(|scope| {
                self.local_declarations
                    .initialized_scalar_local_declarations_for(scope, &reference.name)
                    .iter()
                    .copied()
            })
            .filter(|span| span.end.offset < reference.span.start.offset)
            .max_by_key(|span| span.start.offset);

        latest_barrier
            .is_some_and(|barrier| !self.zsh_array_binding_after_scalar_local_barrier(reference, barrier))
    }

    fn zsh_array_binding_after_scalar_local_barrier(
        &self,
        reference: &Reference,
        barrier: Span,
    ) -> bool {
        self.semantic
            .bindings_for(&reference.name)
            .iter()
            .copied()
            .map(|binding_id| self.semantic.binding(binding_id))
            .any(|binding| {
                binding.span.start.offset > barrier.start.offset
                    && binding.span.start.offset < reference.span.start.offset
                    && self.semantic.binding_visible_at(binding.id, reference.span)
                    && binding_is_array_like(binding)
            })
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
            .or_insert_with(|| self.semantic.resolved_binding(reference_id).map(|binding| binding.id))
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

    fn binding_reset_by_name_only_declaration_before(
        &mut self,
        binding: &Binding,
        at: Span,
    ) -> bool {
        *self
            .binding_reset_by_name_only_before_cache
            .entry((binding.id, at.start.offset))
            .or_insert_with(|| {
                self.local_declarations
                    .name_only_local_declarations_for(binding.scope, &binding.name)
                    .iter()
                    .any(|span| {
                        span.start.offset > binding.span.start.offset
                            && span.end.offset < at.start.offset
                    })
            })
    }

    fn binding_has_prior_local_barrier(&mut self, binding: &Binding) -> bool {
        *self
            .binding_has_prior_local_barrier_cache
            .entry(binding.id)
            .or_insert_with(|| {
                self.local_declarations
                    .local_declarations_for(binding.scope, &binding.name)
                    .iter()
                    .any(|span| span.end.offset < binding.span.start.offset)
            })
    }

    fn binding_is_append_declaration(&mut self, binding: &Binding) -> bool {
        *self
            .binding_is_append_declaration_cache
            .entry(binding.id)
            .or_insert_with(|| {
                self.local_declarations.is_local_append_declaration(
                    binding.scope,
                    &binding.name,
                    binding.span,
                )
            })
    }
}

#[derive(Clone, Copy)]
struct SimpleCommandAncestor {
    id: CommandId,
    assignment_only: bool,
}

struct LocalDeclarationIndex {
    local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    name_only_local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    initialized_scalar_local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    append_local_declaration_spans: FxHashSet<(ScopeId, Name, usize, usize)>,
}

impl LocalDeclarationIndex {
    fn build(semantic: &SemanticModel) -> Self {
        let mut local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut name_only_local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut initialized_scalar_local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut append_local_declaration_spans = FxHashSet::default();

        for declaration in semantic.declarations() {
            if !matches!(declaration.builtin, DeclarationBuiltin::Local) {
                continue;
            }

            let scope = semantic.scope_at(declaration.span.start.offset);
            let declaration_has_array_flag = declaration.operands.iter().any(|operand| {
                matches!(
                    operand,
                    SemanticDeclarationOperand::Flag {
                        flag: 'a' | 'A',
                        ..
                    }
                )
            });
            for operand in &declaration.operands {
                match operand {
                    SemanticDeclarationOperand::Name { name, .. } => {
                        local_declarations_by_scope_name
                            .entry((scope, name.clone()))
                            .or_default()
                            .push(declaration.span);
                        name_only_local_declarations_by_scope_name
                            .entry((scope, name.clone()))
                            .or_default()
                            .push(declaration.span);
                    }
                    SemanticDeclarationOperand::Assignment {
                        name,
                        name_span,
                        append,
                        ..
                    } => {
                        local_declarations_by_scope_name
                            .entry((scope, name.clone()))
                            .or_default()
                            .push(declaration.span);
                        if !*append && !declaration_has_array_flag {
                            initialized_scalar_local_declarations_by_scope_name
                                .entry((scope, name.clone()))
                                .or_default()
                                .push(declaration.span);
                        }
                        if *append {
                            append_local_declaration_spans.insert((
                                scope,
                                name.clone(),
                                name_span.start.offset,
                                name_span.end.offset,
                            ));
                        }
                    }
                    SemanticDeclarationOperand::Flag { .. }
                    | SemanticDeclarationOperand::DynamicWord { .. } => {}
                }
            }
        }

        Self {
            local_declarations_by_scope_name,
            name_only_local_declarations_by_scope_name,
            initialized_scalar_local_declarations_by_scope_name,
            append_local_declaration_spans,
        }
    }

    fn local_declarations_for(&self, scope: ScopeId, name: &Name) -> &[Span] {
        self.local_declarations_by_scope_name
            .get(&(scope, name.clone()))
            .map_or(&[], Vec::as_slice)
    }

    fn name_only_local_declarations_for(&self, scope: ScopeId, name: &Name) -> &[Span] {
        self.name_only_local_declarations_by_scope_name
            .get(&(scope, name.clone()))
            .map_or(&[], Vec::as_slice)
    }

    fn initialized_scalar_local_declarations_for(&self, scope: ScopeId, name: &Name) -> &[Span] {
        self.initialized_scalar_local_declarations_by_scope_name
            .get(&(scope, name.clone()))
            .map_or(&[], Vec::as_slice)
    }

    fn is_local_append_declaration(&self, scope: ScopeId, name: &Name, span: Span) -> bool {
        self.append_local_declaration_spans.contains(&(
            scope,
            name.clone(),
            span.start.offset,
            span.end.offset,
        ))
    }
}

fn span_is_within(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn binding_is_array_like(binding: &Binding) -> bool {
    let declared_array = binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
    (declared_array && !is_uninitialized_local_array_declaration(binding))
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
}

fn binding_resets_indexed_array_type(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::ArithmeticAssignment
            | BindingKind::GetoptsTarget
            | BindingKind::Imported
            | BindingKind::LoopVariable
            | BindingKind::PrintfTarget
    ) || (matches!(binding.kind, BindingKind::ReadTarget)
        && !binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::Declaration(_))
            && !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
            && !binding
                .attributes
                .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC))
}

fn binding_is_initialized_scalar_declaration(binding: &Binding) -> bool {
    matches!(binding.kind, BindingKind::Declaration(_))
        && binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
        && !binding
            .attributes
            .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
}

fn binding_is_sticky_indexed_array(binding: &Binding) -> bool {
    !is_uninitialized_local_array_declaration(binding)
        && (binding.attributes.contains(BindingAttributes::ARRAY)
            || matches!(
                binding.kind,
                BindingKind::ArrayAssignment | BindingKind::MapfileTarget
            ))
}

fn is_uninitialized_local_array_declaration(binding: &Binding) -> bool {
    matches!(binding.kind, BindingKind::Declaration(DeclarationBuiltin::Local))
        && binding
            .attributes
            .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

fn loop_header_word_quote(facts: &LinterFacts<'_>, span: Span) -> Option<WordQuote> {
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

fn binding_suppresses_same_command_array_read(binding: &Binding, assignment_only: bool) -> bool {
    matches!(binding.kind, BindingKind::MapfileTarget)
        || (matches!(binding.kind, BindingKind::ReadTarget)
            && binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::ArrayAssignment) && assignment_only)
}

fn is_bash_runtime_array_name(name: &str) -> bool {
    matches!(
        name,
        "BASH_ALIASES"
            | "BASH_ARGC"
            | "BASH_ARGV"
            | "BASH_CMDS"
            | "BASH_LINENO"
            | "BASH_REMATCH"
            | "BASH_SOURCE"
            | "BASH_VERSINFO"
            | "COMP_WORDS"
            | "COMPREPLY"
            | "COPROC"
            | "DIRSTACK"
            | "FUNCNAME"
            | "GROUPS"
            | "MAPFILE"
            | "PIPESTATUS"
    )
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

fn collect_comma_array_assignment_spans(
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

fn collect_ifs_literal_backslash_assignment_value_spans(
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

fn comma_array_assignment_span(
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

fn array_value_has_unquoted_comma(
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

fn assignment_target_has_assoc_context(assignment: &Assignment, semantic: &SemanticModel) -> bool {
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

fn zsh_option_map_value_allows_comma(value: &shuck_ast::ArrayValueWord, source: &str) -> bool {
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

fn zsh_option_alias_part(part: &str) -> bool {
    part.starts_with('-')
        && part.len() > 1
        && part
            .bytes()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
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
