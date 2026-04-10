use crate::{Checker, Rule, Violation};

use super::source_common::{SourceScopeFilter, source_command_spans_in_sh};

pub struct SourceInsideFunctionInSh;

impl Violation for SourceInsideFunctionInSh {
    fn rule() -> Rule {
        Rule::SourceInsideFunctionInSh
    }

    fn message(&self) -> String {
        "`source` inside a function is not portable in `sh` scripts".to_owned()
    }
}

pub fn source_inside_function_in_sh(checker: &mut Checker) {
    let spans = source_command_spans_in_sh(checker, SourceScopeFilter::InsideFunction);
    checker.report_all(spans, || SourceInsideFunctionInSh);
}
