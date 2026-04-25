use crate::context::FileContextTag;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::static_word_text;
use shuck_semantic::{
    BindingKind, BindingOrigin, OverwrittenFunction as SemanticOverwrittenFunction, ScopeId,
    ScopeKind, UnreachedFunction as SemanticUnreachedFunction, UnreachedFunctionReason,
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
    pub name: String,
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
    let overwritten = checker.semantic_analysis().overwritten_functions().to_vec();
    let unreached = checker
        .semantic_analysis()
        .unreached_functions_with_options(checker.rule_options().c063.semantic_options())
        .to_vec();
    let compat_mode = checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions;

    for overwritten in overwritten {
        let second = checker.semantic().binding(overwritten.second);
        if compat_mode {
            if has_direct_call_to_binding_before_offset(
                checker,
                overwritten.first,
                second.span.start.offset,
            ) {
                continue;
            }
        } else if overwritten.first_called {
            continue;
        }
        if should_suppress_overwrite(checker, &overwritten) {
            continue;
        }

        report_function_definition(
            checker,
            overwritten.first,
            overwritten.name.to_string(),
            FunctionNotReachedReason::Overwritten,
        );
    }

    for unreached in unreached {
        if should_suppress_unreached(checker, &unreached) {
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
        report_function_definition(
            checker,
            unreached.binding,
            unreached.name.to_string(),
            reason,
        );
    }

    if checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions
    {
        report_compat_cutoff_function_definitions(checker);
    }
}

#[derive(Clone, Copy)]
struct FunctionCutoff {
    offset: usize,
    reason: FunctionNotReachedReason,
}

#[derive(Clone, Copy)]
struct CompatCallFact {
    scope: ScopeId,
    span: shuck_ast::Span,
}

#[derive(Clone, Copy)]
struct CompatUnsetCommandFact {
    scope: ScopeId,
    offset: usize,
}

type CompatCallFactsByName = FxHashMap<String, Vec<CompatCallFact>>;
type CompatFunctionBindingsByScope = FxHashMap<ScopeId, Vec<shuck_semantic::BindingId>>;
type CompatUnsetCommandsByTarget = FxHashMap<String, Vec<CompatUnsetCommandFact>>;
type CompatUnsetterFunctionsByTarget = FxHashMap<String, Vec<shuck_ast::Name>>;

struct CompatUnsetFacts {
    commands_by_target: CompatUnsetCommandsByTarget,
    functions_by_target: CompatUnsetterFunctionsByTarget,
}

struct CompatReachState<'a> {
    scope_run_cache: &'a mut FxHashMap<(ScopeId, usize), bool>,
    scope_between_cache: &'a mut FxHashMap<(ScopeId, usize, usize), bool>,
    call_facts_by_name: &'a CompatCallFactsByName,
    function_bindings_by_scope: &'a CompatFunctionBindingsByScope,
}

fn build_compat_call_facts_by_name(checker: &Checker<'_>) -> CompatCallFactsByName {
    let mut calls = CompatCallFactsByName::default();
    for fact in checker.facts().structural_commands() {
        if matches!(fact.command(), shuck_ast::Command::Function(_)) {
            continue;
        }
        let Some(name) = fact.effective_name() else {
            continue;
        };
        calls
            .entry(name.to_owned())
            .or_default()
            .push(CompatCallFact {
                scope: checker.semantic().scope_at(fact.body_span().start.offset),
                span: fact.body_span(),
            });
    }
    calls
}

fn build_compat_function_bindings_by_scope(checker: &Checker<'_>) -> CompatFunctionBindingsByScope {
    let mut bindings = CompatFunctionBindingsByScope::default();
    for header in checker.facts().function_headers() {
        let (Some(scope), Some(binding_id)) = (header.function_scope(), header.binding_id()) else {
            continue;
        };
        bindings.entry(scope).or_default().push(binding_id);
    }
    bindings
}

