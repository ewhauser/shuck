use shuck_ast::{
    Assignment, AssignmentValue, BourneParameterExpansion, BuiltinCommand, Command, DeclOperand,
    ParameterExpansion, Span, WordPart, ZshExpansionTarget,
};

use crate::facts::WordFact;
use crate::rules::common::word::{ExpansionContext, WordQuote};
use crate::{Checker, Rule, Violation};

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
            .filter_map(|assignment| escaped_quote_append_span(checker, assignment, source))
            .collect(),
        Command::Builtin(command) => builtin_assignments(command)
            .iter()
            .filter_map(|assignment| escaped_quote_append_span(checker, assignment, source))
            .collect(),
        Command::Decl(command) => command
            .assignments
            .iter()
            .chain(command.operands.iter().filter_map(|operand| match operand {
                DeclOperand::Assignment(assignment) => Some(assignment),
                DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Dynamic(_) => None,
            }))
            .filter_map(|assignment| escaped_quote_append_span(checker, assignment, source))
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
) -> Option<Span> {
    if !assignment.append || assignment.target.subscript.is_some() {
        return None;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };

    let text = word.span.slice(source);
    let first = text.find("\\\"")?;
    if !word_has_parameter_like_part(&word.parts) || word_has_command_substitution(&word.parts) {
        return None;
    }
    if !has_later_unquoted_command_argument_use(checker, assignment) {
        return None;
    }

    let end = word.span.start.advanced_by(&text[..first + 2]);
    Some(Span::from_positions(word.span.start, end))
}

fn word_has_parameter_like_part(parts: &[shuck_ast::WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::Transformation { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => word_has_parameter_like_part(parts),
        _ => false,
    })
}

fn word_has_command_substitution(parts: &[shuck_ast::WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::CommandSubstitution { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => word_has_command_substitution(parts),
        _ => false,
    })
}

fn has_later_unquoted_command_argument_use(checker: &Checker<'_>, assignment: &Assignment) -> bool {
    checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| fact.classification().quote == WordQuote::Unquoted)
        .filter(|fact| fact.span().start.offset > assignment.target.name_span.start.offset)
        .any(|fact| word_references_name(fact, assignment.target.name.as_str()))
}

fn word_references_name(fact: &WordFact<'_>, target_name: &str) -> bool {
    word_parts_reference_name(&fact.word().parts, target_name)
}

fn word_parts_reference_name(parts: &[shuck_ast::WordPartNode], target_name: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Variable(name) => name.as_str() == target_name,
        WordPart::Parameter(expansion) => {
            parameter_expansion_references_name(expansion, target_name)
        }
        WordPart::ParameterExpansion { reference, .. }
        | WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Substring { reference, .. }
        | WordPart::ArraySlice { reference, .. }
        | WordPart::IndirectExpansion { reference, .. }
        | WordPart::Transformation { reference, .. } => reference.name.as_str() == target_name,
        WordPart::DoubleQuoted { parts, .. } => word_parts_reference_name(parts, target_name),
        _ => false,
    })
}

fn parameter_expansion_references_name(expansion: &ParameterExpansion, target_name: &str) -> bool {
    match expansion.bourne() {
        Some(BourneParameterExpansion::Access { reference })
        | Some(BourneParameterExpansion::Length { reference })
        | Some(BourneParameterExpansion::Indices { reference })
        | Some(BourneParameterExpansion::Indirect { reference, .. })
        | Some(BourneParameterExpansion::Slice { reference, .. })
        | Some(BourneParameterExpansion::Operation { reference, .. })
        | Some(BourneParameterExpansion::Transformation { reference, .. }) => {
            reference.name.as_str() == target_name
        }
        Some(BourneParameterExpansion::PrefixMatch { prefix, .. }) => {
            prefix.as_str() == target_name
        }
        None => expansion.zsh().is_some_and(|zsh| match &zsh.target {
            ZshExpansionTarget::Reference(reference) => reference.name.as_str() == target_name,
            ZshExpansionTarget::Nested(expansion) => {
                parameter_expansion_references_name(expansion, target_name)
            }
            ZshExpansionTarget::Word(word) => word_parts_reference_name(&word.parts, target_name),
            ZshExpansionTarget::Empty => false,
        }),
    }
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
