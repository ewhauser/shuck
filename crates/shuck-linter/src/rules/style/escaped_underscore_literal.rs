use shuck_ast::Span;

use crate::context::FileContextTag;
use crate::{
    Checker, ExpansionContext, Rule, Violation,
    word_literal_part_spans_excluding_parameter_operator_tails,
    word_literal_scan_segments_excluding_expansions,
};

pub struct EscapedUnderscoreLiteral;

impl Violation for EscapedUnderscoreLiteral {
    fn rule() -> Rule {
        Rule::EscapedUnderscoreLiteral
    }

    fn message(&self) -> String {
        "a backslash before `_` is unnecessary and becomes a literal underscore".to_owned()
    }
}

pub fn escaped_underscore_literal(checker: &mut Checker) {
    if checker.file_context().has_tag(FileContextTag::PatchFile) {
        return;
    }

    let source = checker.source();
    let facts = checker.facts();
    let single_quoted_fragments = facts.single_quoted_fragments();
    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| is_relevant_word_context(fact.expansion_context()))
        .filter(|fact| !word_contains_single_quoted_fragment(fact.span(), single_quoted_fragments))
        .filter(|fact| !is_grep_style_argument(facts, fact))
        .filter(|fact| {
            !matches!(
                fact.expansion_context(),
                Some(ExpansionContext::RegexOperand | ExpansionContext::StringTestOperand)
            )
        })
        .flat_map(|fact| {
            word_literal_part_spans_excluding_parameter_operator_tails(fact.word(), source)
                .into_iter()
                .flat_map(|span| needless_backslash_spans(span, source))
                .collect::<Vec<_>>()
        })
        .chain(
            checker
                .facts()
                .word_facts()
                .iter()
                .filter(|fact| {
                    matches!(
                        fact.expansion_context(),
                        Some(ExpansionContext::RedirectTarget(_))
                    )
                })
                .filter(|fact| {
                    !word_contains_single_quoted_fragment(fact.span(), single_quoted_fragments)
                })
                .filter(|fact| !is_grep_style_argument(facts, fact))
                .filter(|fact| {
                    !matches!(
                        fact.expansion_context(),
                        Some(ExpansionContext::RegexOperand | ExpansionContext::StringTestOperand)
                    )
                })
                .flat_map(|fact| redirect_target_needless_backslash_spans(fact, source)),
        )
        .chain(
            facts
                .commands()
                .iter()
                .filter_map(|command| {
                    command
                        .body_name_word()
                        .filter(|word| word.span.slice(source).contains('/'))
                        .map(|word| needless_backslash_spans(word.span, source))
                })
                .flatten(),
        )
        .chain(
            facts
                .pattern_literal_spans()
                .iter()
                .copied()
                .flat_map(|span| needless_backslash_spans(span, source)),
        )
        .chain(
            facts
                .pattern_charclass_spans()
                .iter()
                .copied()
                .flat_map(|span| needless_backslash_spans_in_char_class(span, source)),
        )
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EscapedUnderscoreLiteral);
}

fn is_relevant_word_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(
            ExpansionContext::CommandArgument
                | ExpansionContext::AssignmentValue
                | ExpansionContext::DeclarationAssignmentValue
                | ExpansionContext::RedirectTarget(_)
                | ExpansionContext::ForList
                | ExpansionContext::SelectList
                | ExpansionContext::CasePattern
        )
    )
}

fn word_contains_single_quoted_fragment(
    word_span: Span,
    fragments: &[crate::facts::SingleQuotedFragmentFact],
) -> bool {
    fragments.iter().any(|fragment| {
        let fragment_span = fragment.span();
        fragment_span.start.offset >= word_span.start.offset
            && fragment_span.end.offset <= word_span.end.offset
    })
}

fn redirect_target_needless_backslash_spans(
    fact: &crate::facts::WordFact<'_>,
    source: &str,
) -> Vec<Span> {
    word_literal_scan_segments_excluding_expansions(fact.word(), source)
        .into_iter()
        .flat_map(|span| needless_backslash_spans(span, source))
        .collect()
}

fn is_grep_style_argument<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: &crate::facts::WordFact<'a>,
) -> bool {
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = facts.command(fact.command_id());
    if command
        .body_name_word()
        .is_some_and(|word| word.span == fact.span())
    {
        return false;
    }

    command
        .effective_or_literal_name()
        .is_some_and(|name| name.contains("grep"))
}

fn needless_backslash_spans(word_span: Span, source: &str) -> Vec<Span> {
    let text = word_span.slice(source);
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        if text
            .as_bytes()
            .get(index)
            .is_some_and(|byte| is_needless_backslash_target(*byte))
            && (index - run_start) % 2 == 1
        {
            let start = word_span.start.advanced_by(&text[..index - 1]);
            spans.push(Span::from_positions(start, start));
        }
    }

    spans
}

fn needless_backslash_spans_in_char_class(span: Span, source: &str) -> Vec<Span> {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        if text.as_bytes().get(index).is_some_and(|byte| *byte == b'-')
            && (index - run_start) % 2 == 1
        {
            let start = span.start.advanced_by(&text[..index - 1]);
            spans.push(Span::from_positions(start, start));
        }
    }

    spans
}

fn is_needless_backslash_target(byte: u8) -> bool {
    byte == b'_'
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_indexer::Indexer;
    use shuck_parser::parser::{ParseOutput, Parser, ShellDialect as ParseDialect};

    use crate::test::test_snippet;
    use crate::{
        Diagnostic, LinterSettings, Rule, ShellDialect, lint_file_at_path_with_parse_diagnostics,
    };

    fn test_posix_snippet_at_path(path: &Path, source: &str) -> Vec<Diagnostic> {
        let recovered = Parser::with_dialect(source, ParseDialect::Posix).parse_recovered();
        let output = ParseOutput {
            file: recovered.file,
        };
        let indexer = Indexer::new(source, &output);
        let settings =
            LinterSettings::for_rule(Rule::EscapedUnderscoreLiteral).with_shell(ShellDialect::Sh);
        lint_file_at_path_with_parse_diagnostics(
            &output.file,
            source,
            &indexer,
            &settings,
            None,
            Some(path),
            &recovered.diagnostics,
        )
    }

    #[test]
    fn reports_needless_backslashes_before_underscores() {
        let source = "\
#!/bin/bash
echo foo\\_bar
echo foo\\\\_bar
echo \"foo\\_bar\"
echo prefix\"\\_\"suffix
foo=${x#foo\\_bar}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedUnderscoreLiteral),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![""]
        );
    }

    #[test]
    fn reports_redirect_target_underscores() {
        let source = "\
base64 -d ${vkb64} > ${rootfs}/var/db/xbps/keys/60\\:ae\\:0c\\:d6\\:f0\\:95\\:17\\:80\\:bc\\:93\\:46\\:7a\\:89\\:af\\:a3\\:2d.plist
";
        let diagnostics = test_posix_snippet_at_path(Path::new("/tmp/lxc-void"), source);

        assert!(diagnostics.is_empty());
    }
}
