use rustc_hash::FxHashMap;
use shuck_ast::{CaseTerminator, Span};
use shuck_parser::ZshEmulationMode;
use smallvec::SmallVec;
use std::marker::PhantomData;

use crate::source_closure::SourcePathTemplate;
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
    pub(crate) scope_exits: FxHashMap<ScopeId, Vec<BlockId>>,
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

    pub(crate) fn scope_exits(&self, scope: ScopeId) -> Option<&[BlockId]> {
        self.scope_exits.get(&scope).map(Vec::as_slice)
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

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordedProgram {
    file_commands: RecordedCommandRange,
    function_bodies: FxHashMap<ScopeId, RecordedCommandRange>,
    commands: Vec<RecordedCommand>,
    command_sequence_items: Vec<RecordedCommandId>,
    isolated_regions: Vec<IsolatedRegion>,
    case_arms: Vec<RecordedCaseArm>,
    pipeline_segments: Vec<RecordedPipelineSegment>,
    list_items: Vec<RecordedListItem>,
    elif_branches: Vec<RecordedElifBranch>,
    pub(crate) command_infos: FxHashMap<SpanKey, RecordedCommandInfo>,
    pub(crate) function_body_scopes: FxHashMap<BindingId, ScopeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RecordedCommandId(u32);

impl RecordedCommandId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordedRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> RecordedRange<T> {
    pub(crate) const fn empty() -> Self {
        Self {
            start: 0,
            len: 0,
            marker: PhantomData,
        }
    }

    pub(crate) fn new(start: usize, len: usize) -> Self {
        Self {
            start: match u32::try_from(start) {
                Ok(start) => start,
                Err(err) => panic!("recorded IR start fits in u32: {err}"),
            },
            len: match u32::try_from(len) {
                Ok(len) => len,
                Err(err) => panic!("recorded IR length fits in u32: {err}"),
            },
            marker: PhantomData,
        }
    }

    pub(crate) fn len(self) -> usize {
        self.len as usize
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn slice(self, store: &[T]) -> &[T] {
        let start = self.start as usize;
        &store[start..start + self.len()]
    }
}

impl<T> Default for RecordedRange<T> {
    fn default() -> Self {
        Self::empty()
    }
}

pub(crate) type RecordedCommandRange = RecordedRange<RecordedCommandId>;
pub(crate) type RecordedRegionRange = RecordedRange<IsolatedRegion>;
pub(crate) type RecordedCaseArmRange = RecordedRange<RecordedCaseArm>;
pub(crate) type RecordedPipelineSegmentRange = RecordedRange<RecordedPipelineSegment>;
pub(crate) type RecordedListItemRange = RecordedRange<RecordedListItem>;
pub(crate) type RecordedElifBranchRange = RecordedRange<RecordedElifBranch>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedCommand {
    pub(crate) span: Span,
    pub(crate) nested_regions: RecordedRegionRange,
    pub(crate) kind: RecordedCommandKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IsolatedRegion {
    pub(crate) scope: ScopeId,
    pub(crate) commands: RecordedCommandRange,
}

#[derive(Debug, Clone, Copy)]
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
        first: RecordedCommandId,
        rest: RecordedListItemRange,
    },
    If {
        condition: RecordedCommandRange,
        then_branch: RecordedCommandRange,
        elif_branches: RecordedElifBranchRange,
        else_branch: RecordedCommandRange,
    },
    While {
        condition: RecordedCommandRange,
        body: RecordedCommandRange,
    },
    Until {
        condition: RecordedCommandRange,
        body: RecordedCommandRange,
    },
    For {
        body: RecordedCommandRange,
    },
    Select {
        body: RecordedCommandRange,
    },
    ArithmeticFor {
        body: RecordedCommandRange,
    },
    Case {
        arms: RecordedCaseArmRange,
    },
    BraceGroup {
        body: RecordedCommandRange,
    },
    Subshell {
        body: RecordedCommandRange,
    },
    Pipeline {
        segments: RecordedPipelineSegmentRange,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedCaseArm {
    pub(crate) terminator: CaseTerminator,
    pub(crate) matches_anything: bool,
    pub(crate) commands: RecordedCommandRange,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedPipelineSegment {
    pub(crate) scope: ScopeId,
    pub(crate) command: RecordedCommandId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedListItem {
    pub(crate) operator: RecordedListOperator,
    pub(crate) command: RecordedCommandId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedElifBranch {
    pub(crate) condition: RecordedCommandRange,
    pub(crate) body: RecordedCommandRange,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordedCommandInfo {
    pub(crate) static_callee: Option<String>,
    pub(crate) static_args: Box<[Option<String>]>,
    pub(crate) source_path_template: Option<SourcePathTemplate>,
    pub(crate) zsh_effects: Vec<RecordedZshCommandEffect>,
}

#[derive(Debug, Clone)]
pub(crate) enum RecordedZshCommandEffect {
    Emulate {
        mode: ZshEmulationMode,
        local: bool,
    },
    SetOptions {
        updates: Vec<RecordedZshOptionUpdate>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedZshOptionUpdate {
    Named { name: Box<str>, enable: bool },
    LocalOptions { enable: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordedListOperator {
    And,
    Or,
}

impl RecordedProgram {
    pub(crate) fn file_commands(&self) -> RecordedCommandRange {
        self.file_commands
    }

    pub(crate) fn set_file_commands(&mut self, commands: RecordedCommandRange) {
        self.file_commands = commands;
    }

    pub(crate) fn function_body(&self, scope: ScopeId) -> RecordedCommandRange {
        self.function_bodies
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn set_function_body(&mut self, scope: ScopeId, commands: RecordedCommandRange) {
        self.function_bodies.insert(scope, commands);
    }

    pub(crate) fn function_bodies(&self) -> &FxHashMap<ScopeId, RecordedCommandRange> {
        &self.function_bodies
    }

    pub(crate) fn command(&self, id: RecordedCommandId) -> &RecordedCommand {
        &self.commands[id.index()]
    }

    pub(crate) fn command_mut(&mut self, id: RecordedCommandId) -> &mut RecordedCommand {
        &mut self.commands[id.index()]
    }

    pub(crate) fn commands(&self) -> &[RecordedCommand] {
        &self.commands
    }

    pub(crate) fn commands_in(&self, range: RecordedCommandRange) -> &[RecordedCommandId] {
        range.slice(&self.command_sequence_items)
    }

    pub(crate) fn nested_regions(&self, range: RecordedRegionRange) -> &[IsolatedRegion] {
        range.slice(&self.isolated_regions)
    }

    pub(crate) fn case_arms(&self, range: RecordedCaseArmRange) -> &[RecordedCaseArm] {
        range.slice(&self.case_arms)
    }

    pub(crate) fn pipeline_segments(
        &self,
        range: RecordedPipelineSegmentRange,
    ) -> &[RecordedPipelineSegment] {
        range.slice(&self.pipeline_segments)
    }

    pub(crate) fn list_items(&self, range: RecordedListItemRange) -> &[RecordedListItem] {
        range.slice(&self.list_items)
    }

    pub(crate) fn elif_branches(&self, range: RecordedElifBranchRange) -> &[RecordedElifBranch] {
        range.slice(&self.elif_branches)
    }

    pub(crate) fn push_command(&mut self, command: RecordedCommand) -> RecordedCommandId {
        let id = RecordedCommandId(match u32::try_from(self.commands.len()) {
            Ok(id) => id,
            Err(err) => panic!("recorded command count fits in u32: {err}"),
        });
        self.commands.push(command);
        id
    }

    pub(crate) fn push_command_ids(
        &mut self,
        command_ids: Vec<RecordedCommandId>,
    ) -> RecordedCommandRange {
        push_range(&mut self.command_sequence_items, command_ids)
    }

    pub(crate) fn push_regions(&mut self, regions: Vec<IsolatedRegion>) -> RecordedRegionRange {
        push_range(&mut self.isolated_regions, regions)
    }

    pub(crate) fn push_case_arms(&mut self, arms: Vec<RecordedCaseArm>) -> RecordedCaseArmRange {
        push_range(&mut self.case_arms, arms)
    }

    pub(crate) fn push_pipeline_segments(
        &mut self,
        segments: Vec<RecordedPipelineSegment>,
    ) -> RecordedPipelineSegmentRange {
        push_range(&mut self.pipeline_segments, segments)
    }

    pub(crate) fn push_list_items(
        &mut self,
        list_items: Vec<RecordedListItem>,
    ) -> RecordedListItemRange {
        push_range(&mut self.list_items, list_items)
    }

    pub(crate) fn push_elif_branches(
        &mut self,
        elif_branches: Vec<RecordedElifBranch>,
    ) -> RecordedElifBranchRange {
        push_range(&mut self.elif_branches, elif_branches)
    }
}

fn push_range<T>(store: &mut Vec<T>, mut items: Vec<T>) -> RecordedRange<T> {
    if items.is_empty() {
        return RecordedRange::empty();
    }

    let start = store.len();
    store.append(&mut items);
    RecordedRange::new(start, store.len() - start)
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
    program: &'a RecordedProgram,
    command_bindings: &'a FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: &'a FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    blocks: Vec<BasicBlock>,
    successors: FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
    command_blocks: FxHashMap<SpanKey, Vec<BlockId>>,
    unreachable_causes: FxHashMap<BlockId, Span>,
    scope_entries: FxHashMap<ScopeId, BlockId>,
}

pub(crate) fn build_control_flow_graph(
    program: &RecordedProgram,
    command_bindings: &FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: &FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
) -> ControlFlowGraph {
    let mut builder = GraphBuilder {
        program,
        command_bindings,
        command_references,
        blocks: Vec::new(),
        successors: FxHashMap::default(),
        command_blocks: FxHashMap::default(),
        unreachable_causes: FxHashMap::default(),
        scope_entries: FxHashMap::default(),
    };

    let file = builder.build_sequence(program.file_commands(), &[]);
    let entry = file.entry.unwrap_or_else(|| builder.empty_block());
    builder.scope_entries.insert(ScopeId(0), entry);
    let file_exits = if file.exits.is_empty() {
        vec![entry]
    } else {
        file.exits.clone()
    };
    let mut scope_exits = FxHashMap::default();
    scope_exits.insert(ScopeId(0), file_exits.clone());

    let mut exits = file_exits;

    for (scope, commands) in program.function_bodies() {
        let function = builder.build_sequence(*commands, &[]);
        let function_entry = function.entry.unwrap_or_else(|| builder.empty_block());
        builder.scope_entries.insert(*scope, function_entry);
        let function_exits = if function.exits.is_empty() {
            vec![function_entry]
        } else {
            function.exits
        };
        scope_exits.insert(*scope, function_exits.clone());
        exits.extend(function_exits);
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
        let mut pending = Vec::new();
        let mut unreachable_cause = None;

        for &command_id in self.program.commands_in(commands) {
            let command = self.program.command(command_id);
            let start = self.blocks.len();
            let sequence = self.build_command(command_id, loops);
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

    fn build_command(
        &mut self,
        command_id: RecordedCommandId,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let command = self.program.command(command_id);
        match &command.kind {
            RecordedCommandKind::Linear => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, command.nested_regions, loops);
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
                self.build_list(command_id, *first, *rest, loops)
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => self.build_if(
                command_id,
                *condition,
                *then_branch,
                *elif_branches,
                *else_branch,
                loops,
            ),
            RecordedCommandKind::While { condition, body } => {
                self.build_while_like(command_id, *condition, *body, loops, true)
            }
            RecordedCommandKind::Until { condition, body } => {
                self.build_while_like(command_id, *condition, *body, loops, false)
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body } => {
                self.build_loop_command(command_id, *body, loops)
            }
            RecordedCommandKind::Case { arms } => self.build_case(command_id, *arms, loops),
            RecordedCommandKind::BraceGroup { body } => {
                let sequence = self.build_sequence(*body, loops);
                self.wrap_sequence_with_command_header(command_id, sequence, loops)
            }
            RecordedCommandKind::Subshell { body, .. } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, command.nested_regions, loops);
                let body_sequence = self.build_sequence(*body, loops);
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
                self.attach_nested_regions(block, command.nested_regions, loops);
                for segment in self.program.pipeline_segments(*segments) {
                    let sequence = self.build_command(segment.command, loops);
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

    fn wrap_sequence_with_command_header(
        &mut self,
        command_id: RecordedCommandId,
        mut sequence: SequenceResult,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let command = self.program.command(command_id);
        if command.nested_regions.is_empty() {
            return sequence;
        }

        let header = self.command_block(command.span);
        self.attach_nested_regions(header, command.nested_regions, loops);
        if let Some(entry) = sequence.entry {
            self.add_edge(header, entry, EdgeKind::Sequential);
        } else {
            sequence.exits = vec![header];
        }
        sequence.entry = Some(header);
        sequence
    }

    fn build_list(
        &mut self,
        command: RecordedCommandId,
        first: RecordedCommandId,
        rest: RecordedListItemRange,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let mut current = self.build_command(first, loops);
        let entry = current.entry;
        let mut shortcut_exits = Vec::new();

        for item in self.program.list_items(rest) {
            let next = self.build_command(item.command, loops);
            if let Some(next_entry) = next.entry {
                for exit in &current.exits {
                    let edge = match item.operator {
                        RecordedListOperator::And => EdgeKind::ConditionalTrue,
                        RecordedListOperator::Or => EdgeKind::ConditionalFalse,
                    };
                    self.add_edge(*exit, next_entry, edge);
                }
            }

            shortcut_exits.extend(current.exits.clone());
            current = SequenceResult {
                entry,
                exits: next.exits,
            };
        }

        let mut exits = current.exits;
        exits.extend(shortcut_exits);
        self.wrap_sequence_with_command_header(command, SequenceResult { entry, exits }, loops)
    }

    fn build_if(
        &mut self,
        command: RecordedCommandId,
        condition: RecordedCommandRange,
        then_branch: RecordedCommandRange,
        elif_branches: RecordedElifBranchRange,
        else_branch: RecordedCommandRange,
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

        for elif_branch in self.program.elif_branches(elif_branches) {
            let elif_cond = self.build_sequence(elif_branch.condition, loops);
            if let Some(elif_entry) = elif_cond.entry {
                for exit in &false_exits {
                    self.add_edge(*exit, elif_entry, EdgeKind::ConditionalFalse);
                }
            }

            let elif_body_seq = self.build_sequence(elif_branch.body, loops);
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

        self.wrap_sequence_with_command_header(
            command,
            SequenceResult {
                entry,
                exits: branch_exits,
            },
            loops,
        )
    }

    fn build_while_like(
        &mut self,
        command: RecordedCommandId,
        condition: RecordedCommandRange,
        body: RecordedCommandRange,
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

        self.wrap_sequence_with_command_header(
            command,
            SequenceResult {
                entry,
                exits: vec![exit_block],
            },
            loops,
        )
    }

    fn build_loop_command(
        &mut self,
        command: RecordedCommandId,
        body: RecordedCommandRange,
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let command = self.program.command(command);
        let header = self.command_block(command.span);
        self.attach_nested_regions(header, command.nested_regions, loops);
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
        command: RecordedCommandId,
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

        let mut fallthrough_from = Vec::new();

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
            exits: vec![exit_block],
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
                self.add_edge(block, entry, EdgeKind::Sequential);
            }
        }
    }

    fn command_block(&mut self, span: Span) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        let key = SpanKey::new(span);
        self.blocks.push(BasicBlock {
            id,
            commands: vec![span],
            bindings: self
                .command_bindings
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into_vec(),
            references: self
                .command_references
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into_vec(),
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
