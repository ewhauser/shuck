use rustc_hash::FxHashMap;

use crate::{Rule, code_to_rule};

include!(concat!(env!("OUT_DIR"), "/shellcheck_map_data.rs"));

const SUPPRESSION_ALIAS_CODES: &[(u32, Rule)] = &[
    // Older ShellCheck compatibility codes still appear in user suppressions.
    (2268, Rule::BackslashBeforeCommand),
    (2250, Rule::PatternWithVariable),
    (2350, Rule::XargsWithInlineReplace),
    (2316, Rule::BacktickInCommandPosition),
    (2362, Rule::LocalDeclareCombined),
    (2321, Rule::FunctionKeywordInSh),
    (2234, Rule::SingleTestSubshell),
    (2351, Rule::XPrefixInTest),
    (3084, Rule::SourceInsideFunctionInSh),
];

/// Maps ShellCheck SC codes to Shuck rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckCodeMap {
    sc_to_rule: FxHashMap<u32, Rule>,
    rule_to_sc: FxHashMap<Rule, u32>,
    suppression_aliases: Vec<(u32, Rule)>,
}

impl ShellCheckCodeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mappings(&self) -> impl Iterator<Item = (u32, Rule)> + '_ {
        self.sc_to_rule
            .iter()
            .map(|(sc_code, rule)| (*sc_code, *rule))
    }

    pub fn code_for_rule(&self, rule: Rule) -> Option<u32> {
        self.rule_to_sc.get(&rule).copied()
    }

    /// Look up a ShellCheck code like `SC2086`.
    pub fn resolve(&self, sc_code: &str) -> Option<Rule> {
        Self::parse_code(sc_code).and_then(|number| self.sc_to_rule.get(&number).copied())
    }

    /// Look up all ShellCheck mappings for a code like `SC2086`.
    pub fn resolve_all(&self, sc_code: &str) -> Vec<Rule> {
        let Some(number) = Self::parse_code(sc_code) else {
            return Vec::new();
        };

        let mut rules = self
            .sc_to_rule
            .get(&number)
            .copied()
            .into_iter()
            .collect::<Vec<_>>();
        for &(code, rule) in &self.suppression_aliases {
            if code == number && !rules.contains(&rule) {
                rules.push(rule);
            }
        }
        rules
    }

    fn parse_code(sc_code: &str) -> Option<u32> {
        sc_code
            .strip_prefix("SC")
            .or_else(|| sc_code.strip_prefix("sc"))
            .unwrap_or(sc_code)
            .parse()
            .ok()
    }
}

