use crate::{Checker, Rule, Violation};

pub struct CommandOutputArraySplit;

impl Violation for CommandOutputArraySplit {
    fn rule() -> Rule {
        Rule::CommandOutputArraySplit
    }

    fn message(&self) -> String {
        "avoid splitting command output directly into arrays; use mapfile or read -a".to_owned()
    }
}

pub fn command_output_array_split(checker: &mut Checker) {
    let spans = checker
        .facts()
        .array_assignment_split_word_facts()
        .flat_map(|fact| fact.unquoted_command_substitution_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CommandOutputArraySplit);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_command_substitutions_in_array_assignments() {
        let source = "\
#!/bin/bash
arr=($(printf '%s\\n' a b) `printf '%s\\n' c d` prefix$(printf '%s' z)suffix)
declare listed=($(printf '%s\\n' one two))
arr+=($(printf '%s\\n' tail))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandOutputArraySplit),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(printf '%s\\n' a b)",
                "`printf '%s\\n' c d`",
                "$(printf '%s' z)",
                "$(printf '%s\\n' one two)",
                "$(printf '%s\\n' tail)"
            ]
        );
    }

    #[test]
    fn ignores_quoted_and_non_split_array_contexts() {
        let source = "\
#!/bin/bash
arr=(\"$(printf '%s\\n' a b)\" \"`printf '%s\\n' c d`\")
value=$(printf '%s\\n' scalar)
arr=([0]=$(printf '%s\\n' keyed))
declare -A map=([k]=$(printf '%s\\n' assoc))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandOutputArraySplit),
        );

        assert!(diagnostics.is_empty());
    }
}
