use crate::{Checker, Rule, ShellDialect, Violation};

pub struct SubstringExpansion;

impl Violation for SubstringExpansion {
    fn rule() -> Rule {
        Rule::SubstringExpansion
    }

    fn message(&self) -> String {
        "substring expansion is not portable in `sh`".to_owned()
    }
}

pub fn substring_expansion(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .substring_expansion_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SubstringExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_scalar_and_positional_substring_expansions() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${1:1}\" \"${name:2}\" \"${name::2}\" \"${@:1}\" \"${*:1:2}\" \"${arr[@]:1}\" \"${arr[0]:1}\"\n\
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${1:1}", "${name:2}", "${name::2}", "${@:1}", "${*:1:2}"]
        );
    }

    #[test]
    fn anchors_on_substring_expansions_inside_unquoted_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
Expected: '${expected_commit::7}'
#define LAST_COMMIT_POSITION \"2311 ${GN_COMMIT:0:12}\"
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${expected_commit::7}", "${GN_COMMIT:0:12}"]
        );
    }

    #[test]
    fn ignores_substring_expansion_in_bash() {
        let source = "printf '%s\n' \"${1:1}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubstringExpansion).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_prefix_removal_operands_that_contain_colons() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${name#http://}\" \"${name%https://}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstringExpansion));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
