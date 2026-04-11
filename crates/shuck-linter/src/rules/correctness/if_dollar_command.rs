use crate::{Checker, Rule, Violation};

pub struct IfDollarCommand;

impl Violation for IfDollarCommand {
    fn rule() -> Rule {
        Rule::IfDollarCommand
    }

    fn message(&self) -> String {
        "use the command's exit status directly instead of executing `$(...)` output as a condition"
            .to_owned()
    }
}

pub fn if_dollar_command(checker: &mut Checker) {
    checker.report_all_dedup(
        checker
            .facts()
            .condition_command_substitution_spans()
            .to_vec(),
        || IfDollarCommand,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_command_substitution_condition_commands() {
        let source = "\
#!/bin/bash
if $(python3 -c 'import sys' 2>/dev/null); then echo ok; fi
while $(false); do break; done
until $(false); do break; done
if ! $(false); then echo no; fi
if foo && $(false); then :; fi
if $(false) | cat; then :; fi
if cat | $(false); then :; fi
if time $(false); then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfDollarCommand));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(python3 -c 'import sys' 2>/dev/null)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
            ]
        );
    }

    #[test]
    fn ignores_non_condition_and_wrapper_argument_substitutions() {
        let source = "\
#!/bin/bash
$(false) && echo x
if foo; then :; fi
if \"$(printf '%s' foo)\"; then :; fi
if [[ \"$pm\" == apt ]] && \"$(printf '%s' missing)\" != installed; then :; fi
if command $(false); then :; fi
if env FOO=1 $(false); then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfDollarCommand));

        assert!(diagnostics.is_empty());
    }
}
