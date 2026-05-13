use shuck_ast::{RedirectKind, Span, static_word_text};

use crate::{
    Checker, Diagnostic, Edit, Fix, FixAvailability, RedirectFact, Rule, ShellDialect, Violation,
};

pub struct AmpersandRedirectInSh;

impl Violation for AmpersandRedirectInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AmpersandRedirectInSh
    }

    fn message(&self) -> String {
        "combined `>&` redirection is not portable in `sh`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("expand the combined stdout/stderr redirect".to_owned())
    }
}

pub fn ampersand_redirect_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter_map(|redirect| combined_ampersand_redirect_span(redirect, checker.source()))
        .filter_map(|span| ampersand_redirect_in_sh_fix(span, source))
        .map(|(span, fix)| Diagnostic::new(AmpersandRedirectInSh, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
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

fn ampersand_redirect_in_sh_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let text = span.slice(source);
    let operator_index = text.find(">&")?;
    let target = text[operator_index + 2..].trim_start();
    if target.is_empty() {
        return None;
    }
    Some((
        span,
        Fix::safe_edit(Edit::replacement(format!("> {target} 2>&1"), span)),
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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

    #[test]
    fn applies_safe_fix_to_ampersand_redirects_in_sh() {
        let source = "#!/bin/sh\necho test >& /dev/null\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirectInSh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\necho test > /dev/null 2>&1\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
