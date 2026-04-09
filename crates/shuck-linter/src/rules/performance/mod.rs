pub mod grep_count_pipeline;
pub mod expr_arithmetic;
pub mod single_test_subshell;
pub mod subshell_test_group;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::ExprArithmetic, Path::new("P001.sh"))]
    #[test_case(Rule::GrepCountPipeline, Path::new("P002.sh"))]
    #[test_case(Rule::SingleTestSubshell, Path::new("P003.sh"))]
    #[test_case(Rule::SubshellTestGroup, Path::new("P004.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("performance").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}
