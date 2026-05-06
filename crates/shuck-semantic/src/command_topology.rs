//! Command topology and command-shape queries for semantic models.
//!
//! Shell syntax has several overlapping ideas of "the command at this point". A
//! simple command like `echo hi` is straightforward, but compound commands,
//! logical lists, pipelines, functions, and command substitutions all produce
//! different useful views of the same source.
//!
//! For example, `if build && test; then deploy; fi` has condition commands
//! (`build`, `test`) nested inside an `if`, and the condition list itself has
//! logical-list structure. A pipeline such as `producer | filter |& sink` is
//! similarly represented as a command tree while also exposing a flattened
//! left-to-right pipeline view for rules that care about neighboring segments.
//!
//! Command substitutions are tracked as nested word commands: in
//! `echo "$(date)"`, the outer `echo` command belongs to the structural command
//! stream, while `date` is still queryable as a syntax-backed command with a
//! nested-word depth.

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Span;
use smallvec::SmallVec;

use crate::cfg::{
    CommandConditionRole, CommandId, CommandKind, FlowContext, RecordedCommandKind,
    RecordedListOperator, RecordedPipelineOperatorKind, RecordedProgram,
};
use crate::{ScopeId, SemanticModel, SpanKey};

#[derive(Debug)]
pub(crate) struct CommandTopology {
    /// Commands that correspond directly to syntax nodes, excluding synthetic
    /// range markers recorded only to model control-flow structure.
    pub(crate) syntax_backed_ids: Vec<CommandId>,
    /// Source-order command stream used by callers that want top-level shell
    /// structure without descending into nested word command regions.
    pub(crate) structural_ids: Vec<CommandId>,
    /// Per-command metadata for syntax-backed commands.
    pub(crate) contexts: Vec<Option<SemanticCommandContext>>,
    /// Lookup from exact syntax spans to recorded commands.
    pub(crate) ids_by_syntax_span: FxHashMap<SpanKey, SmallVec<[CommandId; 1]>>,
    /// Parent links for the full structural command tree.
    pub(crate) parent_ids: Vec<Option<CommandId>>,
    /// Child links for the full structural command tree.
    pub(crate) child_ids: Vec<Vec<CommandId>>,
    /// Parent links that skip synthetic structural commands and only connect
    /// syntax-backed commands.
    pub(crate) syntax_backed_parent_ids: Vec<Option<CommandId>>,
    /// Child links that skip synthetic structural commands and only connect
    /// syntax-backed commands.
    pub(crate) syntax_backed_child_ids: Vec<Vec<CommandId>>,
    /// Offset index over syntax spans, so `if echo hi; then :; fi` points at the
    /// innermost syntax command containing an offset.
    pub(crate) syntax_containing_offset_entries: Vec<CommandContainingOffsetEntry>,
    /// Offset index over statement spans, including redirects attached outside
    /// the syntax-node span.
    pub(crate) containing_offset_entries: Vec<CommandContainingOffsetEntry>,
}

/// Syntax-backed command metadata derived during semantic traversal.
///
/// A context answers questions that require both syntax and semantic state. For
/// example, in `if test -f file; then cat file; fi`, the `test` command context
/// records that it is inside an `if` condition, while `cat` is in the body. In
/// `echo "$(date)"`, the `date` context is marked as a nested word command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCommandContext {
    id: CommandId,
    span: Span,
    syntax_span: Span,
    kind: CommandKind,
    scope: ScopeId,
    flow: FlowContext,
    structural: bool,
    nested_word_command: bool,
    nested_word_command_depth: usize,
    in_if_condition: bool,
    in_elif_condition: bool,
    condition_role: Option<CommandConditionRole>,
}

impl SemanticCommandContext {
    /// Semantic command identifier.
    pub fn id(&self) -> CommandId {
        self.id
    }

    /// Statement span including redirects.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Span of the syntactic command node.
    pub fn syntax_span(&self) -> Span {
        self.syntax_span
    }

    /// Syntax command kind.
    pub fn kind(&self) -> CommandKind {
        self.kind
    }

    /// Scope active at the command.
    pub fn scope(&self) -> ScopeId {
        self.scope
    }

    /// Flow context active at the command.
    pub fn flow(&self) -> FlowContext {
        self.flow
    }

    /// Whether this command is part of the structural command stream.
    pub fn is_structural(&self) -> bool {
        self.structural
    }

    /// Whether this command came from a command-like expansion in a word.
    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    /// Number of command-like word-expansion regions enclosing this command.
    pub fn nested_word_command_depth(&self) -> usize {
        self.nested_word_command_depth
    }

    /// Whether this command is inside an `if` or `elif` condition list.
    pub fn is_in_if_condition(&self) -> bool {
        self.in_if_condition
    }

    /// Whether this command is inside an `elif` condition list.
    pub fn is_in_elif_condition(&self) -> bool {
        self.in_elif_condition
    }