fn build_compat_unset_facts(
    checker: &Checker<'_>,
    function_bindings_by_scope: &CompatFunctionBindingsByScope,
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

    let mut commands_by_target = CompatUnsetCommandsByTarget::default();
    let mut function_targets = FxHashMap::<String, FxHashSet<shuck_ast::Name>>::default();
    for fact in checker.facts().structural_commands() {
        if !fact.effective_name_is("unset") {
            continue;
        }
        let Some(unset) = fact.options().unset() else {
            continue;
        };
        if !unset.function_mode || !unset.options_parseable() {
            continue;
        }

        let mut targets = Vec::new();
        for word in unset.operand_words() {
            let Some(text) = static_word_text(word, checker.source()) else {
                break;
            };
            let target = text.into_owned();
            if !targets.contains(&target) {
                targets.push(target);
            }
        }

        if targets.is_empty() {
            continue;
        }

        let offset = fact.body_span().start.offset;
        if command_offset_is_under_dominance_barrier(checker, offset)
            || command_offset_is_unreachable(checker, offset)
        {
            continue;
        }
        let scope = checker.semantic().scope_at(offset);
        let command_fact = CompatUnsetCommandFact { scope, offset };
        for target in targets {
            commands_by_target
                .entry(target.clone())
                .or_default()
                .push(command_fact);
            if let Some(function_scope) = enclosing_function_scope(checker, scope)
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
        commands_by_target,
        functions_by_target,
    }
}

fn report_compat_cutoff_function_definitions(checker: &mut Checker<'_>) {
    let mut scope_run_cache = FxHashMap::default();
    let mut scope_between_cache = FxHashMap::default();
    let call_facts_by_name = build_compat_call_facts_by_name(checker);
    let function_bindings_by_scope = build_compat_function_bindings_by_scope(checker);
    let unset_facts = build_compat_unset_facts(checker, &function_bindings_by_scope);
    let mut reach = CompatReachState {
        scope_run_cache: &mut scope_run_cache,
        scope_between_cache: &mut scope_between_cache,
        call_facts_by_name: &call_facts_by_name,
        function_bindings_by_scope: &function_bindings_by_scope,
    };
    let candidates = checker
        .semantic()
        .bindings()
        .iter()
        .filter(|binding| matches!(binding.kind, BindingKind::FunctionDefinition))
        .filter(|binding| !matches!(binding.kind, BindingKind::Imported))
        .filter_map(|binding| {
            let cutoff =
                first_compat_cutoff_after_binding(checker, binding.id, &mut reach, &unset_facts)?;
            (!has_direct_call_to_binding_before_offset_cached(
                checker,
                binding.id,
                cutoff.offset,
                &mut reach,
            ))
            .then(|| (binding.id, binding.name.to_string(), cutoff.reason))
        })
        .collect::<Vec<_>>();

    for (binding_id, name, reason) in candidates {
        report_function_definition(checker, binding_id, name, reason);
    }
}

fn first_compat_cutoff_after_binding(
    checker: &Checker<'_>,
    binding_id: shuck_semantic::BindingId,
    reach: &mut CompatReachState<'_>,
    unset_facts: &CompatUnsetFacts,
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
    if !binding_definition_is_under_dominance_barrier(checker, binding_id) {
        cutoffs.extend(
            compat_script_terminator_offsets(checker, binding_id, binding_offset, reach)
                .into_iter()
                .map(|offset| FunctionCutoff {
                    offset,
                    reason: FunctionNotReachedReason::ScriptTerminates,
                }),
        );
    }
    let cutoff = cutoffs.into_iter().min_by_key(|cutoff| cutoff.offset)?;
    if matches!(cutoff.reason, FunctionNotReachedReason::ScriptTerminates)
        && has_apparent_infinite_loop_between(checker, binding_offset, cutoff.offset)
    {
        return None;
    }
    if matches!(cutoff.reason, FunctionNotReachedReason::ScriptTerminates)
        && has_top_level_return_between(checker, binding_offset, cutoff.offset)
    {
        return None;
    }

    Some(cutoff)
}

fn binding_definition_is_under_dominance_barrier(
    checker: &Checker<'_>,
    binding_id: shuck_semantic::BindingId,
) -> bool {
    let binding = checker.semantic().binding(binding_id);
    let definition_span = match &binding.origin {
        BindingOrigin::FunctionDefinition { definition_span } => *definition_span,
        _ => binding.span,
    };
    command_offset_is_under_dominance_barrier(checker, definition_span.start.offset)
}

fn unset_function_cutoff_offsets(
    checker: &Checker<'_>,
    name: &str,
    after_offset: usize,
    reach: &mut CompatReachState<'_>,
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
                && command_scope_can_run_persistently_before_offset(
                    checker,
                    fact.scope,
                    fact.offset,
                    reach,
                    &mut FxHashSet::default(),
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
                    command_scope_can_run_persistently_before_offset(
                        checker,
                        site.scope,
                        site.name_span.start.offset,
                        reach,
                        &mut FxHashSet::default(),
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
    binding_id: shuck_semantic::BindingId,
    after_offset: usize,
    reach: &mut CompatReachState<'_>,
) -> Vec<usize> {
    let cfg = checker.semantic_analysis().cfg();
    let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
    let binding = checker.semantic().binding(binding_id);
    let binding_is_file_scope = scope_is_file_scope(checker, binding.scope);

    cfg.script_terminators()
        .iter()
        .filter(|block_id| !unreachable.contains(block_id))
        .flat_map(|block_id| cfg.block(*block_id).commands.iter())
        .filter_map(|span| {
            let offset = span.start.offset;
            let terminator_scope = checker.semantic().scope_at(offset);
            (offset > after_offset
                && !scope_has_transient_ancestor(checker, terminator_scope)
                && terminator_scope_can_cut_off_binding(
                    checker,
                    binding.scope,
                    binding_is_file_scope,
                    terminator_scope,
                    offset,
                    reach,
                )
                && !span_starts_function_definition_command(checker, offset)
                && !span_starts_command_named(checker, offset, "return"))
            .then_some(offset)
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
    reach: &mut CompatReachState<'_>,
) -> bool {
    if binding_is_file_scope && !checker.semantic_analysis().cfg().script_always_terminates() {
        return false;
    }

    command_scope_can_run_before_offset(
        checker,
        binding_scope,
        terminator_offset,
        reach,
        &mut FxHashSet::default(),
    ) && command_scope_can_run_before_offset(
        checker,
        terminator_scope,
        terminator_offset,
        reach,
        &mut FxHashSet::default(),
    )
}

fn scope_is_file_scope(checker: &Checker<'_>, scope: ScopeId) -> bool {
    checker.semantic().scope(scope).parent.is_none()
}

fn scope_has_transient_ancestor(checker: &Checker<'_>, scope: ScopeId) -> bool {
    checker.semantic().ancestor_scopes(scope).any(|scope_id| {
        matches!(
            checker.semantic().scope_kind(scope_id),
            ScopeKind::Subshell | ScopeKind::CommandSubstitution | ScopeKind::Pipeline
        )
    })
}

fn span_starts_function_definition_command(checker: &Checker<'_>, offset: usize) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset == offset
            && matches!(fact.command(), shuck_ast::Command::Function(_))
    })
}

fn span_starts_command_named(checker: &Checker<'_>, offset: usize, name: &str) -> bool {
    checker
        .facts()
        .structural_commands()
        .any(|fact| fact.body_span().start.offset == offset && fact.effective_name_is(name))
}

fn has_direct_call_to_binding_before_offset(
    checker: &Checker<'_>,
    binding_id: shuck_semantic::BindingId,
    before_offset: usize,
) -> bool {
    let mut scope_run_cache = FxHashMap::default();
    let mut scope_between_cache = FxHashMap::default();
    let call_facts_by_name = build_compat_call_facts_by_name(checker);
    let function_bindings_by_scope = build_compat_function_bindings_by_scope(checker);
    let mut reach = CompatReachState {
        scope_run_cache: &mut scope_run_cache,
        scope_between_cache: &mut scope_between_cache,
        call_facts_by_name: &call_facts_by_name,
        function_bindings_by_scope: &function_bindings_by_scope,
    };
    has_direct_call_to_binding_before_offset_cached(checker, binding_id, before_offset, &mut reach)
}

fn has_direct_call_to_binding_before_offset_cached(
    checker: &Checker<'_>,
    binding_id: shuck_semantic::BindingId,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
) -> bool {
    let binding = checker.semantic().binding(binding_id);
    let mut visiting = FxHashSet::default();
    reach
        .call_facts_by_name
        .get(binding.name.as_str())
        .into_iter()
        .flat_map(|facts| facts.iter())
        .any(|fact| {
            !call_has_visible_shadowing_function_definition(
                checker, binding_id, fact.scope, fact.span,
            ) && call_can_reach_binding_before_offset(
                checker,
                fact.scope,
                fact.span.start.offset,
                binding.span.start.offset,
                before_offset,
                reach,
                &mut visiting,
            )
        })
        || checker
            .semantic()
            .call_sites_for(&binding.name)
            .iter()
            .any(|site| {
                !call_has_visible_shadowing_function_definition(
                    checker,
                    binding_id,
                    site.scope,
                    site.name_span,
                ) && call_can_reach_binding_before_offset(
                    checker,
                    site.scope,
                    site.name_span.start.offset,
                    binding.span.start.offset,
                    before_offset,
                    reach,
                    &mut visiting,
                )
            })
}

fn call_can_reach_binding_before_offset(
    checker: &Checker<'_>,
    call_scope: ScopeId,
    call_offset: usize,
    binding_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    if call_offset > before_offset {
        return false;
    }

    call_scope_can_execute_after_offset_before_offset(
        checker,
        call_scope,
        call_offset,
        binding_offset,
        before_offset,
        reach,
        visiting,
    )
}

fn command_scope_can_run_between_offsets(
    checker: &Checker<'_>,
    scope: ScopeId,
    after_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    let Some(function_scope) = enclosing_function_scope(checker, scope) else {
        return true;
    };
    if !visiting.contains(&function_scope)
        && let Some(cached) =
            reach
                .scope_between_cache
                .get(&(function_scope, after_offset, before_offset))
    {
        return *cached;
    }
    if !visiting.insert(function_scope) {
        return false;
    }

    let can_run = reach
        .function_bindings_by_scope
        .get(&function_scope)
        .into_iter()
        .flat_map(|bindings| bindings.iter())
        .any(|function_binding| {
            has_call_to_function_binding_between_offsets(
                checker,
                *function_binding,
                after_offset,
                before_offset,
                reach,
                visiting,
            )
        });

    visiting.remove(&function_scope);
    reach
        .scope_between_cache
        .insert((function_scope, after_offset, before_offset), can_run);
    can_run
}

fn has_call_to_function_binding_between_offsets(
    checker: &Checker<'_>,
    function_binding: shuck_semantic::BindingId,
    after_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    let binding = checker.semantic().binding(function_binding);
    let required_after = after_offset.max(binding.span.start.offset);
    reach
        .call_facts_by_name
        .get(binding.name.as_str())
        .into_iter()
        .flat_map(|facts| facts.iter())
        .any(|fact| {
            let call_offset = fact.span.start.offset;
            call_offset <= before_offset
                && !call_has_visible_shadowing_function_definition(
                    checker,
                    function_binding,
                    fact.scope,
                    fact.span,
                )
                && call_scope_can_execute_after_offset_before_offset(
                    checker,
                    fact.scope,
                    call_offset,
                    required_after,
                    before_offset,
                    reach,
                    visiting,
                )
        })
        || checker
            .semantic()
            .call_sites_for(&binding.name)
            .iter()
            .any(|site| {
                let call_offset = site.name_span.start.offset;
                call_offset <= before_offset
                    && !call_has_visible_shadowing_function_definition(
                        checker,
                        function_binding,
                        site.scope,
                        site.name_span,
                    )
                    && call_scope_can_execute_after_offset_before_offset(
                        checker,
                        site.scope,
                        call_offset,
                        required_after,
                        before_offset,
                        reach,
                        visiting,
                    )
            })
}

fn command_scope_can_run_before_offset(
    checker: &Checker<'_>,
    scope: ScopeId,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    let Some(function_scope) = enclosing_function_scope(checker, scope) else {
        return true;
    };
    if !visiting.contains(&function_scope)
        && let Some(cached) = reach.scope_run_cache.get(&(function_scope, before_offset))
    {
        return *cached;
    }
    if !visiting.insert(function_scope) {
        return false;
    }

    let can_run = reach
        .function_bindings_by_scope
        .get(&function_scope)
        .into_iter()
        .flat_map(|bindings| bindings.iter())
        .any(|function_binding| {
            has_call_to_function_binding_before_offset(
                checker,
                *function_binding,
                before_offset,
                reach,
                visiting,
            )
        });

    visiting.remove(&function_scope);
    reach
        .scope_run_cache
        .insert((function_scope, before_offset), can_run);
    can_run
}

fn command_scope_can_run_persistently_before_offset(
    checker: &Checker<'_>,
    scope: ScopeId,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    if scope_has_transient_ancestor(checker, scope) {
        return false;
    }
    let Some(function_scope) = enclosing_function_scope(checker, scope) else {
        return true;
    };
    if !visiting.insert(function_scope) {
        return false;
    }

    let can_run = reach
        .function_bindings_by_scope
        .get(&function_scope)
        .into_iter()
        .flat_map(|bindings| bindings.iter())
        .any(|function_binding| {
            has_persistent_call_to_function_binding_before_offset(
                checker,
                *function_binding,
                before_offset,
                reach,
                visiting,
            )
        });

    visiting.remove(&function_scope);
    can_run
}

fn has_persistent_call_to_function_binding_before_offset(
    checker: &Checker<'_>,
    function_binding: shuck_semantic::BindingId,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    let binding = checker.semantic().binding(function_binding);
    reach
        .call_facts_by_name
        .get(binding.name.as_str())
        .into_iter()
        .flat_map(|facts| facts.iter())
        .any(|fact| {
            fact.span.start.offset <= before_offset
                && !call_has_visible_shadowing_function_definition(
                    checker,
                    function_binding,
                    fact.scope,
                    fact.span,
                )
                && call_scope_can_execute_persistently_after_offset_before_offset(
                    checker,
                    fact.scope,
                    fact.span.start.offset,
                    binding.span.start.offset,
                    before_offset,
                    reach,
                    visiting,
                )
        })
        || checker
            .semantic()
            .call_sites_for(&binding.name)
            .iter()
            .any(|site| {
                site.name_span.start.offset <= before_offset
                    && !call_has_visible_shadowing_function_definition(
                        checker,
                        function_binding,
                        site.scope,
                        site.name_span,
                    )
                    && call_scope_can_execute_persistently_after_offset_before_offset(
                        checker,
                        site.scope,
                        site.name_span.start.offset,
                        binding.span.start.offset,
                        before_offset,
                        reach,
                        visiting,
                    )
            })
}

fn command_scope_can_run_persistently_between_offsets(
    checker: &Checker<'_>,
    scope: ScopeId,
    after_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    if scope_has_transient_ancestor(checker, scope) {
        return false;
    }
    let Some(function_scope) = enclosing_function_scope(checker, scope) else {
        return true;
    };
    if !visiting.insert(function_scope) {
        return false;
    }

    let can_run = reach
        .function_bindings_by_scope
        .get(&function_scope)
        .into_iter()
        .flat_map(|bindings| bindings.iter())
        .any(|function_binding| {
            has_persistent_call_to_function_binding_between_offsets(
                checker,
                *function_binding,
                after_offset,
                before_offset,
                reach,
                visiting,
            )
        });

    visiting.remove(&function_scope);
    can_run
}

fn has_persistent_call_to_function_binding_between_offsets(
    checker: &Checker<'_>,
    function_binding: shuck_semantic::BindingId,
    after_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    let binding = checker.semantic().binding(function_binding);
    let required_after = after_offset.max(binding.span.start.offset);
    reach
        .call_facts_by_name
        .get(binding.name.as_str())
        .into_iter()
        .flat_map(|facts| facts.iter())
        .any(|fact| {
            let call_offset = fact.span.start.offset;
            call_offset <= before_offset
                && !call_has_visible_shadowing_function_definition(
                    checker,
                    function_binding,
                    fact.scope,
                    fact.span,
                )
                && call_scope_can_execute_persistently_after_offset_before_offset(
                    checker,
                    fact.scope,
                    call_offset,
                    required_after,
                    before_offset,
                    reach,
                    visiting,
                )
        })
        || checker
            .semantic()
            .call_sites_for(&binding.name)
            .iter()
            .any(|site| {
                let call_offset = site.name_span.start.offset;
                call_offset <= before_offset
                    && !call_has_visible_shadowing_function_definition(
                        checker,
                        function_binding,
                        site.scope,
                        site.name_span,
                    )
                    && call_scope_can_execute_persistently_after_offset_before_offset(
                        checker,
                        site.scope,
                        call_offset,
                        required_after,
                        before_offset,
                        reach,
                        visiting,
                    )
            })
}

fn call_scope_can_execute_persistently_after_offset_before_offset(
    checker: &Checker<'_>,
    call_scope: ScopeId,
    call_offset: usize,
    after_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    if call_offset > before_offset || scope_has_transient_ancestor(checker, call_scope) {
        return false;
    }

    if enclosing_function_scope(checker, call_scope).is_none() {
        return call_offset > after_offset;
    }

    command_scope_can_run_persistently_between_offsets(
        checker,
        call_scope,
        after_offset,
        before_offset,
        reach,
        visiting,
    )
}

fn has_call_to_function_binding_before_offset(
    checker: &Checker<'_>,
    function_binding: shuck_semantic::BindingId,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    let binding = checker.semantic().binding(function_binding);
    reach
        .call_facts_by_name
        .get(binding.name.as_str())
        .into_iter()
        .flat_map(|facts| facts.iter())
        .any(|fact| {
            fact.span.start.offset <= before_offset
                && !call_has_visible_shadowing_function_definition(
                    checker,
                    function_binding,
                    fact.scope,
                    fact.span,
                )
                && call_scope_can_execute_after_offset_before_offset(
                    checker,
                    fact.scope,
                    fact.span.start.offset,
                    binding.span.start.offset,
                    before_offset,
                    reach,
                    visiting,
                )
        })
        || checker
            .semantic()
            .call_sites_for(&binding.name)
            .iter()
            .any(|site| {
                site.name_span.start.offset <= before_offset
                    && !call_has_visible_shadowing_function_definition(
                        checker,
                        function_binding,
                        site.scope,
                        site.name_span,
                    )
                    && call_scope_can_execute_after_offset_before_offset(
                        checker,
                        site.scope,
                        site.name_span.start.offset,
                        binding.span.start.offset,
                        before_offset,
                        reach,
                        visiting,
                    )
            })
}

fn call_has_visible_shadowing_function_definition(
    checker: &Checker<'_>,
    binding_id: shuck_semantic::BindingId,
    call_scope: ScopeId,
    call_span: shuck_ast::Span,
) -> bool {
    let binding = checker.semantic().binding(binding_id);
    checker
        .semantic()
        .visible_binding(&binding.name, call_span)
        .is_some_and(|visible| {
            visible.id != binding_id
                && matches!(visible.kind, BindingKind::FunctionDefinition)
                && visible.span.start.offset < call_span.start.offset
                && checker
                    .semantic()
                    .ancestor_scopes(call_scope)
                    .any(|scope| scope == visible.scope)
        })
}

fn call_scope_can_execute_after_offset_before_offset(
    checker: &Checker<'_>,
    call_scope: ScopeId,
    call_offset: usize,
    after_offset: usize,
    before_offset: usize,
    reach: &mut CompatReachState<'_>,
    visiting: &mut FxHashSet<ScopeId>,
) -> bool {
    if call_offset > before_offset {
        return false;
    }

    if enclosing_function_scope(checker, call_scope).is_none() {
        return call_offset > after_offset;
    }

    command_scope_can_run_between_offsets(
        checker,
        call_scope,
        after_offset,
        before_offset,
        reach,
        visiting,
    )
}

fn enclosing_function_scope(checker: &Checker<'_>, scope: ScopeId) -> Option<ScopeId> {
    checker.semantic().ancestor_scopes(scope).find(|scope_id| {
        matches!(
            checker.semantic().scope_kind(*scope_id),
            ScopeKind::Function(_)
        )
    })
}

fn report_function_definition(
    checker: &mut Checker<'_>,
    binding_id: shuck_semantic::BindingId,
    name: String,
    reason: FunctionNotReachedReason,
) {
    let binding = checker.semantic().binding(binding_id);
    let definition_span = match &binding.origin {
        BindingOrigin::FunctionDefinition { definition_span } => *definition_span,
        _ => binding.span,
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
    overwritten: &SemanticOverwrittenFunction,
) -> bool {
    let file_context = checker.file_context();
    let compat_mode = checker
        .rule_options()
        .c063
        .report_unreached_nested_definitions;
    let first = checker.semantic().binding(overwritten.first);
    let second = checker.semantic().binding(overwritten.second);

    if matches!(first.kind, BindingKind::Imported) || matches!(second.kind, BindingKind::Imported) {
        return true;
    }

    if enclosing_function_has_reportable_c063_diagnostic(checker, first.scope) {
        return true;
    }

    if compat_mode {
        return false;
    }

    if file_context.has_tag(FileContextTag::ShellSpec)
        && checker
            .semantic()
            .call_sites_for(&overwritten.name)
            .iter()
            .any(|site| {
                site.name_span.start.offset > first.span.end.offset
                    && site.name_span.start.offset < second.span.start.offset
            })
    {
        return true;
    }

    if file_context.has_tag(FileContextTag::ShellSpec) {
        return true;
    }

    (file_context.has_tag(FileContextTag::TestHarness)
        || file_context.has_tag(FileContextTag::HelperLibrary))
        && (unset_function_between(
            checker,
            overwritten.name.as_str(),
            first.span.end.offset,
            second.span.start.offset,
        ) || (unset_function_anywhere(checker, overwritten.name.as_str())
            && has_intervening_executable_command(
                checker,
                first.span.end.offset,
                second.span.start.offset,
            ))
            || (file_context.has_tag(FileContextTag::ProjectClosure)
                && (checker
                    .semantic()
                    .call_sites_for(&overwritten.name)
                    .is_empty()
                    || has_only_indirect_call_sites_between(
                        checker,
                        overwritten,
                        first.span.end.offset,
                        second.span.start.offset,
                    ))
                && has_intervening_executable_command(
                    checker,
                    first.span.end.offset,
                    second.span.start.offset,
                )))
}

fn should_suppress_unreached(checker: &Checker<'_>, unreached: &SemanticUnreachedFunction) -> bool {
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
                        checker,
                        unreached.binding,
                        terminator_offset,
                    )
                },
            ))
        || (!checker
            .rule_options()
            .c063
            .report_unreached_nested_definitions
            && checker.file_context().has_tag(FileContextTag::ShellSpec))
        || (matches!(
            unreached.reason,
            UnreachedFunctionReason::EnclosingFunctionUnreached
        ) && enclosing_function_has_reportable_c063_diagnostic(checker, binding.scope))
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

fn has_top_level_return_between(
    checker: &Checker<'_>,
    after_offset: usize,
    before_offset: usize,
) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset > after_offset
            && fact.body_span().start.offset < before_offset
            && scope_is_file_scope(
                checker,
                checker.semantic().scope_at(fact.body_span().start.offset),
            )
            && fact.effective_name_is("return")
    })
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

