use shuck_ast::{RedirectKind, Span};

use crate::{Checker, RedirectFact, Rule, ShellDialect, Violation, static_word_text};

pub struct AmpersandRedirectInSh;

impl Violation for AmpersandRedirectInSh {
    fn rule() -> Rule {
        Rule::AmpersandRedirectInSh
    }

    fn message(&self) -> String {
        "combined `>&` redirection is not portable in `sh`".to_owned()
    }
}

pub fn ampersand_redirect_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter_map(|redirect| combined_ampersand_redirect_span(redirect, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AmpersandRedirectInSh);
}

fn combined_ampersand_redirect_span(redirect: &RedirectFact<'_>, source: &str) -> Option<Span> {
    let redirect_data = redirect.redirect();
    if !matches!(
        redirect_data.kind,
        RedirectKind::DupOutput | RedirectKind::Output
    ) {
        return None;
    }

    let analysis = redirect.analysis()?;
    if !analysis.expansion.is_fixed_literal() {
        return None;
    }

    let target = redirect_data.word_target()?;
    let target_text = static_word_text(target, source)?;
    if target_text == "-" || target_text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let redirect_start = redirect_data.span.start.offset;
    let target_start = target.span.start.offset;
    if redirect_start > target_start || target_start > source.len() {
        return None;
    }
    let operator_text = &source[redirect_start..target_start];
    let operator_offset = operator_text.find(">&")?;
    if !operator_text[..operator_offset]
        .chars()
        .all(char::is_whitespace)
    {
        return None;
    }
    if !operator_text[operator_offset..].starts_with(">&") {
        return None;
    }

    Some(redirect_data.span)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_ampersand_redirect_in_sh() {
        let source = "\
#!/bin/sh
echo test >& /dev/null
echo test >&+1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirectInSh),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), ">& /dev/null");
        assert_eq!(diagnostics[1].span.slice(source), ">&+1");
    }

    #[test]
    fn ignores_descriptor_duplication_close_and_explicit_fd_forms() {
        let source = "\
#!/bin/sh
echo test >&2
echo test 1>&2
echo test 1>&+1
echo test 1>&/tmp/log
echo test 2>&/tmp/log
echo test >&\"2\"
echo test 1>&\"2\"
echo test >&-
echo test >&\"$fd\"
echo test 1>\"/tmp/>&log\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirectInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_ampersand_redirect_in_bash() {
        let source = "\
#!/bin/bash
echo test >& /dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirectInSh),
        );

        assert!(diagnostics.is_empty());
    }
}
