use shuck_semantic::{SourceRefDiagnosticClass, SourceRefKind};

use crate::{Checker, Rule, Violation};

pub struct DynamicSourcePath;

impl Violation for DynamicSourcePath {
    fn rule() -> Rule {
        Rule::DynamicSourcePath
    }

    fn message(&self) -> String {
        "source path is built at runtime".to_owned()
    }
}

pub fn dynamic_source_path(checker: &mut Checker) {
    for source_ref in checker.semantic().source_refs() {
        if matches!(source_ref.kind, SourceRefKind::DirectiveDevNull) {
            continue;
        }
        if matches!(
            source_ref.diagnostic_class,
            SourceRefDiagnosticClass::DynamicPath
        ) {
            checker.report(DynamicSourcePath, source_ref.path_span);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn flags_escaped_source_builtins_with_dynamic_paths() {
        let source = "\
#!/bin/bash
\\. \"$rvm_environments_path/$1\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DynamicSourcePath));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$rvm_environments_path/$1\""
        );
    }

    #[test]
    fn ignores_literal_leading_backslashes_in_other_command_names() {
        for source in [
            "#!/bin/bash\n\"\\\\.\" \"$rvm_environments_path/$1\"\n",
            "#!/bin/bash\n'\\source' \"$rvm_environments_path/$1\"\n",
            "#!/bin/bash\n\\\\. \"$rvm_environments_path/$1\"\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::DynamicSourcePath));

            assert!(diagnostics.is_empty(), "{source}");
        }
    }

    #[test]
    fn ignores_parameter_expansion_roots_with_static_path_tails() {
        for source in [
            "#!/bin/bash\nsource \"${rvm_path?}/scripts/rvm\"\n",
            "#!/bin/bash\nsource \"${XDG_CONFIG_HOME:-$HOME/.config}/fzf/fzf.bash\"\n",
            "#!/bin/bash\nsource \"${rvm_path%/*}/scripts/rvm\"\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::DynamicSourcePath));

            assert!(diagnostics.is_empty(), "{source}");
        }
    }

    #[test]
    fn ignores_dynamic_sources_when_own_line_source_directive_persists() {
        for source in [
            "#!/bin/bash\n# shellcheck source=/dev/null\nfoo() { echo hi; }\nsource \"$x\"\n",
            "#!/bin/bash\n# shellcheck source=/dev/null\nif true; then\n  source \"$config_file\"\nfi\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::DynamicSourcePath));

            assert!(diagnostics.is_empty(), "{source}");
        }
    }

    #[test]
    fn ignores_command_substitution_roots_with_static_path_tails() {
        for source in [
            "#!/bin/bash\nsource \"$(git --exec-path)/git-sh-setup\"\n",
            "#!/bin/sh\n. \"$(dirname \"$0\")/autopause-fcns.sh\"\n",
            "#!/bin/ksh\nsource \"$(cd \"$(dirname \"${0}\")\"; pwd)/../nb\"\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::DynamicSourcePath));

            assert!(diagnostics.is_empty(), "{source}");
        }
    }
}