    /// Condition-list role inherited from surrounding shell syntax, if any.
    pub fn condition_role(&self) -> Option<CommandConditionRole> {
        self.condition_role
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommandContainingOffsetEntry {
    start_offset: usize,
    end_offset: usize,
    id: CommandId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CommandContainingOffsetEventKind {
    End,
    Start,
}

#[derive(Debug, Clone, Copy)]
enum CommandContainingOffsetSpan {
    Syntax,
    Statement,
}

#[derive(Debug, Clone, Copy)]
struct CommandContainingOffsetEvent {
    offset: usize,
    end_offset: usize,
    id: CommandId,
    kind: CommandContainingOffsetEventKind,
}

/// A flattened logical list command recorded by semantic analysis.
///
/// Logical lists join commands with `&&` or `||`. For `build && test ||
/// recover`, the flattened view has three segments in source order, with the
/// second segment preceded by `&&` and the third preceded by `||`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticListCommand {
    /// Span of the complete logical list command.
    pub span: Span,
    /// Commands in execution order, including the first command and each following list item.
    pub segments: Box<[SemanticListSegment]>,
}

/// A flattened logical list segment recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticListSegment {
    /// Span of the segment command.
    pub command_span: Span,
    /// Operator that precedes this segment, or `None` for the first segment.
    pub operator_before: Option<SemanticListOperator>,
}

/// A logical list operator recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticListOperator {
    /// Logical operator kind.
    pub kind: SemanticListOperatorKind,
    /// Span of the operator token.
    pub span: Span,
}

/// Logical list operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticListOperatorKind {
    /// `&&`
    And,
    /// `||`
    Or,
}

#[derive(Debug, Clone, Copy)]
struct RecordedListOperatorWithSpan {
    operator: RecordedListOperator,
    span: Span,
}

/// A flattened pipeline command recorded by semantic analysis.
///
/// Pipelines join commands with `|` or `|&`. For `producer | filter |& sink`,
/// the flattened view has three segments and preserves the operator before each
/// non-initial segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticPipelineCommand {
    /// Span of the complete pipeline command.
    pub span: Span,
    /// Commands in execution order, including the first command and each following pipeline segment.
    pub segments: Box<[SemanticPipelineSegment]>,
}

/// A flattened pipeline segment recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticPipelineSegment {
    /// Span of the segment command.
    pub command_span: Span,
    /// Operator that precedes this segment, or `None` for the first segment.
    pub operator_before: Option<SemanticPipelineOperator>,
}

/// A pipeline operator recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticPipelineOperator {
    /// Pipeline operator kind.
    pub kind: SemanticPipelineOperatorKind,
    /// Span of the operator token.
    pub span: Span,
}

/// Pipeline operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticPipelineOperatorKind {
    /// `|`
    Pipe,
    /// `|&`
    PipeAll,
}

impl SemanticModel {
    /// Returns command ids for every syntax-backed recorded command.
    pub fn commands(&self) -> &[CommandId] {
        &self.command_topology().syntax_backed_ids
    }

    /// Returns command ids for the structural command stream.
    pub fn structural_commands(&self) -> &[CommandId] {
        &self.command_topology().structural_ids
    }

    /// Returns the recorded semantic context for `id`.
    pub fn command_context(&self, id: CommandId) -> Option<&SemanticCommandContext> {
        self.command_topology()
            .contexts
            .get(id.index())
            .and_then(Option::as_ref)
    }

    /// Iterates recorded command contexts in command-id order.
    pub fn command_contexts(&self) -> impl Iterator<Item = &SemanticCommandContext> {
        self.command_topology()
            .contexts
            .iter()
            .filter_map(Option::as_ref)
    }

    /// Iterates only structural command contexts.
    pub fn structural_command_contexts(&self) -> impl Iterator<Item = &SemanticCommandContext> {
        self.command_contexts()
            .filter(|context| context.is_structural())
    }

    /// Returns the surrounding condition-list role for `id`, if one applies.
    pub fn command_condition_role(&self, id: CommandId) -> Option<CommandConditionRole> {
        self.command_context(id)
            .and_then(SemanticCommandContext::condition_role)
    }

    /// Returns whether `id` came from a command-like expansion inside a word.
    pub fn command_is_nested_word_command(&self, id: CommandId) -> bool {
        self.command_context(id)
            .is_some_and(SemanticCommandContext::is_nested_word_command)
    }

    /// Returns the recorded statement span for `id`.
    pub fn command_span(&self, id: CommandId) -> Span {
        self.recorded_program.command(id).span
    }

    /// Returns the underlying syntax-node span for `id`.
    pub fn command_syntax_span(&self, id: CommandId) -> Span {
        self.recorded_program.command(id).syntax_span
    }

    /// Returns the syntax-backed command kind for `id`.
    pub fn command_kind(&self, id: CommandId) -> CommandKind {
        self.command_syntax_kind(id)
            .expect("semantic command syntax kind is recorded")
    }

