use crate::{Checker, Rule, Violation};

pub struct FgrepDeprecated;

impl Violation for FgrepDeprecated {
    fn rule() -> Rule {
        Rule::FgrepDeprecated
    }

    fn message(&self) -> String {
        "use `grep -F` instead of `fgrep`".to_owned()
    }
}

pub fn fgrep_deprecated(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("fgrep") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || FgrepDeprecated);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_fgrep_invocations() {
        let source = "\
#!/bin/sh
fgrep foo file
fgrep
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FgrepDeprecated));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["fgrep", "fgrep"]
        );
    }

    #[test]
    fn ignores_wrapped_fgrep_invocations() {
        let source = "\
#!/bin/sh
command fgrep foo file
sudo fgrep foo file
grep -F foo file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FgrepDeprecated));

        assert!(diagnostics.is_empty());
    }
}
