use crate::{Checker, Rule, Violation};

pub struct CommaArrayElements;

impl Violation for CommaArrayElements {
    fn rule() -> Rule {
        Rule::CommaArrayElements
    }

    fn message(&self) -> String {
        "separate array literal elements with whitespace, not commas".to_owned()
    }
}

pub fn comma_array_elements(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.comma_array_assignment_spans(),
        || CommaArrayElements,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_comma_separated_array_literals() {
        let source = "\
#!/bin/bash
a=(alpha,beta)
b=(alpha, beta)
c=([k]=v, [q]=w)
d+=(x,y)
e=(foo,{x,y},bar)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "(alpha,beta)",
                "(alpha, beta)",
                "([k]=v, [q]=w)",
                "(x,y)",
                "(foo,{x,y},bar)"
            ]
        );
    }

    #[test]
    fn ignores_quoted_and_brace_expansion_commas() {
        let source = "\
#!/bin/bash
a=(\"alpha,beta\")
b=('alpha,beta')
c=({x,y})
d=($(printf 'x,y'))
e=({$XDG_CONFIG_HOME,$HOME}/{alacritty,}/{.,}alacritty.ym?)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
