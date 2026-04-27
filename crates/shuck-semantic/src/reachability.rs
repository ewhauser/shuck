use super::*;

struct OverwriteWindow<'a> {
    first: BindingId,
    first_blocks: &'a [BlockId],
    second_blocks: &'a [BlockId],
    cfg: &'a ControlFlowGraph,
    unreachable: &'a FxHashSet<BlockId>,
}

struct FunctionReachWindow<'a> {
    binding: BindingId,
    binding_blocks: &'a [BlockId],
    shadow_blocks: &'a FxHashSet<BlockId>,
    cfg: &'a ControlFlowGraph,
    unreachable: &'a FxHashSet<BlockId>,
    script_terminators: &'a FxHashSet<BlockId>,
}

#[allow(missing_docs)]
impl<'model> SemanticAnalysis<'model> {
    pub fn overwritten_functions(&self) -> &[OverwrittenFunction] {
        self.overwritten_functions
            .get_or_init(|| self.compute_overwritten_functions())
            .as_slice()
    }

    pub fn unreached_functions(&self) -> &[UnreachedFunction] {
        self.unreached_functions
            .get_or_init(|| self.compute_unreached_functions_with_options(Default::default()))
            .as_slice()
    }

    pub fn unreached_functions_with_options(
        &self,
        options: UnreachedFunctionAnalysisOptions,
    ) -> &[UnreachedFunction] {
        if options == UnreachedFunctionAnalysisOptions::default() {
            return self.unreached_functions();
        }

        self.unreached_functions_shellcheck_compat
            .get_or_init(|| self.compute_unreached_functions_with_options(options))
            .as_slice()
    }

    fn overwrite_call_site_resolves_to_binding(
        &self,
        name: &Name,
        site: &CallSite,
        binding_id: BindingId,
    ) -> bool {
        if let Some(binding) = self.model.visible_binding(name, site.span) {
            return binding.id == binding_id;
        }

        let binding = self.model.binding(binding_id);
        if site.scope == binding.scope {
            return false;
        }

        let mut ancestors = self.model.ancestor_scopes(site.scope);
        let Some(site_scope) = ancestors.next() else {
            return false;
        };
        debug_assert_eq!(site_scope, site.scope);

        for scope in ancestors {
            if scope == binding.scope {
                return true;
            }

            if self.model.scopes[scope.index()].bindings.contains_key(name) {
                return false;
            }
        }

        false
    }

    fn reachable_call_site_blocks(
        &self,
        window: &OverwriteWindow<'_>,
        site: &CallSite,
    ) -> Vec<BlockId> {
        self.reachable_call_site_blocks_in_cfg(window.cfg, site, window.unreachable)
    }

    fn reachable_call_site_blocks_in_cfg(
        &self,
        cfg: &ControlFlowGraph,
        site: &CallSite,
        unreachable: &FxHashSet<BlockId>,
    ) -> Vec<BlockId> {
        let command_span = self
            .model
            .recorded_program
            .call_command_spans
            .get(&SpanKey::new(site.span))
            .copied()
            .unwrap_or(site.span);
        cfg.block_ids_for_span(command_span)
            .iter()
            .copied()
            .filter(|block| !unreachable.contains(block))
            .collect()
    }

    fn nested_call_site_is_viable(
        &self,
        scope: ScopeId,
        site_blocks: &[BlockId],
        window: &OverwriteWindow<'_>,
        reachability: &mut ReachabilityCache<'_>,
    ) -> bool {
        let Some(&scope_entry) = window.cfg.scope_entries.get(&scope) else {
            return false;
        };
        let Some(scope_exits) = window.cfg.scope_exits(scope) else {
            return false;
        };

        blocks_have_path(&[scope_entry], site_blocks, reachability)
            && blocks_have_path(site_blocks, scope_exits, reachability)
    }

    fn call_site_executes_between_overwrite(
        &self,
        site: &CallSite,
        window: &OverwriteWindow<'_>,
        reachability: &mut ReachabilityCache<'_>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let site_blocks = self.reachable_call_site_blocks(window, site);
        if site_blocks.is_empty() {
            return false;
        }

        let first_binding = self.model.binding(window.first);
        if site.scope == first_binding.scope {
            return blocks_have_path(window.first_blocks, &site_blocks, reachability)
                && blocks_have_path(&site_blocks, window.second_blocks, reachability);
        }

        if !matches!(self.model.scope_kind(site.scope), ScopeKind::Function(_)) {
            return blocks_have_path(window.first_blocks, &site_blocks, reachability)
                && blocks_have_path(&site_blocks, window.second_blocks, reachability);
        }

        if !self.nested_call_site_is_viable(site.scope, &site_blocks, window, reachability) {
            return false;
        }

        if !visiting_scopes.insert(site.scope) {
            return false;
        }

        let executed = self
            .model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(binding_id, body_scope)| {
                (*body_scope == site.scope).then_some(*binding_id)
            })
            .any(|function_binding| {
                let function_name = self.model.binding(function_binding).name.clone();
                self.model
                    .call_sites_for(&function_name)
                    .iter()
                    .any(|caller| {
                        self.overwrite_call_site_resolves_to_binding(
                            &function_name,
                            caller,
                            function_binding,
                        ) && self.call_site_executes_between_overwrite(
                            caller,
                            window,
                            reachability,
                            visiting_scopes,
                        )
                    })
            });

