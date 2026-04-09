use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionKeywordInSh;

impl Violation for FunctionKeywordInSh {
    fn rule() -> Rule {
        Rule::FunctionKeywordInSh
    }

    fn message(&self) -> String {
        "`function` with trailing `()` is not portable in `sh` scripts".to_owned()
    }
}

pub fn function_keyword_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .function_headers()
        .iter()
        .filter(|header| header.uses_function_keyword() && header.has_trailing_parens())
        .map(|header| header.span_in_source(checker.source()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || FunctionKeywordInSh);
}
