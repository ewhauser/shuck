use crate::facts::source_paths::source_ref_has_unanchored_path;
use crate::{Checker, Rule, ShellDialect, Violation};

pub struct UnanchoredSourcePath;

impl Violation for UnanchoredSourcePath {
    fn rule() -> Rule {
        Rule::UnanchoredSourcePath
    }

    fn message(&self) -> String {
        "source path is relative to the current directory; anchor it to the script directory"
            .to_owned()
    }
}

pub fn unanchored_source_path(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Bash {
        return;
    }

    for source_ref in checker.semantic().source_refs() {
        if source_ref_has_unanchored_path(
            source_ref,
            checker.source(),
            &checker.rule_options().c160.allowed_anchors,
        ) {
            checker.report(UnanchoredSourcePath, source_ref.path_span);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_literal_relative_source_paths() {
        let source = "\
#!/bin/bash
source ./lib.sh
. ../tools/env.sh
source lib/defaults.sh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["./lib.sh", "../tools/env.sh", "lib/defaults.sh"]
        );
    }

    #[test]
    fn accepts_default_script_directory_anchors() {
        for source in [
            "#!/bin/bash\nsource \"${BASH_SOURCE[0]%/*}/lib.sh\"\n",
            "#!/bin/bash\nsource \"${BASH_SOURCE[0]%/*}\"/lib.sh\n",
            "#!/bin/bash\nsource ${BASH_SOURCE[0]%/*}\"/lib.sh\"\n",
            "#!/bin/bash\nsource ${BASH_SOURCE[0]%/*}'/lib.sh'\n",
            "#!/bin/bash\nsource \"$(dirname \"$0\")/lib.sh\"\n",
            "#!/bin/bash\nsource \"$(dirname \"$0\")\"/lib.sh\n",
            "#!/bin/bash\nsource \"$(dirname \"${BASH_SOURCE[0]}\")/lib.sh\"\n",
            "#!/bin/bash\nsource \"$(dirname \"${BASH_SOURCE[0]}\")\"/lib.sh\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
            );

            assert!(diagnostics.is_empty(), "{source}");
        }
    }

    #[test]
    fn accepts_absolute_home_and_whole_command_substitution_paths() {
        for source in [
            "#!/bin/bash\nsource /etc/profile\n",
            "#!/bin/bash\nsource ~/.bashrc\n",
            "#!/bin/bash\nsource ~alice/lib.sh\n",
            "#!/bin/bash\nsource \"$(select_source_file)\"\n",
            "#!/bin/bash\nsource \"$(printf '%s' '/tmp)')\"\n",
            "#!/bin/bash\nsource \"$(printf \"%s\" \"/tmp)\")\"\n",
            "#!/bin/bash\nsource `select_source_file`\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
            );

            assert!(diagnostics.is_empty(), "{source}");
        }
    }

    #[test]
    fn reports_dynamic_tilde_source_paths() {
        let source = "\
#!/bin/bash
source ~${USER}/lib.sh
source ~$USER/lib.sh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["~${USER}/lib.sh", "~$USER/lib.sh"]
        );
    }

    #[test]
    fn reports_quoted_home_source_paths() {
        for source in [
            "#!/bin/bash\nsource \"~/.bashrc\"\n",
            "#!/bin/bash\nsource '~/.bashrc'\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
            );

            assert_eq!(
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.span.slice(source))
                    .collect::<Vec<_>>(),
                vec![source.lines().nth(1).unwrap().split_once(' ').unwrap().1]
            );
        }
    }

    #[test]
    fn reports_dynamic_path_prefixes_with_static_path_tails() {
        let source = "\
#!/bin/bash
source \"$SCRIPT_DIR/lib.sh\"
source \"$(pwd)/lib.sh\"
source '${BASH_SOURCE[0]%/*}/lib.sh'
source '${BASH_SOURCE[0]%/*}'/lib.sh
source \"${BASH_SOURCE[0]%/*}suffix/lib.sh\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"$SCRIPT_DIR/lib.sh\"",
                "\"$(pwd)/lib.sh\"",
                "'${BASH_SOURCE[0]%/*}/lib.sh'",
                "'${BASH_SOURCE[0]%/*}'/lib.sh",
                "\"${BASH_SOURCE[0]%/*}suffix/lib.sh\""
            ]
        );
    }

    #[test]
    fn reports_single_quoted_command_substitution_text() {
        let source = "#!/bin/bash\nsource '$(select_source_file)'\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "'$(select_source_file)'");
    }

    #[test]
    fn custom_allowed_anchors_accept_project_specific_prefixes() {
        let source =
            "#!/bin/bash\nsource \"$SCRIPT_DIR/lib.sh\"\nsource \"$SCRIPT_DIR\"/other.sh\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath)
                .with_c160_allowed_anchors(["$SCRIPT_DIR"]),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn custom_allowed_anchors_require_boundaries() {
        let source = "\
#!/bin/bash
source \"$SCRIPT_DIR/lib.sh\"
source \"$SCRIPT_DIRECTORY/lib.sh\"
source \"$SCRIPT_DIR\"suffix/lib.sh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath)
                .with_c160_allowed_anchors(["$SCRIPT_DIR"]),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"$SCRIPT_DIRECTORY/lib.sh\"",
                "\"$SCRIPT_DIR\"suffix/lib.sh"
            ]
        );
    }

    #[test]
    fn ignores_dynamic_paths_without_static_path_components() {
        let source = "#!/bin/bash\nsource \"$helper\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_dynamic_paths_without_static_components_even_with_source_directive() {
        let source = "\
#!/bin/bash
# shellcheck source=./helper.sh
source \"$helper\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_single_quoted_dynamic_text_even_with_source_directive() {
        let source = "\
#!/bin/bash
# shellcheck source=./helper.sh
source '$(select_source_file)'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "'$(select_source_file)'");
    }

    #[test]
    fn still_reports_static_runtime_paths_with_source_directives() {
        let source = "\
#!/bin/bash
# shellcheck source=./helper.sh
source ./runtime.sh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "./runtime.sh");
    }

    #[test]
    fn still_reports_static_runtime_paths_with_empty_source_directives() {
        let source = "\
#!/bin/bash
# shellcheck source=
source ./runtime.sh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "./runtime.sh");
    }

    #[test]
    fn still_reports_static_runtime_paths_with_dev_null_source_directives() {
        let source = "\
#!/bin/bash
# shellcheck source=/dev/null
source ./runtime.sh
. ../runtime.sh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["./runtime.sh", "../runtime.sh"]
        );
    }

    #[test]
    fn ignores_pure_dynamic_paths_with_dev_null_source_directives() {
        let source = "\
#!/bin/bash
# shellcheck source=/dev/null
source \"$helper\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_non_bash_shells() {
        let source = "#!/bin/sh\n. ./lib.sh\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnanchoredSourcePath).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }
}
