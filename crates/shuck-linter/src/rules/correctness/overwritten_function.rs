use crate::context::FileContextTag;
use crate::{Checker, Rule, Violation};
use shuck_semantic::{BindingKind, OverwrittenFunction as SemanticOverwrittenFunction};

pub struct OverwrittenFunction {
    pub name: String,
}

impl Violation for OverwrittenFunction {
    fn rule() -> Rule {
        Rule::OverwrittenFunction
    }

    fn message(&self) -> String {
        format!(
            "function `{}` is overwritten before any direct call can reach it",
            self.name
        )
    }
}

pub fn overwritten_function(checker: &mut Checker) {
    let overwritten = checker
        .semantic_analysis()
        .overwritten_functions()
        .iter()
        .filter(|overwritten| !overwritten.first_called)
        .filter(|overwritten| !should_suppress_overwrite(checker, overwritten))
        .map(|overwritten| {
            let span = checker.semantic().binding(overwritten.first).span;
            (overwritten.name.to_string(), span)
        })
        .collect::<Vec<_>>();

    for (name, span) in overwritten {
        checker.report(OverwrittenFunction { name }, span);
    }
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

    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

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
