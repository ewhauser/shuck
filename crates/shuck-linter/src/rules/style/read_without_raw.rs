use crate::{Checker, Rule, Violation};

pub struct ReadWithoutRaw;

impl Violation for ReadWithoutRaw {
    fn rule() -> Rule {
        Rule::ReadWithoutRaw
    }

    fn message(&self) -> String {
        "use `read -r` to keep backslashes literal".to_owned()
    }
}

pub fn read_without_raw(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| fact.literal_name() == Some("read"))
        .filter(|fact| fact.read_uses_raw_input() == Some(false))
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report(ReadWithoutRaw, span);
    }
}
