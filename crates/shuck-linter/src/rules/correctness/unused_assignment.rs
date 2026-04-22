use std::collections::{HashMap, HashSet};

use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, ScopeId, ScopeKind, SemanticModel,
};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

type BindingFamilyKey = (Option<ScopeId>, Option<ScopeId>, String);

pub struct UnusedAssignment {
    pub name: String,
}

impl Violation for UnusedAssignment {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnusedAssignment
    }

    fn message(&self) -> String {
        format!("variable `{}` is assigned but never used", self.name)
    }

    fn fix_title(&self) -> Option<String> {
        Some("rename the unused assignment target to `_`".to_owned())
    }
}

pub fn unused_assignment(checker: &mut Checker) {
    let semantic = checker.semantic();
    let unused_bindings = checker
        .semantic_analysis()
        .unused_assignments_with_options(checker.rule_options().c001.semantic_options());
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

        if binding_counts_as_used_family_member(binding, &unused_binding_ids) {
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
            reportable_bindings.extend(binding_ids.into_iter().filter(|binding_id| {
                let binding = semantic.binding(*binding_id);
                !binding
                    .attributes
                    .contains(BindingAttributes::EMPTY_INITIALIZER)
            }));
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

        let name = binding.name.to_string();
        let span = binding.span;

        checker.report_diagnostic(
            Diagnostic::new(UnusedAssignment { name }, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement("_", span))),
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
            BindingKind::LoopVariable
                | BindingKind::AppendAssignment
                | BindingKind::ParameterDefaultAssignment
                | BindingKind::Declaration(_)
        )
}

fn binding_counts_as_used_family_member(
    binding: &Binding,
    unused_binding_ids: &HashSet<BindingId>,
) -> bool {
    if unused_binding_ids.contains(&binding.id) {
        return false;
    }

    if matches!(binding.kind, BindingKind::Declaration(_))
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
    {
        return !binding.references.is_empty();
    }

    true
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
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn anchors_on_variable_name_span_and_attaches_fix_metadata() {
        let source = "#!/bin/sh\nunused=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("rename the unused assignment target to `_`")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_reported_assignment_targets() {
        let source = "\
#!/bin/bash
unused=1
arr[0]=x
read -r read_target <<< \"value\"
printf -v printf_target '%s' ok
while getopts \"ab\" opt; do
  :
done
((arith = 1))
for item in a b; do
  :
done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnusedAssignment),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 7);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
_=1
_[0]=x
read -r _ <<< \"value\"
printf -v _ '%s' ok
while getopts \"ab\" _; do
  :
done
((_ = 1))
for _ in a b; do
  :
done
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_existing_underscore_targets_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
_=1
_[0]=x
read -r _ <<< \"value\"
printf -v _ '%s' ok
while getopts \"ab\" _; do
  :
done
((_ = 1))
for _ in a b; do
  :
done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnusedAssignment),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C001.sh").as_path(),
            &LinterSettings::for_rule(Rule::UnusedAssignment),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C001_fix_C001.sh", result);
        Ok(())
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
    fn arithmetic_indexed_writes_do_not_collapse_to_one_report() {
        let source = "#!/bin/bash\n(( box[1] = 1 ))\n(( box[2] = 2 ))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
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
    fn reports_unused_for_loop_counters() {
        let source = "\
#!/bin/bash
unused=1
for i in {0..5}; do
  printf '%s\\n' retry
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.slice(source), "i");
    }

    #[test]
    fn body_reads_keep_loop_counters_live() {
        let source = "\
#!/bin/bash
for i in {0..5}; do
  printf '%s\\n' \"$i\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn used_loop_variables_keep_prior_dead_assignments_separate() {
        let source = "\
#!/bin/bash
foo=1
foo=2
for foo in a b; do
  printf '%s\\n' \"$foo\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
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
    fn used_uninitialized_local_declarations_keep_dead_branch_arms_separate() {
        let source = "#!/bin/bash\nf(){\n  if a; then\n    foo=1\n  elif b; then\n    local foo\n    echo \"$foo\"\n  else\n    foo=3\n  fi\n}\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[1].span.start.line, 9);
    }

    #[test]
    fn unused_uninitialized_local_branches_do_not_hide_dead_assignments() {
        let source =
            "#!/bin/bash\nf(){\n  if a; then\n    foo=1\n  else\n    local foo\n  fi\n}\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn branch_local_uninitialized_declarations_keep_prior_defs_live() {
        let source = "#!/bin/bash\nf(){\n  foo=1\n  if cond; then\n    local foo\n  fi\n  echo \"$foo\"\n}\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_uninitialized_declarations_do_not_split_linear_chains() {
        let source = "#!/bin/bash\nf(){\n  local foo\n  foo=1\n  foo=2\n}\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 5);
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
    fn used_variable_empty_clear_is_suppressed() {
        let source = "#!/bin/bash\nfoo=1\n: \"$foo\"\nfoo=\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn used_variable_quoted_empty_clear_is_suppressed() {
        let source = "#!/bin/bash\nfoo=1\n: \"$foo\"\nfoo=\"\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn standalone_empty_initializer_is_still_reported() {
        let source = "#!/bin/bash\nfoo=\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn empty_clear_does_not_hide_prior_dead_reassignment() {
        let source = "#!/bin/bash\nfoo=1\n: \"$foo\"\nfoo=2\nfoo=\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn pre_use_empty_initializer_in_used_family_is_suppressed() {
        let source = "#!/bin/bash\nfoo=\nfoo=1\n: \"$foo\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
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
