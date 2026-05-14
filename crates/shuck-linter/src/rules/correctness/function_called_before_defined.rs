use compact_str::CompactString;
use rustc_hash::FxHashSet;
use shuck_ast::Span;
use shuck_semantic::{Binding, BindingId, CallSite, ScopeId, SourceRef, SourceRefKind};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionCalledBeforeDefined {
    pub name: CompactString,
}

impl Violation for FunctionCalledBeforeDefined {
    fn rule() -> Rule {
        Rule::FunctionCalledBeforeDefined
    }

    fn message(&self) -> String {
        format!(
            "function `{}` is called before its definition has run",
            self.name
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CallKey {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
struct RuntimeTarget<'a> {
    span: Span,
    runtime_scope: Option<ScopeId>,
    blocks: &'a [shuck_semantic::BlockId],
}

pub fn function_called_before_defined(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let semantic = checker.semantic();
    let mut reported = FxHashSet::<CallKey>::default();
    let mut violations = Vec::<(Span, CompactString)>::new();

    for function in semantic.function_definition_bindings() {
        for call in semantic.call_sites_for(&function.name) {
            if !call_is_reportable_for_function(checker, call, function) {
                continue;
            }

            let key = CallKey {
                start: call.name_span.start.offset,
                end: call.name_span.end.offset,
            };
            if reported.insert(key) {
                violations.push((call.name_span, call.callee.as_str().into()));
            }
        }
    }

    violations.sort_unstable_by_key(|(span, _)| span.start.offset);
    for (span, name) in violations {
        checker.report(FunctionCalledBeforeDefined { name }, span);
    }
}

fn call_is_reportable_for_function(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> bool {
    if call.name_span.start.offset >= function.span.start.offset {
        return false;
    }
    let call_runtime_scope = checker.semantic().enclosing_function_scope(call.scope);
    let definition_runtime_scope = checker.semantic().enclosing_function_scope(function.scope);
    if definition_runtime_scope.is_some() && call_runtime_scope != definition_runtime_scope {
        return false;
    }
    if function_definition_is_branch_local(checker, function)
        && call_is_in_condition_context(checker, call)
    {
        return false;
    }
    if !call_can_run_before_definition(checker, call, function) {
        return false;
    }
    if checker.rule_options().c161.ignore_after_source
        && source_ref_can_explain_later_definition(checker, call, function)
    {
        return false;
    }
    if call_is_unreachable(checker, call) {
        return false;
    }
    if call_path_terminates_before_function(checker, call, function) {
        return false;
    }
    let prior_visible_definition = checker
        .semantic_analysis()
        .visible_function_binding_defined_before(
            &call.callee,
            call.scope,
            call.name_span.start.offset,
        );
    if prior_visible_definition
        .is_some_and(|prior| prior_visible_definition_guaranteed_before_call(checker, prior, call))
        && !short_circuit_call_can_target_later_definition(checker, call)
    {
        return false;
    }

    true
}

fn prior_visible_definition_guaranteed_before_call(
    checker: &Checker<'_>,
    prior_definition: BindingId,
    call: &CallSite,
) -> bool {
    let semantic = checker.semantic();
    let analysis = checker.semantic_analysis();
    let definition = semantic.binding(prior_definition);
    let definition_blocks = analysis
        .blocks_containing_binding(prior_definition)
        .iter()
        .copied()
        .filter(|block| !analysis.block_is_unreachable(*block))
        .collect::<FxHashSet<_>>();
    if definition_blocks.is_empty() {
        return false;
    }

    let call_blocks = analysis
        .block_ids_for_span(call.span)
        .iter()
        .copied()
        .filter(|block| !analysis.block_is_unreachable(*block))
        .collect::<Vec<_>>();
    if call_blocks.is_empty() {
        return false;
    }

    let entry = analysis
        .flow_entry_block_for_binding_scopes(&[definition.scope], call.name_span.start.offset);
    call_blocks
        .iter()
        .copied()
        .all(|target| analysis.blocks_cover_all_paths_to_block(entry, target, &definition_blocks))
}

fn call_can_run_before_definition(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> bool {
    let call_runtime_scope = checker.semantic().enclosing_function_scope(call.scope);
    let definition_runtime_scope = checker.semantic().enclosing_function_scope(function.scope);
    if call_runtime_scope == definition_runtime_scope {
        return true;
    }
    let Some(call_function_scope) = call_runtime_scope else {
        return true;
    };

    let mut active_scopes = FxHashSet::default();
    function_scope_can_run_before_definition(
        checker,
        call_function_scope,
        definition_runtime_scope,
        function,
        &mut active_scopes,
    )
}

fn function_scope_can_run_before_definition(
    checker: &Checker<'_>,
    scope: ScopeId,
    definition_runtime_scope: Option<ScopeId>,
    function: &Binding,
    active_scopes: &mut FxHashSet<ScopeId>,
) -> bool {
    if !active_scopes.insert(scope) {
        return false;
    }

    let can_run = function_scope_callers_can_run_before_definition(
        checker,
        scope,
        definition_runtime_scope,
        function,
        active_scopes,
    );
    active_scopes.remove(&scope);
    can_run
}

fn function_scope_callers_can_run_before_definition(
    checker: &Checker<'_>,
    scope: ScopeId,
    definition_runtime_scope: Option<ScopeId>,
    function: &Binding,
    active_scopes: &mut FxHashSet<ScopeId>,
) -> bool {
    function_bindings_for_scope(checker, scope)
        .into_iter()
        .any(|function_binding| {
            let caller = checker.semantic().binding(function_binding);
            checker
                .semantic()
                .call_sites_for(&caller.name)
                .iter()
                .any(|site| {
                    if site.span.start.offset >= function.span.start.offset
                        || !call_site_resolves_to_binding(checker, site, function_binding)
                        || !span_can_run(checker, site.span)
                    {
                        return false;
                    }

                    let site_runtime_scope =
                        checker.semantic().enclosing_function_scope(site.scope);
                    if site_runtime_scope == definition_runtime_scope {
                        return span_can_run_before_definition(checker, site.span, function);
                    }

                    let Some(site_function_scope) = site_runtime_scope else {
                        return false;
                    };
                    function_scope_can_run_before_definition(
                        checker,
                        site_function_scope,
                        definition_runtime_scope,
                        function,
                        active_scopes,
                    )
                })
        })
}

fn short_circuit_call_can_target_later_definition(checker: &Checker<'_>, call: &CallSite) -> bool {
    checker.facts().command_facts().lists().iter().any(|list| {
        list.segments()
            .iter()
            .position(|segment| span_contains(segment.span(), call.name_span))
            .is_some_and(|index| index > 0)
    })
}

fn source_ref_can_explain_later_definition(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> bool {
    has_prior_source_ref(checker, call)
        || has_source_ref_before_later_definition(checker, call, function)
        || has_branch_local_loader_source_after_definition(checker, call, function)
}

fn has_prior_source_ref(checker: &Checker<'_>, call: &CallSite) -> bool {
    let call_runtime_scope = checker.semantic().enclosing_function_scope(call.scope);
    let call_blocks = checker.semantic_analysis().block_ids_for_span(call.span);

    checker.semantic().source_refs().iter().any(|source_ref| {
        if matches!(source_ref.kind, SourceRefKind::DirectiveDevNull)
            || !source_ref_has_runtime_source_command(checker, source_ref)
        {
            return false;
        }

        let source_runtime_scope = checker
            .semantic()
            .enclosing_function_scope(checker.semantic().scope_at(source_ref.span.start.offset));
        if source_runtime_scope == call_runtime_scope {
            return source_ref.span.start.offset < call.name_span.start.offset
                && span_can_run_before_call(checker, source_ref.span, call_blocks);
        }

        if source_runtime_scope.is_none() {
            let mut active_scopes = FxHashSet::default();
            return top_level_source_ref_can_run_before_scope(
                checker,
                source_ref.span,
                call_runtime_scope,
                call_blocks,
                &mut active_scopes,
            );
        }

        let Some(source_function_scope) = source_runtime_scope else {
            return false;
        };

        called_function_source_ref_can_run_before_call(
            checker,
            source_function_scope,
            source_ref.span,
            call_runtime_scope,
            call.name_span,
            call_blocks,
        )
    })
}

fn has_source_ref_before_later_definition(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> bool {
    let call_runtime_scope = checker.semantic().enclosing_function_scope(call.scope);
    if call_runtime_scope.is_none() {
        return false;
    }

    checker.semantic().source_refs().iter().any(|source_ref| {
        if matches!(source_ref.kind, SourceRefKind::DirectiveDevNull)
            || !source_ref_has_runtime_source_command(checker, source_ref)
            || source_ref.span.start.offset <= call.name_span.start.offset
            || source_ref.span.start.offset >= function.span.start.offset
        {
            return false;
        }

        let source_runtime_scope = checker
            .semantic()
            .enclosing_function_scope(checker.semantic().scope_at(source_ref.span.start.offset));
        source_runtime_scope == call_runtime_scope
            && span_can_run_before_definition(checker, source_ref.span, function)
    })
}

fn has_branch_local_loader_source_after_definition(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> bool {
    if !function_definition_is_branch_local(checker, function) {
        return false;
    }

    let call_runtime_scope = checker.semantic().enclosing_function_scope(call.scope);
    let definition_runtime_scope = checker.semantic().enclosing_function_scope(function.scope);
    if call_runtime_scope != definition_runtime_scope {
        return false;
    }

    checker.semantic().source_refs().iter().any(|source_ref| {
        if matches!(source_ref.kind, SourceRefKind::DirectiveDevNull)
            || !source_ref_has_runtime_source_command(checker, source_ref)
            || source_ref.span.start.offset <= function.span.start.offset
        {
            return false;
        }

        let source_runtime_scope = checker
            .semantic()
            .enclosing_function_scope(checker.semantic().scope_at(source_ref.span.start.offset));
        source_runtime_scope == definition_runtime_scope
            && definition_can_run_before_source_ref(checker, function, source_ref.span)
    })
}

fn definition_can_run_before_source_ref(
    checker: &Checker<'_>,
    function: &Binding,
    source_span: Span,
) -> bool {
    let definition_blocks = checker
        .semantic_analysis()
        .blocks_containing_binding(function.id);
    let source_blocks = checker.semantic_analysis().block_ids_for_span(source_span);
    if definition_blocks.is_empty() || source_blocks.is_empty() {
        return function.span.start.offset < source_span.start.offset;
    }

    let reachable_definition_blocks = definition_blocks
        .iter()
        .copied()
        .filter(|block| !checker.semantic_analysis().block_is_unreachable(*block))
        .collect::<Vec<_>>();
    if reachable_definition_blocks.is_empty() {
        return false;
    }

    checker.semantic_analysis().blocks_have_path_avoiding(
        &reachable_definition_blocks,
        source_blocks,
        &FxHashSet::default(),
    )
}

fn source_ref_has_runtime_source_command(checker: &Checker<'_>, source_ref: &SourceRef) -> bool {
    checker.facts().commands().iter().any(|fact| {
        (fact.effective_name_is("source") || fact.effective_name_is("."))
            && fact.span().start.offset == source_ref.span.start.offset
    })
}

fn top_level_source_ref_can_run_before_scope(
    checker: &Checker<'_>,
    source_span: Span,
    target_runtime_scope: Option<ScopeId>,
    target_blocks: &[shuck_semantic::BlockId],
    active_scopes: &mut FxHashSet<ScopeId>,
) -> bool {
    let Some(target_function_scope) = target_runtime_scope else {
        return span_can_run_before_blocks(checker, source_span, target_blocks);
    };

    if !active_scopes.insert(target_function_scope) {
        return false;
    }

    let can_run = function_bindings_for_scope(checker, target_function_scope)
        .into_iter()
        .any(|function_binding| {
            let function = checker.semantic().binding(function_binding);
            checker
                .semantic()
                .call_sites_for(&function.name)
                .iter()
                .any(|site| {
                    if !call_site_resolves_to_binding(checker, site, function_binding) {
                        return false;
                    }

                    let site_runtime_scope =
                        checker.semantic().enclosing_function_scope(site.scope);
                    let site_blocks = checker.semantic_analysis().block_ids_for_span(site.span);
                    top_level_source_ref_can_run_before_scope(
                        checker,
                        source_span,
                        site_runtime_scope,
                        site_blocks,
                        active_scopes,
                    )
                })
        });
    active_scopes.remove(&target_function_scope);
    can_run
}

fn called_function_source_ref_can_run_before_call(
    checker: &Checker<'_>,
    source_function_scope: ScopeId,
    source_span: Span,
    call_runtime_scope: Option<ScopeId>,
    call_span: Span,
    call_blocks: &[shuck_semantic::BlockId],
) -> bool {
    if !span_can_run(checker, source_span) {
        return false;
    }

    let mut active_source_scopes = FxHashSet::default();
    let mut active_target_scopes = FxHashSet::default();
    let target = RuntimeTarget {
        span: call_span,
        runtime_scope: call_runtime_scope,
        blocks: call_blocks,
    };
    function_bindings_for_scope(checker, source_function_scope)
        .into_iter()
        .any(|function_binding| {
            let function = checker.semantic().binding(function_binding);
            checker
                .semantic()
                .call_sites_for(&function.name)
                .iter()
                .any(|site| {
                    if !call_site_resolves_to_binding(checker, site, function_binding)
                        || !span_can_run(checker, site.span)
                    {
                        return false;
                    }

                    let site_runtime_scope =
                        checker.semantic().enclosing_function_scope(site.scope);
                    span_in_scope_can_run_before_scope(
                        checker,
                        site.span,
                        site_runtime_scope,
                        target,
                        &mut active_source_scopes,
                        &mut active_target_scopes,
                    )
                })
        })
}

fn span_in_scope_can_run_before_scope(
    checker: &Checker<'_>,
    source_span: Span,
    source_runtime_scope: Option<ScopeId>,
    target: RuntimeTarget<'_>,
    active_source_scopes: &mut FxHashSet<ScopeId>,
    active_target_scopes: &mut FxHashSet<ScopeId>,
) -> bool {
    if source_runtime_scope == target.runtime_scope {
        return source_span.start.offset < target.span.start.offset
            && span_can_run_before_blocks(checker, source_span, target.blocks);
    }

    let Some(target_function_scope) = target.runtime_scope else {
        return false;
    };
    if !active_target_scopes.insert(target_function_scope) {
        return false;
    }

    let can_run = function_bindings_for_scope(checker, target_function_scope)
        .into_iter()
        .any(|function_binding| {
            let function = checker.semantic().binding(function_binding);
            checker
                .semantic()
                .call_sites_for(&function.name)
                .iter()
                .any(|site| {
                    if !call_site_resolves_to_binding(checker, site, function_binding)
                        || !span_can_run(checker, site.span)
                    {
                        return false;
                    }

                    let site_runtime_scope =
                        checker.semantic().enclosing_function_scope(site.scope);
                    let site_blocks = checker.semantic_analysis().block_ids_for_span(site.span);
                    let target_site = RuntimeTarget {
                        span: site.span,
                        runtime_scope: site_runtime_scope,
                        blocks: site_blocks,
                    };
                    span_in_scope_can_run_before_span(
                        checker,
                        source_span,
                        source_runtime_scope,
                        target_site,
                        active_source_scopes,
                    )
                })
        });
    active_target_scopes.remove(&target_function_scope);
    can_run
}

fn span_in_scope_can_run_before_span(
    checker: &Checker<'_>,
    source_span: Span,
    source_runtime_scope: Option<ScopeId>,
    target: RuntimeTarget<'_>,
    active_source_scopes: &mut FxHashSet<ScopeId>,
) -> bool {
    if source_runtime_scope == target.runtime_scope {
        return source_span.start.offset < target.span.start.offset
            && span_can_run_before_blocks(checker, source_span, target.blocks);
    }

    let Some(source_function_scope) = source_runtime_scope else {
        return false;
    };
    if !active_source_scopes.insert(source_function_scope) {
        return false;
    }

    let can_run = function_bindings_for_scope(checker, source_function_scope)
        .into_iter()
        .any(|function_binding| {
            let function = checker.semantic().binding(function_binding);
            checker
                .semantic()
                .call_sites_for(&function.name)
                .iter()
                .any(|site| {
                    if !call_site_resolves_to_binding(checker, site, function_binding)
                        || !span_can_run(checker, site.span)
                    {
                        return false;
                    }

                    let site_runtime_scope =
                        checker.semantic().enclosing_function_scope(site.scope);
                    span_in_scope_can_run_before_span(
                        checker,
                        site.span,
                        site_runtime_scope,
                        target,
                        active_source_scopes,
                    )
                })
        });
    active_source_scopes.remove(&source_function_scope);
    can_run
}

fn function_bindings_for_scope(checker: &Checker<'_>, scope: ScopeId) -> Vec<BindingId> {
    checker
        .semantic_analysis()
        .function_bindings_by_scope()
        .filter(move |(candidate_scope, _)| *candidate_scope == scope)
        .flat_map(|(_, bindings)| bindings.iter().copied())
        .collect()
}

fn call_site_resolves_to_binding(
    checker: &Checker<'_>,
    site: &CallSite,
    function_binding: BindingId,
) -> bool {
    let function = checker.semantic().binding(function_binding);
    checker
        .semantic_analysis()
        .visible_function_binding_at_call(&function.name, site.name_span)
        .or_else(|| {
            checker
                .semantic_analysis()
                .visible_function_binding_defined_before(
                    &function.name,
                    site.scope,
                    site.name_span.start.offset,
                )
        })
        == Some(function_binding)
}

fn span_can_run(checker: &Checker<'_>, span: Span) -> bool {
    let blocks = checker.semantic_analysis().block_ids_for_span(span);
    blocks.is_empty()
        || blocks
            .iter()
            .any(|block| !checker.semantic_analysis().block_is_unreachable(*block))
}

fn span_can_run_before_call(
    checker: &Checker<'_>,
    source_span: Span,
    call_blocks: &[shuck_semantic::BlockId],
) -> bool {
    span_can_run_before_blocks(checker, source_span, call_blocks)
}

fn span_can_run_before_definition(
    checker: &Checker<'_>,
    source_span: Span,
    function: &Binding,
) -> bool {
    let function_blocks = checker
        .semantic_analysis()
        .blocks_containing_binding(function.id);
    if function_blocks.is_empty() {
        return source_span.start.offset < function.span.start.offset;
    }

    span_can_run_before_blocks(checker, source_span, function_blocks)
}

fn span_can_run_before_blocks(
    checker: &Checker<'_>,
    source_span: Span,
    target_blocks: &[shuck_semantic::BlockId],
) -> bool {
    let source_blocks = checker.semantic_analysis().block_ids_for_span(source_span);
    if source_blocks.is_empty() || target_blocks.is_empty() {
        return true;
    }

    let reachable_source_blocks = source_blocks
        .iter()
        .copied()
        .filter(|block| !checker.semantic_analysis().block_is_unreachable(*block))
        .collect::<Vec<_>>();
    if reachable_source_blocks.is_empty() {
        return false;
    }

    checker.semantic_analysis().blocks_have_path_avoiding(
        &reachable_source_blocks,
        target_blocks,
        &FxHashSet::default(),
    )
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn call_is_unreachable(checker: &Checker<'_>, call: &CallSite) -> bool {
    let blocks = checker.semantic_analysis().block_ids_for_span(call.span);
    !blocks.is_empty()
        && blocks
            .iter()
            .all(|block| checker.semantic_analysis().block_is_unreachable(*block))
}

fn call_path_terminates_before_function(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> bool {
    let terminator_blocks = terminator_blocks_between_call_and_function(checker, call, function);
    if terminator_blocks.is_empty() {
        return false;
    }

    let Some(call_block) = innermost_block_for_span(checker, call.span) else {
        return false;
    };
    let mut function_blocks = checker
        .semantic_analysis()
        .blocks_containing_binding(function.id)
        .to_vec();
    if function_blocks.is_empty()
        && let Some(command_id) = checker
            .semantic()
            .innermost_command_id_at(function.span.start.offset)
    {
        function_blocks.extend(
            checker
                .semantic_analysis()
                .block_ids_for_span(checker.semantic().command_span(command_id))
                .iter()
                .copied(),
        );
    }
    if checker.semantic_analysis().block_is_unreachable(call_block) || function_blocks.is_empty() {
        return false;
    }

    function_blocks.iter().copied().all(|function_block| {
        checker.semantic_analysis().blocks_cover_all_paths_to_block(
            call_block,
            function_block,
            &terminator_blocks,
        )
    })
}

fn innermost_block_for_span(checker: &Checker<'_>, span: Span) -> Option<shuck_semantic::BlockId> {
    checker
        .semantic_analysis()
        .block_ids_for_span(span)
        .last()
        .copied()
        .or_else(|| {
            checker
                .semantic()
                .innermost_command_id_at(span.start.offset)
                .and_then(|command_id| {
                    checker
                        .semantic_analysis()
                        .block_ids_for_span(checker.semantic().command_span(command_id))
                        .last()
                        .copied()
                })
        })
}

fn terminator_blocks_between_call_and_function(
    checker: &Checker<'_>,
    call: &CallSite,
    function: &Binding,
) -> FxHashSet<shuck_semantic::BlockId> {
    let call_runtime_scope = checker.semantic().enclosing_function_scope(call.scope);
    checker
        .facts()
        .command_facts()
        .structural_commands()
        .filter(|fact| {
            fact.effective_name_is("exit")
                || (fact.effective_name_is("return")
                    && fact.enclosing_function_scope().is_some()
                    && checker
                        .semantic()
                        .innermost_transient_scope_within_function(fact.scope())
                        .is_none())
        })
        .filter(|fact| fact.enclosing_function_scope() == call_runtime_scope)
        .filter(|fact| {
            fact.body_span().start.offset > call.name_span.start.offset
                && fact.body_span().start.offset < function.span.start.offset
        })
        .flat_map(|fact| {
            let blocks = checker.semantic_analysis().block_ids_for_span(fact.span());
            if !blocks.is_empty() {
                return blocks.to_vec();
            }
            checker
                .semantic_analysis()
                .block_ids_for_span(checker.semantic().command_span(fact.id()))
                .to_vec()
        })
        .collect()
}

fn function_definition_is_branch_local(checker: &Checker<'_>, function: &Binding) -> bool {
    let cfg = checker.semantic_analysis().cfg();
    checker
        .semantic_analysis()
        .blocks_containing_binding(function.id)
        .iter()
        .copied()
        .any(|block| block_is_inside_branch(cfg, block))
}

fn call_is_in_condition_context(checker: &Checker<'_>, call: &CallSite) -> bool {
    checker
        .semantic()
        .innermost_command_id_at(call.name_span.start.offset)
        .and_then(|id| checker.semantic().command_condition_role(id))
        .is_some()
}

fn block_is_inside_branch(
    cfg: &shuck_semantic::ControlFlowGraph,
    mut block: shuck_semantic::BlockId,
) -> bool {
    let mut seen = FxHashSet::default();
    while seen.insert(block) {
        let predecessors = cfg.predecessors(block);
        let [predecessor] = predecessors else {
            return false;
        };
        let Some((_, edge)) = cfg
            .successors(*predecessor)
            .iter()
            .find(|(successor, _)| *successor == block)
        else {
            return false;
        };
        if !matches!(
            edge,
            shuck_semantic::EdgeKind::Sequential | shuck_semantic::EdgeKind::NestedRegion
        ) {
            return true;
        }
        block = *predecessor;
    }

    false
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_top_level_calls_before_definition() {
        let source = "\
#!/bin/bash
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_subshell_calls_before_later_subshell_definition() {
        let source = "\
#!/bin/bash
(
  do_thing
  do_thing() {
    echo hi
  }
)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_command_substitution_calls_before_later_definition() {
        let source = "\
#!/bin/bash
value=\"$(do_thing; do_thing() { echo hi; })\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_calls_after_definition() {
        let source = "\
#!/bin/bash
do_thing() {
  echo hi
}
do_thing
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_straight_line_calls_between_redefinitions() {
        let source = "\
#!/bin/bash
do_thing() {
  echo first
}
do_thing
do_thing() {
  echo second
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_checked_status_calls_before_later_redefinition() {
        let source = "\
#!/bin/bash
do_thing() {
  echo first
}
ready=1 && do_thing
do_thing() {
  echo second
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_calls_after_branch_local_prior_definition_before_later_definition() {
        let source = "\
#!/bin/bash
if [[ $ready ]]; then
  do_thing() {
    echo maybe
  }
fi
do_thing
do_thing() {
  echo later
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_calls_before_branch_local_definition() {
        let source = "\
#!/bin/bash
do_thing
if [[ $ready ]]; then
  do_thing() {
    echo later
  }
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_branch_local_calls_before_later_branch_definition() {
        let source = "\
#!/bin/bash
if [[ $ready ]]; then
  do_thing
  do_thing() {
    echo later
  }
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_top_level_calls_before_later_nested_definition() {
        let source = "\
#!/bin/bash
do_thing
loader() {
  do_thing() {
    echo hi
  }
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_calls_inside_function_bodies() {
        let source = "\
#!/bin/bash
wrapper() {
  do_thing
}
do_thing() {
  echo hi
}
wrapper
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_inside_function_bodies_invoked_before_definition() {
        let source = "\
#!/bin/bash
wrapper() {
  do_thing
}
wrapper
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_multi_hop_function_body_calls_invoked_before_definition() {
        let source = "\
#!/bin/bash
first() {
  do_thing
}
second() {
  first
}
second
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_multi_hop_function_body_calls_invoked_after_definition() {
        let source = "\
#!/bin/bash
first() {
  do_thing
}
second() {
  first
}
do_thing() {
  echo hi
}
second
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_nested_function_calls_before_local_definition() {
        let source = "\
#!/bin/bash
wrapper() {
  do_thing
  do_thing() {
    echo hi
  }
}
wrapper
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_calls_after_source_by_default() {
        let source = "\
#!/bin/bash
source ./helpers.sh
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_before_later_source_by_default() {
        let source = "\
#!/bin/bash
do_thing
source ./helpers.sh
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_calls_after_comment_only_source_directive() {
        let source = "\
#!/bin/bash
# shellcheck source=./helpers.sh
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_calls_after_runtime_source_with_directive_by_default() {
        let source = "\
#!/bin/bash
# shellcheck source=./helpers.sh
source \"$helper\"
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_nested_calls_after_top_level_source_by_default() {
        let source = "\
#!/bin/bash
source ./helpers.sh
wrapper() {
  do_thing
}
wrapper
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_nested_calls_when_top_level_source_runs_before_wrapper_call() {
        let source = "\
#!/bin/bash
wrapper() {
  do_thing
}
source ./helpers.sh
wrapper
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_multi_hop_nested_calls_after_top_level_source_by_default() {
        let source = "\
#!/bin/bash
source ./helpers.sh
first() {
  do_thing
}
second() {
  first
}
second
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_after_source_inside_unexecuted_function() {
        let source = "\
#!/bin/bash
load_helpers() {
  source ./helpers.sh
}
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_calls_after_executed_source_loader_function() {
        let source = "\
#!/bin/bash
load_helpers() {
  source ./helpers.sh
}
load_helpers
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_before_source_loader_function_runs() {
        let source = "\
#!/bin/bash
load_helpers() {
  source ./helpers.sh
}
do_thing
load_helpers
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_nested_calls_after_executed_source_loader_function() {
        let source = "\
#!/bin/bash
wrapper() {
  load_helpers() {
    source ./helpers.sh
  }
  load_helpers
  do_thing
  do_thing() {
    echo hi
  }
}
wrapper
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_nested_calls_when_orchestrator_sources_before_wrapper_call() {
        let source = "\
#!/bin/bash
load_helpers() {
  source ./helpers.sh
}
wrapper() {
  do_thing
}
main() {
  load_helpers
  wrapper
}
main
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_after_unreachable_source() {
        let source = "\
#!/bin/bash
if [[ $fatal ]]; then
  exit 1
  source ./helpers.sh
fi
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn option_can_report_calls_after_source() {
        let source = "\
#!/bin/bash
source ./helpers.sh
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_c161_ignore_after_source(false),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_unreachable_top_level_calls() {
        let source = "\
#!/bin/bash
exit 0
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_calls_on_paths_that_terminate_before_definition() {
        let source = "\
#!/bin/bash
if [[ $# -ne 2 ]]; then
  do_thing
  exit 1
fi
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_when_nonterminating_path_reaches_definition() {
        let source = "\
#!/bin/bash
do_thing
if [[ $fatal ]]; then
  exit 1
fi
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_function_local_calls_when_nonreturning_path_reaches_definition() {
        let source = "\
#!/bin/bash
wrapper() {
  do_thing
  if [[ $fatal ]]; then
    return 1
  fi
  do_thing() {
    echo hi
  }
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_top_level_calls_before_later_definition_after_return() {
        let source = "\
#!/bin/bash
do_thing
return 1
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn reports_subshell_calls_before_later_definition_after_return() {
        let source = "\
#!/bin/bash
wrapper() {
  (
    do_thing
    return 1
    do_thing() {
      echo hi
    }
  )
}
wrapper
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_short_circuit_branches_that_exit_before_definition() {
        let source = "\
#!/bin/bash
[[ $# -ne 2 ]] && { _log FATAL; exit 1; }
_log() {
  printf '%s\\n' \"$1\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_conditional_fallback_definitions() {
        let source = "\
#!/bin/sh
if ! type : >/dev/null 2>&1; then
  type() {
    return 0
  }
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_calls_before_later_definitions_after_runtime_source() {
        let source = "\
#!/bin/sh
wrapper() {
  do_thing
  . ./helpers.sh
  do_thing() {
    echo hi
  }
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_calls_before_later_definitions_after_runtime_source_when_option_disabled() {
        let source = "\
#!/bin/sh
wrapper() {
  do_thing
  . ./helpers.sh
  do_thing() {
    echo hi
  }
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Sh)
                .with_c161_ignore_after_source(false),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_nested_branch_local_loader_stubs() {
        let source = "\
#!/bin/sh
wrapper() {
  if [ \"$COUNT\" -gt 0 ]; then
    _ rebuild
    _() { false; }
    . /tmp/loader
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_nested_branch_local_loader_stubs_when_source_option_disabled() {
        let source = "\
#!/bin/sh
wrapper() {
  if [ \"$COUNT\" -gt 0 ]; then
    _ rebuild
    _() { false; }
    . /tmp/loader
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Sh)
                .with_c161_ignore_after_source(false),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "_");
    }

    #[test]
    fn reports_posix_shell_calls_before_definition() {
        let source = "\
#!/bin/sh
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Sh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_thing");
    }

    #[test]
    fn ignores_zsh() {
        let source = "\
#!/bin/zsh
do_thing
do_thing() {
  echo hi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledBeforeDefined)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