    /// Returns the first syntax-backed command recorded for exact syntax span `span`.
    pub fn command_by_span(&self, span: Span) -> Option<CommandId> {
        self.command_topology()
            .ids_by_syntax_span
            .get(&SpanKey::new(span))
            .and_then(|ids| {
                ids.iter()
                    .copied()
                    .find(|id| self.command_syntax_kind(*id).is_some())
            })
    }

    /// Returns the command recorded for exact syntax span `span` and syntax kind `kind`.
    pub fn command_by_span_and_kind(&self, span: Span, kind: CommandKind) -> Option<CommandId> {
        self.command_topology()
            .ids_by_syntax_span
            .get(&SpanKey::new(span))
            .and_then(|ids| {
                ids.iter()
                    .copied()
                    .find(|id| self.command_syntax_kind(*id) == Some(kind))
            })
    }

    /// Returns the structural parent command of `id`, if one exists.
    pub fn command_parent_id(&self, id: CommandId) -> Option<CommandId> {
        self.command_topology().parent_ids[id.index()]
    }

    /// Returns structural child commands nested under `id`.
    pub fn command_children(&self, id: CommandId) -> &[CommandId] {
        &self.command_topology().child_ids[id.index()]
    }

    /// Returns the syntax-backed parent command of `id`, if one exists.
    pub fn syntax_backed_command_parent_id(&self, id: CommandId) -> Option<CommandId> {
        self.command_topology().syntax_backed_parent_ids[id.index()]
    }

    /// Returns syntax-backed child commands nested directly under `id`.
    pub fn syntax_backed_command_children(&self, id: CommandId) -> &[CommandId] {
        &self.command_topology().syntax_backed_child_ids[id.index()]
    }

    /// Returns the innermost syntax-backed command whose syntax span contains `offset`.
    pub fn innermost_command_id_at(&self, offset: usize) -> Option<CommandId> {
        let topology = self.command_topology();
        innermost_command_id_in_containing_offset_entries(
            &topology.syntax_containing_offset_entries,
            offset,
        )
    }

    /// Returns the innermost command known to contain `offset` using the topology index.
    pub fn innermost_command_id_containing_offset(&self, offset: usize) -> Option<CommandId> {
        let topology = self.command_topology();
        innermost_command_id_in_containing_offset_entries(
            &topology.containing_offset_entries,
            offset,
        )
    }

    /// Returns logical list commands flattened by the semantic traversal.
    pub fn list_commands(&self) -> Vec<SemanticListCommand> {
        self.recorded_program
            .commands()
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let RecordedCommandKind::List { first, rest } = command.kind else {
                    return None;
                };
                let command_id = CommandId(index as u32);
                if self.command_parent_id(command_id).is_some_and(|parent| {
                    matches!(
                        self.recorded_program.command(parent).kind,
                        RecordedCommandKind::List { .. }
                    )
                }) {
                    return None;
                }

                let mut segments = Vec::new();
                self.flatten_list_segment(first, None, &mut segments);
                for item in self.recorded_program.list_items(rest) {
                    self.flatten_list_segment(
                        item.command,
                        Some(RecordedListOperatorWithSpan {
                            operator: item.operator,
                            span: item.operator_span,
                        }),
                        &mut segments,
                    );
                }

                Some(SemanticListCommand {
                    span: command.span,
                    segments: segments.into_boxed_slice(),
                })
            })
            .collect()
    }

    fn flatten_list_segment(
        &self,
        command: CommandId,
        operator_before: Option<RecordedListOperatorWithSpan>,
        out: &mut Vec<SemanticListSegment>,
    ) {
        if let RecordedCommandKind::List { first, rest } =
            self.recorded_program.command(command).kind
        {
            self.flatten_list_segment(first, operator_before, out);
            for item in self.recorded_program.list_items(rest) {
                self.flatten_list_segment(
                    item.command,
                    Some(RecordedListOperatorWithSpan {
                        operator: item.operator,
                        span: item.operator_span,
                    }),
                    out,
                );
            }
            return;
        }

        out.push(SemanticListSegment {
            command_span: self.recorded_program.command(command).span,
            operator_before: operator_before.map(|operator| SemanticListOperator {
                kind: match operator.operator {
                    RecordedListOperator::And => SemanticListOperatorKind::And,
                    RecordedListOperator::Or => SemanticListOperatorKind::Or,
                },
                span: operator.span,
            }),
        });
    }

    /// Returns pipeline commands flattened by the semantic traversal.
    pub fn pipeline_commands(&self) -> Vec<SemanticPipelineCommand> {
        self.recorded_program
            .commands()
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let RecordedCommandKind::Pipeline { segments } = command.kind else {
                    return None;
                };
                let command_id = CommandId(index as u32);
                if self.command_parent_id(command_id).is_some_and(|parent| {
                    matches!(
                        self.recorded_program.command(parent).kind,
                        RecordedCommandKind::Pipeline { .. }
                    )
                }) {
                    return None;
                }

                let mut flattened = Vec::new();
                for segment in self.recorded_program.pipeline_segments(segments) {
                    self.flatten_pipeline_segment(
                        segment.command,
                        segment.operator_before,
                        &mut flattened,
                    );
                }

                Some(SemanticPipelineCommand {
                    span: command.span,
                    segments: flattened.into_boxed_slice(),
                })
            })
            .collect()
    }

    fn flatten_pipeline_segment(
        &self,
        command: CommandId,
        operator_before: Option<crate::cfg::RecordedPipelineOperator>,
        out: &mut Vec<SemanticPipelineSegment>,
    ) {
        if let RecordedCommandKind::Pipeline { segments } =
            self.recorded_program.command(command).kind
        {
            for (index, segment) in self
                .recorded_program
                .pipeline_segments(segments)
                .iter()
                .enumerate()
            {
                let operator = if index == 0 {
                    operator_before
                } else {
                    segment.operator_before
                };
                self.flatten_pipeline_segment(segment.command, operator, out);
            }
            return;
        }

        out.push(SemanticPipelineSegment {
            command_span: self.recorded_program.command(command).span,
            operator_before: operator_before.map(|operator| SemanticPipelineOperator {
                kind: match operator.operator {
                    RecordedPipelineOperatorKind::Pipe => SemanticPipelineOperatorKind::Pipe,
                    RecordedPipelineOperatorKind::PipeAll => SemanticPipelineOperatorKind::PipeAll,
                },
                span: operator.span,
            }),
        });
    }

    fn command_topology(&self) -> &CommandTopology {
        self.command_topology
            .get_or_init(|| build_command_topology(self))
    }

    pub(crate) fn command_syntax_kind(&self, id: CommandId) -> Option<CommandKind> {
        self.recorded_program.command(id).syntax_kind
    }
}

