use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};

use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, BuiltinCommandKind, CommandKind,
    DeclarationBuiltin, ReferenceKind, ScopeId, ScopeKind, SemanticModel,
};

/// Options for nonpersistent assignment analysis.
#[derive(Debug, Clone, Default)]
pub struct NonpersistentAssignmentAnalysisOptions {
    /// Whether Bash pipeline side effects should be treated as persistent.
    pub suppress_bash_pipefail_pipeline_side_effects: bool,
    /// Whether later `return` operands must run in the same transient shell context.
    pub require_return_use_same_execution_context: bool,
    /// Names that callers do not want analyzed for this effect.
    pub ignored_names: Vec<Name>,
}

/// Command envelope used by nonpersistent assignment analysis for command-local reset behavior.
#[derive(Debug, Clone)]
pub struct NonpersistentAssignmentCommandContext {
    /// Full command span.
    pub span: Span,
    /// Names assigned in this command's assignment prefix.
    pub prefix_reset_names: Vec<Name>,
}

/// Extra linter- or embedding-provided read that should participate in this semantic relation.
#[derive(Debug, Clone)]
pub struct NonpersistentAssignmentExtraRead {
    /// Read name.
    pub name: Name,
    /// Span to report as the later read/use.
    pub span: Span,
    /// Scope where the read occurs.
    pub scope: ScopeId,
}

/// Inputs for nonpersistent assignment analysis.
#[derive(Debug, Clone, Default)]
pub struct NonpersistentAssignmentAnalysisContext {
    /// Analysis behavior options.
    pub options: NonpersistentAssignmentAnalysisOptions,
    /// Command envelopes, used to suppress same-command resets and extend reset ranges.
    pub commands: Vec<NonpersistentAssignmentCommandContext>,
    /// Additional runtime reads not represented as normal semantic references.
    pub extra_reads: Vec<NonpersistentAssignmentExtraRead>,
}

/// Result of nonpersistent assignment analysis.
#[derive(Debug, Clone, Default)]
pub struct NonpersistentAssignmentAnalysis {
    /// Effects where a nonpersistent assignment is observed by a later use.
    pub effects: Vec<NonpersistentAssignmentEffect>,
}

/// A nonpersistent assignment paired with a later use that may still see the outer value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonpersistentAssignmentEffect {
    /// Variable name involved in the effect.
    pub name: Name,
    /// Binding made inside a nonpersistent scope.
    pub assignment_binding: BindingId,
    /// Semantic assignment span for the binding.
    pub assignment_span: Span,
    /// Span of the later use.
    pub later_use_span: Span,
    /// Source category of the later use.
    pub later_use_kind: NonpersistentLaterUseKind,
}

/// Source category for a later use in nonpersistent assignment analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonpersistentLaterUseKind {
    /// A normal semantic reference.
    Reference,
    /// A synthetic read introduced by semantic modeling.
    SyntheticRead,
    /// A binding that reads the previous value, such as append or arithmetic assignment.
    Binding,
    /// A caller-provided runtime read.
    ExtraRead,
}

