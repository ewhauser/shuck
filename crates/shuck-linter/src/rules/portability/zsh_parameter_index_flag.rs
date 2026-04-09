use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

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
        .flat_map(|fact| index_flag_spans(fact.word().span.slice(checker.source()), fact.word().span))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshParameterIndexFlag);
}

fn index_flag_spans(text: &str, span: Span) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut search_from = 0usize;

    while let Some(relative) = text[search_from..].find("[(") {
        let start = search_from + relative;
        let before = &text[..start];
        if !before.contains("${") {
            search_from = start + 2;
            continue;
        }

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
        if text.get(close_bracket + 1..close_bracket + 2) != Some("}") {
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

fn targets_non_zsh_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
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
            &LinterSettings::for_rule(Rule::ZshParameterIndexFlag)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
