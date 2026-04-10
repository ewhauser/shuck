use crate::{Checker, CommandSubstitutionKind, Rule, ShellDialect, Violation};

pub struct LsInSubstitution;

impl Violation for LsInSubstitution {
    fn rule() -> Rule {
        Rule::LsInSubstitution
    }

    fn message(&self) -> String {
        "avoid capturing `ls` output in command substitutions; use a glob or `find` instead"
            .to_owned()
    }
}

pub fn ls_in_substitution(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .filter(|substitution| {
            substitution.kind() == CommandSubstitutionKind::Command
                && substitution.body_contains_ls()
                && substitution.stdout_is_captured()
        })
        .map(|substitution| substitution.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LsInSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_raw_ls_command_substitutions() {
        let source = "\
#!/bin/bash
LAYOUTS=\"$(ls layout.*.h | cut -d. -f2 | xargs echo)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsInSubstitution));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "$(ls layout.*.h | cut -d. -f2 | xargs echo)"
        );
    }

    #[test]
    fn ignores_wrapped_ls_and_non_command_substitutions() {
        let source = "\
#!/bin/sh
plain=\"$(command ls)\"
quiet=\"$(ls >/dev/null)\"
empty=\"$(printf foo)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsInSubstitution));

        assert!(diagnostics.is_empty());
    }
}
