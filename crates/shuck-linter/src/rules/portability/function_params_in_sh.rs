use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionParamsInSh;

impl Violation for FunctionParamsInSh {
    fn rule() -> Rule {
        Rule::FunctionParamsInSh
    }

    fn message(&self) -> String {
        "function definitions cannot take parameters in `sh` scripts".to_owned()
    }
}

pub fn function_params_in_sh(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    checker.report_all_dedup(
        checker.facts().function_parameter_fallback_spans().to_vec(),
        || FunctionParamsInSh,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_function_parameter_syntax_in_sh() {
        let source = "\
#!/bin/sh
f(x) { :; }
function g(y) { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["(", "("]
        );
    }

    #[test]
    fn ignores_standard_function_definitions() {
        let source = "\
#!/bin/sh
f() { :; }
function g() { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_function_parameter_syntax_with_subshell_body_in_sh() {
        let source = "\
#!/bin/sh
f(x) ( : )
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn reports_function_parameter_syntax_when_body_starts_on_next_line() {
        let source = "\
#!/bin/sh
f(x)
{ :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn reports_function_parameter_syntax_for_hyphenated_names() {
        let source = "\
#!/bin/sh
my-func(x) { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn reports_function_parameter_syntax_when_comment_precedes_body() {
        let source = "\
#!/bin/sh
f(x) # note
{ :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn reports_function_parameter_syntax_when_line_continuation_precedes_body() {
        let source = "\
#!/bin/sh
f(x) \\
{ :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn ignores_empty_subshell_bodies() {
        let source = "\
#!/bin/sh
wget() { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nf(x) { :; }\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionParamsInSh).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_named_coproc_syntax() {
        let source = "\
#!/bin/sh
coproc pycoproc (python3 \"$pywrapper\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
