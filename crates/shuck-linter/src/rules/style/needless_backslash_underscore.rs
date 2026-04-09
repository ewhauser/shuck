use shuck_ast::Span;

use crate::context::FileContextTag;
use crate::{
    Checker, ExpansionContext, Rule, Violation, word_has_single_literal_part,
    word_literal_part_spans_excluding_parameter_operator_tails,
    word_literal_scan_segments_excluding_expansions,
};

pub struct NeedlessBackslashUnderscore;

impl Violation for NeedlessBackslashUnderscore {
    fn rule() -> Rule {
        Rule::NeedlessBackslashUnderscore
    }

    fn message(&self) -> String {
        "a backslash before n, r, or t is literal".to_owned()
    }
}

pub fn needless_backslash_underscore(checker: &mut Checker) {
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
        .filter(|fact| !fact.is_nested_word_command())
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
                        Some(
                            ExpansionContext::AssignmentValue
                                | ExpansionContext::DeclarationAssignmentValue
                        )
                    )
                })
                .filter(|fact| word_has_single_literal_part(fact.word()))
                .filter(|fact| {
                    !word_contains_single_quoted_fragment(fact.span(), single_quoted_fragments)
                })
                .filter(|fact| !is_grep_style_argument(facts, fact))
                .flat_map(|fact| needless_backslash_spans(fact.span(), source)),
        )
        .chain(
            facts
                .backtick_fragments()
                .iter()
                .copied()
                .flat_map(|fragment| needless_backslash_spans(fragment.span(), source)),
        )
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
                        .body_word_span()
                        .filter(|span| span.slice(source).contains('/'))
                        .map(|span| needless_backslash_spans(span, source))
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
        .filter(|span| !span_within_single_quoted_fragment(*span, single_quoted_fragments))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || NeedlessBackslashUnderscore);
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

fn span_within_single_quoted_fragment(
    span: Span,
    fragments: &[crate::facts::SingleQuotedFragmentFact],
) -> bool {
    fragments.iter().any(|fragment| {
        let fragment_span = fragment.span();
        span.start.offset >= fragment_span.start.offset
            && span.end.offset <= fragment_span.end.offset
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

        if text
            .as_bytes()
            .get(index)
            .is_some_and(|byte| is_needless_backslash_target(*byte))
            && (index - run_start) % 2 == 1
        {
            let start = span.start.advanced_by(&text[..index - 1]);
            spans.push(Span::from_positions(start, start));
        }
    }

    spans
}

fn is_needless_backslash_target(byte: u8) -> bool {
    matches!(byte, b'n' | b'r' | b't')
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_needless_backslashes_before_newline_style_letters() {
        let source = "\
#!/bin/sh
echo \\n
echo foo\\nbar
case x in foo\\t) : ;; esac
cat < foo\\nbar
`echo \\n`
echo \"\\n\"
echo '\\n'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NeedlessBackslashUnderscore),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["", "", "", "", ""]
        );
    }

    #[test]
    fn ignores_single_quoted_fragments_inside_nested_command_substitutions() {
        let source = "\
#!/bin/bash
if [[ \"$TERMUX_APP_PACKAGE_MANAGER\" == \"apt\" ]] && \"$(dpkg-query -W -f '${db:Status-Status}\\n' cabal-install 2>/dev/null)\" != \"installed\"; then
  :
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NeedlessBackslashUnderscore),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_dynamic_path_like_command_names() {
        let source = "\
#!/bin/bash
${bindir}/foo\\nbar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NeedlessBackslashUnderscore),
        );

        assert_eq!(diagnostics.len(), 1);
    }
}
