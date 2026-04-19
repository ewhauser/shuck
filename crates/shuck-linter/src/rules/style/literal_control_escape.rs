use crate::facts::EscapeScanSourceKind;
use crate::{Checker, Rule, Violation};

pub struct LiteralControlEscape;

impl Violation for LiteralControlEscape {
    fn rule() -> Rule {
        Rule::LiteralControlEscape
    }

    fn message(&self) -> String {
        "shell words treat \\n, \\r, and \\t as plain text".to_owned()
    }
}

pub fn literal_control_escape(checker: &mut Checker) {
    let spans = checker
        .facts()
        .escape_scan_matches()
        .iter()
        .copied()
        .filter(|escape| {
            matches!(
                escape.source_kind(),
                EscapeScanSourceKind::WordLiteralPart
                    | EscapeScanSourceKind::RedirectLiteralSegment
                    | EscapeScanSourceKind::DynamicPathCommandName
                    | EscapeScanSourceKind::PatternLiteral
                    | EscapeScanSourceKind::PatternCharClass
                    | EscapeScanSourceKind::SingleLiteralAssignmentWord
                    | EscapeScanSourceKind::BacktickFragment
            )
        })
        .filter(|escape| !escape.inside_single_quoted_fragment())
        .filter(|escape| matches!(escape.escaped_byte(), b'n' | b'r' | b't'))
        .map(|escape| escape.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LiteralControlEscape);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_literal_control_escapes_in_plain_words() {
        let source = "\
#!/bin/sh
echo \\n
echo foo\\nbar
foo=bar\\r
case x in foo\\t) : ;; esac
cat < foo\\nbar
`echo \\n`
command \\rm file
grep \\t file
foo=$(echo \\t)
${bindir}/foo\\tbar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralControlEscape),
        );

        assert_eq!(diagnostics.len(), 10);
    }

    #[test]
    fn ignores_quoted_escapes_and_bare_escaped_command_names() {
        let source = "\
#!/bin/sh
\\rm file
echo \"\\n\"
echo '\\n'
printf '%s\\n' \"\\t\"
cat < \"\\n\"
ALL_JARS=`ls *.jar | tr \"\\n\" \" \"`
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralControlEscape),
        );

        assert!(diagnostics.is_empty());
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
            &LinterSettings::for_rule(Rule::LiteralControlEscape),
        );

        assert!(diagnostics.is_empty());
    }
}