fn build_command_topology(model: &SemanticModel) -> CommandTopology {
    // Build two related trees. The structural tree preserves shell nesting such
    // as `if`, `while`, function bodies, and command substitutions. The
    // syntax-backed tree is the public query surface for commands that callers
    // can point back to concrete source syntax.
    let program = model.recorded_program();
    let command_count = program.commands().len();
    let ids = (0..command_count)
        .map(|index| CommandId(index as u32))
        .collect::<Vec<_>>();
    let mut ids_by_syntax_span = FxHashMap::<SpanKey, SmallVec<[CommandId; 1]>>::default();
    let mut parent_ids = vec![None; command_count];
    let mut child_ids = vec![Vec::new(); command_count];
    let mut nested_region_command_ids = FxHashSet::default();
    let mut nested_region_root_command_ids = FxHashSet::default();

    for id in ids.iter().copied() {
        let command = program.command(id);
        ids_by_syntax_span
            .entry(SpanKey::new(command.syntax_span))
            .or_default()
            .push(id);
        record_command_children(
            program,
            id,
            &mut parent_ids,
            &mut child_ids,
            &mut nested_region_command_ids,
            &mut nested_region_root_command_ids,
        );
    }
    let structural_parent_ids = parent_ids.clone();
    let structural_child_ids = child_ids.clone();

    attach_function_body_commands(model, &ids, &mut parent_ids, &mut child_ids);
    attach_containing_command_parents(model, &ids, &mut parent_ids, &mut child_ids);
    let nested_region_depths = build_nested_region_depths(
        command_count,
        &parent_ids,
        &child_ids,
        &nested_region_root_command_ids,
    );

    let mut structural_ids = ids
        .iter()
        .copied()
        .filter(|id| !nested_region_command_ids.contains(id))
        .collect::<Vec<_>>();
    structural_ids
        .sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));

    let syntax_backed_ids = ids
        .into_iter()
        .filter(|id| program.command(*id).syntax_kind.is_some())
        .collect::<Vec<_>>();
    let syntax_backed_parent_ids =
        build_syntax_backed_command_parent_ids(model, &syntax_backed_ids, &parent_ids);
    let syntax_backed_child_ids =
        build_command_child_ids(command_count, &syntax_backed_ids, &syntax_backed_parent_ids);
    let syntax_containing_offset_entries = build_command_containing_offset_entries(
        model,
        &syntax_backed_ids,
        CommandContainingOffsetSpan::Syntax,
    );
    let containing_offset_entries = build_command_containing_offset_entries(
        model,
        &syntax_backed_ids,
        CommandContainingOffsetSpan::Statement,
    );
    let condition_contexts = build_command_condition_contexts(
        program,
        command_count,
        &structural_parent_ids,
        &structural_child_ids,
    );
    let contexts = build_command_contexts(
        model,
        &syntax_backed_ids,
        &structural_ids,
        &nested_region_command_ids,
        &nested_region_depths,
        &condition_contexts,
    );

    CommandTopology {
        syntax_backed_ids,
        structural_ids,
        contexts,
        ids_by_syntax_span,
        parent_ids,
        child_ids,
        syntax_backed_parent_ids,
        syntax_backed_child_ids,
        syntax_containing_offset_entries,
        containing_offset_entries,
    }
}

