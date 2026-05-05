use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Name;
use shuck_semantic::{
    BindingId, BindingKind, BindingOrigin, DirectFunctionCallReachability,
    DirectFunctionCallWindow, FunctionCallCandidate, FunctionCallPersistence,
    OverwrittenFunction as SemanticOverwrittenFunction, ScopeId,
    UnreachedFunction as SemanticUnreachedFunction, UnreachedFunctionReason,
};

#[derive(Clone, Copy)]
pub enum FunctionNotReachedReason {
    Overwritten,
    ScriptTerminates,
    UnreachableDefinition,
    EnclosingFunctionUnreached,
    Removed,
}

pub struct OverwrittenFunction {
    pub name: CompactString,
    pub reason: FunctionNotReachedReason,
}

impl Violation for OverwrittenFunction {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::OverwrittenFunction
    }

    fn message(&self) -> String {
        match self.reason {
            FunctionNotReachedReason::Overwritten => format!(
                "function `{}` is overwritten before any direct call can reach it",
                self.name
            ),
            FunctionNotReachedReason::ScriptTerminates
            | FunctionNotReachedReason::UnreachableDefinition => format!(
                "function `{}` cannot be reached by a direct call before the script terminates",
                self.name
            ),
            FunctionNotReachedReason::EnclosingFunctionUnreached => format!(
                "function `{}` cannot be reached by a direct call before the enclosing function exits",
                self.name
            ),
            FunctionNotReachedReason::Removed => format!(
                "function `{}` is removed before any direct call can reach it",
                self.name
            ),
        }
    }

    fn fix_title(&self) -> Option<String> {
        match self.reason {
            FunctionNotReachedReason::Overwritten => {
                Some("delete the earlier overwritten function definition".to_owned())
            }
            FunctionNotReachedReason::ScriptTerminates
            | FunctionNotReachedReason::UnreachableDefinition => {
                Some("delete the function definition that cannot be reached".to_owned())
            }
            FunctionNotReachedReason::EnclosingFunctionUnreached => {
                Some("delete the nested function definition that cannot be reached".to_owned())
            }
            FunctionNotReachedReason::Removed => {
                Some("delete the function definition that is removed before use".to_owned())
            }
        }
    }
}

pub fn overwritten_function(checker: &mut Checker) {
    let mut reach = build_direct_function_call_reachability(checker);
    let overwritten = checker.semantic_analysis().overwritten_functions().to_vec();
    let unreached = checker
        .semantic_analysis()
        .unreached_functions_with_options(checker.rule_options().c063.semantic_options())
        .to_vec();
    let compat_mode = checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions;
    let mut reports = Vec::new();

    for overwritten in overwritten {
        let second = checker.semantic().binding(overwritten.second);
        if compat_mode {
            if has_direct_call_to_binding_before_offset(
                &mut reach,
                overwritten.first,
                second.span.start.offset,
            ) {
                continue;
            }
        } else if overwritten.first_called {
            continue;
        }
        if should_suppress_overwrite(checker, &mut reach, &overwritten) {
            continue;
        }

        reports.push((
            overwritten.first,
            overwritten.name.as_str().into(),
            FunctionNotReachedReason::Overwritten,
        ));
    }

    for unreached in unreached {
        if should_suppress_unreached(checker, &mut reach, &unreached) {
            continue;
        }

        let reason = match unreached.reason {
            UnreachedFunctionReason::UnreachableDefinition => {
                FunctionNotReachedReason::UnreachableDefinition
            }
            UnreachedFunctionReason::ScriptTerminates => FunctionNotReachedReason::ScriptTerminates,
            UnreachedFunctionReason::EnclosingFunctionUnreached => {
                FunctionNotReachedReason::EnclosingFunctionUnreached
            }
        };
        reports.push((unreached.binding, unreached.name.as_str().into(), reason));
    }

    if checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions
    {
        reports.extend(report_compat_cutoff_function_definitions(
            checker, &mut reach,
        ));
        reports.extend(report_transient_shadowed_file_scope_definitions(
            checker, &mut reach,
        ));
    }

    drop(reach);
    for (binding, name, reason) in reports {
        report_function_definition(checker, binding, name, reason);
    }
}

#[derive(Clone, Copy)]
struct FunctionCutoff {
    offset: usize,
    reason: FunctionNotReachedReason,
}

#[derive(Clone, Copy)]
struct CompatUnsetCommandFact {
    scope: ScopeId,
    offset: usize,
}

#[derive(Clone, Copy)]
struct CompatScriptTerminatorFact {
    scope: ScopeId,
    offset: usize,
    starts_function_definition: bool,
    starts_return: bool,
}

struct CompatTopLevelControlFacts {
    apparent_infinite_loop_offsets: Vec<usize>,
    return_offsets: Vec<usize>,
}

struct CompatStructuralFacts {
    unset_commands_by_target: CompatUnsetCommandsByTarget,
    scopes_by_offset: FxHashMap<usize, ScopeId>,
    function_definition_offsets: FxHashSet<usize>,
    return_offsets: FxHashSet<usize>,
    top_level_control: CompatTopLevelControlFacts,
}

#[derive(Clone, Copy)]
struct CompatLoopCandidate {
    offset: usize,
    body_span: shuck_ast::Span,
}

type CompatFunctionBindingsByScope = FxHashMap<ScopeId, Vec<BindingId>>;
type CompatUnsetCommandsByTarget = FxHashMap<String, Vec<CompatUnsetCommandFact>>;
type CompatUnsetterFunctionsByTarget = FxHashMap<String, Vec<shuck_ast::Name>>;

struct CompatUnsetFacts {
    commands_by_target: CompatUnsetCommandsByTarget,
    functions_by_target: CompatUnsetterFunctionsByTarget,
}

