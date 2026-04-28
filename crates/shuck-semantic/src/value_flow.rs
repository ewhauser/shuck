use std::cell::RefCell;

use super::*;

#[derive(Debug)]
pub struct SemanticValueFlow<'analysis, 'model> {
    analysis: &'analysis SemanticAnalysis<'model>,
    nonlocal_binding_memo: FxHashMap<(Name, ScopeId, SpanKey), Box<[BindingId]>>,
    nonlocal_binding_visiting: FxHashSet<(Name, ScopeId, SpanKey)>,
    named_function_call_sites_memo: RefCell<FxHashMap<ScopeId, Box<[CallSite]>>>,
    resolved_named_function_call_sites_memo: RefCell<FxHashMap<ScopeId, Box<[CallSite]>>>,
    function_definition_command_memo: RefCell<FxHashMap<ScopeId, Option<CommandId>>>,
}

impl<'model> SemanticAnalysis<'model> {
    pub fn value_flow(&self) -> SemanticValueFlow<'_, 'model> {
        SemanticValueFlow::new(self)
    }
}

impl<'analysis, 'model> SemanticValueFlow<'analysis, 'model> {
    fn new(analysis: &'analysis SemanticAnalysis<'model>) -> Self {
        Self {
            analysis,
            nonlocal_binding_memo: FxHashMap::default(),
            nonlocal_binding_visiting: FxHashSet::default(),
            named_function_call_sites_memo: RefCell::new(FxHashMap::default()),
            resolved_named_function_call_sites_memo: RefCell::new(FxHashMap::default()),
            function_definition_command_memo: RefCell::new(FxHashMap::default()),
        }
    }

    pub fn reaching_value_bindings_for_name(&self, name: &Name, at: Span) -> Vec<BindingId> {
        self.reaching_value_bindings_for_name_inner(name, at, None)
    }

    pub fn reaching_value_bindings_for_name_with_synthetic_use_block(
        &self,
        name: &Name,
        at: Span,
        synthetic_use_block: Option<BlockId>,
    ) -> Vec<BindingId> {
        self.reaching_value_bindings_for_name_inner(name, at, synthetic_use_block)
    }

    fn reaching_value_bindings_for_name_inner(
        &self,
        name: &Name,
        at: Span,
        synthetic_use_block: Option<BlockId>,
    ) -> Vec<BindingId> {
        let mut bindings = self.analysis.reaching_bindings_for_name(name, at);
        self.retain_value_bindings(&mut bindings);
        if self.analysis.reference_id_for_name_at(name, at).is_none() {
            let synthetic_bindings =
                self.synthetic_reaching_value_bindings_for_name(name, at, synthetic_use_block);
            if !synthetic_bindings.is_empty() {
                bindings = synthetic_bindings;
            }
        }
        if bindings.is_empty()
            && let Some(binding_id) = self.latest_visible_value_binding_for_name(name, at)
        {
            bindings.push(binding_id);
        }
        self.sort_and_dedup_bindings(&mut bindings);
        bindings
    }

    pub fn reaching_value_bindings_bypassing(
        &self,
        name: &Name,
        bypass_binding: BindingId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut bindings = self
            .analysis
            .visible_bindings_bypassing(name, bypass_binding, at);
        self.retain_value_bindings(&mut bindings);
        self.sort_and_dedup_bindings(&mut bindings);
        bindings
    }

