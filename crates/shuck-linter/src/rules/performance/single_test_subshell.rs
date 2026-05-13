use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SingleTestSubshell;

impl Violation for SingleTestSubshell {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SingleTestSubshell
    }

    fn message(&self) -> String {
        "drop the subshell around this single test condition".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the subshell parentheses".to_owned())
    }
}

pub fn single_test_subshell(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker.facts().single_test_subshell_spans().to_vec();
    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(SingleTestSubshell, span)
                .with_fix(remove_outer_parens_fix(source, span)),
        );
    }
}

fn remove_outer_parens_fix(source: &str, span: Span) -> Fix {
    let close_start = span.end.offset.saturating_sub(1);
    Fix::unsafe_edits([
        Edit::deletion_at(
            opening_paren_delete_start(source, span.start.offset),
            span.start.offset + 1,
        ),
        Edit::deletion_at(close_start, closing_paren_delete_end(source, close_start)),
    ])
}

fn opening_paren_delete_start(source: &str, open_start: usize) -> usize {
    if source.as_bytes().get(open_start + 1) != Some(&b'\n') {
        return open_start;
    }

    let mut start = open_start;
    while start > 0
        && source
            .as_bytes()
            .get(start - 1)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        start -= 1;
    }
    start
}

fn closing_paren_delete_end(source: &str, close_start: usize) -> usize {
    if !offset_is_indented_line_start(source, close_start) {
        return close_start + 1;
    }

    let mut end = close_start + 1;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        end += 1;
    }
    if source.as_bytes().get(end) != Some(&b';') {
        return close_start + 1;
    }

    end += 1;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        end += 1;
    }
    end
}

fn offset_is_indented_line_start(source: &str, offset: usize) -> bool {
    let line_start = source[..offset].rfind('\n').map_or(0, |offset| offset + 1);
    source[line_start..offset]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn anchors_on_the_condition_subshell() {
        let source = "\
#!/bin/sh
if (test -f /etc/passwd); then :; fi
if (test -f /etc/passwd) >/dev/null 2>&1; then :; fi
if ! (test -f /etc/passwd); then :; fi
if ( ! test -f /etc/passwd ); then :; fi
if (test -f /etc/passwd || test -f /etc/hosts); then :; fi
if ! (test -f /etc/passwd || test -f /etc/hosts); then :; fi
while ([ -f /etc/passwd ]); do :; done
while ! ([ -f /etc/passwd ]); do :; done
until (command test -f /etc/passwd); do :; done
until ! (command test -f /etc/passwd); do :; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SingleTestSubshell));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "(test -f /etc/passwd)",
                "(test -f /etc/passwd)",
                "(test -f /etc/passwd)",
                "( ! test -f /etc/passwd )",
                "(test -f /etc/passwd || test -f /etc/hosts)",
                "([ -f /etc/passwd ])",
                "([ -f /etc/passwd ])",
                "(command test -f /etc/passwd)",
                "(command test -f /etc/passwd)",
            ]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_remove_subshell_parentheses() {
        let source = "#!/bin/sh\nif (test -f /etc/passwd); then :; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SingleTestSubshell),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nif test -f /etc/passwd; then :; fi\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_multiline_condition_subshell() {
        let source = "\
#!/bin/sh
if (
  test -f /etc/passwd
); then
  :
fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SingleTestSubshell),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
if
  test -f /etc/passwd
then
  :
fi
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