        visiting_scopes.remove(&site.scope);
        executed
    }

    fn compute_unreached_functions_with_options(
        &self,
        options: UnreachedFunctionAnalysisOptions,
    ) -> Vec<UnreachedFunction> {
        if self.model.functions.is_empty() {
            return Vec::new();
        }

        let function_count = self
            .model
            .functions
            .values()
            .map(|bindings| bindings.len())
            .sum::<usize>();

        let cfg = self.cfg();
        let skip_termination_reachability = function_count
            > MAX_FUNCTIONS_FOR_TERMINATION_REACHABILITY
            || function_count.saturating_mul(cfg.blocks().len())
                > MAX_TERMINATION_REACHABILITY_WORK;
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let script_terminators = cfg
            .script_terminators()
            .iter()
            .copied()
            .collect::<FxHashSet<_>>();
        let has_termination_boundary = !unreachable.is_empty() || !script_terminators.is_empty();
        if !has_termination_boundary && !options.report_unreached_nested_definitions {
            return Vec::new();
        }

        let binding_blocks = build_binding_block_index(cfg.blocks(), self.model.bindings.len());
        let natural_exits = cfg
            .natural_exits()
            .iter()
            .copied()
            .collect::<FxHashSet<_>>();
        let mut unreached = Vec::new();
        let mut empty_shadow_termination_cache = FxHashMap::default();
        let mut scope_execution_cache = FxHashMap::default();
        let mut nested_scope_execution_cache = FxHashMap::default();
        let mut reported_bindings = FxHashSet::default();

        for (name, bindings) in &self.model.functions {
            for &binding_id in bindings {
                let binding = self.model.binding(binding_id);
                if !matches!(binding.kind, BindingKind::FunctionDefinition) {
                    continue;
                }

                let reachable_blocks =
                    reachable_binding_blocks(binding_id, &binding_blocks, &unreachable);

                if has_termination_boundary {
                    match reachable_blocks.as_deref() {
                        None => {
                            if self.binding_execution_scope_can_run_before_termination(
                                binding_id,
                                cfg,
                                &unreachable,
                                &script_terminators,
                                &mut scope_execution_cache,
                            ) {
                                reported_bindings.insert(binding_id);
                                unreached.push(UnreachedFunction {
                                    name: name.clone(),
                                    binding: binding_id,
                                    reason: UnreachedFunctionReason::UnreachableDefinition,
                                });
                            }
                        }
                        Some(reachable_blocks) => {
                            if !skip_termination_reachability
                                && self.binding_execution_scope_can_run_before_termination(
                                    binding_id,
                                    cfg,
                                    &unreachable,
                                    &script_terminators,
                                    &mut scope_execution_cache,
                                )
                            {
                                let shadow_blocks = self.shadow_function_blocks(
                                    name,
                                    binding_id,
                                    &binding_blocks,
                                    &unreachable,
                                );
                                let window = FunctionReachWindow {
                                    binding: binding_id,
                                    binding_blocks: reachable_blocks,
                                    shadow_blocks: &shadow_blocks,
                                    cfg,
                                    unreachable: &unreachable,
                                    script_terminators: &script_terminators,
                                };
                                let mut visiting_scopes = FxHashSet::default();
                                let has_direct_call = self
                                    .function_binding_has_direct_call_before_termination(
                                        name,
                                        &window,
                                        &mut visiting_scopes,
                                    );

                                if !has_direct_call
                                    && !script_terminators.is_empty()
                                    && all_paths_terminate_before_natural_exit(
                                        reachable_blocks,
                                        cfg,
                                        &script_terminators,
                                        &natural_exits,
                                        &unreachable,
                                        &shadow_blocks,
                                        &mut empty_shadow_termination_cache,
                                    )
                                {
                                    reported_bindings.insert(binding_id);
                                    unreached.push(UnreachedFunction {
                                        name: name.clone(),
                                        binding: binding_id,
                                        reason: UnreachedFunctionReason::ScriptTerminates,
                                    });
                                }
                            }
                        }
                    }
                }

                if options.report_unreached_nested_definitions
                    && !reported_bindings.contains(&binding_id)
                    && self.nested_function_definition_is_unreached(
                        name,
                        binding_id,
                        cfg,
                        &binding_blocks,
                        &unreachable,
                        &mut nested_scope_execution_cache,
                    )
                {
                    reported_bindings.insert(binding_id);
                    unreached.push(UnreachedFunction {
                        name: name.clone(),
                        binding: binding_id,
                        reason: UnreachedFunctionReason::EnclosingFunctionUnreached,
                    });
                }
            }
        }

        unreached.sort_by_key(|unreached| self.model.binding(unreached.binding).span.start.offset);
        unreached
    }

    fn nested_function_definition_is_unreached(
        &self,
        name: &Name,
        binding_id: BindingId,
        cfg: &ControlFlowGraph,
        binding_blocks: &[Vec<BlockId>],
        unreachable: &FxHashSet<BlockId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        let binding = self.model.binding(binding_id);
        let Some(enclosing_scope) = self.enclosing_function_or_transient_scope(binding.scope)
        else {
            return false;
        };

        let empty_terminators = FxHashSet::default();
        if matches!(
            self.model.scope_kind(enclosing_scope),
            ScopeKind::Function(function) if !function.is_anonymous()
        ) && self.binding_execution_scope_can_run_persistently_before_termination(
            binding_id,
            cfg,
            unreachable,
            &empty_terminators,
            scope_execution_cache,
        ) {
            return false;
        }

        let Some(reachable_blocks) =
            reachable_binding_blocks(binding_id, binding_blocks, unreachable)
        else {
            return true;
        };
        let shadow_blocks =
            self.shadow_function_blocks(name, binding_id, binding_blocks, unreachable);
        let window = FunctionReachWindow {
            binding: binding_id,
            binding_blocks: &reachable_blocks,
            shadow_blocks: &shadow_blocks,
            cfg,
            unreachable,
            script_terminators: &empty_terminators,
        };
        let mut visiting_scopes = FxHashSet::default();

        !self.function_binding_has_direct_call_before_termination(
            name,
            &window,
            &mut visiting_scopes,
        )
    }

    fn binding_execution_scope_can_run_before_termination(
        &self,
        binding_id: BindingId,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        let binding = self.model.binding(binding_id);
        let Some(function_scope) = self.enclosing_function_scope(binding.scope) else {
            return true;
        };

        let mut visiting_scopes = FxHashSet::default();
        self.function_scope_can_run_before_termination(
            function_scope,
            cfg,
            unreachable,
            script_terminators,
            &mut visiting_scopes,
            scope_execution_cache,
        )
    }

    fn binding_execution_scope_can_run_persistently_before_termination(
        &self,
        binding_id: BindingId,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        let binding = self.model.binding(binding_id);
        let Some(function_scope) = self.enclosing_function_scope(binding.scope) else {
            return true;
        };

        let mut visiting_scopes = FxHashSet::default();
        self.function_scope_can_run_persistently_before_termination(
            function_scope,
            cfg,
            unreachable,
            script_terminators,
            &mut visiting_scopes,
            scope_execution_cache,
        )
    }

    fn function_scope_can_run_before_termination(
        &self,
        function_scope: ScopeId,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        if !visiting_scopes.contains(&function_scope)
            && let Some(cached) = scope_execution_cache.get(&function_scope)
        {
            return *cached;
        }
        if !visiting_scopes.insert(function_scope) {
            return false;
        }

        let can_run = self
            .model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(binding, body_scope)| (*body_scope == function_scope).then_some(*binding))
            .any(|function_binding| {
                let function = self.model.binding(function_binding);
                self.model
                    .call_sites_for(&function.name)
                    .iter()
                    .any(|site| {
                        self.call_site_can_resolve_to_binding_before_termination(
                            &function.name,
                            site,
                            function_binding,
                            cfg,
                            unreachable,
                            script_terminators,
                        ) && self.call_site_can_execute_after_function_definition(
                            site,
                            function.span.start.offset,
                        ) && self.call_site_context_can_run_before_termination(
                            site,
                            cfg,
                            unreachable,
                            script_terminators,
                            visiting_scopes,
                            scope_execution_cache,
                        )
                    })
            });

        visiting_scopes.remove(&function_scope);
        scope_execution_cache.insert(function_scope, can_run);
        can_run
    }

    fn function_scope_can_run_persistently_before_termination(
        &self,
        function_scope: ScopeId,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        if !visiting_scopes.contains(&function_scope)
            && let Some(cached) = scope_execution_cache.get(&function_scope)
        {
            return *cached;
        }
        if !visiting_scopes.insert(function_scope) {
            return false;
        }

        let can_run = self
            .model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(binding, body_scope)| (*body_scope == function_scope).then_some(*binding))
            .any(|function_binding| {
                let function = self.model.binding(function_binding);
                self.model
                    .call_sites_for(&function.name)
                    .iter()
                    .any(|site| {
                        self.call_site_can_resolve_to_binding_before_termination(
                            &function.name,
                            site,
                            function_binding,
                            cfg,
                            unreachable,
                            script_terminators,
                        ) && self.call_site_can_execute_after_function_definition(
                            site,
                            function.span.start.offset,
                        ) && !self.call_site_runs_in_transient_context(site.scope)
                            && self.call_site_context_can_run_persistently_before_termination(
                                site,
                                cfg,
                                unreachable,
                                script_terminators,
                                visiting_scopes,
                                scope_execution_cache,
                            )
                    })
            });

        visiting_scopes.remove(&function_scope);
        scope_execution_cache.insert(function_scope, can_run);
        can_run
    }

    fn call_site_can_execute_after_function_definition(
        &self,
        site: &CallSite,
        function_offset: usize,
    ) -> bool {
        site.span.start.offset > function_offset || {
            let mut visiting_scopes = FxHashSet::default();
            self.call_site_context_can_run_after_offset(site, function_offset, &mut visiting_scopes)
        }
    }

    fn call_site_context_can_run_after_offset(
        &self,
        site: &CallSite,
        after_offset: usize,
        visiting_scopes: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let Some(function_scope) = self.enclosing_function_scope(site.scope) else {
            return site.span.start.offset > after_offset;
        };
        if !visiting_scopes.insert(function_scope) {
            return false;
        }

        let can_run = self
            .model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(binding_id, body_scope)| {
                (*body_scope == function_scope).then_some(*binding_id)
            })
            .any(|function_binding| {
                let function = self.model.binding(function_binding);
                self.model
                    .call_sites_for(&function.name)
                    .iter()
                    .any(|caller| {
                        self.overwrite_call_site_resolves_to_binding(
                            &function.name,
                            caller,
                            function_binding,
                        ) && (caller.span.start.offset > after_offset
                            || self.call_site_context_can_run_after_offset(
                                caller,
                                after_offset,
                                visiting_scopes,
                            ))
                    })
            });

        visiting_scopes.remove(&function_scope);
        can_run
    }

    fn call_site_context_can_run_before_termination(
        &self,
        site: &CallSite,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        let site_blocks = self.reachable_call_site_blocks_in_cfg(cfg, site, unreachable);
        if site_blocks.is_empty() {
            return false;
        }

        let Some(function_scope) = self.enclosing_function_scope(site.scope) else {
            return blocks_have_path_avoiding(
                cfg,
                &[cfg.entry()],
                &site_blocks,
                script_terminators,
            );
        };
        let Some(scope_entry) = cfg.scope_entry(function_scope) else {
            return false;
        };
        if !blocks_have_path_avoiding(cfg, &[scope_entry], &site_blocks, script_terminators) {
            return false;
        }

        self.function_scope_can_run_before_termination(
            function_scope,
            cfg,
            unreachable,
            script_terminators,
            visiting_scopes,
            scope_execution_cache,
        )
    }

    fn call_site_context_can_run_persistently_before_termination(
        &self,
        site: &CallSite,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
        scope_execution_cache: &mut FxHashMap<ScopeId, bool>,
    ) -> bool {
        let site_blocks = self.reachable_call_site_blocks_in_cfg(cfg, site, unreachable);
        if site_blocks.is_empty() {
            return false;
        }

        let Some(function_scope) = self.enclosing_function_scope(site.scope) else {
            return blocks_have_path_avoiding(
                cfg,
                &[cfg.entry()],
                &site_blocks,
                script_terminators,
            );
        };
        let Some(scope_entry) = cfg.scope_entry(function_scope) else {
            return false;
        };
        if !blocks_have_path_avoiding(cfg, &[scope_entry], &site_blocks, script_terminators) {
            return false;
        }

        self.function_scope_can_run_persistently_before_termination(
            function_scope,
            cfg,
            unreachable,
            script_terminators,
            visiting_scopes,
            scope_execution_cache,
        )
    }

    pub(crate) fn call_site_runs_in_transient_context(&self, scope: ScopeId) -> bool {
        self.model.ancestor_scopes(scope).any(|scope_id| {
            matches!(
                self.model.scope_kind(scope_id),
                ScopeKind::Subshell | ScopeKind::CommandSubstitution | ScopeKind::Pipeline
            )
        })
    }

    fn function_binding_has_direct_call_before_termination(
        &self,
        name: &Name,
        window: &FunctionReachWindow<'_>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
    ) -> bool {
        self.model.call_sites_for(name).iter().any(|site| {
            self.call_site_can_resolve_to_binding_on_reachable_path(name, site, window)
                && self.call_site_executes_before_termination(site, window, visiting_scopes)
        })
    }

    fn call_site_can_resolve_to_binding_before_termination(
        &self,
        name: &Name,
        site: &CallSite,
        binding_id: BindingId,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
        script_terminators: &FxHashSet<BlockId>,
    ) -> bool {
        let binding = self.model.binding(binding_id);
        let site_blocks = self.reachable_call_site_blocks_in_cfg(cfg, site, unreachable);
        if site_blocks.is_empty() {
            return false;
        }

        if let Some(visible) = self.model.visible_binding(name, site.span)
            && visible.id != binding_id
            && visible.scope != binding.scope
        {
            return false;
        }

        if site.scope == binding.scope
            || self
                .model
                .ancestor_scopes(site.scope)
                .any(|scope| scope == binding.scope)
        {
            let binding_blocks = reachable_blocks_for_binding(cfg, binding_id, unreachable);
            if binding_blocks.is_empty() {
                return false;
            }
            let shadow_blocks =
                self.shadow_function_blocks_from_cfg(name, binding_id, cfg, unreachable);
            return blocks_have_path_avoiding_many(
                cfg,
                &binding_blocks,
                &site_blocks,
                &shadow_blocks,
                script_terminators,
            );
        }

        self.overwrite_call_site_resolves_to_binding(name, site, binding_id)
    }

    fn call_site_can_resolve_to_binding_on_reachable_path(
        &self,
        name: &Name,
        site: &CallSite,
        window: &FunctionReachWindow<'_>,
    ) -> bool {
        if self.call_site_has_prior_shadowing_function_definition(name, site, window.binding) {
            return false;
        }

        let binding = self.model.binding(window.binding);
        if site.scope == binding.scope {
            let site_blocks =
                self.reachable_call_site_blocks_in_cfg(window.cfg, site, window.unreachable);
            return !site_blocks.is_empty()
                && blocks_have_path_avoiding_many(
                    window.cfg,
                    window.binding_blocks,
                    &site_blocks,
                    window.shadow_blocks,
                    window.script_terminators,
                );
        }

        if self.overwrite_call_site_resolves_to_binding(name, site, window.binding) {
            return true;
        }

        if self.call_site_can_use_function_binding_provided_earlier(site, window) {
            return true;
        }

        if !self
            .model
            .ancestor_scopes(site.scope)
            .any(|scope| scope == binding.scope)
        {
            return false;
        }
        let Some(function_scope) = self.enclosing_function_scope(site.scope) else {
            return false;
        };

        self.model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(function_binding, body_scope)| {
                (*body_scope == function_scope).then_some(*function_binding)
            })
            .any(|function_binding| {
                !reachable_blocks_for_binding(window.cfg, function_binding, window.unreachable)
                    .is_empty()
            })
    }

    fn call_site_has_prior_shadowing_function_definition(
        &self,
        name: &Name,
        site: &CallSite,
        binding_id: BindingId,
    ) -> bool {
        let binding = self.model.binding(binding_id);
        for scope in self.model.ancestor_scopes(site.scope) {
            if scope == binding.scope {
                return false;
            }

            if self.model.scopes[scope.index()]
                .bindings
                .get(name)
                .into_iter()
                .flat_map(|bindings| bindings.iter())
                .any(|other| {
                    *other != binding_id
                        && matches!(
                            self.model.binding(*other).kind,
                            BindingKind::FunctionDefinition
                        )
                        && self.model.binding(*other).span.start.offset < site.span.start.offset
                })
            {
                return true;
            }
        }

        false
    }

    fn call_site_can_use_function_binding_provided_earlier(
        &self,
        site: &CallSite,
        window: &FunctionReachWindow<'_>,
    ) -> bool {
        let binding = self.model.binding(window.binding);
        let Some(provider_scope) = self.enclosing_function_scope(binding.scope) else {
            return false;
        };
        let consumer_execution_scope = self.call_site_execution_scope(site.scope);
        if self.function_scope_is_called_before_offset(
            consumer_execution_scope,
            binding.span.start.offset,
            window,
        ) {
            return false;
        }
        let site_blocks =
            self.reachable_call_site_blocks_in_cfg(window.cfg, site, window.unreachable);
        if site_blocks.is_empty() {
            return false;
        }

        self.model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(function_binding, body_scope)| {
                (*body_scope == provider_scope).then_some(*function_binding)
            })
            .any(|provider_binding| {
                let provider = self.model.binding(provider_binding);
                self.model
                    .call_sites_for(&provider.name)
                    .iter()
                    .any(|provider_site| {
                        provider_site.span.start.offset < site.span.start.offset
                            && self.call_site_execution_scope(provider_site.scope)
                                == consumer_execution_scope
                            && self.overwrite_call_site_resolves_to_binding(
                                &provider.name,
                                provider_site,
                                provider_binding,
                            )
                            && {
                                let provider_blocks = self.reachable_call_site_blocks_in_cfg(
                                    window.cfg,
                                    provider_site,
                                    window.unreachable,
                                );
                                !provider_blocks.is_empty()
                                    && blocks_have_path_avoiding_many(
                                        window.cfg,
                                        &provider_blocks,
                                        &site_blocks,
                                        window.shadow_blocks,
                                        window.script_terminators,
                                    )
                            }
                    })
            })
    }

    fn function_scope_is_called_before_offset(
        &self,
        scope: ScopeId,
        offset: usize,
        window: &FunctionReachWindow<'_>,
    ) -> bool {
        self.model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(function_binding, body_scope)| {
                (*body_scope == scope).then_some(*function_binding)
            })
            .any(|function_binding| {
                let function = self.model.binding(function_binding);
                self.model
                    .call_sites_for(&function.name)
                    .iter()
                    .any(|site| {
                        site.span.start.offset < offset
                            && self.overwrite_call_site_resolves_to_binding(
                                &function.name,
                                site,
                                function_binding,
                            )
                            && !self
                                .reachable_call_site_blocks_in_cfg(
                                    window.cfg,
                                    site,
                                    window.unreachable,
                                )
                                .is_empty()
                    })
            })
    }

    fn call_site_execution_scope(&self, scope: ScopeId) -> ScopeId {
        self.enclosing_function_scope(scope).unwrap_or(scope)
    }

    fn call_site_executes_before_termination(
        &self,
        site: &CallSite,
        window: &FunctionReachWindow<'_>,
        visiting_scopes: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let site_blocks =
            self.reachable_call_site_blocks_in_cfg(window.cfg, site, window.unreachable);
        if site_blocks.is_empty() {
            return false;
        }

        if self.call_site_can_use_function_binding_provided_earlier(site, window) {
            return true;
        }

        let binding = self.model.binding(window.binding);
        if site.scope == binding.scope {
            return blocks_have_path_avoiding_many(
                window.cfg,
                window.binding_blocks,
                &site_blocks,
                window.shadow_blocks,
                window.script_terminators,
            );
        }

        let Some(function_scope) = self.enclosing_function_scope(site.scope) else {
            return blocks_have_path_avoiding_many(
                window.cfg,
                window.binding_blocks,
                &site_blocks,
                window.shadow_blocks,
                window.script_terminators,
            );
        };
        if function_scope == binding.scope {
            return blocks_have_path_avoiding_many(
                window.cfg,
                window.binding_blocks,
                &site_blocks,
                window.shadow_blocks,
                window.script_terminators,
            );
        }

        let Some(scope_entry) = window.cfg.scope_entry(function_scope) else {
            return false;
        };
        if !blocks_have_path_avoiding(
            window.cfg,
            &[scope_entry],
            &site_blocks,
            window.script_terminators,
        ) {
            return false;
        }

        if !visiting_scopes.insert(function_scope) {
            return false;
        }

        let executed = self
            .model
            .recorded_program
            .function_body_scopes
            .iter()
            .filter_map(|(binding_id, body_scope)| {
                (*body_scope == function_scope).then_some(*binding_id)
            })
            .any(|function_binding| {
                let function = self.model.binding(function_binding);
                let function_name = function.name.clone();
                let function_offset = function.span.start.offset;
                self.model
                    .call_sites_for(&function_name)
                    .iter()
                    .any(|caller| {
                        self.call_site_can_resolve_to_binding_before_termination(
                            &function_name,
                            caller,
                            function_binding,
                            window.cfg,
                            window.unreachable,
                            window.script_terminators,
                        ) && self.call_site_can_execute_after_function_definition(
                            caller,
                            function_offset,
                        ) && self.call_site_executes_before_termination(
                            caller,
                            window,
                            visiting_scopes,
                        )
                    })
            });

        visiting_scopes.remove(&function_scope);
        executed
    }

    fn shadow_function_blocks(
        &self,
        name: &Name,
        binding_id: BindingId,
        binding_blocks: &[Vec<BlockId>],
        unreachable: &FxHashSet<BlockId>,
    ) -> FxHashSet<BlockId> {
        let binding = self.model.binding(binding_id);
        self.model
            .function_definitions(name)
            .iter()
            .copied()
            .filter(|other| *other != binding_id)
            .filter(|other| {
                let other_binding = self.model.binding(*other);
                other_binding.scope == binding.scope
                    && other_binding.span.start.offset > binding.span.start.offset
            })
            .flat_map(|other| {
                binding_blocks
                    .get(other.index())
                    .into_iter()
                    .flat_map(|blocks| blocks.iter())
                    .copied()
                    .filter(|block| !unreachable.contains(block))
            })
            .collect()
    }

    fn shadow_function_blocks_from_cfg(
        &self,
        name: &Name,
        binding_id: BindingId,
        cfg: &ControlFlowGraph,
        unreachable: &FxHashSet<BlockId>,
    ) -> FxHashSet<BlockId> {
        let binding = self.model.binding(binding_id);
        self.model
            .function_definitions(name)
            .iter()
            .copied()
            .filter(|other| *other != binding_id)
            .filter(|other| {
                let other_binding = self.model.binding(*other);
                other_binding.scope == binding.scope
                    && other_binding.span.start.offset > binding.span.start.offset
            })
            .flat_map(|other| reachable_blocks_for_binding(cfg, other, unreachable))
            .collect()
    }

    fn enclosing_function_scope(&self, scope: ScopeId) -> Option<ScopeId> {
        self.model.ancestor_scopes(scope).find(|scope_id| {
            matches!(
                self.model.scope_kind(*scope_id),
                ScopeKind::Function(function) if !function.is_anonymous()
            )
        })
    }

    fn enclosing_function_or_transient_scope(&self, scope: ScopeId) -> Option<ScopeId> {
        self.model.ancestor_scopes(scope).find(|scope_id| {
            matches!(
                self.model.scope_kind(*scope_id),
                ScopeKind::Function(function) if !function.is_anonymous()
            ) || matches!(
                self.model.scope_kind(*scope_id),
                ScopeKind::Subshell | ScopeKind::CommandSubstitution | ScopeKind::Pipeline
            )
        })
    }

    fn compute_overwritten_functions(&self) -> Vec<OverwrittenFunction> {
        if self.model.functions.is_empty() {
            return Vec::new();
        }

        let cfg = self.cfg();
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let script_terminators = cfg
            .script_terminators()
            .iter()
            .copied()
            .collect::<FxHashSet<_>>();
        let binding_blocks = build_binding_block_index(cfg.blocks(), self.model.bindings.len());
        let mut reachability = ReachabilityCache::new(cfg);
        let mut overwritten = Vec::new();

        for (name, bindings) in &self.model.functions {
            let mut bindings_by_scope = FxHashMap::<ScopeId, Vec<BindingId>>::default();
            for &binding in bindings {
                bindings_by_scope
                    .entry(self.model.binding(binding).scope)
                    .or_default()
                    .push(binding);
            }

            for scope_bindings in bindings_by_scope.values_mut() {
                scope_bindings
                    .sort_by_key(|binding| self.model.binding(*binding).span.start.offset);

                for pair in scope_bindings.windows(2) {
                    let first = pair[0];
                    let second = pair[1];
                    let Some(first_blocks) =
                        reachable_binding_blocks(first, &binding_blocks, &unreachable)
                    else {
                        continue;
                    };
                    let Some(second_blocks) =
                        reachable_binding_blocks(second, &binding_blocks, &unreachable)
                    else {
                        continue;
                    };

                    if !blocks_have_path(&first_blocks, &second_blocks, &mut reachability)
                        || !all_paths_reach_any_block_or_terminate(
                            &first_blocks,
                            &second_blocks,
                            cfg,
                            &unreachable,
                            &script_terminators,
                        )
                    {
                        continue;
                    }

                    let window = OverwriteWindow {
                        first,
                        first_blocks: &first_blocks,
                        second_blocks: &second_blocks,
                        cfg,
                        unreachable: &unreachable,
                    };
                    let mut visiting_scopes = FxHashSet::default();
                    let first_called = self.model.call_sites_for(name).iter().any(|site| {
                        self.overwrite_call_site_resolves_to_binding(name, site, first)
                            && self.call_site_executes_between_overwrite(
                                site,
                                &window,
                                &mut reachability,
                                &mut visiting_scopes,
                            )
                    });

                    overwritten.push(OverwrittenFunction {
                        name: name.clone(),
                        first,
                        second,
                        first_called,
                    });
                }
            }
        }

        overwritten.sort_by_key(|overwritten| {
            (
                self.model.binding(overwritten.first).span.start.offset,
                self.model.binding(overwritten.second).span.start.offset,
            )
        });
        overwritten
    }
}

