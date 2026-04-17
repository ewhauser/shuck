use crate::{Checker, Rule, Violation};

pub struct IfsSetToLiteralBackslashN;

impl Violation for IfsSetToLiteralBackslashN {
    fn rule() -> Rule {
        Rule::IfsSetToLiteralBackslashN
    }

    fn message(&self) -> String {
        "backslashes in IFS stay literal".to_owned()
    }
}

pub fn ifs_set_to_literal_backslash_n(checker: &mut Checker) {
    checker.report_all_dedup(
        checker
            .facts()
            .ifs_literal_backslash_assignment_value_spans()
            .to_vec(),
        || IfsSetToLiteralBackslashN,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_ifs_assignment_values() {
        let source = "\
#!/bin/bash
IFS='\\n'
export IFS=\"x\\n\"
while IFS='\\ \\|\\ ' read -r serial board_serial; do
  :
done < /dev/null
foo() {
  local IFS='\\n\\t'
}
declare IFS='prefix\\nsuffix'
bar='\\n'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IfsSetToLiteralBackslashN),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "'\\n'",
                "\"x\\n\"",
                "'\\ \\|\\ '",
                "'\\n\\t'",
                "'prefix\\nsuffix'",
            ]
        );
    }

    #[test]
    fn ignores_non_literal_or_non_ifs_assignments() {
        let source = "\
#!/bin/bash
IFS=$'\\n'
IFS=' | '
foo='\\n'
bar=bar-n
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IfsSetToLiteralBackslashN),
        );

        assert!(diagnostics.is_empty());
    }
}
