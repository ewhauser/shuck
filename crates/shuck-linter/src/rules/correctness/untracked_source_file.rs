use shuck_semantic::{SourceRefDiagnosticClass, SourceRefKind};

use crate::{Checker, Rule, Violation};

pub struct UntrackedSourceFile;

impl Violation for UntrackedSourceFile {
    fn rule() -> Rule {
        Rule::UntrackedSourceFile
    }

    fn message(&self) -> String {
        "sourced file is not available to this analysis".to_owned()
    }
}

pub fn untracked_source_file(checker: &mut Checker) {
    for source_ref in checker.semantic().source_refs() {
        if source_ref.explicitly_provided {
            continue;
        }

        if matches!(source_ref.kind, SourceRefKind::DirectiveDevNull) {
            continue;
        }
        if matches!(
            source_ref.diagnostic_class,
            SourceRefDiagnosticClass::UntrackedFile
        ) {
            checker.report(UntrackedSourceFile, source_ref.path_span);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use shuck_semantic::SourcePathResolver;
    use tempfile::tempdir;

    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule, lint_file_at_path_with_resolver};

    struct TestSourceResolver {
        helper: PathBuf,
    }

    impl SourcePathResolver for TestSourceResolver {
        fn resolve_candidate_paths(&self, _source_path: &Path, candidate: &str) -> Vec<PathBuf> {
            (candidate == "./known_pins.db")
                .then_some(self.helper.clone())
                .into_iter()
                .collect()
        }
    }

    #[test]
    fn reports_missing_literal_source() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. ./missing.sh\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "./missing.sh");
    }

    #[test]
    fn ignores_resolved_literal_source() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "#!/bin/sh\n. ./helper.sh\n";
        fs::write(&helper, "echo ok\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_resolved_literal_source_when_helper_is_not_an_input() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "#!/bin/sh\n. ./helper.sh\n";
        fs::write(&helper, "echo ok\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "./helper.sh");
    }

    #[test]
    fn reports_directive_pinned_dynamic_source_when_helper_is_not_an_input() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("known_pins.db");
        let source = "\
#!/bin/bash
scriptfolder=\"$(dirname \"$0\")/\"
known_pins_dbfile=\"known_pins.db\"
# shellcheck source=./known_pins.db
source \"${scriptfolder}${known_pins_dbfile}\"
";
        fs::write(&helper, "echo ok\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"${scriptfolder}${known_pins_dbfile}\""
        );
    }

    #[test]
    fn reports_no_space_shellcheck_source_directive_when_helper_is_missing() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/bash
scriptfolder=\"$(dirname \"$0\")/\"
known_pins_dbfile=\"known_pins.db\"
#shellcheck source=./known_pins.db
source \"${scriptfolder}${known_pins_dbfile}\"
";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"${scriptfolder}${known_pins_dbfile}\""
        );
    }

    #[test]
    fn reports_directive_pinned_dynamic_source_inside_function_when_helper_is_missing() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/bash
