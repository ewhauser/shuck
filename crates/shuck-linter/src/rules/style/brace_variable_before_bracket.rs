use crate::{Checker, Rule, ShellDialect, Violation};

pub struct BraceVariableBeforeBracket;

impl Violation for BraceVariableBeforeBracket {
    fn rule() -> Rule {
        Rule::BraceVariableBeforeBracket
    }

    fn message(&self) -> String {
        "brace variable expansions before adjacent `[` text".to_owned()
    }
}

pub fn brace_variable_before_bracket(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    checker.report_all_dedup(
        checker
            .facts()
            .brace_variable_before_bracket_spans()
            .to_vec(),
        || BraceVariableBeforeBracket,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

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
}