fn build_command_contexts(
    model: &SemanticModel,
    syntax_backed_ids: &[CommandId],
    structural_ids: &[CommandId],
    nested_region_command_ids: &FxHashSet<CommandId>,
    nested_region_depths: &[usize],
    condition_contexts: &[CommandConditionContext],
) -> Vec<Option<SemanticCommandContext>> {
    let program = model.recorded_program();
    let structural_ids = structural_ids.iter().copied().collect::<FxHashSet<_>>();
    let mut contexts = vec![None; program.commands().len()];
    for id in syntax_backed_ids.iter().copied() {
        let command = program.command(id);
        let Some(kind) = command.syntax_kind else {
            continue;
        };
        let Some(scope) = command.scope else {
            continue;
        };
        let Some(flow) = command.flow_context else {
            continue;
        };
        contexts[id.index()] = Some(SemanticCommandContext {
            id,
            span: command.span,
            syntax_span: command.syntax_span,
            kind,
            scope,
            flow,
            structural: structural_ids.contains(&id),
            nested_word_command: nested_region_command_ids.contains(&id),
            nested_word_command_depth: nested_region_depths[id.index()],
            in_if_condition: condition_contexts
                .get(id.index())
                .is_some_and(|context| context.in_if_condition),
            in_elif_condition: condition_contexts
                .get(id.index())
                .is_some_and(|context| context.in_elif_condition),
            condition_role: condition_contexts
                .get(id.index())
                .and_then(|context| context.role),
        });
    }
    contexts
}

#[derive(Debug, Clone, Copy, Default)]
struct CommandConditionContext {
    role: Option<CommandConditionRole>,
    in_if_condition: bool,
    in_elif_condition: bool,
}

fn build_command_condition_contexts(
    program: &RecordedProgram,
    command_count: usize,
    parent_ids: &[Option<CommandId>],
    child_ids: &[Vec<CommandId>],
) -> Vec<CommandConditionContext> {
    let mut starts = vec![SmallVec::<[ConditionAssignment; 1]>::new(); command_count];
    let mut contexts = vec![CommandConditionContext::default(); command_count];
    for id in (0..command_count).map(|index| CommandId(index as u32)) {
        record_command_condition_starts(program, id, &mut starts);
    }

    let mut visited = vec![false; command_count];
    for id in (0..command_count).map(|index| CommandId(index as u32)) {
        if parent_ids[id.index()].is_none() {
            propagate_command_condition_contexts(
                id,
                CommandConditionContext::default(),
                &starts,
                child_ids,
                &mut contexts,
                &mut visited,
            );
        }
    }
    for id in (0..command_count).map(|index| CommandId(index as u32)) {
        if !visited[id.index()] {
            propagate_command_condition_contexts(
                id,
                CommandConditionContext::default(),
                &starts,
                child_ids,
                &mut contexts,
                &mut visited,
            );
        }
    }
    contexts
}

fn record_command_condition_starts(
    program: &RecordedProgram,
    id: CommandId,
    starts: &mut [SmallVec<[ConditionAssignment; 1]>],
) {
    match program.command(id).kind {
        RecordedCommandKind::If {
            condition,
            elif_branches,
            ..
        } => {
            record_condition_range_starts(
                program,
                condition,
                ConditionAssignment {
                    role: CommandConditionRole::If,
                    in_if_condition: true,
                    in_elif_condition: false,
                },
                starts,
            );
            for branch in program.elif_branches(elif_branches) {
                record_condition_range_starts(
                    program,
                    branch.condition,
                    ConditionAssignment {
                        role: CommandConditionRole::Elif,
                        in_if_condition: true,
                        in_elif_condition: true,
                    },
                    starts,
                );
            }
        }
        RecordedCommandKind::While { condition, .. } => {
            record_condition_range_starts(
                program,
                condition,
                ConditionAssignment {
                    role: CommandConditionRole::While,
                    in_if_condition: false,
                    in_elif_condition: false,
                },
                starts,
            );
        }
        RecordedCommandKind::Until { condition, .. } => {
            record_condition_range_starts(
                program,
                condition,
                ConditionAssignment {
                    role: CommandConditionRole::Until,
                    in_if_condition: false,
                    in_elif_condition: false,
                },
                starts,
            );
        }
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit
        | RecordedCommandKind::List { .. }
        | RecordedCommandKind::For { .. }
        | RecordedCommandKind::Select { .. }
        | RecordedCommandKind::ArithmeticFor { .. }
        | RecordedCommandKind::Case { .. }
        | RecordedCommandKind::BraceGroup { .. }
        | RecordedCommandKind::Always { .. }
        | RecordedCommandKind::Subshell { .. }
        | RecordedCommandKind::Pipeline { .. } => {}
    }
}

