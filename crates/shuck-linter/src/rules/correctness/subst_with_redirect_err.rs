use shuck_ast::WordPart;

use crate::rules::common::query::{self, CommandWalkOptions, visit_command_words};
use crate::{Checker, Rule, Violation};

use super::subst_with_redirect::{CommandSubstitutionRedirect, command_substitution_redirect};

pub struct SubstWithRedirectErr;

impl Violation for SubstWithRedirectErr {
    fn rule() -> Rule {
        Rule::SubstWithRedirectErr
    }

    fn message(&self) -> String {
        "command substitution redirects its output inside the subshell".to_owned()
    }
}

pub fn subst_with_redirect_err(checker: &mut Checker) {
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
                        == CommandSubstitutionRedirect::DevNull
                    {
                        spans.push(span);
                    }
                }
            });
        },
    );

    for span in spans {
        checker.report(SubstWithRedirectErr, span);
    }
}
