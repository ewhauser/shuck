use shuck_ast::{Redirect, RedirectKind, Span};

use crate::{Checker, Rule, Violation, WrapperKind};

pub struct SudoRedirectionOrder;

impl Violation for SudoRedirectionOrder {
    fn rule() -> Rule {
        Rule::SudoRedirectionOrder
    }

    fn message(&self) -> String {
        "redirections on `sudo` still run in the current shell".to_owned()
    }
}

pub fn sudo_redirection_order(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.has_wrapper(WrapperKind::SudoFamily) && fact.options().sudo_family().is_some()
        })
        .flat_map(|fact| {
            fact.redirect_facts().iter().filter_map(|redirect| {
                (is_hazardous_sudo_redirect(redirect.redirect())
                    && !redirect
                        .analysis()
                        .is_some_and(|analysis| analysis.is_definitely_dev_null()))
                .then_some(sudo_redirect_span(redirect.redirect()))
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SudoRedirectionOrder);
}

fn is_hazardous_sudo_redirect(redirect: &Redirect) -> bool {
    if redirect.fd.is_some() || redirect.fd_var.is_some() {
        return false;
    }

    matches!(
        redirect.kind,
        RedirectKind::Input
            | RedirectKind::Output
            | RedirectKind::Append
            | RedirectKind::OutputBoth
    )
}

fn sudo_redirect_span(redirect: &Redirect) -> Span {
    let start = match redirect.kind {
        RedirectKind::OutputBoth => redirect.span.start.advanced_by("&"),
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupOutput
        | RedirectKind::DupInput => redirect.span.start,
    };
    let end = match redirect.kind {
        RedirectKind::Append => start.advanced_by(">>"),
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupOutput
        | RedirectKind::DupInput
        | RedirectKind::OutputBoth => start.advanced_by(">"),
    };

    if redirect.kind == RedirectKind::Input {
        return Span::from_positions(start, start.advanced_by("<"));
    }

    Span::from_positions(start, end)
}

#[cfg(test)]
fn operator_slices(source: &str) -> Vec<&str> {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    test_snippet(
        source,
        &LinterSettings::for_rule(Rule::SudoRedirectionOrder),
    )
    .iter()
    .map(|diagnostic| diagnostic.span.slice(source))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::operator_slices;
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_default_output_and_input_redirect_operators() {
        let source = "\
#!/bin/bash
sudo printf '%s\\n' ok > out.txt >> log.txt < input.txt
";
        assert_eq!(operator_slices(source), vec![">", ">>", "<"]);
    }

    #[test]
    fn reports_tee_input_redirects_but_skips_dev_null_sink() {
        let source = "\
#!/bin/bash
sudo tee /tmp/out < input.txt >/dev/null
";
        assert_eq!(operator_slices(source), vec!["<"]);
    }

    #[test]
    fn skips_explicit_file_descriptors_and_dev_null_input() {
        let source = "\
#!/bin/bash
sudo printf '%s\\n' ok 1> out.txt 2>> err.txt 0< input.txt
sudo cat < /dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SudoRedirectionOrder),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_bash_output_both_on_the_output_operator() {
        let source = "#!/bin/bash\nsudo cat &> out.txt\n";
        assert_eq!(operator_slices(source), vec![">"]);
    }
}
