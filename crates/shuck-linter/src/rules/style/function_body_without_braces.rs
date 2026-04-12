use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionBodyWithoutBraces;

impl Violation for FunctionBodyWithoutBraces {
    fn rule() -> Rule {
        Rule::FunctionBodyWithoutBraces
    }

    fn message(&self) -> String {
        "function body should use a brace group instead of a bare compound command".to_owned()
    }
}

pub fn function_body_without_braces(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    checker.report_all_dedup(
        checker
            .facts()
            .function_body_without_braces_spans()
            .to_vec(),
        || FunctionBodyWithoutBraces,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_function_bodies_written_as_bare_compound_commands() {
        let source = "\
#!/bin/bash
f() [[ -n \"$x\" ]]
g() ( echo hi )
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionBodyWithoutBraces),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[[ -n \"$x\" ]]", "echo hi )"]
        );
    }

    #[test]
    fn ignores_braced_function_bodies() {
        let source = "\
#!/bin/bash
f() { [[ -n \"$x\" ]]; }
g() { ( echo hi ); }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionBodyWithoutBraces),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