fn build_binding_block_index(blocks: &[BasicBlock], binding_count: usize) -> Vec<Vec<BlockId>> {
    let mut binding_blocks = vec![Vec::new(); binding_count];
    for block in blocks {
        for &binding in &block.bindings {
            binding_blocks[binding.index()].push(block.id);
        }
    }
    binding_blocks
}

fn reachable_binding_blocks(
    binding: BindingId,
    binding_blocks: &[Vec<BlockId>],
    unreachable: &FxHashSet<BlockId>,
) -> Option<Vec<BlockId>> {
    let blocks = binding_blocks
        .get(binding.index())
        .into_iter()
        .flat_map(|blocks| blocks.iter())
        .copied()
        .filter(|block| !unreachable.contains(block))
        .collect::<Vec<_>>();

    (!blocks.is_empty()).then_some(blocks)
}

fn blocks_have_path(
    starts: &[BlockId],
    ends: &[BlockId],
    reachability: &mut ReachabilityCache<'_>,
) -> bool {
    starts.iter().copied().any(|start| {
        ends.iter()
            .copied()
            .any(|end| reachability.reaches(start, end))
    })
}

fn all_paths_reach_any_block_or_terminate(
    starts: &[BlockId],
    targets: &[BlockId],
    cfg: &ControlFlowGraph,
    unreachable: &FxHashSet<BlockId>,
    script_terminators: &FxHashSet<BlockId>,
) -> bool {
    let targets = targets.iter().copied().collect::<FxHashSet<_>>();
    starts.iter().copied().all(|start| {
        path_reaches_any_target_or_terminates(
            start,
            &targets,
            cfg,
            unreachable,
            script_terminators,
            &mut FxHashSet::default(),
        )
    })
}

