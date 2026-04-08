use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct TrapStringExpansion;

impl Violation for TrapStringExpansion {
    fn rule() -> Rule {
        Rule::TrapStringExpansion
    }

    fn message(&self) -> String {
        "double-quoted trap handlers expand variables when the trap is set".to_owned()
    }
}

pub fn trap_string_expansion(checker: &mut Checker) {
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::TrapAction)
        .flat_map(|fact| fact.double_quoted_expansion_spans().iter().copied())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(TrapStringExpansion, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_each_expansion_inside_the_trap_action() {
        let source = "trap \"echo $x $(date) ${y}\" EXIT\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)", "${y}"]
        );
    }

    #[test]
    fn ignores_trap_listing_modes() {
        let source = "trap -p EXIT\ntrap -l TERM\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_expansions_inside_mixed_quoted_trap_words() {
        let source = "trap foo\"$x\"bar\"$(date)\" EXIT\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)"]
        );
    }
}
