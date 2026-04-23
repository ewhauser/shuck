use shuck_ast::{DeclOperand, static_word_text};

use crate::{Checker, Rule, Violation};

pub struct SpacedAssignment;

impl Violation for SpacedAssignment {
    fn rule() -> Rule {
        Rule::SpacedAssignment
    }

    fn message(&self) -> String {
        "remove spaces around `=` in this assignment".to_owned()
    }
}

pub fn spaced_assignment(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| fact.declaration())
        .flat_map(|declaration| {
            declaration
                .operands
                .windows(2)
                .filter_map(|pair| spaced_assignment_span(pair, source))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SpacedAssignment);
}

fn spaced_assignment_span(pair: &[DeclOperand], source: &str) -> Option<shuck_ast::Span> {
    let [DeclOperand::Name(_), DeclOperand::Dynamic(word)] = pair else {
        return None;
    };

    static_word_text(word, source)?
        .starts_with('=')
        .then_some(word.span)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_stray_equals_word() {
        let source = "\
#!/bin/sh
export foo =bar
readonly bar = baz
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpacedAssignment));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["=bar", "="]
        );
    }

    #[test]
    fn ignores_tight_assignments_and_plain_commands() {
        let source = "\
#!/bin/sh
export foo=bar
foo =bar
foo= bar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpacedAssignment));

        assert!(diagnostics.is_empty());
    }
}
