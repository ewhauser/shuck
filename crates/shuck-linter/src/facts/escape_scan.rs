use shuck_ast::Span;

use super::{BacktickFragmentFact, CommandFact, SingleQuotedFragmentFact, WordFact};
use crate::FileContext;
use crate::context::FileContextTag;
use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::span::{
    word_has_single_literal_part, word_literal_part_spans_excluding_parameter_operator_tails,
    word_literal_scan_segments_excluding_expansions,
};

pub(super) struct EscapeScanContext<'a> {
    pub(super) source: &'a str,
    pub(super) file_context: &'a FileContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapeScanSourceKind {
    WordLiteralPart,
    RedirectLiteralSegment,
    DynamicPathCommandName,
    PatternLiteral,
    PatternCharClass,
    ParameterPatternCharClass,
    SingleLiteralAssignmentWord,
    BacktickFragment,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EscapeScanMatch {
    span: Span,
    escaped_byte: u8,
    source_kind: EscapeScanSourceKind,
    grep_style_argument: bool,
    tr_operand_argument: bool,
    #[cfg_attr(not(test), allow(dead_code))]
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

    pub(crate) fn is_grep_style_argument(self) -> bool {
        self.grep_style_argument
    }

    pub(crate) fn is_tr_operand_argument(self) -> bool {
        self.tr_operand_argument
    }

    #[cfg_attr(not(test), allow(dead_code))]
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
    parameter_pattern_spans: &[Span],
    single_quoted_fragments: &[SingleQuotedFragmentFact],
    backtick_fragments: &[BacktickFragmentFact],
    context: EscapeScanContext<'_>,
) -> Vec<EscapeScanMatch> {
    if context.file_context.has_tag(FileContextTag::PatchFile) {
        return Vec::new();
    }

    let mut matches = Vec::new();

    for fact in words
        .iter()
        .filter(|fact| is_relevant_word_context(fact.expansion_context()))
    {
        let grep_style_argument = is_grep_style_argument(commands, fact);
        let tr_operand_argument = is_tr_operand_argument(commands, fact);
        if is_regex_like_context(fact.expansion_context()) {
            continue;
        }

        let host_contains_single_quoted_fragment =
            span_contains_single_quoted_fragment(fact.span(), single_quoted_fragments);

        for span in
            word_literal_part_spans_excluding_parameter_operator_tails(fact.word(), context.source)
        {
            append_escape_scan_matches(
                &mut matches,
                span,
                context.source,
                EscapeScanSourceKind::WordLiteralPart,
                grep_style_argument,
                tr_operand_argument,
                host_contains_single_quoted_fragment,
                single_quoted_fragments,
            );
        }
    }

    for fact in words
        .iter()
        .filter(|fact| is_assignment_value_context(fact.expansion_context()))
    {
        if !word_has_single_literal_part(fact.word()) {
            continue;
        }

        append_escape_scan_matches(
            &mut matches,
            fact.span(),
            context.source,
            EscapeScanSourceKind::SingleLiteralAssignmentWord,
            is_grep_style_argument(commands, fact),
            is_tr_operand_argument(commands, fact),
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
        let grep_style_argument = is_grep_style_argument(commands, fact);
        let tr_operand_argument = is_tr_operand_argument(commands, fact);
        if is_regex_like_context(fact.expansion_context()) {
            continue;
        }

        let host_contains_single_quoted_fragment =
            span_contains_single_quoted_fragment(fact.span(), single_quoted_fragments);

        for span in word_literal_scan_segments_excluding_expansions(fact.word(), context.source) {
            append_escape_scan_matches(
                &mut matches,
                span,
                context.source,
                EscapeScanSourceKind::RedirectLiteralSegment,
                grep_style_argument,
                tr_operand_argument,
                host_contains_single_quoted_fragment,
                single_quoted_fragments,
            );
        }
    }

    for command in commands {
        let Some(span) = command
            .body_word_span()
            .filter(|span| span.slice(context.source).contains('/'))
        else {
            continue;
        };

        append_escape_scan_matches(
            &mut matches,
            span,
            context.source,
            EscapeScanSourceKind::DynamicPathCommandName,
            false,
            false,
            span_contains_single_quoted_fragment(span, single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for span in pattern_literal_spans {
        append_escape_scan_matches(
            &mut matches,
            *span,
            context.source,
            EscapeScanSourceKind::PatternLiteral,
            false,
            false,
            span_contains_single_quoted_fragment(*span, single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for span in pattern_charclass_spans {
        let source_kind = if span_within_any(*span, parameter_pattern_spans) {
            EscapeScanSourceKind::ParameterPatternCharClass
        } else {
            EscapeScanSourceKind::PatternCharClass
        };
        append_escape_scan_matches(
            &mut matches,
            *span,
            context.source,
            source_kind,
            false,
            false,
            span_contains_single_quoted_fragment(*span, single_quoted_fragments),
            single_quoted_fragments,
        );
    }

    for fragment in backtick_fragments {
        append_escape_scan_matches(
            &mut matches,
            fragment.span(),
            context.source,
            EscapeScanSourceKind::BacktickFragment,
            false,
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
    grep_style_argument: bool,
    tr_operand_argument: bool,
    host_contains_single_quoted_fragment: bool,
    single_quoted_fragments: &[SingleQuotedFragmentFact],
) {
    let text = scan_span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;

    while index < bytes.len() {
        if in_single_quotes {
            if bytes[index] == b'\'' {
                in_single_quotes = false;
            }
            index += 1;
            continue;
        }

        if in_double_quotes {
            match bytes[index] {
                b'"' => {
                    in_double_quotes = false;
                    index += 1;
                }
                b'\\' => {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                }
                _ => index += 1,
            }
            continue;
        }

        match bytes[index] {
            b'\'' => {
                in_single_quotes = true;
                index += 1;
                continue;
            }
            b'"' => {
                in_double_quotes = true;
                index += 1;
                continue;
            }
            b'\\' => {}
            _ => {
                index += 1;
                continue;
            }
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
            grep_style_argument,
            tr_operand_argument,
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
            && span.end.offset < fragment_span.end.offset
    })
}

fn span_within_any(span: Span, hosts: &[Span]) -> bool {
    hosts
        .iter()
        .any(|host| span.start.offset >= host.start.offset && span.end.offset <= host.end.offset)
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

fn is_tr_operand_argument(commands: &[CommandFact<'_>], fact: &WordFact<'_>) -> bool {
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    commands[fact.command_id().index()]
        .options()
        .tr()
        .is_some_and(|tr| {
            tr.operand_words()
                .iter()
                .any(|word| word.span == fact.span())
        })
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
    fn keeps_adjacent_escapes_outside_single_quoted_fragments() {
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
                            && escape.host_contains_single_quoted_fragment()
                    })
                    .expect("expected nested command word match");

                assert!(!nested_match.inside_single_quoted_fragment());
            },
        );
    }

    #[test]
    fn marks_grep_style_arguments_without_dropping_the_match() {
        let source = r#"#!/bin/sh
grep foo\tbar file
echo foo\tbar
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Posix,
            ShellDialect::Sh,
            |matches| {
                let grep_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b't'
                            && escape.span().start.line == 2
                            && escape.source_kind() == EscapeScanSourceKind::WordLiteralPart
                    })
                    .expect("expected grep argument match");
                assert!(grep_match.is_grep_style_argument());

                let echo_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b't'
                            && escape.span().start.line == 3
                            && escape.source_kind() == EscapeScanSourceKind::WordLiteralPart
                    })
                    .expect("expected ordinary argument match");
                assert!(!echo_match.is_grep_style_argument());
            },
        );
    }

    #[test]
    fn marks_tr_operands_without_dropping_the_match() {
        let source = r#"#!/bin/bash
printf '%s\n' "$value" | tr \. _
echo foo\.bar
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Bash,
            ShellDialect::Bash,
            |matches| {
                let tr_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b'.'
                            && escape.span().start.line == 2
                            && escape.source_kind() == EscapeScanSourceKind::WordLiteralPart
                    })
                    .expect("expected tr operand match");
                assert!(tr_match.is_tr_operand_argument());

                let echo_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b'.'
                            && escape.span().start.line == 3
                            && escape.source_kind() == EscapeScanSourceKind::WordLiteralPart
                    })
                    .expect("expected ordinary word match");
                assert!(!echo_match.is_tr_operand_argument());
            },
        );
    }

    #[test]
    fn distinguishes_parameter_expansion_char_classes() {
        let source = r#"#!/bin/bash
case "$x" in [a\-z]) : ;; esac
name="${name//[^a-zA-Z0-9_\-]/}"
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Bash,
            ShellDialect::Bash,
            |matches| {
                let conditional_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b'-'
                            && escape.span().start.line == 2
                            && escape.source_kind() == EscapeScanSourceKind::PatternCharClass
                    })
                    .expect("expected case-pattern char class match");

                let parameter_match = matches
                    .iter()
                    .copied()
                    .find(|escape| {
                        escape.escaped_byte() == b'-'
                            && escape.span().start.line == 3
                            && escape.source_kind()
                                == EscapeScanSourceKind::ParameterPatternCharClass
                    })
                    .expect("expected parameter-pattern char class match");

                assert_eq!(conditional_match.escaped_byte(), b'-');
                assert_eq!(parameter_match.escaped_byte(), b'-');
            },
        );
    }

    #[test]
    fn skips_backslashes_inside_double_quotes_when_scanning_raw_fragments() {
        let source = r#"#!/bin/sh
ALL_JARS=`ls *.jar | tr "\n" " "`
cat < "\n"
"#;

        with_matches(
            source,
            None,
            ParseShellDialect::Posix,
            ShellDialect::Sh,
            |matches| {
                assert!(!matches.iter().any(|escape| {
                    escape.escaped_byte() == b'n'
                        && matches!(
                            escape.source_kind(),
                            EscapeScanSourceKind::BacktickFragment
                                | EscapeScanSourceKind::RedirectLiteralSegment
                        )
                }));
            },
        );
    }
}
