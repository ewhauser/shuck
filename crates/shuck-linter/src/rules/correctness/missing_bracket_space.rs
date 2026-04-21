use std::collections::HashSet;

use shuck_ast::Span;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct MissingBracketSpace;

impl Violation for MissingBracketSpace {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::MissingBracketSpace
    }

    fn message(&self) -> String {
        "this unary `[` test operator is missing its operand before the closing `]`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a space before the closing `]`".to_owned())
    }
}

pub fn missing_bracket_space(checker: &mut Checker) {
    let mut seen_lines = HashSet::new();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let span = fact.glued_closing_bracket_operand_span()?;
            let insert_offset = fact.glued_closing_bracket_insert_offset()?;
            seen_lines
                .insert(span.start.line)
                .then_some((span, insert_offset))
        })
        .map(|(span, insert_offset)| diagnostic_for_glued_closing_bracket(span, insert_offset))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn diagnostic_for_glued_closing_bracket(span: Span, insert_offset: usize) -> crate::Diagnostic {
    crate::Diagnostic::new(MissingBracketSpace, span)
        .with_fix(Fix::unsafe_edit(Edit::insertion(insert_offset, " ")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nif [ -d /tmp]; then\n  :\nfi\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("insert a space before the closing `]`")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_glued_closing_brackets() {
        let source = "\
#!/bin/sh
if [ -d /tmp]; then
  :
fi
if [ ! -n \"$dir\"]; then
  :
fi
if [ -a /tmp]; then
  :
fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MissingBracketSpace),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
if [ -d /tmp ]; then
  :
fi
if [ ! -n \"$dir\" ]; then
  :
fi
if [ -a /tmp ]; then
  :
fi
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C030.sh").as_path(),
            &LinterSettings::for_rule(Rule::MissingBracketSpace),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C030_fix_C030.sh", result);
        Ok(())
    }
}
