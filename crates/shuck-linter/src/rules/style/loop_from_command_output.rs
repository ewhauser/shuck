use crate::{Checker, Rule, Violation};

pub struct LoopFromCommandOutput;

impl Violation for LoopFromCommandOutput {
    fn rule() -> Rule {
        Rule::LoopFromCommandOutput
    }

    fn message(&self) -> String {
        "iterating over command output is fragile; use globs, arrays, or explicit delimiters"
            .to_owned()
    }
}

pub fn loop_from_command_output(checker: &mut Checker) {
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .filter(|header| !header.is_nested_word_command())
        .flat_map(|header| header.words().iter())
        .filter(|word| {
            word.has_unquoted_command_substitution() && word.contains_ls_substitution()
        })
        .map(|word| word.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || LoopFromCommandOutput);
}
