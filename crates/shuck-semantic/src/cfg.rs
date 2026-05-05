use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{BuiltinCommand, CaseTerminator, Command, CompoundCommand, Name, Span};
use shuck_parser::ZshEmulationMode;
use smallvec::{SmallVec, smallvec};
use std::marker::PhantomData;

use crate::function_resolution::resolved_function_calls_with_callee_scope;
use crate::source_closure::SourcePathTemplate;
use crate::{Binding, BindingId, CallSite, ReferenceId, Scope, ScopeId, SpanKey};

/// Stable identifier for a CFG basic block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub(crate) u32);

impl BlockId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// One basic block in the control-flow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    /// Unique identifier for this block.
    pub id: BlockId,
    /// Command spans represented by the block.
    pub commands: SmallVec<[Span; 1]>,
    /// Bindings introduced in the block.
    pub bindings: SmallVec<[BindingId; 2]>,
    /// References recorded in the block.
    pub references: SmallVec<[ReferenceId; 4]>,
}

/// Edge label describing why control can flow between two blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Straight-line control flow.
    Sequential,
    /// True branch of a conditional.
    ConditionalTrue,
    /// False branch of a conditional.
    ConditionalFalse,
    /// Loop back-edge.
    LoopBack,
    /// Loop exit edge.
    LoopExit,
    /// Edge into a case arm.
    CaseArm,
    /// Fallthrough edge between case arms.
    CaseFallthrough,
    /// Continue edge between case arms.
    CaseContinue,
    /// Entry into a nested isolated region.
    NestedRegion,
}

/// Execution-context facts active at a command or block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlowContext {
    /// Whether execution is inside a function body.
    pub in_function: bool,
    /// Current loop nesting depth.
    pub loop_depth: u32,
    /// Whether execution is inside a subshell-like environment.
    pub in_subshell: bool,
    /// Whether execution is inside a brace or block context.
    pub in_block: bool,
    /// Whether the command's exit status is checked by surrounding syntax.
    pub exit_status_checked: bool,
}

/// Role a command plays when it appears in a condition list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandConditionRole {
    /// An `if` condition command.
    If,
    /// An `elif` condition command.
    Elif,
    /// A `while` condition command.
    While,
    /// An `until` condition command.
    Until,
}

/// Control-flow graph built from the semantic command stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlowGraph {
    blocks: Vec<BasicBlock>,
    successors: Vec<SmallVec<[FlowEdge; 2]>>,
    predecessors: Vec<SmallVec<[BlockId; 2]>>,
    entry: BlockId,
    exits: Vec<BlockId>,
    natural_exits: Vec<BlockId>,
    script_terminators: Vec<BlockId>,
    script_always_terminates: bool,
    unreachable: Vec<BlockId>,
    pub(crate) scope_entries: FxHashMap<ScopeId, BlockId>,
    pub(crate) scope_exits: FxHashMap<ScopeId, SmallVec<[BlockId; 2]>>,
    pub(crate) command_blocks: FxHashMap<SpanKey, SmallVec<[BlockId; 1]>>,
    pub(crate) unreachable_causes: FxHashMap<BlockId, UnreachableCause>,
}

type FlowEdge = (BlockId, EdgeKind);

impl ControlFlowGraph {
    /// Returns all basic blocks in allocation order.
    pub fn blocks(&self) -> &[BasicBlock] {
        &self.blocks
    }

