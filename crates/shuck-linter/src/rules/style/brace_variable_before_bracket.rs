use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct BraceVariableBeforeBracket;

impl Violation for BraceVariableBeforeBracket {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::BraceVariableBeforeBracket
    }

    fn message(&self) -> String {
        "brace variable expansions before adjacent `[` text".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("wrap the variable expansion in braces".to_owned())
    }
}

pub fn brace_variable_before_bracket(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .brace_variable_before_bracket_spans()
        .iter()
        .copied()
        .map(|span| {
            let diagnostic = Diagnostic::new(BraceVariableBeforeBracket, span);
            match brace_variable_fix(span, source) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            }
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn brace_variable_fix(span: Span, source: &str) -> Option<Fix> {
    let offset = span.start.offset;
    let tail = source.get(offset..)?;
    let rest = tail.strip_prefix('$')?;
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }

    let mut name_len = first.len_utf8();
    for (index, ch) in chars {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            name_len = index + ch.len_utf8();
        } else {
            break;
        }
    }

    Some(Fix::safe_edits([
        Edit::insertion(offset + '$'.len_utf8(), "{"),
        Edit::insertion(offset + '$'.len_utf8() + name_len, "}"),
    ]))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_unbraced_variables_before_bracket_text() {
        let source = "\
#!/bin/sh
echo \"$foo[0]\"
echo \"$key[[:space:]]\"
echo game$game[0]
echo \"$foo[\"
$cmd[0] arg
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceVariableBeforeBracket),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 7), (3, 7), (4, 10), (5, 7), (6, 1)]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.start == diagnostic.span.end)
        );
    }

    #[test]
    fn ignores_braced_special_and_quote_split_forms() {
        let source = "\
#!/bin/sh
echo \"${foo}[0]\"
echo \"${foo}[[:space:]]\"
echo \"$foo\"\"[0]\"
echo \"$foo\"'[0]'
echo \"$foo\\[0]\"
echo \"$1[0]\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceVariableBeforeBracket),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_corpus_style_regex_suffix_forms() {
        let source = "\
#!/bin/bash
check() {
  local cmd=\"$1\"
  command git grep -E \"^[^#]*\\\\<$cmd[[:space:]]+\"
}
sed_var() {
  sed -i \"/\\\\$symon['$1']/s|=.*|='$2';|\" setup.inc
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceVariableBeforeBracket),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(4, 33), (7, 14)]
        );
    }

    #[test]
    fn ignores_zsh_array_subscripts() {
        let source = "\
#!/bin/zsh
echo \"$foo[1]\"
print $reply[2]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceVariableBeforeBracket)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_by_bracing_variable_expansions() {
        let source = "\
#!/bin/sh
echo \"$foo[0]\"
echo game$game[0]
$cmd[0] arg
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BraceVariableBeforeBracket),
            Applicability::Safe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo \"${foo}[0]\"
echo game${game}[0]
${cmd}[0] arg
"
        );
        assert_eq!(result.fixes_applied, 3);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S077.sh").as_path(),
            &LinterSettings::for_rule(Rule::BraceVariableBeforeBracket),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S077_fix_S077.sh", result);
        Ok(())
    }
}
