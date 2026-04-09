use shuck_ast::{Command, Span, Word};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ZshAssignmentToZero;

impl Violation for ZshAssignmentToZero {
    fn rule() -> Rule {
        Rule::ZshAssignmentToZero
    }

    fn message(&self) -> String {
        "assigning to `0` is a zsh-only pattern".to_owned()
    }
}

pub fn zsh_assignment_to_zero(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Bash {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let words: Vec<&Word> = match fact.command() {
                Command::Simple(command) => std::iter::once(&command.name)
                    .chain(command.args.iter())
                    .collect(),
                Command::Decl(_) => fact.body_args().iter().copied().collect(),
                Command::Builtin(_)
                | Command::Binary(_)
                | Command::Compound(_)
                | Command::Function(_)
                | Command::AnonymousFunction(_) => Vec::new(),
            };

            words
                .into_iter()
                .filter_map(|word| assignment_to_zero_span(word, checker.source()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshAssignmentToZero);
}

fn assignment_to_zero_span(word: &Word, source: &str) -> Option<Span> {
    word.span
        .slice(source)
        .starts_with("0=")
        .then_some(Span::from_positions(word.span.start, word.span.start.advanced_by("0")))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_assignments_to_zero_in_zsh_scripts() {
        let source = "#!/bin/zsh\n0=${(%):-%N}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshAssignmentToZero).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn anchors_on_the_assignment_target_name() {
        let source = "#!/bin/bash\n0=\"$PWD\"\n";
        let diagnostics =
            test_snippet(
                source,
                &LinterSettings::for_rule(Rule::ZshAssignmentToZero)
                    .with_shell(ShellDialect::Bash),
            );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "0");
    }
}
