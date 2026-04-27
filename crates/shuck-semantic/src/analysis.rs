use super::*;
use crate::cfg::build_control_flow_graph;
use crate::dataflow;
use crate::reachability::block_reaches_without;

#[allow(missing_docs)]
impl<'model> SemanticAnalysis<'model> {
    pub(crate) fn new(model: &'model SemanticModel) -> Self {
        Self {
            model,
            cfg: OnceLock::new(),
            exact_variable_dataflow: OnceLock::new(),
            dataflow: OnceLock::new(),
            unused_assignments: OnceLock::new(),
            unused_assignments_shellcheck_compat: OnceLock::new(),
            uninitialized_references: OnceLock::new(),
            uninitialized_reference_certainties: OnceLock::new(),
            dead_code: OnceLock::new(),
            unreachable_blocks: OnceLock::new(),
            binding_block_index: OnceLock::new(),
            overwritten_functions: OnceLock::new(),
            unreached_functions: OnceLock::new(),
            unreached_functions_shellcheck_compat: OnceLock::new(),
            scope_provided_binding_index: OnceLock::new(),
        }
    }

    pub fn cfg(&self) -> &ControlFlowGraph {
        self.cfg.get_or_init(|| {
            build_control_flow_graph(
                &self.model.recorded_program,
                &self.model.command_bindings,
                &self.model.command_references,
                &self.model.scopes,
                &self.model.bindings,
                &self.model.call_sites,
            )
        })
    }

    /// Returns the CFG's unreachable blocks as an indexed set.
    #[doc(hidden)]
    pub fn unreachable_blocks(&self) -> &FxHashSet<BlockId> {
        self.unreachable_blocks
            .get_or_init(|| self.cfg().unreachable().iter().copied().collect())
    }

    /// Returns whether a CFG block is unreachable.
    #[doc(hidden)]
    pub fn block_is_unreachable(&self, block_id: BlockId) -> bool {
        self.unreachable_blocks().contains(&block_id)
    }

    pub fn visible_function_binding_at_call(
        &self,
        name: &Name,
        name_span: Span,
    ) -> Option<BindingId> {
        self.model
            .call_sites_for(name)
            .iter()
            .find(|site| site.name_span == name_span)?;

        self.visible_function_call_bindings()
            .get(&SpanKey::new(name_span))
            .copied()
    }

