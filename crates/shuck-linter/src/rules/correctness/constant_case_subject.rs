use crate::{Checker, Rule, Violation};

pub struct ConstantCaseSubject;

impl Violation for ConstantCaseSubject {
    fn rule() -> Rule {
        Rule::ConstantCaseSubject
    }

    fn message(&self) -> String {
        "this `case` statement switches on a fixed literal".to_owned()
    }
}

pub fn constant_case_subject(checker: &mut Checker) {
    let spans = checker
        .facts()
        .case_subject_facts()
        .filter(|fact| fact.classification().is_fixed_literal())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || ConstantCaseSubject);
}
