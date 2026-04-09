use crate::{Checker, Rule, ShellDialect, Violation};

pub struct UppercaseExpansion;

impl Violation for UppercaseExpansion {
    fn rule() -> Rule {
        Rule::CaseModificationExpansion
    }

    fn message(&self) -> String {
        "case-modification expansion is not portable in `sh`".to_owned()
    }
}

pub fn uppercase_expansion(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .case_modification_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UppercaseExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_case_modification_expansions_only() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${1^^}\" \"${name^pattern}\" \"${name,,}\" \"${arr[0]^^}\" \"${arr[@],,}\" \"${name//x/y}\" \"${!name^^}\" \"${name@Q}\"\n\
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseModificationExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${1^^}",
                "${name^pattern}",
                "${name,,}",
                "${arr[0]^^}",
                "${arr[@],,}",
            ]
        );
    }

    #[test]
    fn anchors_on_case_modification_inside_unquoted_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
Expected: '${commit^^}'
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseModificationExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${commit^^}"]
        );
    }

    #[test]
    fn ignores_case_modification_in_bash() {
        let source = "printf '%s\n' \"${name^^}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseModificationExpansion)
                .with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
