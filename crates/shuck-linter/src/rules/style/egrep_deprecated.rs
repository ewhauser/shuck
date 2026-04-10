use crate::{Checker, Rule, Violation};

pub struct EgrepDeprecated;

impl Violation for EgrepDeprecated {
    fn rule() -> Rule {
        Rule::EgrepDeprecated
    }

    fn message(&self) -> String {
        "use `grep -E` instead of `egrep`".to_owned()
    }
}

pub fn egrep_deprecated(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("egrep") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || EgrepDeprecated);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_egrep_invocations() {
        let source = "\
#!/bin/sh
egrep foo file
egrep
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EgrepDeprecated));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["egrep", "egrep"]
        );
    }

    #[test]
    fn ignores_wrapped_egrep_invocations() {
        let source = "\
#!/bin/sh
command egrep foo file
sudo egrep foo file
grep -E foo file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EgrepDeprecated));

        assert!(diagnostics.is_empty());
    }
}
