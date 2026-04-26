use super::*;

const INDEX_BUILD_THRESHOLD: usize = 1024;

#[derive(Debug)]
pub(crate) struct PossibleVariableMisspellingIndex {
    bindings: MisspellingCandidateSet,
    presence_tests: MisspellingCandidateSet,
}

impl PossibleVariableMisspellingIndex {
    pub(crate) fn candidate_name(&self, target_name: &str) -> Option<&str> {
        self.bindings
            .candidate_name(target_name, CandidateTieBreak::Binding)
            .or_else(|| {
                self.presence_tests
                    .candidate_name(target_name, CandidateTieBreak::PresenceTest)
            })
    }
}

#[derive(Debug)]
struct MisspellingCandidateSet {
    entries: Vec<MisspellingCandidate>,
    lookup: OnceLock<MisspellingCandidateLookup>,
}

impl MisspellingCandidateSet {
    fn new(entries: Vec<MisspellingCandidate>) -> Self {
        Self {
            entries,
            lookup: OnceLock::new(),
        }
    }

    fn candidate_name(&self, target_name: &str, tie_break: CandidateTieBreak) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        if self.entries.len() < INDEX_BUILD_THRESHOLD {
            return scan_candidate_entries(&self.entries, target_name, tie_break);
        }

        self.lookup
            .get_or_init(|| MisspellingCandidateLookup::from_entries(&self.entries))
            .candidate_name(&self.entries, target_name, tie_break)
    }
}

#[derive(Debug)]
struct MisspellingCandidateLookup {
    casefold_exact: FxHashMap<Box<str>, SmallVec<[usize; 2]>>,
    edit1_deletions: FxHashMap<Box<str>, SmallVec<[usize; 4]>>,
    env_trie: MisspellingCandidateTrie,
}

#[derive(Debug)]
struct MisspellingCandidateTrie {
    nodes: Vec<MisspellingCandidateTrieNode>,
}

#[derive(Debug, Default)]
struct MisspellingCandidateTrieNode {
    children: SmallVec<[(u8, usize); 4]>,
    candidate_ids: SmallVec<[usize; 1]>,
}

impl MisspellingCandidateTrie {
    fn new() -> Self {
        Self {
            nodes: vec![MisspellingCandidateTrieNode::default()],
        }
    }

    fn insert(&mut self, name: &str, candidate_id: usize) {
        let mut node_id = 0;
        for byte in name.bytes() {
            if let Some((_, child_id)) = self.nodes[node_id]
                .children
                .iter()
                .find(|(child_byte, _)| *child_byte == byte)
            {
                node_id = *child_id;
                continue;
            }

            let child_id = self.nodes.len();
            self.nodes.push(MisspellingCandidateTrieNode::default());
            self.nodes[node_id].children.push((byte, child_id));
            node_id = child_id;
        }
        self.nodes[node_id].candidate_ids.push(candidate_id);
    }

    fn edit2_candidate_ids(&self, target_name: &str) -> SmallVec<[usize; 16]> {
        let target = target_name.as_bytes();
        let initial_row = (0..=target.len())
            .map(|index| u8::try_from(index).unwrap_or(3).min(3))
            .collect::<SmallVec<[u8; 32]>>();
        let mut ids = SmallVec::<[usize; 16]>::new();

        for (byte, child_id) in self.nodes[0].children.iter().copied() {
            self.collect_edit2_candidate_ids(child_id, byte, target, &initial_row, &mut ids);
        }

        ids
    }

