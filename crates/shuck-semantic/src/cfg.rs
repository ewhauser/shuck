use rustc_hash::FxHashMap;
use shuck_ast::{CaseTerminator, ListOperator, Span};

use crate::{BindingId, ReferenceId, ScopeId, SpanKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub(crate) u32);

impl BlockId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub commands: Vec<Span>,
    pub bindings: Vec<BindingId>,
    pub references: Vec<ReferenceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Sequential,
    ConditionalTrue,
    ConditionalFalse,
    LoopBack,
    LoopExit,
    CaseArm,
    CaseFallthrough,
    CaseContinue,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlowContext {
    pub in_function: bool,
    pub loop_depth: u32,
    pub in_subshell: bool,
    pub in_block: bool,
    pub exit_status_checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlowGraph {
    blocks: Vec<BasicBlock>,
    successors: FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
    predecessors: FxHashMap<BlockId, Vec<BlockId>>,
    entry: BlockId,
    exits: Vec<BlockId>,
    unreachable: Vec<BlockId>,
    pub(crate) scope_entries: FxHashMap<ScopeId, BlockId>,
    pub(crate) command_blocks: FxHashMap<SpanKey, Vec<BlockId>>,
    pub(crate) unreachable_causes: FxHashMap<BlockId, Span>,
}

impl ControlFlowGraph {
    pub fn blocks(&self) -> &[BasicBlock] {
        &self.blocks
    }

    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.index()]
    }

    pub fn successors(&self, id: BlockId) -> &[(BlockId, EdgeKind)] {
        self.successors.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn predecessors(&self, id: BlockId) -> &[BlockId] {
        self.predecessors.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn entry(&self) -> BlockId {
        self.entry
    }

    pub fn exits(&self) -> &[BlockId] {
        &self.exits
    }

    pub fn unreachable(&self) -> &[BlockId] {
        &self.unreachable
    }

    pub(crate) fn block_ids_for_span(&self, span: Span) -> &[BlockId] {
        self.command_blocks
            .get(&SpanKey::new(span))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn unreachable_cause(&self, id: BlockId) -> Option<Span> {
        self.unreachable_causes.get(&id).copied()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedProgram {
    pub(crate) file_commands: Vec<RecordedCommand>,
    pub(crate) function_bodies: FxHashMap<ScopeId, Vec<RecordedCommand>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedCommand {
    pub(crate) span: Span,
    pub(crate) nested_regions: Vec<IsolatedRegion>,
    pub(crate) kind: RecordedCommandKind,
}

#[derive(Debug, Clone)]
pub(crate) struct IsolatedRegion {
    pub(crate) scope: ScopeId,
    pub(crate) commands: Vec<RecordedCommand>,
}

#[derive(Debug, Clone)]
pub(crate) enum RecordedCommandKind {
    Linear,
    Break {
        depth: usize,
    },
    Continue {
        depth: usize,
    },
    Return,
    Exit,
    List {
        first: Box<RecordedCommand>,
        rest: Vec<(ListOperator, RecordedCommand)>,
    },
    If {
        condition: Vec<RecordedCommand>,
        then_branch: Vec<RecordedCommand>,
        elif_branches: Vec<(Vec<RecordedCommand>, Vec<RecordedCommand>)>,
        else_branch: Vec<RecordedCommand>,
    },
    While {
        condition: Vec<RecordedCommand>,
        body: Vec<RecordedCommand>,
    },
    Until {
        condition: Vec<RecordedCommand>,
        body: Vec<RecordedCommand>,
    },
    For {
        body: Vec<RecordedCommand>,
    },
    Select {
        body: Vec<RecordedCommand>,
    },
    ArithmeticFor {
        body: Vec<RecordedCommand>,
    },
    Case {
        arms: Vec<RecordedCaseArm>,
    },
    BraceGroup {
        body: Vec<RecordedCommand>,
    },
    Subshell {
        body: Vec<RecordedCommand>,
    },
    Pipeline {
        segments: Vec<RecordedPipelineSegment>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedCaseArm {
    pub(crate) terminator: CaseTerminator,
    pub(crate) commands: Vec<RecordedCommand>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedPipelineSegment {
    pub(crate) scope: ScopeId,
    pub(crate) command: RecordedCommand,
}

struct SequenceResult {
    entry: Option<BlockId>,
    exits: Vec<BlockId>,
}

#[derive(Clone, Copy)]
struct LoopTarget {
    continue_target: BlockId,
    break_target: BlockId,
}

struct GraphBuilder<'a> {
    command_bindings: &'a FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: &'a FxHashMap<SpanKey, Vec<ReferenceId>>,
    blocks: Vec<BasicBlock>,
    successors: FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
    command_blocks: FxHashMap<SpanKey, Vec<BlockId>>,
    unreachable_causes: FxHashMap<BlockId, Span>,
    scope_entries: FxHashMap<ScopeId, BlockId>,
}

pub(crate) fn build_control_flow_graph(
    program: &RecordedProgram,
    command_bindings: &FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: &FxHashMap<SpanKey, Vec<ReferenceId>>,
) -> ControlFlowGraph {
    let mut builder = GraphBuilder {
        command_bindings,
        command_references,
        blocks: Vec::new(),
        successors: FxHashMap::default(),
        command_blocks: FxHashMap::default(),
        unreachable_causes: FxHashMap::default(),
        scope_entries: FxHashMap::default(),
    };

    let file = builder.build_sequence(&program.file_commands, &[]);
    let entry = file.entry.unwrap_or_else(|| builder.empty_block());
    builder.scope_entries.insert(ScopeId(0), entry);

    let mut exits = if file.exits.is_empty() {
        vec![entry]
    } else {
        file.exits
    };

    for (scope, commands) in &program.function_bodies {
        let function = builder.build_sequence(commands, &[]);
        let function_entry = function.entry.unwrap_or_else(|| builder.empty_block());
        builder.scope_entries.insert(*scope, function_entry);
        if function.exits.is_empty() {
            exits.push(function_entry);
        } else {
            exits.extend(function.exits);
        }
    }

    let predecessors = derive_predecessors(&builder.successors);
    let unreachable =
        compute_unreachable(&builder.blocks, &builder.scope_entries, &builder.successors);

    ControlFlowGraph {
        blocks: builder.blocks,
        successors: builder.successors,
        predecessors,
        entry,
        exits,
        unreachable,
        scope_entries: builder.scope_entries,
        command_blocks: builder.command_blocks,
        unreachable_causes: builder.unreachable_causes,
    }
}

impl<'a> GraphBuilder<'a> {
    fn build_sequence(
        &mut self,
        commands: &[RecordedCommand],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let mut entry = None;
        let mut pending = Vec::new();
        let mut unreachable_cause = None;

        for command in commands {
            let start = self.blocks.len();
            let sequence = self.build_command(command, loops);
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
                unreachable_cause = Some(command.span);
            } else {
                pending = sequence.exits;
                unreachable_cause = None;
            }
        }

        SequenceResult {
            entry,
            exits: pending,
        }
    }

    fn build_command(&mut self, command: &RecordedCommand, loops: &[LoopTarget]) -> SequenceResult {
        match &command.kind {
            RecordedCommandKind::Linear => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, &command.nested_regions, loops);
                SequenceResult {
                    entry: Some(block),
                    exits: vec![block],
                }
            }
            RecordedCommandKind::Break { depth } => {
                let block = self.command_block(command.span);
                if let Some(target) = resolve_break_target(loops, *depth) {
                    self.add_edge(block, target.break_target, EdgeKind::LoopExit);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: Vec::new(),
                }
            }
            RecordedCommandKind::Continue { depth } => {
                let block = self.command_block(command.span);
                if let Some(target) = resolve_break_target(loops, *depth) {
                    self.add_edge(block, target.continue_target, EdgeKind::LoopBack);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: Vec::new(),
                }
            }
            RecordedCommandKind::Return | RecordedCommandKind::Exit => {
                let block = self.command_block(command.span);
                SequenceResult {
                    entry: Some(block),
                    exits: Vec::new(),
                }
            }
            RecordedCommandKind::List { first, rest } => {
                self.build_list(command, first, rest, loops)
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => self.build_if(
                command,
                condition,
                then_branch,
                elif_branches,
                else_branch,
                loops,
            ),
            RecordedCommandKind::While { condition, body } => {
                self.build_while_like(command, condition, body, loops, true)
            }
            RecordedCommandKind::Until { condition, body } => {
                self.build_while_like(command, condition, body, loops, false)
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body } => {
                self.build_loop_command(command, body, loops)
            }
            RecordedCommandKind::Case { arms } => self.build_case(command, arms, loops),
            RecordedCommandKind::BraceGroup { body } => self.build_sequence(body, loops),
            RecordedCommandKind::Subshell { body, .. } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, &command.nested_regions, loops);
                let body_sequence = self.build_sequence(body, loops);
                if let Some(body_entry) = body_sequence.entry {
                    self.add_edge(block, body_entry, EdgeKind::Sequential);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: vec![block],
                }
            }
            RecordedCommandKind::Pipeline { segments } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, &command.nested_regions, loops);
                for segment in segments {
                    let sequence = self.build_command(&segment.command, loops);
                    if let Some(segment_entry) = sequence.entry {
                        self.scope_entries
                            .entry(segment.scope)
                            .or_insert(segment_entry);
                        self.add_edge(block, segment_entry, EdgeKind::Sequential);
                    }
                }
                SequenceResult {
                    entry: Some(block),
                    exits: vec![block],
                }
            }
        }
    }

    fn build_list(
        &mut self,
        command: &RecordedCommand,
        first: &RecordedCommand,
        rest: &[(ListOperator, RecordedCommand)],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let mut current = self.build_command(first, loops);
        let entry = current.entry;
        let mut shortcut_exits = Vec::new();

        for (op, command) in rest {
            let next = self.build_command(command, loops);
            if let Some(next_entry) = next.entry {
                for exit in &current.exits {
                    let edge = match op {
                        ListOperator::And => EdgeKind::ConditionalTrue,
                        ListOperator::Or => EdgeKind::ConditionalFalse,
                        ListOperator::Semicolon | ListOperator::Background => EdgeKind::Sequential,
                    };
                    self.add_edge(*exit, next_entry, edge);
                }
            }

            if matches!(op, ListOperator::And | ListOperator::Or) {
                shortcut_exits.extend(current.exits.clone());
            }

            current = if matches!(op, ListOperator::Semicolon | ListOperator::Background) {
                next
            } else {
                SequenceResult {
                    entry,
                    exits: next.exits,
                }
            };
        }

        let mut exits = current.exits;
        exits.extend(shortcut_exits);
        self.attach_nested_regions_from_command(command);
        SequenceResult { entry, exits }
    }

    fn build_if(
        &mut self,
        command: &RecordedCommand,
        condition: &[RecordedCommand],
        then_branch: &[RecordedCommand],
        elif_branches: &[(Vec<RecordedCommand>, Vec<RecordedCommand>)],
        else_branch: &[RecordedCommand],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let condition_seq = self.build_sequence(condition, loops);
        let entry = condition_seq.entry.or_else(|| Some(self.empty_block()));
        let mut false_exits = condition_seq.exits.clone();

        let then_seq = self.build_sequence(then_branch, loops);
        if let (Some(cond_entry), Some(then_entry)) = (entry, then_seq.entry) {
            for exit in &condition_seq.exits {
                self.add_edge(*exit, then_entry, EdgeKind::ConditionalTrue);
            }
            if condition_seq.exits.is_empty() {
                self.add_edge(cond_entry, then_entry, EdgeKind::ConditionalTrue);
            }
        }

        let mut branch_exits = then_seq.exits;

        for (elif_condition, elif_body) in elif_branches {
            let elif_cond = self.build_sequence(elif_condition, loops);
            if let Some(elif_entry) = elif_cond.entry {
                for exit in &false_exits {
                    self.add_edge(*exit, elif_entry, EdgeKind::ConditionalFalse);
                }
            }

            let elif_body_seq = self.build_sequence(elif_body, loops);
            if let Some(body_entry) = elif_body_seq.entry {
                for exit in &elif_cond.exits {
                    self.add_edge(*exit, body_entry, EdgeKind::ConditionalTrue);
                }
            }

            false_exits = elif_cond.exits;
            branch_exits.extend(elif_body_seq.exits);
        }

        let else_seq = self.build_sequence(else_branch, loops);
        if let Some(else_entry) = else_seq.entry {
            for exit in &false_exits {
                self.add_edge(*exit, else_entry, EdgeKind::ConditionalFalse);
            }
            branch_exits.extend(else_seq.exits);
        } else {
            branch_exits.extend(false_exits);
        }

        self.attach_nested_regions_from_command(command);
        SequenceResult {
            entry,
            exits: branch_exits,
        }
    }

    fn build_while_like(
        &mut self,
        command: &RecordedCommand,
        condition: &[RecordedCommand],
        body: &[RecordedCommand],
        loops: &[LoopTarget],
        while_sense: bool,
    ) -> SequenceResult {
        let exit_block = self.empty_block();
        let condition_seq = self.build_sequence(condition, loops);
        let entry = condition_seq.entry.or_else(|| Some(self.empty_block()));
        let continue_target = condition_seq.entry.unwrap_or(exit_block);
        let mut next_loops = loops.to_vec();
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

        self.attach_nested_regions_from_command(command);
        SequenceResult {
            entry,
            exits: vec![exit_block],
        }
    }

    fn build_loop_command(
        &mut self,
        command: &RecordedCommand,
        body: &[RecordedCommand],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let header = self.command_block(command.span);
        self.attach_nested_regions(header, &command.nested_regions, loops);
        let exit_block = self.empty_block();
        let mut next_loops = loops.to_vec();
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
            exits: vec![exit_block],
        }
    }

    fn build_case(
        &mut self,
        command: &RecordedCommand,
        arms: &[RecordedCaseArm],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let head = self.command_block(command.span);
        self.attach_nested_regions(head, &command.nested_regions, loops);
        let exit_block = self.empty_block();
        let mut fallthrough_from = Vec::new();

        for arm in arms {
            let arm_seq = self.build_sequence(&arm.commands, loops);
            if let Some(arm_entry) = arm_seq.entry {
                self.add_edge(head, arm_entry, EdgeKind::CaseArm);
                for block in &fallthrough_from {
                    self.add_edge(*block, arm_entry, EdgeKind::CaseFallthrough);
                }
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
                    fallthrough_from = arm_seq.exits.clone();
                    for block in &fallthrough_from {
                        self.successors
                            .entry(*block)
                            .or_default()
                            .push((head, EdgeKind::CaseContinue));
                    }
                }
            }
        }

        if arms.is_empty() {
            self.add_edge(head, exit_block, EdgeKind::Sequential);
        }

        SequenceResult {
            entry: Some(head),
            exits: vec![exit_block],
        }
    }

    fn attach_nested_regions(
        &mut self,
        block: BlockId,
        regions: &[IsolatedRegion],
        loops: &[LoopTarget],
    ) {
        for region in regions {
            let sequence = self.build_sequence(&region.commands, loops);
            if let Some(entry) = sequence.entry {
                self.scope_entries.entry(region.scope).or_insert(entry);
                self.add_edge(block, entry, EdgeKind::Sequential);
            }
        }
    }

    fn attach_nested_regions_from_command(&mut self, command: &RecordedCommand) {
        if command.nested_regions.is_empty() {
            return;
        }
        if let Some(blocks) = self
            .command_blocks
            .get(&SpanKey::new(command.span))
            .cloned()
        {
            for block in blocks {
                self.attach_nested_regions(block, &command.nested_regions, &[]);
            }
        }
    }

    fn command_block(&mut self, span: Span) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        let key = SpanKey::new(span);
        self.blocks.push(BasicBlock {
            id,
            commands: vec![span],
            bindings: self.command_bindings.get(&key).cloned().unwrap_or_default(),
            references: self
                .command_references
                .get(&key)
                .cloned()
                .unwrap_or_default(),
        });
        self.command_blocks.entry(key).or_default().push(id);
        id
    }

    fn empty_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            commands: Vec::new(),
            bindings: Vec::new(),
            references: Vec::new(),
        });
        id
    }

    fn add_edge(&mut self, from: BlockId, to: BlockId, kind: EdgeKind) {
        self.successors.entry(from).or_default().push((to, kind));
    }
}

fn resolve_break_target(loops: &[LoopTarget], depth: usize) -> Option<&LoopTarget> {
    loops.iter().rev().nth(depth.saturating_sub(1))
}

fn derive_predecessors(
    successors: &FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
) -> FxHashMap<BlockId, Vec<BlockId>> {
    let mut predecessors: FxHashMap<BlockId, Vec<BlockId>> = FxHashMap::default();
    for (block, edges) in successors {
        for (target, _) in edges {
            predecessors.entry(*target).or_default().push(*block);
        }
    }
    predecessors
}

fn compute_unreachable(
    blocks: &[BasicBlock],
    roots: &FxHashMap<ScopeId, BlockId>,
    successors: &FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
) -> Vec<BlockId> {
    let mut visited = FxHashMap::default();
    let mut stack: Vec<BlockId> = roots.values().copied().collect();
    while let Some(block) = stack.pop() {
        if visited.insert(block, ()).is_some() {
            continue;
        }
        if let Some(edges) = successors.get(&block) {
            for (target, _) in edges {
                stack.push(*target);
            }
        }
    }

    blocks
        .iter()
        .filter_map(|block| (!visited.contains_key(&block.id)).then_some(block.id))
        .collect()
}
