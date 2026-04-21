use std::collections::{HashMap, HashSet};

use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, ScopeId, ScopeKind, SemanticModel,
};

use crate::{Checker, Rule, Violation};

type BindingFamilyKey = (Option<ScopeId>, Option<ScopeId>, String);

pub struct UnusedAssignment {
    pub name: String,
}

impl Violation for UnusedAssignment {
    fn rule() -> Rule {
        Rule::UnusedAssignment
    }

    fn message(&self) -> String {
        format!("variable `{}` is assigned but never used", self.name)
    }
}

pub fn unused_assignment(checker: &mut Checker) {
    let semantic = checker.semantic();
    let unused_bindings = checker.semantic_analysis().unused_assignments();
    let unused_binding_ids = unused_bindings.iter().copied().collect::<HashSet<_>>();
    let mut families_with_used_bindings = HashSet::new();
    let mut unused_bindings_by_family = HashMap::<BindingFamilyKey, Vec<_>>::new();
    let mut last_unused_binding_by_family = HashMap::new();
    let mut local_family_scopes = HashMap::with_capacity(semantic.bindings().len());
    let mut family_keys = HashMap::with_capacity(semantic.bindings().len());

    for binding in semantic.bindings() {
        if binding.name.as_str() == "_" {
            continue;
        }

        let isolated_scope = isolated_family_scope(semantic, binding.scope);
        let local_scope = binding_local_family_scope(semantic, &local_family_scopes, binding);
        local_family_scopes.insert(binding.id, local_scope);
        family_keys.insert(
            binding.id,
            (
                isolated_scope,
                local_scope,
                binding_target_key(checker, binding),
            ),
        );
    }

    for binding in semantic.bindings() {
        if binding.name.as_str() == "_" {
            continue;
        }

        if !participates_in_unused_assignment_family(binding.kind, binding.attributes) {
            continue;
        }

        if !unused_binding_ids.contains(&binding.id) {
            families_with_used_bindings.insert(binding_family_key(&family_keys, binding.id));
        }
    }

    for binding_id in unused_bindings {
        let binding = semantic.binding(*binding_id);
        if binding.name.as_str() == "_" {
            continue;
        }

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

        let family = binding_family_key(&family_keys, binding.id);

        unused_bindings_by_family
            .entry(family.clone())
            .or_default()
            .push(*binding_id);

        last_unused_binding_by_family
            .entry(family)
            .and_modify(|current_binding_id| {
                let current = semantic.binding(*current_binding_id);
                if binding_follows_in_source(
                    current.span.start.offset,
                    current.span.end.offset,
                    binding.span.start.offset,
                    binding.span.end.offset,
                ) {
                    *current_binding_id = *binding_id;
                }
            })
            .or_insert(*binding_id);
    }

    let mut reportable_bindings = Vec::new();
    for (family, binding_ids) in unused_bindings_by_family {
        if families_with_used_bindings.contains(&family) {
            reportable_bindings.extend(binding_ids);
            continue;
        }

        if let Some(binding_id) = last_unused_binding_by_family.get(&family).copied() {
            reportable_bindings.push(binding_id);
        }
    }
    reportable_bindings
        .sort_unstable_by_key(|binding_id| semantic.binding(*binding_id).span.start.offset);

    for binding_id in reportable_bindings {
        let binding = semantic.binding(binding_id);

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

        // Exported variables are consumed by child processes.
        if binding.attributes.contains(BindingAttributes::EXPORTED) {
            continue;
        }

        // Namerefs redirect to another variable; the binding itself is not
        // a conventional assignment.
        if matches!(binding.kind, BindingKind::Nameref) {
            continue;
        }

        checker.report(
            UnusedAssignment {
                name: binding.name.to_string(),
            },
            binding.span,
        );
    }
}

