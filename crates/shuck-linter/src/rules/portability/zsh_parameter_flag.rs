use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct ZshParameterFlag;

impl Violation for ZshParameterFlag {
    fn rule() -> Rule {
        Rule::ZshParameterFlag
    }

    fn message(&self) -> String {
        "this shell can't apply parameter operators directly to command substitutions".to_owned()
    }
}

pub fn zsh_parameter_flag(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .command_substitution_parameter_operation_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshParameterFlag);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_defaulting_and_numeric_slice_forms_on_regular_parameters() {
        let source = "#!/bin/sh\nx=${value:-fallback}\ny=${value:0:1}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_parameter_operators_on_command_substitution_targets() {
        let source = "\
#!/bin/sh
x=${$(svn info):gs/%/%%}
y=${$(svn info):0:1}
z=${$(svn info):-fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].span.slice(source), "${$(svn info)");
        assert_eq!(diagnostics[1].span.slice(source), "${$(svn info)");
        assert_eq!(diagnostics[2].span.slice(source), "${$(svn info)");
    }

    #[test]
    fn ignores_nested_parameter_targets() {
        let source = "\
#!/bin/sh
path=${${(%):-%x}:a:h}
dir=${${custom_datafile:-$HOME/.z}:A}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_simple_non_command_substitution_colon_forms() {
        let source = "#!/bin/sh\nx=${branch:gs/%/%%}\ny=${PWD:h}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nx=${$(svn info):gs/%/%%}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterFlag).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_single_quoted_parameter_flag_text() {
        let source = "#!/bin/sh\nx='${$(svn info):gs/%/%%}'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert!(diagnostics.is_empty());
    }
}
