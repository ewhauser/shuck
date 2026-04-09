pub mod conditional_portability;
pub mod declare_command;
pub mod function_keyword;
pub mod function_keyword_in_sh;
pub mod let_command;
pub mod local_variable_in_sh;
pub mod source_builtin_in_sh;
pub mod source_inside_function_in_sh;
pub mod zsh_always_block;
pub mod zsh_brace_if;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::DoubleBracketInSh, Path::new("X001.sh"))]
    #[test_case(Rule::TestEqualityOperator, Path::new("X002.sh"))]
    #[test_case(Rule::LocalVariableInSh, Path::new("X003.sh"))]
    #[test_case(Rule::FunctionKeyword, Path::new("X004.sh"))]
    #[test_case(Rule::LetCommand, Path::new("X015.sh"))]
    #[test_case(Rule::DeclareCommand, Path::new("X016.sh"))]
    #[test_case(Rule::SourceBuiltinInSh, Path::new("X031.sh"))]
    #[test_case(Rule::IfElifBashTest, Path::new("X033.sh"))]
    #[test_case(Rule::ExtendedGlobInTest, Path::new("X034.sh"))]
    #[test_case(Rule::ZshBraceIf, Path::new("X038.sh"))]
    #[test_case(Rule::ZshAlwaysBlock, Path::new("X039.sh"))]
    #[test_case(Rule::ArraySubscriptTest, Path::new("X040.sh"))]
    #[test_case(Rule::ArraySubscriptCondition, Path::new("X041.sh"))]
    #[test_case(Rule::ExtglobInTest, Path::new("X046.sh"))]
    #[test_case(Rule::FunctionKeywordInSh, Path::new("X052.sh"))]
    #[test_case(Rule::GreaterThanInDoubleBracket, Path::new("X058.sh"))]
    #[test_case(Rule::RegexMatchInSh, Path::new("X059.sh"))]
    #[test_case(Rule::VTestInSh, Path::new("X060.sh"))]
    #[test_case(Rule::ATestInSh, Path::new("X061.sh"))]
    #[test_case(Rule::OptionTestInSh, Path::new("X073.sh"))]
    #[test_case(Rule::StickyBitTestInSh, Path::new("X074.sh"))]
    #[test_case(Rule::OwnershipTestInSh, Path::new("X075.sh"))]
    #[test_case(Rule::SourceInsideFunctionInSh, Path::new("X080.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("portability").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}

pub mod zsh_redir_pipe;