fn path_reaches_any_target_or_terminates(
    block: BlockId,
    targets: &FxHashSet<BlockId>,
    cfg: &ControlFlowGraph,
    unreachable: &FxHashSet<BlockId>,
    script_terminators: &FxHashSet<BlockId>,
    visiting: &mut FxHashSet<BlockId>,
) -> bool {
    if unreachable.contains(&block) {
        return false;
    }
    if targets.contains(&block) || script_terminators.contains(&block) {
        return true;
    }
    if !visiting.insert(block) {
        return false;
    }

    let successors = cfg
        .successors(block)
        .iter()
        .filter(|(_, edge)| !matches!(edge, EdgeKind::NestedRegion))
        .copied()
        .collect::<Vec<_>>();
    let reaches = !successors.is_empty()
        && successors.iter().all(|(successor, _)| {
            path_reaches_any_target_or_terminates(
                *successor,
                targets,
                cfg,
                unreachable,
                script_terminators,
                visiting,
            )
        });

    visiting.remove(&block);
    reaches
}

fn reachable_blocks_for_binding(
    cfg: &ControlFlowGraph,
    binding: BindingId,
    unreachable: &FxHashSet<BlockId>,
) -> Vec<BlockId> {
    cfg.blocks()
        .iter()
        .filter(|block| block.bindings.contains(&binding) && !unreachable.contains(&block.id))
        .map(|block| block.id)
        .collect()
}

