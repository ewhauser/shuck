use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ReplacementExpansion;

impl Violation for ReplacementExpansion {
    fn rule() -> Rule {
        Rule::ReplacementExpansion
    }

    fn message(&self) -> String {
        "replacement expansion is not portable in `sh`".to_owned()
    }
}

pub fn replacement_expansion(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .replacement_expansion_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ReplacementExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_replacement_expansions_only() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${name//x/y}\" \"${name/x/y}\" \"${name/#x/y}\" \"${name/%x/y}\" \"${arr[0]//x/y}\" \"${arr[@]/x/y}\" \"${arr[*]//x}\" \"${name/${needle}/y}\" \"${name^^}\" \"${name:1}\" \"${!name//x/y}\" \"${name@Q}\"\n\
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${name//x/y}",
                "${name/x/y}",
                "${name/#x/y}",
                "${name/%x/y}",
                "${arr[0]//x/y}",
                "${arr[@]/x/y}",
                "${arr[*]//x}",
                "${name/${needle}/y}",
            ]
        );
    }

    #[test]
    fn anchors_on_replacement_expansions_inside_unquoted_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
Expected: '${commit//old/new}'
Escaped: '\\${commit//old/new}'
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${commit//old/new}"]
        );
    }

    #[test]
    fn ignores_replacement_expansion_in_bash() {
        let source = "printf '%s\n' \"${name//x/y}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReplacementExpansion).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
