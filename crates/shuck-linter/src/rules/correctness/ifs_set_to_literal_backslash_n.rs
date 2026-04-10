use shuck_ast::{Assignment, AssignmentValue, BuiltinCommand, Command, DeclOperand, Span};

use crate::{Checker, Rule, Violation, static_word_text};

pub struct IfsSetToLiteralBackslashN;

impl Violation for IfsSetToLiteralBackslashN {
    fn rule() -> Rule {
        Rule::IfsSetToLiteralBackslashN
    }

    fn message(&self) -> String {
        "IFS contains a literal \\n sequence".to_owned()
    }
}

pub fn ifs_set_to_literal_backslash_n(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| command_assignment_spans(fact.command(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || IfsSetToLiteralBackslashN);
}

fn command_assignment_spans(command: &Command, source: &str) -> Vec<Span> {
    match command {
        Command::Simple(command) => command
            .assignments
            .iter()
            .filter_map(|assignment| {
                assignment_value_contains_literal_backslash_n(assignment, source)
            })
            .collect(),
        Command::Builtin(command) => builtin_assignments(command)
            .iter()
            .filter_map(|assignment| {
                assignment_value_contains_literal_backslash_n(assignment, source)
            })
            .collect(),
        Command::Decl(command) => command
            .assignments
            .iter()
            .chain(command.operands.iter().filter_map(|operand| match operand {
                DeclOperand::Assignment(assignment) => Some(assignment),
                DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Dynamic(_) => None,
            }))
            .filter_map(|assignment| {
                assignment_value_contains_literal_backslash_n(assignment, source)
            })
            .collect(),
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => Vec::new(),
    }
}

fn builtin_assignments(command: &BuiltinCommand) -> &[Assignment] {
    match command {
        BuiltinCommand::Break(command) => &command.assignments,
        BuiltinCommand::Continue(command) => &command.assignments,
        BuiltinCommand::Return(command) => &command.assignments,
        BuiltinCommand::Exit(command) => &command.assignments,
    }
}

fn assignment_value_contains_literal_backslash_n(
    assignment: &Assignment,
    source: &str,
) -> Option<Span> {
    if assignment.target.name.as_str() != "IFS" {
        return None;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };

    static_word_text(word, source)
        .is_some_and(|text| text.contains("\\n"))
        .then_some(word.span)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_ifs_assignment_values() {
        let source = "\
#!/bin/sh
IFS='\\n'
export IFS=\"x\\n\"
foo() {
  local IFS='\\n\\t'
}
declare IFS='prefix\\nsuffix'
bar='\\n'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IfsSetToLiteralBackslashN),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'\\n'", "\"x\\n\"", "'\\n\\t'", "'prefix\\nsuffix'"]
        );
    }

    #[test]
    fn ignores_non_literal_or_non_ifs_assignments() {
        let source = "\
#!/bin/sh
IFS=$'\\n'
foo='\\n'
bar=bar-n
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IfsSetToLiteralBackslashN),
        );

        assert!(diagnostics.is_empty());
    }
}
