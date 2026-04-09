use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionKeyword;

impl Violation for FunctionKeyword {
    fn rule() -> Rule {
        Rule::FunctionKeyword
    }

    fn message(&self) -> String {
        "`function` is not portable in `sh` scripts".to_owned()
    }
}

pub fn function_keyword(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .function_headers()
        .iter()
        .filter(|header| header.uses_function_keyword())
        .map(|header| header.span_in_source(checker.source()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || FunctionKeyword);
}