fn blocks_have_path_avoiding(
    cfg: &ControlFlowGraph,
    starts: &[BlockId],
    ends: &[BlockId],
    avoid: &FxHashSet<BlockId>,
) -> bool {
    starts.iter().copied().any(|start| {
        ends.iter()
            .copied()
            .any(|end| block_reaches_avoiding(cfg, start, end, avoid))
    })
}

fn blocks_have_path_avoiding_many(
    cfg: &ControlFlowGraph,
    starts: &[BlockId],
    ends: &[BlockId],
    first_avoid: &FxHashSet<BlockId>,
    second_avoid: &FxHashSet<BlockId>,
) -> bool {
    starts.iter().copied().any(|start| {
        ends.iter()
            .copied()
            .any(|end| block_reaches_avoiding_many(cfg, start, end, first_avoid, second_avoid))
    })
}

fn block_reaches_avoiding(
    cfg: &ControlFlowGraph,
    start: BlockId,
    end: BlockId,
    avoid: &FxHashSet<BlockId>,
) -> bool {
    let empty = FxHashSet::default();
    block_reaches_avoiding_many(cfg, start, end, avoid, &empty)
}

fn block_reaches_avoiding_many(
    cfg: &ControlFlowGraph,
    start: BlockId,
    end: BlockId,
    first_avoid: &FxHashSet<BlockId>,
    second_avoid: &FxHashSet<BlockId>,
) -> bool {
    let mut visited = FxHashSet::default();
    let mut stack = vec![start];
    while let Some(block) = stack.pop() {
        if !visited.insert(block) {
            continue;
        }
        if block == end {
            return true;
        }
        if first_avoid.contains(&block) || second_avoid.contains(&block) {
            continue;
        }
        for (successor, _) in cfg.successors(block) {
            stack.push(*successor);
        }
    }

    false
}

