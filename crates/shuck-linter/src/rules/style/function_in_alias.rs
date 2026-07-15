use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct FunctionInAlias;

impl Violation for FunctionInAlias {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::FunctionInAlias
    }

    fn message(&self) -> String {
        "avoid positional parameters in alias definitions".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the alias with a function".to_owned())
    }
}

pub fn function_in_alias(checker: &mut Checker) {
    checker.report_fact_diagnostics_dedup(|facts, report| {
        for fact in facts.command_facts().function_in_alias_facts() {
            let diagnostic = Diagnostic::new(FunctionInAlias, fact.span());
            let diagnostic = match fact.replacement() {
                Some((span, replacement)) => {
                    diagnostic.with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span)))
                }
                None => diagnostic,
            };
            report(diagnostic);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_positional_parameters_embedded_in_alias_definitions() {
        let source = "\
#!/bin/sh
alias first='echo $1'
alias rest='printf \"%s\\n\" \"$@\"'
alias conditional='echo ${1+\"$@\"}'
alias escaped_then_pos='echo \\$$1'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "first='echo $1'",
                "rest='printf \"%s\\n\" \"$@\"'",
                "conditional='echo ${1+\"$@\"}'",
                "escaped_then_pos='echo \\$$1'",
            ]
        );
    }

    #[test]
    fn ignores_aliases_without_static_positional_parameters() {
        let source = "\
#!/bin/sh
alias foo=$BAR
alias bar='$(printf hi)'
alias baz='noglob gtl'
alias brace='echo {a,b}'
alias func='helper() { echo hi; }'
alias literal='echo \\$1'
alias literal_braced='echo \\${1}'
alias quoted='echo '\"'\"'$1'\"'\"''
alias pid='echo $$1'
alias double=\"echo $1\"
alias -p
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_replace_alias_with_function() {
        let source = "#!/bin/sh\nalias greet='printf \"%s\\n\" \"$1\"'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FunctionInAlias),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ngreet() { printf \"%s\\n\" \"$1\"; }\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_unsafe_alias_command_shapes_unfixed() {
        let source = "#!/bin/sh\nbuiltin alias wrapped='echo $1'\nalias first='echo $1' second='echo $2'\nalias foo-bar='echo $1'\nalias greet='echo $1 # note'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 5);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn leaves_incomplete_operator_alias_bodies_unfixed() {
        let source = "#!/bin/sh\nalias greet='echo \"$1\" &&'\nalias pipe='echo \"$1\" |'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S057.sh").as_path(),
            &LinterSettings::for_rule(Rule::FunctionInAlias),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("S057_fix_S057.sh", result);
        Ok(())
    }
}