#[derive(Debug, Clone, Copy)]
struct CandidateNonpersistentAssignment {
    binding_id: BindingId,
    effective_local: bool,
    enclosing_function_scope: Option<ScopeId>,
    execution_context: Option<ScopeId>,
    assignment_span: Span,
    subshell_start: usize,
    subshell_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct NonpersistentScopeSpan {
    span: Span,
}

#[derive(Debug, Clone, Copy)]
struct PersistentReset {
    offset: usize,
    command_index: Option<usize>,
    command_end_offset: usize,
}

#[derive(Debug, Clone, Default)]
struct CommandOffsetLookup {
    entries: Vec<CommandOffsetLookupEntry>,
}

#[derive(Debug, Clone, Copy)]
struct CommandOffsetLookupEntry {
    offset: usize,
    index: usize,
}

impl SemanticModel {
    /// Analyze assignments whose effects do not persist outside a subshell-like scope but are used
    /// later as though they had.
    pub fn analyze_nonpersistent_assignments(
        &self,
        context: &NonpersistentAssignmentAnalysisContext,
    ) -> NonpersistentAssignmentAnalysis {
        let scope_spans_by_id = self
            .scopes()
            .iter()
            .map(|scope| (scope.id, scope.span))
            .collect::<FxHashMap<_, _>>();
        let ignored_names = context
            .options
            .ignored_names
            .iter()
            .collect::<FxHashSet<_>>();
        let mut candidate_bindings_by_scope: FxHashMap<
            (Name, usize, usize),
            CandidateNonpersistentAssignment,
        > = FxHashMap::default();
        let mut persistent_reset_offsets_by_name: FxHashMap<Name, Vec<usize>> =
            FxHashMap::default();
        let mut command_query_offsets = Vec::new();
        let mut relevant_references = Vec::new();
        let mut relevant_synthetic_reads = Vec::new();

        for binding in self.bindings() {
            if !is_nonpersistent_assignment_candidate(binding.kind, binding.attributes) {
                continue;
            }
            if ignored_names.contains(&binding.name) {
                continue;
            }

            let Some(nonpersistent_scope) = nonpersistent_scope_span_for_assignment(
                self,
                binding.scope,
                &scope_spans_by_id,
                context.options.suppress_bash_pipefail_pipeline_side_effects,
            ) else {
                continue;
            };

            candidate_bindings_by_scope
                .entry((
                    binding.name.clone(),
                    nonpersistent_scope.span.start.offset(),
                    nonpersistent_scope.span.end.offset(),
                ))
                .or_insert(CandidateNonpersistentAssignment {
                    binding_id: binding.id,
                    effective_local: binding_effectively_targets_local(self, binding),
                    enclosing_function_scope: self.enclosing_function_scope(binding.scope),
                    execution_context: self
                        .innermost_transient_scope_within_function(binding.scope),
                    assignment_span: binding.span,
                    subshell_start: nonpersistent_scope.span.start.offset(),
                    subshell_end: nonpersistent_scope.span.end.offset(),
                });
        }

        let mut candidate_bindings_by_name: FxHashMap<Name, Vec<CandidateNonpersistentAssignment>> =
            FxHashMap::default();
        for ((name, _, _), candidate) in candidate_bindings_by_scope {
            candidate_bindings_by_name
                .entry(name)
                .or_default()
                .push(candidate);
        }
        if candidate_bindings_by_name.is_empty() {
            return NonpersistentAssignmentAnalysis::default();
        }
        for candidates in candidate_bindings_by_name.values_mut() {
            candidates.sort_by_key(|candidate| {
                (
                    candidate.subshell_end,
                    candidate.assignment_span.start.offset(),
                    candidate.assignment_span.end.offset(),
                )
            });
        }

        for binding in self.bindings() {
            if !is_persistent_subshell_reset_binding(binding.kind, binding.attributes) {
                continue;
            }
            if ignored_names.contains(&binding.name) {
                continue;
            }
            persistent_reset_offsets_by_name
                .entry(binding.name.clone())
                .or_default()
                .push(binding.span.start.offset());
            command_query_offsets.push(binding.span.start.offset());
        }

        for reference in self.references() {
            if matches!(reference.kind, ReferenceKind::DeclarationName) {
                continue;
            }
            if ignored_names.contains(&reference.name) {
                continue;
            }
            if candidate_bindings_by_name.contains_key(&reference.name) {
                command_query_offsets.push(reference.span.start.offset());
                relevant_references.push(reference);
            }
        }

        for synthetic_read in self.synthetic_reads() {
            if ignored_names.contains(synthetic_read.name()) {
                continue;
            }
            if candidate_bindings_by_name.contains_key(synthetic_read.name()) {
                command_query_offsets.push(synthetic_read.span().start.offset());
                relevant_synthetic_reads.push(synthetic_read);
            }
        }

        for read in &context.extra_reads {
            if ignored_names.contains(&read.name) {
                continue;
            }
            if candidate_bindings_by_name.contains_key(&read.name) {
                command_query_offsets.push(read.span.start.offset());
            }
        }

        let command_offsets = build_command_offset_lookup(&context.commands, command_query_offsets);
        let persistent_resets_by_name: FxHashMap<Name, PersistentResetIndex> =
            persistent_reset_offsets_by_name
                .into_iter()
                .map(|(name, offsets)| {
                    let resets = offsets.into_iter().map(|offset| {
                        let command_index =
                            precomputed_command_index_for_offset(&command_offsets, offset);
                        let command_end_offset = command_index
                            .and_then(|index| context.commands.get(index))
                            .map(|command| command.span.end.offset())
                            .unwrap_or(offset);

                        PersistentReset {
                            offset,
                            command_index,
                            command_end_offset,
                        }
                    });
                    (name, PersistentResetIndex::new(resets))
                })
                .collect();
        let no_resets = PersistentResetIndex::default();

        let mut effects = Vec::new();
        for reference in relevant_references {
            let Some(candidates) = candidate_bindings_by_name.get(&reference.name) else {
                continue;
            };

            let resets = persistent_resets_by_name
                .get(&reference.name)
                .unwrap_or(&no_resets);
            let event_command_index = precomputed_command_index_for_offset(
                &command_offsets,
                reference.span.start.offset(),
            );
            let resolved = self.resolved_binding(reference.id);
            let reference_function_scope = self.enclosing_function_scope(reference.scope);
            if let Some(candidate) = latest_unreset_candidate(
                candidates,
                resets,
                reference.span.start.offset(),
                reference.span.start.offset(),
                event_command_index,
                |candidate| {
                    resolved_binding_allows_subshell_later_use(
                        resolved,
                        candidate,
                        reference.span.start.offset(),
                        reference_function_scope,
                    ) && self.return_later_use_allows_candidate(
                        candidate,
                        reference.span.start.offset(),
                        context.options.require_return_use_same_execution_context,
                    )
                },
            ) {
                effects.push(effect_for_candidate(
                    reference.name.clone(),
                    *candidate,
                    reference.span,
                    NonpersistentLaterUseKind::Reference,
                ));
            }
        }

        for synthetic_read in relevant_synthetic_reads {
            let Some(candidates) = candidate_bindings_by_name.get(synthetic_read.name()) else {
                continue;
            };

            let resets = persistent_resets_by_name
                .get(synthetic_read.name())
                .unwrap_or(&no_resets);
            let synthetic_command_index = precomputed_command_index_for_offset(
                &command_offsets,
                synthetic_read.span().start.offset(),
            );
            let same_command_prefix_reset = synthetic_command_index
                .and_then(|index| context.commands.get(index))
                .is_some_and(|command| {
                    command
                        .prefix_reset_names
                        .iter()
                        .any(|name| name == synthetic_read.name())
                });
            let synthetic_command_end_offset = synthetic_command_index
                .and_then(|index| context.commands.get(index))
                .map(|command| command.span.end.offset())
                .unwrap_or(synthetic_read.span().start.offset());
            let synthetic_function_scope = self.enclosing_function_scope(synthetic_read.scope());
            let candidate = if same_command_prefix_reset {
                None
            } else {
                latest_unreset_candidate(
                    candidates,
                    resets,
                    synthetic_read.span().start.offset(),
                    synthetic_command_end_offset,
                    None,
                    |candidate| {
                        candidate_allows_unresolved_later_use(candidate, synthetic_function_scope)
                    },
                )
            };
            if let Some(candidate) = candidate {
                effects.push(effect_for_candidate(
                    synthetic_read.name().clone(),
                    *candidate,
                    synthetic_read.span(),
                    NonpersistentLaterUseKind::SyntheticRead,
                ));
            }
        }

        for read in &context.extra_reads {
            let Some(candidates) = candidate_bindings_by_name.get(&read.name) else {
                continue;
            };

            let resets = persistent_resets_by_name
                .get(&read.name)
                .unwrap_or(&no_resets);
            let event_command_index =
                precomputed_command_index_for_offset(&command_offsets, read.span.start.offset());
            let read_function_scope = self.enclosing_function_scope(read.scope);
            if let Some(candidate) = latest_unreset_candidate(
                candidates,
                resets,
                read.span.start.offset(),
                read.span.start.offset(),
                event_command_index,
                |candidate| candidate_allows_unresolved_later_use(candidate, read_function_scope),
            ) {
                effects.push(effect_for_candidate(
                    read.name.clone(),
                    *candidate,
                    read.span,
                    NonpersistentLaterUseKind::ExtraRead,
                ));
            }
        }

        for binding in self.bindings() {
            if !is_nonpersistent_later_use_binding(binding.kind, binding.attributes) {
                continue;
            }
            if ignored_names.contains(&binding.name) {
                continue;
            }

            let Some(candidates) = candidate_bindings_by_name.get(&binding.name) else {
                continue;
            };

            let resets = persistent_resets_by_name
                .get(&binding.name)
                .unwrap_or(&no_resets);
            let binding_function_scope = self.enclosing_function_scope(binding.scope);
            if let Some(candidate) = latest_unreset_candidate(
                candidates,
                resets,
                binding.span.start.offset(),
                binding.span.start.offset(),
                None,
                |candidate| {
                    candidate_allows_unresolved_later_use(candidate, binding_function_scope)
                },
            ) {
                effects.push(effect_for_candidate(
                    binding.name.clone(),
                    *candidate,
                    binding.span,
                    NonpersistentLaterUseKind::Binding,
                ));
            }
        }

        let mut seen = FxHashSet::default();
        effects.retain(|effect| {
            seen.insert((
                effect.assignment_binding,
                effect.later_use_span.start.offset(),
                effect.later_use_span.end.offset(),
                effect.name.clone(),
            ))
        });
        effects.sort_by_key(|effect| {
            (
                effect.later_use_span.start.offset(),
                effect.later_use_span.end.offset(),
                effect.assignment_span.start.offset(),
                effect.assignment_span.end.offset(),
            )
        });

        NonpersistentAssignmentAnalysis { effects }
    }
}

impl SemanticModel {
    fn return_later_use_allows_candidate(
        &self,
        candidate: &CandidateNonpersistentAssignment,
        later_offset: usize,
        require_same_execution_context: bool,
    ) -> bool {
        if !require_same_execution_context {
            return true;
        }
        let Some(command_id) = self.innermost_command_id_at(later_offset) else {
            return true;
        };
        if !matches!(
            self.command_kind(command_id),
            CommandKind::Builtin(BuiltinCommandKind::Return)
        ) {
            return true;
        }
        if self.shell_behavior_at(later_offset).zsh_sh_emulation() {
            return true;
        }

        let later_scope = self.scope_at(later_offset);
        let later_context = self.innermost_transient_scope_within_function(later_scope);
        later_context == candidate.execution_context
    }
}

fn effect_for_candidate(
    name: Name,
    candidate: CandidateNonpersistentAssignment,
    later_use_span: Span,
    later_use_kind: NonpersistentLaterUseKind,
) -> NonpersistentAssignmentEffect {
    NonpersistentAssignmentEffect {
        name,
        assignment_binding: candidate.binding_id,
        assignment_span: candidate.assignment_span,
        later_use_span,
        later_use_kind,
    }
}

fn is_nonpersistent_assignment_candidate(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment => !attributes.contains(BindingAttributes::LOCAL),
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::Imported => false,
        BindingKind::FunctionDefinition | BindingKind::Nameref => false,
    }
}

