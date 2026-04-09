use rustc_hash::FxHashMap;

use crate::Rule;

/// Maps shellcheck SC codes to shuck rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckCodeMap {
    map: FxHashMap<u32, Rule>,
    comparison: Vec<(u32, Rule)>,
}

impl ShellCheckCodeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mappings(&self) -> impl Iterator<Item = (u32, Rule)> + '_ {
        self.map.iter().map(|(sc_code, rule)| (*sc_code, *rule))
    }

    /// Mappings that are stable enough to compare in the large corpus harness.
    pub fn comparison_mappings(&self) -> impl Iterator<Item = (u32, Rule)> + '_ {
        self.comparison.iter().copied()
    }

    /// Look up a shellcheck code like `SC2086`.
    pub fn resolve(&self, sc_code: &str) -> Option<Rule> {
        let number = sc_code
            .strip_prefix("SC")
            .or_else(|| sc_code.strip_prefix("sc"))?
            .parse()
            .ok()?;
        if number == 2262 {
            return Some(Rule::TemplateBraceInCommand);
        }
        if number == 2261 {
            return Some(Rule::NonAbsoluteShebang);
        }
        if number == 2260 {
            return Some(Rule::RedirectToCommandName);
        }
        if number == 2253 {
            return Some(Rule::StatusCaptureAfterBranchTest);
        }
        if number == 2281 {
            return Some(Rule::BackslashBeforeClosingBacktick);
        }
        if number == 2282 {
            return Some(Rule::PositionalParamAsOperator);
        }
        if number == 2283 {
            return Some(Rule::DoubleParenGrouping);
        }
        if number == 2284 {
            return Some(Rule::UnicodeQuoteInString);
        }
        if number == 2385 {
            return Some(Rule::UnicodeSingleQuoteInSingleQuotes);
        }
        self.map.get(&number).copied()
    }
}

