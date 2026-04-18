use std::collections::HashSet;

use crate::{Checker, Rule, Violation};

pub struct MissingBracketSpace;

impl Violation for MissingBracketSpace {
    fn rule() -> Rule {
        Rule::MissingBracketSpace
    }

    fn message(&self) -> String {
        "this unary `[` test operator is missing its operand before the closing `]`".to_owned()
    }
}

pub fn missing_bracket_space(checker: &mut Checker) {
    let mut seen_lines = HashSet::new();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.glued_closing_bracket_operand_span())
        .filter(|span| seen_lines.insert(span.start.line))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MissingBracketSpace);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bracket_tests_with_a_glued_closing_bracket() {
        let source = "\
#!/bin/sh
if [ -d /tmp]; then
  :
fi
if [ \"$dir\" = /tmp]; then
  :
fi
if [ -n \"$dir\"]; then
  :
fi
if [ -a /tmp]; then
  :
fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 9), (8, 9), (11, 9)]
        );
    }

    #[test]
    fn keeps_only_the_first_unary_match_per_line() {
        let source = "\
#!/bin/sh
if [ ! -d \"$a\"] || [ ! -d \"$b\"]; then
  :
fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            (
                diagnostics[0].span.start.line,
                diagnostics[0].span.start.column
            ),
            (2, 11)
        );
    }

    #[test]
    fn ignores_well_spaced_or_differently_malformed_tests() {
        let source = "\
#!/bin/sh
if [ -d /tmp ]; then
  :
fi
if [ x = \"]\"; then
  :
fi
echo /tmp]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
