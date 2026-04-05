pub mod noop;
pub mod pipe_to_kill;
pub mod script_scope_local;
pub mod unused_assignment;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::UnusedAssignment, Path::new("C001.sh"))]
    #[test_case(Rule::LocalTopLevel, Path::new("C014.sh"))]
    #[test_case(Rule::PipeToKill, Path::new("C046.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("correctness").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}
