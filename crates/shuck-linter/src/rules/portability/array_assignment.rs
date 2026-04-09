use shuck_ast::{Assignment, AssignmentValue, BuiltinCommand, Command, DeclOperand, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ArrayAssignment;

impl Violation for ArrayAssignment {
    fn rule() -> Rule {
        Rule::ArrayAssignment
    }

    fn message(&self) -> String {
        "array assignment is not portable in `sh`".to_owned()
    }
}

pub fn array_assignment(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| command_array_assignment_spans(command, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayAssignment);
}

fn command_array_assignment_spans(command: &crate::CommandFact<'_>, source: &str) -> Vec<Span> {
    match command.command() {
        Command::Simple(command) => command
            .assignments
            .iter()
            .filter_map(|assignment| array_assignment_span(assignment, source))
            .collect(),
        Command::Builtin(command) => builtin_assignments(command)
            .iter()
            .filter_map(|assignment| array_assignment_span(assignment, source))
            .collect(),
        Command::Decl(command) => command
            .assignments
            .iter()
            .chain(command.operands.iter().filter_map(|operand| match operand {
                DeclOperand::Assignment(assignment) => Some(assignment),
                DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Dynamic(_) => None,
            }))
            .filter_map(|assignment| array_assignment_span(assignment, source))
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

fn array_assignment_span(assignment: &Assignment, source: &str) -> Option<Span> {
    match &assignment.value {
        AssignmentValue::Compound(_) => {
            let text = assignment.span.slice(source);
            let value_offset = text
                .find("+=")
                .map(|idx| idx + 2)
                .or_else(|| text.find('=').map(|idx| idx + 1))?;
            let value_start = assignment.span.start.advanced_by(&text[..value_offset]);
            Some(Span::from_positions(value_start, assignment.span.end))
        }
        AssignmentValue::Scalar(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_array_assignment_literals() {
        let source = "\
#!/bin/sh
items=(one two)
export visible=(left right)
returnable() {
  export nested=(inside)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayAssignment));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["(one two)", "(left right)", "(inside)"]
        );
    }

    #[test]
    fn ignores_scalar_assignments_and_bash_scripts() {
        let source = "\
#!/bin/sh
scalar=value
export other=still_scalar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayAssignment));
        assert!(diagnostics.is_empty());

        let bash_source = "#!/bin/bash\nitems=(one two)\n";
        let diagnostics = test_snippet(
            bash_source,
            &LinterSettings::for_rule(Rule::ArrayAssignment).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
