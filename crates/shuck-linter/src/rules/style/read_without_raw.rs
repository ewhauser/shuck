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
        .structural_commands()
        .filter(|fact| fact.effective_name_is("read"))
        .filter(|fact| {
            fact.options()
                .read()
                .is_some_and(|read| !read.uses_raw_input)
        })
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report(ReadWithoutRaw, span);
    }
}
