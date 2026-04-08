use crate::{Checker, Rule, Violation};

pub struct LineOrientedInput;

impl Violation for LineOrientedInput {
    fn rule() -> Rule {
        Rule::LineOrientedInput
    }

    fn message(&self) -> String {
        "iterating over command output in a `for` loop splits lines on whitespace".to_owned()
    }
}

pub fn line_oriented_input(checker: &mut Checker) {
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .flat_map(|header| header.words().iter())
        .filter(|word| word.has_command_substitution())
        .map(|word| word.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || LineOrientedInput);
}
