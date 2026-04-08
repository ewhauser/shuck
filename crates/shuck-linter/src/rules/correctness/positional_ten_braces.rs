use crate::{Checker, Rule, Violation};

pub struct PositionalTenBraces;

impl Violation for PositionalTenBraces {
    fn rule() -> Rule {
        Rule::PositionalTenBraces
    }

    fn message(&self) -> String {
        "use braces for positional parameters above 9".to_owned()
    }
}

pub fn positional_ten_braces(checker: &mut Checker) {
    let spans = checker
        .facts()
        .positional_parameter_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || PositionalTenBraces);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_positional_ten_in_assignment_subscripts() {
        let source = "#!/bin/bash\narr[$10]=1\ndeclare other[$10]=1\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PositionalTenBraces));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$10", "$10"]
        );
    }
}
