use shuck_ast::{Command, Redirect, RedirectKind, Word, WordPart};

use crate::rules::common::{
    command::{self, WrapperKind},
    query::{self, CommandWalkOptions},
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
    let mut spans = Vec::new();

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

            let has_hazardous_redirect = query::command_redirects(command).iter().any(|redirect| {
                redirects_output_to_file(redirect) && !redirect_target_is_dev_null(redirect, source)
            });
            if !has_hazardous_redirect {
                return;
            }

            let span = match command {
                Command::Simple(command) => command.span,
                Command::Builtin(_)
                | Command::Decl(_)
                | Command::Pipeline(_)
                | Command::List(_)
                | Command::Compound(_, _)
                | Command::Function(_) => normalized.body_span,
            };
            spans.push(span);
        },
    );

    for span in spans {
        checker.report(SudoRedirectionOrder, span);
    }
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

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}
