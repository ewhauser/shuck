use crate::facts::WordFactHostKind;
use crate::{Checker, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation};
use shuck_ast::Span;

pub struct LeadingGlobArgument;

impl Violation for LeadingGlobArgument {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LeadingGlobArgument
    }

    fn message(&self) -> String {
        "wildcard arguments should be guarded with `./` or `--`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("prefix the wildcard argument with `./`".to_owned())
    }
}

pub fn leading_glob_argument(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
        .filter_map(|fact| reportable_glob_diagnostic(checker, fact))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn reportable_glob_diagnostic(
    checker: &Checker<'_>,
    fact: crate::facts::WordOccurrenceRef<'_, '_>,
) -> Option<crate::Diagnostic> {
    let command = checker.facts().command(fact.command_id());
    if command_exempts_glob_warning(command.effective_name()) {
        return None;
    }
    if checker
        .facts()
        .command(fact.command_id())
        .zsh_options()
        .is_some_and(|options| options.glob.is_definitely_off())
    {
        return None;
    }

    let text = fact.span().slice(checker.source());
    let prefix = text.chars().next()?;
    if !matches!(prefix, '*' | '?') {
        return None;
    }
    if fact.starts_with_extglob() {
        return None;
    }

    if fact.operand_class()?.is_fixed_literal() {
        return None;
    }

    if command_has_separator_before(command, fact.span(), checker.source()) {
        return None;
    }

    let word_span = fact.span();
    Some(
        crate::Diagnostic::new(LeadingGlobArgument, anchor_span(word_span)).with_fix(
            Fix::unsafe_edit(Edit::insertion(word_span.start.offset, "./")),
        ),
    )
}

fn command_exempts_glob_warning(command: Option<&str>) -> bool {
    matches!(command, Some("echo" | "printf"))
}

fn command_has_separator_before(
    command: crate::facts::CommandFactRef<'_, '_>,
    target_span: Span,
    source: &str,
) -> bool {
    command
        .arena_body_args(source)
        .into_iter()
        .take_while(|word| word.span() != target_span)
        .filter_map(|word| word.static_text(source))
        .any(|arg| arg == "--")
}

fn anchor_span(span: Span) -> Span {
    Span::from_positions(span.start, span.start.advanced_by("*"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn anchors_on_wildcard_and_attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nrm *\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LeadingGlobArgument));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "*");
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("prefix the wildcard argument with `./`")
        );
    }

    #[test]
    fn reports_unguarded_leading_globs() {
        let source = "\
rm *
cat ?a
command mv *a dest/
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LeadingGlobArgument));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*", "?", "*"]
        );
    }

    #[test]
    fn ignores_guarded_and_prefixed_patterns() {
        let source = "\
rm -- *
rm ./*
rm foo/*
rm a*
set -- *
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LeadingGlobArgument));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_unsafe_fix_to_reported_wildcard_arguments() {
        let source = "\
rm *
cat ?a
command mv *a dest/
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobArgument),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "rm ./*\ncat ./?a\ncommand mv ./*a dest/\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_guarded_and_exempt_patterns_unchanged_when_fixing() {
        let source = "\
rm -- *
rm ./*
rm foo/*
rm a*
echo *
printf '%s\\n' *
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobArgument),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_leading_extglob_patterns() {
        let source = "\
shopt -s extglob
rm ?(*.txt)
rm *(@.txt)
rm *.@(jpg|png)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobArgument)
                .with_shell(crate::ShellDialect::Bash),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
    }

    #[test]
    fn ignores_echo_and_printf_arguments() {
        let source = "\
echo *
command echo *
printf '%s\\n' *
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LeadingGlobArgument));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_globs_when_noglob_is_active() {
        let source = "setopt no_glob\nrm *\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobArgument)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_noglob_wrapped_commands_in_zsh() {
        let source = "noglob rm *\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobArgument)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C012.sh").as_path(),
            &LinterSettings::for_rule(Rule::LeadingGlobArgument),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C012_fix_C012.sh", result);
        Ok(())
    }
}