fn build_compat_structural_facts(checker: &Checker<'_>) -> CompatStructuralFacts {
    let mut unset_commands_by_target = CompatUnsetCommandsByTarget::default();
    let mut scopes_by_offset = FxHashMap::<usize, ScopeId>::default();
    let mut function_definition_offsets = FxHashSet::<usize>::default();
    let mut return_offsets = FxHashSet::<usize>::default();
    let mut top_level_return_offsets = Vec::new();
    let mut top_level_loop_candidates = Vec::new();
    let mut break_offsets = Vec::new();

    for fact in checker.facts().structural_commands() {
        let offset = fact.body_span().start.offset;
        scopes_by_offset.entry(offset).or_insert(fact.scope());
        let is_function = matches!(fact.command(), shuck_ast::Command::Function(_));
        if is_function {
            function_definition_offsets.insert(offset);
        }

        if matches!(
            fact.command(),
            shuck_ast::Command::Builtin(shuck_ast::BuiltinCommand::Break(_))
        ) {
            break_offsets.push(offset);
        }

        let is_return = fact.effective_name_is("return");
        if is_return {
            return_offsets.insert(offset);
        }

        let apparent_loop_body_span = apparent_infinite_loop_body_span(checker, fact.command());
        if is_return || apparent_loop_body_span.is_some() {
            let scope = fact.scope();
            if scope_is_file_scope(checker, scope) {
                if is_return {
                    top_level_return_offsets.push(offset);
                }
                if let Some(body_span) = apparent_loop_body_span {
                    top_level_loop_candidates.push(CompatLoopCandidate { offset, body_span });
                }
            }
        }
    }

    let mut seen_function_unset_targets = FxHashSet::<Name>::default();
    for binding in checker.semantic().function_definition_bindings() {
        if !seen_function_unset_targets.insert(binding.name.clone()) {
            continue;
        }

        for fact in checker
            .facts()
            .function_unset_commands_for_name(&binding.name)
        {
            let offset = fact.body_span().start.offset;
            if !command_offset_is_under_dominance_barrier(checker, offset)
                && !command_offset_is_unreachable(checker, offset)
            {
                unset_commands_by_target
                    .entry(binding.name.to_string())
                    .or_default()
                    .push(CompatUnsetCommandFact {
                        scope: fact.scope(),
                        offset,
                    });
            }
        }
    }

    let apparent_infinite_loop_offsets = top_level_loop_candidates
        .into_iter()
        .filter(|candidate| {
            !break_offsets.iter().any(|break_offset| {
                *break_offset >= candidate.body_span.start.offset
                    && *break_offset <= candidate.body_span.end.offset
            })
        })
        .map(|candidate| candidate.offset)
        .collect();

    CompatStructuralFacts {
        unset_commands_by_target,
        scopes_by_offset,
        function_definition_offsets,
        return_offsets,
        top_level_control: CompatTopLevelControlFacts {
            apparent_infinite_loop_offsets,
            return_offsets: top_level_return_offsets,
        },
    }
}

fn supplemental_function_call_candidates<'checker, 'model>(
    checker: &'checker Checker<'model>,
) -> impl Iterator<Item = FunctionCallCandidate> + 'checker {
    checker
        .facts()
        .structural_commands()
        .filter(|fact| !matches!(fact.command(), shuck_ast::Command::Function(_)))
        .filter_map(|fact| {
            let callee = fact.effective_name()?;
            Some(FunctionCallCandidate {
                callee: Name::from(callee),
                scope: fact.scope(),
                name_span: fact.body_span(),
                command_span: fact.body_span(),
            })
        })
}

fn build_direct_function_call_reachability<'checker, 'model>(
    checker: &'checker Checker<'model>,
) -> DirectFunctionCallReachability<'checker, 'model> {
    checker
        .semantic_analysis()
        .direct_function_call_reachability(supplemental_function_call_candidates(checker))
}

fn build_compat_function_bindings_by_scope(checker: &Checker<'_>) -> CompatFunctionBindingsByScope {
    checker
        .semantic_analysis()
        .function_bindings_by_scope()
        .map(|(scope, bindings)| (scope, bindings.to_vec()))
        .collect()
}

fn build_compat_script_terminator_facts(
    checker: &Checker<'_>,
    structural_facts: &CompatStructuralFacts,
) -> Vec<CompatScriptTerminatorFact> {
    let cfg = checker.semantic_analysis().cfg();
    let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
    cfg.script_terminators()
        .iter()
        .filter(|block_id| !unreachable.contains(block_id))
        .flat_map(|block_id| cfg.block(*block_id).commands.iter())
        .filter_map(|span| {
            let offset = span.start.offset;
            let scope = structural_facts
                .scopes_by_offset
                .get(&offset)
                .copied()
                .unwrap_or_else(|| checker.semantic().scope_at(offset));
            (!scope_has_transient_ancestor(checker, scope)).then_some(CompatScriptTerminatorFact {
                scope,
                offset,
                starts_function_definition: structural_facts
                    .function_definition_offsets
                    .contains(&offset),
                starts_return: structural_facts.return_offsets.contains(&offset),
            })
        })
        .collect()
}

impl CompatTopLevelControlFacts {
    fn has_apparent_infinite_loop_between(&self, start_offset: usize, end_offset: usize) -> bool {
        self.apparent_infinite_loop_offsets
            .iter()
            .any(|offset| *offset > start_offset && *offset < end_offset)
    }

    fn has_return_between(&self, start_offset: usize, end_offset: usize) -> bool {
        self.return_offsets
            .iter()
            .any(|offset| *offset > start_offset && *offset < end_offset)
    }
}

fn build_compat_unset_facts(
    checker: &Checker<'_>,
    function_bindings_by_scope: &CompatFunctionBindingsByScope,
    unset_commands_by_target: &CompatUnsetCommandsByTarget,
) -> CompatUnsetFacts {
    let mut function_names_by_scope = FxHashMap::<ScopeId, Vec<shuck_ast::Name>>::default();
    for (scope, binding_ids) in function_bindings_by_scope {
        for binding_id in binding_ids {
            let binding = checker.semantic().binding(*binding_id);
            function_names_by_scope
                .entry(*scope)
                .or_default()
                .push(binding.name.clone());
        }
    }

    let mut function_targets = FxHashMap::<String, FxHashSet<shuck_ast::Name>>::default();
    for (target, command_facts) in unset_commands_by_target {
        for command_fact in command_facts {
            if let Some(function_scope) = checker
                .semantic()
                .enclosing_function_scope(command_fact.scope)
                && let Some(function_names) = function_names_by_scope.get(&function_scope)
            {
                function_targets
                    .entry(target.clone())
                    .or_default()
                    .extend(function_names.iter().cloned());
            }
        }
    }

    let functions_by_target = function_targets
        .into_iter()
        .map(|(target, names)| (target, names.into_iter().collect()))
        .collect();

    CompatUnsetFacts {
        commands_by_target: unset_commands_by_target.clone(),
        functions_by_target,
    }
}