#[derive(Debug, Clone, Copy)]
struct ConditionAssignment {
    role: CommandConditionRole,
    in_if_condition: bool,
    in_elif_condition: bool,
}

fn record_condition_range_starts(
    program: &RecordedProgram,
    range: crate::cfg::RecordedCommandRange,
    assignment: ConditionAssignment,
    starts: &mut [SmallVec<[ConditionAssignment; 1]>],
) {
    for command in program.commands_in(range).iter().copied() {
        starts[command.index()].push(assignment);
    }
}

fn propagate_command_condition_contexts(
    root: CommandId,
    inherited: CommandConditionContext,
    starts: &[SmallVec<[ConditionAssignment; 1]>],
    child_ids: &[Vec<CommandId>],
    contexts: &mut [CommandConditionContext],
    visited: &mut [bool],
) {
    let mut stack = vec![(root, inherited)];
    while let Some((id, mut context)) = stack.pop() {
        if visited[id.index()] {
            continue;
        }
        visited[id.index()] = true;

        for assignment in &starts[id.index()] {
            context.role = Some(assignment.role);
            context.in_if_condition |= assignment.in_if_condition;
            context.in_elif_condition |= assignment.in_elif_condition;
        }
        contexts[id.index()] = context;

        for child in child_ids[id.index()].iter().rev().copied() {
            stack.push((child, context));
        }
    }
}

fn build_syntax_backed_command_parent_ids(
    model: &SemanticModel,
    syntax_backed_ids: &[CommandId],
    parent_ids: &[Option<CommandId>],
) -> Vec<Option<CommandId>> {
    let mut syntax_backed_parent_ids = vec![None; parent_ids.len()];
    for id in syntax_backed_ids.iter().copied() {
        let mut current = parent_ids[id.index()];
        while let Some(parent) = current {
            if model.command_syntax_kind(parent).is_some() {
                syntax_backed_parent_ids[id.index()] = Some(parent);
                break;
            }
            current = parent_ids[parent.index()];
        }
    }
    syntax_backed_parent_ids
}

fn build_command_child_ids(
    command_count: usize,
    command_ids: &[CommandId],
    parent_ids: &[Option<CommandId>],
) -> Vec<Vec<CommandId>> {
    let mut child_ids = vec![Vec::new(); command_count];
    for child in command_ids.iter().copied() {
        if let Some(parent) = parent_ids[child.index()] {
            child_ids[parent.index()].push(child);
        }
    }
    child_ids
}

fn build_command_containing_offset_entries(
    model: &SemanticModel,
    syntax_backed_ids: &[CommandId],
    span_kind: CommandContainingOffsetSpan,
) -> Vec<CommandContainingOffsetEntry> {
    let mut events = syntax_backed_ids
        .iter()
        .copied()
        .flat_map(|id| {
            let span = match span_kind {
                CommandContainingOffsetSpan::Syntax => model.command_syntax_span(id),
                CommandContainingOffsetSpan::Statement => model.command_span(id),
            };
            [
                CommandContainingOffsetEvent {
                    offset: span.start.offset,
                    end_offset: span.end.offset,
                    id,
                    kind: CommandContainingOffsetEventKind::Start,
                },
                CommandContainingOffsetEvent {
                    offset: span.end.offset.saturating_add(1),
                    end_offset: span.end.offset,
                    id,
                    kind: CommandContainingOffsetEventKind::End,
                },
            ]
        })
        .collect::<Vec<_>>();
    events.sort_unstable_by(|left, right| {
        left.offset
            .cmp(&right.offset)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| right.end_offset.cmp(&left.end_offset))
            .then_with(|| left.id.index().cmp(&right.id.index()))
    });

    let mut entries = Vec::new();
    let mut active = Vec::<CommandId>::new();
    let mut index = 0;
    while let Some(event) = events.get(index).copied() {
        let offset = event.offset;
        while events.get(index).is_some_and(|event| {
            event.offset == offset && event.kind == CommandContainingOffsetEventKind::End
        }) {
            let id = events[index].id;
            active.retain(|active_id| *active_id != id);
            index += 1;
        }
        while events.get(index).is_some_and(|event| {
            event.offset == offset && event.kind == CommandContainingOffsetEventKind::Start
        }) {
            active.push(events[index].id);
            index += 1;
        }

        let Some(next_offset) = events.get(index).map(|event| event.offset) else {
            break;
        };
        if offset < next_offset
            && let Some(id) = active.last().copied()
        {
            push_command_containing_offset_entry(&mut entries, offset, next_offset - 1, id);
        }
    }

    entries
}

pub(crate) fn innermost_command_id_in_containing_offset_entries(
    entries: &[CommandContainingOffsetEntry],
    offset: usize,
) -> Option<CommandId> {
    let upper_bound = entries.partition_point(|entry| entry.start_offset <= offset);
    let entry = entries.get(upper_bound.checked_sub(1)?)?;
    (offset <= entry.end_offset).then_some(entry.id)
}

