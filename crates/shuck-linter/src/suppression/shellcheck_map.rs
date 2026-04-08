use rustc_hash::FxHashMap;

use crate::Rule;

/// Maps shellcheck SC codes to shuck rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckCodeMap {
    map: FxHashMap<u32, Rule>,
}

impl ShellCheckCodeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mappings(&self) -> impl Iterator<Item = (u32, Rule)> + '_ {
        self.map.iter().map(|(sc_code, rule)| (*sc_code, *rule))
    }

    /// Look up a shellcheck code like `SC2086`.
    pub fn resolve(&self, sc_code: &str) -> Option<Rule> {
        let number = sc_code
            .strip_prefix("SC")
            .or_else(|| sc_code.strip_prefix("sc"))?
            .parse()
            .ok()?;
        self.map.get(&number).copied()
    }
}

impl Default for ShellCheckCodeMap {
    fn default() -> Self {
        Self {
            map: FxHashMap::from_iter([
                (2005, Rule::EchoedCommandSubstitution),
                (2006, Rule::LegacyBackticks),
                (2007, Rule::LegacyArithmeticExpansion),
                (1037, Rule::PositionalTenBraces),
                (1090, Rule::DynamicSourcePath),
                (2016, Rule::SingleQuotedLiteral),
                (2013, Rule::LineOrientedInput),
                (2015, Rule::ChainedTestBranches),
                (1019, Rule::EmptyTest),
                (2024, Rule::SudoRedirectionOrder),
                (2034, Rule::UnusedAssignment),
                (2044, Rule::FindOutputLoop),
                (2045, Rule::LoopFromCommandOutput),
                (2046, Rule::UnquotedCommandSubstitution),
                (2059, Rule::PrintfFormatVariable),
                (2038, Rule::FindOutputToXargs),
                (2064, Rule::TrapStringExpansion),
                (2068, Rule::UnquotedArrayExpansion),
                (2076, Rule::QuotedBashRegex),
                (2086, Rule::UnquotedExpansion),
                (2104, Rule::LoopControlOutsideLoop),
                (2216, Rule::PipeToKill),
                (2155, Rule::ExportCommandSubstitution),
                (2157, Rule::ConstantComparisonTest),
                (2158, Rule::LiteralUnaryStringTest),
                (2162, Rule::ReadWithoutRaw),
                (2078, Rule::TruthyLiteralTest),
                (2168, Rule::LocalTopLevel),
                (2194, Rule::ConstantCaseSubject),
                (2154, Rule::UndefinedVariable),
                (2241, Rule::InvalidExitStatus),
                (2242, Rule::CasePatternVar),
                (2257, Rule::ArithmeticRedirectionTarget),
                (2250, Rule::PatternWithVariable),
                (2255, Rule::SubstWithRedirect),
                (2256, Rule::SubstWithRedirectErr),
                (2266, Rule::OverwrittenFunction),
                (2365, Rule::UnreachableAfterExit),
            ]),
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
        assert_eq!(map.resolve("SC1037"), Some(Rule::PositionalTenBraces));
        assert_eq!(map.resolve("SC1090"), Some(Rule::DynamicSourcePath));
        assert_eq!(map.resolve("SC2016"), Some(Rule::SingleQuotedLiteral));
        assert_eq!(map.resolve("SC2013"), Some(Rule::LineOrientedInput));
        assert_eq!(map.resolve("SC2015"), Some(Rule::ChainedTestBranches));
        assert_eq!(map.resolve("SC1019"), Some(Rule::EmptyTest));
        assert_eq!(map.resolve("SC2024"), Some(Rule::SudoRedirectionOrder));
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
        assert_eq!(map.resolve("SC2216"), Some(Rule::PipeToKill));
        assert_eq!(map.resolve("SC2155"), Some(Rule::ExportCommandSubstitution));
        assert_eq!(map.resolve("SC2157"), Some(Rule::ConstantComparisonTest));
        assert_eq!(map.resolve("SC2158"), Some(Rule::LiteralUnaryStringTest));
        assert_eq!(map.resolve("SC2078"), Some(Rule::TruthyLiteralTest));
        assert_eq!(map.resolve("SC2162"), Some(Rule::ReadWithoutRaw));
        assert_eq!(map.resolve("SC2168"), Some(Rule::LocalTopLevel));
        assert_eq!(map.resolve("SC2194"), Some(Rule::ConstantCaseSubject));
        assert_eq!(map.resolve("sc2154"), Some(Rule::UndefinedVariable));
        assert_eq!(map.resolve("SC2241"), Some(Rule::InvalidExitStatus));
        assert_eq!(map.resolve("SC2242"), Some(Rule::CasePatternVar));
        assert_eq!(
            map.resolve("SC2257"),
            Some(Rule::ArithmeticRedirectionTarget)
        );
        assert_eq!(map.resolve("SC2250"), Some(Rule::PatternWithVariable));
        assert_eq!(map.resolve("SC2255"), Some(Rule::SubstWithRedirect));
        assert_eq!(map.resolve("SC2256"), Some(Rule::SubstWithRedirectErr));
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
                (1019, Rule::EmptyTest),
                (1037, Rule::PositionalTenBraces),
                (1090, Rule::DynamicSourcePath),
                (2005, Rule::EchoedCommandSubstitution),
                (2006, Rule::LegacyBackticks),
                (2007, Rule::LegacyArithmeticExpansion),
                (2013, Rule::LineOrientedInput),
                (2015, Rule::ChainedTestBranches),
                (2016, Rule::SingleQuotedLiteral),
                (2024, Rule::SudoRedirectionOrder),
                (2034, Rule::UnusedAssignment),
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
                (2154, Rule::UndefinedVariable),
                (2155, Rule::ExportCommandSubstitution),
                (2157, Rule::ConstantComparisonTest),
                (2158, Rule::LiteralUnaryStringTest),
                (2162, Rule::ReadWithoutRaw),
                (2168, Rule::LocalTopLevel),
                (2194, Rule::ConstantCaseSubject),
                (2216, Rule::PipeToKill),
                (2241, Rule::InvalidExitStatus),
                (2242, Rule::CasePatternVar),
                (2250, Rule::PatternWithVariable),
                (2255, Rule::SubstWithRedirect),
                (2256, Rule::SubstWithRedirectErr),
                (2257, Rule::ArithmeticRedirectionTarget),
                (2266, Rule::OverwrittenFunction),
                (2365, Rule::UnreachableAfterExit),
            ]
        );
    }

    #[test]
    fn every_real_rule_has_a_shellcheck_mapping() {
        let map = ShellCheckCodeMap::default();
        let mapped_rules: std::collections::HashSet<Rule> =
            map.mappings().map(|(_, rule)| rule).collect();

        let unmapped: Vec<&str> = Rule::iter()
            .filter(|r| !mapped_rules.contains(r))
            .map(|r| r.code())
            .collect();

        assert!(
            unmapped.is_empty(),
            "rules without a shellcheck mapping: {unmapped:?}"
        );
    }
}