fn report_compat_cutoff_function_definitions(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
) -> Vec<(BindingId, CompactString, FunctionNotReachedReason)> {
    let structural_facts = build_compat_structural_facts(checker);
    let function_bindings_by_scope = build_compat_function_bindings_by_scope(checker);
    let unset_facts = build_compat_unset_facts(
        checker,
        &function_bindings_by_scope,
        &structural_facts.unset_commands_by_target,
    );
    let script_terminators = build_compat_script_terminator_facts(checker, &structural_facts);
    reach.enable_activation_index();

    checker
        .semantic()
        .function_definition_bindings()
        .filter_map(|binding| {
            let cutoff = first_compat_cutoff_after_binding(
                checker,
                binding.id,
                reach,
                &unset_facts,
                &script_terminators,
                &structural_facts.top_level_control,
            )?;
            (!binding_has_direct_call_before_offset(reach, binding.id, cutoff.offset))
                .then(|| (binding.id, binding.name.as_str().into(), cutoff.reason))
        })
        .collect::<Vec<_>>()
}

fn report_transient_shadowed_file_scope_definitions(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
) -> Vec<(BindingId, CompactString, FunctionNotReachedReason)> {
    let transient_shadow_offsets = transient_function_shadow_offsets_by_name(checker);

    checker
        .semantic()
        .function_definition_bindings()
        .filter(|binding| scope_is_file_scope(checker, binding.scope))
        .filter_map(|binding| {
            let offsets = transient_shadow_offsets.get(&binding.name)?;
            let first_shadow_offset = offsets
                .iter()
                .copied()
                .find(|offset| *offset > binding.span.start.offset)?;
            if !checker.semantic_analysis().cfg().script_always_terminates() {
                return None;
            }
            let terminator_offset =
                last_script_terminator_offset_after(checker, binding.span.start.offset)?;

            if has_direct_call_to_binding_before_offset(reach, binding.id, first_shadow_offset)
                || has_non_transient_direct_call_to_binding_between_offsets(
                    reach,
                    binding.id,
                    first_shadow_offset,
                    terminator_offset,
                )
            {
                return None;
            }

            Some((
                binding.id,
                binding.name.as_str().into(),
                FunctionNotReachedReason::ScriptTerminates,
            ))
        })
        .collect()
}

fn transient_function_shadow_offsets_by_name(checker: &Checker<'_>) -> FxHashMap<Name, Vec<usize>> {
    let mut offsets_by_name = FxHashMap::<Name, Vec<usize>>::default();
    for header in checker.facts().function_headers() {
        let Some(scope) = header.function_scope() else {
            continue;
        };
        if !scope_has_transient_ancestor(checker, scope) {
            continue;
        }
        let Some((name, span)) = header.static_name_entry() else {
            continue;
        };
        offsets_by_name
            .entry(name.clone())
            .or_default()
            .push(span.start.offset);
    }
    for offsets in offsets_by_name.values_mut() {
        offsets.sort_unstable();
    }
    offsets_by_name
}

fn first_compat_cutoff_after_binding(
    checker: &Checker<'_>,
    binding_id: BindingId,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    unset_facts: &CompatUnsetFacts,
    script_terminators: &[CompatScriptTerminatorFact],
    top_level_control: &CompatTopLevelControlFacts,
) -> Option<FunctionCutoff> {
    let binding = checker.semantic().binding(binding_id);
    let binding_offset = binding.span.start.offset;

    let mut cutoffs = Vec::new();
    cutoffs.extend(
        unset_function_cutoff_offsets(
            checker,
            binding.name.as_str(),
            binding_offset,
            reach,
            unset_facts,
        )
        .into_iter()
        .map(|offset| FunctionCutoff {
            offset,
            reason: FunctionNotReachedReason::Removed,
        }),
    );
    cutoffs.extend(
        compat_script_terminator_offsets(
            checker,
            binding_id,
            binding_offset,
            reach,
            script_terminators,
        )
        .into_iter()
        .map(|offset| FunctionCutoff {
            offset,
            reason: FunctionNotReachedReason::ScriptTerminates,
        }),
    );
    let cutoff = cutoffs.into_iter().min_by_key(|cutoff| cutoff.offset)?;
    if matches!(cutoff.reason, FunctionNotReachedReason::ScriptTerminates)
        && top_level_control.has_apparent_infinite_loop_between(binding_offset, cutoff.offset)
    {
        return None;
    }
    if matches!(cutoff.reason, FunctionNotReachedReason::ScriptTerminates)
        && top_level_control.has_return_between(binding_offset, cutoff.offset)
    {
        return None;
    }

    Some(cutoff)
}

fn unset_function_cutoff_offsets(
    checker: &Checker<'_>,
    name: &str,
    after_offset: usize,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    unset_facts: &CompatUnsetFacts,
) -> Vec<usize> {
    let mut offsets = unset_facts
        .commands_by_target
        .get(name)
        .into_iter()
        .flat_map(|facts| facts.iter())
        .filter(|fact| {
            fact.offset > after_offset
                && !command_offset_is_under_dominance_barrier(checker, fact.offset)
                && reach.scope_can_run_before_offset(
                    fact.scope,
                    fact.offset,
                    FunctionCallPersistence::PersistentOnly,
                )
        })
        .map(|fact| fact.offset)
        .collect::<Vec<_>>();

    for unsetter in unset_facts
        .functions_by_target
        .get(name)
        .into_iter()
        .flat_map(|names| names.iter())
    {
        offsets.extend(
            checker
                .semantic()
                .call_sites_for(unsetter)
                .iter()
                .filter(|site| site.name_span.start.offset > after_offset)
                .filter(|site| {
                    !command_offset_is_under_dominance_barrier(checker, site.name_span.start.offset)
                })
                .filter(|site| {
                    reach.scope_can_run_before_offset(
                        site.scope,
                        site.name_span.start.offset,
                        FunctionCallPersistence::PersistentOnly,
                    )
                })
                .map(|site| site.name_span.start.offset),
        );
    }

    offsets
}

fn command_offset_is_under_dominance_barrier(checker: &Checker<'_>, offset: usize) -> bool {
    let Some(mut command_id) = checker.facts().innermost_command_id_at(offset) else {
        return false;
    };

    loop {
        if checker.facts().command_is_dominance_barrier(command_id) {
            return true;
        }
        let Some(parent_id) = checker.facts().command_parent_id(command_id) else {
            return false;
        };
        command_id = parent_id;
    }
}

