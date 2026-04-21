use crate::{
    Checker, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation, WordOccurrenceRef,
    WordQuote,
};

pub struct TrapStringExpansion;

impl Violation for TrapStringExpansion {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::TrapStringExpansion
    }

    fn message(&self) -> String {
        "double-quoted trap handlers expand variables when the trap is set".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the trap action with single quotes".to_owned())
    }
}

pub fn trap_string_expansion(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .expansion_word_facts(ExpansionContext::TrapAction)
        .filter(|fact| fact.classification().quote == WordQuote::FullyQuoted)
        .flat_map(|fact| trap_string_expansion_diagnostics(fact, source))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn trap_string_expansion_diagnostics(
    fact: WordOccurrenceRef<'_, '_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    let fix = fact
        .single_quoted_equivalent_if_plain_double_quoted(source)
        .map(|replacement| Fix::unsafe_edit(Edit::replacement(replacement, fact.span())));

    fact.double_quoted_expansion_spans()
        .iter()
        .copied()
        .map(|span| {
            let diagnostic = crate::Diagnostic::new(TrapStringExpansion, span);
            match fix.as_ref() {
                Some(fix) => diagnostic.with_fix(fix.clone()),
                None => diagnostic,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, Diagnostic, LinterSettings, Rule, assert_diagnostics_diff};

    fn c008_diagnostics(source: &str) -> Vec<Diagnostic> {
        test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion))
    }

    #[test]
    fn attaches_unsafe_fix_metadata_for_plain_double_quoted_trap_actions() {
        let source = "trap \"echo $x\" EXIT\n";
        let diagnostics = c008_diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("rewrite the trap action with single quotes")
        );
    }

    #[test]
    fn reports_each_expansion_inside_the_trap_action() {
        let source = "trap \"echo $x $(date) ${y}\" EXIT\n";
        let diagnostics = c008_diagnostics(source);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)", "${y}"]
        );
    }

    #[test]
    fn ignores_trap_listing_modes() {
        let source = "trap -p EXIT\ntrap -l TERM\n";
        let diagnostics = c008_diagnostics(source);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_mixed_quoted_trap_words() {
        let source = "\
trap foo\"$x\"bar EXIT
trap \"$x\"\"$y\" EXIT
trap 'result=$?; '\"delete_container $container msg\"' || : ; exit $result' EXIT
";
        let diagnostics = c008_diagnostics(source);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_plain_double_quoted_trap_actions() {
        let source = "\
#!/bin/sh
trap \"echo $x $(date) ${y}\" EXIT
trap -- \"printf '%s\\n' $handler \\$HOME \\\"ok\\\" \\`date\\`\" INT
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::TrapStringExpansion),
            Applicability::Unsafe,
        );

        assert_eq!(result.diagnostics.len(), 4);
        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
trap 'echo $x $(date) ${y}' EXIT
trap -- 'printf '\\''%s\\n'\\'' $handler $HOME \"ok\" `date`' INT
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_locale_double_quoted_trap_actions_unchanged_when_fixing() {
        let source = "#!/bin/bash\ntrap $\"echo $x\" EXIT\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::TrapStringExpansion),
            Applicability::Unsafe,
        );

        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].fix.is_none());
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
        assert_eq!(result.fixed_diagnostics[0].span.slice(source), "$x");
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C008.sh").as_path(),
            &LinterSettings::for_rule(Rule::TrapStringExpansion),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C008_fix_C008.sh", result);
        Ok(())
    }
}
