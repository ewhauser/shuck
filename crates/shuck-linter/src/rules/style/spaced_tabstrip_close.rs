use crate::{Checker, Rule, Violation};

pub struct SpacedTabstripClose;

impl Violation for SpacedTabstripClose {
    fn rule() -> Rule {
        Rule::SpacedTabstripClose
    }

    fn message(&self) -> String {
        "this `<<-` closer must be indented with tabs only".to_owned()
    }
}

pub fn spaced_tabstrip_close(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.spaced_tabstrip_close_spans(),
        || SpacedTabstripClose,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
