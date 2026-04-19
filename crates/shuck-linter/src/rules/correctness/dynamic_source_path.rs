use shuck_semantic::{SourceRefDiagnosticClass, SourceRefKind};

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
        if matches!(source_ref.kind, SourceRefKind::DirectiveDevNull) {
            continue;
        }
        if matches!(
            source_ref.diagnostic_class,
            SourceRefDiagnosticClass::DynamicPath
        ) {
            checker.report(DynamicSourcePath, source_ref.path_span);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn flags_escaped_source_builtins_with_dynamic_paths() {
        let source = "\
#!/bin/bash
\\. \"$rvm_environments_path/$1\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DynamicSourcePath));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$rvm_environments_path/$1\""
        );
    }
}
