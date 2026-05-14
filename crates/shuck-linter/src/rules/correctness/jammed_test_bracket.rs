use shuck_ast::Span;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct JammedTestBracket;

impl Violation for JammedTestBracket {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::JammedTestBracket
    }

    fn message(&self) -> String {
        "put a space after the opening test bracket".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a space after the opening bracket".to_owned())
    }
}

pub fn jammed_test_bracket(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let diagnostics = checker
        .facts()
        .command_facts()
        .jammed_test_bracket_facts()
        .iter()
        .map(|(span, insert_offset)| diagnostic_for_jammed_test_bracket(*span, *insert_offset))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn diagnostic_for_jammed_test_bracket(span: Span, insert_offset: usize) -> crate::Diagnostic {
    crate::Diagnostic::new(JammedTestBracket, span)
        .with_fix(Fix::unsafe_edit(Edit::insertion(insert_offset, " ")))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_opening_test_brackets_glued_to_operands() {
        let source = "\
#!/bin/bash
[! -r /etc/passwd ]
[foo]
[foo ]
[[foo]]
[[foo ]]
if [! -r x ]; then :; fi
case x in a)[! -r x ];; esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::JammedTestBracket));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 1, 2),
                (3, 1, 2),
                (4, 1, 2),
                (5, 1, 3),
                (6, 1, 3),
                (7, 4, 5),
                (8, 13, 14)
            ]
        );
    }

    #[test]
    fn ignores_spaced_tests_and_other_glued_words() {
        let source = "\
#!/bin/bash
[ ! -r /etc/passwd ]
[ foo]
[[ foo]]
if[ -r x ]; then :; fi
echo [foo]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::JammedTestBracket));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn skips_zsh() {
        let source = "#!/bin/zsh\n[foo]\n[[foo]]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::JammedTestBracket).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_unsafe_fix_after_the_opening_bracket() {
        let source = "\
#!/bin/bash
[! -r /etc/passwd ]
[[foo]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::JammedTestBracket),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ ! -r /etc/passwd ]
[[ foo]]
"
        );
    }
}
