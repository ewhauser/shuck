use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct IfDollarCommand;

impl Violation for IfDollarCommand {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::IfDollarCommand
    }

    fn message(&self) -> String {
        "use the command's exit status directly instead of executing the output from `$(...)`"
            .to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the command substitution wrapper".to_owned())
    }
}

pub fn if_dollar_command(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .command_substitution_command_spans()
        .iter()
        .copied()
        .filter_map(|span| if_dollar_command_fix(span, source))
        .map(|(span, fix)| Diagnostic::new(IfDollarCommand, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn if_dollar_command_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let text = span.slice(source);
    let body = text.strip_prefix("$(")?.strip_suffix(')')?;
    Some((span, Fix::unsafe_edit(Edit::replacement(body, span))))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_command_substitution_condition_commands() {
        let source = "\
#!/bin/bash
$(false) && echo x
! $(false)
$(false)
if $(python3 -c 'import sys' 2>/dev/null); then echo ok; fi
while $(false); do break; done
until $(false); do break; done
if ! $(false); then echo no; fi
if foo && $(false); then :; fi
if $(false); echo ok; then :; fi
if $(false) | cat; then :; fi
if cat | $(false); then :; fi
if { $(false); }; then :; fi
if ( $(false) ); then :; fi
if time $(false); then :; fi
echo \"$(if $(false); then :; fi)\"
cat <(if $(false); then :; fi)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfDollarCommand));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(false)",
                "$(false)",
                "$(false)",
                "$(python3 -c 'import sys' 2>/dev/null)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
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
$(false) --arg
if foo; then :; fi
if \"$(printf '%s' foo)\"; then :; fi
if [[ \"$pm\" == apt ]] && \"$(printf '%s' missing)\" != installed; then :; fi
if $(command -v rvm) -v > /dev/null 2>&1; then :; fi
if $(tc-getSTRIP) --enable-deterministic-archives |& grep -q aarch; then :; fi
if command $(false); then :; fi
if env FOO=1 $(false); then :; fi
if `printf '%s' ok`; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfDollarCommand));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_by_unwrapping_command_substitution_condition() {
        let source = "#!/bin/bash\nif $(false); then echo ok; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IfDollarCommand),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\nif false; then echo ok; fi\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
