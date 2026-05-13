use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SpacedTabstripClose;

impl Violation for SpacedTabstripClose {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SpacedTabstripClose
    }

    fn message(&self) -> String {
        "this `<<-` closer must be indented with tabs only".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete spaces before the tab-stripped heredoc closer".to_owned())
    }
}

pub fn spaced_tabstrip_close(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .source_facts()
        .spaced_tabstrip_close_spans()
        .iter()
        .copied()
        .filter_map(|span| spaced_tabstrip_close_fix(span, source))
        .map(|(span, fix)| Diagnostic::new(SpacedTabstripClose, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn spaced_tabstrip_close_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let start = span.start.offset;
    let line = source.get(start..)?.split_inclusive('\n').next()?;
    let mut edits = Vec::new();
    let mut offset = start;
    for ch in line.chars() {
        match ch {
            ' ' => edits.push(Edit::deletion_at(offset, offset + 1)),
            '\t' => {}
            _ => break,
        }
        offset += ch.len_utf8();
    }
    (!edits.is_empty()).then(|| (span, Fix::unsafe_edits(edits)))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_space_indented_tabstrip_close_candidate() {
        let source = "\
#!/bin/sh
cat <<-END
x
  END
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn reports_mixed_tab_and_space_before_candidate_close() {
        let source = "\
#!/bin/sh
cat <<-END
x
\t END
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn applies_unsafe_fix_by_deleting_spaces_before_tabstrip_close() {
        let source = "#!/bin/sh\ncat <<-END\nx\n\t END\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SpacedTabstripClose),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\ncat <<-END\nx\n\tEND\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_tab_indented_close_candidates_without_spaces() {
        let source = "\
#!/bin/sh
cat <<-END
x
\tEND
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_plain_heredoc_close_candidates() {
        let source = "\
#!/bin/sh
cat <<END
x
  END
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_each_spaced_candidate_line() {
        let source = "\
#!/bin/sh
cat <<-END
x
  END
 \tEND
END
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SpacedTabstripClose));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[1].span.start.line, 5);
    }
}
