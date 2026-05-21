use rustc_hash::FxHashSet as HashSet;

use super::FactSpan;

#[derive(Debug, Clone, Default)]
pub(super) struct BreakFacts {
    pub(super) pipeline: HashSet<FactSpan>,
    pub(super) list_item: HashSet<FactSpan>,
    pub(super) background: HashSet<FactSpan>,
}

impl BreakFacts {
    #[cfg(feature = "benchmarking")]
    pub(super) fn len(&self) -> usize {
        self.pipeline.len() + self.list_item.len() + self.background.len()
    }
}
