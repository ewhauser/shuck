use shuck_ast::Span;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct MissingSpaceBeforeBracketClose;

impl Violation for MissingSpaceBeforeBracketClose {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::MissingSpaceBeforeBracketClose
    }

    fn message(&self) -> String {
        "put a space before the closing test bracket".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a space before the closing bracket".to_owned())
    }
}

pub fn missing_space_before_bracket_close(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let span = fact.missing_space_before_bracket_close_span()?;
            let insert_offset = fact.missing_space_before_bracket_close_insert_offset()?;
            Some(diagnostic_for_missing_space_before_bracket_close(
                span,
                insert_offset,
            ))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn diagnostic_for_missing_space_before_bracket_close(
    span: Span,
    insert_offset: usize,
) -> crate::Diagnostic {
    crate::Diagnostic::new(MissingSpaceBeforeBracketClose, span)
        .with_fix(Fix::unsafe_edit(Edit::insertion(insert_offset, " ")))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_bracket_tests_with_a_glued_closing_bracket() {
        let source = "\
#!/bin/bash
[ foo]
[ -d /tmp]
[ \"$dir\" = /tmp]
[foo]
[[ foo]]
[[foo]]
[ foo] ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingSpaceBeforeBracketClose),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 7), (3, 11), (4, 17), (5, 6), (6, 9), (7, 8), (8, 7)]
        );
    }

    #[test]
    fn ignores_spaced_tests_and_non_test_commands() {
        let source = "\
#!/bin/bash
[ foo ]
[foo ]
test foo]
echo foo]
[ \"foo]\" ]
[[ foo ]]
[[ ($x == y)]]
[ foo ] bar]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingSpaceBeforeBracketClose),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_unsafe_fix_before_the_closing_bracket() {
        let source = "\
#!/bin/bash
[ \"$dir\" = /tmp]
[[ foo]]
[foo]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MissingSpaceBeforeBracketClose),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ \"$dir\" = /tmp ]
[[ foo ]]
[foo ]
"
        );
    }
}