fn has_apparent_infinite_loop_between(
    checker: &Checker<'_>,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset > start_offset
            && fact.body_span().start.offset < end_offset
            && scope_is_file_scope(
                checker,
                checker.semantic().scope_at(fact.body_span().start.offset),
            )
            && command_is_apparent_infinite_loop(checker, fact.command())
    })
}

fn command_is_apparent_infinite_loop(checker: &Checker<'_>, command: &shuck_ast::Command) -> bool {
    let source = checker.source();
    match command {
        shuck_ast::Command::Compound(shuck_ast::CompoundCommand::While(command)) => {
            condition_text_is(source, command.condition.span, &["true", ":"])
                && !loop_body_contains_break(checker, command.body.span)
        }
        shuck_ast::Command::Compound(shuck_ast::CompoundCommand::Until(command)) => {
            condition_text_is(source, command.condition.span, &["false"])
                && !loop_body_contains_break(checker, command.body.span)
        }
        _ => false,
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
    scope: ScopeId,
) -> bool {
    let Some(enclosing_scope) = enclosing_function_scope(checker, scope) else {
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
                && !should_suppress_overwrite(checker, candidate)
        });

    has_unreached_diagnostic || has_overwrite_diagnostic
}

fn unset_function_between(
    checker: &Checker<'_>,
    name: &str,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.effective_name_is("unset")
            && fact.body_span().start.offset > start_offset
            && fact.body_span().start.offset < end_offset
            && fact
                .options()
                .unset()
                .is_some_and(|unset| unset.targets_function_name(checker.source(), name))
    })
}

