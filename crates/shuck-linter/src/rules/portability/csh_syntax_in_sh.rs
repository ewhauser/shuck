use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation, static_word_text};

pub struct CshSyntaxInSh;

impl Violation for CshSyntaxInSh {
    fn rule() -> Rule {
        Rule::CshSyntaxInSh
    }

    fn message(&self) -> String {
        "csh-style `set name = value` syntax is not portable to this shell".to_owned()
    }
}

pub fn csh_syntax_in_sh(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("set"))
        .filter_map(|fact| {
            let [name_word, value_word, ..] = fact.body_args() else {
                return None;
            };
            let name = static_word_text(name_word, checker.source())?;
            is_shell_name(&name).then_some(())?;

            let value = static_word_text(value_word, checker.source())?;
            value.starts_with('=').then_some(Span::from_positions(
                value_word_start(value_word),
                value_word_start(value_word).advanced_by("="),
            ))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CshSyntaxInSh);
}

fn value_word_start(word: &shuck_ast::Word) -> shuck_ast::Position {
    word.span.start
}

fn is_shell_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
    fn ignores_regular_set_usage() {
        let source = "#!/bin/sh\nset -- path = /usr/bin\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CshSyntaxInSh));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nset path = ( /usr/bin )\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CshSyntaxInSh).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
