//! CFG construction from the recorded semantic command stream.
//!
//! This module decides the block and edge shape for shell syntax. For example:
//!
//! ```sh
//! while read line; do
//!   [[ $line == skip ]] && continue
//!   printf '%s\n' "$line"
//! done
//! echo done
//! ```
//!
//! The loop condition flows either into the body or to the block after the loop,
//! `continue` points back to the condition, and ordinary body exits loop back
//! after `printf`. Those edges give later analyses a shell-shaped graph without
//! asking each analysis to rediscover loop, branch, pipeline, or `case` control
//! flow from syntax.

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{CaseTerminator, Name, Span};
use smallvec::{SmallVec, smallvec};

use crate::cfg::termination::compute_script_terminating_call_spans;
use crate::cfg::{
    BasicBlock, BlockId, CommandId, ControlFlowGraph, EdgeKind, FlowEdge, RecordedCaseArmRange,
    RecordedCommandKind, RecordedCommandRange, RecordedElifBranchRange, RecordedListItemRange,
    RecordedListOperator, RecordedProgram, RecordedRegionRange, UnreachableCause,
    UnreachableCauseKind,
};
use crate::{Binding, BindingId, CallSite, ReferenceId, Scope, ScopeId, SpanKey};

#[derive(Debug, Clone, Copy)]
struct FlatListSegment {
    operator_before: Option<(RecordedListOperator, Span)>,
    command: CommandId,
}

struct SequenceResult {
    entry: Option<BlockId>,
    exits: SmallVec<[BlockId; 2]>,
    terminal_cause: Option<UnreachableCause>,
}

fn merge_terminal_causes<const N: usize>(
    fallback_span: Span,
    causes: [Option<UnreachableCause>; N],
) -> Option<UnreachableCause> {
    let mut merged: Option<UnreachableCause> = None;
    for cause in causes.into_iter().flatten() {
        merged = Some(match merged {
            None => cause,
            Some(existing)
                if existing.kind == UnreachableCauseKind::LoopControl
                    && cause.kind == UnreachableCauseKind::LoopControl =>
            {
                existing
            }
            Some(_) => UnreachableCause::shell_terminator(fallback_span),
        });
    }
    merged
}

#[derive(Clone, Copy)]
struct IfRanges {
    condition: RecordedCommandRange,
    then_branch: RecordedCommandRange,
    elif_branches: RecordedElifBranchRange,
    else_branch: RecordedCommandRange,
}

#[derive(Clone, Copy)]
struct LoopTarget {
    continue_target: BlockId,
    break_target: BlockId,
}

struct GraphBuilder<'a> {
    program: &'a RecordedProgram,
    command_bindings: &'a FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: &'a FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    script_terminating_calls: FxHashSet<SpanKey>,
    blocks: Vec<BasicBlock>,
    successors: Vec<SmallVec<[FlowEdge; 2]>>,
    command_blocks: FxHashMap<SpanKey, SmallVec<[BlockId; 1]>>,
    unreachable_causes: FxHashMap<BlockId, UnreachableCause>,
    scope_entries: FxHashMap<ScopeId, BlockId>,
    script_terminators: Vec<BlockId>,
}

