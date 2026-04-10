use crate::{Checker, Rule, Violation};

pub struct SubshellInArithmetic;

impl Violation for SubshellInArithmetic {
    fn rule() -> Rule {
        Rule::SubshellInArithmetic
    }

    fn message(&self) -> String {
        "avoid command substitutions inside arithmetic expansion".to_owned()
    }
}

pub fn subshell_in_arithmetic(checker: &mut Checker) {
    let spans = checker
        .facts()
        .arithmetic_command_substitution_spans()
        .to_vec();

    checker.report_all_dedup(spans, || SubshellInArithmetic);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path, test_snippet};
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test]
    fn reports_command_substitutions_inside_arithmetic_expansion() -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", Rule::SubshellInArithmetic.code(), "C077.sh");
        let (diagnostics, source) = test_path(
            Path::new("correctness").join("C077.sh").as_path(),
            &LinterSettings::for_rule(Rule::SubshellInArithmetic),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }

    #[test]
    fn reports_command_substitutions_in_arithmetic_for_clauses() {
        let source = "#!/bin/bash\nfor (( i=$(printf 1); i < 3; i++ )); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellInArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$(printf 1)");
    }

    #[test]
    fn reports_command_substitutions_in_wrapped_substring_offset_arithmetic() {
        let source =
            "#!/bin/bash\nrest=abcdef\nprintf '%s\\n' \"${rest:$((${#rest}-$(printf 1)))}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellInArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$(printf 1)");
    }

    #[test]
    fn ignores_escaped_command_substitution_tokens_in_wrapped_substring_offset_arithmetic() {
        let source = "#!/bin/bash\ns=abcdef\ni=1\nprintf '%s\\n' \"${s:$(($i+\\$(printf 1)))}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellInArithmetic),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
