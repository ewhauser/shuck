use crate::{Checker, Rule, Violation};

pub struct PlusPrefixInAssignment;

impl Violation for PlusPrefixInAssignment {
    fn rule() -> Rule {
        Rule::PlusPrefixInAssignment
    }

    fn message(&self) -> String {
        "this looks like an assignment, but the shell parses it as a command name".to_owned()
    }
}

pub fn plus_prefix_in_assignment(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.assignment_like_command_name_spans(),
        || PlusPrefixInAssignment,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_assignment_like_words_with_a_leading_plus() {
        let source = "\
#!/bin/bash
+YYYY=\"$( date +%Y )\"
export +MONTH=12
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusPrefixInAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["+YYYY=\"$( date +%Y )\"", "+MONTH=12"]
        );
    }

    #[test]
    fn ignores_regular_commands_and_non_identifier_targets() {
        let source = "\
#!/bin/sh
echo +YEAR=2024
+1=bad
name+=still_ok
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusPrefixInAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn anchors_on_invalid_assignment_like_command_names_without_a_leading_plus() {
        let source = r#"#!/bin/sh
network.wan.proto='dhcp'
@VAR@=$(. /etc/profile >/dev/null 2>&1; echo "${@VAR@}")
"${NINJA:=ninja}"
"#;
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusPrefixInAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "network.wan.proto='dhcp'",
                "@VAR@=$(. /etc/profile >/dev/null 2>&1; echo \"${@VAR@}\")"
            ]
        );
    }

    #[test]
    fn ignores_assignment_like_text_after_literal_arrow_prefix() {
        let source = r#"#!/bin/bash
rvm_info="
  bash: \"$(command -v bash) => $(version_for bash)\"
  zsh:  \"$(command -v zsh) => $(version_for zsh)\"
"
"#;
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusPrefixInAssignment),
        );

        assert!(diagnostics.is_empty());
    }
}
