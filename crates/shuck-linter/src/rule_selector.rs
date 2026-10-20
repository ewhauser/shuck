use std::collections::HashMap;
use std::str::FromStr;

use thiserror::Error;

use crate::{Category, NamedGroup, Rule, RuleSet, code_to_rule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSelector {
    All,
    Category(Category),
    Named(NamedGroup),
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
            Self::Named(group) => group.rule_set(),
            Self::Prefix(prefix) => Rule::iter()
                .filter(|rule| rule.code().starts_with(prefix))
                .collect(),
            Self::Rule(rule) => std::iter::once(*rule).collect(),
        }
    }

    /// How narrowly this selector targets rules. When one layer of selectors
    /// both enables and disables a rule, the more specific selector wins.
    pub fn specificity(&self) -> usize {
        match self {
            Self::All => 0,
            Self::Category(_) => 1,
            Self::Named(_) => 1,
            Self::Prefix(prefix) => 2 + prefix.len(),
            Self::Rule(_) => usize::MAX,
        }
    }
}

/// Parses configured rule selectors, rejecting empty entries. `scope` names
/// the setting in error messages, such as `lint.select`.
pub fn parse_rule_selectors(
    selectors: &[String],
    scope: &str,
) -> anyhow::Result<Vec<RuleSelector>> {
    selectors
        .iter()
        .map(|selector| {
            let selector = selector.trim();
            if selector.is_empty() {
                return Err(anyhow::anyhow!(
                    "invalid {scope} selector: selector cannot be empty"
                ));
            }

            selector
                .parse::<RuleSelector>()
                .map_err(|err| anyhow::anyhow!("invalid {scope} selector `{selector}`: {err}"))
        })
        .collect()
}

/// Applies one layer of `select` / `extend-select` / `ignore` selectors to
/// `current`.
///
/// Selectors are applied from least to most specific, so `select = ["ALL"]`
/// with `ignore = ["C001"]` drops only C001, and `select = ["C001"]` with
/// `ignore = ["C"]` keeps C001. When `select` is present it replaces
/// `current`; otherwise the layer edits `current` in place.
pub fn apply_rule_selector_layer(
    current: RuleSet,
    select: Option<&[RuleSelector]>,
    extend_select: &[RuleSelector],
    ignore: &[RuleSelector],
) -> RuleSet {
    let mut specificities = select
        .into_iter()
        .flatten()
        .chain(extend_select.iter())
        .chain(ignore.iter())
        .map(RuleSelector::specificity)
        .collect::<Vec<_>>();
    specificities.sort_unstable();
    specificities.dedup();

    let mut updates = HashMap::<Rule, bool>::new();
    for specificity in specificities {
        for selector in select
            .into_iter()
            .flatten()
            .chain(extend_select.iter())
            .filter(|selector| selector.specificity() == specificity)
        {
            for rule in selector.into_rule_set().iter() {
                updates.insert(rule, true);
            }
        }
        for selector in ignore
            .iter()
            .filter(|selector| selector.specificity() == specificity)
        {
            for rule in selector.into_rule_set().iter() {
                updates.insert(rule, false);
            }
        }
    }

    if select.is_some() {
        updates
            .into_iter()
            .filter_map(|(rule, enabled)| enabled.then_some(rule))
            .collect()
    } else {
        let mut rules = current;
        for (rule, enabled) in updates {
            rules.set(rule, enabled);
        }
        rules
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

        if let Some(group) = NamedGroup::from_name(selector) {
            return Ok(Self::Named(group));
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
    fn parses_named_groups() {
        let selector = RuleSelector::from_str("google").unwrap();
        assert_eq!(selector, RuleSelector::Named(NamedGroup::Google));
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
                Rule::ExprSubstrInTest,
                Rule::StringComparedWithEq,
                Rule::AFlagInDoubleBracket,
                Rule::FunctionReferencesUnsetParam,
                Rule::UnreachableAfterExit,
                Rule::UnusedHeredoc,
                Rule::UncheckedDirectoryChangeInFunction,
                Rule::ContinueOutsideLoopInFunction,
                Rule::CaseGlobReachability,
                Rule::CaseDefaultBeforeGlob,
            ]
        );
    }

    #[test]
    fn rejects_removed_rule_codes() {
        assert_eq!(
            RuleSelector::from_str("S026"),
            Err(SelectorParseError::Unknown("S026".to_owned()))
        );
        assert_eq!(
            RuleSelector::from_str("SH-092"),
            Err(SelectorParseError::Unknown("SH-092".to_owned()))
        );
        assert_eq!(
            RuleSelector::from_str("goog"),
            Err(SelectorParseError::Unknown("goog".to_owned()))
        );
    }

    #[test]
    fn named_group_can_be_trimmed_by_exact_rule_ignore() {
        let rules = apply_rule_selector_layer(
            RuleSet::EMPTY,
            Some(&[RuleSelector::Named(NamedGroup::Google)]),
            &[],
            &[RuleSelector::Rule(Rule::UnusedAssignment)],
        );

        assert!(!rules.contains(Rule::UnusedAssignment));
        assert!(rules.contains(Rule::UnquotedExpansion));
    }

    #[test]
    fn named_group_can_be_trimmed_by_prefix_ignore() {
        let rules = apply_rule_selector_layer(
            RuleSet::EMPTY,
            Some(&[RuleSelector::Named(NamedGroup::Google)]),
            &[],
            &[RuleSelector::Prefix("S00".to_owned())],
        );

        assert!(!rules.contains(Rule::UnquotedExpansion));
        assert!(!rules.contains(Rule::ReadWithoutRaw));
        assert!(rules.contains(Rule::ExportCommandSubstitution));
        assert!(rules.contains(Rule::UnusedAssignment));
    }
}
