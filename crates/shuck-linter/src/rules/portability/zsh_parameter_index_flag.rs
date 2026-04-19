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
        .zsh_parameter_index_flag_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshParameterIndexFlag);
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
    fn ignores_reference_targets_with_zsh_style_subscripts() {
        let source = "#!/bin/sh\nx=${map[(I)needle]}\ny=\"${precmd_functions[(r)_z_precmd]}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterIndexFlag),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_quoted_word_targets_with_indexing() {
        let source = "#!/bin/sh\nx=${\"$foo\"[1]}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterIndexFlag),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${\"$foo\"");
    }
}
