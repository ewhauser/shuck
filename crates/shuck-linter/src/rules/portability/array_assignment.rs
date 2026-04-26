use shuck_ast::{ArenaFileCommandKind, AssignmentNode, AssignmentValueNode, DeclOperandNode, Span};

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

fn command_array_assignment_spans(
    command: crate::CommandFactRef<'_, '_>,
    source: &str,
) -> Vec<Span> {
    match command.command_kind() {
        ArenaFileCommandKind::Simple | ArenaFileCommandKind::Builtin => command
            .arena_assignments()
            .iter()
            .filter_map(|assignment| array_assignment_span(assignment, source))
            .collect(),
        ArenaFileCommandKind::Decl => command
            .arena_assignments()
            .iter()
            .chain(command.arena_declaration_operands().iter().filter_map(
                |operand| match operand {
                    DeclOperandNode::Assignment(assignment) => Some(assignment),
                    DeclOperandNode::Flag(_)
                    | DeclOperandNode::Name(_)
                    | DeclOperandNode::Dynamic(_) => None,
                },
            ))
            .filter_map(|assignment| array_assignment_span(assignment, source))
            .collect(),
        ArenaFileCommandKind::Binary
        | ArenaFileCommandKind::Compound
        | ArenaFileCommandKind::Function
        | ArenaFileCommandKind::AnonymousFunction => Vec::new(),
    }
}

fn array_assignment_span(assignment: &AssignmentNode, source: &str) -> Option<Span> {
    match &assignment.value {
        AssignmentValueNode::Compound(_) => {
            let text = assignment.span.slice(source);
            let value_offset = text
                .find("+=")
                .map(|idx| idx + 2)
                .or_else(|| text.find('=').map(|idx| idx + 1))?;
            let value_start = assignment.span.start.advanced_by(&text[..value_offset]);
            Some(Span::from_positions(value_start, assignment.span.end))
        }
        AssignmentValueNode::Scalar(_) => None,
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
