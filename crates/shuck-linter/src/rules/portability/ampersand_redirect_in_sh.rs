use shuck_ast::{RedirectKind, Span};

use crate::{Checker, RedirectFact, Rule, ShellDialect, Violation};

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
    if redirect_data.kind != RedirectKind::DupOutput {
        return None;
    }

    let analysis = redirect.analysis()?;
    if !analysis.expansion.is_fixed_literal() || analysis.numeric_descriptor_target.is_some() {
        return None;
    }

    let target = redirect_data.word_target()?;
    if target.span.slice(source) == "-" {
        return None;
    }

    let redirect_text = redirect_data.span.slice(source);
    if !redirect_text.starts_with(">&") {
        return None;
    }

    Some(Span::from_positions(
        redirect_data.span.start,
        redirect_data.span.start.advanced_by(">&"),
    ))
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
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirectInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), ">&");
    }

    #[test]
    fn ignores_descriptor_duplication_and_close_forms() {
        let source = "\
#!/bin/sh
echo test >&2
echo test 1>&2
echo test >&-
echo test >&\"$fd\"
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
