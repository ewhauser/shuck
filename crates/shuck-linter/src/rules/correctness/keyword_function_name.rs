use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct KeywordFunctionName;

impl Violation for KeywordFunctionName {
    fn rule() -> Rule {
        Rule::KeywordFunctionName
    }

    fn message(&self) -> String {
        "function keyword without trailing `()` is not portable in `sh` scripts".to_owned()
    }
}

pub fn keyword_function_name(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .function_headers()
        .iter()
        .filter(|header| header.uses_function_keyword() && !header.has_trailing_parens())
        .map(|header| header.function_span_in_source(checker.source()))
        .collect::<Vec<Span>>();

    checker.report_all_dedup(spans, || KeywordFunctionName);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_function_keyword_without_parens_in_sh() {
        let source = "\
#!/bin/sh
function plain { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::KeywordFunctionName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::KeywordFunctionName);
        assert_eq!(diagnostics[0].span.slice(source), "function plain { :; }");
    }

    #[test]
    fn ignores_function_keyword_with_trailing_parens_in_sh() {
        let source = "\
#!/bin/sh
function plain() { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::KeywordFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_function_keyword_in_bash() {
        let source = "\
#!/bin/bash
function plain { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::KeywordFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
