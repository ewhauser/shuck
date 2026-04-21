use shuck_ast::RedirectKind;

use crate::{Checker, Edit, Fix, FixAvailability, RedirectFact, Rule, ShellDialect, Violation};

pub struct AmpersandRedirection;

impl Violation for AmpersandRedirection {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::AmpersandRedirection
    }

    fn message(&self) -> String {
        "use of `&>` is not portable in `sh`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite `&>` as separate stdout and stderr redirects".to_owned())
    }
}

pub fn ampersand_redirection(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter(|redirect| redirect.redirect().kind == RedirectKind::OutputBoth)
        .map(|redirect| {
            (
                redirect.redirect().span,
                ampersand_redirection_fix(redirect, checker.source()),
            )
        })
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        let diagnostic = crate::Diagnostic::new(AmpersandRedirection, span);
        if let Some(fix) = fix {
            checker.report_diagnostic_dedup(diagnostic.with_fix(fix));
        } else {
            checker.report_diagnostic_dedup(diagnostic);
        }
    }
}

fn ampersand_redirection_fix(redirect: &RedirectFact<'_>, source: &str) -> Option<Fix> {
    let redirect_data = redirect.redirect();
    if redirect_data.kind != RedirectKind::OutputBoth
        || redirect_data.fd.is_some()
        || redirect_data.fd_var.is_some()
    {
        return None;
    }

    let operator_span = redirect.operator_span();
    if source[..operator_span.start.offset]
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    let target_span = redirect.target_span()?;

    Some(Fix::safe_edits([
        Edit::replacement(">", operator_span),
        Edit::insertion(target_span.end.offset, " 2>&1"),
    ]))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_ampersand_redirection_in_sh() {
        let source = "\
#!/bin/sh
: &>out
echo ok &> /dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "&>out");
        assert_eq!(diagnostics[1].span.slice(source), "&> /dev/null");
    }

    #[test]
    fn applies_safe_fix_to_plain_ampersand_redirections() {
        let source = "\
#!/bin/sh
: &>out
echo ok &> /dev/null
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n: >out 2>&1\necho ok > /dev/null 2>&1\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_digit_prefixed_ampersand_redirections_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
echo ok 2&>file
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
        assert_eq!(result.fixed_diagnostics[0].span.slice(source), "&>file");
    }

    #[test]
    fn ignores_ampersand_redirection_in_bash() {
        let source = "\
#!/bin/bash
: &>out
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("portability").join("X012.sh").as_path(),
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("X012_fix_X012.sh", result);
        Ok(())
    }
}
