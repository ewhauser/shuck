use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};

use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, DeclarationBuiltin, ReferenceKind, ScopeId,
    ScopeKind, SemanticModel,
};

/// Options for nonpersistent assignment analysis.
#[derive(Debug, Clone, Default)]
pub struct NonpersistentAssignmentAnalysisOptions {
    /// Whether Bash pipeline side effects should be treated as persistent.
    pub suppress_bash_pipefail_pipeline_side_effects: bool,
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
                    nonpersistent_scope.span.start.offset,
                    nonpersistent_scope.span.end.offset,
                ))
                .or_insert(CandidateNonpersistentAssignment {
                    binding_id: binding.id,
                    effective_local: binding_effectively_targets_local(self, binding),
                    enclosing_function_scope: enclosing_function_scope_for_scope(
                        self,
                        binding.scope,
                    ),
                    assignment_span: binding.span,
                    subshell_start: nonpersistent_scope.span.start.offset,
                    subshell_end: nonpersistent_scope.span.end.offset,
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
        for candidates in candidate_bindings_by_name.values_mut() {
            candidates.sort_by_key(|candidate| {
                (
                    candidate.subshell_end,
                    candidate.assignment_span.start.offset,
                    candidate.assignment_span.end.offset,
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
                .push(binding.span.start.offset);
            command_query_offsets.push(binding.span.start.offset);
        }

        for reference in self.references() {
            if matches!(reference.kind, ReferenceKind::DeclarationName) {
                continue;
            }
            if ignored_names.contains(&reference.name) {
                continue;
            }
            if candidate_bindings_by_name.contains_key(&reference.name) {
                command_query_offsets.push(reference.span.start.offset);
                relevant_references.push(reference);
            }
        }

        for synthetic_read in self.synthetic_reads() {
            if ignored_names.contains(synthetic_read.name()) {
                continue;
            }
            if candidate_bindings_by_name.contains_key(synthetic_read.name()) {
                command_query_offsets.push(synthetic_read.span().start.offset);
                relevant_synthetic_reads.push(synthetic_read);
            }
        }

        for read in &context.extra_reads {
            if ignored_names.contains(&read.name) {
                continue;
            }
            if candidate_bindings_by_name.contains_key(&read.name) {
                command_query_offsets.push(read.span.start.offset);
            }
        }

        let command_offsets = build_command_offset_lookup(&context.commands, command_query_offsets);
        let persistent_reset_offsets_by_name: FxHashMap<Name, Vec<PersistentReset>> =
            persistent_reset_offsets_by_name
                .into_iter()
                .map(|(name, offsets)| {
                    let resets = offsets
                        .into_iter()
                        .map(|offset| {
                            let command_index =
                                precomputed_command_index_for_offset(&command_offsets, offset);
                            let command_end_offset = command_index
                                .and_then(|index| context.commands.get(index))
                                .map(|command| command.span.end.offset)
                                .unwrap_or(offset);

                            PersistentReset {
                                offset,
                                command_index,
                                command_end_offset,
                            }
                        })
                        .collect();
                    (name, resets)
                })
                .collect();

        let mut effects = Vec::new();
        for reference in relevant_references {
            let Some(candidates) = candidate_bindings_by_name.get(&reference.name) else {
                continue;
            };

            let reset_offsets = persistent_reset_offsets_by_name
                .get(&reference.name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let event_command_index =
                precomputed_command_index_for_offset(&command_offsets, reference.span.start.offset);
            let resolved = self.resolved_binding(reference.id);
            let reference_function_scope =
                enclosing_function_scope_for_scope(self, reference.scope);
            if let Some(candidate) = candidates.iter().rev().find(|candidate| {
                reference.span.start.offset > candidate.subshell_end
                    && !has_intervening_persistent_reset(
                        reset_offsets,
                        candidate.subshell_end,
                        reference.span.start.offset,
                        event_command_index,
                    )
                    && resolved_binding_allows_subshell_later_use(
                        resolved,
                        candidate,
                        reference.span.start.offset,
                        reference_function_scope,
                    )
            }) {
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

            let reset_offsets = persistent_reset_offsets_by_name
                .get(synthetic_read.name())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let synthetic_command_index = precomputed_command_index_for_offset(
                &command_offsets,
                synthetic_read.span().start.offset,
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
                .map(|command| command.span.end.offset)
                .unwrap_or(synthetic_read.span().start.offset);
            let synthetic_function_scope =
                enclosing_function_scope_for_scope(self, synthetic_read.scope());
            if let Some(candidate) = candidates.iter().rev().find(|candidate| {
                synthetic_read.span().start.offset > candidate.subshell_end
                    && !same_command_prefix_reset
                    && candidate_allows_unresolved_later_use(candidate, synthetic_function_scope)
                    && !has_intervening_persistent_reset(
                        reset_offsets,
                        candidate.subshell_end,
                        synthetic_command_end_offset,
                        None,
                    )
            }) {
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

            let reset_offsets = persistent_reset_offsets_by_name
                .get(&read.name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let event_command_index =
                precomputed_command_index_for_offset(&command_offsets, read.span.start.offset);
            let read_function_scope = enclosing_function_scope_for_scope(self, read.scope);
            if let Some(candidate) = candidates.iter().rev().find(|candidate| {
                read.span.start.offset > candidate.subshell_end
                    && candidate_allows_unresolved_later_use(candidate, read_function_scope)
                    && !has_intervening_persistent_reset(
                        reset_offsets,
                        candidate.subshell_end,
                        read.span.start.offset,
                        event_command_index,
                    )
            }) {
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

            let reset_offsets = persistent_reset_offsets_by_name
                .get(&binding.name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let binding_function_scope = enclosing_function_scope_for_scope(self, binding.scope);
            if let Some(candidate) = candidates.iter().rev().find(|candidate| {
                binding.span.start.offset > candidate.subshell_end
                    && candidate_allows_unresolved_later_use(candidate, binding_function_scope)
                    && !has_intervening_persistent_reset(
                        reset_offsets,
                        candidate.subshell_end,
                        binding.span.start.offset,
                        None,
                    )
            }) {
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
                effect.later_use_span.start.offset,
                effect.later_use_span.end.offset,
                effect.name.clone(),
            ))
        });
        effects.sort_by_key(|effect| {
            (
                effect.later_use_span.start.offset,
                effect.later_use_span.end.offset,
                effect.assignment_span.start.offset,
                effect.assignment_span.end.offset,
            )
        });

        NonpersistentAssignmentAnalysis { effects }
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
    if resolved.span.start.offset > reference_offset {
        return true;
    }
    if resolved.span.start.offset < candidate.subshell_start {
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

    let binding_function_scope = enclosing_function_scope_for_scope(semantic, binding.scope);
    semantic
        .previous_visible_binding(&binding.name, binding.span, Some(binding.span))
        .is_some_and(|previous| {
            previous.attributes.contains(BindingAttributes::LOCAL)
                && enclosing_function_scope_for_scope(semantic, previous.scope)
                    == binding_function_scope
        })
}

fn enclosing_function_scope_for_scope(semantic: &SemanticModel, scope: ScopeId) -> Option<ScopeId> {
    semantic
        .ancestor_scopes(scope)
        .find(|scope| matches!(semantic.scope_kind(*scope), ScopeKind::Function(_)))
}

fn has_intervening_persistent_reset(
    resets: &[PersistentReset],
    candidate_end: usize,
    event_offset: usize,
    event_command_index: Option<usize>,
) -> bool {
    resets.iter().any(|reset| {
        let effective_offset = if reset.offset > candidate_end {
            reset.offset
        } else {
            reset.command_end_offset
        };

        effective_offset > candidate_end
            && effective_offset < event_offset
            && event_command_index
                .is_none_or(|event_index| reset.command_index != Some(event_index))
    })
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
            .offset
            .cmp(&right_span.start.offset)
            .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
            .then_with(|| right.cmp(left))
    });

    let mut entries = Vec::with_capacity(offsets.len());
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some(index) = command_order.get(next_command).copied() {
            let span = commands[index].span;
            if span.start.offset > offset {
                break;
            }

            pop_finished_commands(&mut active_commands, span.start.offset);
            active_commands.push(OpenCommand {
                end_offset: span.end.offset,
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