fn all_paths_terminate_before_natural_exit(
    starts: &[BlockId],
    cfg: &ControlFlowGraph,
    script_terminators: &FxHashSet<BlockId>,
    natural_exits: &FxHashSet<BlockId>,
    unreachable: &FxHashSet<BlockId>,
    shadow_blocks: &FxHashSet<BlockId>,
    empty_shadow_cache: &mut FxHashMap<BlockId, bool>,
) -> bool {
    let mut saw_termination = false;
    let cacheable = shadow_blocks.is_empty();
    let mut context = TerminationPathContext {
        cfg,
        script_terminators,
        natural_exits,
        unreachable,
        shadow_blocks,
        empty_shadow_cache,
        cacheable,
        saw_termination: &mut saw_termination,
    };
    starts.iter().copied().all(|start| {
        path_terminates_before_natural_exit(start, &mut FxHashSet::default(), &mut context)
    }) && *context.saw_termination
}

struct TerminationPathContext<'a, 'cache, 'seen> {
    cfg: &'a ControlFlowGraph,
    script_terminators: &'a FxHashSet<BlockId>,
    natural_exits: &'a FxHashSet<BlockId>,
    unreachable: &'a FxHashSet<BlockId>,
    shadow_blocks: &'a FxHashSet<BlockId>,
    empty_shadow_cache: &'cache mut FxHashMap<BlockId, bool>,
    cacheable: bool,
    saw_termination: &'seen mut bool,
}

