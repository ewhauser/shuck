use shuck_ast::{Assignment, AssignmentValue, BuiltinCommand, Command, DeclOperand, Span};

use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

pub struct AppendWithEscapedQuotes;

impl Violation for AppendWithEscapedQuotes {
    fn rule() -> Rule {
        Rule::AppendWithEscapedQuotes
    }

    fn message(&self) -> String {
        "escaped quotes in `+=` text will stay literal".to_owned()
    }
}

pub fn append_with_escaped_quotes(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| command_assignment_spans(checker, fact.command(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AppendWithEscapedQuotes);
}

fn command_assignment_spans(checker: &Checker<'_>, command: &Command, source: &str) -> Vec<Span> {
    match command {
        Command::Simple(command) => command
            .assignments
            .iter()
            .filter_map(|assignment| {
                escaped_quote_append_span(
                    checker,
                    assignment,
                    source,
                    WordFactContext::Expansion(ExpansionContext::AssignmentValue),
                )
            })
            .collect(),
        Command::Builtin(command) => builtin_assignments(command)
            .iter()
            .filter_map(|assignment| {
                escaped_quote_append_span(
                    checker,
                    assignment,
                    source,
                    WordFactContext::Expansion(ExpansionContext::AssignmentValue),
                )
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
                escaped_quote_append_span(
                    checker,
                    assignment,
                    source,
                    WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue),
                )
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

fn escaped_quote_append_span(
    checker: &Checker<'_>,
    assignment: &Assignment,
    source: &str,
    context: WordFactContext,
) -> Option<Span> {
    if !assignment.append || assignment.target.subscript.is_some() {
        return None;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };

    let fact = checker.facts().word_fact(word.span, context)?;
    let classification = fact.classification();
    let text = word.span.slice(source);
    let first = text.find("\\\"")?;
    if !classification.has_scalar_expansion() || classification.has_command_substitution() {
        return None;
    }
    if !has_later_unquoted_command_argument_use(checker, assignment) {
        return None;
    }

    let end = word.span.start.advanced_by(&text[..first + 2]);
    Some(Span::from_positions(word.span.start, end))
}

fn has_later_unquoted_command_argument_use(checker: &Checker<'_>, assignment: &Assignment) -> bool {
    checker.facts().has_later_unquoted_command_argument_use(
        &assignment.target.name,
        assignment.target.name_span.start.offset,
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_escaped_quote_segments_in_append_assignments() {
        let source = "\
#!/bin/sh
CFLAGS+=\" -DDIR=\\\"$PREFIX/share/\\\"\"\n\
$CC $CFLAGS -c test.c -o test.o\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\" -DDIR=\\\"");
    }

    #[test]
    fn ignores_non_append_and_non_escaped_quote_assignments() {
        let source = "\
#!/bin/sh
CFLAGS=\" -DDIR=\\\"$PREFIX/share/\\\"\"\nCFLAGS+=\" -DDIR=$PREFIX/share/\"\nCFLAGS+=\" -DDIR=\\\"arm\\\"\"\nshell+=$(printf '%s' \"\\\"$PREFIX\\\"\")\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_append_strings_only_reused_as_quoted_arguments() {
        let source = "\
#!/bin/bash
GO_LDFLAGS=\"\"
GO_LDFLAGS+=\" -X \\\"main.GitVersion=$VERSION\\\"\"\n\
go build -ldflags \"$GO_LDFLAGS\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_escaped_quote_appends_without_later_unquoted_argument_use() {
        let source = "\
#!/bin/sh
echo $CFLAGS
CFLAGS+=\" -DDIR=\\\"$PREFIX/share/\\\"\"\n\
printf '%s\\n' \"$CFLAGS\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_subscripted_appends() {
        let source = "\
#!/bin/bash
DEPENDENTS[0]+=\" --slave \\\"$prefix/bin/tool\\\" \\\"tool\\\" \\\"$prefix/bin/tool-real\\\"\"\n\
printf '%s\\n' \"${DEPENDENTS[0]}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendWithEscapedQuotes),
        );

        assert!(diagnostics.is_empty());
    }
}
