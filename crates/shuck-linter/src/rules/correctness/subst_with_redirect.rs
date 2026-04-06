use shuck_ast::{Command, Redirect, RedirectKind, WordPart};

use crate::rules::common::query::{
    self, CommandWalkOptions, visit_command_redirects, visit_command_words,
};
use crate::{Checker, Rule, Violation};

use super::syntax::static_word_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionRedirect {
    None,
    DevNull,
    Other,
}

pub struct SubstWithRedirect;

impl Violation for SubstWithRedirect {
    fn rule() -> Rule {
        Rule::SubstWithRedirect
    }

    fn message(&self) -> String {
        "command substitution redirects its output away".to_owned()
    }
}

pub fn subst_with_redirect(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            visit_command_words(command, &mut |word| {
                for (part, span) in word.parts_with_spans() {
                    let WordPart::CommandSubstitution(commands) = part else {
                        continue;
                    };

                    if command_substitution_redirect(commands, source)
                        == CommandSubstitutionRedirect::Other
                    {
                        spans.push(span);
                    }
                }
            });
        },
    );

    for span in spans {
        checker.report(SubstWithRedirect, span);
    }
}

pub fn command_substitution_redirect(
    commands: &[Command],
    source: &str,
) -> CommandSubstitutionRedirect {
    let mut kind = CommandSubstitutionRedirect::None;

    query::walk_commands(
        commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            visit_command_redirects(command, &mut |redirect| {
                if !redirects_stdout(redirect) {
                    return;
                }

                if redirect_target_is_dev_null(redirect, source) {
                    kind = CommandSubstitutionRedirect::DevNull;
                } else if kind == CommandSubstitutionRedirect::None {
                    kind = CommandSubstitutionRedirect::Other;
                }
            });
        },
    );

    kind
}

fn redirects_stdout(redirect: &Redirect) -> bool {
    match redirect.kind {
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::DupOutput => redirect.fd.unwrap_or(1) == 1,
        RedirectKind::OutputBoth => true,
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupInput => false,
    }
}

fn redirect_target_is_dev_null(redirect: &Redirect, source: &str) -> bool {
    static_word_text(&redirect.target, source).as_deref() == Some("/dev/null")
}