known_pins_dbfile=\"known_pins.db\"
function wps_pin_database_prerequisites() {
  #shellcheck source=./known_pins.db
  source \"${scriptfolder}${known_pins_dbfile}\"
}
";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"${scriptfolder}${known_pins_dbfile}\""
        );
    }

    #[test]
    fn reports_directive_pinned_dynamic_source_with_resolver_when_helper_is_not_an_input() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("known_pins.db");
        let source = "\
#!/bin/bash
scriptfolder=\"$(dirname \"$0\")/\"
known_pins_dbfile=\"known_pins.db\"
# shellcheck source=./known_pins.db
source \"${scriptfolder}${known_pins_dbfile}\"
";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "echo ok\n").unwrap();

        let parse_result = Parser::with_dialect(source, shuck_parser::ShellDialect::Bash)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &parse_result);
        let resolver = TestSourceResolver {
            helper: helper.clone(),
        };

        let diagnostics = lint_file_at_path_with_resolver(
            &parse_result.file,
            source,
            &indexer,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
            None,
            Some(&main),
            Some(&resolver),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"${scriptfolder}${known_pins_dbfile}\""
        );
    }

    #[test]
    fn ignores_dynamic_sources_that_belong_to_c002() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"$helper\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_current_user_tilde_sources_that_belong_to_c002() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. ~/.bashrc\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_quoted_current_user_tilde_sources_as_untracked_files() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"~/.bashrc\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"~/.bashrc\"");
    }

    #[test]
    fn reports_escaped_current_user_tilde_sources_as_untracked_files() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \\~/.bashrc\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\\~/.bashrc");
    }

    #[test]
    fn reports_single_variable_path_tail_without_a_resolver() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"$CRASHDIR/starts/start_legacy.sh\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$CRASHDIR/starts/start_legacy.sh\""
        );
    }

    #[test]
    fn reports_bash_source_dir_templates_that_belong_to_c003() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/bash\n. \"$(dirname \"${BASH_SOURCE[0]}\")/helper.sh\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$(dirname \"${BASH_SOURCE[0]}\")/helper.sh\""
        );
    }

    #[test]
    fn reports_parameter_expansion_roots_with_static_path_tails() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. ${BUILD_ROOT}/sh/functions.sh\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "${BUILD_ROOT}/sh/functions.sh"
        );
    }

    #[test]
    fn reports_quoted_parameter_expansion_roots_with_static_path_tails() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"${BUILD_ROOT}/sh/functions.sh\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"${BUILD_ROOT}/sh/functions.sh\""
        );
    }

    #[test]
    fn reports_parameter_operator_roots_with_static_path_tails() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"${BUILD_ROOT:-$HOME/.config}/sh/functions.sh\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"${BUILD_ROOT:-$HOME/.config}/sh/functions.sh\""
        );
    }

    #[test]
    fn reports_command_substitution_roots_with_static_path_tails() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"$(dirname \"$0\")/autopause-fcns.sh\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$(dirname \"$0\")/autopause-fcns.sh\""
        );
    }

    #[test]
    fn reports_negated_parameter_expansion_roots_with_static_path_tails() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/sh
if [ ! -f ${BUILD_ROOT}/sh/functions.sh ]; then
  exit 1
elif ! . ${BUILD_ROOT}/sh/functions.sh; then
  exit 1
fi
";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "${BUILD_ROOT}/sh/functions.sh"
        );
    }

    #[test]
    fn ignores_single_variable_leaf_tail_that_belongs_to_c002() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"$helper\".generated\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_resolved_multi_dynamic_templates_that_belong_to_c002() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("tests/main.sh");
        let helper = temp.path().join("src/helper.sh");
        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let source = "#!/bin/sh\nload() { . \"$ROOT/src/$1\"; }\nload helper.sh\n";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "echo helper\n").unwrap();

        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);

        let main_path = main.clone();
        let helper_path = helper.clone();
        let resolver = move |source_path: &Path, _candidate: &str| {
            if source_path == main_path.as_path() {
                vec![helper_path.clone()]
            } else {
                Vec::new()
            }
        };

        let diagnostics = lint_file_at_path_with_resolver(
            &output.file,
            source,
            &indexer,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
            None,
            Some(&main),
            Some(&resolver),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_resolved_dynamic_source_when_helper_is_not_an_input() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("tests/main.sh");
        let helper = temp.path().join("scripts/rvm");
        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let source = "#!/bin/sh\nsource \"$rvm_path/scripts/rvm\"\n";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "echo helper\n").unwrap();

        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);

        let main_path = main.clone();
        let helper_path = helper.clone();
        let resolver = move |source_path: &Path, candidate: &str| {
            if source_path == main_path.as_path() && candidate == "scripts/rvm" {
                vec![helper_path.clone()]
            } else {
                Vec::new()
            }
        };

        let diagnostics = lint_file_at_path_with_resolver(
            &output.file,
            source,
            &indexer,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
            None,
            Some(&main),
            Some(&resolver),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$rvm_path/scripts/rvm\""
        );
    }
}
