use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct AvoidLetBuiltin;

impl Violation for AvoidLetBuiltin {
    fn rule() -> Rule {
        Rule::AvoidLetBuiltin
    }

    fn message(&self) -> String {
        "prefer arithmetic expansion instead of `let`".to_owned()
    }
}

pub fn avoid_let_builtin(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Ksh) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.wrappers().is_empty() && fact.effective_name_is("let"))
        .filter_map(|fact| let_command_span(checker.source(), fact))
        .collect::<Vec<_>>();

    checker.report_all(spans, || AvoidLetBuiltin);
}

fn let_command_span(source: &str, fact: &crate::facts::CommandFact<'_>) -> Option<Span> {
    let name = fact.body_name_word()?;
    let mut end = fact
        .body_args()
        .last()
        .map_or(name.span.end, |word| word.span.end);

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

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

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
}
