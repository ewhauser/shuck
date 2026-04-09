use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ZshPromptBracket;

impl Violation for ZshPromptBracket {
    fn rule() -> Rule {
        Rule::ZshPromptBracket
    }

    fn message(&self) -> String {
        "zsh prompt escape syntax is not portable to this shell".to_owned()
    }
}

pub fn zsh_prompt_bracket(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| prompt_bracket_spans(fact.word().span.slice(checker.source()), fact.word().span))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshPromptBracket);
}

fn prompt_bracket_spans(text: &str, span: Span) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut search_from = 0usize;

    while let Some(relative_start) = text[search_from..].find("%{") {
        let start = search_from + relative_start;
        let after_start = start + 2;
        if !text[after_start..].contains("%}") {
            search_from = after_start;
            continue;
        }

        let start_position = span.start.advanced_by(&text[..start]);
        spans.push(Span::from_positions(
            start_position,
            start_position.advanced_by("%{"),
        ));
        search_from = after_start;
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
    fn ignores_words_without_closing_prompt_escape() {
        let source = "#!/bin/sh\nX=\"%{$fg_bold[blue]text\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshPromptBracket));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nX=\"%{$fg_bold[blue]%}text\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshPromptBracket).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
