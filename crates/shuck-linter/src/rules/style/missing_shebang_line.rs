use crate::{Checker, Rule, ShellDialect, Violation};

pub struct MissingShebangLine;

impl Violation for MissingShebangLine {
    fn rule() -> Rule {
        Rule::MissingShebangLine
    }

    fn message(&self) -> String {
        "add a shebang as the first line of the file".to_owned()
    }
}

pub fn missing_shebang_line(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Unknown {
        return;
    }

    if let Some(span) = checker.facts().missing_shebang_line_span() {
        checker.report(MissingShebangLine, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_comment_first_line_without_shebang() {
        let source = "# /etc/config/myapp.conf\necho hello\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MissingShebangLine));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "# /etc/config/myapp.conf"
        );
    }

    #[test]
    fn ignores_real_or_malformed_shebang_headers_and_directives() {
        for source in [
            "#!/bin/sh\necho hello\n",
            " #!/bin/sh\necho hello\n",
            "# !/bin/sh\necho hello\n",
            "# comment\n#!/bin/sh\n",
            "# shellcheck shell=sh\necho hello\n",
            "\n# comment\necho hello\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::MissingShebangLine));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn ignores_when_shell_is_known_from_context() {
        let source = "# comment\necho hello\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingShebangLine).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_indented_comment_first_line_without_shebang() {
        let source = " # comment\necho hello\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MissingShebangLine));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), " # comment");
    }
}
