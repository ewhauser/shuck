use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct HyphenatedFunctionName;

impl Violation for HyphenatedFunctionName {
    fn rule() -> Rule {
        Rule::HyphenatedFunctionName
    }

    fn message(&self) -> String {
        "function names cannot contain hyphens in `sh` scripts".to_owned()
    }
}

pub fn hyphenated_function_name(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .function_headers()
        .iter()
        .flat_map(|header| {
            header.entries().iter().filter_map(|entry| {
                let name = entry
                    .static_name()
                    .map(|name| name.as_str())
                    .unwrap_or_else(|| entry.word_span().slice(source));
                name.contains('-')
                    .then_some(header.function_span_in_source(source))
            })
        })
        .collect::<Vec<Span>>();

    checker.report_all_dedup(spans, || HyphenatedFunctionName);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_hyphenated_function_names_in_sh() {
        let source = "\
#!/bin/sh
my-func() { :; }
function other-func { :; }
function third-func() { :; }
my-func
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "my-func() { :; }",
                "function other-func { :; }",
                "function third-func() { :; }"
            ]
        );
    }

    #[test]
    fn ignores_non_hyphenated_function_names() {
        let source = "\
#!/bin/sh
my_func() { :; }
function other_func { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_posix_hyphenated_function_names() {
        let source = "\
#!/bin/sh
my-func() { :; }
termux_run_build-package() { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["my-func() { :; }", "termux_run_build-package() { :; }"]
        );
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nmy-func() { :; }\n";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
