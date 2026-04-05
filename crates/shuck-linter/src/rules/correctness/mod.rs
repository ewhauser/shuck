pub mod constant_case_subject;
pub mod constant_comparison_test;
pub mod empty_test;
pub mod find_output_loop;
pub mod find_output_to_xargs;
pub mod literal_unary_string_test;
pub mod loop_control_outside_loop;
pub mod noop;
pub mod overwritten_function;
pub mod pipe_to_kill;
pub mod quoted_bash_regex;
pub mod script_scope_local;
pub mod single_quoted_literal;
pub mod sudo_redirection_order;
pub mod syntax;
pub mod trap_string_expansion;
pub mod truthy_literal_test;
pub mod unused_assignment;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::test_path;
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test_case(Rule::UnusedAssignment, Path::new("C001.sh"))]
    #[test_case(Rule::SingleQuotedLiteral, Path::new("C005.sh"))]
    #[test_case(Rule::FindOutputToXargs, Path::new("C007.sh"))]
    #[test_case(Rule::TrapStringExpansion, Path::new("C008.sh"))]
    #[test_case(Rule::QuotedBashRegex, Path::new("C009.sh"))]
    #[test_case(Rule::FindOutputLoop, Path::new("C013.sh"))]
    #[test_case(Rule::LocalTopLevel, Path::new("C014.sh"))]
    #[test_case(Rule::SudoRedirectionOrder, Path::new("C015.sh"))]
    #[test_case(Rule::ConstantComparisonTest, Path::new("C017.sh"))]
    #[test_case(Rule::LoopControlOutsideLoop, Path::new("C018.sh"))]
    #[test_case(Rule::LiteralUnaryStringTest, Path::new("C019.sh"))]
    #[test_case(Rule::TruthyLiteralTest, Path::new("C020.sh"))]
    #[test_case(Rule::ConstantCaseSubject, Path::new("C021.sh"))]
    #[test_case(Rule::EmptyTest, Path::new("C022.sh"))]
    #[test_case(Rule::PipeToKill, Path::new("C046.sh"))]
    #[test_case(Rule::OverwrittenFunction, Path::new("C063.sh"))]
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