    pub fn ancestor_value_bindings_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let visible_scopes = self
            .model()
            .ancestor_scopes(scope)
            .collect::<FxHashSet<_>>();
        let mut bindings = self
            .model()
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|binding_id| {
                let binding = self.model().binding(*binding_id);
                visible_scopes.contains(&binding.scope)
                    && binding.span.end.offset <= at.start.offset
            })
            .collect::<Vec<_>>();
        self.retain_value_bindings(&mut bindings);
        self.sort_and_dedup_bindings(&mut bindings);
        bindings
    }

    pub fn helper_value_bindings_before(&mut self, name: &Name, at: Span) -> Vec<BindingId> {
        let mut bindings = self
            .model()
            .ancestor_scopes(self.model().scope_at(at.start.offset))
            .flat_map(|scope| {
                self.nonlocal_value_bindings_from_called_functions_before(name, scope, at)
            })
            .collect::<Vec<_>>();
        self.sort_and_dedup_bindings(&mut bindings);
        bindings
    }

    pub fn nonlocal_value_bindings_from_called_functions_before(
        &mut self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let caller_has_visible_local = self.scope_has_visible_local_binding_before(name, scope, at);

        let key = (name.clone(), scope, SpanKey::new(at));
        if let Some(cached) = self.nonlocal_binding_memo.get(&key) {
            return cached.to_vec();
        }
        if !self.nonlocal_binding_visiting.insert(key.clone()) {
            return Vec::new();
        }

        let mut bindings = self
            .nonlocal_bindings_from_functions_called_in_scope_before(name, scope, at)
            .into_iter()
            .collect::<FxHashSet<_>>();

        if !caller_has_visible_local
            && let Some(caller_bindings) = self.nonlocal_bindings_reaching_all_callers(name, scope)
        {
            bindings.extend(caller_bindings);
        }

        self.nonlocal_binding_visiting.remove(&key);
        let mut bindings = bindings.into_iter().collect::<Vec<_>>();
        self.retain_value_bindings(&mut bindings);
        self.sort_and_dedup_bindings(&mut bindings);
        self.nonlocal_binding_memo
            .insert(key, bindings.clone().into_boxed_slice());
        bindings
    }

    pub fn named_function_call_sites(&self, scope: ScopeId) -> Vec<CallSite> {
        if let Some(cached) = self.named_function_call_sites_memo.borrow().get(&scope) {
            return cached.to_vec();
        }

        let Some(function_kind) = self.named_function_kind(scope) else {
            return Vec::new();
        };

        let mut caller_sites = Vec::new();
        let mut seen_sites = FxHashSet::default();
        for function_name in function_kind.static_names() {
            for site in self.model().call_sites_for(function_name) {
                if site.scope == scope {
                    continue;
                }
                if seen_sites.insert((site.scope, site.span.start.offset, site.span.end.offset)) {
                    caller_sites.push(site.clone());
                }
            }
        }

        self.named_function_call_sites_memo
            .borrow_mut()
            .insert(scope, caller_sites.clone().into_boxed_slice());
        caller_sites
    }

    pub fn called_function_scopes_before(
        &self,
        scope: ScopeId,
        limit_offset: usize,
    ) -> Vec<ScopeId> {
        let mut scopes = Vec::new();
        let mut seen_scopes = FxHashSet::default();

        for (&function_binding, &callee_scope) in
            &self.model().recorded_program.function_body_scopes
        {
            let binding = self.model().binding(function_binding);
            let called_before = self
                .model()
                .call_sites_for(&binding.name)
                .iter()
                .any(|site| {
                    site.scope == scope
                        && self.call_site_dominates_offset(site.span, limit_offset)
                        && self.function_scope_resolves_at_call_site(
                            callee_scope,
                            &binding.name,
                            site,
                        )
                });
            if called_before && seen_scopes.insert(callee_scope) {
                scopes.push(callee_scope);
            }
        }

        scopes.sort_by_key(|scope| self.model().scope(*scope).span.start.offset);
        scopes
    }

    pub fn called_function_scopes_before_relaxed(
        &self,
        scope: ScopeId,
        limit_offset: usize,
    ) -> Vec<ScopeId> {
        let mut scopes = Vec::new();
        let mut seen_scopes = FxHashSet::default();

        for (&_function_binding, &callee_scope) in
            &self.model().recorded_program.function_body_scopes
        {
            let Some(function_kind) = self.named_function_kind(callee_scope) else {
                continue;
            };
            let Some(definition_command) = self.function_definition_command_for_scope(callee_scope)
            else {
                continue;
            };

            let called_before = function_kind.static_names().iter().any(|function_name| {
                self.model()
                    .call_sites_for(function_name)
                    .iter()
                    .any(|site| {
                        site.scope == scope
                            && site.span.start.offset < limit_offset
                            && self
                                .definition_command_resolves_at_call(definition_command, site.span)
                    })
            });
            if called_before && seen_scopes.insert(callee_scope) {
                scopes.push(callee_scope);
            }
        }

        scopes.sort_by_key(|callee_scope| self.model().scope(*callee_scope).span.start.offset);
        scopes
    }

    pub fn transitively_called_function_scopes_before(
        &self,
        scope: ScopeId,
        limit_offset: usize,
    ) -> Vec<ScopeId> {
        let mut seen_scopes = FxHashSet::default();
        let mut scopes = Vec::new();
        self.collect_transitively_called_function_scopes_before(
            scope,
            limit_offset,
            false,
            &mut seen_scopes,
            &mut scopes,
        );
        scopes
    }

    pub fn transitively_called_function_scopes_before_relaxed(
        &self,
        scope: ScopeId,
        limit_offset: usize,
    ) -> Vec<ScopeId> {
        let mut seen_scopes = FxHashSet::default();
        let mut scopes = Vec::new();
        self.collect_transitively_called_function_scopes_before(
            scope,
            limit_offset,
            true,
            &mut seen_scopes,
            &mut scopes,
        );
        scopes
    }

    pub fn value_bindings_cover_all_paths_to_span(
        &self,
        bindings: &[BindingId],
        target_span: Span,
    ) -> bool {
        let target_blocks = self
            .analysis
            .cfg()
            .blocks()
            .iter()
            .filter(|block| block.commands.contains(&target_span))
            .map(|block| block.id)
            .collect::<FxHashSet<_>>();
        if target_blocks.is_empty() {
            return true;
        }

        let cover_blocks = bindings
            .iter()
            .copied()
            .filter_map(|binding_id| self.analysis.block_for_binding(binding_id))
            .collect::<FxHashSet<_>>();
        if !cover_blocks.is_disjoint(&target_blocks) {
            return true;
        }

        let binding_scopes = bindings
            .iter()
            .copied()
            .map(|binding_id| self.model().binding(binding_id).scope)
            .collect::<Vec<_>>();
        let entry = self
            .analysis
            .flow_entry_block_for_binding_scopes(&binding_scopes, target_span.start.offset);
        target_blocks.iter().copied().all(|target_block| {
            self.analysis
                .blocks_cover_all_paths_to_block(entry, target_block, &cover_blocks)
        })
    }

    pub fn binding_visible_at(&self, binding_id: BindingId, at: Span) -> bool {
        self.model().binding_visible_at(binding_id, at)
    }

    pub fn binding_can_supply_parameter_value(&self, binding_id: BindingId) -> bool {
        let binding = self.model().binding(binding_id);
        match binding.origin {
            BindingOrigin::FunctionDefinition { .. } => false,
            BindingOrigin::Declaration { .. } => {
                binding_is_name_only_declaration(binding)
                    || binding.attributes.intersects(
                        BindingAttributes::DECLARATION_INITIALIZED | BindingAttributes::INTEGER,
                    )
            }
            _ => true,
        }
    }

    fn synthetic_reaching_value_bindings_for_name(
        &self,
        name: &Name,
        at: Span,
        synthetic_use_block: Option<BlockId>,
    ) -> Vec<BindingId> {
        let Some(reference_block) =
            synthetic_use_block.or_else(|| self.block_for_name_use_site(name, at))
        else {
            return Vec::new();
        };

        let unreachable = self.analysis.unreachable_blocks();
        let candidates = self
            .model()
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|binding_id| self.binding_can_supply_parameter_value(*binding_id))
            .filter(|binding_id| self.model().binding_visible_at(*binding_id, at))
            .filter_map(|binding_id| {
                let block_id = self.analysis.block_for_binding(binding_id)?;
                (!unreachable.contains(&block_id)).then_some((binding_id, block_id))
            })
            .collect::<Vec<_>>();

        let mut bindings = candidates
            .iter()
            .copied()
            .filter(|(binding_id, binding_block)| {
                !self.binding_is_shadowed_before_synthetic_use(
                    *binding_id,
                    *binding_block,
                    at,
                    &candidates,
                ) && self.binding_block_reaches_synthetic_use(
                    *binding_id,
                    *binding_block,
                    reference_block,
                    &candidates,
                    unreachable,
                )
            })
            .map(|(binding_id, _)| binding_id)
            .collect::<Vec<_>>();
        self.sort_and_dedup_bindings(&mut bindings);
        bindings
    }

    fn binding_is_shadowed_before_synthetic_use(
        &self,
        binding_id: BindingId,
        binding_block: BlockId,
        at: Span,
        candidates: &[(BindingId, BlockId)],
    ) -> bool {
        let binding = self.model().binding(binding_id);
        candidates.iter().any(|(other_id, other_block)| {
            *other_id != binding_id && *other_block == binding_block && {
                let other = self.model().binding(*other_id);
                other.span.start.offset > binding.span.start.offset
                    && other.span.start.offset < at.start.offset
            }
        })
    }

    fn binding_block_reaches_synthetic_use(
        &self,
        binding_id: BindingId,
        binding_block: BlockId,
        reference_block: BlockId,
        candidates: &[(BindingId, BlockId)],
        unreachable: &FxHashSet<BlockId>,
    ) -> bool {
        let blocked_blocks = candidates
            .iter()
            .copied()
            .filter(|(other_id, _)| *other_id != binding_id)
            .map(|(_, block_id)| block_id)
            .collect::<FxHashSet<_>>();
        let cfg = self.analysis.cfg();
        let mut stack = vec![binding_block];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if block_id != binding_block && blocked_blocks.contains(&block_id) {
                continue;
            }
            if block_id == reference_block {
                return true;
            }
            if unreachable.contains(&block_id) || !seen.insert(block_id) {
                continue;
            }
            for (successor, _) in cfg.successors(block_id) {
                stack.push(*successor);
            }
        }

        false
    }

    fn latest_visible_value_binding_for_name(&self, name: &Name, at: Span) -> Option<BindingId> {
        self.model()
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|binding_id| self.binding_can_supply_parameter_value(*binding_id))
            .filter(|binding_id| self.model().binding_visible_at(*binding_id, at))
            .max_by_key(|binding_id| self.model().binding(*binding_id).span.start.offset)
    }

    fn block_for_name_use_site(&self, name: &Name, at: Span) -> Option<BlockId> {
        if let Some(reference_id) = self.analysis.reference_id_for_name_at(name, at) {
            return self.analysis.block_for_reference_id(reference_id);
        }

        let command_id = self
            .model()
            .innermost_command_id_at(at.start.offset)
            .or_else(|| self.innermost_command_id_containing_offset(at.start.offset))?;
        self.analysis
            .block_ids_for_span(self.model().command_syntax_span(command_id))
            .first()
            .copied()
    }

    fn innermost_command_id_containing_offset(&self, offset: usize) -> Option<CommandId> {
        self.model()
            .commands()
            .iter()
            .copied()
            .filter(|command_id| {
                let span = self.model().command_syntax_span(*command_id);
                span.start.offset <= offset && offset <= span.end.offset
            })
            .max_by(|left, right| {
                let left_span = self.model().command_syntax_span(*left);
                let right_span = self.model().command_syntax_span(*right);
                left_span
                    .start
                    .offset
                    .cmp(&right_span.start.offset)
                    .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
            })
    }

    fn nonlocal_bindings_from_functions_called_in_scope_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut bindings = Vec::new();

        for callee_scope in self.called_function_scopes_providing_name(name) {
            let Some(function_kind) = self.named_function_kind(callee_scope) else {
                continue;
            };

            let called_before = function_kind.static_names().iter().any(|function_name| {
                self.model()
                    .call_sites_for(function_name)
                    .iter()
                    .any(|site| {
                        site.scope == scope
                            && self.function_scope_resolves_at_call_site(
                                callee_scope,
                                function_name,
                                site,
                            )
                            && self.call_site_dominates_offset(site.span, at.start.offset)
                    })
            });
            if !called_before {
                continue;
            }

            bindings.extend(self.nonlocal_bindings_for_name_in_scope(name, callee_scope));
        }

        bindings
    }

    fn nonlocal_bindings_reaching_all_callers(
        &mut self,
        name: &Name,
        scope: ScopeId,
    ) -> Option<FxHashSet<BindingId>> {
        let caller_sites = self.resolved_named_function_call_sites(scope);

        let mut saw_caller = false;
        let mut union = FxHashSet::default();
        for site in caller_sites {
            saw_caller = true;
            let branch = self
                .caller_value_bindings_before(name, site.scope, site.span)
                .into_iter()
                .collect::<FxHashSet<_>>();
            if branch.is_empty() {
                return Some(FxHashSet::default());
            }
            union.extend(branch);
        }

        saw_caller.then_some(union)
    }

    fn resolved_named_function_call_sites(&self, scope: ScopeId) -> Vec<CallSite> {
        if let Some(cached) = self
            .resolved_named_function_call_sites_memo
            .borrow()
            .get(&scope)
        {
            return cached.to_vec();
        }

        let Some(function_kind) = self.named_function_kind(scope) else {
            return Vec::new();
        };

        let mut caller_sites = Vec::new();
        let mut seen_sites = FxHashSet::default();
        for function_name in function_kind.static_names() {
            for site in self.model().call_sites_for(function_name) {
                if site.scope == scope {
                    continue;
                }
                if !self.function_scope_resolves_at_call_site(scope, function_name, site) {
                    continue;
                }
                if seen_sites.insert((site.scope, site.span.start.offset, site.span.end.offset)) {
                    caller_sites.push(site.clone());
                }
            }
        }

        self.resolved_named_function_call_sites_memo
            .borrow_mut()
            .insert(scope, caller_sites.clone().into_boxed_slice());
        caller_sites
    }

    pub fn caller_value_bindings_before(
        &mut self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut branch = self.reaching_value_bindings_for_name(name, at);
        branch.extend(self.ancestor_value_bindings_before(name, scope, at));
        branch.extend(self.nonlocal_value_bindings_from_called_functions_before(name, scope, at));
        if self.scope_has_visible_local_binding_before(name, scope, at) {
            branch.retain(|binding_id| self.model().binding(*binding_id).scope == scope);
        }
        self.sort_and_dedup_bindings(&mut branch);
        branch
    }

    fn called_function_scopes_providing_name(&self, name: &Name) -> Vec<ScopeId> {
        self.model()
            .bindings_for(name)
            .iter()
            .copied()
            .filter_map(|binding_id| {
                let binding = self.model().binding(binding_id);
                (!binding.attributes.contains(BindingAttributes::LOCAL)
                    && matches!(
                        self.model().scope(binding.scope).kind,
                        ScopeKind::Function(_)
                    ))
                .then_some(binding.scope)
            })
            .collect::<FxHashSet<_>>()
            .into_iter()
            .collect()
    }

    fn nonlocal_bindings_for_name_in_scope(
        &self,
        name: &Name,
        scope: ScopeId,
    ) -> impl Iterator<Item = BindingId> + '_ {
        self.model()
            .bindings_for(name)
            .iter()
            .copied()
            .filter(move |binding_id| {
                let binding = self.model().binding(*binding_id);
                binding.scope == scope && !binding.attributes.contains(BindingAttributes::LOCAL)
            })
    }

    fn named_function_kind(&self, scope: ScopeId) -> Option<&FunctionScopeKind> {
        match &self.model().scope(scope).kind {
            ScopeKind::Function(function) if !function.static_names().is_empty() => Some(function),
            ScopeKind::File
            | ScopeKind::Function(_)
            | ScopeKind::Subshell
            | ScopeKind::CommandSubstitution
            | ScopeKind::Pipeline => None,
        }
    }

    fn function_scope_resolves_at_call_site(
        &self,
        callee_scope: ScopeId,
        function_name: &Name,
        site: &CallSite,
    ) -> bool {
        if let Some(binding_id) = self
            .analysis
            .visible_function_binding_at_call(function_name, site.name_span)
        {
            return self.analysis.function_scope_for_binding(binding_id) == Some(callee_scope);
        }

        let Some(function_kind) = self.named_function_kind(callee_scope) else {
            return false;
        };
        if !function_kind.contains_name(function_name) {
            return false;
        }

        self.function_bindings_for_scope(callee_scope)
            .into_iter()
            .any(|binding_id| {
                self.function_binding_may_resolve_at_call(binding_id, function_name, site)
            })
    }

    fn function_binding_may_resolve_at_call(
        &self,
        binding_id: BindingId,
        function_name: &Name,
        site: &CallSite,
    ) -> bool {
        let Some(scope) = self.analysis.function_scope_for_binding(binding_id) else {
            return false;
        };
        let Some(function_kind) = self.named_function_kind(scope) else {
            return false;
        };
        if !function_kind.contains_name(function_name) {
            return false;
        }

        if let Some(definition_command) = self.function_definition_command_for_binding(binding_id)
            && self.definition_command_resolves_at_call(definition_command, site.span)
        {
            return true;
        }

        self.possible_function_bindings_cover_call(function_name, binding_id, site)
    }

    fn function_definition_command_for_binding(&self, binding_id: BindingId) -> Option<CommandId> {
        let binding_scope = self.analysis.function_scope_for_binding(binding_id)?;
        let binding_span = self.model().binding(binding_id).span;
        self.model().commands().iter().copied().find(|command_id| {
            let command_span = self.model().command_span(*command_id);
            span_contains(command_span, binding_span)
                && self
                    .model()
                    .function_definition_binding_for_command_span(command_span)
                    .and_then(|candidate| self.analysis.function_scope_for_binding(candidate))
                    == Some(binding_scope)
        })
    }

    fn function_definition_command_for_scope(&self, scope: ScopeId) -> Option<CommandId> {
        if let Some(cached) = self.function_definition_command_memo.borrow().get(&scope) {
            return *cached;
        }

        let command_id = self
            .function_bindings_for_scope(scope)
            .into_iter()
            .filter_map(|binding_id| self.function_definition_command_for_binding(binding_id))
            .min_by_key(|command_id| self.model().command_span(*command_id).start.offset);
        self.function_definition_command_memo
            .borrow_mut()
            .insert(scope, command_id);
        command_id
    }

    fn function_bindings_for_scope(&self, scope: ScopeId) -> Vec<BindingId> {
        self.model()
            .function_binding_scope_index()
            .get(&scope)
            .map(|bindings| bindings.iter().copied().collect())
            .unwrap_or_default()
    }

    fn possible_function_bindings_cover_call(
        &self,
        function_name: &Name,
        binding_id: BindingId,
        site: &CallSite,
    ) -> bool {
        let candidates = self
            .model()
            .function_definitions(function_name)
            .iter()
            .copied()
            .filter(|candidate| {
                self.function_definition_command_for_binding(*candidate)
                    .is_some_and(|command_id| {
                        self.definition_command_scope_can_reach_call(command_id, site.span)
                            && self.model().command_span(command_id).end.offset
                                <= site.span.start.offset
                    })
            })
            .collect::<Vec<_>>();
        candidates.len() > 1
            && candidates.contains(&binding_id)
            && self.value_bindings_cover_all_paths_to_span(&candidates, site.span)
    }

    fn definition_command_resolves_at_call(&self, command_id: CommandId, call_span: Span) -> bool {
        if !self.definition_command_is_visible_at_call(command_id, call_span) {
            return false;
        }

        let command_span = self.model().command_span(command_id);
        let definition_scope = self
            .analysis
            .enclosing_function_scope_at(command_span.start.offset);
        let call_scope = self
            .analysis
            .enclosing_function_scope_at(call_span.start.offset);

        if definition_scope.is_none() && call_scope.is_some() {
            return true;
        }

        command_span.end.offset <= call_span.start.offset
    }

    fn definition_command_scope_can_reach_call(
        &self,
        command_id: CommandId,
        call_span: Span,
    ) -> bool {
        let command_span = self.model().command_span(command_id);
        let command_scope = self
            .analysis
            .enclosing_function_scope_at(command_span.start.offset);
        let call_scope = self
            .analysis
            .enclosing_function_scope_at(call_span.start.offset);
        command_scope.is_none() || command_scope == call_scope
    }

    fn definition_command_is_visible_at_call(
        &self,
        command_id: CommandId,
        call_span: Span,
    ) -> bool {
        if !self.definition_command_scope_can_reach_call(command_id, call_span) {
            return false;
        }

        let mut parent_id = self.model().command_parent_id(command_id);
        while let Some(id) = parent_id {
            if self.command_is_dominance_barrier(id) {
                return false;
            }
            parent_id = self.model().command_parent_id(id);
        }

        true
    }

    fn call_site_dominates_offset(&self, call_span: Span, limit_offset: usize) -> bool {
        if call_span.start.offset >= limit_offset {
            return false;
        }

        let Some(mut command_id) = self
            .model()
            .innermost_command_id_at(call_span.start.offset)
            .or_else(|| self.innermost_command_id_containing_offset(call_span.start.offset))
        else {
            return true;
        };

        while self.model().command_syntax_span(command_id) != call_span {
            let Some(parent_id) = self.model().command_parent_id(command_id) else {
                return true;
            };
            command_id = parent_id;
        }

        let mut current = self.model().command_parent_id(command_id);
        while let Some(command_id) = current {
            let command_span = self.model().command_syntax_span(command_id);
            if command_span.end.offset > limit_offset {
                break;
            }
            if command_span.start.offset < call_span.start.offset
                && self.command_is_dominance_barrier(command_id)
            {
                return false;
            }
            current = self.model().command_parent_id(command_id);
        }

        true
    }

    fn collect_transitively_called_function_scopes_before(
        &self,
        scope: ScopeId,
        limit_offset: usize,
        relaxed: bool,
        seen_scopes: &mut FxHashSet<ScopeId>,
        scopes: &mut Vec<ScopeId>,
    ) {
        let callee_scopes = if relaxed {
            self.called_function_scopes_before_relaxed(scope, limit_offset)
        } else {
            self.called_function_scopes_before(scope, limit_offset)
        };

        for callee_scope in callee_scopes {
            if !seen_scopes.insert(callee_scope) {
                continue;
            }
            scopes.push(callee_scope);

            let next_limit_offset = self
                .function_definition_command_for_scope(callee_scope)
                .map(|command_id| self.model().command_span(command_id).end.offset)
                .unwrap_or_else(|| self.model().scope(callee_scope).span.end.offset);
            self.collect_transitively_called_function_scopes_before(
                callee_scope,
                next_limit_offset,
                relaxed,
                seen_scopes,
                scopes,
            );
        }
    }

    fn command_is_dominance_barrier(&self, command_id: CommandId) -> bool {
        match self.model().command_kind(command_id) {
            CommandKind::Binary => true,
            CommandKind::Compound(kind) => !matches!(
                kind,
                CompoundCommandKind::BraceGroup
                    | CompoundCommandKind::Arithmetic
                    | CompoundCommandKind::Time
            ),
            CommandKind::Simple
            | CommandKind::Builtin(_)
            | CommandKind::Decl
            | CommandKind::Function
            | CommandKind::AnonymousFunction => false,
        }
    }

    fn scope_has_visible_local_binding_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> bool {
        self.model()
            .bindings_for(name)
            .iter()
            .copied()
            .any(|binding_id| {
                let binding = self.model().binding(binding_id);
                binding.scope == scope
                    && binding.span.end.offset <= at.start.offset
                    && binding.attributes.contains(BindingAttributes::LOCAL)
            })
    }

    fn retain_value_bindings(&self, bindings: &mut Vec<BindingId>) {
        bindings.retain(|binding_id| self.binding_can_supply_parameter_value(*binding_id));
    }

    fn sort_and_dedup_bindings(&self, bindings: &mut Vec<BindingId>) {
        bindings.sort_by_key(|binding_id| self.model().binding(*binding_id).span.start.offset);
        bindings.dedup();
    }

    fn model(&self) -> &'model SemanticModel {
        self.analysis.model
    }
}

fn binding_is_name_only_declaration(binding: &Binding) -> bool {
    matches!(binding.origin, BindingOrigin::Declaration { .. })
        && binding.attributes.contains(BindingAttributes::LOCAL)
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}