impl Default for ShellCheckCodeMap {
    fn default() -> Self {
        let comparison = vec![
            (2003, Rule::ExprArithmetic),
            (2005, Rule::EchoedCommandSubstitution),
            (2006, Rule::LegacyBackticks),
            (2007, Rule::LegacyArithmeticExpansion),
            (2009, Rule::DoubleParenGrouping),
            (1037, Rule::PositionalTenBraces),
            (1047, Rule::MissingFi),
            (1072, Rule::BrokenTestParse),
            (1073, Rule::BrokenTestEnd),
            (1075, Rule::ElseIf),
            (1078, Rule::OpenDoubleQuote),
            (1080, Rule::LinebreakInTest),
            (1090, Rule::DynamicSourcePath),
            (1091, Rule::UntrackedSourceFile),
            (1101, Rule::BackslashBeforeClosingBacktick),
            (1102, Rule::PositionalParamAsOperator),
            (1110, Rule::UnicodeQuoteInString),
            (1127, Rule::CStyleComment),
            (1132, Rule::CPrototypeFragment),
            (2164, Rule::UncheckedDirectoryChange),
            (2016, Rule::SingleQuotedLiteral),
            (2013, Rule::LineOrientedInput),
            (2015, Rule::ChainedTestBranches),
            (1019, Rule::EmptyTest),
            (2024, Rule::SudoRedirectionOrder),
            (2034, Rule::UnusedAssignment),
            (2035, Rule::LeadingGlobArgument),
            (2044, Rule::FindOutputLoop),
            (2045, Rule::LoopFromCommandOutput),
            (2046, Rule::UnquotedCommandSubstitution),
            (2059, Rule::PrintfFormatVariable),
            (3043, Rule::LocalVariableInSh),
            (2038, Rule::FindOutputToXargs),
            (2064, Rule::TrapStringExpansion),
            (2068, Rule::UnquotedArrayExpansion),
            (2076, Rule::QuotedBashRegex),
            (2086, Rule::UnquotedExpansion),
            (2104, Rule::LoopControlOutsideLoop),
            (2126, Rule::GrepCountPipeline),
            (2112, Rule::FunctionKeyword),
            (2216, Rule::PipeToKill),
            (2233, Rule::SingleTestSubshell),
            (2235, Rule::SubshellTestGroup),
            // ShellCheck 0.11.0 reports `let` portability warnings as SC3039.
            // Keep SC3042 as a suppression alias, but prefer the current code for comparisons.
            (3042, Rule::LetCommand),
            (3039, Rule::LetCommand),
            (3046, Rule::SourceBuiltinInSh),
            // ShellCheck 0.11.0 reports `source` inside functions as SC3051.
            (3051, Rule::SourceInsideFunctionInSh),
            (2155, Rule::ExportCommandSubstitution),
            (2157, Rule::ConstantComparisonTest),
            (2158, Rule::LiteralUnaryStringTest),
            (2162, Rule::ReadWithoutRaw),
            (2078, Rule::TruthyLiteralTest),
            (2168, Rule::LocalTopLevel),
            (2194, Rule::ConstantCaseSubject),
            (2210, Rule::BadRedirectionFdOrder),
            (2154, Rule::UndefinedVariable),
            (2239, Rule::NonAbsoluteShebang),
            (2288, Rule::TemplateBraceInCommand),
            (2241, Rule::InvalidExitStatus),
            (2242, Rule::CasePatternVar),
            (2248, Rule::BareSlashMarker),
            (2257, Rule::ArithmeticRedirectionTarget),
            (2264, Rule::NestedParameterExpansion),
            (2250, Rule::PatternWithVariable),
            (2255, Rule::SubstWithRedirect),
            (2256, Rule::SubstWithRedirectErr),
            (2238, Rule::RedirectToCommandName),
            (2259, Rule::SubshellTestGroup),
            (2266, Rule::OverwrittenFunction),
            (2270, Rule::IfMissingThen),
            (2271, Rule::ElseWithoutThen),
            (2272, Rule::MissingSemicolonBeforeBrace),
            (2273, Rule::EmptyFunctionBody),
            (2274, Rule::BareClosingBrace),
            (2319, Rule::StatusCaptureAfterBranchTest),
            (2365, Rule::UnreachableAfterExit),
            (3010, Rule::DoubleBracketInSh),
            (3012, Rule::GreaterThanInDoubleBracket),
            (3014, Rule::TestEqualityOperator),
            (3015, Rule::RegexMatchInSh),
            (3016, Rule::VTestInSh),
            (3017, Rule::ATestInSh),
            (3062, Rule::OptionTestInSh),
            (3065, Rule::StickyBitTestInSh),
            (3067, Rule::OwnershipTestInSh),
        ];

        Self {
            map: FxHashMap::from_iter([
                (1001, Rule::EscapedUnderscore),
                (1002, Rule::EscapedUnderscoreLiteral),
                (1003, Rule::SingleQuoteBackslash),
                (1004, Rule::LiteralBackslash),
                (1012, Rule::NeedlessBackslashUnderscore),
                (2267, Rule::LiteralBackslashInSingleQuotes),
                (2005, Rule::EchoedCommandSubstitution),
                (2006, Rule::LegacyBackticks),
                (2007, Rule::LegacyArithmeticExpansion),
                (2003, Rule::ExprArithmetic),
                (2126, Rule::GrepCountPipeline),
                (2009, Rule::DoubleParenGrouping),
                (2233, Rule::SingleTestSubshell),
                (2235, Rule::SubshellTestGroup),
                (2259, Rule::SubshellTestGroup),
                (1037, Rule::PositionalTenBraces),
                (1047, Rule::MissingFi),
                (1070, Rule::ZshRedirPipe),
                (1072, Rule::BrokenTestParse),
                (1073, Rule::BrokenTestEnd),
                (1075, Rule::ElseIf),
                (1078, Rule::OpenDoubleQuote),
                (1080, Rule::LinebreakInTest),
                (1090, Rule::DynamicSourcePath),
                (1091, Rule::UntrackedSourceFile),
                (1101, Rule::BackslashBeforeClosingBacktick),
                (1102, Rule::PositionalParamAsOperator),
                (1110, Rule::UnicodeQuoteInString),
                (2385, Rule::UnicodeSingleQuoteInSingleQuotes),
                (1127, Rule::CStyleComment),
                (1129, Rule::ZshBraceIf),
                (1130, Rule::ZshAlwaysBlock),
                (1132, Rule::CPrototypeFragment),
                (2164, Rule::UncheckedDirectoryChange),
                (2016, Rule::SingleQuotedLiteral),
                (2013, Rule::LineOrientedInput),
                (2015, Rule::ChainedTestBranches),
                (1019, Rule::EmptyTest),
                (2024, Rule::SudoRedirectionOrder),
                (2034, Rule::UnusedAssignment),
                (2035, Rule::LeadingGlobArgument),
                (2044, Rule::FindOutputLoop),
                (2045, Rule::LoopFromCommandOutput),
                (2046, Rule::UnquotedCommandSubstitution),
                (2059, Rule::PrintfFormatVariable),
                (3043, Rule::LocalVariableInSh),
                (2038, Rule::FindOutputToXargs),
                (2064, Rule::TrapStringExpansion),
                (2068, Rule::UnquotedArrayExpansion),
                (2076, Rule::QuotedBashRegex),
                (2086, Rule::UnquotedExpansion),
                (2104, Rule::LoopControlOutsideLoop),
                (2112, Rule::FunctionKeyword),
                (2216, Rule::PipeToKill),
                (3039, Rule::LetCommand),
                (3042, Rule::LetCommand),
                (3044, Rule::DeclareCommand),
                (3046, Rule::SourceBuiltinInSh),
                (2321, Rule::FunctionKeywordInSh),
                (3051, Rule::SourceInsideFunctionInSh),
                (3084, Rule::SourceInsideFunctionInSh),
                (2155, Rule::ExportCommandSubstitution),
                (2157, Rule::ConstantComparisonTest),
                (2158, Rule::LiteralUnaryStringTest),
                (2162, Rule::ReadWithoutRaw),
                (2078, Rule::TruthyLiteralTest),
                (2168, Rule::LocalTopLevel),
                (2194, Rule::ConstantCaseSubject),
                (2210, Rule::BadRedirectionFdOrder),
                (2154, Rule::UndefinedVariable),
                (2239, Rule::NonAbsoluteShebang),
                (2288, Rule::TemplateBraceInCommand),
                (2241, Rule::InvalidExitStatus),
                (2242, Rule::CasePatternVar),
                (2248, Rule::BareSlashMarker),
                (2257, Rule::ArithmeticRedirectionTarget),
                (2264, Rule::NestedParameterExpansion),
                (2250, Rule::PatternWithVariable),
                (2255, Rule::SubstWithRedirect),
                (2256, Rule::SubstWithRedirectErr),
                (2238, Rule::RedirectToCommandName),
                (2266, Rule::OverwrittenFunction),
                (2270, Rule::IfMissingThen),
                (2271, Rule::ElseWithoutThen),
                (2272, Rule::MissingSemicolonBeforeBrace),
                (2273, Rule::EmptyFunctionBody),
                (2274, Rule::BareClosingBrace),
                (2319, Rule::StatusCaptureAfterBranchTest),
                (2365, Rule::UnreachableAfterExit),
                (3010, Rule::DoubleBracketInSh),
                (3012, Rule::GreaterThanInDoubleBracket),
                (3014, Rule::TestEqualityOperator),
                (3015, Rule::RegexMatchInSh),
                (3016, Rule::VTestInSh),
                (3017, Rule::ATestInSh),
                (3062, Rule::OptionTestInSh),
                (3065, Rule::StickyBitTestInSh),
                (3067, Rule::OwnershipTestInSh),
            ]),
            comparison,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_codes_and_ignores_unknown_ones() {
        let map = ShellCheckCodeMap::default();

        assert_eq!(map.resolve("SC2034"), Some(Rule::UnusedAssignment));
        assert_eq!(map.resolve("SC2005"), Some(Rule::EchoedCommandSubstitution));
        assert_eq!(map.resolve("SC2006"), Some(Rule::LegacyBackticks));
        assert_eq!(map.resolve("SC2007"), Some(Rule::LegacyArithmeticExpansion));
        assert_eq!(map.resolve("SC2003"), Some(Rule::ExprArithmetic));
        assert_eq!(map.resolve("SC2126"), Some(Rule::GrepCountPipeline));
        assert_eq!(map.resolve("SC2009"), Some(Rule::DoubleParenGrouping));
        assert_eq!(map.resolve("SC2233"), Some(Rule::SingleTestSubshell));
        assert_eq!(map.resolve("SC2235"), Some(Rule::SubshellTestGroup));
        assert_eq!(map.resolve("SC2259"), Some(Rule::SubshellTestGroup));
        assert_eq!(map.resolve("SC1037"), Some(Rule::PositionalTenBraces));
        assert_eq!(map.resolve("SC1001"), Some(Rule::EscapedUnderscore));
        assert_eq!(map.resolve("SC1002"), Some(Rule::EscapedUnderscoreLiteral));
        assert_eq!(map.resolve("SC1003"), Some(Rule::SingleQuoteBackslash));
        assert_eq!(map.resolve("SC1004"), Some(Rule::LiteralBackslash));
        assert_eq!(
            map.resolve("SC1012"),
            Some(Rule::NeedlessBackslashUnderscore)
        );
        assert_eq!(
            map.resolve("SC2267"),
            Some(Rule::LiteralBackslashInSingleQuotes)
        );
        assert_eq!(map.resolve("SC1047"), Some(Rule::MissingFi));
        assert_eq!(map.resolve("SC1070"), Some(Rule::ZshRedirPipe));
        assert_eq!(map.resolve("SC1072"), Some(Rule::BrokenTestParse));
        assert_eq!(map.resolve("SC1073"), Some(Rule::BrokenTestEnd));
        assert_eq!(map.resolve("SC1075"), Some(Rule::ElseIf));
        assert_eq!(map.resolve("SC1078"), Some(Rule::OpenDoubleQuote));
        assert_eq!(map.resolve("SC1080"), Some(Rule::LinebreakInTest));
        assert_eq!(map.resolve("SC1090"), Some(Rule::DynamicSourcePath));
        assert_eq!(map.resolve("SC1091"), Some(Rule::UntrackedSourceFile));
        assert_eq!(
            map.resolve("SC1101"),
            Some(Rule::BackslashBeforeClosingBacktick)
        );
        assert_eq!(map.resolve("SC1102"), Some(Rule::PositionalParamAsOperator));
        assert_eq!(map.resolve("SC1110"), Some(Rule::UnicodeQuoteInString));
        assert_eq!(
            map.resolve("SC2385"),
            Some(Rule::UnicodeSingleQuoteInSingleQuotes)
        );
        assert_eq!(map.resolve("SC1129"), Some(Rule::ZshBraceIf));
        assert_eq!(map.resolve("SC1127"), Some(Rule::CStyleComment));
        assert_eq!(map.resolve("SC1130"), Some(Rule::ZshAlwaysBlock));
        assert_eq!(map.resolve("SC1132"), Some(Rule::CPrototypeFragment));
        assert_eq!(map.resolve("SC2164"), Some(Rule::UncheckedDirectoryChange));
        assert_eq!(map.resolve("SC2016"), Some(Rule::SingleQuotedLiteral));
        assert_eq!(map.resolve("SC2013"), Some(Rule::LineOrientedInput));
        assert_eq!(map.resolve("SC2015"), Some(Rule::ChainedTestBranches));
        assert_eq!(map.resolve("SC1019"), Some(Rule::EmptyTest));
        assert_eq!(map.resolve("SC2024"), Some(Rule::SudoRedirectionOrder));
        assert_eq!(map.resolve("SC2035"), Some(Rule::LeadingGlobArgument));
        assert_eq!(map.resolve("SC2044"), Some(Rule::FindOutputLoop));
        assert_eq!(map.resolve("SC2045"), Some(Rule::LoopFromCommandOutput));
        assert_eq!(
            map.resolve("SC2046"),
            Some(Rule::UnquotedCommandSubstitution)
        );
        assert_eq!(map.resolve("SC2059"), Some(Rule::PrintfFormatVariable));
        assert_eq!(map.resolve("SC2038"), Some(Rule::FindOutputToXargs));
        assert_eq!(map.resolve("SC2064"), Some(Rule::TrapStringExpansion));
        assert_eq!(map.resolve("SC2068"), Some(Rule::UnquotedArrayExpansion));
        assert_eq!(map.resolve("SC2076"), Some(Rule::QuotedBashRegex));
        assert_eq!(map.resolve("SC2086"), Some(Rule::UnquotedExpansion));
        assert_eq!(map.resolve("SC2104"), Some(Rule::LoopControlOutsideLoop));
        assert_eq!(map.resolve("SC2112"), Some(Rule::FunctionKeyword));
        assert_eq!(map.resolve("SC2216"), Some(Rule::PipeToKill));
        assert_eq!(map.resolve("SC3039"), Some(Rule::LetCommand));
        assert_eq!(map.resolve("SC3042"), Some(Rule::LetCommand));
        assert_eq!(map.resolve("SC3044"), Some(Rule::DeclareCommand));
        assert_eq!(map.resolve("SC3046"), Some(Rule::SourceBuiltinInSh));
        assert_eq!(map.resolve("SC2321"), Some(Rule::FunctionKeywordInSh));
        assert_eq!(map.resolve("SC3051"), Some(Rule::SourceInsideFunctionInSh));
        assert_eq!(map.resolve("SC3084"), Some(Rule::SourceInsideFunctionInSh));
        assert_eq!(map.resolve("SC2155"), Some(Rule::ExportCommandSubstitution));
        assert_eq!(map.resolve("SC2157"), Some(Rule::ConstantComparisonTest));
        assert_eq!(map.resolve("SC2158"), Some(Rule::LiteralUnaryStringTest));
        assert_eq!(map.resolve("SC2078"), Some(Rule::TruthyLiteralTest));
        assert_eq!(map.resolve("SC2162"), Some(Rule::ReadWithoutRaw));
        assert_eq!(map.resolve("SC2168"), Some(Rule::LocalTopLevel));
        assert_eq!(map.resolve("SC2194"), Some(Rule::ConstantCaseSubject));
        assert_eq!(map.resolve("SC2210"), Some(Rule::BadRedirectionFdOrder));
        assert_eq!(map.resolve("sc2154"), Some(Rule::UndefinedVariable));
        assert_eq!(map.resolve("SC2241"), Some(Rule::InvalidExitStatus));
        assert_eq!(map.resolve("SC2242"), Some(Rule::CasePatternVar));
        assert_eq!(map.resolve("SC2248"), Some(Rule::BareSlashMarker));
        assert_eq!(
            map.resolve("SC2253"),
            Some(Rule::StatusCaptureAfterBranchTest)
        );
        assert_eq!(
            map.resolve("SC2319"),
            Some(Rule::StatusCaptureAfterBranchTest)
        );
        assert_eq!(
            map.resolve("SC2257"),
            Some(Rule::ArithmeticRedirectionTarget)
        );
        assert_eq!(map.resolve("SC2250"), Some(Rule::PatternWithVariable));
        assert_eq!(map.resolve("SC2255"), Some(Rule::SubstWithRedirect));
        assert_eq!(map.resolve("SC2256"), Some(Rule::SubstWithRedirectErr));
        assert_eq!(map.resolve("SC2238"), Some(Rule::RedirectToCommandName));
        assert_eq!(map.resolve("SC2268"), None);
        assert_eq!(map.resolve("SC2239"), Some(Rule::NonAbsoluteShebang));
        assert_eq!(map.resolve("SC2260"), Some(Rule::RedirectToCommandName));
        assert_eq!(map.resolve("SC2261"), Some(Rule::NonAbsoluteShebang));
        assert_eq!(map.resolve("SC2262"), Some(Rule::TemplateBraceInCommand));
        assert_eq!(map.resolve("SC2264"), Some(Rule::NestedParameterExpansion));
        assert_eq!(map.resolve("SC2270"), Some(Rule::IfMissingThen));
        assert_eq!(map.resolve("SC2271"), Some(Rule::ElseWithoutThen));
        assert_eq!(
            map.resolve("SC2272"),
            Some(Rule::MissingSemicolonBeforeBrace)
        );
        assert_eq!(map.resolve("SC2273"), Some(Rule::EmptyFunctionBody));
        assert_eq!(map.resolve("SC2274"), Some(Rule::BareClosingBrace));
        assert_eq!(
            map.resolve("SC2281"),
            Some(Rule::BackslashBeforeClosingBacktick)
        );
        assert_eq!(map.resolve("SC2282"), Some(Rule::PositionalParamAsOperator));
        assert_eq!(map.resolve("SC2283"), Some(Rule::DoubleParenGrouping));
        assert_eq!(map.resolve("SC2284"), Some(Rule::UnicodeQuoteInString));
        assert_eq!(map.resolve("SC2288"), Some(Rule::TemplateBraceInCommand));
        assert_eq!(map.resolve("SC2266"), Some(Rule::OverwrittenFunction));
        assert_eq!(map.resolve("SC2365"), Some(Rule::UnreachableAfterExit));
        assert_eq!(map.resolve("SC7777"), None);
    }

    #[test]
    fn exposes_all_mappings() {
        let mut mappings = ShellCheckCodeMap::default().mappings().collect::<Vec<_>>();
        mappings.sort_unstable_by_key(|(sc_code, _)| *sc_code);

        assert_eq!(
            mappings,
            vec![
                (1001, Rule::EscapedUnderscore),
                (1002, Rule::EscapedUnderscoreLiteral),
                (1003, Rule::SingleQuoteBackslash),
                (1004, Rule::LiteralBackslash),
                (1012, Rule::NeedlessBackslashUnderscore),
                (1019, Rule::EmptyTest),
                (1037, Rule::PositionalTenBraces),
                (1047, Rule::MissingFi),
                (1070, Rule::ZshRedirPipe),
                (1072, Rule::BrokenTestParse),
                (1073, Rule::BrokenTestEnd),
                (1075, Rule::ElseIf),
                (1078, Rule::OpenDoubleQuote),
                (1080, Rule::LinebreakInTest),
                (1090, Rule::DynamicSourcePath),
                (1091, Rule::UntrackedSourceFile),
                (1101, Rule::BackslashBeforeClosingBacktick),
                (1102, Rule::PositionalParamAsOperator),
                (1110, Rule::UnicodeQuoteInString),
                (1127, Rule::CStyleComment),
                (1129, Rule::ZshBraceIf),
                (1130, Rule::ZshAlwaysBlock),
                (1132, Rule::CPrototypeFragment),
                (2003, Rule::ExprArithmetic),
                (2005, Rule::EchoedCommandSubstitution),
                (2006, Rule::LegacyBackticks),
                (2007, Rule::LegacyArithmeticExpansion),
                (2009, Rule::DoubleParenGrouping),
                (2013, Rule::LineOrientedInput),
                (2015, Rule::ChainedTestBranches),
                (2016, Rule::SingleQuotedLiteral),
                (2024, Rule::SudoRedirectionOrder),
                (2034, Rule::UnusedAssignment),
                (2035, Rule::LeadingGlobArgument),
                (2038, Rule::FindOutputToXargs),
                (2044, Rule::FindOutputLoop),
                (2045, Rule::LoopFromCommandOutput),
                (2046, Rule::UnquotedCommandSubstitution),
                (2059, Rule::PrintfFormatVariable),
                (2064, Rule::TrapStringExpansion),
                (2068, Rule::UnquotedArrayExpansion),
                (2076, Rule::QuotedBashRegex),
                (2078, Rule::TruthyLiteralTest),
                (2086, Rule::UnquotedExpansion),
                (2104, Rule::LoopControlOutsideLoop),
                (2112, Rule::FunctionKeyword),
                (2126, Rule::GrepCountPipeline),
                (2154, Rule::UndefinedVariable),
                (2155, Rule::ExportCommandSubstitution),
                (2157, Rule::ConstantComparisonTest),
                (2158, Rule::LiteralUnaryStringTest),
                (2162, Rule::ReadWithoutRaw),
                (2164, Rule::UncheckedDirectoryChange),
                (2168, Rule::LocalTopLevel),
                (2194, Rule::ConstantCaseSubject),
                (2210, Rule::BadRedirectionFdOrder),
                (2216, Rule::PipeToKill),
                (2233, Rule::SingleTestSubshell),
                (2235, Rule::SubshellTestGroup),
                (2238, Rule::RedirectToCommandName),
                (2239, Rule::NonAbsoluteShebang),
                (2241, Rule::InvalidExitStatus),
                (2242, Rule::CasePatternVar),
                (2248, Rule::BareSlashMarker),
                (2250, Rule::PatternWithVariable),
                (2255, Rule::SubstWithRedirect),
                (2256, Rule::SubstWithRedirectErr),
                (2257, Rule::ArithmeticRedirectionTarget),
                (2259, Rule::SubshellTestGroup),
                (2264, Rule::NestedParameterExpansion),
                (2266, Rule::OverwrittenFunction),
                (2267, Rule::LiteralBackslashInSingleQuotes),
                (2270, Rule::IfMissingThen),
                (2271, Rule::ElseWithoutThen),
                (2272, Rule::MissingSemicolonBeforeBrace),
                (2273, Rule::EmptyFunctionBody),
                (2274, Rule::BareClosingBrace),
                (2288, Rule::TemplateBraceInCommand),
                (2319, Rule::StatusCaptureAfterBranchTest),
                (2321, Rule::FunctionKeywordInSh),
                (2365, Rule::UnreachableAfterExit),
                (2385, Rule::UnicodeSingleQuoteInSingleQuotes),
                (3010, Rule::DoubleBracketInSh),
                (3012, Rule::GreaterThanInDoubleBracket),
                (3014, Rule::TestEqualityOperator),
                (3015, Rule::RegexMatchInSh),
                (3016, Rule::VTestInSh),
                (3017, Rule::ATestInSh),
                (3039, Rule::LetCommand),
                (3042, Rule::LetCommand),
                (3043, Rule::LocalVariableInSh),
                (3044, Rule::DeclareCommand),
                (3046, Rule::SourceBuiltinInSh),
                (3051, Rule::SourceInsideFunctionInSh),
                (3062, Rule::OptionTestInSh),
                (3065, Rule::StickyBitTestInSh),
                (3067, Rule::OwnershipTestInSh),
                (3084, Rule::SourceInsideFunctionInSh),
            ]
        );
    }

    #[test]
    fn every_real_rule_has_a_primary_shellcheck_mapping() {
        let map = ShellCheckCodeMap::default();
        let mapped_rules: std::collections::HashSet<Rule> =
            map.mappings().map(|(_, rule)| rule).collect();
        let allowed_unmapped = std::collections::HashSet::from([
            Rule::IfElifBashTest,
            Rule::ExtendedGlobInTest,
            Rule::ArraySubscriptTest,
            Rule::ArraySubscriptCondition,
            Rule::ExtglobInTest,
            Rule::BackslashBeforeCommand,
        ]);

        let unmapped: Vec<&str> = Rule::iter()
            .filter(|r| !mapped_rules.contains(r) && !allowed_unmapped.contains(r))
            .map(|r| r.code())
            .collect();

        assert!(
            unmapped.is_empty(),
            "rules without a shellcheck mapping: {unmapped:?}"
        );
    }

    #[test]
    fn comparison_mappings_skip_ambiguous_codes() {
        let comparison = ShellCheckCodeMap::default()
            .comparison_mappings()
            .collect::<Vec<_>>();

        assert!(comparison.contains(&(3039, Rule::LetCommand)));
        assert!(comparison.contains(&(3042, Rule::LetCommand)));
        assert!(comparison.contains(&(3046, Rule::SourceBuiltinInSh)));
        assert!(comparison.contains(&(3051, Rule::SourceInsideFunctionInSh)));
        assert!(!comparison.contains(&(2321, Rule::FunctionKeywordInSh)));
        assert!(!comparison.contains(&(3084, Rule::SourceInsideFunctionInSh)));
        assert!(!comparison.contains(&(3044, Rule::DeclareCommand)));
    }
}
