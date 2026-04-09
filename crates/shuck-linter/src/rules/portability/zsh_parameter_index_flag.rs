use shuck_ast::Span;

use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct ZshParameterIndexFlag;

impl Violation for ZshParameterIndexFlag {
    fn rule() -> Rule {
        Rule::ZshParameterIndexFlag
    }

    fn message(&self) -> String {
        "zsh parameter index flags are not portable to this shell".to_owned()
    }
}

pub fn zsh_parameter_index_flag(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| {
            index_flag_spans(fact.word().span.slice(checker.source()), fact.word().span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshParameterIndexFlag);
}

fn index_flag_spans(text: &str, span: Span) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut search_from = 0usize;

    while let Some(relative) = text[search_from..].find("[(") {
        let start = search_from + relative;
        let Some(expansion_start) = innermost_parameter_expansion_start(text, start) else {
            search_from = start + 2;
            continue;
        };

        let Some(close_paren) = text[start + 2..].find(')') else {
            search_from = start + 2;
            continue;
        };
        let close_paren = start + 2 + close_paren;
        if close_paren == start + 2 {
            search_from = start + 2;
            continue;
        }
        if !text[start + 2..close_paren]
            .chars()
            .all(|ch| ch.is_ascii_alphabetic())
        {
            search_from = start + 2;
            continue;
        }
        let Some(close_bracket) = text[close_paren + 1..].find(']') else {
            search_from = close_paren + 1;
            continue;
        };
        let close_bracket = close_paren + 1 + close_bracket;
        if parameter_expansion_end(text, expansion_start) != Some(close_bracket + 1) {
            search_from = close_bracket + 1;
            continue;
        }

        let bracket_start = span.start.advanced_by(&text[..start]);
        spans.push(Span::from_positions(
            bracket_start,
            bracket_start.advanced_by("["),
        ));
        search_from = close_bracket + 1;
    }

    spans
}

fn innermost_parameter_expansion_start(text: &str, end: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut stack = Vec::new();
    let mut index = 0usize;

    while index < end {
        if bytes[index..].starts_with(b"${") {
            stack.push(index);
            index += 2;
            continue;
        }
        if bytes[index] == b'}' {
            stack.pop();
        }
        index += 1;
    }

    stack.last().copied()
}

fn parameter_expansion_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = start;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"${") {
            depth += 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_plain_braced_subscripts_without_flags() {
        let source = "#!/bin/sh\nx=${array[1]}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterIndexFlag),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nx=${\"$(rsync --version 2>&1)\"[(w)3]}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterIndexFlag).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_literal_index_flag_text_after_a_closed_expansion() {
        let source = "#!/bin/sh\nx=\"${a}[(w)3]}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterIndexFlag),
        );

        assert!(diagnostics.is_empty());
    }
}
