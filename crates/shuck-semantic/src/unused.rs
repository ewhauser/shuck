use super::*;
use crate::dataflow;

#[allow(missing_docs)]
impl SemanticModel {
    pub(crate) fn needs_precise_unused_assignments(&self) -> bool {
        if self.heuristic_unused_assignments.is_empty() {
            return false;
        }

        if !self.synthetic_reads.is_empty()
            || !self.entry_bindings.is_empty()
            || !self.indirect_targets_by_reference.is_empty()
        {
            return true;
        }

        let has_call_sites = !self.call_sites.is_empty();
        self.heuristic_unused_assignments.iter().any(|binding_id| {
            let binding = &self.bindings[binding_id.index()];
            self.runtime.is_always_used_binding(&binding.name)
                || self
                    .binding_index
                    .get(&binding.name)
                    .is_some_and(|binding_ids| binding_ids.len() > 1)
                || (has_call_sites
                    && matches!(
                        self.scopes[binding.scope.index()].kind,
                        ScopeKind::Function(_)
                    )
                    && !binding.attributes.contains(BindingAttributes::LOCAL))
        })
    }

    fn can_use_heuristic_unused_assignments_with_linear_cfg(&self, cfg: &ControlFlowGraph) -> bool {
        self.references.is_empty()
            && self.synthetic_reads.is_empty()
            && self.entry_bindings.is_empty()
            && self.indirect_targets_by_reference.is_empty()
            && self.call_sites.is_empty()
            && cfg.blocks().len() <= 1
            && !self.heuristic_unused_assignments.iter().any(|binding_id| {
                self.runtime
                    .is_always_used_binding(&self.bindings[binding_id.index()].name)
            })
    }
}

#[allow(missing_docs)]
impl<'model> SemanticAnalysis<'model> {
    /// Returns every binding that dataflow proves is never read again.
    ///
    /// Higher layers may still collapse mutually exclusive branch families down to a single
    /// diagnostic, but this accessor keeps the full dead-binding set so callers can make that
    /// policy decision with complete family context.
    pub fn unused_assignments(&self) -> &[BindingId] {
        if !self.model.needs_precise_unused_assignments() {
            return &self.model.heuristic_unused_assignments;
        }

        self.unused_assignments
            .get_or_init(|| {
                let cfg = self.cfg();
                if self
                    .model
                    .can_use_heuristic_unused_assignments_with_linear_cfg(cfg)
                {
                    return self.model.heuristic_unused_assignments.clone();
                }
                let context = self.model.dataflow_context(cfg);
                let exact = self.exact_variable_dataflow();
                dataflow::analyze_unused_assignments(&context, exact)
            })
            .as_slice()
    }

    /// Returns every dead binding using the requested behavior flags.
    pub fn unused_assignments_with_options(
        &self,
        options: UnusedAssignmentAnalysisOptions,
    ) -> &[BindingId] {
        if options == UnusedAssignmentAnalysisOptions::default() {
            return self.unused_assignments();
        }

        if !self.model.needs_precise_unused_assignments() {
            return &self.model.heuristic_unused_assignments;
        }

        self.unused_assignments_shellcheck_compat
            .get_or_init(|| {
                let cfg = self.cfg();
                if self
                    .model
                    .can_use_heuristic_unused_assignments_with_linear_cfg(cfg)
                {
                    return self.model.heuristic_unused_assignments.clone();
                }
                let context = self.model.dataflow_context(cfg);
                let exact = self.exact_variable_dataflow();
                dataflow::analyze_unused_assignments_with_options(&context, exact, options)
            })
            .as_slice()
    }
}