impl Default for ShellCheckCodeMap {
    fn default() -> Self {
        let mut sc_to_rule = FxHashMap::default();
        let mut rule_to_sc = FxHashMap::default();

        for &(rule_code, sc_code) in RULE_SHELLCHECK_CODES {
            let Some(rule) = code_to_rule(rule_code) else {
                continue;
            };

            let old_rule = sc_to_rule.insert(sc_code, rule);
            assert!(
                old_rule.is_none(),
                "duplicate ShellCheck mapping for SC{sc_code}: {old_rule:?} and {rule:?}"
            );

            let old_code = rule_to_sc.insert(rule, sc_code);
            assert!(
                old_code.is_none(),
                "duplicate Shuck mapping for {rule:?}: {old_code:?} and {sc_code}"
            );
        }

        Self {
            sc_to_rule,
            rule_to_sc,
            suppression_aliases: SUPPRESSION_ALIAS_CODES.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn resolves_known_codes_and_ignores_unknown_ones() {
        let map = ShellCheckCodeMap::default();

        assert_eq!(map.resolve("SC2034"), Some(Rule::UnusedAssignment));
        assert_eq!(map.resolve("2034"), Some(Rule::UnusedAssignment));
        assert_eq!(map.resolve("sc2154"), Some(Rule::UndefinedVariable));
        assert_eq!(map.resolve("SC2086"), Some(Rule::UnquotedExpansion));
        assert_eq!(map.resolve("SC1012"), Some(Rule::LiteralControlEscape));
        assert_eq!(map.resolve("SC2238"), Some(Rule::RedirectToCommandName));
        assert_eq!(map.resolve("SC2063"), Some(Rule::LeadingGlobInGrepPattern));
        assert_eq!(map.resolve("SC3034"), Some(Rule::BashFileSlurp));
        assert_eq!(map.resolve("SC2096"), Some(Rule::DuplicateShebangFlag));
        assert_eq!(map.resolve("SC2318"), Some(Rule::LocalCrossReference));
        assert_eq!(map.resolve("SC2260"), Some(Rule::RedirectBeforePipe));
        assert_eq!(
            map.resolve("SC1087"),
            Some(Rule::BraceVariableBeforeBracket)
        );
        assert_eq!(map.resolve("SC2317"), Some(Rule::UnreachableAfterExit));
        assert_eq!(map.resolve("SC7777"), None);
    }

    #[test]
    fn resolve_all_keeps_legacy_suppression_aliases() {
        let map = ShellCheckCodeMap::default();

        assert_eq!(map.resolve_all("SC2086"), vec![Rule::UnquotedExpansion]);
        assert_eq!(
            map.resolve_all("SC2268"),
            vec![Rule::XPrefixInTest, Rule::BackslashBeforeCommand]
        );
        assert_eq!(
            map.resolve_all("SC2316"),
            vec![Rule::LocalDeclareCombined, Rule::BacktickInCommandPosition]
        );
        assert_eq!(map.resolve_all("SC2362"), vec![Rule::LocalDeclareCombined]);
        assert_eq!(map.resolve_all("SC2250"), vec![Rule::PatternWithVariable]);
        assert_eq!(
            map.resolve_all("SC2350"),
            vec![Rule::XargsWithInlineReplace]
        );
        assert_eq!(
            map.resolve_all("SC2321"),
            vec![Rule::ArrayIndexArithmetic, Rule::FunctionKeywordInSh]
        );
        assert_eq!(map.resolve_all("SC2234"), vec![Rule::SingleTestSubshell]);
        assert_eq!(
            map.resolve_all("SC3084"),
            vec![Rule::SourceInsideFunctionInSh]
        );
        assert_eq!(map.resolve_all("SC2260"), vec![Rule::RedirectBeforePipe]);
    }

    #[test]
    fn mappings_are_unique_in_both_directions() {
        let map = ShellCheckCodeMap::default();

        let sc_codes = map
            .mappings()
            .map(|(sc_code, _)| sc_code)
            .collect::<Vec<_>>();
        let rules = map.mappings().map(|(_, rule)| rule).collect::<Vec<_>>();

        assert_eq!(
            sc_codes.len(),
            sc_codes.iter().copied().collect::<HashSet<_>>().len()
        );
        assert_eq!(
            rules.len(),
            rules.iter().copied().collect::<HashSet<_>>().len()
        );
    }

    #[test]
    fn can_reverse_lookup_canonical_codes() {
        let map = ShellCheckCodeMap::default();

        assert_eq!(map.code_for_rule(Rule::UnusedAssignment), Some(2034));
        assert_eq!(map.code_for_rule(Rule::RedirectToCommandName), Some(2238));
        assert_eq!(map.code_for_rule(Rule::DuplicateShebangFlag), Some(2096));
        assert_eq!(map.code_for_rule(Rule::BashFileSlurp), Some(3034));
        assert_eq!(map.code_for_rule(Rule::UnreachableAfterExit), Some(2317));
        assert_eq!(map.code_for_rule(Rule::LiteralControlEscape), Some(1012));
        assert_eq!(
            map.code_for_rule(Rule::BraceVariableBeforeBracket),
            Some(1087)
        );
        assert_eq!(map.code_for_rule(Rule::BackslashBeforeCommand), None);
        assert_eq!(
            map.code_for_rule(Rule::LeadingGlobInGrepPattern),
            Some(2063)
        );
        assert_eq!(map.code_for_rule(Rule::MixedAndOrInCondition), None);
    }
}