pub(crate) fn build_control_flow_graph(
    program: &RecordedProgram,
    command_bindings: &FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: &FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    scopes: &[Scope],
    bindings: &[Binding],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
) -> ControlFlowGraph {
    let script_terminating_calls = compute_script_terminating_call_spans(
        program,
        bindings,
        call_sites,
        visible_function_call_bindings,
    );
    let command_count = program.commands().len();
    let scope_count = scopes.len().max(1);
    let mut builder = GraphBuilder {
        program,
        command_bindings,
        command_references,
        script_terminating_calls,
        blocks: Vec::with_capacity(command_count),
        successors: Vec::with_capacity(command_count),
        command_blocks: FxHashMap::with_capacity_and_hasher(command_count, Default::default()),
        unreachable_causes: FxHashMap::default(),
        scope_entries: FxHashMap::with_capacity_and_hasher(scope_count, Default::default()),
        script_terminators: Vec::new(),
    };

    let file = builder.build_sequence(program.file_commands(), &[]);
    let entry = file.entry.unwrap_or_else(|| builder.empty_block());
    builder.scope_entries.insert(ScopeId(0), entry);
    let script_always_terminates = file.exits.is_empty() && file.entry.is_some();
    let mut natural_exits: Vec<BlockId> = if file.entry.is_none() {
        vec![entry]
    } else {
        file.exits.iter().copied().collect()
    };
    let file_exits = if file.exits.is_empty() {
        smallvec![entry]
    } else {
        file.exits.clone()
    };
    let mut scope_exits = FxHashMap::with_capacity_and_hasher(scope_count, Default::default());
    scope_exits.insert(ScopeId(0), file_exits.clone());

    let mut exits = file_exits.iter().copied().collect::<Vec<_>>();

    for (scope, commands) in program.function_bodies() {
        let function = builder.build_sequence(*commands, &[]);
        let function_entry = function.entry.unwrap_or_else(|| builder.empty_block());
        builder.scope_entries.insert(*scope, function_entry);
        if function.entry.is_none() {
            natural_exits.push(function_entry);
        } else {
            natural_exits.extend(function.exits.iter().copied());
        }
        let function_exits = if function.exits.is_empty() {
            smallvec![function_entry]
        } else {
            function.exits
        };
        scope_exits.insert(*scope, function_exits.clone());
        exits.extend(function_exits.iter().copied());
    }

    let unreachable =
        compute_unreachable(&builder.blocks, &builder.scope_entries, &builder.successors);
    let predecessors = derive_predecessors(&builder.successors);

    ControlFlowGraph {
        blocks: builder.blocks,
        successors: builder.successors,
        predecessors,
        entry,
        exits,
        natural_exits,
        script_terminators: builder.script_terminators,
        script_always_terminates,
        unreachable,
        scope_entries: builder.scope_entries,
        scope_exits,
        command_blocks: builder.command_blocks,
        unreachable_causes: builder.unreachable_causes,
    }
}

impl<'a> GraphBuilder<'a> {
    fn build_sequence(
        &mut self,
        commands: RecordedCommandRange,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let mut entry = None;
        let mut pending = SmallVec::<[BlockId; 2]>::new();
        let mut unreachable_cause = None;

        for &command_id in self.program.commands_in(commands) {
            let command = self.program.command(command_id);
            let start = self.blocks.len();
            let sequence = self.build_command(command_id, loops, unreachable_cause.is_some());
            if entry.is_none() {
                entry = sequence.entry;
            }
            if let Some(command_entry) = sequence.entry {
                if let Some(cause) = unreachable_cause {
                    for block in &self.blocks[start..] {
                        self.unreachable_causes.insert(block.id, cause);
                    }
                } else {
                    for block in &pending {
                        self.add_edge(*block, command_entry, EdgeKind::Sequential);
                    }
                }
            }

            if sequence.exits.is_empty() {
                pending.clear();
                unreachable_cause.get_or_insert_with(|| {
                    sequence
                        .terminal_cause
                        .unwrap_or_else(|| UnreachableCause::shell_terminator(command.span))
                });
            } else {
                pending = sequence.exits;
            }
        }

        let terminal_cause = if pending.is_empty() {
            unreachable_cause
        } else {
            None
        };

        SequenceResult {
            entry,
            exits: pending,
            terminal_cause,
        }
    }

