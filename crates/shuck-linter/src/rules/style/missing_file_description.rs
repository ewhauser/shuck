use crate::{Checker, Rule, Violation};

pub struct MissingFileDescription;

impl Violation for MissingFileDescription {
    fn rule() -> Rule {
        Rule::MissingFileDescription
    }

    fn message(&self) -> String {
        "add a file header comment describing the script".to_owned()
    }
}

pub fn missing_file_description(checker: &mut Checker) {
    let Some(fact) = checker
        .facts()
        .source_facts()
        .missing_file_description_comment()
    else {
        return;
    };

    if fact.is_shebang_only_file() && checker.rule_options().s081.ignore_shebang_only_files {
        return;
    }

    checker.report(MissingFileDescription, fact.span());
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_code_immediately_after_shebang() {
        let source = "#!/bin/bash\nls -la\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "ls -la");
    }

    #[test]
    fn reports_code_after_shebang_with_leading_blank_lines() {
        let source = "\n#!/bin/bash\nls -la\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "ls -la");
    }

    #[test]
    fn accepts_comment_block_after_shebang() {
        let source = "#!/bin/bash\n# Lists the current directory.\nls -la\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn accepts_comment_block_after_shebang_and_blank_lines() {
        let source = "#!/bin/bash\n\n# Lists the current directory.\nls -la\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_code_first_files_without_shebang() {
        let source = "ls -la\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "ls -la");
    }

    #[test]
    fn accepts_comment_first_files_without_shebang() {
        let source = "# Lists the current directory.\nls -la\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_shebang_only_files_by_default() {
        let source = "#!/bin/bash\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "#!/bin/bash");
    }

    #[test]
    fn can_ignore_shebang_only_files() {
        let source = "#!/bin/bash\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFileDescription)
                .with_s081_ignore_shebang_only_files(true),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
