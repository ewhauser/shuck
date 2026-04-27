use super::*;
use crate::dataflow;

#[allow(missing_docs)]
impl<'model> SemanticAnalysis<'model> {
    pub fn uninitialized_references(&self) -> &[UninitializedReference] {
        self.uninitialized_references
            .get_or_init(|| {
                let cfg = self.cfg();
                let context = self.model.dataflow_context(cfg);
                let exact = self.exact_variable_dataflow();
                dataflow::analyze_uninitialized_references(&context, exact)
            })
            .as_slice()
    }

    pub(crate) fn reference_for_name_at(&self, name: &Name, at: Span) -> Option<&Reference> {
        let references = self.model.reference_index.get(name)?;
        let first_candidate = references.partition_point(|reference_id| {
            self.model.references[reference_id.index()]
                .span
                .start
                .offset
                < at.start.offset
        });

        references[first_candidate..]
            .iter()
            .find_map(|reference_id| {
                let reference = &self.model.references[reference_id.index()];
                (reference.span.start.offset >= at.start.offset
                    && reference.span.end.offset <= at.end.offset
                    && !matches!(
                        reference.kind,
                        ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
                    ))
                .then_some(reference)
            })
    }

    /// Returns the semantic reference for a named expansion contained by `at`.
    #[doc(hidden)]
    pub fn reference_id_for_name_at(&self, name: &Name, at: Span) -> Option<ReferenceId> {
        self.reference_for_name_at(name, at)
            .map(|reference| reference.id)
    }

    /// Returns the CFG block containing `reference_id`, if it was recorded in the CFG.
    #[doc(hidden)]
    pub fn block_for_reference_id(&self, reference_id: ReferenceId) -> Option<BlockId> {
        let exact = self.exact_variable_dataflow();
        exact.reference_block(&self.model.references[reference_id.index()])
    }

    /// Returns the CFG block containing `binding_id`, if it was recorded in the CFG.
    #[doc(hidden)]
    pub fn block_for_binding(&self, binding_id: BindingId) -> Option<BlockId> {
        self.exact_variable_dataflow().binding_block(binding_id)
    }
}
