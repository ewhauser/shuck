use crate::{Checker, Rule, Violation};

pub struct OpenDoubleQuote;

impl Violation for OpenDoubleQuote {
    fn rule() -> Rule {
        Rule::OpenDoubleQuote
    }

    fn message(&self) -> String {
        "double-quoted string looks unterminated".to_owned()
    }
}

pub fn open_double_quote(checker: &mut Checker) {
    let spans = checker
        .facts()
        .open_double_quote_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || OpenDoubleQuote);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_suspicious_opening_double_quote() {
        let source = "#!/bin/bash\necho \"#!/bin/bash\nif [[ \"$@\" =~ x ]]; then :; fi\n\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn ignores_regular_multiline_double_quotes() {
        let source = "#!/bin/sh\necho \"line one\nline two\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_sc2140_style_multiline_quote_joins() {
        let source = "\
#!/bin/bash
echo \"[Unit]
Description=Heimdall
ExecStart=\"/usr/bin/php\" artisan serve
WantedBy=multi-user.target\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_each_reopened_quote_window_in_a_single_word() {
        let source = "\
#!/bin/bash
echo \"alpha
\"$foo\"beta
\"$bar\"gamma\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.column, 6);
    }

    #[test]
    fn reports_independent_reopened_quote_windows_across_multiple_arguments() {
        let source = "\
#!/bin/bash
echo \"a
\"$x\"b\" \"c
\"$y\"d\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.column, 8);
    }

    #[test]
    fn ignores_multiline_triple_quote_script_builders() {
        let source = "\
#!/bin/bash
echo \"\"\"#!/usr/bin/env bash
echo \"GEM_HOME FIRST: \\$GEM_HOME\"
echo \"GEM_PATH: \\$GEM_PATH\"
\"\"\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert!(diagnostics.is_empty());
    }
}
