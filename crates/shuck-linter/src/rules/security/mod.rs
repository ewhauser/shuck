pub mod find_execdir_with_shell;
pub mod eval_on_array;
pub mod rm_glob_on_variable_path;
pub mod ssh_local_expansion;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::FindExecDirWithShell, Path::new("K004.sh"))]
    #[test_case(Rule::RmGlobOnVariablePath, Path::new("K001.sh"))]
    #[test_case(Rule::SshLocalExpansion, Path::new("K002.sh"))]
    #[test_case(Rule::EvalOnArray, Path::new("K003.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("security").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}
