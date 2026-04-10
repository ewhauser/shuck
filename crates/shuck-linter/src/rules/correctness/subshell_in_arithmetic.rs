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

    use crate::test::test_path;
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
}
