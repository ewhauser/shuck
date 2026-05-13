use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct AvoidLetBuiltin;

impl Violation for AvoidLetBuiltin {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::AvoidLetBuiltin
    }

    fn message(&self) -> String {
        "prefer arithmetic expansion instead of `let`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite `let` as an arithmetic command".to_owned())
    }
}

pub fn avoid_let_builtin(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Ksh) {
        return;
    }

    let source = checker.source();
    let reports = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.wrappers().is_empty() && fact.effective_name_is("let"))
        .filter_map(|fact| {
            Some(LetCommandReport {
                diagnostic_span: let_command_span(source, fact)?,
                fix: let_command_fix(source, fact),
            })
        })
        .collect::<Vec<_>>();

    for report in reports {
        let mut diagnostic = Diagnostic::new(AvoidLetBuiltin, report.diagnostic_span);
        if let Some(fix) = report.fix {
            diagnostic = diagnostic.with_fix(fix);
        }
        checker.report_diagnostic(diagnostic);
    }
}

struct LetCommandReport {
    diagnostic_span: Span,
    fix: Option<Fix>,
}

fn let_command_fix(source: &str, fact: crate::facts::CommandFactRef<'_, '_>) -> Option<Fix> {
    let name = fact.body_name_word()?;
    let args = fact.body_args();
    let last_arg = args.last()?;
    let operands = args
        .iter()
        .map(|word| let_operand_text(source, word))
        .filter(|operand| operand != "--")
        .collect::<Vec<_>>();
    if operands.is_empty() {
        return None;
    }

    let replacement = format!("(( {} ))", operands.join(", "));
    let span = Span::from_positions(name.span.start, last_arg.span.end);
    Some(Fix::unsafe_edit(Edit::replacement(replacement, span)))
}

fn let_operand_text(source: &str, word: &shuck_ast::Word) -> String {
    word.quoted_content_span_in_source(source).map_or_else(
        || word.span.slice(source).to_owned(),
        |span| span.slice(source).to_owned(),
    )
}

fn let_command_span(source: &str, fact: crate::facts::CommandFactRef<'_, '_>) -> Option<Span> {
    let name = fact.body_name_word()?;
    let mut end = fact
        .body_args()
        .last()
        .map_or(name.span.end, |word| let_argument_end(source, word));

    let tail = source.get(end.offset..)?;
    let mut padding_end = end;
    for ch in tail.chars() {
        match ch {
            ' ' => padding_end = padding_end.advanced_by(" "),
            '\t' => padding_end = padding_end.advanced_by("\t"),
            ';' => {
                end = padding_end;
                break;
            }
            '\r' | '\n' => break,
            _ => break,
        }
    }

    Some(Span::from_positions(name.span.start, end))
}

fn let_argument_end(source: &str, word: &shuck_ast::Word) -> shuck_ast::Position {
    if word.is_fully_quoted() {
        let text = word.span.slice(source);
        if let Some(trimmed) = text.strip_suffix('"').or_else(|| text.strip_suffix('\'')) {
            return word.span.start.advanced_by(trimmed);
        }
    }

    word.span.end
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_let_builtin_in_bash() {
        let source = "#!/usr/bin/env bash\nlet 10\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AvoidLetBuiltin));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 7);
    }

    #[test]
    fn anchors_quoted_let_builtin_before_closing_quote() {
        let source = "#!/usr/bin/env bash\nlet \"number %= RANGE\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AvoidLetBuiltin));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 21);
    }

    #[test]
    fn ignores_non_bash_or_ksh_shells() {
        let source = "#!/bin/sh\nlet 10\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AvoidLetBuiltin).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_wrapped_let_commands() {
        let source = "#!/usr/bin/env bash\ncommand let 10\nbuiltin let 10\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AvoidLetBuiltin));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_rewrite_let_builtin() {
        let source = "#!/usr/bin/env bash\nlet count+=1 \"total %= RANGE\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AvoidLetBuiltin),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/usr/bin/env bash\n(( count+=1, total %= RANGE ))\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_without_option_sentinel_operand() {
        let source = "#!/usr/bin/env bash\nlet -- i=1\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AvoidLetBuiltin),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/usr/bin/env bash\n(( i=1 ))\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn skips_unsafe_fix_for_only_option_sentinel_operand() {
        let source = "#!/usr/bin/env bash\nlet --\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AvoidLetBuiltin),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }
}
