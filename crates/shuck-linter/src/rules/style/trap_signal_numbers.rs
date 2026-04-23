use shuck_ast::{Span, Word, static_word_text};

use crate::rules::portability::trap_common::parse_trap_args;
use crate::{Checker, Rule, ShellDialect, Violation};

pub struct TrapSignalNumbers;

impl Violation for TrapSignalNumbers {
    fn rule() -> Rule {
        Rule::TrapSignalNumbers
    }

    fn message(&self) -> String {
        "prefer symbolic signal names in `trap` instead of numeric IDs".to_owned()
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
        .flat_map(|fact| trap_numeric_signal_spans(fact.body_args(), checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TrapSignalNumbers);
}

fn trap_numeric_signal_spans(args: &[&Word], source: &str) -> Vec<Span> {
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
            static_word_text(word, source)
                .as_deref()
                .is_some_and(is_numeric_signal_name)
                .then_some(word.span)
        })
        .collect()
}

fn is_numeric_signal_name(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_numeric_trap_signals() {
        let source = "\
#!/bin/sh
trap 'echo caught signal' 1 2 13 15
trap -- '' 0
trap -- '' 9 10
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapSignalNumbers));

        assert_eq!(diagnostics.len(), 7);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["1", "2", "13", "15", "0", "9", "10"]
        );
    }

    #[test]
    fn ignores_symbolic_and_listing_modes() {
        let source = "\
#!/bin/sh
trap '' HUP INT TERM
trap -l 1
trap -p 2
trap -lp 15
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapSignalNumbers));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn does_not_run_for_other_shells() {
        let source = "\
#!/bin/zsh
trap 'echo caught signal' 1 2
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TrapSignalNumbers).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
