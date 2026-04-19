use crate::{Checker, Rule, Violation};

pub struct InvalidExitStatus;

impl Violation for InvalidExitStatus {
    fn rule() -> Rule {
        Rule::InvalidExitStatus
    }

    fn message(&self) -> String {
        "`exit` expects a numeric status".to_owned()
    }
}

pub fn invalid_exit_status(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.options().exit().copied())
        .filter(|exit| exit.has_invalid_status_argument())
        .filter_map(|exit| exit.status_word.map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || InvalidExitStatus);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_only_static_non_numeric_exit_values() {
        let source = "exit 42\nexit nope\nexit \"$status\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::InvalidExitStatus));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "nope");
    }

    #[test]
    fn reports_empty_and_mixed_literal_exit_values() {
        let source = "\
code=3
exit \"\"
exit \"123$code\"
exit \"$code\"
exit \"${code}$(printf 4)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::InvalidExitStatus));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"\"", "\"123$code\""]
        );
    }
}
