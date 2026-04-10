use shuck_ast::{Span, Word};

use crate::{Checker, Rule, ShellDialect, Violation, static_word_text};

pub struct SignalNameInTrap;

impl Violation for SignalNameInTrap {
    fn rule() -> Rule {
        Rule::SignalNameInTrap
    }

    fn message(&self) -> String {
        "symbolic signal names with a `SIG` prefix are not portable in `sh` scripts".to_owned()
    }
}

pub fn signal_name_in_trap(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("trap"))
        .flat_map(|fact| trap_signal_name_spans(fact.body_args(), checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SignalNameInTrap);
}

fn trap_signal_name_spans(args: &[&Word], source: &str) -> Vec<Span> {
    let signal_words = match args.first().and_then(|word| static_word_text(word, source)) {
        Some(first) if trap_is_listing_mode(first.as_str()) => return Vec::new(),
        Some(first) if first == "--" => {
            if args.len() == 2 {
                &args[1..]
            } else if args.len() > 2 {
                &args[2..]
            } else {
                return Vec::new();
            }
        }
        _ => {
            if args.len() == 1 {
                args
            } else if args.len() > 1 {
                &args[1..]
            } else {
                return Vec::new();
            }
        }
    };

    signal_words
        .iter()
        .filter_map(|word| {
            let text = static_word_text(word, source)?;
            (text.len() > 3
                && text.starts_with("SIG")
                && text[3..]
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()))
            .then_some(word.span)
        })
        .collect()
}

fn trap_is_listing_mode(text: &str) -> bool {
    text.strip_prefix('-').is_some_and(|flags| {
        !flags.is_empty() && flags.chars().all(|flag| matches!(flag, 'l' | 'p'))
    })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_sig_prefixed_signal_names() {
        let source = "\
#!/bin/sh
trap '' SIGHUP
trap -- '' SIGINT
trap '' HUP
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["SIGHUP", "SIGINT"]
        );
    }

    #[test]
    fn ignores_listing_modes_and_bash_shells() {
        let source = "\
#!/bin/bash
trap -p SIGINT
trap '' SIGINT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_combined_listing_flags_in_sh() {
        let source = "\
#!/bin/sh
trap -lp SIGHUP
trap -pl SIGINT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_actionless_sig_prefixed_signal_names() {
        let source = "\
#!/bin/sh
trap SIGHUP
trap -- SIGINT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["SIGHUP", "SIGINT"]
        );
    }
}
