use crate::{Checker, Rule, Violation};

pub struct LegacyBackticks;

impl Violation for LegacyBackticks {
    fn rule() -> Rule {
        Rule::LegacyBackticks
    }

    fn message(&self) -> String {
        "prefer `$(...)` over legacy backtick substitution".to_owned()
    }
}

pub fn legacy_backticks(checker: &mut Checker) {
    let spans = checker
        .facts()
        .backtick_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(LegacyBackticks, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_each_backtick_fragment() {
        let source = "echo \"prefix `date` suffix `uname`\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`", "`uname`"]
        );
    }
}
