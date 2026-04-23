use crate::{Checker, Rule, Violation};

use super::source_common::{SourceScopeFilter, source_command_spans_in_sh};

pub struct SourceBuiltinInSh;

impl Violation for SourceBuiltinInSh {
    fn rule() -> Rule {
        Rule::SourceBuiltinInSh
    }

    fn message(&self) -> String {
        "`source` is not portable in `sh` scripts".to_owned()
    }
}

pub fn source_builtin_in_sh(checker: &mut Checker) {
    let spans = source_command_spans_in_sh(checker, SourceScopeFilter::Any);
    checker.report_all(spans, || SourceBuiltinInSh);
}
