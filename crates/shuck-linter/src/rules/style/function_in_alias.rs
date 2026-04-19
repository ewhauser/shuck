use crate::{Checker, Rule, Violation};

pub struct FunctionInAlias;

impl Violation for FunctionInAlias {
    fn rule() -> Rule {
        Rule::FunctionInAlias
    }

    fn message(&self) -> String {
        "avoid defining functions inside alias strings".to_owned()
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
    fn reports_function_definitions_embedded_in_alias_strings() {
        let source = "\
#!/bin/sh
alias gtl='gtl(){ git tag --sort=-v:refname -n -l \"${1}*\" }; noglob gtl'
alias h='function h { echo hi; }'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "gtl='gtl(){ git tag --sort=-v:refname -n -l \"${1}*\" }; noglob gtl'",
                "h='function h { echo hi; }'",
            ]
        );
    }

    #[test]
    fn ignores_non_definition_alias_expansions() {
        let source = "\
#!/bin/sh
alias foo=$BAR
alias bar='$(printf hi)'
alias baz='noglob gtl'
alias brace='echo {a,b}'
alias not_fn='helper { echo hi; }'
alias positional='${1+\"$@\"}'
alias -p
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert!(diagnostics.is_empty());
    }
}
