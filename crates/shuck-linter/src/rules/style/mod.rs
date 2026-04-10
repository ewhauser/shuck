pub mod ampersand_semicolon;
pub mod arithmetic_score_line;
pub mod array_index_arithmetic;
pub mod avoid_let_builtin;
pub mod backslash_before_command;
pub mod dollar_in_arithmetic;
pub mod dollar_in_arithmetic_context;
pub mod echo_here_doc;
pub mod echo_inside_command_substitution;
pub mod echoed_command_substitution;
pub mod grep_output_in_test;
pub mod escaped_underscore;
pub mod escaped_underscore_literal;
pub mod export_command_substitution;
pub mod heredoc_end_space;
pub mod ifs_equals_ambiguity;
pub mod legacy_arithmetic_expansion;
pub mod legacy_backticks;
pub mod linebreak_before_and;
pub mod literal_backslash;
pub mod literal_backslash_in_single_quotes;
pub mod literal_braces;
pub mod loop_from_command_output;
pub mod ls_grep_pipeline;
pub mod needless_backslash_underscore;
pub mod printf_format_variable;
pub mod ps_grep_pipeline;
pub mod read_without_raw;
pub mod single_quote_backslash;
pub mod spaced_tabstrip_close;
pub mod suspect_closing_quote;
pub mod syntax;
pub mod trailing_directive;
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
    #[test_case(Rule::EchoInsideCommandSubstitution, Path::new("S016.sh"))]
    #[test_case(Rule::GrepOutputInTest, Path::new("S019.sh"))]
    #[test_case(Rule::PsGrepPipeline, Path::new("S012.sh"))]
    #[test_case(Rule::LsGrepPipeline, Path::new("S013.sh"))]
    #[test_case(Rule::AvoidLetBuiltin, Path::new("S022.sh"))]
    #[test_case(Rule::EchoHereDoc, Path::new("S033.sh"))]
    #[test_case(Rule::ArrayIndexArithmetic, Path::new("S034.sh"))]
    #[test_case(Rule::ArithmeticScoreLine, Path::new("S035.sh"))]
    #[test_case(Rule::DollarInArithmetic, Path::new("S045.sh"))]
    #[test_case(Rule::DollarInArithmeticContext, Path::new("S048.sh"))]
    #[test_case(Rule::EscapedUnderscore, Path::new("S023.sh"))]
    #[test_case(Rule::EscapedUnderscoreLiteral, Path::new("S027.sh"))]
    #[test_case(Rule::SingleQuoteBackslash, Path::new("S024.sh"))]
    #[test_case(Rule::LiteralBackslash, Path::new("S025.sh"))]
    #[test_case(Rule::LiteralBackslashInSingleQuotes, Path::new("S039.sh"))]
    #[test_case(Rule::NeedlessBackslashUnderscore, Path::new("S026.sh"))]
    #[test_case(Rule::BackslashBeforeCommand, Path::new("S040.sh"))]
    #[test_case(Rule::IfsEqualsAmbiguity, Path::new("S042.sh"))]
    #[test_case(Rule::SuspectClosingQuote, Path::new("S028.sh"))]
    #[test_case(Rule::LiteralBraces, Path::new("S029.sh"))]
    #[test_case(Rule::HeredocEndSpace, Path::new("S030.sh"))]
    #[test_case(Rule::TrailingDirective, Path::new("S031.sh"))]
    #[test_case(Rule::LinebreakBeforeAnd, Path::new("S072.sh"))]
    #[test_case(Rule::SpacedTabstripClose, Path::new("S073.sh"))]
    #[test_case(Rule::AmpersandSemicolon, Path::new("S074.sh"))]
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
