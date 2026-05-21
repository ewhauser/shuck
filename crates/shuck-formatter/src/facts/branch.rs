use rustc_hash::FxHashSet as HashSet;

use super::FactSpan;

#[derive(Debug, Clone, Default)]
pub(super) struct BranchFacts {
    pub(super) inline_group_sequences: HashSet<FactSpan>,
    pub(super) inline_case_item_bodies: HashSet<FactSpan>,
}

impl BranchFacts {
    #[cfg(feature = "benchmarking")]
    pub(super) fn len(&self) -> usize {
        self.inline_group_sequences.len() + self.inline_case_item_bodies.len()
    }
}
