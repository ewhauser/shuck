use std::collections::{HashMap, HashSet};

use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, BindingOrigin, ReferenceKind,
};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

type BindingFamilyKey = String;

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
    if all_reportable_assignment_spans_suppressed(checker, semantic) {
        return;
    }

    let unused_bindings = checker
        .semantic_analysis()
        .unused_assignments_with_options(checker.rule_options().c001.semantic_options());
    let unused_binding_ids = unused_bindings.iter().copied().collect::<HashSet<_>>();
    let mut families_with_used_bindings = HashSet::new();
    let mut suppressed_binding_offsets_by_family = HashMap::<BindingFamilyKey, Vec<usize>>::new();
    let mut unused_bindings_by_family = HashMap::<BindingFamilyKey, Vec<_>>::new();
    let mut last_unused_binding_by_family = HashMap::new();
    let mut family_keys = HashMap::with_capacity(semantic.bindings().len());

    for binding in semantic.bindings() {
        if is_intentionally_unused_binding(binding) {
            continue;
        }

        family_keys.insert(binding.id, binding.name.to_string());
    }

    for reference in semantic.references() {
        if matches!(reference.kind, ReferenceKind::DeclarationName)
            || is_underscore_name(reference.name.as_str())
        {
            continue;
        }

        families_with_used_bindings.insert(reference.name.to_string());
    }

    for binding in semantic.bindings() {
        if is_intentionally_unused_binding(binding) {
            continue;
        }

        if !participates_in_unused_assignment_family(binding.kind, binding.attributes) {
            continue;
        }

        let family = binding_family_key(&family_keys, binding.id);
        let report_span = report_span_for_binding(checker, binding);
        if checker.is_suppressed_at(Rule::UnusedAssignment, report_span) {
            suppressed_binding_offsets_by_family
                .entry(family.clone())
                .or_default()
                .push(report_span.start.offset);
        }

        if binding.attributes.contains(BindingAttributes::EXPORTED) {
            families_with_used_bindings.insert(family);
            continue;
        }

        if binding_counts_as_used_family_member(binding, &unused_binding_ids) {
            families_with_used_bindings.insert(family);
        }
    }

    for binding_id in unused_bindings {
        let binding = semantic.binding(*binding_id);
        if is_intentionally_unused_binding(binding) {
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
    for family in unused_bindings_by_family.keys() {
        if families_with_used_bindings.contains(family) {
            continue;
        }

        if let Some(binding_id) = last_unused_binding_by_family.get(family).copied() {
            let binding = semantic.binding(binding_id);
            let report_offset = report_span_for_binding(checker, binding).start.offset;
            if suppressed_binding_offsets_by_family
                .get(family)
                .is_some_and(|offsets| offsets.iter().any(|offset| *offset >= report_offset))
            {
                continue;
            }
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
        let report_span = report_span_for_binding(checker, binding);
        let fix_span = binding.span;

        checker.report_diagnostic(
            Diagnostic::new(UnusedAssignment { name }, report_span)
                .with_fix(Fix::unsafe_edit(Edit::replacement("_", fix_span))),
        );
    }
}

fn all_reportable_assignment_spans_suppressed(
    checker: &Checker<'_>,
    semantic: &shuck_semantic::SemanticModel,
) -> bool {
    let mut saw_reportable_binding = false;
    for binding in semantic.bindings() {
        if is_intentionally_unused_binding(binding) {
            continue;
        }

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

        if binding.attributes.contains(BindingAttributes::EXPORTED)
            || matches!(binding.kind, BindingKind::Nameref)
        {
            continue;
        }

        saw_reportable_binding = true;
        if !checker.is_suppressed_at(
            Rule::UnusedAssignment,
            report_span_for_binding(checker, binding),
        ) {
            return false;
        }
    }

    saw_reportable_binding
}

fn is_intentionally_unused_binding(binding: &Binding) -> bool {
    is_underscore_name(binding.name.as_str()) || is_intentionally_unused_read_placeholder(binding)
}

fn is_intentionally_unused_read_placeholder(binding: &Binding) -> bool {
    matches!(binding.kind, BindingKind::ReadTarget)
        && matches!(binding.name.as_str(), "rest" | "REST")
}

fn is_underscore_name(name: &str) -> bool {
    name.starts_with('_')
}

fn is_reportable_unused_assignment(kind: BindingKind, _attributes: BindingAttributes) -> bool {
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
        BindingKind::Declaration(_) => true,
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

fn participates_in_unused_assignment_family(
    kind: BindingKind,
    _attributes: BindingAttributes,
) -> bool {
    matches!(
        kind,
        BindingKind::Assignment
            | BindingKind::ArrayAssignment
            | BindingKind::LoopVariable
            | BindingKind::ReadTarget
            | BindingKind::MapfileTarget
            | BindingKind::PrintfTarget
            | BindingKind::GetoptsTarget
            | BindingKind::ArithmeticAssignment
            | BindingKind::AppendAssignment
            | BindingKind::ParameterDefaultAssignment
            | BindingKind::Declaration(_)
    )
}

fn binding_counts_as_used_family_member(
    binding: &Binding,
    unused_binding_ids: &HashSet<BindingId>,
) -> bool {
    if matches!(binding.kind, BindingKind::AppendAssignment) {
        return true;
    }

    if binding
        .attributes
        .contains(BindingAttributes::SELF_REFERENTIAL_READ)
    {
        return true;
    }

    if binding
        .attributes
        .contains(BindingAttributes::EMPTY_INITIALIZER)
    {
        return false;
    }

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

fn report_span_for_binding(checker: &Checker<'_>, binding: &Binding) -> shuck_ast::Span {
    match binding.origin {
        BindingOrigin::LoopVariable {
            definition_span, ..
        } => loop_keyword_report_span(checker, definition_span).unwrap_or(definition_span),
        BindingOrigin::Assignment {
            definition_span, ..
        }
        | BindingOrigin::ParameterDefaultAssignment { definition_span }
        | BindingOrigin::Imported { definition_span }
        | BindingOrigin::FunctionDefinition { definition_span }
        | BindingOrigin::BuiltinTarget {
            definition_span, ..
        }
        | BindingOrigin::Declaration { definition_span }
        | BindingOrigin::Nameref { definition_span } => definition_span,
        BindingOrigin::ArithmeticAssignment { target_span, .. } => target_span,
    }
}

fn loop_keyword_report_span(
    checker: &Checker<'_>,
    definition_span: shuck_ast::Span,
) -> Option<shuck_ast::Span> {
    if let Some(header) = checker.facts().for_headers().iter().find(|header| {
        header
            .command()
            .targets
            .iter()
            .any(|target| target.span == definition_span)
    }) {
        return Some(keyword_span(header.command().span, "for"));
    }

    checker
        .facts()
        .select_headers()
        .iter()
        .find(|header| header.command().variable_span == definition_span)
        .map(|header| keyword_span(header.command().span, "select"))
}

fn keyword_span(command_span: shuck_ast::Span, keyword: &str) -> shuck_ast::Span {
    shuck_ast::Span::from_positions(command_span.start, command_span.start.advanced_by(keyword))
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
    family_keys.get(&binding_id).cloned().unwrap_or_default()
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
    fn reads_before_assignments_suppress_unused_assignment_for_that_name() {
        let source = "#!/bin/bash\necho \"$foo\"\nfoo=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn uncalled_function_reads_suppress_unused_assignment_for_that_name() {
        let source = "#!/bin/bash\nfoo=1\nshow_foo() { echo \"$foo\"; }\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn variable_reads_do_not_conflict_with_same_named_functions() {
        let source = "#!/bin/bash\nprogress=\nprogress=1\nprogress() { :; }\n[ \"$progress\" ] && progress ok\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn read_rest_names_are_treated_as_intentional_placeholders() {
        let source = "#!/bin/bash\nread -r cron_id rest\nprintf '%s\\n' \"$cron_id\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_declaration_queries_do_not_create_unused_variable_targets() {
        let source = "\
#!/bin/bash
if ! declare -f -F config_unset >/dev/null; then
  :
fi
eval \"$(declare -f cd)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn plain_rest_names_are_reported() {
        let source = "#!/bin/bash\nrest=1\nREST=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "rest");
        assert_eq!(diagnostics[1].span.slice(source), "REST");
    }

    #[test]
    fn read_rest_placeholders_do_not_hide_real_dead_assignments() {
        let source = "\
#!/bin/bash
rest=1
read -r field rest
printf '%s\\n' \"$field\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "rest");
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn read_rest_placeholders_do_not_hide_branch_declarations() {
        let source = "\
#!/bin/bash
f(){
  if cond; then
    local rest
  else
    read -r _ rest
  fi
}
f
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "rest");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn unread_read_targets_are_still_reported() {
        let source = "#!/bin/bash\nread -r first second\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "first");
        assert_eq!(diagnostics[1].span.slice(source), "second");
    }

    #[test]
    fn reports_last_dead_binding_when_every_conditional_arm_assigns_the_name() {
        let source = "\
#!/bin/sh
if [ \"$ARCH\" = \"arm\" ]; then
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
else
  LIBDIRSUFFIX=\"\"
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 7);
        assert_eq!(diagnostics[0].span.slice(source), "LIBDIRSUFFIX");
    }

    #[test]
    fn reports_last_dead_binding_when_branch_family_ends_with_empty_clear() {
        let source = "\
#!/bin/sh
if [ \"$ARCH\" = \"arm\" ]; then
  foo=\"x\"
else
  foo=\"\"
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 5);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn array_writes_collapse_to_the_last_name_report() {
        let source = "#!/bin/bash\nemoji[grinning]=1\nprintf '%s\\n' \"$OTHER\"\nemoji[smile]=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "emoji[smile]");
    }

    #[test]
    fn arithmetic_indexed_writes_collapse_to_the_last_name_report() {
        let source = "#!/bin/bash\n(( box[1] = 1 ))\n(( box[2] = 2 ))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn local_families_collapse_by_name_inside_subshells() {
        let source = "#!/bin/bash\n(f(){ local foo=1; }\ng(){ local foo=2; }\nf\ng)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
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
        assert_eq!(diagnostics[1].span.slice(source), "for");
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
    fn used_loop_variables_suppress_prior_dead_assignments() {
        let source = "\
#!/bin/bash
foo=1
foo=2
for foo in a b; do
  printf '%s\\n' \"$foo\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn later_exports_suppress_the_name_family() {
        let source = "#!/bin/sh\nfoo=1\nexport foo=2\nbar=1\nexport bar=\nbar=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_scopes_collapse_by_name() {
        let source = "#!/bin/bash\nf(){ local foo=1; }\ng(){ local foo=2; }\nf\ng\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn later_appends_suppress_the_name_family() {
        let source = "#!/bin/bash\nfoo=1\nfoo+=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn command_prefix_assignments_are_treated_as_consumed() {
        let source = "#!/bin/sh\nfoo=1 echo ok\nbar=1 export baz=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn eval_arguments_keep_delayed_references_live() {
        let source = "#!/bin/bash\nDEF=default\nVAR=name\neval \"$VAR=\\${$VAR:-$DEF}\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn eval_single_quoted_strings_do_not_keep_assignments_live() {
        let source = "#!/bin/sh\nas_lineno_1=$LINENO\neval 'test \"$as_lineno_1\"'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "as_lineno_1");
    }

    #[test]
    fn eval_escaped_dollar_payloads_do_not_keep_assignments_live() {
        let source = r#"#!/bin/bash
foo=1
eval "echo \\\$foo"
"#;
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn eval_comment_payloads_do_not_keep_assignments_live() {
        let source = r#"#!/bin/bash
foo=1
eval "echo ok # \$foo"
"#;
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn variable_set_array_tests_keep_target_family_live() {
        let source = "\
#!/bin/bash
f() {
  local -A seen
  seen=()
  if [[ ! -v \"seen[${key}]\" ]]; then
    seen[${key}]=1
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn invalid_variable_set_test_operands_do_not_keep_assignments_live() {
        let source = "\
#!/bin/bash
foo=1
[[ -v '$foo' ]]
[[ -v 1foo ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn quoted_variable_set_test_operands_keep_assignments_live() {
        let source = "\
#!/bin/bash
foo=1
[[ -v 'foo' ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn flags_parent_is_runtime_consumed() {
        let source = "#!/bin/sh\nFLAGS_PARENT=\"git flow\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn used_uninitialized_local_declarations_suppress_dead_branch_arms() {
        let source = "#!/bin/bash\nf(){\n  if a; then\n    foo=1\n  elif b; then\n    local foo\n    echo \"$foo\"\n  else\n    foo=3\n  fi\n}\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_uninitialized_local_branches_report_the_last_dead_binding() {
        let source =
            "#!/bin/bash\nf(){\n  if a; then\n    foo=1\n  else\n    local foo\n  fi\n}\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 6);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
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
    fn reports_declaration_only_bindings_by_default() {
        let source = "\
#!/bin/bash
f(){
  local cur
  declare words
}
f
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "cur");
        assert_eq!(diagnostics[1].span.slice(source), "words");
    }

    #[test]
    fn isolated_execution_scopes_collapse_by_name() {
        let source = "#!/bin/bash\nfoo=1\n(foo=2)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn later_local_reassignments_collapse_across_functions() {
        let source = "#!/bin/bash\nf(){ local foo=; foo=1; }\ng(){ local foo=; foo=2; }\nf\ng\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
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
    fn used_variable_suppresses_prior_dead_reassignment_before_empty_clear() {
        let source = "#!/bin/bash\nfoo=1\n: \"$foo\"\nfoo=2\nfoo=\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pre_use_empty_initializer_in_used_family_is_suppressed() {
        let source = "#!/bin/bash\nfoo=\nfoo=1\n: \"$foo\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn used_non_reportable_bindings_suppress_dead_branch_arms() {
        let source = "#!/bin/bash\nif a; then\n  foo=1\nelif b; then\n  foo+=x\n  echo \"$foo\"\nelse\n  foo=3\nfi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }
}
