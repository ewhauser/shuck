use crate::facts::EscapeScanSourceKind;
use crate::{Checker, Rule, Violation};

pub struct NeedlessBackslashUnderscore;

impl Violation for NeedlessBackslashUnderscore {
    fn rule() -> Rule {
        Rule::NeedlessBackslashUnderscore
    }

    fn message(&self) -> String {
        "a backslash before n, r, or t is literal".to_owned()
    }
}

pub fn needless_backslash_underscore(checker: &mut Checker) {
    let spans = checker
        .facts()
        .escape_scan_matches()
        .iter()
        .copied()
        .filter(|escape| match escape.source_kind() {
            EscapeScanSourceKind::WordLiteralPart => {
                !escape.is_nested_word_command() && !escape.host_contains_single_quoted_fragment()
            }
            EscapeScanSourceKind::RedirectLiteralSegment
            | EscapeScanSourceKind::SingleLiteralAssignmentWord => {
                !escape.host_contains_single_quoted_fragment()
            }
            EscapeScanSourceKind::DynamicPathCommandName
            | EscapeScanSourceKind::PatternLiteral
            | EscapeScanSourceKind::PatternCharClass
            | EscapeScanSourceKind::BacktickFragment => true,
        })
        .filter(|escape| !escape.inside_single_quoted_fragment())
        .filter(|escape| matches!(escape.escaped_byte(), b'n' | b'r' | b't'))
        .map(|escape| escape.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || NeedlessBackslashUnderscore);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_needless_backslashes_before_newline_style_letters() {
        let source = "\
#!/bin/sh
echo \\n
echo foo\\nbar
case x in foo\\t) : ;; esac
cat < foo\\nbar
`echo \\n`
echo \"\\n\"
echo '\\n'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NeedlessBackslashUnderscore),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["", "", "", "", ""]
        );
    }

    #[test]
    fn ignores_single_quoted_fragments_inside_nested_command_substitutions() {
        let source = "\
#!/bin/bash
if [[ \"$TERMUX_APP_PACKAGE_MANAGER\" == \"apt\" ]] && \"$(dpkg-query -W -f '${db:Status-Status}\\n' cabal-install 2>/dev/null)\" != \"installed\"; then
  :
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NeedlessBackslashUnderscore),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_dynamic_path_like_command_names() {
        let source = "\
#!/bin/bash
${bindir}/foo\\nbar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NeedlessBackslashUnderscore),
        );

        assert_eq!(diagnostics.len(), 1);
    }
}
