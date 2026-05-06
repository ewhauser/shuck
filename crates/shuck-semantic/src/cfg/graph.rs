//! Public CFG data structures and query methods.
//!
//! The graph stores the result of control-flow construction as basic blocks plus
//! labeled edges. A script such as:
//!
//! ```sh
//! if build; then
//!   test
//! else
//!   exit 1
//! fi
//! deploy
//! ```
//!
//! has branch edges out of the `build` condition, an explicit terminator block
//! for `exit`, and a natural path from the successful branch to `deploy`. The
//! API here exposes those blocks, edge labels, scope entries, exits, and
//! unreachable blocks to dataflow and reachability consumers.

use rustc_hash::FxHashMap;
use shuck_ast::Span;
use smallvec::SmallVec;

use crate::{BindingId, ReferenceId, ScopeId, SpanKey};

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
    pub(crate) blocks: Vec<BasicBlock>,
    pub(crate) successors: Vec<SmallVec<[FlowEdge; 2]>>,
    pub(crate) predecessors: Vec<SmallVec<[BlockId; 2]>>,
    pub(crate) entry: BlockId,
    pub(crate) exits: Vec<BlockId>,
    pub(crate) natural_exits: Vec<BlockId>,
    pub(crate) script_terminators: Vec<BlockId>,
    pub(crate) script_always_terminates: bool,
    pub(crate) unreachable: Vec<BlockId>,
    pub(crate) scope_entries: FxHashMap<ScopeId, BlockId>,
    pub(crate) scope_exits: FxHashMap<ScopeId, SmallVec<[BlockId; 2]>>,
    pub(crate) command_blocks: FxHashMap<SpanKey, SmallVec<[BlockId; 1]>>,
    pub(crate) unreachable_causes: FxHashMap<BlockId, UnreachableCause>,
}

pub(crate) type FlowEdge = (BlockId, EdgeKind);

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
    pub(super) fn shell_terminator(span: Span) -> Self {
        Self {
            span,
            kind: UnreachableCauseKind::ShellTerminator,
        }
    }

    pub(super) fn loop_control(span: Span) -> Self {
        Self {
            span,
            kind: UnreachableCauseKind::LoopControl,
        }
    }
}