fn path_terminates_before_natural_exit(
    block: BlockId,
    visiting: &mut FxHashSet<BlockId>,
    context: &mut TerminationPathContext<'_, '_, '_>,
) -> bool {
    if context.cacheable
        && let Some(cached) = context.empty_shadow_cache.get(&block)
    {
        if *cached {
            *context.saw_termination = true;
        }
        return *cached;
    }
    if context.unreachable.contains(&block) || context.shadow_blocks.contains(&block) {
        return false;
    }
    if context.script_terminators.contains(&block) {
        *context.saw_termination = true;
        if context.cacheable {
            context.empty_shadow_cache.insert(block, true);
        }
        return true;
    }
    if context.natural_exits.contains(&block) {
        if context.cacheable {
            context.empty_shadow_cache.insert(block, false);
        }
        return false;
    }
    if !visiting.insert(block) {
        return false;
    }

    let successors = context
        .cfg
        .successors(block)
        .iter()
        .filter(|(_, edge)| !matches!(edge, EdgeKind::NestedRegion))
        .copied()
        .collect::<Vec<_>>();
    let terminates = !successors.is_empty()
        && successors.iter().all(|(successor, _)| {
            path_terminates_before_natural_exit(*successor, visiting, context)
        });

    visiting.remove(&block);
    if context.cacheable {
        context.empty_shadow_cache.insert(block, terminates);
    }
    terminates
}

