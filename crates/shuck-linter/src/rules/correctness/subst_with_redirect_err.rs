use shuck_ast::WordPart;

use crate::{Checker, Rule, Violation};

use super::subst_with_redirect::{CommandSubstitutionRedirect, command_substitution_redirect};
use super::syntax::{visit_command_words, walk_commands};

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

    walk_commands(&checker.ast().commands, &mut |command, _| {
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
    });

    for span in spans {
        checker.report(SubstWithRedirectErr, span);
    }
}