fn unset_function_anywhere(checker: &Checker<'_>, name: &str) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.effective_name_is("unset")
            && fact
                .options()
                .unset()
                .is_some_and(|unset| unset.targets_function_name(checker.source(), name))
    })
}

fn has_intervening_executable_command(
    checker: &Checker<'_>,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    checker.facts().structural_commands().any(|fact| {
        fact.body_span().start.offset > start_offset
            && fact.body_span().start.offset < end_offset
            && !matches!(fact.command(), shuck_ast::Command::Function(_))
    })
}

fn has_only_indirect_call_sites_between(
    checker: &Checker<'_>,
    overwritten: &SemanticOverwrittenFunction,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    let first = checker.semantic().binding(overwritten.first);
    let call_sites = checker.semantic().call_sites_for(&overwritten.name);
    let has_nested_call_site = call_sites.iter().any(|site| site.scope != first.scope);
    let has_same_scope_call_between = call_sites.iter().any(|site| {
        site.scope == first.scope
            && site.span.start.offset > start_offset
            && site.span.start.offset < end_offset
    });

    has_nested_call_site && !has_same_scope_call_between
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::test::{test_path_with_fix, test_snippet_at_path, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn shellspec_nested_helper_factories_are_suppressed() {
        let source = "\
Describe 'matcher'
factory() {
  shellspec_matcher__match() { :; }
  shellspec_matcher__match() { :; }
}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__matcher_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn shellspec_top_level_example_helpers_are_suppressed() {
        let source = "\
Describe 'matcher'
  Specify 'first'
    helper() { return 0; }
  End

  Specify 'second'
    helper() { return 1; }
  End
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__matcher_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_double_swaps_after_unset_are_suppressed() {
        let source = "\
curl() { printf '%s\\n' first; }
unset -f curl
curl() { printf '%s\\n' second; }
curl
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/tests/nvm_compare_checksum_test.sh"),
            source,
            &LinterSettings::for_rule(Rule::OverwrittenFunction),
        );

        assert!(diagnostics.is_empty());
    }

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
    fn cleanup_unset_elsewhere_suppresses_test_double_swaps() {
        let source = "\
cleanup() {
  unset -f nvm_compute_checksum
}
nvm_compute_checksum() {
  echo first
}
try_err nvm_compare_checksum
nvm_compute_checksum() {
  echo second
}
try_err nvm_compare_checksum
cleanup
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/project/tests/nvm_compare_checksum_test.sh"),
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
    fn opaque_helper_calls_before_redefinition_are_suppressed() {
        let source = "\
\\. ./helpers.sh
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
