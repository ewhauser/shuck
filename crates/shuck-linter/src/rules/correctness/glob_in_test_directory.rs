use shuck_ast::{ConditionalUnaryOp, Span, Word, static_word_text};

use crate::{
    Checker, ConditionalFact, ConditionalNodeFact, Diagnostic, Edit, Fix, FixAvailability,
    LinterFacts, Rule, SimpleTestFact, SimpleTestShape, Violation,
};

pub struct GlobInTestDirectory;

impl Violation for GlobInTestDirectory {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobInTestDirectory
    }

    fn message(&self) -> String {
        "unquoted globs in file tests can match multiple paths".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the file-test operand".to_owned())
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

    for span in spans {
        checker.report_diagnostic_dedup(Diagnostic::new(GlobInTestDirectory, span).with_fix(
            Fix::unsafe_edit(Edit::replacement(
                format!("\"{}\"", span.slice(source)),
                span,
            )),
        ));
    }
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
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn applies_unsafe_fix_to_unquoted_file_test_globs() {
        let source = "\
#!/bin/bash
[ -d mtp2* ]
[ -e bar* -a -L baz* ]
[[ -r qux* && -w quux* ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestDirectory),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 5);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ -d \"mtp2*\" ]
[ -e \"bar*\" -a -L \"baz*\" ]
[[ -r \"qux*\" && -w \"quux*\" ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_quoted_globs_and_non_file_tests_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[ -d \"mtp2*\" ]
test -n mtp2*
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestDirectory),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C102.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobInTestDirectory),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C102_fix_C102.sh", result);
        Ok(())
    }
}
