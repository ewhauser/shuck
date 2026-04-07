use shuck_ast::{Command, CompoundCommand, StmtSeq, Word, WordPart};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, LinterFacts, Rule, Violation};

pub struct FindOutputLoop;

impl Violation for FindOutputLoop {
    fn rule() -> Rule {
        Rule::FindOutputLoop
    }

    fn message(&self) -> String {
        "expanding `find` output in a `for` loop splits paths on whitespace".to_owned()
    }
}

pub fn find_output_loop(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            let Command::Compound(CompoundCommand::For(command)) = command else {
                return;
            };

            let Some(words) = &command.words else {
                return;
            };

            for word in words {
                if word_contains_find_substitution(word, checker.facts()) {
                    spans.push(word.span);
                }
            }
        },
    );

    for span in spans {
        checker.report(FindOutputLoop, span);
    }
}

fn word_contains_find_substitution(word: &Word, facts: &LinterFacts<'_>) -> bool {
    word.parts
        .iter()
        .any(|part| part_contains_find_substitution(&part.kind, facts))
}

fn part_contains_find_substitution(part: &WordPart, facts: &LinterFacts<'_>) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_find_substitution(&part.kind, facts)),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            commands_start_with_find(body, facts)
        }
        _ => false,
    }
}

fn commands_start_with_find(commands: &StmtSeq, facts: &LinterFacts<'_>) -> bool {
    matches!(commands.as_slice(), [command] if command_starts_with_find(command, facts))
}

fn command_starts_with_find(command: &shuck_ast::Stmt, facts: &LinterFacts<'_>) -> bool {
    if let Some(segments) = query::pipeline_segments(&command.command) {
        return matches!(segments.as_slice(), [segment] if command_starts_with_find(segment, facts));
    }

    facts
        .command_for_stmt(command)
        .is_some_and(|fact| fact.effective_name_is("find"))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_wrapped_find_substitutions_in_for_loops() {
        let source = "for item in $(command find . -type f); do :; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputLoop));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "$(command find . -type f)"
        );
    }

    #[test]
    fn ignores_non_find_substitutions() {
        let source = "for item in $(command printf '%s\\n' hi); do :; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputLoop));

        assert!(diagnostics.is_empty());
    }
}
