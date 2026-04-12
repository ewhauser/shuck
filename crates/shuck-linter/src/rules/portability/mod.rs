pub mod ampersand_redirect_in_sh;
pub mod ampersand_redirection;
pub mod ansi_c_quoting;
pub mod array_assignment;
pub mod array_keys_in_sh;
pub mod array_reference;
pub mod base_prefix_in_arithmetic;
pub mod bash_case_fallthrough;
pub mod bash_file_slurp;
pub mod brace_expansion;
pub mod brace_fd_redirection;
pub mod c_style_for_arithmetic_in_sh;
pub mod c_style_for_in_sh;
pub mod conditional_portability;
pub mod coproc;
pub mod csh_syntax_in_sh;
pub mod declare_command;
pub mod dollar_string_in_sh;
pub mod errexit_trap_in_sh;
pub mod function_keyword;
pub mod function_keyword_in_sh;
pub mod function_params_in_sh;
pub mod here_string;
pub mod hyphenated_function_name;
pub mod indirect_expansion;
pub mod legacy_arithmetic_in_sh;
pub mod let_command;
pub mod local_variable_in_sh;
pub mod multi_var_for_loop;
pub mod nested_zsh_substitution;
pub mod pipe_stderr_in_sh;
pub mod pipefail_option;
pub mod plus_equals_append;
pub mod plus_equals_in_sh;
pub mod printf_q_format_in_sh;
pub mod process_substitution;
pub mod replacement_expansion;
pub mod select_loop;
pub mod signal_name_in_trap;
pub mod source_builtin_in_sh;
mod source_common;
pub mod source_inside_function_in_sh;
pub mod sourced_with_args;
pub mod standalone_arithmetic;
pub mod star_glob_removal_in_sh;
pub mod substring_expansion;
mod trap_common;
pub mod trap_err;
pub mod uppercase_expansion;
pub mod wait_option;
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
    #[test_case(Rule::FunctionParamsInSh, Path::new("X035.sh"))]
    #[test_case(Rule::BashCaseFallthrough, Path::new("X005.sh"))]
    #[test_case(Rule::ProcessSubstitution, Path::new("X006.sh"))]
    #[test_case(Rule::AnsiCQuoting, Path::new("X007.sh"))]
    #[test_case(Rule::BraceExpansion, Path::new("X010.sh"))]
    #[test_case(Rule::HereString, Path::new("X011.sh"))]
    #[test_case(Rule::StandaloneArithmetic, Path::new("X008.sh"))]
    #[test_case(Rule::SelectLoop, Path::new("X009.sh"))]
    #[test_case(Rule::Coproc, Path::new("X014.sh"))]
    #[test_case(Rule::AmpersandRedirection, Path::new("X012.sh"))]
    #[test_case(Rule::ArrayAssignment, Path::new("X013.sh"))]
    #[test_case(Rule::LetCommand, Path::new("X015.sh"))]
    #[test_case(Rule::DeclareCommand, Path::new("X016.sh"))]
    #[test_case(Rule::TrapErr, Path::new("X017.sh"))]
    #[test_case(Rule::IndirectExpansion, Path::new("X018.sh"))]
    #[test_case(Rule::ArrayReference, Path::new("X019.sh"))]
    #[test_case(Rule::BraceFdRedirection, Path::new("X020.sh"))]
    #[test_case(Rule::PipefailOption, Path::new("X021.sh"))]
    #[test_case(Rule::WaitOption, Path::new("X022.sh"))]
    #[test_case(Rule::SubstringExpansion, Path::new("X023.sh"))]
    #[test_case(Rule::CaseModificationExpansion, Path::new("X024.sh"))]
    #[test_case(Rule::ReplacementExpansion, Path::new("X025.sh"))]
    #[test_case(Rule::BashFileSlurp, Path::new("X026.sh"))]
    #[test_case(Rule::SourceBuiltinInSh, Path::new("X031.sh"))]
    #[test_case(Rule::PrintfQFormatInSh, Path::new("X032.sh"))]
    #[test_case(Rule::IfElifBashTest, Path::new("X033.sh"))]
    #[test_case(Rule::ExtendedGlobInTest, Path::new("X034.sh"))]
    #[test_case(Rule::ExtglobCase, Path::new("X037.sh"))]
    #[test_case(Rule::ExtglobInCasePattern, Path::new("X048.sh"))]
    #[test_case(Rule::ZshBraceIf, Path::new("X038.sh"))]
    #[test_case(Rule::ZshAlwaysBlock, Path::new("X039.sh"))]
    #[test_case(Rule::SourcedWithArgs, Path::new("X042.sh"))]
    #[test_case(Rule::ZshFlagExpansion, Path::new("X043.sh"))]
    #[test_case(Rule::NestedZshSubstitution, Path::new("X044.sh"))]
    #[test_case(Rule::PlusEqualsAppend, Path::new("X045.sh"))]
    #[test_case(Rule::MultiVarForLoop, Path::new("X047.sh"))]
    #[test_case(Rule::ZshPromptBracket, Path::new("X049.sh"))]
    #[test_case(Rule::CshSyntaxInSh, Path::new("X050.sh"))]
    #[test_case(Rule::ZshNestedExpansion, Path::new("X051.sh"))]
    #[test_case(Rule::ZshAssignmentToZero, Path::new("X053.sh"))]
    #[test_case(Rule::DollarStringInSh, Path::new("X055.sh"))]
    #[test_case(Rule::ExtglobInSh, Path::new("X054.sh"))]
    #[test_case(Rule::CStyleForInSh, Path::new("X056.sh"))]
    #[test_case(Rule::LegacyArithmeticInSh, Path::new("X057.sh"))]
    #[test_case(Rule::CStyleForArithmeticInSh, Path::new("X062.sh"))]
    #[test_case(Rule::PlusEqualsInSh, Path::new("X064.sh"))]
    #[test_case(Rule::ArrayKeysInSh, Path::new("X071.sh"))]
    #[test_case(Rule::CaretNegationInBracket, Path::new("X065.sh"))]
    #[test_case(Rule::StarGlobRemovalInSh, Path::new("X081.sh"))]
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
    #[test_case(Rule::HyphenatedFunctionName, Path::new("X067.sh"))]
    #[test_case(Rule::ErrexitTrapInSh, Path::new("X068.sh"))]
    #[test_case(Rule::SignalNameInTrap, Path::new("X069.sh"))]
    #[test_case(Rule::BasePrefixInArithmetic, Path::new("X070.sh"))]
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