    fn collect_edit2_candidate_ids(
        &self,
        node_id: usize,
        node_byte: u8,
        target: &[u8],
        previous_row: &[u8],
        ids: &mut SmallVec<[usize; 16]>,
    ) {
        let mut current_row = SmallVec::<[u8; 32]>::new();
        current_row.push(previous_row[0].saturating_add(1));
        let mut row_min = current_row[0];

        for (index, target_byte) in target.iter().enumerate() {
            let insertion = current_row[index].saturating_add(1);
            let deletion = previous_row[index + 1].saturating_add(1);
            let substitution = previous_row[index] + u8::from(*target_byte != node_byte);
            let value = insertion.min(deletion).min(substitution).min(3);
            current_row.push(value);
            row_min = row_min.min(value);
        }

        if current_row[target.len()] <= 2 {
            ids.extend_from_slice(&self.nodes[node_id].candidate_ids);
        }
        if row_min > 2 {
            return;
        }

        for (byte, child_id) in self.nodes[node_id].children.iter().copied() {
            self.collect_edit2_candidate_ids(child_id, byte, target, &current_row, ids);
        }
    }
}

impl MisspellingCandidateLookup {
    fn from_entries(entries: &[MisspellingCandidate]) -> Self {
        let mut index = Self {
            casefold_exact: FxHashMap::default(),
            edit1_deletions: FxHashMap::default(),
            env_trie: MisspellingCandidateTrie::new(),
        };

        for (id, entry) in entries.iter().enumerate() {
            let name = entry.name.as_str();
            index
                .casefold_exact
                .entry(canonical_ascii_uppercase(name).into_boxed_str())
                .or_default()
                .push(id);

            if !is_environment_style_name(name) || name.len() < 4 {
                continue;
            }

            index.env_trie.insert(name, id);
            for key in edit1_deletion_keys(name) {
                index.edit1_deletions.entry(key).or_default().push(id);
            }
        }

        index
    }

    fn candidate_name<'a>(
        &self,
        entries: &'a [MisspellingCandidate],
        target_name: &str,
        tie_break: CandidateTieBreak,
    ) -> Option<&'a str> {
        if let Some(best) = self.best_exact(entries, target_name, tie_break) {
            return Some(best);
        }
        if target_name.len() < 3 {
            return None;
        }
        if let Some(best) = self.best_edit1(entries, target_name, tie_break) {
            return Some(best);
        }
        self.best_edit2_strong_shape(entries, target_name, tie_break)
    }

    fn best_exact<'a>(
        &self,
        entries: &'a [MisspellingCandidate],
        target_name: &str,
        tie_break: CandidateTieBreak,
    ) -> Option<&'a str> {
        if target_name.len() < 4 {
            return None;
        }
        let ids = self
            .casefold_exact
            .get(canonical_ascii_uppercase(target_name).as_str())?;
        self.best_from_ids(
            entries,
            target_name,
            tie_break,
            ids.iter().copied(),
            Some(0),
        )
    }

    fn best_edit1<'a>(
        &self,
        entries: &'a [MisspellingCandidate],
        target_name: &str,
        tie_break: CandidateTieBreak,
    ) -> Option<&'a str> {
        let mut ids = SmallVec::<[usize; 16]>::new();
        for key in edit1_deletion_keys(target_name) {
            if let Some(key_ids) = self.edit1_deletions.get(&key) {
                ids.extend_from_slice(key_ids);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        self.best_from_ids(entries, target_name, tie_break, ids.into_iter(), Some(2))
    }

    fn best_edit2_strong_shape<'a>(
        &self,
        entries: &'a [MisspellingCandidate],
        target_name: &str,
        tie_break: CandidateTieBreak,
    ) -> Option<&'a str> {
        let ids = self.env_trie.edit2_candidate_ids(target_name);
        self.best_from_ids(entries, target_name, tie_break, ids.into_iter(), Some(3))
    }

    fn best_from_ids<'a>(
        &self,
        entries: &'a [MisspellingCandidate],
        target_name: &str,
        tie_break: CandidateTieBreak,
        ids: impl IntoIterator<Item = usize>,
        required_rank: Option<u8>,
    ) -> Option<&'a str> {
        ids.into_iter()
            .filter_map(|id| {
                let entry = &entries[id];
                if entry.name == target_name {
                    return None;
                }
                let rank = candidate_match_rank(target_name, entry.name.as_str())?;
                if required_rank.is_some_and(|required| rank != required) {
                    return None;
                }
                Some((id, rank, entry))
            })
            .min_by(|left, right| compare_candidates(*left, *right, tie_break))
            .map(|(_, _, entry)| entry.name.as_str())
    }
}