fn is_reportable_unused_assignment(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::AppendAssignment | BindingKind::ParameterDefaultAssignment => false,
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

fn participates_in_unused_assignment_family(
    kind: BindingKind,
    attributes: BindingAttributes,
) -> bool {
    is_reportable_unused_assignment(kind, attributes)
        || matches!(
            kind,
            BindingKind::AppendAssignment | BindingKind::ParameterDefaultAssignment
        )
}

fn binding_follows_in_source(
    current_start: usize,
    current_end: usize,
    candidate_start: usize,
    candidate_end: usize,
) -> bool {
    candidate_start > current_start
        || (candidate_start == current_start && candidate_end > current_end)
}

fn binding_family_key(
    family_keys: &HashMap<BindingId, BindingFamilyKey>,
    binding_id: BindingId,
) -> BindingFamilyKey {
    family_keys
        .get(&binding_id)
        .cloned()
        .unwrap_or_else(|| (None, None, String::new()))
}

fn binding_target_key(checker: &Checker<'_>, binding: &Binding) -> String {
    checker
        .facts()
        .binding_target_span(binding.id)
        .map(|span| span.slice(checker.source()).to_string())
        .unwrap_or_else(|| binding.name.to_string())
}

fn binding_local_family_scope(
    semantic: &SemanticModel,
    family_scopes: &HashMap<BindingId, Option<ScopeId>>,
    binding: &Binding,
) -> Option<ScopeId> {
    if binding.attributes.contains(BindingAttributes::LOCAL) {
        Some(binding.scope)
    } else {
        inherited_local_family_scope(semantic, family_scopes, binding)
    }
}

fn isolated_family_scope(semantic: &SemanticModel, scope: ScopeId) -> Option<ScopeId> {
    semantic.ancestor_scopes(scope).find(|candidate| {
        matches!(
            semantic.scope_kind(*candidate),
            ScopeKind::Subshell | ScopeKind::CommandSubstitution | ScopeKind::Pipeline
        )
    })
}

fn inherited_local_family_scope(
    semantic: &SemanticModel,
    family_scopes: &HashMap<BindingId, Option<ScopeId>>,
    binding: &Binding,
) -> Option<ScopeId> {
    let prior =
        semantic.previous_visible_binding(&binding.name, binding.span, Some(binding.span))?;

    (prior.scope == binding.scope)
        .then(|| family_scopes.get(&prior.id).copied().flatten())
        .flatten()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_variable_name_span() {
        let source = "#!/bin/sh\nunused=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn reports_only_the_last_unused_binding_for_a_name() {
        let source = "#!/bin/sh\nfoo=1\nfoo=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn unrelated_array_writes_do_not_collapse_to_one_report() {
        let source = "#!/bin/bash\nemoji[grinning]=1\nprintf '%s\\n' \"$OTHER\"\nemoji[smile]=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 4);
    }

    #[test]
    fn local_families_stay_distinct_inside_subshells() {
        let source = "#!/bin/bash\n(f(){ local foo=1; }\ng(){ local foo=2; }\nf\ng)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn uses_source_order_when_function_bindings_share_a_name() {
        let source = "#!/bin/bash\nf(){ foo=1; }\nfoo=2\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn later_exports_suppress_the_name_family() {
        let source = "#!/bin/sh\nfoo=1\nexport foo=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn keeps_distinct_local_scopes_separate() {
        let source = "#!/bin/bash\nf(){ local foo=1; }\ng(){ local foo=2; }\nf\ng\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn later_non_reportable_bindings_do_not_hide_earlier_assignments() {
        let source = "#!/bin/bash\nfoo=1\nfoo+=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn isolated_execution_scopes_keep_separate_dedup_families() {
        let source = "#!/bin/bash\nfoo=1\n(foo=2)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn later_local_reassignments_stay_separate_across_functions() {
        let source = "#!/bin/bash\nf(){ local foo=; foo=1; }\ng(){ local foo=; foo=2; }\nf\ng\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn used_non_reportable_bindings_keep_dead_branch_arms_separate() {
        let source = "#!/bin/bash\nif a; then\n  foo=1\nelif b; then\n  foo+=x\n  echo \"$foo\"\nelse\n  foo=3\nfi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.line, 8);
    }
}