fn command_offset_is_unreachable(checker: &Checker<'_>, offset: usize) -> bool {
    let cfg = checker.semantic_analysis().cfg();
    cfg.unreachable().iter().any(|block_id| {
        cfg.block(*block_id)
            .commands
            .iter()
            .any(|span| span.start.offset == offset)
    })
}

fn compat_script_terminator_offsets(
    checker: &Checker<'_>,
    binding_id: BindingId,
    after_offset: usize,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    script_terminators: &[CompatScriptTerminatorFact],
) -> Vec<usize> {
    let binding = checker.semantic().binding(binding_id);
    let binding_is_file_scope = scope_is_file_scope(checker, binding.scope);

    script_terminators
        .iter()
        .filter_map(|terminator| {
            (terminator.offset > after_offset
                && terminator_scope_can_cut_off_binding(
                    checker,
                    binding.scope,
                    binding_is_file_scope,
                    terminator.scope,
                    terminator.offset,
                    reach,
                )
                && !terminator.starts_function_definition
                && !terminator.starts_return)
                .then_some(terminator.offset)
        })
        .max()
        .into_iter()
        .collect()
}

fn terminator_scope_can_cut_off_binding(
    checker: &Checker<'_>,
    binding_scope: ScopeId,
    binding_is_file_scope: bool,
    terminator_scope: ScopeId,
    terminator_offset: usize,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
) -> bool {
    if binding_is_file_scope && !checker.semantic_analysis().cfg().script_always_terminates() {
        return false;
    }

    reach.scope_can_run_before_offset(
        binding_scope,
        terminator_offset,
        FunctionCallPersistence::Any,
    ) && reach.scope_can_run_before_offset(
        terminator_scope,
        terminator_offset,
        FunctionCallPersistence::Any,
    )
}

fn scope_is_file_scope(checker: &Checker<'_>, scope: ScopeId) -> bool {
    checker.semantic().scope(scope).parent.is_none()
}

fn scope_has_transient_ancestor(checker: &Checker<'_>, scope: ScopeId) -> bool {
    checker
        .semantic_analysis()
        .scope_runs_in_transient_context(scope)
}

fn has_direct_call_to_binding_before_offset(
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    binding_id: BindingId,
    before_offset: usize,
) -> bool {
    binding_has_direct_call_before_offset(reach, binding_id, before_offset)
}

fn binding_has_direct_call_before_offset(
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    binding_id: BindingId,
    before_offset: usize,
) -> bool {
    reach.binding_has_reachable_direct_call(
        binding_id,
        DirectFunctionCallWindow::before_offset(before_offset),
    )
}

fn has_non_transient_direct_call_to_binding_between_offsets(
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    binding_id: BindingId,
    after_offset: usize,
    before_offset: usize,
) -> bool {
    reach.binding_has_reachable_direct_call(
        binding_id,
        DirectFunctionCallWindow::between_offsets(after_offset, before_offset)
            .require_non_transient_call(),
    )
}

fn report_function_definition(
    checker: &mut Checker<'_>,
    binding_id: BindingId,
    name: CompactString,
    reason: FunctionNotReachedReason,
) {
    let binding = checker.semantic().binding(binding_id);
    let definition_span = match &binding.origin {
        BindingOrigin::FunctionDefinition { definition_span } => *definition_span,
        BindingOrigin::Assignment { .. }
        | BindingOrigin::LoopVariable { .. }
        | BindingOrigin::ParameterDefaultAssignment { .. }
        | BindingOrigin::Imported { .. }
        | BindingOrigin::BuiltinTarget { .. }
        | BindingOrigin::ArithmeticAssignment { .. }
        | BindingOrigin::Declaration { .. }
        | BindingOrigin::Nameref { .. } => binding.span,
    };
    let diagnostic_span = trim_trailing_function_separator(definition_span, checker.source());

    checker.report_diagnostic_dedup(
        Diagnostic::new(OverwrittenFunction { name, reason }, diagnostic_span)
            .with_fix(Fix::unsafe_edit(Edit::deletion(definition_span))),
    );
}

fn trim_trailing_function_separator(span: shuck_ast::Span, source: &str) -> shuck_ast::Span {
    let mut trimmed = span.slice(source);
    loop {
        let without_whitespace = trimmed.trim_end_matches(char::is_whitespace);
        if let Some(without_semicolon) = without_whitespace.strip_suffix(';') {
            trimmed = without_semicolon;
            continue;
        }
        trimmed = without_whitespace;
        break;
    }
    shuck_ast::Span::from_positions(span.start, span.start.advanced_by(trimmed))
}

fn should_suppress_overwrite(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    overwritten: &SemanticOverwrittenFunction,
) -> bool {
    let compat_mode = checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions;
    let first = checker.semantic().binding(overwritten.first);
    let second = checker.semantic().binding(overwritten.second);

    if matches!(first.kind, BindingKind::Imported) || matches!(second.kind, BindingKind::Imported) {
        return true;
    }

    if enclosing_function_has_reportable_c063_diagnostic(checker, reach, first.scope) {
        return true;
    }

    if compat_mode {
        return false;
    }

    false
}

fn should_suppress_unreached(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    unreached: &SemanticUnreachedFunction,
) -> bool {
    let binding = checker.semantic().binding(unreached.binding);
    let compat_mode = checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions;

    matches!(binding.kind, BindingKind::Imported)
        || (matches!(unreached.reason, UnreachedFunctionReason::ScriptTerminates)
            && has_apparent_infinite_loop_after(checker, binding.span.start.offset))
        || (compat_mode
            && matches!(unreached.reason, UnreachedFunctionReason::ScriptTerminates)
            && has_top_level_return_after(checker, binding.span.start.offset))
        || (compat_mode
            && matches!(unreached.reason, UnreachedFunctionReason::ScriptTerminates)
            && last_script_terminator_offset_after(checker, binding.span.start.offset).is_some_and(
                |terminator_offset| {
                    has_direct_call_to_binding_before_offset(
                        reach,
                        unreached.binding,
                        terminator_offset,
                    )
                },
            ))
        || (matches!(
            unreached.reason,
            UnreachedFunctionReason::EnclosingFunctionUnreached
        ) && enclosing_function_has_reportable_c063_diagnostic(checker, reach, binding.scope))
        || (compat_mode
            && matches!(
                unreached.reason,
                UnreachedFunctionReason::EnclosingFunctionUnreached
            )
            && has_direct_call_inside_enclosing_function(checker, reach, unreached.binding))
        || (compat_mode
            && matches!(
                unreached.reason,
                UnreachedFunctionReason::EnclosingFunctionUnreached
            )
            && enclosing_function_scope_can_run_persistently(checker, reach, binding.scope))
}

