use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Span;

use crate::{Binding, BindingId, BlockId, ControlFlowGraph, Reference, ReferenceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachingDefinitions {
    pub reaching_in: FxHashMap<BlockId, FxHashSet<BindingId>>,
    pub reaching_out: FxHashMap<BlockId, FxHashSet<BindingId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedAssignment {
    pub binding: BindingId,
    pub reason: UnusedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnusedReason {
    Overwritten { by: BindingId },
    ScopeEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninitializedReference {
    pub reference: ReferenceId,
    pub certainty: UninitializedCertainty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninitializedCertainty {
    Definite,
    Possible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadCode {
    pub unreachable: Vec<Span>,
    pub cause: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataflowResult {
    pub reaching_definitions: ReachingDefinitions,
    pub unused_assignments: Vec<UnusedAssignment>,
    pub uninitialized_references: Vec<UninitializedReference>,
    pub dead_code: Vec<DeadCode>,
    pub(crate) unused_assignment_ids: Vec<BindingId>,
}

impl DataflowResult {
    pub fn unused_assignment_ids(&self) -> &[BindingId] {
        &self.unused_assignment_ids
    }
}

pub(crate) fn analyze(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    references: &[Reference],
) -> DataflowResult {
    let block_ids = cfg.blocks().iter().map(|block| block.id).collect::<Vec<_>>();
    let mut reaching_in: FxHashMap<BlockId, FxHashSet<BindingId>> = FxHashMap::default();
    let mut reaching_out: FxHashMap<BlockId, FxHashSet<BindingId>> = FxHashMap::default();

    let gen_sets = block_ids
        .iter()
        .map(|block_id| (*block_id, gen_set(cfg, *block_id, bindings)))
        .collect::<FxHashMap<_, _>>();
    let kill_sets = block_ids
        .iter()
        .map(|block_id| (*block_id, kill_set(cfg, *block_id, bindings)))
        .collect::<FxHashMap<_, _>>();

    let mut changed = true;
    while changed {
        changed = false;
        for block_id in &block_ids {
            let incoming = cfg
                .predecessors(*block_id)
                .iter()
                .flat_map(|predecessor| reaching_out.get(predecessor).into_iter().flatten().copied())
                .collect::<FxHashSet<_>>();
            let outgoing = gen_sets
                .get(block_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .chain(
                    incoming
                        .iter()
                        .copied()
                        .filter(|binding| !kill_sets.get(block_id).is_some_and(|kills| kills.contains(binding))),
                )
                .collect::<FxHashSet<_>>();

            if reaching_in.get(block_id) != Some(&incoming) {
                reaching_in.insert(*block_id, incoming);
                changed = true;
            }
            if reaching_out.get(block_id) != Some(&outgoing) {
                reaching_out.insert(*block_id, outgoing);
                changed = true;
            }
        }
    }

    let reaching_definitions = ReachingDefinitions {
        reaching_in,
        reaching_out,
    };

    let reference_blocks = reference_blocks(cfg);
    let binding_blocks = binding_blocks(cfg);
    let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();

    let maybe_defined = block_ids
        .iter()
        .map(|block_id| {
            (
                *block_id,
                names_from_bindings(
                    reaching_definitions
                        .reaching_in
                        .get(block_id)
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .copied(),
                    bindings,
                ),
            )
        })
        .collect::<FxHashMap<_, _>>();

    let definitely_defined = block_ids
        .iter()
        .map(|block_id| {
            let predecessors = cfg.predecessors(*block_id);
            if predecessors.is_empty() {
                return (*block_id, FxHashSet::default());
            }
            let mut predecessor_sets = predecessors
                .iter()
                .map(|predecessor| {
                    names_from_bindings(
                        reaching_definitions
                            .reaching_out
                            .get(predecessor)
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .copied(),
                        bindings,
                    )
                })
                .collect::<Vec<_>>();
            let first = predecessor_sets.pop().unwrap_or_default();
            let intersection = predecessor_sets.into_iter().fold(first, |acc, set| {
                acc.intersection(&set).cloned().collect()
            });
            (*block_id, intersection)
        })
        .collect::<FxHashMap<_, _>>();

    let mut uninitialized_references = Vec::new();
    for reference in references {
        let Some(block_id) = reference_blocks.get(&reference.id).copied() else {
            continue;
        };
        if unreachable.contains(&block_id) {
            continue;
        }
        let maybe = maybe_defined
            .get(&block_id)
            .is_some_and(|names| names.contains(&reference.name));
        let definite = definitely_defined
            .get(&block_id)
            .is_some_and(|names| names.contains(&reference.name));

        if !maybe {
            uninitialized_references.push(UninitializedReference {
                reference: reference.id,
                certainty: UninitializedCertainty::Definite,
            });
        } else if !definite {
            uninitialized_references.push(UninitializedReference {
                reference: reference.id,
                certainty: UninitializedCertainty::Possible,
            });
        }
    }

    let mut used_bindings = FxHashSet::default();
    for reference in references {
        let Some(block_id) = reference_blocks.get(&reference.id).copied() else {
            continue;
        };
        if unreachable.contains(&block_id) {
            continue;
        }
        if let Some(incoming) = reaching_definitions.reaching_in.get(&block_id) {
            for binding in incoming {
                if bindings[binding.index()].name == reference.name {
                    used_bindings.insert(*binding);
                }
            }
        }
    }

    let mut unused_assignments = Vec::new();
    let mut unused_assignment_ids = Vec::new();
    for binding in bindings {
        let Some(block_id) = binding_blocks.get(&binding.id).copied() else {
            continue;
        };
        if unreachable.contains(&block_id) || used_bindings.contains(&binding.id) {
            continue;
        }

        let reason = next_overwrite(binding, bindings)
            .map(|by| UnusedReason::Overwritten { by })
            .unwrap_or(UnusedReason::ScopeEnd);
        unused_assignments.push(UnusedAssignment {
            binding: binding.id,
            reason,
        });
        unused_assignment_ids.push(binding.id);
    }

    let mut dead_code_by_cause: FxHashMap<(usize, usize), (Span, Vec<Span>)> = FxHashMap::default();
    for block_id in cfg.unreachable() {
        let block = cfg.block(*block_id);
        if block.commands.is_empty() {
            continue;
        }
        let cause = cfg
            .unreachable_cause(*block_id)
            .unwrap_or_else(|| block.commands[0]);
        dead_code_by_cause
            .entry((cause.start.offset, cause.end.offset))
            .or_insert_with(|| (cause, Vec::new()))
            .1
            .extend(block.commands.iter().copied());
    }
    let dead_code = dead_code_by_cause
        .into_iter()
        .map(|(_, (cause, unreachable))| DeadCode { unreachable, cause })
        .collect();

    DataflowResult {
        reaching_definitions,
        unused_assignments,
        uninitialized_references,
        dead_code,
        unused_assignment_ids,
    }
}

fn gen_set(cfg: &ControlFlowGraph, block_id: BlockId, bindings: &[Binding]) -> FxHashSet<BindingId> {
    let mut latest_by_name = FxHashMap::default();
    for binding in &cfg.block(block_id).bindings {
        latest_by_name.insert(bindings[binding.index()].name.clone(), *binding);
    }
    latest_by_name.into_values().collect()
}

fn kill_set(
    cfg: &ControlFlowGraph,
    block_id: BlockId,
    bindings: &[Binding],
) -> FxHashSet<BindingId> {
    let block = cfg.block(block_id);
    let names = block
        .bindings
        .iter()
        .map(|binding| bindings[binding.index()].name.clone())
        .collect::<FxHashSet<_>>();
    bindings
        .iter()
        .filter(|binding| names.contains(&binding.name) && !block.bindings.contains(&binding.id))
        .map(|binding| binding.id)
        .collect()
}

fn names_from_bindings(
    bindings_iter: impl Iterator<Item = BindingId>,
    bindings: &[Binding],
) -> FxHashSet<shuck_ast::Name> {
    bindings_iter
        .map(|binding| bindings[binding.index()].name.clone())
        .collect()
}

fn reference_blocks(cfg: &ControlFlowGraph) -> FxHashMap<ReferenceId, BlockId> {
    let mut map = FxHashMap::default();
    for block in cfg.blocks() {
        for reference in &block.references {
            map.insert(*reference, block.id);
        }
    }
    map
}

fn binding_blocks(cfg: &ControlFlowGraph) -> FxHashMap<BindingId, BlockId> {
    let mut map = FxHashMap::default();
    for block in cfg.blocks() {
        for binding in &block.bindings {
            map.insert(*binding, block.id);
        }
    }
    map
}

fn next_overwrite(binding: &Binding, bindings: &[Binding]) -> Option<BindingId> {
    bindings
        .iter()
        .filter(|candidate| {
            candidate.name == binding.name && candidate.span.start.offset > binding.span.start.offset
        })
        .min_by_key(|candidate| candidate.span.start.offset)
        .map(|candidate| candidate.id)
}
