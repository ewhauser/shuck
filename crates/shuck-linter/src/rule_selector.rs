use std::str::FromStr;

use thiserror::Error;

use crate::{Category, Rule, RuleSet, code_to_rule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSelector {
    All,
    Category(Category),
    Prefix(String),
    Rule(Rule),
}

impl RuleSelector {
    pub fn into_rule_set(&self) -> RuleSet {
        match self {
            Self::All => RuleSet::all(),
            Self::Category(category) => Rule::iter()
                .filter(|rule| rule.category() == *category)
                .collect(),
            Self::Prefix(prefix) => Rule::iter()
                .filter(|rule| rule.code().starts_with(prefix))
                .collect(),
            Self::Rule(rule) => std::iter::once(*rule).collect(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SelectorParseError {
    #[error("unknown rule selector `{0}`")]
    Unknown(String),
}

impl FromStr for RuleSelector {
    type Err = SelectorParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let selector = value.trim();
        if selector == "ALL" {
            return Ok(Self::All);
        }

        if let Some(rule) = code_to_rule(selector) {
            return Ok(Self::Rule(rule));
        }

        if let Some(category) = Category::from_prefix(selector) {
            return Ok(Self::Category(category));
        }

        if Rule::iter().any(|rule| rule.code().starts_with(selector)) {
            return Ok(Self::Prefix(selector.to_owned()));
        }

        Err(SelectorParseError::Unknown(selector.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_category_prefixes() {
        let selector = RuleSelector::from_str("C").unwrap();
        assert_eq!(selector, RuleSelector::Category(Category::Correctness));
    }

    #[test]
    fn expands_exact_rules() {
        let selector = RuleSelector::from_str("C124").unwrap();
        assert_eq!(
            selector.into_rule_set().iter().collect::<Vec<_>>(),
            vec![Rule::UnreachableAfterExit]
        );
    }

    #[test]
    fn parses_rule_prefixes() {
        let selector = RuleSelector::from_str("C12").unwrap();
        assert_eq!(
            selector.into_rule_set().iter().collect::<Vec<_>>(),
            vec![
                Rule::FunctionReferencesUnsetParam,
                Rule::UnreachableAfterExit,
                Rule::UnusedHeredoc,
                Rule::UncheckedDirectoryChangeInFunction,
                Rule::ContinueOutsideLoopInFunction,
            ]
        );
    }
}
