use shuck_ast::Span;

use crate::facts::words::FactWordRef;
use crate::rules::portability::trap_common::parse_trap_args;
use crate::{Checker, Rule, ShellDialect, Violation};

pub struct TrapSignalNumbers;

impl Violation for TrapSignalNumbers {
    fn rule() -> Rule {
        Rule::TrapSignalNumbers
    }

    fn message(&self) -> String {
        "use symbolic signal names in `trap` for non-portable numeric IDs".to_owned()
    }
}

pub fn trap_signal_numbers(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("trap"))
        .flat_map(|fact| {
            trap_numeric_signal_spans(&fact.arena_body_args(checker.source()), checker.source())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TrapSignalNumbers);
}

fn trap_numeric_signal_spans<'a>(args: &'a [FactWordRef<'a>], source: &'a str) -> Vec<Span> {
    if !trap_action_allows_numeric_signal_report(args, source) {
        return Vec::new();
    }

    let Some(parsed) = parse_trap_args(args, source) else {
        return Vec::new();
    };
    if parsed.listing_mode {
        return Vec::new();
    }

    parsed
        .signal_words
        .iter()
        .filter_map(|word| {
            word.static_text(source)
                .as_deref()
                .is_some_and(is_reportable_numeric_signal)
                .then_some(word.span())
        })
        .collect()
}

fn trap_action_allows_numeric_signal_report<'a>(
    args: &'a [FactWordRef<'a>],
    source: &'a str,
) -> bool {
    if args.len() < 2 {
        return false;
    }

    !matches!(
        args.first()
            .and_then(|word| word.static_text(source))
            .as_deref(),
        Some("-" | "--")
    )
}

fn is_reportable_numeric_signal(text: &str) -> bool {
    is_numeric_signal_name(text) && !is_portable_numeric_trap_alias(text)
}

fn is_numeric_signal_name(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
}

fn is_portable_numeric_trap_alias(text: &str) -> bool {
    matches!(text, "0" | "1" | "2" | "3" | "6" | "9" | "14" | "15")
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_numeric_trap_signals() {
        let source = "\
#!/bin/sh
trap 'echo caught signal' 4 5 7 8
trap '' 10 11 12 13 16
trap '' 00 01 015
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapSignalNumbers));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "4", "5", "7", "8", "10", "11", "12", "13", "16", "00", "01", "015"
            ]
        );
    }

    #[test]
    fn ignores_portable_numeric_aliases_symbolic_and_listing_modes() {
        let source = "\
#!/bin/sh
trap '' HUP INT TERM
trap '' 0 1 2 3 6 9 14 15
trap -l 1
trap -p 2
trap -lp 15
command trap '' 9
trap - 13
trap -- '' 13
trap -- - 13
trap 13
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapSignalNumbers));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn does_not_run_for_other_shells() {
        let source = "\
#!/bin/zsh
trap 'echo caught signal' 4 10
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TrapSignalNumbers).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
