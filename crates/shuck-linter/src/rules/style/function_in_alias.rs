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
    let fixable_spans = checker
        .facts()
        .function_in_alias_facts()
        .iter()
        .map(|fact| fact.span())
        .collect::<Vec<_>>();
    let diagnostics = checker
        .facts()
        .function_in_alias_facts()
        .iter()
        .map(|fact| {
            Diagnostic::new(FunctionInAlias, fact.span()).with_fix(Fix::unsafe_edit(
                Edit::replacement(fact.replacement(), fact.replacement_span()),
            ))
        })
        .chain(
            checker
                .facts()
                .function_in_alias_spans()
                .iter()
                .copied()
                .filter(|span| !fixable_spans.contains(span))
                .map(|span| Diagnostic::new(FunctionInAlias, span)),
        )
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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
    fn applies_unsafe_fix_without_double_terminating_alias_semicolon() {
        let source = "#!/bin/sh\nalias greet='echo $1;'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FunctionInAlias),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\ngreet() { echo $1; }\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn reports_non_fixable_alias_names_without_fix() {
        let source = "\
#!/bin/sh
alias foo+bar='echo $1'
alias foo-bar='echo $1'
alias foo.bar='echo $1'
alias 9foo='echo $1'
alias if='echo $1'
alias for='echo $1'
alias function='echo $1'
alias export='echo $1'
alias readonly='echo $1'
alias unset='echo $1'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "foo+bar='echo $1'",
                "foo-bar='echo $1'",
                "foo.bar='echo $1'",
                "9foo='echo $1'",
                "if='echo $1'",
                "for='echo $1'",
                "function='echo $1'",
                "export='echo $1'",
                "readonly='echo $1'",
                "unset='echo $1'",
            ]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn leaves_multi_definition_alias_commands_unfixed() {
        let source = "#!/bin/sh\nalias first='echo $1' second='echo $2'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["first='echo $1'", "second='echo $2'"]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn leaves_wrapped_alias_commands_unfixed() {
        let source = "#!/bin/sh\nbuiltin alias greet='echo $1'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet='echo $1'");
        assert!(diagnostics[0].fix.is_none());
    }

    #[test]
    fn leaves_assignment_prefixed_alias_commands_unfixed() {
        let source = "#!/bin/sh\nFOO=bar alias greet='echo $1'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet='echo $1'");
        assert!(diagnostics[0].fix.is_none());
    }

    #[test]
    fn leaves_comment_alias_bodies_unfixed() {
        let source = "#!/bin/sh\nalias greet='echo $1 # note'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet='echo $1 # note'");
        assert!(diagnostics[0].fix.is_none());
    }

    #[test]
    fn leaves_multiline_comment_alias_bodies_unfixed() {
        let source = "#!/bin/sh\nalias greet='echo $1\n# note'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet='echo $1\n# note'");
        assert!(diagnostics[0].fix.is_none());
    }

    #[test]
    fn leaves_escaped_newline_comment_alias_bodies_unfixed() {
        let source = "#!/bin/sh\nalias greet='echo $1 \\\n# note'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet='echo $1 \\\n# note'"
        );
        assert!(diagnostics[0].fix.is_none());
    }

    #[test]
    fn leaves_incomplete_operator_alias_bodies_unfixed() {
        let source = "\
#!/bin/sh
alias greet='echo \"$1\" &&'
alias pipe='printf \"%s\\n\" \"$1\" |'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["greet='echo \"$1\" &&'", "pipe='printf \"%s\\n\" \"$1\" |'",]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
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
}