    pub fn resolved_function_call_sites<'analysis>(
        &'analysis self,
        name: &'analysis Name,
    ) -> impl Iterator<Item = (&'analysis CallSite, BindingId)> + 'analysis {
        self.model.call_sites_for(name).iter().filter_map(|site| {
            self.visible_function_call_bindings()
                .get(&SpanKey::new(site.name_span))
                .copied()
                .map(|binding| (site, binding))
        })
    }

    pub fn function_call_arity_sites<'analysis>(
        &'analysis self,
        name: &'analysis Name,
    ) -> impl Iterator<Item = (&'analysis CallSite, BindingId)> + 'analysis {
        self.model.call_sites_for(name).iter().filter_map(|site| {
            self.visible_function_call_bindings()
                .get(&SpanKey::new(site.name_span))
                .copied()
                .or_else(|| self.lexical_function_binding_for_call_offset(name, site))
                .map(|binding| (site, binding))
        })
    }

    pub fn function_scope_for_binding(&self, binding_id: BindingId) -> Option<ScopeId> {
        self.model
            .recorded_program
            .function_body_scopes
            .get(&binding_id)
            .copied()
    }

    pub fn visible_function_binding_defined_before(
        &self,
        name: &Name,
        site_scope: ScopeId,
        site_offset: usize,
    ) -> Option<BindingId> {
        self.model
            .ancestor_scopes(site_scope)
            .find_map(|scope| self.latest_function_binding_before(name, scope, site_offset))
    }

    fn lexical_function_binding_for_call_offset(
        &self,
        name: &Name,
        site: &CallSite,
    ) -> Option<BindingId> {
        let scopes = self.model.ancestor_scopes(site.scope).collect::<Vec<_>>();

        scopes
            .iter()
            .copied()
            .find_map(|scope| {
                self.latest_function_binding_before(name, scope, site.name_span.start.offset)
            })
            .or_else(|| {
                scopes
                    .iter()
                    .copied()
                    .find_map(|scope| self.earliest_function_binding_in_scope(name, scope))
            })
    }

    fn latest_function_binding_before(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> Option<BindingId> {
        self.model
            .function_definitions(name)
            .iter()
            .copied()
            .filter(|candidate| self.model.binding(*candidate).scope == scope)
            .filter(|candidate| self.model.binding(*candidate).span.start.offset < offset)
            .max_by_key(|candidate| self.model.binding(*candidate).span.start.offset)
    }

    fn earliest_function_binding_in_scope(&self, name: &Name, scope: ScopeId) -> Option<BindingId> {
        self.model
            .function_definitions(name)
            .iter()
            .copied()
            .filter(|candidate| self.model.binding(*candidate).scope == scope)
            .min_by_key(|candidate| self.model.binding(*candidate).span.start.offset)
    }

    fn visible_function_call_bindings(&self) -> &FxHashMap<SpanKey, BindingId> {
        self.model.visible_function_call_bindings()
    }

    #[doc(hidden)]
    pub fn function_bindings_by_scope(&self) -> impl Iterator<Item = (ScopeId, &[BindingId])> + '_ {
        self.model
            .function_binding_scope_index()
            .iter()
            .map(|(scope, bindings)| (*scope, bindings.as_slice()))
    }

    #[doc(hidden)]
    pub fn block_ids_for_span(&self, span: Span) -> &[BlockId] {
        self.cfg().block_ids_for_span(span)
    }

    pub(crate) fn exact_variable_dataflow(&self) -> &ExactVariableDataflow {
        self.exact_variable_dataflow.get_or_init(|| {
            let cfg = self.cfg();
            let context = self.model.dataflow_context(cfg);
            dataflow::build_exact_variable_dataflow(&context)
        })
    }

    #[allow(dead_code)]
    pub(crate) fn dataflow(&self) -> &DataflowResult {
        self.dataflow.get_or_init(|| {
            let cfg = self.cfg();
            let context = self.model.dataflow_context(cfg);
            let exact = self.exact_variable_dataflow();
            dataflow::analyze(&context, exact)
        })
    }

    #[cfg(test)]
    pub(crate) fn materialized_reaching_definitions(&self) -> ReachingDefinitions {
        let cfg = self.cfg();
        let context = self.model.dataflow_context(cfg);
        let exact = self.exact_variable_dataflow();
        dataflow::materialize_reaching_definitions(&context, exact)
    }

    /// Returns the innermost function scope containing `offset`.
    #[doc(hidden)]
    pub fn enclosing_function_scope_at(&self, offset: usize) -> Option<ScopeId> {
        self.model
            .ancestor_scopes(self.model.scope_at(offset))
            .find(|scope| matches!(self.model.scope(*scope).kind, ScopeKind::Function(_)))
    }

    /// Returns whether a binding's scope is `ancestor_scope` or nested below it.
    #[doc(hidden)]
    pub fn binding_is_in_scope_or_descendant(
        &self,
        binding_id: BindingId,
        ancestor_scope: ScopeId,
    ) -> bool {
        self.model
            .ancestor_scopes(self.model.binding(binding_id).scope)
            .any(|scope| scope == ancestor_scope)
    }

    /// Returns the entry block that covers the common runtime scope of `binding_scopes`.
    #[doc(hidden)]
    pub fn flow_entry_block_for_binding_scopes(
        &self,
        binding_scopes: &[ScopeId],
        reference_offset: usize,
    ) -> BlockId {
        let cfg = self.cfg();
        self.model
            .ancestor_scopes(self.model.scope_at(reference_offset))
            .find_map(|scope| {
                if !matches!(
                    self.model.scope(scope).kind,
                    ScopeKind::Function(_) | ScopeKind::File
                ) {
                    return None;
                }
                binding_scopes
                    .iter()
                    .copied()
                    .all(|binding_scope| {
                        self.model
                            .ancestor_scopes(binding_scope)
                            .any(|ancestor| ancestor == scope)
                    })
                    .then(|| cfg.scope_entry(scope))
                    .flatten()
            })
            .unwrap_or_else(|| cfg.entry())
    }

    /// Returns true when every path from `entry` to `target` crosses one of `cover_blocks`.
    #[doc(hidden)]
    pub fn blocks_cover_all_paths_to_block(
        &self,
        entry: BlockId,
        target: BlockId,
        cover_blocks: &FxHashSet<BlockId>,
    ) -> bool {
        if cover_blocks.contains(&target) {
            return true;
        }

        let cfg = self.cfg();
        let unreachable = self.unreachable_blocks();
        let mut stack = vec![entry];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if cover_blocks.contains(&block_id)
                || unreachable.contains(&block_id)
                || !seen.insert(block_id)
            {
                continue;
            }
            if block_id == target {
                return false;
            }
            for (successor, _) in cfg.successors(block_id) {
                stack.push(*successor);
            }
        }

        true
    }

    /// Returns true when a binding's CFG block dominates a named reference from the relevant
    /// runtime entry block. Same-block ordering remains caller policy because it often depends on
    /// rule-local structural facts.
    #[doc(hidden)]
    pub fn binding_dominates_reference_from_flow_entry(
        &self,
        binding_id: BindingId,
        name: &Name,
        at: Span,
        same_block_dominates: bool,
    ) -> bool {
        let Some(reference_id) = self.reference_id_for_name_at(name, at) else {
            return false;
        };
        let Some(reference_block) = self.block_for_reference_id(reference_id) else {
            return false;
        };
        let Some(binding_block) = self.block_for_binding(binding_id) else {
            return false;
        };
        if binding_block == reference_block {
            return same_block_dominates;
        }

        let binding_scope = self.model.binding(binding_id).scope;
        let entry = self.flow_entry_block_for_binding_scopes(&[binding_scope], at.start.offset);
        self.blocks_cover_all_paths_to_block(
            entry,
            reference_block,
            &FxHashSet::from_iter([binding_block]),
        )
    }

    pub fn reaching_bindings_for_name(&self, name: &Name, at: Span) -> Vec<BindingId> {
        let cfg = self.cfg();
        let context = self.model.dataflow_context(cfg);
        let exact = self.exact_variable_dataflow();

        if let Some(reference) = self.reference_for_name_at(name, at) {
            let reaching = exact.reaching_bindings_for_reference(&context, reference);
            if !reaching.is_empty() {
                return reaching;
            }
        }

        self.model
            .visible_binding(name, at)
            .map(|binding| vec![binding.id])
            .unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn visible_bindings_bypassing(
        &self,
        name: &Name,
        binding_id: BindingId,
        at: Span,
    ) -> Vec<BindingId> {
        let cfg = self.cfg();
        let exact = self.exact_variable_dataflow();
        let Some(reference) = self.reference_for_name_at(name, at) else {
            return Vec::new();
        };
        let Some(reference_block) = exact.reference_block(reference) else {
            return Vec::new();
        };
        let Some(binding_block) = exact.binding_block(binding_id) else {
            return Vec::new();
        };
        if reference_block == binding_block {
            return Vec::new();
        }

        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        if unreachable.contains(&reference_block) || unreachable.contains(&binding_block) {
            return Vec::new();
        }

        self.model
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|other_binding| *other_binding != binding_id)
            .filter(|other_binding| self.model.binding_visible_at(*other_binding, at))
            .filter_map(|other_binding| {
                exact
                    .binding_block(other_binding)
                    .filter(|other_block| !unreachable.contains(other_block))
                    .filter(|other_block| {
                        block_reaches_without(cfg, *other_block, reference_block, binding_block)
                    })
                    .map(|_| other_binding)
            })
            .collect()
    }

    pub fn dead_code(&self) -> &[DeadCode] {
        self.dead_code
            .get_or_init(|| dataflow::analyze_dead_code(self.cfg()))
            .as_slice()
    }

    pub fn is_reachable(&self, span: &Span) -> bool {
        let cfg = self.cfg();
        cfg.block_ids_for_span(*span)
            .iter()
            .all(|block| !cfg.unreachable().contains(block))
    }

    #[doc(hidden)]
    pub fn scope_provided_bindings(&self, scope: ScopeId) -> &[ProvidedBinding] {
        self.scope_provided_binding_index()
            .provided_bindings_by_scope
            .get(scope.index())
            .map(Box::as_ref)
            .unwrap_or(&[])
    }

    #[doc(hidden)]
    pub fn definite_provider_scopes(&self, name: &Name) -> &[ScopeId] {
        self.scope_provided_binding_index()
            .definite_provider_scopes_by_name
            .get(name)
            .map(Box::as_ref)
            .unwrap_or(&[])
    }

    #[doc(hidden)]
    pub fn summarize_scope_provided_bindings(&self, scope: ScopeId) -> Vec<ProvidedBinding> {
        self.scope_provided_bindings(scope).to_vec()
    }

    pub(crate) fn summarize_scope_provided_functions(
        &self,
        scope: ScopeId,
    ) -> Vec<ProvidedBinding> {
        let cfg = self.cfg();
        let exact = self.exact_variable_dataflow();
        let context = self.model.dataflow_context(cfg);
        dataflow::summarize_scope_provided_functions(&context, exact, scope)
    }

    fn scope_provided_binding_index(&self) -> &ScopeProvidedBindingIndex {
        self.scope_provided_binding_index.get_or_init(|| {
            let cfg = self.cfg();
            let exact = self.exact_variable_dataflow();
            let context = self.model.dataflow_context(cfg);
            let mut provided_bindings_by_scope = Vec::with_capacity(self.model.scopes.len());
            let mut definite_provider_scopes_by_name = FxHashMap::<Name, Vec<ScopeId>>::default();

            for scope in self.model.scopes.iter().map(|scope| scope.id) {
                let provided_bindings =
                    dataflow::summarize_scope_provided_bindings(&context, exact, scope);
                for binding in &provided_bindings {
                    if binding.certainty == ContractCertainty::Definite {
                        definite_provider_scopes_by_name
                            .entry(binding.name.clone())
                            .or_default()
                            .push(scope);
                    }
                }
                provided_bindings_by_scope.push(provided_bindings.into_boxed_slice());
            }

            let definite_provider_scopes_by_name = definite_provider_scopes_by_name
                .into_iter()
                .map(|(name, scopes)| (name, scopes.into_boxed_slice()))
                .collect();

            ScopeProvidedBindingIndex {
                provided_bindings_by_scope,
                definite_provider_scopes_by_name,
            }
        })
    }
}
