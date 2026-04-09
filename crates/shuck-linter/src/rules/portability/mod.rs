pub mod ampersand_redirect_in_sh;
pub mod ampersand_redirection;
pub mod bash_case_fallthrough;
pub mod brace_fd_redirection;
pub mod conditional_portability;
pub mod coproc;
pub mod csh_syntax_in_sh;
pub mod declare_command;
pub mod function_keyword;
pub mod function_keyword_in_sh;
pub mod let_command;
pub mod local_variable_in_sh;
pub mod multi_var_for_loop;
pub mod nested_zsh_substitution;
pub mod pipe_stderr_in_sh;
pub mod select_loop;
pub mod source_builtin_in_sh;
pub mod source_inside_function_in_sh;
pub mod sourced_with_args;
pub mod standalone_arithmetic;
pub mod zsh_always_block;
pub mod zsh_array_subscript_in_case;
pub mod zsh_assignment_to_zero;
pub mod zsh_brace_if;
pub mod zsh_flag_expansion;
pub mod zsh_nested_expansion;
pub mod zsh_parameter_flag;
pub mod zsh_parameter_index_flag;
pub mod zsh_prompt_bracket;
pub mod zsh_redir_pipe;

pub(crate) fn targets_non_zsh_shell(shell: crate::ShellDialect) -> bool {
    matches!(
        shell,
        crate::ShellDialect::Sh
            | crate::ShellDialect::Bash
            | crate::ShellDialect::Dash
            | crate::ShellDialect::Ksh
            | crate::ShellDialect::Mksh
    )
}

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
    #[test_case(Rule::BashCaseFallthrough, Path::new("X005.sh"))]
    #[test_case(Rule::StandaloneArithmetic, Path::new("X008.sh"))]
    #[test_case(Rule::SelectLoop, Path::new("X009.sh"))]
    #[test_case(Rule::Coproc, Path::new("X014.sh"))]
    #[test_case(Rule::AmpersandRedirection, Path::new("X012.sh"))]
    #[test_case(Rule::LetCommand, Path::new("X015.sh"))]
    #[test_case(Rule::DeclareCommand, Path::new("X016.sh"))]
    #[test_case(Rule::BraceFdRedirection, Path::new("X020.sh"))]
    #[test_case(Rule::SourceBuiltinInSh, Path::new("X031.sh"))]
    #[test_case(Rule::IfElifBashTest, Path::new("X033.sh"))]
    #[test_case(Rule::ExtendedGlobInTest, Path::new("X034.sh"))]
    #[test_case(Rule::ZshBraceIf, Path::new("X038.sh"))]
    #[test_case(Rule::ZshAlwaysBlock, Path::new("X039.sh"))]
    #[test_case(Rule::SourcedWithArgs, Path::new("X042.sh"))]
    #[test_case(Rule::ZshFlagExpansion, Path::new("X043.sh"))]
    #[test_case(Rule::NestedZshSubstitution, Path::new("X044.sh"))]
    #[test_case(Rule::MultiVarForLoop, Path::new("X047.sh"))]
    #[test_case(Rule::ZshPromptBracket, Path::new("X049.sh"))]
    #[test_case(Rule::CshSyntaxInSh, Path::new("X050.sh"))]
    #[test_case(Rule::ZshNestedExpansion, Path::new("X051.sh"))]
    #[test_case(Rule::ZshAssignmentToZero, Path::new("X053.sh"))]
    #[test_case(Rule::ZshParameterFlag, Path::new("X076.sh"))]
    #[test_case(Rule::ZshArraySubscriptInCase, Path::new("X078.sh"))]
    #[test_case(Rule::ZshParameterIndexFlag, Path::new("X079.sh"))]
    #[test_case(Rule::ArraySubscriptTest, Path::new("X040.sh"))]
    #[test_case(Rule::ArraySubscriptCondition, Path::new("X041.sh"))]
    #[test_case(Rule::ExtglobInTest, Path::new("X046.sh"))]
    #[test_case(Rule::FunctionKeywordInSh, Path::new("X052.sh"))]
    #[test_case(Rule::GreaterThanInDoubleBracket, Path::new("X058.sh"))]
    #[test_case(Rule::RegexMatchInSh, Path::new("X059.sh"))]
    #[test_case(Rule::VTestInSh, Path::new("X060.sh"))]
    #[test_case(Rule::ATestInSh, Path::new("X061.sh"))]
    #[test_case(Rule::AmpersandRedirectInSh, Path::new("X063.sh"))]
    #[test_case(Rule::PipeStderrInSh, Path::new("X066.sh"))]
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
