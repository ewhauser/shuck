use crate::facts::EscapeScanSourceKind;
use crate::{Checker, Rule, Violation};

pub struct EscapedUnderscoreLiteral;

impl Violation for EscapedUnderscoreLiteral {
    fn rule() -> Rule {
        Rule::EscapedUnderscoreLiteral
    }

    fn message(&self) -> String {
        "a backslash before `_` is unnecessary and becomes a literal underscore".to_owned()
    }
}

pub fn escaped_underscore_literal(checker: &mut Checker) {
    let spans = checker
        .facts()
        .escape_scan_matches()
        .iter()
        .copied()
        .filter(|escape| match escape.source_kind() {
            EscapeScanSourceKind::WordLiteralPart
            | EscapeScanSourceKind::RedirectLiteralSegment => {
                !escape.host_contains_single_quoted_fragment()
            }
            EscapeScanSourceKind::DynamicPathCommandName
            | EscapeScanSourceKind::PatternLiteral
            | EscapeScanSourceKind::PatternCharClass => true,
            EscapeScanSourceKind::SingleLiteralAssignmentWord
            | EscapeScanSourceKind::BacktickFragment => false,
        })
        .filter(|escape| match escape.source_kind() {
            EscapeScanSourceKind::PatternCharClass => escape.escaped_byte() == b'-',
            _ => escape.escaped_byte() == b'_',
        })
        .map(|escape| escape.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EscapedUnderscoreLiteral);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_indexer::Indexer;
    use shuck_parser::parser::{ParseOutput, Parser, ShellDialect as ParseDialect};

    use crate::test::test_snippet;
    use crate::{
        Diagnostic, LinterSettings, Rule, ShellDialect, lint_file_at_path_with_parse_diagnostics,
    };

    fn test_posix_snippet_at_path(path: &Path, source: &str) -> Vec<Diagnostic> {
        let recovered = Parser::with_dialect(source, ParseDialect::Posix).parse_recovered();
        let output = ParseOutput {
            file: recovered.file,
        };
        let indexer = Indexer::new(source, &output);
        let settings =
            LinterSettings::for_rule(Rule::EscapedUnderscoreLiteral).with_shell(ShellDialect::Sh);
        lint_file_at_path_with_parse_diagnostics(
            &output.file,
            source,
            &indexer,
            &settings,
            None,
            Some(path),
            &recovered.diagnostics,
        )
    }

    #[test]
    fn reports_needless_backslashes_before_underscores() {
        let source = "\
#!/bin/bash
echo foo\\_bar
echo foo\\\\_bar
echo \"foo\\_bar\"
echo prefix\"\\_\"suffix
foo=${x#foo\\_bar}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedUnderscoreLiteral),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![""]
        );
    }

    #[test]
    fn reports_redirect_target_underscores() {
        let source = "\
base64 -d ${vkb64} > ${rootfs}/var/db/xbps/keys/60\\:ae\\:0c\\:d6\\:f0\\:95\\:17\\:80\\:bc\\:93\\:46\\:7a\\:89\\:af\\:a3\\:2d.plist
";
        let diagnostics = test_posix_snippet_at_path(Path::new("/tmp/lxc-void"), source);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_dynamic_path_like_command_names() {
        let source = "\
#!/bin/bash
${bindir}/foo\\_bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedUnderscoreLiteral),
        );

        assert_eq!(diagnostics.len(), 1);
    }
}