#[derive(Debug, Clone)]
struct MisspellingCandidate {
    name: String,
    first_span: Span,
}

#[derive(Debug, Clone, Copy)]
enum CandidateTieBreak {
    Binding,
    PresenceTest,
}

pub(super) fn build_possible_variable_misspelling_index(
    semantic: &SemanticModel,
    presence_test_references_by_name: &FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: &FxHashMap<Name, Vec<PresenceTestNameFact>>,
) -> PossibleVariableMisspellingIndex {
    let binding_entries = semantic
        .bindings()
        .iter()
        .filter(|binding| is_sc2154_defining_binding(binding.kind))
        .filter(|binding| binding.name.as_str().len() >= 4)
        .map(|binding| MisspellingCandidate {
            name: binding.name.to_string(),
            first_span: binding.span,
        })
        .collect();
    let presence_entries = build_presence_test_entries(
        semantic,
        presence_test_references_by_name,
        presence_test_names_by_name,
    );

    PossibleVariableMisspellingIndex {
        bindings: MisspellingCandidateSet::new(binding_entries),
        presence_tests: MisspellingCandidateSet::new(presence_entries),
    }
}

pub(super) fn should_scan_possible_variable_misspelling_candidates(
    semantic: &SemanticModel,
    presence_test_references_by_name: &FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: &FxHashMap<Name, Vec<PresenceTestNameFact>>,
) -> bool {
    let raw_candidate_count = semantic.bindings().len()
        + presence_test_references_by_name.len()
        + presence_test_names_by_name.len();
    if raw_candidate_count < INDEX_BUILD_THRESHOLD {
        return true;
    }

    let binding_count = semantic
        .bindings()
        .iter()
        .filter(|binding| is_sc2154_defining_binding(binding.kind))
        .filter(|binding| binding.name.as_str().len() >= 4)
        .take(INDEX_BUILD_THRESHOLD)
        .count();
    if binding_count >= INDEX_BUILD_THRESHOLD {
        return false;
    }

    binding_count + presence_test_references_by_name.len() + presence_test_names_by_name.len()
        < INDEX_BUILD_THRESHOLD
}

pub(super) fn scan_possible_variable_misspelling_candidate(
    semantic: &SemanticModel,
    presence_test_references_by_name: &FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: &FxHashMap<Name, Vec<PresenceTestNameFact>>,
    target_name: &str,
) -> Option<String> {
    semantic
        .bindings()
        .iter()
        .filter(|binding| is_sc2154_defining_binding(binding.kind))
        .filter(|binding| binding.name.as_str() != target_name)
        .filter(|binding| binding.name.as_str().len() >= 4)
        .filter_map(|binding| {
            candidate_match_rank(target_name, binding.name.as_str()).map(|rank| {
                (
                    rank,
                    binding.span.start.offset,
                    binding.span.end.offset,
                    binding.name.as_str(),
                )
            })
        })
        .min_by_key(|(rank, start, end, _)| (*rank, *start, *end))
        .map(|(_, _, _, name)| name.to_owned())
        .or_else(|| {
            scan_presence_tested_candidate_name(
                semantic,
                presence_test_references_by_name,
                presence_test_names_by_name,
                target_name,
            )
            .map(ToOwned::to_owned)
        })
}

fn scan_presence_tested_candidate_name<'a>(
    semantic: &SemanticModel,
    presence_test_references_by_name: &'a FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: &'a FxHashMap<Name, Vec<PresenceTestNameFact>>,
    target_name: &str,
) -> Option<&'a str> {
    presence_test_references_by_name
        .keys()
        .chain(presence_test_names_by_name.keys())
        .filter(|candidate_name| candidate_name.as_str() != target_name)
        .filter_map(|candidate_name| {
            let first_span = first_presence_test_span(
                semantic,
                candidate_name,
                presence_test_references_by_name,
                presence_test_names_by_name,
            )?;
            candidate_match_rank(target_name, candidate_name.as_str()).map(|rank| {
                (
                    rank,
                    first_span.start.offset,
                    first_span.end.offset,
                    candidate_name.as_str(),
                )
            })
        })
        .min_by(|left, right| {
            (left.0, left.1, left.2)
                .cmp(&(right.0, right.1, right.2))
                .then_with(|| left.3.cmp(right.3))
        })
        .map(|(_, _, _, name)| name)
}

