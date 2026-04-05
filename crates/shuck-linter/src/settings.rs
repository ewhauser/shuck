use rustc_hash::FxHashMap;

use crate::{Category, Rule, RuleSelector, RuleSet, Severity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinterSettings {
    pub rules: RuleSet,
    pub severity_overrides: FxHashMap<Rule, Severity>,
}

impl Default for LinterSettings {
    fn default() -> Self {
        Self {
            rules: Self::default_rules(),
            severity_overrides: FxHashMap::default(),
        }
    }
}

impl LinterSettings {
    pub fn default_rules() -> RuleSet {
        Rule::iter()
            .filter(|rule| matches!(rule.category(), Category::Correctness | Category::Security))
            .collect()
    }

    pub fn from_selectors(select: &[RuleSelector], ignore: &[RuleSelector]) -> Self {
        let mut rules = RuleSet::EMPTY;
        for selector in select {
            rules = rules.union(&selector.into_rule_set());
        }
        for selector in ignore {
            rules = rules.subtract(&selector.into_rule_set());
        }

        Self {
            rules,
            ..Self::default()
        }
    }
}