fn push_command_containing_offset_entry(
    entries: &mut Vec<CommandContainingOffsetEntry>,
    start_offset: usize,
    end_offset: usize,
    id: CommandId,
) {
    if let Some(last) = entries.last_mut()
        && last.id == id
        && last.end_offset.saturating_add(1) == start_offset
    {
        last.end_offset = end_offset;
        return;
    }

    entries.push(CommandContainingOffsetEntry {
        start_offset,
        end_offset,
        id,
    });
}

fn attach_containing_command_parents(
    model: &SemanticModel,
    command_ids: &[CommandId],
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    let mut sorted = command_ids.to_vec();
    sorted.sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));

    let mut stack = Vec::<CommandId>::new();
    for child in sorted {
        let child_span = model.command_syntax_span(child);
        while stack.last().is_some_and(|candidate| {
            !contains_command_span(model.command_syntax_span(*candidate), child_span)
        }) {
            stack.pop();
        }

        if parent_ids[child.index()].is_none()
            && let Some(parent) = stack.iter().rev().copied().find(|candidate| {
                *candidate != child
                    && contains_command_span(model.command_syntax_span(*candidate), child_span)
                    && !would_create_command_parent_cycle(*candidate, child, parent_ids)
            })
        {
            assign_command_parent(parent, child, parent_ids, child_ids);
        }
        stack.push(child);
    }
}

fn record_command_children(
    program: &RecordedProgram,
    parent: CommandId,
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
    nested_region_command_ids: &mut FxHashSet<CommandId>,
    nested_region_root_command_ids: &mut FxHashSet<CommandId>,
) {
    let command = program.command(parent);
    for region in program.nested_regions(command.nested_regions) {
        for child in commands_in_range_recursive(program, region.commands) {
            nested_region_command_ids.insert(child);
        }
        for child in program.commands_in(region.commands).iter().copied() {
            nested_region_root_command_ids.insert(child);
            assign_command_parent(parent, child, parent_ids, child_ids);
        }
    }

    match command.kind {
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit => {}
        RecordedCommandKind::List { first, rest } => {
            assign_command_parent(parent, first, parent_ids, child_ids);
            for item in program.list_items(rest) {
                assign_command_parent(parent, item.command, parent_ids, child_ids);
            }
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            assign_range_parent(program, parent, condition, parent_ids, child_ids);
            assign_range_parent(program, parent, then_branch, parent_ids, child_ids);
            for branch in program.elif_branches(elif_branches) {
                assign_range_parent(program, parent, branch.condition, parent_ids, child_ids);
                assign_range_parent(program, parent, branch.body, parent_ids, child_ids);
            }
            assign_range_parent(program, parent, else_branch, parent_ids, child_ids);
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            assign_range_parent(program, parent, condition, parent_ids, child_ids);
            assign_range_parent(program, parent, body, parent_ids, child_ids);
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body }
        | RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::Subshell { body } => {
            assign_range_parent(program, parent, body, parent_ids, child_ids);
        }
        RecordedCommandKind::Always { body, always_body } => {
            assign_range_parent(program, parent, body, parent_ids, child_ids);
            assign_range_parent(program, parent, always_body, parent_ids, child_ids);
        }
        RecordedCommandKind::Case { arms } => {
            for arm in program.case_arms(arms) {
                assign_range_parent(program, parent, arm.commands, parent_ids, child_ids);
            }
        }
        RecordedCommandKind::Pipeline { segments } => {
            for segment in program.pipeline_segments(segments) {
                assign_command_parent(parent, segment.command, parent_ids, child_ids);
            }
        }
    }
}

fn assign_range_parent(
    program: &RecordedProgram,
    parent: CommandId,
    range: crate::cfg::RecordedCommandRange,
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    for child in program.commands_in(range).iter().copied() {
        assign_command_parent(parent, child, parent_ids, child_ids);
    }
}

fn assign_command_parent(
    parent: CommandId,
    child: CommandId,
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    if parent != child
        && parent_ids[child.index()].is_none()
        && !would_create_command_parent_cycle(parent, child, parent_ids)
    {
        parent_ids[child.index()] = Some(parent);
        child_ids[parent.index()].push(child);
    }
}

fn would_create_command_parent_cycle(
    parent: CommandId,
    child: CommandId,
    parent_ids: &[Option<CommandId>],
) -> bool {
    let mut current = Some(parent);
    while let Some(id) = current {
        if id == child {
            return true;
        }
        current = parent_ids[id.index()];
    }
    false
}