fn is_nonpersistent_later_use_binding(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::AppendAssignment => true,
        BindingKind::ArithmeticAssignment => true,
        BindingKind::Declaration(DeclarationBuiltin::Export) => {
            !attributes.contains(BindingAttributes::LOCAL)
        }
        BindingKind::Declaration(_) => false,
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::FunctionDefinition
        | BindingKind::Imported
        | BindingKind::Nameref => false,
    }
}

fn is_persistent_subshell_reset_binding(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment => !attributes.contains(BindingAttributes::LOCAL),
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::Imported => false,
        BindingKind::FunctionDefinition | BindingKind::Nameref => false,
    }
}

fn nonpersistent_scope_span_for_assignment(
    semantic: &SemanticModel,
    scope: ScopeId,
    scope_spans_by_id: &FxHashMap<ScopeId, Span>,
    suppress_bash_pipefail_pipeline_side_effects: bool,
) -> Option<NonpersistentScopeSpan> {
    semantic
        .ancestor_scopes(scope)
        .find(|scope_id| match semantic.scope_kind(*scope_id) {
            ScopeKind::Pipeline => !suppress_bash_pipefail_pipeline_side_effects,
            ScopeKind::Subshell | ScopeKind::CommandSubstitution => true,
            ScopeKind::Function(_) | ScopeKind::File => false,
        })
        .and_then(|scope_id| scope_spans_by_id.get(&scope_id).copied())
        .map(|span| NonpersistentScopeSpan { span })
}