pub(crate) fn block_reaches_without(
    cfg: &ControlFlowGraph,
    start: BlockId,
    end: BlockId,
    avoided: BlockId,
) -> bool {
    if start == avoided {
        return false;
    }

    let mut visited = FxHashSet::default();
    let mut stack = vec![start];

    while let Some(block) = stack.pop() {
        if block == avoided || !visited.insert(block) {
            continue;
        }
        if block == end {
            return true;
        }
        for (successor, _) in cfg.successors(block) {
            stack.push(*successor);
        }
    }

    false
}

struct ReachabilityCache<'a> {
    cfg: &'a ControlFlowGraph,
    cache: FxHashMap<BlockId, FxHashSet<BlockId>>,
}

impl<'a> ReachabilityCache<'a> {
    fn new(cfg: &'a ControlFlowGraph) -> Self {
        Self {
            cfg,
            cache: FxHashMap::default(),
        }
    }

    fn reaches(&mut self, start: BlockId, end: BlockId) -> bool {
        self.cache
            .entry(start)
            .or_insert_with(|| {
                let mut visited = FxHashSet::default();
                let mut stack = vec![start];

                while let Some(block) = stack.pop() {
                    if !visited.insert(block) {
                        continue;
                    }
                    for (successor, _) in self.cfg.successors(block) {
                        stack.push(*successor);
                    }
                }

                visited
            })
            .contains(&end)
    }
}
