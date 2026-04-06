use shuck_ast::{Redirect, RedirectKind};

use crate::rules::common::{
    command::{self, WrapperKind},
    query::{self, CommandWalkOptions},
    span,
    word::static_word_text,
};
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
    let source = checker.source();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let normalized = command::normalize_command(command, source);
            if !normalized.has_wrapper(WrapperKind::SudoFamily) {
                return;
            }

            if normalized.effective_name_is("tee") {
                return;
            }

            for redirect in query::command_redirects(command) {
                if redirects_output_to_file(redirect)
                    && !redirect_target_is_dev_null(redirect, source)
                {
                    checker
                        .report_dedup(SudoRedirectionOrder, span::redirect_target_span(redirect));
                }
            }
        },
    );
}

fn redirects_output_to_file(redirect: &Redirect) -> bool {
    matches!(
        redirect.kind,
        RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::ReadWrite
            | RedirectKind::OutputBoth
    )
}

fn redirect_target_is_dev_null(redirect: &Redirect, source: &str) -> bool {
    static_word_text(&redirect.target, source).as_deref() == Some("/dev/null")
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
}
