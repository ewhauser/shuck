use shuck_ast::{ArenaFileCommandKind, AssignmentNode, Span};

use crate::{Checker, FactWordRef, Rule, ShellDialect, Violation};

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
        .flat_map(|fact| match fact.command_kind() {
            ArenaFileCommandKind::Simple => fact
                .arena_assignments()
                .iter()
                .filter_map(typed_assignment_to_zero_span)
                .chain(
                    fact.arena_body_name_word(checker.source())
                        .and_then(|word| assignment_like_word_span(word, checker.source())),
                )
                .collect::<Vec<_>>(),
            ArenaFileCommandKind::Decl => fact
                .arena_body_args(checker.source())
                .into_iter()
                .filter_map(|word| assignment_like_word_span(word, checker.source()))
                .collect::<Vec<_>>(),
            ArenaFileCommandKind::Builtin
            | ArenaFileCommandKind::Binary
            | ArenaFileCommandKind::Compound
            | ArenaFileCommandKind::Function
            | ArenaFileCommandKind::AnonymousFunction => Vec::new(),
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshAssignmentToZero);
}

fn assignment_like_word_span(word: FactWordRef<'_>, source: &str) -> Option<Span> {
    word.span()
        .slice(source)
        .starts_with("0=")
        .then_some(Span::from_positions(
            word.span().start,
            word.span().start.advanced_by("0"),
        ))
}

fn typed_assignment_to_zero_span(assignment: &AssignmentNode) -> Option<Span> {
    (assignment.target.name.as_str() == "0").then_some(Span::from_positions(
        assignment.target.name_span.start,
        assignment.target.name_span.start.advanced_by("0"),
    ))
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
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshAssignmentToZero).with_shell(ShellDialect::Bash),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "0");
    }

    #[test]
    fn ignores_non_assignment_arguments_starting_with_zero_equals() {
        let source = "#!/bin/bash\necho 0=tmp\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshAssignmentToZero).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
