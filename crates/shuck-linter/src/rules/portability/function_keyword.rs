use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionKeyword;

impl Violation for FunctionKeyword {
    fn rule() -> Rule {
        Rule::FunctionKeyword
    }

    fn message(&self) -> String {
        "`function` is not portable in `sh` scripts".to_owned()
    }
}

pub fn function_keyword(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .function_headers()
        .iter()
        .filter(|header| header.uses_function_keyword() && !header.has_trailing_parens())
        .map(|header| header.function_span_in_source(checker.source()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || FunctionKeyword);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_function_keyword_without_parens_over_full_function_span() {
        let source = "\
#!/bin/sh
function greet
{
  printf '%s\\n' hi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionKeyword));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "function greet\n{\n  printf '%s\\n' hi\n}"
        );
    }
}
