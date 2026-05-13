use std::path::Path;

use crate::{Checker, Rule, Violation};

pub struct ShebangShellPolicy;

impl Violation for ShebangShellPolicy {
    fn rule() -> Rule {
        Rule::ShebangShellPolicy
    }

    fn message(&self) -> String {
        "shebang interpreter is outside the configured shell policy".to_owned()
    }
}

pub fn shebang_shell_policy(checker: &mut Checker) {
    let Some(shebang) = checker.facts().source_facts().shebang_interpreter() else {
        return;
    };

    if !interpreter_is_allowed(
        shebang.interpreter(),
        &checker.rule_options().s078.allowed_shells,
    ) {
        checker.report(ShebangShellPolicy, shebang.span());
    }
}

fn interpreter_is_allowed(interpreter: &str, allowed_shells: &[String]) -> bool {
    let interpreter = normalize_shell_name(interpreter);
    allowed_shells
        .iter()
        .map(|allowed| normalize_shell_name(allowed))
        .any(|allowed| allowed == interpreter)
}

fn normalize_shell_name(shell: &str) -> String {
    Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_shebangs_outside_the_allowed_shell_policy() {
        let source = "#!/bin/sh\necho hello\n  #!/bin/dash\necho again\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ShebangShellPolicy));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["#!/bin/sh"]
        );
    }

    #[test]
    fn reports_indented_shebangs_outside_the_allowed_shell_policy() {
        let source = "  #!/bin/sh\necho hello\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ShebangShellPolicy));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["  #!/bin/sh"]
        );
    }

    #[test]
    fn accepts_configured_shell_names_and_env_shebangs() {
        for source in [
            "#!/usr/bin/env bash\necho hello\n",
            "#!/usr/bin/env -S /bin/zsh -f\necho hello\n",
            "#!/opt/homebrew/bin/zsh\necho hello\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::ShebangShellPolicy)
                    .with_s078_allowed_shells(["bash", "zsh"]),
            );

            assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
        }
    }

    #[test]
    fn ignores_files_without_a_parseable_shebang_interpreter() {
        for source in ["echo hello\n", "# !/bin/sh\necho hello\n"] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::ShebangShellPolicy));

            assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
        }
    }
}