fn build_nested_region_depths(
    command_count: usize,
    parent_ids: &[Option<CommandId>],
    child_ids: &[Vec<CommandId>],
    nested_region_root_command_ids: &FxHashSet<CommandId>,
) -> Vec<usize> {
    let mut depths = vec![0; command_count];
    let mut stack = Vec::new();
    for index in (0..command_count).rev() {
        let id = CommandId(index as u32);
        if parent_ids[id.index()].is_none() {
            stack.push((id, 0));
        }
    }
    while let Some((id, depth)) = stack.pop() {
        depths[id.index()] = depth;
        for child in child_ids[id.index()].iter().rev().copied() {
            let child_depth = if nested_region_root_command_ids.contains(&child) {
                depth + 1
            } else {
                depth
            };
            stack.push((child, child_depth));
        }
    }
    depths
}

fn commands_in_range_recursive(
    program: &RecordedProgram,
    range: crate::cfg::RecordedCommandRange,
) -> Vec<CommandId> {
    let mut commands = Vec::new();
    for command in program.commands_in(range).iter().copied() {
        commands.push(command);
        commands.extend(command_descendants(program, command));
    }
    commands
}

fn command_descendants(program: &RecordedProgram, command: CommandId) -> Vec<CommandId> {
    let mut descendants = Vec::new();
    let command = program.command(command);
    for region in program.nested_regions(command.nested_regions) {
        descendants.extend(commands_in_range_recursive(program, region.commands));
    }
    match command.kind {
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit => {}
        RecordedCommandKind::List { first, rest } => {
            descendants.push(first);
            descendants.extend(command_descendants(program, first));
            for item in program.list_items(rest) {
                descendants.push(item.command);
                descendants.extend(command_descendants(program, item.command));
            }
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            descendants.extend(commands_in_range_recursive(program, condition));
            descendants.extend(commands_in_range_recursive(program, then_branch));
            for branch in program.elif_branches(elif_branches) {
                descendants.extend(commands_in_range_recursive(program, branch.condition));
                descendants.extend(commands_in_range_recursive(program, branch.body));
            }
            descendants.extend(commands_in_range_recursive(program, else_branch));
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            descendants.extend(commands_in_range_recursive(program, condition));
            descendants.extend(commands_in_range_recursive(program, body));
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body }
        | RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::Subshell { body } => {
            descendants.extend(commands_in_range_recursive(program, body));
        }
        RecordedCommandKind::Always { body, always_body } => {
            descendants.extend(commands_in_range_recursive(program, body));
            descendants.extend(commands_in_range_recursive(program, always_body));
        }
        RecordedCommandKind::Case { arms } => {
            for arm in program.case_arms(arms) {
                descendants.extend(commands_in_range_recursive(program, arm.commands));
            }
        }
        RecordedCommandKind::Pipeline { segments } => {
            for segment in program.pipeline_segments(segments) {
                descendants.push(segment.command);
                descendants.extend(command_descendants(program, segment.command));
            }
        }
    }
    descendants
}

fn attach_function_body_commands(
    model: &SemanticModel,
    command_ids: &[CommandId],
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    let mut function_ids = command_ids
        .iter()
        .copied()
        .filter(|id| model.command_syntax_kind(*id) == Some(CommandKind::Function))
        .collect::<Vec<_>>();
    if function_ids.is_empty() {
        return;
    }
    function_ids
        .sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));

    let mut body_children = Vec::new();
    for body in model.recorded_program.function_bodies().values().copied() {
        for child in model.recorded_program.commands_in(body).iter().copied() {
            body_children.push(child);
        }
    }
    body_children
        .sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));
    body_children.dedup();

    let mut active_functions = Vec::new();
    let mut next_function = 0usize;
    for child in body_children {
        if parent_ids[child.index()].is_some() {
            continue;
        }
        let child_span = model.command_syntax_span(child);
        while let Some(function_id) = function_ids.get(next_function).copied() {
            let function_span = model.command_syntax_span(function_id);
            if function_span.start.offset > child_span.start.offset {
                break;
            }
            while active_functions.last().is_some_and(|active| {
                !contains_command_span(model.command_syntax_span(*active), function_span)
            }) {
                active_functions.pop();
            }
            active_functions.push(function_id);
            next_function += 1;
        }
        while active_functions.last().is_some_and(|active| {
            !contains_command_span(model.command_syntax_span(*active), child_span)
        }) {
            active_functions.pop();
        }

        if let Some(parent) = active_functions.iter().rev().copied().find(|candidate| {
            *candidate != child
                && contains_command_span(model.command_syntax_span(*candidate), child_span)
        }) {
            assign_command_parent(parent, child, parent_ids, child_ids);
        }
    }
}

fn contains_command_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn compare_command_ids_by_syntax_span(
    model: &SemanticModel,
    left: CommandId,
    right: CommandId,
) -> std::cmp::Ordering {
    let left_span = model.command_syntax_span(left);
    let right_span = model.command_syntax_span(right);
    left_span
        .start
        .offset
        .cmp(&right_span.start.offset)
        .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
        .then_with(|| right.index().cmp(&left.index()))
}
