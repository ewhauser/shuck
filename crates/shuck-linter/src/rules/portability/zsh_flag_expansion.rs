use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct ZshFlagExpansion;

impl Violation for ZshFlagExpansion {
    fn rule() -> Rule {
        Rule::ZshFlagExpansion
    }

    fn message(&self) -> String {
        "zsh parameter modifier syntax is not portable to this shell".to_owned()
    }
}

pub fn zsh_flag_expansion(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| fact.zsh_flag_modifier_spans())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshFlagExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_modifier_forms_inside_nested_targets() {
        let source =
            "#!/bin/bash\nunset ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*} 2>/dev/null\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshFlagExpansion));

        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn ignores_modifier_forms_in_zsh_scripts() {
        let source = "#!/bin/zsh\nx=${(f)foo}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshFlagExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_empty_target_modifier_forms() {
        let source = "#!/bin/sh\nx=${(%):-%x}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshFlagExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_bare_split_modifier_forms() {
        let source = "#!/bin/bash\n${=compiler} \"$@\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshFlagExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_modifier_forms_inside_positive_zsh_compatibility_guards() {
        let source = "\
#!/bin/bash
if [[ -n ${ZSH_VERSION-} ]]; then
  unset ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*} 2>/dev/null
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshFlagExpansion));

        assert_eq!(diagnostics.len(), 2);
    }
}
