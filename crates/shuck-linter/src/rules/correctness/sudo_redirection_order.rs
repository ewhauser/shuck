use shuck_ast::RedirectKind;

use crate::rules::common::command::WrapperKind;
use crate::{Checker, Rule, Violation};

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
        .filter(|fact| !fact.effective_name_is("tee"))
        .flat_map(|fact| {
            fact.redirect_facts().iter().filter_map(|redirect| {
                (redirects_output_to_file(redirect.redirect().kind)
                    && !redirect
                        .analysis()
                        .is_some_and(|analysis| analysis.is_definitely_dev_null()))
                .then(|| redirect.target_span())
                .flatten()
            })
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(SudoRedirectionOrder, span);
    }
}

fn redirects_output_to_file(kind: RedirectKind) -> bool {
    matches!(
        kind,
        RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::ReadWrite
            | RedirectKind::OutputBoth
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_each_hazardous_redirect_target() {
        let source = "#!/bin/bash\nsudo printf '%s\\n' ok > out.txt 2>> err.log\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SudoRedirectionOrder),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["out.txt", "err.log"]
        );
    }

    #[test]
    fn handles_doas_and_run0_like_sudo() {
        let source = "#!/bin/bash\ndoas printf '%s\\n' ok > out.txt\nrun0 tee out.txt >/dev/null\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SudoRedirectionOrder),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "out.txt");
    }
}
