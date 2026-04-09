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
    let mut start = 0usize;

    if let Some(first) = args.first().and_then(|word| static_word_text(word, source)) {
        match first.as_str() {
            "-p" | "-l" => return Vec::new(),
            "--" => start = 1,
            _ => {}
        }
    }

    if args.len() <= start + 1 {
        return Vec::new();
    }

    args[start + 1..]
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
}
