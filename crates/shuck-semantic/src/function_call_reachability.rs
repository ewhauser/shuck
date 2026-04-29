use super::*;

/// Direct function-call candidate supplied by higher-level analyses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCallCandidate {
    /// Callee name.
    pub callee: Name,
    /// Scope that contains the call.
    pub scope: ScopeId,
    /// Span of the callee token.
    pub name_span: Span,
    /// Span of the enclosing command.
    pub command_span: Span,
}

/// Controls whether transient execution contexts are accepted for a direct-call query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FunctionCallPersistence {
    /// Count calls regardless of whether they happen in transient contexts.
    #[default]
    Any,
    /// Count only calls whose effects can persist in the parent environment.
    PersistentOnly,
}

/// Offset and scope constraints for a direct function-call reachability query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectFunctionCallWindow {
    /// Inclusive lower offset bound for eligible calls.
    pub after_offset: usize,
    /// Exclusive upper offset bound for eligible calls.
    pub before_offset: usize,
    /// Whether transient execution contexts are acceptable.
    pub persistence: FunctionCallPersistence,
    /// Optional scope boundary that eligible calls must stay within.
    pub scope_boundary: Option<ScopeId>,
    /// Whether the call itself must avoid transient command contexts.
    pub require_non_transient_call: bool,
}

impl DirectFunctionCallWindow {
    /// Creates a window that accepts calls before `before_offset`.
    pub fn before_offset(before_offset: usize) -> Self {
        Self {
            after_offset: 0,
            before_offset,
            persistence: FunctionCallPersistence::Any,
            scope_boundary: None,
            require_non_transient_call: false,
        }
    }

    /// Creates a window that accepts calls between `after_offset` and `before_offset`.
    pub fn between_offsets(after_offset: usize, before_offset: usize) -> Self {
        Self {
            after_offset,
            before_offset,
            persistence: FunctionCallPersistence::Any,
            scope_boundary: None,
            require_non_transient_call: false,
        }
    }

    /// Restricts the window to persistently reachable calls only.
    pub fn persistent(mut self) -> Self {
        self.persistence = FunctionCallPersistence::PersistentOnly;
        self
    }

    /// Requires the call site itself to avoid transient command contexts.
    pub fn require_non_transient_call(mut self) -> Self {
        self.require_non_transient_call = true;
        self
    }

