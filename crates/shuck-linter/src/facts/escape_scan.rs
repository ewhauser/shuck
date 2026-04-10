use shuck_ast::Span;

use super::{BacktickFragmentFact, CommandFact, SingleQuotedFragmentFact, WordFact};
use crate::FileContext;
use crate::context::FileContextTag;
use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::span::{
    word_has_single_literal_part, word_literal_part_spans_excluding_parameter_operator_tails,
    word_literal_scan_segments_excluding_expansions,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapeScanSourceKind {
    WordLiteralPart,
    RedirectLiteralSegment,
    DynamicPathCommandName,
    PatternLiteral,
    PatternCharClass,
    SingleLiteralAssignmentWord,
    BacktickFragment,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EscapeScanMatch {
    span: Span,
    escaped_byte: u8,
    source_kind: EscapeScanSourceKind,
    nested_word_command: bool,
    host_contains_single_quoted_fragment: bool,
    inside_single_quoted_fragment: bool,
}

impl EscapeScanMatch {
    pub(crate) fn span(self) -> Span {
        self.span
    }

    pub(crate) fn escaped_byte(self) -> u8 {
        self.escaped_byte
    }

    pub(crate) fn source_kind(self) -> EscapeScanSourceKind {
        self.source_kind
    }

    pub(crate) fn is_nested_word_command(self) -> bool {
        self.nested_word_command
    }

    pub(crate) fn host_contains_single_quoted_fragment(self) -> bool {
        self.host_contains_single_quoted_fragment
    }

    pub(crate) fn inside_single_quoted_fragment(self) -> bool {
        self.inside_single_quoted_fragment
    }
}

pub(super) fn build_escape_scan_matches(
    commands: &[CommandFact<'_>],
    words: &[WordFact<'_>],
    pattern_literal_spans: &[Span],
    pattern_charclass_spans: &[Span],
    single_quoted_fragments: &[SingleQuotedFragmentFact],
    backtick_fragments: &[BacktickFragmentFact],
    source: &str,
    file_context: &FileContext,
) -> Vec<EscapeScanMatch> {
    if file_context.has_tag(FileContextTag::PatchFile) {
        return Vec::new();
    }

    let mut matches = Vec::new();

    for fact in words
        .iter()
        .filter(|fact| is_relevant_word_context(fact.expansion_context()))
    {
        if is_grep_style_argument(commands, fact) || is_regex_like_context(fact.expansion_context())
        {
            continue;
        }

        let host_contains_single_quoted_fragment =
            span_contains_single_quoted_fragment(fact.span(), single_quoted_fragments);

        for span in word_literal_part_spans_excluding_parameter_operator_tails(fact.word(), source)
        {
            append_escape_scan_matches(
                &mut matches,
                span,
                source,
                EscapeScanSourceKind::WordLiteralPart,
                fact.is_nested_word_command(),
                host_contains_single_quoted_fragment,
                single_quoted_fragments,
            );
        }
    }

    for fact in words
        .iter()
        .filter(|fact| is_assignment_value_context(fact.expansion_context()))
    {
        if !word_has_single_literal_part(fact.word()) || is_grep_style_argument(commands, fact) {
            continue;
        }

        append_escape_scan_matches(
            &mut matches,
            fact.span(),
            source,
            EscapeScanSourceKind::SingleLiteralAssignmentWord,
            fact.is_nested_word_command(),
            span_contains_single_quoted_fragment(fact.span(), single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for fact in words.iter().filter(|fact| {
        matches!(
            fact.expansion_context(),
            Some(ExpansionContext::RedirectTarget(_))
        )
    }) {
        if is_grep_style_argument(commands, fact) || is_regex_like_context(fact.expansion_context())
        {
            continue;
        }

        let host_contains_single_quoted_fragment =
            span_contains_single_quoted_fragment(fact.span(), single_quoted_fragments);

        for span in word_literal_scan_segments_excluding_expansions(fact.word(), source) {
            append_escape_scan_matches(
                &mut matches,
                span,
                source,
                EscapeScanSourceKind::RedirectLiteralSegment,
                fact.is_nested_word_command(),
                host_contains_single_quoted_fragment,
                single_quoted_fragments,
            );
        }
    }

    for command in commands {
        let Some(span) = command
            .body_word_span()
            .filter(|span| span.slice(source).contains('/'))
        else {
            continue;
        };

        append_escape_scan_matches(
            &mut matches,
            span,
            source,
            EscapeScanSourceKind::DynamicPathCommandName,
            command.is_nested_word_command(),
            span_contains_single_quoted_fragment(span, single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for span in pattern_literal_spans {
        append_escape_scan_matches(
            &mut matches,
            *span,
            source,
            EscapeScanSourceKind::PatternLiteral,
            false,
            span_contains_single_quoted_fragment(*span, single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for span in pattern_charclass_spans {
        append_escape_scan_matches(
            &mut matches,
            *span,
            source,
            EscapeScanSourceKind::PatternCharClass,
            false,
            span_contains_single_quoted_fragment(*span, single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for fragment in backtick_fragments {
        append_escape_scan_matches(
            &mut matches,
            fragment.span(),
            source,
            EscapeScanSourceKind::BacktickFragment,
            false,
            span_contains_single_quoted_fragment(fragment.span(), single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    matches
}

fn append_escape_scan_matches(
    matches: &mut Vec<EscapeScanMatch>,
    scan_span: Span,
    source: &str,
    source_kind: EscapeScanSourceKind,
    nested_word_command: bool,
    host_contains_single_quoted_fragment: bool,
    single_quoted_fragments: &[SingleQuotedFragmentFact],
) {
    let text = scan_span.slice(source);
    let bytes = text.as_bytes();
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

        let Some(&escaped_byte) = bytes.get(index) else {
            continue;
        };

        if (index - run_start) % 2 == 0 {
            continue;
        }

        let start = scan_span.start.advanced_by(&text[..index - 1]);
        let report_span = Span::from_positions(start, start);
        matches.push(EscapeScanMatch {
            span: report_span,
            escaped_byte,
            source_kind,
            nested_word_command,
            host_contains_single_quoted_fragment,
            inside_single_quoted_fragment: span_within_single_quoted_fragment(
                report_span,
                single_quoted_fragments,
            ),
        });
    }
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

fn is_assignment_value_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(ExpansionContext::AssignmentValue | ExpansionContext::DeclarationAssignmentValue)
    )
}

fn is_regex_like_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(ExpansionContext::RegexOperand | ExpansionContext::StringTestOperand)
    )
}

fn span_contains_single_quoted_fragment(
    span: Span,
    fragments: &[SingleQuotedFragmentFact],
) -> bool {
    fragments.iter().any(|fragment| {
        let fragment_span = fragment.span();
        fragment_span.start.offset >= span.start.offset
            && fragment_span.end.offset <= span.end.offset
    })
}

fn span_within_single_quoted_fragment(span: Span, fragments: &[SingleQuotedFragmentFact]) -> bool {
    fragments.iter().any(|fragment| {
        let fragment_span = fragment.span();
        span.start.offset >= fragment_span.start.offset
            && span.end.offset <= fragment_span.end.offset
    })
}

fn is_grep_style_argument(commands: &[CommandFact<'_>], fact: &WordFact<'_>) -> bool {
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = &commands[fact.command_id().index()];
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
    use shuck_semantic::SemanticModel;

    use super::EscapeScanSourceKind;
    use crate::{LinterFacts, ShellDialect, classify_file_context};

    fn with_matches(
        source: &str,
        path: Option<&Path>,
        parse_dialect: ParseShellDialect,
        shell: ShellDialect,
        visit: impl FnOnce(&[super::EscapeScanMatch]),
    ) {
        let output = Parser::with_dialect(source, parse_dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, path, shell);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        visit(facts.escape_scan_matches());
    }

    #[test]
    fn records_only_odd_escape_runs_and_char_class_matches() {
        let source = r#"#!/bin/sh
echo foo\_bar
echo foo\\_bar
case x in [a\-z]) : ;; esac
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Posix,
            ShellDialect::Sh,
            |matches| {
                let underscore_word_matches = matches
                    .iter()
                    .filter(|escape| {
                        escape.escaped_byte() == b'_'
                            && escape.source_kind() == EscapeScanSourceKind::WordLiteralPart
                    })
                    .count();

                assert_eq!(underscore_word_matches, 1);
                assert!(matches.iter().any(|escape| {
                    escape.escaped_byte() == b'-'
                        && escape.source_kind() == EscapeScanSourceKind::PatternCharClass
                }));
            },
        );
    }

    #[test]
    fn records_backtick_fragment_matches() {
        let source = r#"#!/bin/sh
`echo \n`
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Posix,
            ShellDialect::Sh,
            |matches| {
                let backtick_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b'n'
                            && escape.source_kind() == EscapeScanSourceKind::BacktickFragment
                    })
                    .expect("expected backtick fragment match");

                assert!(!backtick_match.inside_single_quoted_fragment());
            },
        );
    }

    #[test]
    fn records_nested_word_command_single_quote_metadata() {
        let source = r#"#!/bin/bash
echo "$(printf prefix'quoted'\n)"
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Bash,
            ShellDialect::Bash,
            |matches| {
                let nested_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b'n'
                            && escape.source_kind() == EscapeScanSourceKind::WordLiteralPart
                            && escape.is_nested_word_command()
                            && escape.host_contains_single_quoted_fragment()
                    })
                    .expect("expected nested command word match");

                assert!(nested_match.inside_single_quoted_fragment());
            },
        );
    }
}
