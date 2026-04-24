use crate::{Checker, Rule, Violation};

pub struct FunctionInAlias;

impl Violation for FunctionInAlias {
    fn rule() -> Rule {
        Rule::FunctionInAlias
    }

    fn message(&self) -> String {
        "avoid positional parameters in alias definitions".to_owned()
    }
}

pub fn function_in_alias(checker: &mut Checker) {
    let spans = checker.facts().function_in_alias_spans().to_vec();
    checker.report_all_dedup(spans, || FunctionInAlias);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_positional_parameters_embedded_in_alias_definitions() {
        let source = "\
#!/bin/sh
alias first='echo $1'
alias rest='printf \"%s\\n\" \"$@\"'
alias conditional='echo ${1+\"$@\"}'
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
alias double=\"echo $1\"
alias -p
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert!(diagnostics.is_empty());
    }
}