    /// Returns the block with identifier `id`.
    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.index()]
    }

    /// Returns the outgoing edges from block `id`.
    pub fn successors(&self, id: BlockId) -> &[(BlockId, EdgeKind)] {
        self.successors
            .get(id.index())
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the incoming predecessors of block `id`.
    pub fn predecessors(&self, id: BlockId) -> &[BlockId] {
        self.predecessors
            .get(id.index())
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the entry block for the whole script.
    pub fn entry(&self) -> BlockId {
        self.entry
    }

    /// Returns all exit blocks, including exits caused by explicit termination.
    pub fn exits(&self) -> &[BlockId] {
        &self.exits
    }

    /// Returns exits reachable without an explicit script terminator.
    pub fn natural_exits(&self) -> &[BlockId] {
        &self.natural_exits
    }

    /// Returns blocks that terminate script execution, such as `exit` or top-level `return`.
    pub fn script_terminators(&self) -> &[BlockId] {
        &self.script_terminators
    }

    /// Returns whether every reachable path through the script terminates.
    pub fn script_always_terminates(&self) -> bool {
        self.script_always_terminates
    }

    /// Returns the entry block for `scope`, when the CFG records one.
    pub fn scope_entry(&self, scope: ScopeId) -> Option<BlockId> {
        self.scope_entries.get(&scope).copied()
    }

    pub(crate) fn scope_exits(&self, scope: ScopeId) -> Option<&[BlockId]> {
        self.scope_exits.get(&scope).map(SmallVec::as_slice)
    }

    /// Returns blocks proven unreachable.
    pub fn unreachable(&self) -> &[BlockId] {
        &self.unreachable
    }

    pub(crate) fn block_ids_for_span(&self, span: Span) -> &[BlockId] {
        self.command_blocks
            .get(&SpanKey::new(span))
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn unreachable_cause(&self, id: BlockId) -> Option<UnreachableCause> {
        self.unreachable_causes.get(&id).copied()
    }
}

/// Broad category for an unreachable-code cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnreachableCauseKind {
    /// A shell terminator such as `exit`, `return`, or fatal control transfer.
    ShellTerminator,
    /// Loop control such as `break` or `continue`.
    LoopControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnreachableCause {
    pub(crate) span: Span,
    pub(crate) kind: UnreachableCauseKind,
}

impl UnreachableCause {
    fn shell_terminator(span: Span) -> Self {
        Self {
            span,
            kind: UnreachableCauseKind::ShellTerminator,
        }
    }

    fn loop_control(span: Span) -> Self {
        Self {
            span,
            kind: UnreachableCauseKind::LoopControl,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordedProgram {
    file_commands: RecordedCommandRange,
    function_bodies: FxHashMap<ScopeId, RecordedCommandRange>,
    commands: Vec<RecordedCommand>,
    command_sequence_items: Vec<CommandId>,
    statement_sequence_commands: Vec<StatementSequenceCommand>,
    isolated_regions: Vec<IsolatedRegion>,
    case_arms: Vec<RecordedCaseArm>,
    pipeline_segments: Vec<RecordedPipelineSegment>,
    list_items: Vec<RecordedListItem>,
    elif_branches: Vec<RecordedElifBranch>,
    pub(crate) command_infos: FxHashMap<SpanKey, RecordedCommandInfo>,
    pub(crate) function_body_scopes: FxHashMap<BindingId, ScopeId>,
    pub(crate) call_command_spans: FxHashMap<SpanKey, Span>,
}

/// A statement-sequence item flattened out of structured syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementSequenceCommand {
    body_span: Span,
    stmt_span: Span,
}

impl StatementSequenceCommand {
    /// Returns the span of the command body without trailing statement punctuation.
    pub fn body_span(&self) -> Span {
        self.body_span
    }

    /// Returns the span of the full statement item.
    pub fn stmt_span(&self) -> Span {
        self.stmt_span
    }
}

/// Stable identifier for a semantic command in the recorded command stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub(crate) u32);

impl CommandId {
    /// Returns the zero-based index used by internal command-storage vectors.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// High-level command category recorded by semantic traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    /// A simple command.
    Simple,
    /// A builtin command with one of the tracked control-flow forms.
    Builtin(BuiltinCommandKind),
    /// A declaration builtin command.
    Decl,
    /// A binary command such as `[[ ... ]]` or `(( ... ))`.
    Binary,
    /// A compound command.
    Compound(CompoundCommandKind),
    /// A named function definition.
    Function,
    /// An anonymous function-like definition.
    AnonymousFunction,
}

impl CommandKind {
    /// Classifies a parsed AST command into a semantic command kind.
    pub fn from_command(command: &Command) -> Self {
        match command {
            Command::Simple(_) => Self::Simple,
            Command::Builtin(command) => Self::Builtin(BuiltinCommandKind::from_builtin(command)),
            Command::Decl(_) => Self::Decl,
            Command::Binary(_) => Self::Binary,
            Command::Compound(command) => {
                Self::Compound(CompoundCommandKind::from_compound(command))
            }
            Command::Function(_) => Self::Function,
            Command::AnonymousFunction(_) => Self::AnonymousFunction,
        }
    }
}

/// Builtin command kinds that affect control flow directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinCommandKind {
    /// `break`
    Break,
    /// `continue`
    Continue,
    /// `return`
    Return,
    /// `exit`
    Exit,
}

impl BuiltinCommandKind {
    fn from_builtin(command: &BuiltinCommand) -> Self {
        match command {
            BuiltinCommand::Break(_) => Self::Break,
            BuiltinCommand::Continue(_) => Self::Continue,
            BuiltinCommand::Return(_) => Self::Return,
            BuiltinCommand::Exit(_) => Self::Exit,
        }
    }
}

/// Compound command kinds tracked by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompoundCommandKind {
    /// `if`
    If,
    /// `for`
    For,
    /// `repeat`
    Repeat,
    /// `foreach`
    Foreach,
    /// Arithmetic `for (( ...; ...; ... ))`
    ArithmeticFor,
    /// `while`
    While,
    /// `until`
    Until,
    /// `case`
    Case,
    /// `select`
    Select,
    /// Subshell `( ... )`
    Subshell,
    /// Brace group `{ ...; }`
    BraceGroup,
    /// Arithmetic command.
    Arithmetic,
    /// `time`
    Time,
    /// Conditional command such as `[[ ... ]]`.
    Conditional,
    /// `coproc`
    Coproc,
    /// `always`
    Always,
}

impl CompoundCommandKind {
    fn from_compound(command: &CompoundCommand) -> Self {
        match command {
            CompoundCommand::If(_) => Self::If,
            CompoundCommand::For(_) => Self::For,
            CompoundCommand::Repeat(_) => Self::Repeat,
            CompoundCommand::Foreach(_) => Self::Foreach,
            CompoundCommand::ArithmeticFor(_) => Self::ArithmeticFor,
            CompoundCommand::While(_) => Self::While,
            CompoundCommand::Until(_) => Self::Until,
            CompoundCommand::Case(_) => Self::Case,
            CompoundCommand::Select(_) => Self::Select,
            CompoundCommand::Subshell(_) => Self::Subshell,
            CompoundCommand::BraceGroup(_) => Self::BraceGroup,
            CompoundCommand::Arithmetic(_) => Self::Arithmetic,
            CompoundCommand::Time(_) => Self::Time,
            CompoundCommand::Conditional(_) => Self::Conditional,
            CompoundCommand::Coproc(_) => Self::Coproc,
            CompoundCommand::Always(_) => Self::Always,
        }
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

pub(crate) type RecordedCommandRange = RecordedRange<CommandId>;
pub(crate) type RecordedRegionRange = RecordedRange<IsolatedRegion>;
pub(crate) type RecordedCaseArmRange = RecordedRange<RecordedCaseArm>;
pub(crate) type RecordedPipelineSegmentRange = RecordedRange<RecordedPipelineSegment>;
pub(crate) type RecordedListItemRange = RecordedRange<RecordedListItem>;
pub(crate) type RecordedElifBranchRange = RecordedRange<RecordedElifBranch>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedCommand {
    pub(crate) span: Span,
    pub(crate) syntax_span: Span,
    pub(crate) syntax_kind: Option<CommandKind>,
    pub(crate) scope: Option<ScopeId>,
    pub(crate) flow_context: Option<FlowContext>,
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
        first: CommandId,
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
    Always {
        body: RecordedCommandRange,
        always_body: RecordedCommandRange,
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
    pub(crate) operator_before: Option<RecordedPipelineOperator>,
    pub(crate) scope: ScopeId,
    pub(crate) command: CommandId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedListItem {
    pub(crate) operator: RecordedListOperator,
    pub(crate) operator_span: Span,
    pub(crate) command: CommandId,
}

#[derive(Debug, Clone, Copy)]
struct FlatListSegment {
    operator_before: Option<(RecordedListOperator, Span)>,
    command: CommandId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedPipelineOperator {
    pub(crate) operator: RecordedPipelineOperatorKind,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordedPipelineOperatorKind {
    Pipe,
    PipeAll,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedElifBranch {
    pub(crate) condition: RecordedCommandRange,
    pub(crate) body: RecordedCommandRange,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordedCommandInfo {
    pub(crate) static_callee: Option<compact_str::CompactString>,
    pub(crate) dynamic_name_span: Option<Span>,
    pub(crate) static_args: Box<[Option<String>]>,
    pub(crate) source_path_template: Option<SourcePathTemplate>,
    pub(crate) zsh_effects: Vec<RecordedZshCommandEffect>,
}

impl RecordedCommandInfo {
    pub(crate) fn is_empty(&self) -> bool {
        self.static_callee.is_none()
            && self.dynamic_name_span.is_none()
            && self.static_args.is_empty()
            && self.source_path_template.is_none()
            && self.zsh_effects.is_empty()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RecordedZshCommandEffect {
    Emulate {
        mode: ZshEmulationMode,
        local: bool,
    },
    EmulateUnknown {
        local: bool,
    },
    SetOptions {
        updates: Vec<RecordedZshOptionUpdate>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedZshOptionUpdate {
    Named { name: Box<str>, enable: bool },
    UnknownName,
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

    pub(crate) fn command(&self, id: CommandId) -> &RecordedCommand {
        &self.commands[id.index()]
    }

    pub(crate) fn command_mut(&mut self, id: CommandId) -> &mut RecordedCommand {
        &mut self.commands[id.index()]
    }

    pub(crate) fn commands(&self) -> &[RecordedCommand] {
        &self.commands
    }

    pub(crate) fn commands_in(&self, range: RecordedCommandRange) -> &[CommandId] {
        range.slice(&self.command_sequence_items)
    }

    pub fn statement_sequence_commands(&self) -> &[StatementSequenceCommand] {
        &self.statement_sequence_commands
    }

    pub(crate) fn push_statement_sequence_command(&mut self, body_span: Span, stmt_span: Span) {
        self.statement_sequence_commands
            .push(StatementSequenceCommand {
                body_span,
                stmt_span,
            });
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

    pub(crate) fn push_command(&mut self, command: RecordedCommand) -> CommandId {
        let id = CommandId(match u32::try_from(self.commands.len()) {
            Ok(id) => id,
            Err(err) => panic!("recorded command count fits in u32: {err}"),
        });
        self.commands.push(command);
        id
    }

    pub(crate) fn push_command_ids(&mut self, command_ids: Vec<CommandId>) -> RecordedCommandRange {
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

#[derive(Debug, Clone, Copy, Default)]
struct ExitEffect {
    may_continue: bool,
    may_return: bool,
    may_exit: bool,
}

impl ExitEffect {
    fn continuing() -> Self {
        Self {
            may_continue: true,
            may_return: false,
            may_exit: false,
        }
    }

    fn returning() -> Self {
        Self {
            may_continue: false,
            may_return: true,
            may_exit: false,
        }
    }

    fn exiting() -> Self {
        Self {
            may_continue: false,
            may_return: false,
            may_exit: true,
        }
    }

    fn combine_alternative(&mut self, other: Self) {
        self.may_continue |= other.may_continue;
        self.may_return |= other.may_return;
        self.may_exit |= other.may_exit;
    }
}

fn compute_script_terminating_call_spans(
    program: &RecordedProgram,
    bindings: &[Binding],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
) -> FxHashSet<SpanKey> {
    if program.function_body_scopes.is_empty() || call_sites.is_empty() {
        return FxHashSet::default();
    }

    let mut terminating_call_spans = FxHashSet::default();
    let mut always_exiting_scopes = FxHashSet::default();

    loop {
        let mut changed = false;
        for &scope in program.function_bodies().keys() {
            if always_exiting_scopes.contains(&scope) {
                continue;
            }
            if function_scope_always_exits_script(program, scope, &terminating_call_spans) {
                always_exiting_scopes.insert(scope);
                changed = true;
            }
        }
        changed |= resolve_script_terminating_call_spans(
            program,
            bindings,
            call_sites,
            visible_function_call_bindings,
            &always_exiting_scopes,
            &mut terminating_call_spans,
        );
        if !changed {
            break;
        }
    }

    terminating_call_spans
}

fn file_entry_can_return_before_function_definition(
    program: &RecordedProgram,
    function_definition_offset: usize,
) -> bool {
    command_range_can_return_before(program, program.file_commands(), function_definition_offset)
}

fn command_range_can_return_before(
    program: &RecordedProgram,
    commands: RecordedCommandRange,
    before_offset: usize,
) -> bool {
    program
        .commands_in(commands)
        .iter()
        .copied()
        .any(|command| command_can_return_before(program, command, before_offset))
}

fn command_can_return_before(
    program: &RecordedProgram,
    command_id: CommandId,
    before_offset: usize,
) -> bool {
    let command = program.command(command_id);
    if command.span.start.offset >= before_offset {
        return false;
    }

    match program.command(command_id).kind {
        RecordedCommandKind::Return => command.span.start.offset < before_offset,
        RecordedCommandKind::List { first, rest } => {
            command_can_return_before(program, first, before_offset)
                || program
                    .list_items(rest)
                    .iter()
                    .any(|item| command_can_return_before(program, item.command, before_offset))
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            command_range_can_return_before(program, condition, before_offset)
                || command_range_can_return_before(program, then_branch, before_offset)
                || program.elif_branches(elif_branches).iter().any(|branch| {
                    command_range_can_return_before(program, branch.condition, before_offset)
                        || command_range_can_return_before(program, branch.body, before_offset)
                })
                || command_range_can_return_before(program, else_branch, before_offset)
        }
        RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body } => {
            command_range_can_return_before(program, body, before_offset)
        }
        RecordedCommandKind::Always { body, always_body } => {
            command_range_can_return_before(program, body, before_offset)
                || command_range_can_return_before(program, always_body, before_offset)
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            command_range_can_return_before(program, condition, before_offset)
                || command_range_can_return_before(program, body, before_offset)
        }
        RecordedCommandKind::Case { arms } => program
            .case_arms(arms)
            .iter()
            .any(|arm| command_range_can_return_before(program, arm.commands, before_offset)),
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Exit
        | RecordedCommandKind::Subshell { .. }
        | RecordedCommandKind::Pipeline { .. } => false,
    }
}

fn resolve_script_terminating_call_spans(
    program: &RecordedProgram,
    bindings: &[Binding],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
    always_exiting_scopes: &FxHashSet<ScopeId>,
    terminating_call_spans: &mut FxHashSet<SpanKey>,
) -> bool {
    let mut changed = false;

    for call in resolved_function_calls_with_callee_scope(
        call_sites,
        visible_function_call_bindings,
        &program.function_body_scopes,
    ) {
        if !always_exiting_scopes.contains(&call.callee_scope) {
            continue;
        }
        if file_entry_can_return_before_function_definition(
            program,
            bindings[call.binding.index()].span.start.offset,
        ) {
            continue;
        }
        let command_span = recorded_command_span_for_call_site(program, call.site);
        changed |= terminating_call_spans.insert(SpanKey::new(command_span));
    }

    changed
}

pub(crate) fn recorded_command_span_for_call_site(
    program: &RecordedProgram,
    site: &CallSite,
) -> Span {
    if let Some(command_span) = program
        .call_command_spans
        .get(&SpanKey::new(site.span))
        .copied()
    {
        return command_span;
    }

    program
        .commands()
        .iter()
        .filter(|command| {
            matches!(command.kind, RecordedCommandKind::Linear)
                && span_contains(command.span, site.span)
        })
        .min_by_key(|command| {
            (
                command.span.end.offset - command.span.start.offset,
                command.span.start.offset,
            )
        })
        .or_else(|| {
            program
                .commands()
                .iter()
                .filter(|command| span_contains(command.span, site.span))
                .min_by_key(|command| {
                    (
                        command.span.end.offset - command.span.start.offset,
                        command.span.start.offset,
                    )
                })
        })
        .map(|command| command.span)
        .unwrap_or(site.span)
}

fn function_scope_always_exits_script(
    program: &RecordedProgram,
    scope: ScopeId,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> bool {
    let effect = sequence_exit_effect(
        program,
        program.function_body(scope),
        terminating_call_spans,
    );
    !effect.may_continue && !effect.may_return && effect.may_exit
}

fn sequence_exit_effect(
    program: &RecordedProgram,
    commands: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut sequence_effect = ExitEffect::default();
    let mut may_continue = true;

    for &command_id in program.commands_in(commands) {
        if !may_continue {
            break;
        }

        let command_effect = command_exit_effect(program, command_id, terminating_call_spans);
        sequence_effect.may_exit |= command_effect.may_exit;
        sequence_effect.may_return |= command_effect.may_return;
        may_continue = command_effect.may_continue;
    }

    sequence_effect.may_continue = may_continue;
    sequence_effect
}

fn command_exit_effect(
    program: &RecordedProgram,
    command_id: CommandId,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let command = program.command(command_id);

    match command.kind {
        RecordedCommandKind::Linear => {
            let span_key = SpanKey::new(command.span);
            if terminating_call_spans.contains(&span_key) {
                ExitEffect::exiting()
            } else {
                ExitEffect::continuing()
            }
        }
        RecordedCommandKind::Break { .. } | RecordedCommandKind::Continue { .. } => {
            ExitEffect::continuing()
        }
        RecordedCommandKind::Return => ExitEffect::returning(),
        RecordedCommandKind::Exit => ExitEffect::exiting(),
        RecordedCommandKind::List { first, rest } => {
            list_exit_effect(program, first, rest, terminating_call_spans)
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => if_exit_effect(
            program,
            condition,
            then_branch,
            elif_branches,
            else_branch,
            terminating_call_spans,
        ),
        RecordedCommandKind::BraceGroup { body } => {
            sequence_exit_effect(program, body, terminating_call_spans)
        }
        RecordedCommandKind::Always { body, always_body } => {
            always_exit_effect(program, body, always_body, terminating_call_spans)
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            loop_exit_effect(program, condition, body, terminating_call_spans)
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body } => {
            counted_loop_exit_effect(program, body, terminating_call_spans)
        }
        RecordedCommandKind::Case { arms } => {
            case_exit_effect(program, arms, terminating_call_spans)
        }
        RecordedCommandKind::Subshell { .. } | RecordedCommandKind::Pipeline { .. } => {
            ExitEffect::continuing()
        }
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn list_exit_effect(
    program: &RecordedProgram,
    first: CommandId,
    rest: RecordedListItemRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let first_effect = command_exit_effect(program, first, terminating_call_spans);
    if !first_effect.may_continue {
        return first_effect;
    }

    let mut effect = first_effect;
    for item in program.list_items(rest) {
        let item_effect = command_exit_effect(program, item.command, terminating_call_spans);
        effect.may_continue = true;
        effect.may_return |= item_effect.may_return;
        effect.may_exit |= item_effect.may_exit;
        effect.may_continue |= item_effect.may_continue;
    }
    effect
}

fn if_exit_effect(
    program: &RecordedProgram,
    condition: RecordedCommandRange,
    then_branch: RecordedCommandRange,
    elif_branches: RecordedElifBranchRange,
    else_branch: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let condition_effect = sequence_exit_effect(program, condition, terminating_call_spans);
    let mut effect = ExitEffect {
        may_continue: false,
        may_return: condition_effect.may_return,
        may_exit: condition_effect.may_exit,
    };

    if !condition_effect.may_continue {
        return effect;
    }

    effect.combine_alternative(sequence_exit_effect(
        program,
        then_branch,
        terminating_call_spans,
    ));
    effect.combine_alternative(elif_chain_exit_effect(
        program,
        elif_branches,
        else_branch,
        terminating_call_spans,
    ));
    effect
}

fn elif_chain_exit_effect(
    program: &RecordedProgram,
    elif_branches: RecordedElifBranchRange,
    else_branch: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut false_path = if else_branch.is_empty() {
        ExitEffect::continuing()
    } else {
        sequence_exit_effect(program, else_branch, terminating_call_spans)
    };

    for branch in program.elif_branches(elif_branches).iter().rev() {
        let condition_effect =
            sequence_exit_effect(program, branch.condition, terminating_call_spans);
        let mut branch_effect = ExitEffect {
            may_continue: false,
            may_return: condition_effect.may_return,
            may_exit: condition_effect.may_exit,
        };

        if condition_effect.may_continue {
            branch_effect.combine_alternative(sequence_exit_effect(
                program,
                branch.body,
                terminating_call_spans,
            ));
            branch_effect.combine_alternative(false_path);
        }

        false_path = branch_effect;
    }

    false_path
}

fn loop_exit_effect(
    program: &RecordedProgram,
    condition: RecordedCommandRange,
    body: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut effect = ExitEffect::continuing();
    let condition_effect = sequence_exit_effect(program, condition, terminating_call_spans);
    let body_effect = sequence_exit_effect(program, body, terminating_call_spans);
    effect.may_return |= condition_effect.may_return || body_effect.may_return;
    effect.may_exit |= condition_effect.may_exit || body_effect.may_exit;
    effect
}

fn counted_loop_exit_effect(
    program: &RecordedProgram,
    body: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut effect = ExitEffect::continuing();
    let body_effect = sequence_exit_effect(program, body, terminating_call_spans);
    effect.may_return |= body_effect.may_return;
    effect.may_exit |= body_effect.may_exit;
    effect
}

fn case_exit_effect(
    program: &RecordedProgram,
    arms: RecordedCaseArmRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let arms = program.case_arms(arms);
    let mut effect = if arms.iter().any(|arm| arm.matches_anything) {
        ExitEffect::default()
    } else {
        ExitEffect::continuing()
    };

    for arm in arms {
        effect.combine_alternative(sequence_exit_effect(
            program,
            arm.commands,
            terminating_call_spans,
        ));
    }

    effect
}

fn always_exit_effect(
    program: &RecordedProgram,
    body: RecordedCommandRange,
    always_body: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let body_effect = sequence_exit_effect(program, body, terminating_call_spans);
    let always_effect = sequence_exit_effect(program, always_body, terminating_call_spans);
    let mut effect = ExitEffect::default();

    if body_effect.may_continue {
        effect.combine_alternative(always_effect);
    }
    if body_effect.may_return {
        effect.may_return = true;
    }
    if body_effect.may_exit {
        effect.may_exit = true;
    }

    effect
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