    /// Restricts the window to calls that stay within `scope`.
    pub fn within_scope(mut self, scope: ScopeId) -> Self {
        self.scope_boundary = Some(scope);
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FunctionCallResolutionKey {
    binding: BindingId,
    call_scope: ScopeId,
    visibility_start: usize,
    visibility_end: usize,
    cfg_start: usize,
    cfg_end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FunctionScopeActivation {
    activation_offset: usize,
    required_before_offset: usize,
    persistent: bool,
}

struct FunctionActivationIndex {
    activations_by_scope: FxHashMap<ScopeId, Vec<FunctionScopeActivation>>,
}

#[derive(Clone, Copy)]
struct FunctionActivationEdge {
    target_scope: ScopeId,
    target_binding_offset: usize,
    call_offset: usize,
    persistent: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FunctionActivationEdgeKey {
    target_binding: BindingId,
    target_scope: ScopeId,
    call_scope: ScopeId,
    call_offset: usize,
}

#[derive(Clone, Copy)]
struct BoundaryCallWindow {
    after_offset: usize,
    before_offset: usize,
    boundary_scope: ScopeId,
    persistence: FunctionCallPersistence,
    require_non_transient_call: bool,
}

/// Reachability engine for direct calls to function bindings.
pub struct DirectFunctionCallReachability<'analysis, 'model> {
    analysis: &'analysis SemanticAnalysis<'model>,
    supplemental_calls_by_name: FxHashMap<Name, Vec<FunctionCallCandidate>>,
    scope_run_cache: FxHashMap<(ScopeId, usize), bool>,
    scope_between_cache: FxHashMap<(ScopeId, usize, usize), bool>,
    call_resolution_cache: FxHashMap<FunctionCallResolutionKey, bool>,
    activation_index: Option<FunctionActivationIndex>,
}

impl<'analysis, 'model> DirectFunctionCallReachability<'analysis, 'model> {
    pub(crate) fn new(
        analysis: &'analysis SemanticAnalysis<'model>,
        supplemental_calls: impl IntoIterator<Item = FunctionCallCandidate>,
    ) -> Self {
        let mut supplemental_calls_by_name =
            FxHashMap::<Name, Vec<FunctionCallCandidate>>::default();
        for call in supplemental_calls {
            supplemental_calls_by_name
                .entry(call.callee.clone())
                .or_default()
                .push(call);
        }

        Self {
            analysis,
            supplemental_calls_by_name,
            scope_run_cache: FxHashMap::default(),
            scope_between_cache: FxHashMap::default(),
            call_resolution_cache: FxHashMap::default(),
            activation_index: None,
        }
    }

    /// Enables an auxiliary activation index that speeds up repeated reachability queries.
    pub fn enable_activation_index(&mut self) {
        if self.activation_index.is_none() {
            self.activation_index = Some(self.build_activation_index());
        }
    }

    /// Returns whether `binding_id` has a direct call that satisfies `window`.
    pub fn binding_has_reachable_direct_call(
        &mut self,
        binding_id: BindingId,
        window: DirectFunctionCallWindow,
    ) -> bool {
        let binding = self.analysis.model.binding(binding_id);
        let after_offset = window.after_offset.max(binding.span.start.offset);
        if let Some(boundary_scope) = window.scope_boundary {
            let window = BoundaryCallWindow {
                after_offset,
                before_offset: window.before_offset,
                boundary_scope,
                persistence: window.persistence,
                require_non_transient_call: window.require_non_transient_call,
            };
            return self.has_call_to_function_binding_inside_boundary(binding_id, window);
        }

        self.has_call_to_function_binding_between_offsets(
            binding_id,
            after_offset,
            window.before_offset,
            window.persistence,
            window.require_non_transient_call,
            &mut FxHashSet::default(),
        )
    }

    /// Returns whether `scope` can execute before `before_offset`.
    pub fn scope_can_run_before_offset(
        &mut self,
        scope: ScopeId,
        before_offset: usize,
        persistence: FunctionCallPersistence,
    ) -> bool {
        match persistence {
            FunctionCallPersistence::Any => self.command_scope_can_run_before_offset(
                scope,
                before_offset,
                &mut FxHashSet::default(),
            ),
            FunctionCallPersistence::PersistentOnly => self
                .command_scope_can_run_persistently_before_offset(
                    scope,
                    before_offset,
                    &mut FxHashSet::default(),
                ),
        }
    }

    /// Returns whether `scope` can execute between `after_offset` and `before_offset`.
    pub fn scope_can_run_between_offsets(
        &mut self,
        scope: ScopeId,
        after_offset: usize,
        before_offset: usize,
        persistence: FunctionCallPersistence,
    ) -> bool {
        match persistence {
            FunctionCallPersistence::Any => self.command_scope_can_run_between_offsets(
                scope,
                after_offset,
                before_offset,
                &mut FxHashSet::default(),
            ),
            FunctionCallPersistence::PersistentOnly => self
                .command_scope_can_run_persistently_between_offsets(
                    scope,
                    after_offset,
                    before_offset,
                    &mut FxHashSet::default(),
                ),
        }
    }

    fn model(&self) -> &'model SemanticModel {
        self.analysis.model
    }

    fn call_candidates_for(&self, name: &Name) -> Vec<FunctionCallCandidate> {
        let mut candidates = self
            .supplemental_calls_by_name
            .get(name)
            .cloned()
            .unwrap_or_default();
        candidates.extend(self.model().call_sites_for(name).iter().map(|site| {
            FunctionCallCandidate {
                callee: site.callee.clone(),
                scope: site.scope,
                name_span: site.name_span,
                command_span: site.span,
            }
        }));
        candidates
    }

    fn build_activation_index(&mut self) -> FunctionActivationIndex {
        let edges_by_caller = self.build_activation_edges_by_caller();
        let mut index = FunctionActivationIndex {
            activations_by_scope: FxHashMap::default(),
        };
        let mut pending = Vec::new();

        for edge in edges_by_caller
            .get(&None)
            .into_iter()
            .flat_map(|edges| edges.iter())
        {
            if edge.call_offset <= edge.target_binding_offset {
                continue;
            }
            let activation = FunctionScopeActivation {
                activation_offset: edge.call_offset,
                required_before_offset: edge.call_offset,
                persistent: edge.persistent,
            };
            if index.insert(edge.target_scope, activation) {
                pending.push((edge.target_scope, activation));
            }
        }

        while let Some((scope, activation)) = pending.pop() {
            if !index.contains(scope, activation) {
                continue;
            }
            for edge in edges_by_caller
                .get(&Some(scope))
                .into_iter()
                .flat_map(|edges| edges.iter())
            {
                if activation.activation_offset <= edge.target_binding_offset {
                    continue;
                }
                let next = FunctionScopeActivation {
                    activation_offset: activation.activation_offset,
                    required_before_offset: activation.required_before_offset.max(edge.call_offset),
                    persistent: activation.persistent && edge.persistent,
                };
                if index.insert(edge.target_scope, next) {
                    pending.push((edge.target_scope, next));
                }
            }
        }

        index
    }

    fn build_activation_edges_by_caller(
        &mut self,
    ) -> FxHashMap<Option<ScopeId>, Vec<FunctionActivationEdge>> {
        let mut edges_by_caller =
            FxHashMap::<Option<ScopeId>, Vec<FunctionActivationEdge>>::default();
        let mut seen_edges = FxHashSet::<FunctionActivationEdgeKey>::default();
        let scope_bindings = self
            .model()
            .function_binding_scope_index()
            .iter()
            .map(|(scope, bindings)| (*scope, bindings.to_vec()))
            .collect::<Vec<_>>();

        for (target_scope, binding_ids) in scope_bindings {
            for target_binding in binding_ids {
                let binding = self.model().binding(target_binding);
                for call in self.call_candidates_for(&binding.name) {
                    self.add_activation_edge(
                        target_binding,
                        target_scope,
                        binding.span.start.offset,
                        &call,
                        &mut seen_edges,
                        &mut edges_by_caller,
                    );
                }
            }
        }

        edges_by_caller
    }

    fn add_activation_edge(
        &mut self,
        target_binding: BindingId,
        target_scope: ScopeId,
        target_binding_offset: usize,
        call: &FunctionCallCandidate,
        seen_edges: &mut FxHashSet<FunctionActivationEdgeKey>,
        edges_by_caller: &mut FxHashMap<Option<ScopeId>, Vec<FunctionActivationEdge>>,
    ) {
        if !self.call_may_resolve_to_binding_cached(target_binding, call) {
            return;
        }

        let key = FunctionActivationEdgeKey {
            target_binding,
            target_scope,
            call_scope: call.scope,
            call_offset: call.name_span.start.offset,
        };
        if !seen_edges.insert(key) {
            return;
        }

        let edge = FunctionActivationEdge {
            target_scope,
            target_binding_offset,
            call_offset: call.name_span.start.offset,
            persistent: !self.analysis.scope_runs_in_transient_context(call.scope),
        };
        edges_by_caller
            .entry(self.enclosing_function_scope(call.scope))
            .or_default()
            .push(edge);
    }

    fn command_scope_can_run_between_offsets(
        &mut self,
        scope: ScopeId,
        after_offset: usize,
        before_offset: usize,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if let Some(activation_index) = &self.activation_index {
            return activation_index.scope_can_run_between_offsets(
                self,
                scope,
                after_offset,
                before_offset,
                false,
            );
        }

        let Some(function_scope) = self.enclosing_function_scope(scope) else {
            return true;
        };
        if !visiting.contains(&function_scope)
            && let Some(cached) =
                self.scope_between_cache
                    .get(&(function_scope, after_offset, before_offset))
        {
            return *cached;
        }
        if !visiting.insert(function_scope) {
            return false;
        }

        let binding_ids = self.function_bindings_for_scope(function_scope);
        let can_run = binding_ids.into_iter().any(|function_binding| {
            self.has_call_to_function_binding_between_offsets(
                function_binding,
                after_offset,
                before_offset,
                FunctionCallPersistence::Any,
                false,
                visiting,
            )
        });

        visiting.remove(&function_scope);
        self.scope_between_cache
            .insert((function_scope, after_offset, before_offset), can_run);
        can_run
    }

    fn command_scope_can_run_before_offset(
        &mut self,
        scope: ScopeId,
        before_offset: usize,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if let Some(activation_index) = &self.activation_index {
            return activation_index.scope_can_run_before_offset(self, scope, before_offset, false);
        }

        let Some(function_scope) = self.enclosing_function_scope(scope) else {
            return true;
        };
        if !visiting.contains(&function_scope)
            && let Some(cached) = self.scope_run_cache.get(&(function_scope, before_offset))
        {
            return *cached;
        }
        if !visiting.insert(function_scope) {
            return false;
        }

        let binding_ids = self.function_bindings_for_scope(function_scope);
        let can_run = binding_ids.into_iter().any(|function_binding| {
            self.has_call_to_function_binding_between_offsets(
                function_binding,
                0,
                before_offset,
                FunctionCallPersistence::Any,
                false,
                visiting,
            )
        });

        visiting.remove(&function_scope);
        self.scope_run_cache
            .insert((function_scope, before_offset), can_run);
        can_run
    }

    fn command_scope_can_run_persistently_before_offset(
        &mut self,
        scope: ScopeId,
        before_offset: usize,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if self.analysis.scope_runs_in_transient_context(scope) {
            return false;
        }
        if let Some(activation_index) = &self.activation_index {
            return activation_index.scope_can_run_before_offset(self, scope, before_offset, true);
        }

        let Some(function_scope) = self.enclosing_function_scope(scope) else {
            return true;
        };
        if !visiting.insert(function_scope) {
            return false;
        }

        let binding_ids = self.function_bindings_for_scope(function_scope);
        let can_run = binding_ids.into_iter().any(|function_binding| {
            self.has_call_to_function_binding_between_offsets(
                function_binding,
                0,
                before_offset,
                FunctionCallPersistence::PersistentOnly,
                false,
                visiting,
            )
        });

        visiting.remove(&function_scope);
        can_run
    }

    fn command_scope_can_run_persistently_between_offsets(
        &mut self,
        scope: ScopeId,
        after_offset: usize,
        before_offset: usize,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if self.analysis.scope_runs_in_transient_context(scope) {
            return false;
        }
        if let Some(activation_index) = &self.activation_index {
            return activation_index.scope_can_run_between_offsets(
                self,
                scope,
                after_offset,
                before_offset,
                true,
            );
        }

        let Some(function_scope) = self.enclosing_function_scope(scope) else {
            return true;
        };
        if !visiting.insert(function_scope) {
            return false;
        }

        let binding_ids = self.function_bindings_for_scope(function_scope);
        let can_run = binding_ids.into_iter().any(|function_binding| {
            self.has_call_to_function_binding_between_offsets(
                function_binding,
                after_offset,
                before_offset,
                FunctionCallPersistence::PersistentOnly,
                false,
                visiting,
            )
        });

        visiting.remove(&function_scope);
        can_run
    }

    fn has_call_to_function_binding_between_offsets(
        &mut self,
        function_binding: BindingId,
        after_offset: usize,
        before_offset: usize,
        persistence: FunctionCallPersistence,
        require_non_transient_call: bool,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let binding = self.model().binding(function_binding);
        let required_after = after_offset.max(binding.span.start.offset);
        self.call_candidates_for(&binding.name)
            .into_iter()
            .any(|call| {
                let call_offset = call.name_span.start.offset;
                call_offset <= before_offset
                    && (!require_non_transient_call
                        || !self.analysis.scope_runs_in_transient_context(call.scope))
                    && self.call_may_resolve_to_binding_cached(function_binding, &call)
                    && self.call_scope_can_execute_after_offset_before_offset(
                        call.scope,
                        call_offset,
                        required_after,
                        before_offset,
                        persistence,
                        visiting,
                    )
            })
    }

    fn call_scope_can_execute_after_offset_before_offset(
        &mut self,
        call_scope: ScopeId,
        call_offset: usize,
        after_offset: usize,
        before_offset: usize,
        persistence: FunctionCallPersistence,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if call_offset > before_offset {
            return false;
        }
        if matches!(persistence, FunctionCallPersistence::PersistentOnly)
            && self.analysis.scope_runs_in_transient_context(call_scope)
        {
            return false;
        }

        if self.enclosing_function_scope(call_scope).is_none() {
            return call_offset > after_offset;
        }

        match persistence {
            FunctionCallPersistence::Any => self.command_scope_can_run_between_offsets(
                call_scope,
                after_offset,
                before_offset,
                visiting,
            ),
            FunctionCallPersistence::PersistentOnly => self
                .command_scope_can_run_persistently_between_offsets(
                    call_scope,
                    after_offset,
                    before_offset,
                    visiting,
                ),
        }
    }

    fn has_call_to_function_binding_inside_boundary(
        &mut self,
        function_binding: BindingId,
        window: BoundaryCallWindow,
    ) -> bool {
        self.has_call_to_function_binding_inside_boundary_with_visiting(
            function_binding,
            window,
            &mut FxHashSet::default(),
        )
    }

    fn has_call_to_function_binding_inside_boundary_with_visiting(
        &mut self,
        function_binding: BindingId,
        window: BoundaryCallWindow,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let binding = self.model().binding(function_binding);
        let nested_window = BoundaryCallWindow {
            after_offset: window.after_offset.max(binding.span.start.offset),
            ..window
        };
        self.call_candidates_for(&binding.name)
            .into_iter()
            .any(|call| {
                let call_offset = call.name_span.start.offset;
                call_offset <= window.before_offset
                    && self.call_is_inside_scope(call.scope, window.boundary_scope)
                    && (!window.require_non_transient_call
                        || !self.analysis.scope_runs_in_transient_context(call.scope))
                    && (!matches!(window.persistence, FunctionCallPersistence::PersistentOnly)
                        || !self.analysis.scope_runs_in_transient_context(call.scope))
                    && self.call_may_resolve_to_binding_cached(function_binding, &call)
                    && self.call_scope_can_execute_inside_boundary_after_offset_before_offset(
                        call.scope,
                        call_offset,
                        nested_window,
                        visiting,
                    )
            })
    }

    fn call_scope_can_execute_inside_boundary_after_offset_before_offset(
        &mut self,
        call_scope: ScopeId,
        call_offset: usize,
        window: BoundaryCallWindow,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if call_offset > window.before_offset {
            return false;
        }

        let Some(function_scope) = self.enclosing_function_scope(call_scope) else {
            return call_offset > window.after_offset;
        };
        if function_scope == window.boundary_scope {
            return call_offset > window.after_offset;
        }

        self.command_scope_can_run_inside_boundary_between_offsets(call_scope, window, visiting)
    }

    fn command_scope_can_run_inside_boundary_between_offsets(
        &mut self,
        scope: ScopeId,
        window: BoundaryCallWindow,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let Some(function_scope) = self.enclosing_function_scope(scope) else {
            return true;
        };
        if function_scope == window.boundary_scope {
            return true;
        }
        if !self.call_is_inside_scope(function_scope, window.boundary_scope) {
            return false;
        }
        if !visiting.insert(function_scope) {
            return false;
        }

        let binding_ids = self.function_bindings_for_scope(function_scope);
        let can_run = binding_ids.into_iter().any(|function_binding| {
            self.has_call_to_function_binding_inside_boundary_with_visiting(
                function_binding,
                window,
                visiting,
            )
        });

        visiting.remove(&function_scope);
        can_run
    }

    fn call_may_resolve_to_binding_cached(
        &mut self,
        binding_id: BindingId,
        call: &FunctionCallCandidate,
    ) -> bool {
        let key = FunctionCallResolutionKey {
            binding: binding_id,
            call_scope: call.scope,
            visibility_start: call.name_span.start.offset,
            visibility_end: call.name_span.end.offset,
            cfg_start: call.command_span.start.offset,
            cfg_end: call.command_span.end.offset,
        };
        if let Some(cached) = self.call_resolution_cache.get(&key) {
            return *cached;
        }

        let resolved = self.analysis.function_call_may_resolve_to_binding(
            binding_id,
            call.scope,
            call.name_span,
            call.command_span,
        );
        self.call_resolution_cache.insert(key, resolved);
        resolved
    }

    fn enclosing_function_scope(&self, scope: ScopeId) -> Option<ScopeId> {
        self.model().enclosing_function_scope(scope)
    }

    fn call_is_inside_scope(&self, scope: ScopeId, ancestor_scope: ScopeId) -> bool {
        scope == ancestor_scope
            || self
                .model()
                .ancestor_scopes(scope)
                .any(|scope| scope == ancestor_scope)
    }

    fn function_bindings_for_scope(&self, scope: ScopeId) -> Vec<BindingId> {
        self.model()
            .function_binding_scope_index()
            .get(&scope)
            .map(|bindings| bindings.iter().copied().collect())
            .unwrap_or_default()
    }
}

impl FunctionActivationIndex {
    fn insert(&mut self, scope: ScopeId, activation: FunctionScopeActivation) -> bool {
        let activations = self.activations_by_scope.entry(scope).or_default();
        if activations
            .iter()
            .any(|existing| activation_dominates(*existing, activation))
        {
            return false;
        }
        activations.retain(|existing| !activation_dominates(activation, *existing));
        activations.push(activation);
        true
    }

    fn contains(&self, scope: ScopeId, activation: FunctionScopeActivation) -> bool {
        self.activations_by_scope
            .get(&scope)
            .is_some_and(|activations| activations.contains(&activation))
    }

    fn scope_can_run_before_offset(
        &self,
        reachability: &DirectFunctionCallReachability<'_, '_>,
        scope: ScopeId,
        before_offset: usize,
        persistent: bool,
    ) -> bool {
        let Some(function_scope) = reachability.enclosing_function_scope(scope) else {
            return true;
        };
        self.activations_by_scope
            .get(&function_scope)
            .is_some_and(|activations| {
                activations.iter().any(|activation| {
                    activation.required_before_offset <= before_offset
                        && (!persistent || activation.persistent)
                })
            })
    }

    fn scope_can_run_between_offsets(
        &self,
        reachability: &DirectFunctionCallReachability<'_, '_>,
        scope: ScopeId,
        after_offset: usize,
        before_offset: usize,
        persistent: bool,
    ) -> bool {
        let Some(function_scope) = reachability.enclosing_function_scope(scope) else {
            return true;
        };
        self.activations_by_scope
            .get(&function_scope)
            .is_some_and(|activations| {
                activations.iter().any(|activation| {
                    activation.activation_offset > after_offset
                        && activation.required_before_offset <= before_offset
                        && (!persistent || activation.persistent)
                })
            })
    }
}

fn activation_dominates(left: FunctionScopeActivation, right: FunctionScopeActivation) -> bool {
    left.activation_offset >= right.activation_offset
        && left.required_before_offset <= right.required_before_offset
        && (left.persistent || !right.persistent)
}
