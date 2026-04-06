use shuck_semantic::SourceRefKind;

use crate::{Checker, Rule, Violation};

pub struct DynamicSourcePath;

impl Violation for DynamicSourcePath {
    fn rule() -> Rule {
        Rule::DynamicSourcePath
    }

    fn message(&self) -> String {
        "source path is built at runtime".to_owned()
    }
}

pub fn dynamic_source_path(checker: &mut Checker) {
    for source_ref in checker.semantic().source_refs() {
        if matches!(
            source_ref.kind,
            SourceRefKind::Dynamic | SourceRefKind::SingleVariableStaticTail { .. }
        ) {
            checker.report(DynamicSourcePath, source_ref.path_span);
        }
    }
}
