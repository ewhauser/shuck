use shuck_semantic::{SourceRefKind, SourceRefResolution};

use crate::{Checker, Rule, Violation};

pub struct UntrackedSourceFile;

impl Violation for UntrackedSourceFile {
    fn rule() -> Rule {
        Rule::UntrackedSourceFile
    }

    fn message(&self) -> String {
        "sourced file is not available to this analysis".to_owned()
    }
}

pub fn untracked_source_file(checker: &mut Checker) {
    for source_ref in checker.semantic().source_refs() {
        if source_ref.explicitly_provided {
            continue;
        }

        let report = match (&source_ref.kind, source_ref.resolution) {
            (SourceRefKind::DirectiveDevNull, _) => false,
            (_, SourceRefResolution::Resolved) => true,
            (SourceRefKind::Literal(_) | SourceRefKind::Directive(_), _) => true,
            _ => false,
        };

        if !report {
            continue;
        }

        checker.report(UntrackedSourceFile, source_ref.path_span);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use tempfile::tempdir;

    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule, lint_file_at_path_with_resolver};

    #[test]
    fn reports_missing_literal_source() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. ./missing.sh\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "./missing.sh");
    }

    #[test]
    fn ignores_resolved_literal_source() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "#!/bin/sh\n. ./helper.sh\n";
        fs::write(&helper, "echo ok\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_resolved_literal_source_when_helper_is_not_an_input() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "#!/bin/sh\n. ./helper.sh\n";
        fs::write(&helper, "echo ok\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "./helper.sh");
    }

    #[test]
    fn ignores_dynamic_sources_that_belong_to_c002() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "#!/bin/sh\n. \"$helper\"\n";

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_resolved_dynamic_source_when_helper_is_not_an_input() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("tests/main.sh");
        let helper = temp.path().join("scripts/rvm");
        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let source = "#!/bin/sh\nsource \"$rvm_path/scripts/rvm\"\n";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "echo helper\n").unwrap();

        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);

        let main_path = main.clone();
        let helper_path = helper.clone();
        let resolver = move |source_path: &Path, candidate: &str| {
            if source_path == main_path.as_path() && candidate == "scripts/rvm" {
                vec![helper_path.clone()]
            } else {
                Vec::new()
            }
        };

        let diagnostics = lint_file_at_path_with_resolver(
            &output.file,
            source,
            &indexer,
            &LinterSettings::for_rule(Rule::UntrackedSourceFile),
            None,
            Some(&main),
            Some(&resolver),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"$rvm_path/scripts/rvm\""
        );
    }
}