fn scan_candidate_entries<'a>(
    entries: &'a [MisspellingCandidate],
    target_name: &str,
    tie_break: CandidateTieBreak,
) -> Option<&'a str> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(id, entry)| {
            if entry.name == target_name {
                return None;
            }
            let rank = candidate_match_rank(target_name, entry.name.as_str())?;
            Some((id, rank, entry))
        })
        .min_by(|left, right| compare_candidates(*left, *right, tie_break))
        .map(|(_, _, entry)| entry.name.as_str())
}

fn build_presence_test_entries(
    semantic: &SemanticModel,
    presence_test_references_by_name: &FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: &FxHashMap<Name, Vec<PresenceTestNameFact>>,
) -> Vec<MisspellingCandidate> {
    let mut names = FxHashSet::<Name>::default();
    names.extend(presence_test_references_by_name.keys().cloned());
    names.extend(presence_test_names_by_name.keys().cloned());

    names
        .into_iter()
        .filter_map(|name| {
            let first_span = first_presence_test_span(
                semantic,
                &name,
                presence_test_references_by_name,
                presence_test_names_by_name,
            )?;
            Some(MisspellingCandidate {
                name: name.to_string(),
                first_span,
            })
        })
        .collect()
}

fn first_presence_test_span(
    semantic: &SemanticModel,
    candidate_name: &Name,
    presence_test_references_by_name: &FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: &FxHashMap<Name, Vec<PresenceTestNameFact>>,
) -> Option<Span> {
    presence_test_references_by_name
        .get(candidate_name)
        .into_iter()
        .flatten()
        .map(|presence| semantic.reference(presence.reference_id()).span)
        .chain(
            presence_test_names_by_name
                .get(candidate_name)
                .into_iter()
                .flatten()
                .map(|presence| presence.tested_span()),
        )
        .min_by_key(|span| (span.start.offset, span.end.offset))
}

fn compare_candidates(
    left: (usize, u8, &MisspellingCandidate),
    right: (usize, u8, &MisspellingCandidate),
    tie_break: CandidateTieBreak,
) -> std::cmp::Ordering {
    let (_, left_rank, left_entry) = left;
    let (_, right_rank, right_entry) = right;
    let ordering = (
        left_rank,
        left_entry.first_span.start.offset,
        left_entry.first_span.end.offset,
    )
        .cmp(&(
            right_rank,
            right_entry.first_span.start.offset,
            right_entry.first_span.end.offset,
        ));
    if !ordering.is_eq() {
        return ordering;
    }

    match tie_break {
        CandidateTieBreak::Binding => left.0.cmp(&right.0),
        CandidateTieBreak::PresenceTest => left_entry.name.cmp(&right_entry.name),
    }
}

fn candidate_match_rank(target_name: &str, candidate_name: &str) -> Option<u8> {
    if target_name.len() >= 4
        && target_name.len() == candidate_name.len()
        && candidate_name
            .as_bytes()
            .eq_ignore_ascii_case(target_name.as_bytes())
    {
        return Some(0);
    }

    if !is_environment_style_name(candidate_name)
        || target_name.len() < 3
        || candidate_name.len() < 4
        || target_name.len().abs_diff(candidate_name.len()) > 2
    {
        return None;
    }

    if ascii_edit_distance_at_most(target_name.as_bytes(), candidate_name.as_bytes(), 1) {
        return Some(2);
    }
    if !has_strong_two_edit_shape(target_name, candidate_name) {
        return None;
    }
    ascii_edit_distance_at_most(target_name.as_bytes(), candidate_name.as_bytes(), 2).then_some(3)
}

