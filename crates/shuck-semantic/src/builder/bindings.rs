use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn visit_assignment_reads_into(
        &mut self,
        assignment: &'a Assignment,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        self.visit_var_ref_subscript_words(
            Some(&assignment.target.name),
            assignment.target.subscript.as_deref(),
            WordVisitKind::Expansion,
            flow,
            nested_regions,
        );
        self.visit_assignment_value_into(assignment, flow, nested_regions);
    }

    pub(super) fn visit_assignment_into(
        &mut self,
        assignment: &'a Assignment,
        declaration_kind: Option<(BindingKind, ScopeId)>,
        mut attributes: BindingAttributes,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let reference_start = self.references.len();
        self.visit_assignment_reads_into(assignment, flow, nested_regions);
        let zsh_scalar_subscript_assignment =
            self.assignment_target_uses_zsh_scalar_subscript(assignment);
        let explicit_array_declaration = declaration_kind.is_some()
            && attributes.intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
        let (kind, scope) = declaration_kind.unwrap_or_else(|| {
            let kind = if assignment.append {
                BindingKind::AppendAssignment
            } else if matches!(assignment.value, AssignmentValue::Compound(_))
                || (assignment.target.subscript.is_some() && !zsh_scalar_subscript_assignment)
            {
                BindingKind::ArrayAssignment
            } else {
                BindingKind::Assignment
            };
            (kind, self.current_scope())
        });
        attributes |= assignment_binding_attributes(assignment);
        if zsh_scalar_subscript_assignment && !explicit_array_declaration {
            attributes.remove(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
        }
        if assignment_has_empty_initializer(assignment, self.source) {
            attributes |= BindingAttributes::EMPTY_INITIALIZER;
        }
        let self_referential_refs =
            self.newly_added_reference_ids_reading_name(&assignment.target.name, reference_start);
        if !self_referential_refs.is_empty() {
            attributes |= BindingAttributes::SELF_REFERENTIAL_READ;
            self.self_referential_assignment_refs
                .extend(self_referential_refs);
        }
        if assignment.target.subscript.is_some()
            && !attributes.contains(BindingAttributes::ASSOC)
            && self
                .resolve_reference(
                    &assignment.target.name,
                    self.current_scope(),
                    assignment.target.name_span.start.offset,
                )
                .map(|binding_id| {
                    self.bindings[binding_id.index()]
                        .attributes
                        .contains(BindingAttributes::ASSOC)
                })
                .unwrap_or(false)
        {
            attributes |= BindingAttributes::ARRAY | BindingAttributes::ASSOC;
        }

        let binding = self.add_binding(
            &assignment.target.name,
            kind,
            scope,
            assignment.target.name_span,
            binding_origin_for_assignment(assignment, self.source),
            attributes,
        );
        self.record_prompt_assignment_references(assignment);
        if let Some(hint) = indirect_target_hint(assignment, self.source) {
            self.indirect_target_hints.insert(binding, hint);
        }
    }

    fn assignment_target_uses_zsh_scalar_subscript(&self, assignment: &Assignment) -> bool {
        if self.shell_profile.dialect != ShellDialect::Zsh {
            return false;
        }
        let Some(subscript) = assignment.target.subscript.as_deref() else {
            return false;
        };
        if subscript.selector().is_some() {
            return false;
        }
        self.resolve_reference(
            &assignment.target.name,
            self.current_scope(),
            assignment.target.name_span.start.offset,
        )
        .is_some_and(|binding_id| {
            !crate::binding::is_array_like_binding(&self.bindings[binding_id.index()])
        })
    }

    pub(super) fn record_prompt_assignment_references(&mut self, assignment: &'a Assignment) {
        let AssignmentValue::Scalar(word) = &assignment.value else {
            return;
        };

        match assignment.target.name.as_str() {
            "PS1" => {
                for (name, span) in prompt_assignment_reference_names(word, self.source) {
                    self.add_reference(&name, ReferenceKind::ImplicitRead, span);
                }
            }
            "PS4" => {
                for name in escaped_prompt_assignment_reference_names(word, self.source) {
                    self.add_reference(
                        &name,
                        ReferenceKind::PromptExpansion,
                        assignment.target.name_span,
                    );
                }
            }
            _ => {}
        }
    }

    pub(super) fn visit_assignment_value_into(
        &mut self,
        assignment: &'a Assignment,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
            }
            AssignmentValue::Compound(array) => {
                self.visit_array_expr_into(array, WordVisitKind::Expansion, flow, nested_regions);
            }
        }
    }

    pub(super) fn binding_was_cleared_in_scope_after(
        &self,
        name: &Name,
        scope: ScopeId,
        binding_offset: usize,
    ) -> bool {
        self.cleared_variables
            .get(&(scope, name.clone()))
            .is_some_and(|cleared_offsets| {
                cleared_offsets
                    .iter()
                    .any(|cleared_offset| *cleared_offset > binding_offset)
            })
    }

    pub(super) fn binding_was_cleared_in_scope_between(
        &self,
        name: &Name,
        scope: ScopeId,
        binding_offset: usize,
        lookup_offset: usize,
    ) -> bool {
        self.cleared_variables
            .get(&(scope, name.clone()))
            .is_some_and(|cleared_offsets| {
                cleared_offsets.iter().any(|cleared_offset| {
                    *cleared_offset > binding_offset && *cleared_offset < lookup_offset
                })
            })
    }

    pub(super) fn binding_was_cleared_before_lookup(
        &self,
        binding: &Binding,
        lookup_scope: ScopeId,
        lookup_offset: usize,
    ) -> bool {
        for scope in ancestor_scopes(&self.scopes, lookup_scope) {
            let clear_lower_bound = if scope == binding.scope {
                binding.span.start.offset
            } else {
                0
            };
            let clear_upper_bound = if self.completed_scopes.contains(&scope) {
                usize::MAX
            } else {
                lookup_offset
            };
            if self.binding_was_cleared_in_scope_between(
                &binding.name,
                scope,
                clear_lower_bound,
                clear_upper_bound,
            ) {
                return true;
            }
            if scope == binding.scope {
                break;
            }
        }
        false
    }

    pub(super) fn has_uncleared_local_binding_in_scope(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> bool {
        self.scopes[scope.index()]
            .bindings
            .get(name)
            .and_then(|bindings| {
                bindings.iter().rev().copied().find(|binding_id| {
                    let binding = &self.bindings[binding_id.index()];
                    binding.span.start.offset <= offset
                        && binding.attributes.contains(BindingAttributes::LOCAL)
                })
            })
            .is_some_and(|binding_id| {
                !self.binding_was_cleared_in_scope_after(
                    name,
                    scope,
                    self.bindings[binding_id.index()].span.start.offset,
                )
            })
    }

    pub(super) fn add_binding(
        &mut self,
        name: &Name,
        kind: BindingKind,
        scope: ScopeId,
        span: Span,
        origin: BindingOrigin,
        attributes: BindingAttributes,
    ) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            id,
            name: name.clone(),
            kind,
            origin,
            scope,
            span,
            references: Vec::new(),
            attributes,
        });
        self.binding_index.entry(name.clone()).or_default().push(id);
        match self.scopes[scope.index()].bindings.get_mut(name.as_str()) {
            Some(v) => v.push(id),
            None => {
                self.scopes[scope.index()]
                    .bindings
                    .insert(name.clone(), vec![id]);
            }
        }
        if matches!(kind, BindingKind::FunctionDefinition) {
            self.functions.entry(name.clone()).or_default().push(id);
        }
        if let Some(command) = self.command_stack.last().copied() {
            self.command_bindings
                .entry(SpanKey::new(command))
                .or_default()
                .push(id);
        }

        let binding = &self.bindings[id.index()];
        self.observer.record_binding(binding);
        id
    }
}
