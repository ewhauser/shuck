//! Control-flow modeling for the semantic command stream.
//!
//! The semantic builder first records shell syntax into a compact command stream, then this
//! module turns that stream into a control-flow graph (CFG). A command such as:
//!
//! ```sh
//! if prepare; then
//!   deploy
//! else
//!   rollback
//! fi
//! notify
//! ```
//!
//! becomes blocks for the condition and each branch, with `ConditionalTrue` and
//! `ConditionalFalse` edges from the condition and sequential edges from whichever branch can
//! continue into `notify`. The graph keeps shell-specific shape instead of pretending the script
//! is a simple list of statements: loops add back-edges for `continue`, `break` edges leave the
//! active loop, and `case` arms model fallthrough terminators separately from normal exits.
//!
//! Nested execution regions are represented too. For example, in:
//!
//! ```sh
//! echo "$(date)" | while read line; do
//!   printf '%s\n' "$line"
//! done
//! ```
//!
//! the command substitution and pipeline segment bodies are attached as isolated regions. That
//! lets later analyses ask whether a reference occurs in the top-level flow, a function body, a
//! subshell-like context, or another nested region without having to re-walk the AST.
//!
//! The split here mirrors that pipeline:
//!
//! - `recorded` owns the compact command IR emitted by semantic traversal.
//! - `termination` reasons about function calls that always terminate script execution.
//! - `builder` constructs blocks and edges from recorded commands.
//! - `graph` exposes the finished CFG surface consumed by reachability and dataflow.

mod builder;
mod graph;
mod recorded;
mod termination;

#[cfg(test)]
mod tests;

pub(crate) use builder::build_control_flow_graph;
pub use graph::{
    BasicBlock, BlockId, CommandConditionRole, ControlFlowGraph, EdgeKind, FlowContext,
    UnreachableCauseKind,
};
pub(crate) use graph::{FlowEdge, UnreachableCause};
pub use recorded::{
    BuiltinCommandKind, CommandId, CommandKind, CompoundCommandKind, StatementSequenceCommand,
};
pub(crate) use recorded::{
    IsolatedRegion, RecordedCaseArm, RecordedCaseArmRange, RecordedCommand, RecordedCommandInfo,
    RecordedCommandKind, RecordedCommandRange, RecordedElifBranch, RecordedElifBranchRange,
    RecordedListItem, RecordedListItemRange, RecordedListOperator, RecordedPipelineOperator,
    RecordedPipelineOperatorKind, RecordedPipelineSegment, RecordedPipelineSegmentRange,
    RecordedProgram, RecordedRegionRange, RecordedZshCommandEffect, RecordedZshOptionUpdate,
};
pub(crate) use termination::recorded_command_span_for_call_site;