    fn build_command(
        &mut self,
        command_id: CommandId,
        loops: &[LoopTarget],
        force_command_header: bool,
    ) -> SequenceResult {
        let command = self.program.command(command_id);
        match &command.kind {
            RecordedCommandKind::Linear => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, command.nested_regions, loops);
                let exits = if self
                    .script_terminating_calls
                    .contains(&SpanKey::new(command.span))
                {
                    self.script_terminators.push(block);
                    SmallVec::new()
                } else {
                    smallvec![block]
                };
                SequenceResult {
                    entry: Some(block),
                    exits,
                    terminal_cause: if self
                        .script_terminating_calls
                        .contains(&SpanKey::new(command.span))
                    {
                        Some(UnreachableCause::shell_terminator(command.span))
                    } else {
                        None
                    },
                }
            }
            RecordedCommandKind::Break { depth } => {
                let block = self.command_block(command.span);
                if let Some(target) = resolve_break_target(loops, *depth) {
                    self.add_edge(block, target.break_target, EdgeKind::LoopExit);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: SmallVec::new(),
                    terminal_cause: Some(UnreachableCause::loop_control(command.span)),
                }
            }
            RecordedCommandKind::Continue { depth } => {
                let block = self.command_block(command.span);
                if let Some(target) = resolve_break_target(loops, *depth) {
                    self.add_edge(block, target.continue_target, EdgeKind::LoopBack);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: SmallVec::new(),
                    terminal_cause: Some(UnreachableCause::loop_control(command.span)),
                }
            }
            RecordedCommandKind::Return | RecordedCommandKind::Exit => {
                let block = self.command_block(command.span);
                if matches!(command.kind, RecordedCommandKind::Exit) {
                    self.script_terminators.push(block);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: SmallVec::new(),
                    terminal_cause: Some(UnreachableCause::shell_terminator(command.span)),
                }
            }
            RecordedCommandKind::List { first, rest } => {
                self.build_list(command_id, *first, *rest, loops, force_command_header)
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => self.build_if(
                command_id,
                IfRanges {
                    condition: *condition,
                    then_branch: *then_branch,
                    elif_branches: *elif_branches,
                    else_branch: *else_branch,
                },
                loops,
                force_command_header,
            ),
            RecordedCommandKind::While { condition, body } => self.build_while_like(
                command_id,
                *condition,
                *body,
                loops,
                true,
                force_command_header,
            ),
            RecordedCommandKind::Until { condition, body } => self.build_while_like(
                command_id,
                *condition,
                *body,
                loops,
                false,
                force_command_header,
            ),
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body } => {
                self.build_loop_command(command_id, *body, loops)
            }
            RecordedCommandKind::Case { arms } => self.build_case(command_id, *arms, loops),
            RecordedCommandKind::BraceGroup { body } => {
                let sequence = self.build_sequence(*body, loops);
                self.wrap_sequence_with_command_header(
                    command_id,
                    sequence,
                    loops,
                    force_command_header,
                )
            }
            RecordedCommandKind::Always { body, always_body } => {
                self.build_always(command_id, *body, *always_body, loops, force_command_header)
            }
            RecordedCommandKind::Subshell { body, .. } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, command.nested_regions, loops);
                let body_sequence = self.build_sequence(*body, loops);
                if let Some(body_entry) = body_sequence.entry {
                    self.add_edge(block, body_entry, EdgeKind::NestedRegion);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: smallvec![block],
                    terminal_cause: None,
                }
            }
            RecordedCommandKind::Pipeline { segments } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, command.nested_regions, loops);
                for segment in self.program.pipeline_segments(*segments) {
                    let sequence = self.build_command(segment.command, loops, false);
                    if let Some(segment_entry) = sequence.entry {
                        self.scope_entries
                            .entry(segment.scope)
                            .or_insert(segment_entry);
                        self.add_edge(block, segment_entry, EdgeKind::NestedRegion);
                    }
                }
                SequenceResult {
                    entry: Some(block),
                    exits: smallvec![block],
                    terminal_cause: None,
                }
            }
        }
    }

    fn build_always(
        &mut self,
        command_id: CommandId,
        body: RecordedCommandRange,
        always_body: RecordedCommandRange,
        loops: &[LoopTarget],
        force_command_header: bool,
    ) -> SequenceResult {
        let body_start = self.blocks.len();
        let body_sequence = self.build_sequence(body, loops);
        let body_end = self.blocks.len();
        let always_sequence = self.build_sequence(always_body, loops);

        if let Some(always_entry) = always_sequence.entry {
            if body_sequence.exits.is_empty() {
                let terminal_blocks = self.terminal_leaf_blocks(body_start, body_end);
                for block in terminal_blocks {
                    self.add_edge(block, always_entry, EdgeKind::Sequential);
                }
            } else {
                for block in &body_sequence.exits {
                    self.add_edge(*block, always_entry, EdgeKind::Sequential);
                }
            }
        }

        let exits = if body_sequence.exits.is_empty() {
            SmallVec::new()
        } else {
            always_sequence.exits
        };
        let terminal_cause = if exits.is_empty() {
            if body_sequence.exits.is_empty() {
                body_sequence.terminal_cause
            } else {
                always_sequence.terminal_cause
            }
        } else {
            None
        };
        let entry = body_sequence.entry.or(always_sequence.entry);
        self.wrap_sequence_with_command_header(
            command_id,
            SequenceResult {
                entry,
                exits,
                terminal_cause,
            },
            loops,
            force_command_header,
        )
    }

    fn terminal_leaf_blocks(&self, start: usize, end: usize) -> SmallVec<[BlockId; 2]> {
        self.blocks[start..end]
            .iter()
            .filter(|block| self.successors[block.id.index()].is_empty())
            .map(|block| block.id)
            .collect()
    }

    fn wrap_sequence_with_command_header(
        &mut self,
        command_id: CommandId,
        mut sequence: SequenceResult,
        loops: &[LoopTarget],
        force_command_header: bool,
    ) -> SequenceResult {
        let command = self.program.command(command_id);
        let key = SpanKey::new(command.span);
        let has_direct_command_facts =
            self.command_bindings.contains_key(&key) || self.command_references.contains_key(&key);
        if command.nested_regions.is_empty() && !force_command_header && !has_direct_command_facts {
            return sequence;
        }

        let header = self.command_block(command.span);
        self.attach_nested_regions(header, command.nested_regions, loops);
        if let Some(entry) = sequence.entry {
            self.add_edge(header, entry, EdgeKind::Sequential);
        } else {
            sequence.exits = smallvec![header];
            sequence.terminal_cause = None;
        }
        sequence.entry = Some(header);
        sequence
    }

    fn build_list(
        &mut self,
        command: CommandId,
        first: CommandId,
        rest: RecordedListItemRange,
        loops: &[LoopTarget],
        force_command_header: bool,
    ) -> SequenceResult {
        let mut segments = Vec::new();
        collect_flat_list_segments(self.program, first, None, &mut segments);
        for item in self.program.list_items(rest) {
            collect_flat_list_segments(
                self.program,
                item.command,
                Some((item.operator, item.operator_span)),
                &mut segments,
            );
        }

        let mut segments = segments.into_iter();
        let first = segments
            .next()
            .expect("recorded logical list has at least one segment");
        let current = self.build_command(first.command, loops, false);
        let entry = current.entry;
        let mut success_exits = current.exits.clone();
        let mut failure_exits = current.exits.clone();
        let mut success_cause = current
            .exits
            .is_empty()
            .then_some(current.terminal_cause)
            .flatten();
        let mut failure_cause = success_cause;

        for item in segments {
            let (operator, _) = item
                .operator_before
                .expect("logical list rest segment has an operator");
            let next_start = self.blocks.len();
            let next = self.build_command(item.command, loops, false);
            let (triggering_exits, triggering_cause, edge_kind) = match operator {
                RecordedListOperator::And => {
                    (&success_exits, success_cause, EdgeKind::ConditionalTrue)
                }
                RecordedListOperator::Or => {
                    (&failure_exits, failure_cause, EdgeKind::ConditionalFalse)
                }
            };

            if let Some(next_entry) = next.entry {
                if triggering_exits.is_empty() {
                    if let Some(cause) = triggering_cause {
                        self.mark_unreachable_blocks_since(next_start, cause);
                    }
                } else {
                    for exit in triggering_exits {
                        self.add_edge(*exit, next_entry, edge_kind);
                    }
                }
            }

            let next_reachable = next.entry.is_some() && !triggering_exits.is_empty();
            let (next_success_exits, mut next_failure_exits, next_terminal_cause) =
                if next_reachable {
                    (next.exits.clone(), next.exits, next.terminal_cause)
                } else {
                    (SmallVec::new(), SmallVec::new(), triggering_cause)
                };
            let next_success_cause = next_success_exits
                .is_empty()
                .then_some(next_terminal_cause)
                .flatten();
            let next_failure_cause = next_failure_exits
                .is_empty()
                .then_some(next_terminal_cause)
                .flatten();

            match operator {
                RecordedListOperator::And => {
                    let existing_failure_cause = failure_exits.is_empty().then_some(failure_cause);
                    append_unique_block_ids(&mut failure_exits, &next_failure_exits);
                    failure_cause = if failure_exits.is_empty() {
                        merge_terminal_causes(
                            self.program.command(command).span,
                            [existing_failure_cause.flatten(), next_failure_cause],
                        )
                    } else {
                        None
                    };
                    success_exits = next_success_exits;
                    success_cause = next_success_cause;
                }
                RecordedListOperator::Or => {
                    let existing_success_cause = success_exits.is_empty().then_some(success_cause);
                    append_unique_block_ids(&mut success_exits, &next_success_exits);
                    success_cause = if success_exits.is_empty() {
                        merge_terminal_causes(
                            self.program.command(command).span,
                            [existing_success_cause.flatten(), next_success_cause],
                        )
                    } else {
                        None
                    };
                    failure_exits = std::mem::take(&mut next_failure_exits);
                    failure_cause = next_failure_cause;
                }
            }
        }

        let mut exits = success_exits;
        append_unique_block_ids(&mut exits, &failure_exits);
        let terminal_cause = if exits.is_empty() {
            merge_terminal_causes(
                self.program.command(command).span,
                [success_cause, failure_cause],
            )
        } else {
            None
        };
        self.wrap_sequence_with_command_header(
            command,
            SequenceResult {
                entry,
                exits,
                terminal_cause,
            },
            loops,
            force_command_header,
        )
    }

    fn mark_unreachable_blocks_since(&mut self, start: usize, cause: UnreachableCause) {
        for block in &self.blocks[start..] {
            self.unreachable_causes.insert(block.id, cause);
        }
    }

    fn build_if(
        &mut self,
        command: CommandId,
        ranges: IfRanges,
        loops: &[LoopTarget],
        force_command_header: bool,
    ) -> SequenceResult {
        let condition_seq = self.build_sequence(ranges.condition, loops);
        let entry = condition_seq.entry.or_else(|| Some(self.empty_block()));
        let mut false_exits = condition_seq.exits.clone();
        let mut false_cause = false_exits
            .is_empty()
            .then_some(condition_seq.terminal_cause)
            .flatten();

        let then_start = self.blocks.len();
        let then_seq = self.build_sequence(ranges.then_branch, loops);
        if false_exits.is_empty()
            && let Some(cause) = false_cause
        {
            self.mark_unreachable_blocks_since(then_start, cause);
        }
        if let (Some(_cond_entry), Some(then_entry)) = (entry, then_seq.entry) {
            for exit in &condition_seq.exits {
                self.add_edge(*exit, then_entry, EdgeKind::ConditionalTrue);
            }
        }

        let then_reachable = !condition_seq.exits.is_empty();
        let mut branch_exits = if then_reachable {
            then_seq.exits
        } else {
            SmallVec::new()
        };
        let mut branch_cause = if then_reachable && branch_exits.is_empty() {
            then_seq.terminal_cause
        } else {
            None
        };

        for elif_branch in self.program.elif_branches(ranges.elif_branches) {
            let elif_reachable = !false_exits.is_empty();
            let elif_start = self.blocks.len();
            let elif_cond = self.build_sequence(elif_branch.condition, loops);
            if false_exits.is_empty()
                && let Some(cause) = false_cause
            {
                self.mark_unreachable_blocks_since(elif_start, cause);
            }
            if let Some(elif_entry) = elif_cond.entry {
                for exit in &false_exits {
                    self.add_edge(*exit, elif_entry, EdgeKind::ConditionalFalse);
                }
            }

            let elif_body_start = self.blocks.len();
            let elif_body_seq = self.build_sequence(elif_branch.body, loops);
            let elif_body_reachable = elif_reachable && !elif_cond.exits.is_empty();
            let elif_cond_cause = elif_cond
                .exits
                .is_empty()
                .then_some(elif_cond.terminal_cause)
                .flatten();
            let elif_body_unreachable_cause = if elif_reachable {
                elif_cond_cause
            } else {
                false_cause
            };
            if !elif_body_reachable && let Some(cause) = elif_body_unreachable_cause {
                self.mark_unreachable_blocks_since(elif_body_start, cause);
            }
            if let Some(body_entry) = elif_body_seq.entry {
                for exit in &elif_cond.exits {
                    self.add_edge(*exit, body_entry, EdgeKind::ConditionalTrue);
                }
            }

            false_exits = if elif_reachable {
                elif_cond.exits
            } else {
                SmallVec::new()
            };
            false_cause = if false_exits.is_empty() {
                if elif_reachable {
                    elif_cond_cause
                } else {
                    false_cause
                }
            } else {
                None
            };
            let elif_body_cause = elif_body_seq
                .exits
                .is_empty()
                .then_some(elif_body_seq.terminal_cause)
                .flatten();
            if elif_body_reachable {
                append_unique_block_ids(&mut branch_exits, &elif_body_seq.exits);
                branch_cause = if branch_exits.is_empty() {
                    merge_terminal_causes(
                        self.program.command(command).span,
                        [branch_cause, elif_body_cause],
                    )
                } else {
                    None
                };
            }
        }

        let else_reachable = !false_exits.is_empty();
        let else_start = self.blocks.len();
        let else_seq = self.build_sequence(ranges.else_branch, loops);
        if false_exits.is_empty()
            && let Some(cause) = false_cause
        {
            self.mark_unreachable_blocks_since(else_start, cause);
        }
        if let Some(else_entry) = else_seq.entry {
            for exit in &false_exits {
                self.add_edge(*exit, else_entry, EdgeKind::ConditionalFalse);
            }
            if else_reachable {
                let else_cause = else_seq
                    .exits
                    .is_empty()
                    .then_some(else_seq.terminal_cause)
                    .flatten();
                append_unique_block_ids(&mut branch_exits, &else_seq.exits);
                branch_cause = if branch_exits.is_empty() {
                    merge_terminal_causes(
                        self.program.command(command).span,
                        [branch_cause, else_cause],
                    )
                } else {
                    None
                };
            } else if branch_exits.is_empty() {
                branch_cause = merge_terminal_causes(
                    self.program.command(command).span,
                    [branch_cause, false_cause],
                );
            } else {
                branch_cause = None;
            }
        } else {
            branch_exits.extend(false_exits);
            if branch_exits.is_empty() {
                branch_cause = merge_terminal_causes(
                    self.program.command(command).span,
                    [branch_cause, false_cause],
                );
            } else {
                branch_cause = None;
            }
        }

        self.wrap_sequence_with_command_header(
            command,
            SequenceResult {
                entry,
                exits: branch_exits,
                terminal_cause: branch_cause,
            },
            loops,
            force_command_header,
        )
    }

    fn build_while_like(
        &mut self,
        command: CommandId,
        condition: RecordedCommandRange,
        body: RecordedCommandRange,
        loops: &[LoopTarget],
        while_sense: bool,
        force_command_header: bool,
    ) -> SequenceResult {
        let exit_block = self.empty_block();
        let condition_seq = self.build_sequence(condition, loops);
        let entry = condition_seq.entry.or_else(|| Some(self.empty_block()));
        let continue_target = condition_seq.entry.unwrap_or(exit_block);
        let mut next_loops = SmallVec::<[LoopTarget; 2]>::from_slice(loops);
        next_loops.push(LoopTarget {
            continue_target,
            break_target: exit_block,
        });
        let body_seq = self.build_sequence(body, &next_loops);

        if let Some(body_entry) = body_seq.entry {
            for exit in &condition_seq.exits {
                self.add_edge(
                    *exit,
                    body_entry,
                    if while_sense {
                        EdgeKind::ConditionalTrue
                    } else {
                        EdgeKind::ConditionalFalse
                    },
                );
                self.add_edge(
                    *exit,
                    exit_block,
                    if while_sense {
                        EdgeKind::ConditionalFalse
                    } else {
                        EdgeKind::ConditionalTrue
                    },
                );
            }
            for exit in &body_seq.exits {
                self.add_edge(*exit, continue_target, EdgeKind::LoopBack);
            }
        } else {
            for exit in &condition_seq.exits {
                self.add_edge(*exit, exit_block, EdgeKind::LoopExit);
            }
        }

        self.wrap_sequence_with_command_header(
            command,
            SequenceResult {
                entry,
                exits: smallvec![exit_block],
                terminal_cause: None,
            },
            loops,
            force_command_header,
        )
    }

    fn build_loop_command(
        &mut self,
        command: CommandId,
        body: RecordedCommandRange,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let command = self.program.command(command);
        let header = self.command_block(command.span);
        self.attach_nested_regions(header, command.nested_regions, loops);
        let exit_block = self.empty_block();
        let mut next_loops = SmallVec::<[LoopTarget; 2]>::from_slice(loops);
        next_loops.push(LoopTarget {
            continue_target: header,
            break_target: exit_block,
        });
        let body_seq = self.build_sequence(body, &next_loops);
        if let Some(body_entry) = body_seq.entry {
            self.add_edge(header, body_entry, EdgeKind::ConditionalTrue);
            self.add_edge(header, exit_block, EdgeKind::ConditionalFalse);
            for exit in &body_seq.exits {
                self.add_edge(*exit, header, EdgeKind::LoopBack);
            }
        } else {
            self.add_edge(header, exit_block, EdgeKind::LoopExit);
        }
        SequenceResult {
            entry: Some(header),
            exits: smallvec![exit_block],
            terminal_cause: None,
        }
    }

    fn build_case(
        &mut self,
        command: CommandId,
        arms: RecordedCaseArmRange,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let command = self.program.command(command);
        let arms = self.program.case_arms(arms);
        let head = self.command_block(command.span);
        self.attach_nested_regions(head, command.nested_regions, loops);
        let exit_block = self.empty_block();
        let dispatch_count = arms.len().max(1);
        let mut dispatch_blocks = Vec::with_capacity(dispatch_count);
        dispatch_blocks.push(head);
        for _ in 1..dispatch_count {
            dispatch_blocks.push(self.empty_block());
        }
        let mut arm_sequences = Vec::with_capacity(arms.len());
        for arm in arms {
            let mut arm_seq = self.build_sequence(arm.commands, loops);
            if arm_seq.entry.is_none() {
                let empty_arm = self.empty_block();
                arm_seq.entry = Some(empty_arm);
                arm_seq.exits.push(empty_arm);
            }
            arm_sequences.push(arm_seq);
        }

        for (dispatch_index, dispatch) in dispatch_blocks.iter().copied().enumerate() {
            let mut matched_all = false;
            for (arm_index, arm) in arms.iter().enumerate().skip(dispatch_index) {
                let Some(arm_entry) = arm_sequences[arm_index].entry else {
                    unreachable!("empty case arms get a synthetic CFG block");
                };
                self.add_edge(dispatch, arm_entry, EdgeKind::CaseArm);
                if arm.matches_anything {
                    matched_all = true;
                    break;
                }
            }
            if !matched_all {
                self.add_edge(dispatch, exit_block, EdgeKind::Sequential);
            }
        }

        let mut fallthrough_from = SmallVec::<[BlockId; 2]>::new();

        for (arm_index, arm) in arms.iter().enumerate() {
            let arm_seq = &arm_sequences[arm_index];
            let Some(arm_entry) = arm_seq.entry else {
                unreachable!("empty case arms get a synthetic CFG block");
            };
            for block in &fallthrough_from {
                self.add_edge(*block, arm_entry, EdgeKind::CaseFallthrough);
            }

            match arm.terminator {
                CaseTerminator::Break => {
                    for exit in &arm_seq.exits {
                        self.add_edge(*exit, exit_block, EdgeKind::LoopExit);
                    }
                    fallthrough_from.clear();
                }
                CaseTerminator::FallThrough => {
                    fallthrough_from = arm_seq.exits.clone();
                }
                CaseTerminator::Continue => {
                    let continue_target = dispatch_blocks
                        .get(arm_index + 1)
                        .copied()
                        .unwrap_or(exit_block);
                    for block in &arm_seq.exits {
                        self.add_edge(*block, continue_target, EdgeKind::CaseContinue);
                    }
                    fallthrough_from.clear();
                }
                CaseTerminator::ContinueMatching => {
                    let continue_target = dispatch_blocks
                        .get(arm_index + 1)
                        .copied()
                        .unwrap_or(exit_block);
                    for block in &arm_seq.exits {
                        self.add_edge(*block, continue_target, EdgeKind::CaseContinue);
                    }
                    fallthrough_from.clear();
                }
            }
        }

        SequenceResult {
            entry: Some(head),
            exits: smallvec![exit_block],
            terminal_cause: None,
        }
    }

    fn attach_nested_regions(
        &mut self,
        block: BlockId,
        regions: RecordedRegionRange,
        loops: &[LoopTarget],
    ) {
        for region in self.program.nested_regions(regions) {
            let sequence = self.build_sequence(region.commands, loops);
            if let Some(entry) = sequence.entry {
                self.scope_entries.entry(region.scope).or_insert(entry);
                self.add_edge(block, entry, EdgeKind::NestedRegion);
            }
        }
    }

    fn command_block(&mut self, span: Span) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        let key = SpanKey::new(span);
        self.blocks.push(BasicBlock {
            id,
            commands: smallvec![span],
            bindings: self.command_bindings.get(&key).cloned().unwrap_or_default(),
            references: self
                .command_references
                .get(&key)
                .cloned()
                .unwrap_or_default(),
        });
        self.successors.push(SmallVec::new());
        self.command_blocks.entry(key).or_default().push(id);
        id
    }

    fn empty_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            commands: SmallVec::new(),
            bindings: SmallVec::new(),
            references: SmallVec::new(),
        });
        self.successors.push(SmallVec::new());
        id
    }

    fn add_edge(&mut self, from: BlockId, to: BlockId, kind: EdgeKind) {
        self.successors[from.index()].push((to, kind));
    }
}

