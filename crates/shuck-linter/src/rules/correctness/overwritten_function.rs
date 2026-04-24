use crate::context::FileContextTag;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};
use shuck_semantic::{
    BindingKind, BindingOrigin, OverwrittenFunction as SemanticOverwrittenFunction,
    UnreachedFunction as SemanticUnreachedFunction, UnreachedFunctionReason,
};

#[derive(Clone, Copy)]
pub enum FunctionNotReachedReason {
    Overwritten,
    ScriptTerminates,
    UnreachableDefinition,
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
        }
    }
}

pub fn overwritten_function(checker: &mut Checker) {
    let overwritten = checker.semantic_analysis().overwritten_functions().to_vec();
    let unreached = checker.semantic_analysis().unreached_functions().to_vec();

    for overwritten in overwritten {
        if overwritten.first_called {
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
        };
        report_function_definition(
            checker,
            unreached.binding,
            unreached.name.to_string(),
            reason,
        );
    }
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
    let diagnostic_span = trim_trailing_whitespace(definition_span, checker.source());

    checker.report_diagnostic_dedup(
        Diagnostic::new(OverwrittenFunction { name, reason }, diagnostic_span)
            .with_fix(Fix::unsafe_edit(Edit::deletion(definition_span))),
    );
}

fn trim_trailing_whitespace(span: shuck_ast::Span, source: &str) -> shuck_ast::Span {
    let trimmed = span.slice(source).trim_end_matches(char::is_whitespace);
    shuck_ast::Span::from_positions(span.start, span.start.advanced_by(trimmed))
}

fn should_suppress_overwrite(
    checker: &Checker<'_>,
    overwritten: &SemanticOverwrittenFunction,
) -> bool {
    let file_context = checker.file_context();
    let first = checker.semantic().binding(overwritten.first);
    let second = checker.semantic().binding(overwritten.second);

    if matches!(first.kind, BindingKind::Imported) || matches!(second.kind, BindingKind::Imported) {
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

    matches!(binding.kind, BindingKind::Imported)
        || checker.file_context().has_tag(FileContextTag::ShellSpec)
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
