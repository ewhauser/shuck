use std::path::PathBuf;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

use crate::{Category, Rule, RuleSelector, RuleSet, Severity, ShellDialect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinterSettings {
    pub rules: RuleSet,
    pub severity_overrides: FxHashMap<Rule, Severity>,
    pub shell: ShellDialect,
    pub analyzed_paths: Option<Arc<FxHashSet<PathBuf>>>,
}

impl Default for LinterSettings {
    fn default() -> Self {
        Self {
            rules: Self::default_rules(),
            severity_overrides: FxHashMap::default(),
            shell: ShellDialect::Unknown,
            analyzed_paths: None,
        }
    }
}

impl LinterSettings {
    pub fn for_rule(rule: Rule) -> Self {
        Self {
            rules: RuleSet::from_iter([rule]),
            ..Self::default()
        }
    }

    pub fn for_rules(rules: impl IntoIterator<Item = Rule>) -> Self {
        Self {
            rules: rules.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn default_rules() -> RuleSet {
        Rule::iter()
            .filter(|rule| {
                matches!(rule.category(), Category::Correctness | Category::Security)
                    || matches!(rule, Rule::AmpersandSemicolon)
            })
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

    pub fn with_shell(mut self, shell: ShellDialect) -> Self {
        self.shell = shell;
        self
    }

    pub fn with_analyzed_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.analyzed_paths = Some(Arc::new(
            paths
                .into_iter()
                .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
                .collect(),
        ));
        self
    }
}
