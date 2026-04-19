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
        .map(|header| header.function_span_in_source(checker.source()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || FunctionKeywordInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_function_keyword_with_parens_over_full_function_span() {
        let source = "\
#!/bin/sh
function greet()
{
  printf '%s\\n' hi
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::FunctionKeywordInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "function greet()\n{\n  printf '%s\\n' hi\n}"
        );
    }

    #[test]
    fn ignores_function_keyword_without_trailing_parens() {
        let source = "\
#!/bin/sh
function greet { :; }
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::FunctionKeywordInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_non_sh_shells() {
        let source = "\
#!/bin/bash
function greet() { :; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
