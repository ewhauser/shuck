pub mod arithmetic_redirection_target;
pub mod broken_test_end;
pub mod broken_test_parse;
pub mod c_style_comment;
pub mod case_pattern_var;
pub mod chained_test_branches;
pub mod constant_case_subject;
pub mod constant_comparison_test;
pub mod dynamic_source_path;
pub mod empty_test;
pub mod else_if;
pub mod find_output_loop;
pub mod find_output_to_xargs;
pub mod invalid_exit_status;
pub mod leading_glob_argument;
pub mod linebreak_in_test;
pub mod line_oriented_input;
pub mod literal_unary_string_test;
pub mod loop_control_outside_loop;
pub mod missing_fi;
pub mod open_double_quote;
pub mod overwritten_function;
pub mod pattern_with_variable;
pub mod pipe_to_kill;
pub mod positional_ten_braces;
pub mod quoted_bash_regex;
pub mod script_scope_local;
pub mod single_quoted_literal;
pub mod subst_with_redirect;
pub mod subst_with_redirect_err;
pub mod sudo_redirection_order;
pub mod syntax;
pub mod trap_string_expansion;
pub mod truthy_literal_test;
pub mod unchecked_directory_change;
pub mod undefined_variable;
pub mod unreachable_after_exit;
pub mod untracked_source_file;
pub mod unused_assignment;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::UnusedAssignment, Path::new("C001.sh"))]
    #[test_case(Rule::DynamicSourcePath, Path::new("C002.sh"))]
    #[test_case(Rule::UntrackedSourceFile, Path::new("C003.sh"))]
    #[test_case(Rule::UncheckedDirectoryChange, Path::new("C004.sh"))]
    #[test_case(Rule::SingleQuotedLiteral, Path::new("C005.sh"))]
    #[test_case(Rule::UndefinedVariable, Path::new("C006.sh"))]
    #[test_case(Rule::FindOutputToXargs, Path::new("C007.sh"))]
    #[test_case(Rule::TrapStringExpansion, Path::new("C008.sh"))]
    #[test_case(Rule::QuotedBashRegex, Path::new("C009.sh"))]
    #[test_case(Rule::ChainedTestBranches, Path::new("C010.sh"))]
    #[test_case(Rule::LineOrientedInput, Path::new("C011.sh"))]
    #[test_case(Rule::LeadingGlobArgument, Path::new("C012.sh"))]
    #[test_case(Rule::FindOutputLoop, Path::new("C013.sh"))]
    #[test_case(Rule::LocalTopLevel, Path::new("C014.sh"))]
    #[test_case(Rule::SudoRedirectionOrder, Path::new("C015.sh"))]
    #[test_case(Rule::ConstantComparisonTest, Path::new("C017.sh"))]
    #[test_case(Rule::LoopControlOutsideLoop, Path::new("C018.sh"))]
    #[test_case(Rule::LiteralUnaryStringTest, Path::new("C019.sh"))]
    #[test_case(Rule::TruthyLiteralTest, Path::new("C020.sh"))]
    #[test_case(Rule::ConstantCaseSubject, Path::new("C021.sh"))]
    #[test_case(Rule::EmptyTest, Path::new("C022.sh"))]
    #[test_case(Rule::PositionalTenBraces, Path::new("C025.sh"))]
    #[test_case(Rule::MissingFi, Path::new("C035.sh"))]
    #[test_case(Rule::BrokenTestEnd, Path::new("C036.sh"))]
    #[test_case(Rule::BrokenTestParse, Path::new("C037.sh"))]
    #[test_case(Rule::ElseIf, Path::new("C038.sh"))]
    #[test_case(Rule::OpenDoubleQuote, Path::new("C039.sh"))]
    #[test_case(Rule::LinebreakInTest, Path::new("C040.sh"))]
    #[test_case(Rule::CStyleComment, Path::new("C041.sh"))]
    #[test_case(Rule::PipeToKill, Path::new("C046.sh"))]
    #[test_case(Rule::InvalidExitStatus, Path::new("C047.sh"))]
    #[test_case(Rule::CasePatternVar, Path::new("C048.sh"))]
    #[test_case(Rule::ArithmeticRedirectionTarget, Path::new("C050.sh"))]
    #[test_case(Rule::PatternWithVariable, Path::new("C055.sh"))]
    #[test_case(Rule::SubstWithRedirect, Path::new("C057.sh"))]
    #[test_case(Rule::SubstWithRedirectErr, Path::new("C058.sh"))]
    #[test_case(Rule::OverwrittenFunction, Path::new("C063.sh"))]
    #[test_case(Rule::UnreachableAfterExit, Path::new("C124.sh"))]
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
