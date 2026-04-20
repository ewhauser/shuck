use crate::facts::WordFactHostKind;
use crate::{Checker, ExpansionContext, Rule, Violation};
use shuck_ast::Span;

pub struct LeadingGlobArgument;

impl Violation for LeadingGlobArgument {
    fn rule() -> Rule {
        Rule::LeadingGlobArgument
    }

    fn message(&self) -> String {
        "wildcard arguments should be guarded with `./` or `--`".to_owned()
    }
}

pub fn leading_glob_argument(checker: &mut Checker) {
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
        .filter_map(|fact| reportable_glob_span(checker, fact))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LeadingGlobArgument);
}

fn reportable_glob_span(
    checker: &Checker<'_>,
    fact: crate::facts::WordOccurrenceRef<'_, '_>,
) -> Option<Span> {
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

    Some(anchor_span(fact.span()))
}

fn command_exempts_glob_warning(command: Option<&str>) -> bool {
    matches!(command, Some("echo" | "printf"))
}

fn command_has_separator_before(
    command: &crate::facts::CommandFact<'_>,
    target_span: Span,
    source: &str,
) -> bool {
    command
        .body_args()
        .iter()
        .take_while(|word| word.span != target_span)
        .filter_map(|word| crate::rules::common::word::static_word_text(word, source))
        .any(|arg| arg == "--")
}

fn anchor_span(span: Span) -> Span {
    Span::from_positions(span.start, span.start.advanced_by("*"))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