fn has_strong_two_edit_shape(target_name: &str, candidate_upper: &str) -> bool {
    let common_prefix = common_prefix_len(target_name.as_bytes(), candidate_upper.as_bytes());
    let common_suffix = common_suffix_len(
        &target_name.as_bytes()[common_prefix..],
        &candidate_upper.as_bytes()[common_prefix..],
    );

    matches!((target_name, candidate_upper), ("CFLAGS", "CXXFLAGS"))
        || matches!((target_name, candidate_upper), ("OS_NAME", "HOSTNAME"))
        || has_separator_plural_compaction(target_name, candidate_upper)
        || common_prefix >= 5
        || common_suffix >= 5
        || (common_prefix >= 4 && common_suffix >= 4)
}

fn has_separator_plural_compaction(left: &str, right: &str) -> bool {
    compacted_plural_matches(left, right) || compacted_plural_matches(right, left)
}

fn compacted_plural_matches(pluralish_name: &str, compact_singular_name: &str) -> bool {
    let Some((prefix, last_segment)) = pluralish_name.rsplit_once('_') else {
        return false;
    };
    let Some(singular_segment) = last_segment.strip_suffix('S') else {
        return false;
    };
    if compacted_len(prefix) < 4 || singular_segment.len() < 4 {
        return false;
    }

    let compacted = pluralish_name
        .chars()
        .filter(|char| *char != '_')
        .collect::<String>();
    compacted
        .strip_suffix('S')
        .is_some_and(|singular| singular == compact_singular_name)
}

fn compacted_len(name: &str) -> usize {
    name.chars().filter(|char| *char != '_').count()
}

fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn common_suffix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn ascii_edit_distance_at_most(left: &[u8], right: &[u8], max_distance: u8) -> bool {
    if left.len().abs_diff(right.len()) > usize::from(max_distance) {
        return false;
    }
    ascii_edit_distance_at_most_inner(left, right, max_distance)
}

fn ascii_edit_distance_at_most_inner(mut left: &[u8], mut right: &[u8], edits_left: u8) -> bool {
    while let (Some((&left_byte, left_rest)), Some((&right_byte, right_rest))) =
        (left.split_first(), right.split_first())
    {
        if left_byte != right_byte {
            break;
        }
        left = left_rest;
        right = right_rest;
    }

    while let (Some((&left_byte, left_rest)), Some((&right_byte, right_rest))) =
        (left.split_last(), right.split_last())
    {
        if left_byte != right_byte {
            break;
        }
        left = left_rest;
        right = right_rest;
    }

    if left.is_empty() || right.is_empty() {
        return left.len().max(right.len()) <= usize::from(edits_left);
    }
    if edits_left == 0 || left.len().abs_diff(right.len()) > usize::from(edits_left) {
        return false;
    }

    ascii_edit_distance_at_most_inner(&left[1..], right, edits_left - 1)
        || ascii_edit_distance_at_most_inner(left, &right[1..], edits_left - 1)
        || ascii_edit_distance_at_most_inner(&left[1..], &right[1..], edits_left - 1)
}

fn edit1_deletion_keys(name: &str) -> Vec<Box<str>> {
    let bytes = name.as_bytes();
    let mut keys = Vec::with_capacity(bytes.len() + 1);
    keys.push(name.into());
    for skip in 0..bytes.len() {
        let mut key = String::with_capacity(bytes.len().saturating_sub(1));
        key.push_str(&name[..skip]);
        key.push_str(&name[skip + 1..]);
        keys.push(key.into_boxed_str());
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn canonical_ascii_uppercase(name: &str) -> String {
    name.chars().map(|char| char.to_ascii_uppercase()).collect()
}

fn is_environment_style_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|char| char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_')
}

fn is_sc2154_defining_binding(kind: BindingKind) -> bool {
    !matches!(
        kind,
        BindingKind::FunctionDefinition | BindingKind::Imported
    )
}
