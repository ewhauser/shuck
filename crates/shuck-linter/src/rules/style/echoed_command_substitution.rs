use shuck_ast::Span;

use crate::{
    Checker, CommandSubstitutionKind, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability,
    Rule, SubstitutionHostKind, Violation, WordFactContext,
};

pub struct EchoedCommandSubstitution;

impl Violation for EchoedCommandSubstitution {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::EchoedCommandSubstitution
    }

    fn message(&self) -> String {
        "call the command directly instead of echoing its substitution".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the echo wrapper".to_owned())
    }
}

pub fn echoed_command_substitution(checker: &mut Checker) {
    let source = checker.source();
    let reports = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("echo"))
        .filter_map(|fact| {
            let name = fact.body_name_word()?;
            let [word] = fact.body_args() else {
                return None;
            };

            checker
                .facts()
                .word_fact(
                    word.span,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                )
                .filter(|fact| fact.classification().has_plain_command_substitution())
                .filter(|_| {
                    !fact.substitution_facts().iter().any(|substitution| {
                        substitution.kind() == CommandSubstitutionKind::Command
                            && matches!(
                                substitution.host_kind(),
                                SubstitutionHostKind::CommandArgument
                            )
                            && substitution.host_word_span() == word.span
                            && (substitution.is_bash_file_slurp()
                                || substitution.body_has_multiple_statements()
                                || (substitution.uses_backtick_syntax()
                                    && !substitution.unquoted_in_host()))
                    })
                })
                .map(|_| EchoedCommandSubstitutionReport {
                    diagnostic_span: word.span,
                    fix_span: Span::from_positions(name.span.start, word.span.end),
                })
        })
        .collect::<Vec<_>>();

    for report in reports {
        let mut diagnostic = Diagnostic::new(EchoedCommandSubstitution, report.diagnostic_span);
        if let Some(fix) =
            echoed_command_substitution_fix(source, report.diagnostic_span, report.fix_span)
        {
            diagnostic = diagnostic.with_fix(fix);
        }
        checker.report_diagnostic_dedup(diagnostic);
    }
}

struct EchoedCommandSubstitutionReport {
    diagnostic_span: Span,
    fix_span: Span,
}

fn echoed_command_substitution_fix(
    source: &str,
    diagnostic_span: Span,
    fix_span: Span,
) -> Option<Fix> {
    let text = diagnostic_span.slice(source);
    let body = command_substitution_body(text)?;
    Some(Fix::unsafe_edit(Edit::replacement(body, fix_span)))
}

fn command_substitution_body(text: &str) -> Option<&str> {
    let unquoted = text
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(text);
    unquoted
        .strip_prefix("$(")
        .and_then(|inner| inner.strip_suffix(')'))
        .or_else(|| {
            unquoted
                .strip_prefix('`')
                .and_then(|inner| inner.strip_suffix('`'))
        })
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn only_reports_plain_command_substitutions() {
        let source = "echo \"$(date)\"\necho \"date: $(date)\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(diagnostics[0].span.slice(source), "\"$(date)\"");
    }

    #[test]
    fn ignores_echoes_with_extra_arguments() {
        let source = "echo prefix $(date)\necho \"$(date)\" suffix\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_echo_flag_forms_and_bash_file_slurps() {
        let source = "echo -n \"$(date)\"\necho -e \"$(date)\"\necho \"$(< file.txt)\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_single_argument_echoes_inside_binary_command_lists() {
        let source = "value=\"$(true && echo \"$(date)\")\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$(date)\""]
        );
    }

    #[test]
    fn ignores_quoted_backticks_and_multi_statement_substitutions() {
        let source = r#"SCRIPT=$(echo "`basename "$0"`")
value=$(echo $(printf '%s\n' one; printf '%s\n' two) | tr -d ' ')
"#;
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_echoes_inside_prefixed_nested_command_substitutions() {
        let source = "cp -v $filename $OUT/$(echo $(basename $filename .fuzz))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(basename $filename .fuzz)"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_remove_echo_wrapper() {
        let source = "#!/bin/sh\necho \"$(date)\"\necho $(basename \"$path\" .txt)\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EchoedCommandSubstitution),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ndate\nbasename \"$path\" .txt\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