fn resolved_binding_allows_subshell_later_use(
    resolved: Option<&Binding>,
    candidate: &CandidateNonpersistentAssignment,
    reference_offset: usize,
    reference_function_scope: Option<ScopeId>,
) -> bool {
    let Some(resolved) = resolved else {
        return candidate_allows_unresolved_later_use(candidate, reference_function_scope);
    };
    if resolved.id == candidate.binding_id {
        return false;
    }
    if resolved.span.start.offset() > reference_offset {
        return true;
    }
    if resolved.span.start.offset() < candidate.subshell_start {
        return true;
    }

    matches!(resolved.kind, BindingKind::Declaration(_))
        && !resolved
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

fn candidate_allows_unresolved_later_use(
    candidate: &CandidateNonpersistentAssignment,
    later_function_scope: Option<ScopeId>,
) -> bool {
    !candidate.effective_local || later_function_scope == candidate.enclosing_function_scope
}

fn binding_effectively_targets_local(semantic: &SemanticModel, binding: &Binding) -> bool {
    if binding.attributes.contains(BindingAttributes::LOCAL) {
        return true;
    }

    let binding_function_scope = semantic.enclosing_function_scope(binding.scope);
    semantic
        .previous_visible_binding(&binding.name, binding.span, Some(binding.span))
        .is_some_and(|previous| {
            previous.attributes.contains(BindingAttributes::LOCAL)
                && semantic.enclosing_function_scope(previous.scope) == binding_function_scope
        })
}

/// Finds the latest candidate whose nonpersistent scope ends before `use_offset`, has no
/// persistent reset between its scope end and `event_offset`, and satisfies `allows`.
///
/// `candidates` must be sorted by `subshell_end`. Walking them from the latest scope end
/// backwards only widens the window searched for an intervening reset, so once a reset
/// intervenes for one candidate it intervenes for every earlier one and the search stops.
fn latest_unreset_candidate<'a>(
    candidates: &'a [CandidateNonpersistentAssignment],
    resets: &PersistentResetIndex,
    use_offset: usize,
    event_offset: usize,
    event_command_index: Option<usize>,
    mut allows: impl FnMut(&CandidateNonpersistentAssignment) -> bool,
) -> Option<&'a CandidateNonpersistentAssignment> {
    let earlier = candidates.partition_point(|candidate| candidate.subshell_end < use_offset);
    for candidate in candidates[..earlier].iter().rev() {
        if resets.has_intervening_reset(candidate.subshell_end, event_offset, event_command_index) {
            return None;
        }
        if allows(candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Persistent resets for one name, indexed for window queries.
///
/// A reset intervenes between a candidate scope end `c` and an event at `e` when its
/// effective offset falls strictly inside `(c, e)`. The effective offset is the reset's own
/// offset when that lies after `c`, and otherwise the end of the command containing the reset.
/// Because a reset's command always ends at or after the reset itself, that is equivalent to
/// either the reset offset or its command end lying inside `(c, e)`, which two sorted lists
/// answer with binary searches.
#[derive(Debug, Clone, Default)]
struct PersistentResetIndex {
    by_offset: Vec<(usize, Option<usize>)>,
    by_command_end: Vec<(usize, Option<usize>)>,
}

impl PersistentResetIndex {
    fn new(resets: impl IntoIterator<Item = PersistentReset>) -> Self {
        let mut by_offset = Vec::new();
        let mut by_command_end = Vec::new();
        for reset in resets {
            by_offset.push((reset.offset, reset.command_index));
            by_command_end.push((
                reset.command_end_offset.max(reset.offset),
                reset.command_index,
            ));
        }
        by_offset.sort_unstable();
        by_command_end.sort_unstable();
        Self {
            by_offset,
            by_command_end,
        }
    }

    fn has_intervening_reset(
        &self,
        candidate_end: usize,
        event_offset: usize,
        event_command_index: Option<usize>,
    ) -> bool {
        if candidate_end >= event_offset {
            return false;
        }
        let window_has_reset = |sorted: &[(usize, Option<usize>)]| {
            let start = sorted.partition_point(|(offset, _)| *offset <= candidate_end);
            sorted[start..]
                .iter()
                .take_while(|(offset, _)| *offset < event_offset)
                .any(|(_, command_index)| {
                    event_command_index
                        .is_none_or(|event_index| *command_index != Some(event_index))
                })
        };
        window_has_reset(&self.by_offset) || window_has_reset(&self.by_command_end)
    }
}

fn build_command_offset_lookup(
    commands: &[NonpersistentAssignmentCommandContext],
    mut offsets: Vec<usize>,
) -> CommandOffsetLookup {
    if offsets.is_empty() || commands.is_empty() {
        return CommandOffsetLookup::default();
    }

    offsets.sort_unstable();
    offsets.dedup();

    let mut command_order = (0..commands.len()).collect::<Vec<_>>();
    command_order.sort_unstable_by(|left, right| {
        let left_span = commands[*left].span;
        let right_span = commands[*right].span;
        left_span
            .start
            .offset()
            .cmp(&right_span.start.offset())
            .then_with(|| right_span.end.offset().cmp(&left_span.end.offset()))
            .then_with(|| right.cmp(left))
    });

    let mut entries = Vec::with_capacity(offsets.len());
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some(index) = command_order.get(next_command).copied() {
            let span = commands[index].span;
            if span.start.offset() > offset {
                break;
            }

            pop_finished_commands(&mut active_commands, span.start.offset());
            active_commands.push(OpenCommand {
                end_offset: span.end.offset(),
                index,
            });
            next_command += 1;
        }

        pop_finished_commands(&mut active_commands, offset);
        if let Some(command) = active_commands.last() {
            entries.push(CommandOffsetLookupEntry {
                offset,
                index: command.index,
            });
        }
    }

    CommandOffsetLookup { entries }
}

fn precomputed_command_index_for_offset(
    command_offsets: &CommandOffsetLookup,
    offset: usize,
) -> Option<usize> {
    command_offsets
        .entries
        .binary_search_by_key(&offset, |entry| entry.offset)
        .ok()
        .map(|index| command_offsets.entries[index].index)
}

#[derive(Debug, Clone, Copy)]
struct OpenCommand {
    end_offset: usize,
    index: usize,
}

fn pop_finished_commands(active_commands: &mut Vec<OpenCommand>, offset: usize) {
    while active_commands
        .last()
        .is_some_and(|command| command.end_offset < offset)
    {
        active_commands.pop();
    }
}
