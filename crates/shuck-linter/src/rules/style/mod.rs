pub mod echoed_command_substitution;
pub mod escaped_underscore;
pub mod export_command_substitution;
pub mod legacy_arithmetic_expansion;
pub mod legacy_backticks;
pub mod loop_from_command_output;
pub mod printf_format_variable;
pub mod read_without_raw;
pub mod single_quote_backslash;
pub mod syntax;
pub mod unquoted_array_expansion;
pub mod unquoted_command_substitution;
pub mod unquoted_expansion;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::UnquotedExpansion, Path::new("S001.sh"))]
    #[test_case(Rule::ReadWithoutRaw, Path::new("S002.sh"))]
    #[test_case(Rule::LoopFromCommandOutput, Path::new("S003.sh"))]
    #[test_case(Rule::UnquotedCommandSubstitution, Path::new("S004.sh"))]
    #[test_case(Rule::LegacyBackticks, Path::new("S005.sh"))]
    #[test_case(Rule::LegacyArithmeticExpansion, Path::new("S006.sh"))]
    #[test_case(Rule::PrintfFormatVariable, Path::new("S007.sh"))]
    #[test_case(Rule::UnquotedArrayExpansion, Path::new("S008.sh"))]
    #[test_case(Rule::EchoedCommandSubstitution, Path::new("S009.sh"))]
    #[test_case(Rule::ExportCommandSubstitution, Path::new("S010.sh"))]
    #[test_case(Rule::EscapedUnderscore, Path::new("S023.sh"))]
    #[test_case(Rule::SingleQuoteBackslash, Path::new("S024.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("style").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}