fn resolve_break_target(loops: &[LoopTarget], depth: usize) -> Option<&LoopTarget> {
    loops.iter().rev().nth(depth.saturating_sub(1))
}

fn collect_flat_list_segments(
    program: &RecordedProgram,
    command: CommandId,
    operator_before: Option<(RecordedListOperator, Span)>,
    out: &mut Vec<FlatListSegment>,
) {
    if let RecordedCommandKind::List { first, rest } = program.command(command).kind {
        collect_flat_list_segments(program, first, operator_before, out);
        for item in program.list_items(rest) {
            collect_flat_list_segments(
                program,
                item.command,
                Some((item.operator, item.operator_span)),
                out,
            );
        }
        return;
    }

    out.push(FlatListSegment {
        operator_before,
        command,
    });
}

fn derive_predecessors(successors: &[SmallVec<[FlowEdge; 2]>]) -> Vec<SmallVec<[BlockId; 2]>> {
    let mut predecessors = vec![SmallVec::<[BlockId; 2]>::new(); successors.len()];
    for (block_index, edges) in successors.iter().enumerate() {
        let block = BlockId(block_index as u32);
        for (target, _) in edges {
            predecessors[target.index()].push(block);
        }
    }
    predecessors
}

fn compute_unreachable(
    blocks: &[BasicBlock],
    roots: &FxHashMap<ScopeId, BlockId>,
    successors: &[SmallVec<[FlowEdge; 2]>],
) -> Vec<BlockId> {
    let mut visited = FxHashSet::with_capacity_and_hasher(blocks.len(), Default::default());
    let mut stack: Vec<BlockId> = roots.values().copied().collect();
    while let Some(block) = stack.pop() {
        if !visited.insert(block) {
            continue;
        }
        for (target, _) in &successors[block.index()] {
            stack.push(*target);
        }
    }

    blocks
        .iter()
        .filter_map(|block| (!visited.contains(&block.id)).then_some(block.id))
        .collect()
}

fn append_unique_block_ids(target: &mut SmallVec<[BlockId; 2]>, source: &[BlockId]) {
    for &block in source {
        if !target.contains(&block) {
            target.push(block);
        }
    }
}
