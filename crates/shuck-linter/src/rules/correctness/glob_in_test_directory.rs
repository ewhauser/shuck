use shuck_ast::{ConditionalUnaryOp, Span, Word, static_word_text};

use crate::{
    Checker, ConditionalFact, ConditionalNodeFact, LinterFacts, Rule, SimpleTestFact,
    SimpleTestShape, Violation,
};

pub struct GlobInTestDirectory;

impl Violation for GlobInTestDirectory {
    fn rule() -> Rule {
        Rule::GlobInTestDirectory
    }

    fn message(&self) -> String {
        "unquoted globs in file tests can match multiple paths".to_owned()
    }
}

pub fn glob_in_test_directory(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                spans.extend(simple_test_file_test_spans(
                    simple_test,
                    checker.facts(),
                    source,
                ));
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_file_test_spans(
                    conditional,
                    checker.facts(),
                    source,
                ));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInTestDirectory);
}

fn simple_test_file_test_spans(
    simple_test: &SimpleTestFact<'_>,
    facts: &LinterFacts<'_>,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    if let Some(span) = simple_test_unary_file_test_span(simple_test, facts, source) {
        spans.push(span);
    }
    spans.extend(collect_directory_operand_spans(
        simple_test.operands(),
        facts,
        source,
    ));
    spans
}

fn conditional_file_test_spans(
    conditional: &ConditionalFact<'_>,
    facts: &LinterFacts<'_>,
    source: &str,
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary) if is_file_test_unary_op(unary.op()) => unary
                .operand()
                .word()
                .and_then(|word| reportable_glob_span(word, facts, source)),
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => None,
        })
        .collect()
}

fn is_file_test_unary_op(op: ConditionalUnaryOp) -> bool {
    matches!(
        op,
        ConditionalUnaryOp::Exists
            | ConditionalUnaryOp::RegularFile
            | ConditionalUnaryOp::Directory
            | ConditionalUnaryOp::CharacterSpecial
            | ConditionalUnaryOp::BlockSpecial
            | ConditionalUnaryOp::NamedPipe
            | ConditionalUnaryOp::Socket
            | ConditionalUnaryOp::Symlink
            | ConditionalUnaryOp::Sticky
            | ConditionalUnaryOp::SetGroupId
            | ConditionalUnaryOp::SetUserId
            | ConditionalUnaryOp::GroupOwned
            | ConditionalUnaryOp::UserOwned
            | ConditionalUnaryOp::Modified
            | ConditionalUnaryOp::Readable
            | ConditionalUnaryOp::Writable
            | ConditionalUnaryOp::Executable
            | ConditionalUnaryOp::NonEmptyFile
    )
}

fn collect_directory_operand_spans(
    operands: &[&Word],
    facts: &LinterFacts<'_>,
    source: &str,
) -> Vec<Span> {
    let operand_texts = operands
        .iter()
        .map(|word| static_word_text(word, source))
        .collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < operands.len() {
        while index < operands.len() && is_simple_test_separator(operand_texts[index].as_deref()) {
            index += 1;
        }

        if index >= operands.len() {
            break;
        }

        if operand_texts[index].as_deref() == Some("!") {
            index += 1;
            continue;
        }

        if is_file_test_operator(operand_texts[index].as_deref()) {
            if index + 1 < operands.len()
                && let Some(span) = reportable_glob_span(operands[index + 1], facts, source)
            {
                spans.push(span);
            }
            index += 2;
            continue;
        }

        index += 1;
    }

    spans
}

fn simple_test_unary_file_test_span(
    simple_test: &SimpleTestFact<'_>,
    facts: &LinterFacts<'_>,
    source: &str,
) -> Option<Span> {
    if simple_test.effective_shape() != SimpleTestShape::Unary {
        return None;
    }

    if !is_simple_test_file_test_unary_operator(
        simple_test
            .effective_operator_word()
            .and_then(|word| static_word_text(word, source))
            .as_deref(),
    ) {
        return None;
    }

    simple_test
        .effective_operands()
        .get(1)
        .and_then(|word| reportable_glob_span(word, facts, source))
}

fn reportable_glob_span(word: &Word, facts: &LinterFacts<'_>, source: &str) -> Option<Span> {
    facts
        .any_word_fact(word.span)
        .is_some_and(|fact| !fact.active_literal_glob_spans(source).is_empty())
        .then_some(word.span)
}

fn is_simple_test_separator(token: Option<&str>) -> bool {
    matches!(token, Some("-a" | "-o" | "(" | ")" | "\\(" | "\\)"))
}

fn is_file_test_operator(token: Option<&str>) -> bool {
    matches!(
        token,
        Some(
            "-e" | "-f"
                | "-d"
                | "-c"
                | "-b"
                | "-p"
                | "-S"
                | "-h"
                | "-L"
                | "-k"
                | "-g"
                | "-u"
                | "-G"
                | "-O"
                | "-N"
                | "-r"
                | "-w"
                | "-x"
                | "-s"
        )
    )
}

fn is_simple_test_file_test_unary_operator(token: Option<&str>) -> bool {
    matches!(token, Some("-a")) || is_file_test_operator(token)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_globs_in_simple_and_conditional_file_tests() {
        let source = "\
#!/bin/bash
[ -d mtp2* ]
test -f foo*
[ -a foo_alias* ]
[ -h link_alias* ]
[ -e bar* -a -L baz* ]
[[ -r qux* && -w quux* ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::GlobInTestDirectory));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "mtp2*",
                "foo*",
                "foo_alias*",
                "link_alias*",
                "bar*",
                "baz*",
                "qux*",
                "quux*",
            ]
        );
    }

    #[test]
    fn ignores_quoted_globs_and_non_file_tests() {
        let source = "\
#!/bin/bash
[ -d \"mtp2*\" ]
[[ -d \"mtp2*\" ]]
test -n mtp2*
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::GlobInTestDirectory));

        assert!(diagnostics.is_empty());
    }
}
