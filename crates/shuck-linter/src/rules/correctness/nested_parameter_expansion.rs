use crate::{Checker, Rule, Violation};

pub struct NestedParameterExpansion;

impl Violation for NestedParameterExpansion {
    fn rule() -> Rule {
        Rule::NestedParameterExpansion
    }

    fn message(&self) -> String {
        "nested parameter expansion appears inside `${...}`".to_owned()
    }
}

pub fn nested_parameter_expansion(checker: &mut Checker) {
    let spans = checker
        .facts()
        .nested_parameter_expansion_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || NestedParameterExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nested_parameter_expansion_targets() {
        let source = "\
#!/bin/sh
x=\"${${FALLBACK:-default}:-value}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedParameterExpansion),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn ignores_nested_expansions_in_default_operands_and_literals() {
        let source = "\
#!/bin/sh
x=\"${fallback:-${value:-default}}\"
echo '${${ignored}:-value}'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedParameterExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_nested_expansions() {
        let source = "\
#!/bin/zsh
[[ -n \"$ZSH\" ]] || export ZSH=\"${${(%):-%x}:a:h}\"
unset ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*} 2>/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedParameterExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_style_nested_expansions_in_non_zsh_shells() {
        let source = "#!/bin/sh\nx=${${(M)path:#/*}:-$PWD/$path}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedParameterExpansion),
        );

        assert!(diagnostics.is_empty());
    }
}