fn last_script_terminator_offset_after(
    checker: &Checker<'_>,
    after_offset: usize,
) -> Option<usize> {
    let cfg = checker.semantic_analysis().cfg();
    let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
    cfg.script_terminators()
        .iter()
        .filter(|block_id| !unreachable.contains(block_id))
        .flat_map(|block_id| cfg.block(*block_id).commands.iter())
        .filter_map(|span| {
            let offset = span.start.offset;
            let scope = checker.semantic().scope_at(offset);
            (offset > after_offset && !scope_has_transient_ancestor(checker, scope))
                .then_some(offset)
        })
        .max()
}

fn has_top_level_return_after(checker: &Checker<'_>, after_offset: usize) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset > after_offset
            && scope_is_file_scope(
                checker,
                checker.semantic().scope_at(fact.body_span().start.offset),
            )
            && fact.effective_name_is("return")
    })
}

fn enclosing_function_scope_can_run_persistently(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    scope: ScopeId,
) -> bool {
    if checker.semantic().enclosing_function_scope(scope).is_none() {
        return false;
    }

    reach.scope_can_run_before_offset(scope, usize::MAX, FunctionCallPersistence::PersistentOnly)
}

fn has_direct_call_inside_enclosing_function(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    binding_id: BindingId,
) -> bool {
    let binding = checker.semantic().binding(binding_id);
    let Some(enclosing_scope) = checker.semantic().enclosing_function_scope(binding.scope) else {
        return false;
    };

    reach.binding_has_reachable_direct_call(
        binding_id,
        DirectFunctionCallWindow::between_offsets(binding.span.start.offset, usize::MAX)
            .within_scope(enclosing_scope),
    )
}

fn has_apparent_infinite_loop_after(checker: &Checker<'_>, offset: usize) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset > offset
            && scope_is_file_scope(
                checker,
                checker.semantic().scope_at(fact.body_span().start.offset),
            )
            && command_is_apparent_infinite_loop(checker, fact.command())
    })
}

fn command_is_apparent_infinite_loop(checker: &Checker<'_>, command: &shuck_ast::Command) -> bool {
    apparent_infinite_loop_body_span(checker, command)
        .is_some_and(|body_span| !loop_body_contains_break(checker, body_span))
}

fn apparent_infinite_loop_body_span(
    checker: &Checker<'_>,
    command: &shuck_ast::Command,
) -> Option<shuck_ast::Span> {
    let source = checker.source();
    match command {
        shuck_ast::Command::Compound(shuck_ast::CompoundCommand::While(command)) => {
            condition_text_is(source, command.condition.span, &["true", ":"])
                .then_some(command.body.span)
        }
        shuck_ast::Command::Compound(shuck_ast::CompoundCommand::Until(command)) => {
            condition_text_is(source, command.condition.span, &["false"])
                .then_some(command.body.span)
        }
        shuck_ast::Command::Simple(_)
        | shuck_ast::Command::Builtin(_)
        | shuck_ast::Command::Decl(_)
        | shuck_ast::Command::Binary(_)
        | shuck_ast::Command::Compound(_)
        | shuck_ast::Command::Function(_)
        | shuck_ast::Command::AnonymousFunction(_) => None,
    }
}

fn condition_text_is(source: &str, span: shuck_ast::Span, values: &[&str]) -> bool {
    let text = span.slice(source).trim().trim_end_matches(';').trim();
    values.contains(&text)
}

fn loop_body_contains_break(checker: &Checker<'_>, body_span: shuck_ast::Span) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset >= body_span.start.offset
            && fact.body_span().end.offset <= body_span.end.offset
            && matches!(
                fact.command(),
                shuck_ast::Command::Builtin(shuck_ast::BuiltinCommand::Break(_))
            )
    })
}

fn enclosing_function_has_reportable_c063_diagnostic(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    scope: ScopeId,
) -> bool {
    let Some(enclosing_scope) = checker.semantic().enclosing_function_scope(scope) else {
        return false;
    };
    let enclosing_bindings = checker
        .facts()
        .function_headers()
        .iter()
        .filter(|header| header.function_scope() == Some(enclosing_scope))
        .filter_map(|header| header.binding_id())
        .collect::<FxHashSet<_>>();

    let has_unreached_diagnostic = checker
        .semantic_analysis()
        .unreached_functions_with_options(checker.rule_options().c063.semantic_options())
        .iter()
        .any(|candidate| enclosing_bindings.contains(&candidate.binding));
    let has_overwrite_diagnostic = checker
        .semantic_analysis()
        .overwritten_functions()
        .iter()
        .any(|candidate| {
            !candidate.first_called
                && enclosing_bindings.contains(&candidate.first)
                && !should_suppress_overwrite(checker, reach, candidate)
        });
    let has_compat_cutoff_diagnostic = checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions
        && enclosing_bindings
            .iter()
            .any(|binding| compat_cutoff_would_report_binding(checker, reach, *binding));

    has_unreached_diagnostic || has_overwrite_diagnostic || has_compat_cutoff_diagnostic
}

