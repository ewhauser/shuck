use shuck_ast::{Span, Word};

use super::trap_common::parse_trap_args;
use crate::rules::common::word::leading_static_word_prefix_fix_in_source;
use crate::{Checker, Fix, FixAvailability, Rule, ShellDialect, Violation, static_word_text};

const SIG_PREFIX_LEN: usize = 3;
const FIX_TITLE: &str = "remove the leading `SIG` prefix from the trap signal name";

pub struct SignalNameInTrap;

#[derive(Clone)]
struct SignalNameInTrapOccurrence {
    report_span: Span,
    fix: Option<Fix>,
}

impl Violation for SignalNameInTrap {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::SignalNameInTrap
    }

    fn message(&self) -> String {
        "symbolic signal names with a `SIG` prefix are not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some(FIX_TITLE.to_owned())
    }
}

pub fn signal_name_in_trap(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let occurrences = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("trap"))
        .flat_map(|fact| trap_signal_name_occurrences(fact.body_args(), checker.source()))
        .collect::<Vec<_>>();

    for occurrence in occurrences {
        let diagnostic = crate::Diagnostic::new(SignalNameInTrap, occurrence.report_span);
        checker.report_diagnostic_dedup(match occurrence.fix {
            Some(fix) => diagnostic.with_fix(fix),
            None => diagnostic,
        });
    }
}

fn trap_signal_name_occurrences(args: &[&Word], source: &str) -> Vec<SignalNameInTrapOccurrence> {
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
            let text = static_word_text(word, source)?;
            (text.len() > 3
                && text
                    .get(..3)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SIG"))
                && text[3..]
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()))
            .then(|| {
                let span = word
                    .quoted_content_span_in_source(source)
                    .unwrap_or(word.span);
                SignalNameInTrapOccurrence {
                    report_span: span,
                    fix: signal_name_in_trap_fix(word, source),
                }
            })
        })
        .collect()
}

fn signal_name_in_trap_fix(word: &Word, source: &str) -> Option<Fix> {
    leading_static_word_prefix_fix_in_source(word, source, SIG_PREFIX_LEN)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::FIX_TITLE;
    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn reports_case_insensitive_sig_prefixes() {
        let source = "\
#!/bin/sh
trap '' sigint
trap -- '' SigTerm
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["sigint", "SigTerm"]
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata_to_reported_signal_names() {
        let source = "\
#!/bin/sh
trap '' SIGINT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(diagnostics[0].fix_title.as_deref(), Some(FIX_TITLE));
    }

    #[test]
    fn applies_unsafe_fix_to_sig_prefixed_signal_names() {
        let source = "\
#!/bin/sh
trap '' SIGHUP
trap -- '' SIGINT
trap SIGTERM
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SignalNameInTrap),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
trap '' HUP
trap -- '' INT
trap TERM
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_inside_quoted_signal_operands() {
        let source = "\
#!/bin/sh
trap '' \"SIGINT\"
trap '' 'SIGTERM'
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SignalNameInTrap),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["SIGINT", "SIGTERM"]
        );
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
trap '' \"INT\"
trap '' 'TERM'
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_inside_mixed_and_dollar_quoted_signal_operands() {
        let source = "\
#!/bin/sh
trap '' \"SIG\"INT
trap '' $'SIGTERM'
trap '' 'SIG'QUIT
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SignalNameInTrap),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
trap '' INT
trap '' $'TERM'
trap '' QUIT
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_cooked_signal_prefixes_unfixed_when_raw_source_does_not_match() {
        let source = "\
#!/bin/sh
trap '' $'S\\x49GINT'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SignalNameInTrap));
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SignalNameInTrap),
            Applicability::Unsafe,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].fix.is_none());
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(
            result
                .fixed_diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$'S\\x49GINT'"]
        );
    }

    #[test]
    fn leaves_listing_modes_and_portable_signal_names_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
trap -p SIGINT
trap -- '' HUP
trap EXIT
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SignalNameInTrap),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("portability").join("X069.sh").as_path(),
            &LinterSettings::for_rule(Rule::SignalNameInTrap),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("X069_fix_X069.sh", result);
        Ok(())
    }
}