fn compat_cutoff_would_report_binding(
    checker: &Checker<'_>,
    reach: &mut DirectFunctionCallReachability<'_, '_>,
    binding_id: BindingId,
) -> bool {
    let structural_facts = build_compat_structural_facts(checker);
    let function_bindings_by_scope = build_compat_function_bindings_by_scope(checker);
    let unset_facts = build_compat_unset_facts(
        checker,
        &function_bindings_by_scope,
        &structural_facts.unset_commands_by_target,
    );
    let script_terminators = build_compat_script_terminator_facts(checker, &structural_facts);
    let Some(cutoff) = first_compat_cutoff_after_binding(
        checker,
        binding_id,
        reach,
        &unset_facts,
        &script_terminators,
        &structural_facts.top_level_control,
    ) else {
        return false;
    };

    !binding_has_direct_call_before_offset(reach, binding_id, cutoff.offset)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::test::{test_path_with_fix, test_snippet_at_path, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn ordinary_overwrites_still_report() {
        let source = "\
myfunc() { return 1; }
myfunc() { return 0; }
myfunc
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
    }

    #[test]
    fn c063_option_reports_shellspec_overwrite_despite_later_body_call() {
        let source = "\
parse() { :; }
restargs() {
  parse \"$@\"
}
parse() { :; }
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__getoptions_base_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn attaches_unsafe_fix_metadata_for_reported_overwrites() {
        let source = "\
myfunc() { return 1; }
myfunc() { return 0; }
myfunc
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("delete the earlier overwritten function definition")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_overwritten_functions() {
        let source = "\
myfunc() { return 1; }
myfunc() { return 0; }
myfunc
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "myfunc() { return 0; }\nmyfunc\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn plain_unset_does_not_suppress_function_overwrites() {
        let source = "\
curl() { printf '%s\\n' first; }
unset curl
curl() { printf '%s\\n' second; }
curl
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/tests/nvm_compare_checksum_test.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
    }

    #[test]
    fn calls_before_redefinition_do_not_report() {
        let source = "\
myfunc() { return 1; }
myfunc
myfunc() { return 0; }
myfunc
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn functions_before_script_termination_report() {
        let source = "\
myfunc() { echo hi; }
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn functions_at_plain_eof_do_not_report() {
        let source = "myfunc() { echo hi; }\n";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn c063_option_trims_nested_function_list_separator_from_report_span() {
        let source = "mock() { trans() { echo trans; }; }\n";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        let nested = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.span.start.column == 10)
            .expect("expected nested function diagnostic");
        assert_eq!(nested.span.slice(source), "trans() { echo trans; }");
    }

    #[test]
    fn nested_functions_at_plain_eof_do_not_report_by_default() {
        let source = "\
outer() {
  inner() { echo hi; }
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn c063_option_reports_unreached_nested_functions_at_plain_eof() {
        let source = "\
outer() {
  inner() { echo hi; }
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("delete the nested function definition that cannot be reached")
        );
    }

    #[test]
    fn c063_option_reports_shellspec_mock_installed_nested_helpers() {
        let source = "\
Describe 'run evaluation'
  It 'restores mocked function'
    echo_foo() { echo 'foo'; }
    mock_foo() {
      echo_foo() { echo 'FOO'; }
    }
    When run mock_foo
  End
End
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__evaluation_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 5);
    }

    #[test]
    fn c063_option_suppresses_shellspec_nested_helpers_when_enclosing_function_reports() {
        let source = "\
Describe 'run evaluation'
  mock() {
    helper() { echo helper; }
  }
  mock() { :; }
End
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__evaluation_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn c063_option_suppresses_nested_child_when_enclosing_function_reports() {
        let source = "\
outer() {
  inner() {
    child() { echo child; }
  }
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn c063_option_suppresses_nested_child_when_enclosing_function_terminates() {
        let source = "\
outer() {
  inner() { echo child; }
}
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_suppresses_nested_overwrite_when_enclosing_function_reports() {
        let source = "\
outer() {
  inner() { echo first; }
  inner() { echo second; }
}
outer() { :; }
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_reports_dynamic_only_function_before_terminating_call() {
        let source = "\
v_echo() { env \"$@\"; }
V_ECHO=v_echo
cleanup() { exit \"$1\"; }
${V_ECHO} printf '%s\\n' hi || cleanup 1
cleanup 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_reports_nested_function_only_reached_in_command_substitution() {
        let source = "\
outer() {
  inner() { echo hi; }
}
value=$(outer)
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn c063_option_accepts_nested_function_called_inside_command_substitution() {
        let source = "\
outer() {
  inner() { echo hi; }
  inner
}
value=$(outer)
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn c063_option_reports_nested_function_before_eventual_script_termination() {
        let source = "\
outer() {
  inner() { echo hi; }
}
main() {
  outer
  exit 0
}
if should_run; then
  main
fi
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn c063_option_accepts_branch_local_helper_called_through_branch_local_function() {
        let source = "\
if use_iproute; then
  normalize_route() { sed 's/ /_/g'; }
  save_route() {
    value=$(normalize_route)
  }
else
  save_route() { :; }
fi
save_route
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/vpnc-script"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_reports_conditional_nested_function_before_script_termination() {
        let source = "\
runner() {
  if install_hook; then
    hook() { :; }
    run_hook_loader
  fi
}
runner
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn c063_option_reports_trap_only_nested_function_before_script_termination() {
        let source = "\
init() {
  if use_lock; then
    cleanup_lock() { rm -f \"$lock\"; }
    trap 'cleanup_lock' EXIT
  fi
}
main() {
  init
  exit 0
}
main
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn c063_option_accepts_nested_function_called_before_eventual_script_termination() {
        let source = "\
outer() {
  inner() { echo hi; }
}
main() {
  outer
  inner
  exit 0
}
if should_run; then
  main
fi
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn c063_option_accepts_nested_function_when_enclosing_function_is_called_transitively() {
        let source = "\
outer() {
  inner() { :; }
}
driver() {
  if should_run; then
    outer
  fi
}
driver
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_accepts_nested_recursive_parser_helpers_called_through_pipeline() {
        let source = "\
jsonsh() {
  parse_array() {
    parse_value
  }
  parse_object() {
    parse_value
  }
  parse_value() {
    case \"$token\" in
      '[') parse_array ;;
      '{') parse_object ;;
    esac
  }
  parse() {
    parse_value
  }
  tokenize | parse
}
jsonsh | read value
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_accepts_nested_function_called_from_enclosing_body() {
        let source = "\
outer() {
  inner() { :; }
  inner
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_reports_nested_function_only_called_through_dynamic_wrapper() {
        let source = "\
outer() {
  leaf() { :; }
  wrapper() { leaf; }
  \"$@\"
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );
        let lines = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.start.line)
            .collect::<Vec<_>>();

        assert_eq!(lines, [2, 3], "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_reports_nested_function_called_only_before_its_definition() {
        let source = "\
outer() {
  inner
  inner() { :; }
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn c063_option_reports_shadowed_file_scope_call_before_script_termination() {
        let source = "\
redefine() { echo redefine; }
if [ \"$(redefine() { :; }; redefine)\" = redefine ]; then
  echo changed
fi
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/shellspec-inspection.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_accepts_file_scope_call_after_transient_shadow() {
        let source = "\
redefine() { echo redefine; }
if [ \"$(redefine() { :; }; redefine)\" = redefine ]; then
  echo changed
fi
redefine
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/shellspec-inspection.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_accepts_transient_shadow_with_conditional_terminator() {
        let source = "\
redefine() { echo redefine; }
if should_stop; then
  exit 0
fi
if [ \"$(redefine() { :; }; redefine)\" = redefine ]; then
  echo changed
fi
redefine
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/shellspec-inspection.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_accepts_pre_shadow_wrapper_call_after_transient_shadow() {
        let source = "\
redefine() { echo redefine; }
wrapper() { redefine; }
if [ \"$(redefine() { :; }; redefine)\" = redefine ]; then
  echo changed
fi
wrapper
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/shellspec-inspection.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_reports_file_scope_function_only_called_under_inner_shadow() {
        let source = "\
redefine() { echo redefine; }
wrapper() {
  redefine() { echo inner; }
  redefine
}
wrapper
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/shellspec-inspection.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_counts_calls_inside_final_terminating_driver_function() {
        let source = "\
helper() { echo hi; }
finish() { exit 0; }
main() {
  helper
  finish
}
main
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/install.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_counts_earlier_body_calls_when_function_runs_after_definition() {
        let source = "\
caller() {
  late_helper
}
late_helper() { echo hi; }
caller
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_counts_earlier_body_calls_before_terminating_function_exit() {
        let source = "\
caller() {
  late_helper
  exit 0
}
late_helper() { echo hi; }
caller
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_counts_transitive_earlier_body_calls_when_driver_runs_after_definition() {
        let source = "\
worker() {
  late_helper
}
driver() {
  worker
  exit 0
}
late_helper() { echo hi; }
driver
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_body_calls_when_enclosing_function_runs_before_definition() {
        let source = "\
helper() { echo hi; }
main() {
  late
}
main
late() {
  helper
}
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );
        let lines = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.start.line)
            .collect::<Vec<_>>();

        assert_eq!(lines, [1, 6], "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_calls_after_unreachable_script_exit() {
        let source = "\
helper() { echo hi; }
exit 0
helper
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_ignores_conditional_unset_cutoff() {
        let source = "\
helper() { echo hi; }
false || unset -f helper
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_conditional_unsetter_call_cutoff() {
        let source = "\
cleanup() { unset -f helper; }
helper() { echo hi; }
false || cleanup
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_unsetter_call_inside_conditional_branch() {
        let source = "\
cleanup() { unset -f helper; }
helper() { echo hi; }
if failed; then
  cleanup
fi
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_unsetter_with_conditional_body_unset() {
        let source = "\
cleanup() {
  if failed; then
    unset -f helper
  fi
}
helper() { echo hi; }
cleanup
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_unsetter_with_unreachable_body_unset() {
        let source = "\
cleanup() {
  return
  unset -f helper
}
helper() { echo hi; }
cleanup
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_uncalled_nested_unsetter_helper() {
        let source = "\
cleanup() {
  nested() { unset -f helper; }
}
helper() { echo hi; }
cleanup
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_unsetter_call_inside_command_substitution() {
        let source = "\
cleanup() { unset -f helper; }
helper() { echo hi; }
: \"$(cleanup)\"
helper
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_unreachable_terminator_after_infinite_loop() {
        let source = "\
helper() { echo hi; }
while true; do
  :
done
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_reports_when_static_loop_breaks_before_terminator() {
        let source = "\
helper() { echo hi; }
while true; do
  break
done
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_reports_when_infinite_loop_is_inside_called_function() {
        let source = "\
helper() { echo hi; }
main() {
  while true; do
    :
  done
  exit 0
}
main
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 1);
    }

    #[test]
    fn c063_option_ignores_top_level_return_as_script_cutoff() {
        let source = "\
helper() { echo hi; }
return 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/lib.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_top_level_return_before_later_exit() {
        let source = "\
helper() { echo hi; }
return 0
exit 1
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_conditional_top_level_return_before_later_exit() {
        let source = "\
helper() { echo hi; }
${__SOURCED__:+false} : || return 0
exit 1
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_subshell_exit_as_script_cutoff() {
        let source = "\
helper() { (exit 123) && :; }
helper
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_non_guaranteed_file_scope_exit() {
        let source = "\
helper() { echo hi; }
if cond; then
  exit 0
fi
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn c063_option_ignores_conditionally_defined_functions_before_later_exit() {
        let source = "\
if cond; then
  helper() { echo hi; }
else
  helper() { echo bye; }
fi
helper
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_c063_report_unreached_nested_definitions(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn direct_calls_before_script_termination_do_not_report() {
        let source = "\
myfunc() { echo hi; }
myfunc
exit 0
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_function_definitions_report() {
        let source = "\
exit 0
myfunc() { echo hi; }
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn unreachable_function_definitions_report_alongside_unreachable_code() {
        let source = "\
exit 0
myfunc() { echo hi; }
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rules([Rule::OverwrittenFunction, Rule::UnreachableAfterExit]),
        );
        let rules = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.rule)
            .collect::<Vec<_>>();

        assert!(rules.contains(&Rule::OverwrittenFunction));
        assert!(rules.contains(&Rule::UnreachableAfterExit));
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C063.sh").as_path(),
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C063_fix_C063.sh", result);
        Ok(())
    }

    #[test]
    fn branch_local_redefinitions_do_not_report() {
        let source = "\
if cond; then
  helper() { return 0; }
else
  helper() { return 1; }
fi
helper
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn case_arm_redefinitions_do_not_report() {
        let source = "\
case $mode in
  a)
    helper() { return 0; }
    ;;
  b)
    helper() { return 1; }
    ;;
esac
helper
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn helper_factories_in_distinct_scopes_do_not_collide() {
        let source = "\
factory_one() {
  helper() { return 0; }
  helper
}
factory_two() {
  helper() { return 1; }
  helper
}
factory_one
factory_two
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn transitive_direct_calls_before_redefinition_do_not_report() {
        let source = "\
\\. ./helpers.sh
run_case() {
  helper
}
helper() { printf '%s\\n' first; }
run_case
helper() { printf '%s\\n' second; }
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/tests/helper_swap_test.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn shadowed_nested_calls_still_report_outer_overwrites() {
        let source = "\
run_case() {
  helper() { printf '%s\\n' local; }
  helper
}
helper() { printf '%s\\n' first; }
run_case
helper() { printf '%s\\n' second; }
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::OverwrittenFunction);
    }

    #[test]
    fn sourced_helper_overrides_in_helper_libraries_are_suppressed() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("libexec/bats-gather-tests");
        let helper = temp.path().join("libexec/test_functions.bash");
        let source = "\
#!/usr/bin/env bash
source ./test_functions.bash
bats_test_function() { printf '%s\\n' local; }
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(
            &helper,
            "bats_test_function() { printf '%s\\n' imported; }\n",
        )
        .unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_helper_overrides_in_nested_helper_scopes_are_suppressed() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("libexec/bats-exec-file");
        let helper = temp.path().join("libexec/tracing.bash");
        let source = "\
#!/usr/bin/env bash
runner() {
  source ./tracing.bash
  prepare_context
  bats_setup_tracing() { printf '%s\\n' local; }
}
runner
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(
            &helper,
            "bats_setup_tracing() { printf '%s\\n' imported; }\n",
        )
        .unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn nested_helper_library_reimports_are_suppressed() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("libexec/bats-exec-file");
        let tracing = temp.path().join("libexec/tracing.bash");
        let test_functions = temp.path().join("libexec/test_functions.bash");
        let warnings = temp.path().join("libexec/warnings.bash");
        let source = "\
#!/usr/bin/env bash
runner() {
  # shellcheck source=./tracing.bash
  source ./tracing.bash
  # shellcheck source=./test_functions.bash
  source ./test_functions.bash
}
runner
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(tracing.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(&tracing, "bats_setup_tracing() { :; }\n").unwrap();
        fs::write(
            &test_functions,
            "#!/usr/bin/env bash\nsource ./warnings.bash\n",
        )
        .unwrap();
        fs::write(&warnings, "#!/usr/bin/env bash\nsource ./tracing.bash\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction).with_analyzed_paths([
                main.clone(),
                tracing.clone(),
                test_functions.clone(),
                warnings.clone(),
            ]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn project_closure_reimports_in_regular_scripts_are_suppressed() {
        let temp = tempdir().unwrap();
        let main = temp.path().join(".bash.d/mysql.sh");
        let functions = temp.path().join(".bash.d/functions.sh");
        let os_detection = temp.path().join(".bash.d/os_detection.sh");
        let source = "\
#!/usr/bin/env bash
. ./os_detection.sh
. ./functions.sh
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(
            &functions,
            "#!/usr/bin/env bash\n. ./os_detection.sh\nfunctions_loaded() { :; }\n",
        )
        .unwrap();
        fs::write(&os_detection, "#!/usr/bin/env bash\nget_os() { :; }\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction).with_analyzed_paths([
                main.clone(),
                functions.clone(),
                os_detection.clone(),
            ]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_project_closure_overrides_in_regular_scripts_are_suppressed() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("themes/custom.theme.bash");
        let base = temp.path().join("themes/base.theme.bash");
        let source = "\
#!/usr/bin/env bash
source ./base.theme.bash
prompt_setter() { printf '%s\\n' local; }
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(&base, "prompt_setter() { printf '%s\\n' imported; }\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_analyzed_paths([main.clone(), base.clone()]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_helper_collisions_from_different_origins_are_ignored() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("libexec/bats-exec-file");
        let first_helper = temp.path().join("libexec/first.bash");
        let second_helper = temp.path().join("libexec/second.bash");
        let test_functions = temp.path().join("libexec/test_functions.bash");
        let warnings = temp.path().join("libexec/warnings.bash");
        let source = "\
#!/usr/bin/env bash
runner() {
  # shellcheck source=./first.bash
  source ./first.bash
  # shellcheck source=./test_functions.bash
  source ./test_functions.bash
}
runner
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(first_helper.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(
            &first_helper,
            "bats_setup_tracing() { printf '%s\\n' first; }\n",
        )
        .unwrap();
        fs::write(
            &test_functions,
            "#!/usr/bin/env bash\nsource ./warnings.bash\n",
        )
        .unwrap();
        fs::write(&warnings, "#!/usr/bin/env bash\nsource ./second.bash\n").unwrap();
        fs::write(
            &second_helper,
            "bats_setup_tracing() { printf '%s\\n' second; }\n",
        )
        .unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction).with_analyzed_paths([
                main.clone(),
                first_helper.clone(),
                second_helper.clone(),
                test_functions.clone(),
                warnings.clone(),
            ]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_helper_collisions_with_partial_origin_overlap_are_ignored() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("libexec/bats-exec-file");
        let first_helper = temp.path().join("libexec/first.bash");
        let second_helper = temp.path().join("libexec/second.bash");
        let shared = temp.path().join("libexec/shared.bash");
        let source = "\
#!/usr/bin/env bash
runner() {
  # shellcheck source=./first.bash
  source ./first.bash
  # shellcheck source=./second.bash
  source ./second.bash
}
runner
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(
            &first_helper,
            "#!/usr/bin/env bash\nsource ./shared.bash\nbats_setup_tracing() { printf '%s\\n' first; }\n",
        )
        .unwrap();
        fs::write(
            &second_helper,
            "#!/usr/bin/env bash\nsource ./shared.bash\nbats_setup_tracing() { printf '%s\\n' second; }\n",
        )
        .unwrap();
        fs::write(&shared, "bats_setup_tracing() { printf '%s\\n' shared; }\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction).with_analyzed_paths([
                main.clone(),
                first_helper.clone(),
                second_helper.clone(),
                shared.clone(),
            ]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_helper_overrides_in_regular_scripts_are_ignored() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/usr/bin/env bash
source ./helper.sh
helper() { printf '%s\\n' local; }
";

        fs::write(&main, source).unwrap();
        fs::write(&helper, "helper() { printf '%s\\n' imported; }\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_helper_collisions_are_ignored() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("libexec/bats-gather-tests");
        let first_helper = temp.path().join("libexec/first.bash");
        let second_helper = temp.path().join("libexec/second.bash");
        let source = "\
#!/usr/bin/env bash
source ./first.bash
source ./second.bash
";

        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, source).unwrap();
        fs::write(
            &first_helper,
            "bats_test_function() { printf '%s\\n' first; }\n",
        )
        .unwrap();
        fs::write(
            &second_helper,
            "bats_test_function() { printf '%s\\n' second; }\n",
        )
        .unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction).with_analyzed_paths([
                main.clone(),
                first_helper.clone(),
                second_helper.clone(),
            ]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
