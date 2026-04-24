use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use globset::{Glob, GlobMatcher};
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use shuck_semantic::{UnreachedFunctionAnalysisOptions, UnusedAssignmentAnalysisOptions};

use crate::{Category, Rule, RuleSelector, RuleSet, Severity, ShellDialect};

// ShellCheck's optional checks currently map only to compat-only behavior
// toggles in this repository, not to standalone implemented non-style rules.
const DEFAULT_DISABLED_NON_STYLE_RULES: &[Rule] = &[];

/// Per-rule behavior overrides applied during lint analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinterRuleOptions {
    /// Behavior overrides for `C001`.
    pub c001: C001RuleOptions,
    /// Behavior overrides for `C063`.
    pub c063: C063RuleOptions,
}

/// Behavior overrides for `C001` unused-assignment analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct C001RuleOptions {
    /// Whether scalar indirect-expansion targets like `${!name}` count as a use of the target.
    /// Disabled by default to match ShellCheck. Array-like targets such as
    /// `name=arr[@]; ${!name}` stay live in both modes.
    pub treat_indirect_expansion_targets_as_used: bool,
}

impl C001RuleOptions {
    pub(crate) fn semantic_options(&self) -> UnusedAssignmentAnalysisOptions {
        UnusedAssignmentAnalysisOptions {
            treat_indirect_expansion_targets_as_used: self.treat_indirect_expansion_targets_as_used,
        }
    }
}

/// Behavior overrides for `C063` overwritten/unreached function analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct C063RuleOptions {
    /// Whether nested function definitions should be reported when no reachable direct call
    /// reaches the enclosing function scope before that scope exits.
    pub report_unreached_nested_definitions: bool,
}

impl C063RuleOptions {
    pub(crate) fn semantic_options(&self) -> UnreachedFunctionAnalysisOptions {
        UnreachedFunctionAnalysisOptions {
            report_unreached_nested_definitions: self.report_unreached_nested_definitions,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinterSettings {
    pub rules: RuleSet,
    pub severity_overrides: FxHashMap<Rule, Severity>,
    pub shell: ShellDialect,
    pub ambient_shell_options: AmbientShellOptions,
    pub analyzed_paths: Option<Arc<FxHashSet<PathBuf>>>,
    pub per_file_ignores: Arc<CompiledPerFileIgnoreList>,
    pub report_environment_style_names: bool,
    pub rule_options: LinterRuleOptions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AmbientShellOptions {
    pub errexit: bool,
    pub pipefail: bool,
}

impl Default for LinterSettings {
    fn default() -> Self {
        Self {
            rules: Self::default_rules(),
            severity_overrides: FxHashMap::default(),
            shell: ShellDialect::Unknown,
            ambient_shell_options: AmbientShellOptions::default(),
            analyzed_paths: None,
            per_file_ignores: Arc::new(CompiledPerFileIgnoreList::default()),
            report_environment_style_names: false,
            rule_options: LinterRuleOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerFileIgnore {
    pattern: String,
    rules: RuleSet,
}

impl PerFileIgnore {
    pub fn new(pattern: impl Into<String>, rules: RuleSet) -> Self {
        Self {
            pattern: pattern.into(),
            rules,
        }
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub const fn rules(&self) -> RuleSet {
        self.rules
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompiledPerFileIgnoreList {
    project_root: PathBuf,
    entries: Vec<CompiledPerFileIgnore>,
}

impl PartialEq for CompiledPerFileIgnoreList {
    fn eq(&self, other: &Self) -> bool {
        self.project_root == other.project_root && self.entries == other.entries
    }
}

impl Eq for CompiledPerFileIgnoreList {}

#[derive(Debug, Clone)]
struct CompiledPerFileIgnore {
    pattern: String,
    basename_matcher: GlobMatcher,
    relative_matcher: GlobMatcher,
    absolute_matcher: GlobMatcher,
    negated: bool,
    rules: RuleSet,
}

impl PartialEq for CompiledPerFileIgnore {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.negated == other.negated && self.rules == other.rules
    }
}

impl Eq for CompiledPerFileIgnore {}

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
            .filter(|rule| !matches!(rule.category(), Category::Style))
            .collect::<RuleSet>()
            .subtract(&default_disabled_non_style_rules())
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

    pub fn with_ambient_shell_options(
        mut self,
        ambient_shell_options: AmbientShellOptions,
    ) -> Self {
        self.ambient_shell_options = ambient_shell_options;
        self
    }

    pub fn analyzed_path_set(paths: impl IntoIterator<Item = PathBuf>) -> Arc<FxHashSet<PathBuf>> {
        Arc::new(
            paths
                .into_iter()
                .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
                .collect(),
        )
    }

    pub fn with_analyzed_path_set(mut self, paths: Arc<FxHashSet<PathBuf>>) -> Self {
        self.analyzed_paths = Some(paths);
        self
    }

    pub fn with_analyzed_paths(self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.with_analyzed_path_set(Self::analyzed_path_set(paths))
    }

    pub fn with_per_file_ignores(mut self, per_file_ignores: CompiledPerFileIgnoreList) -> Self {
        self.per_file_ignores = Arc::new(per_file_ignores);
        self
    }

    pub fn with_c001_treat_indirect_expansion_targets_as_used(mut self, value: bool) -> Self {
        self.rule_options
            .c001
            .treat_indirect_expansion_targets_as_used = value;
        self
    }

    pub fn with_c063_report_unreached_nested_definitions(mut self, value: bool) -> Self {
        self.rule_options.c063.report_unreached_nested_definitions = value;
        self
    }

    pub fn per_file_ignored_rules(&self, path: Option<&Path>) -> RuleSet {
        path.map_or(RuleSet::EMPTY, |path| {
            self.per_file_ignores.ignored_rules(path)
        })
    }
}

fn default_disabled_non_style_rules() -> RuleSet {
    DEFAULT_DISABLED_NON_STYLE_RULES.iter().copied().collect()
}

impl CompiledPerFileIgnoreList {
    pub fn resolve(
        project_root: impl Into<PathBuf>,
        per_file_ignores: impl IntoIterator<Item = PerFileIgnore>,
    ) -> Result<Self> {
        let project_root = project_root.into();
        let entries = per_file_ignores
            .into_iter()
            .map(|per_file_ignore| {
                let mut pattern = per_file_ignore.pattern().to_owned();
                let negated = pattern.starts_with('!');
                if negated {
                    pattern.drain(..1);
                }

                let basename_matcher = Glob::new(&pattern)
                    .with_context(|| format!("invalid glob {:?}", per_file_ignore.pattern()))?
                    .compile_matcher();
                let relative_matcher = Glob::new(&pattern)
                    .with_context(|| format!("invalid glob {:?}", per_file_ignore.pattern()))?
                    .compile_matcher();
                let absolute_matcher = Glob::new(&pattern)
                    .with_context(|| format!("invalid glob {:?}", per_file_ignore.pattern()))?
                    .compile_matcher();

                Ok(CompiledPerFileIgnore {
                    pattern: per_file_ignore.pattern().to_owned(),
                    basename_matcher,
                    relative_matcher,
                    absolute_matcher,
                    negated,
                    rules: per_file_ignore.rules(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            project_root,
            entries,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ignored_rules(&self, path: &Path) -> RuleSet {
        let relative_path = path.strip_prefix(&self.project_root).unwrap_or(path);
        let file_name = relative_path.file_name().or_else(|| path.file_name());
        let Some(file_name) = file_name else {
            return RuleSet::EMPTY;
        };

        self.entries.iter().fold(RuleSet::EMPTY, |ignored, entry| {
            let matches = entry.basename_matcher.is_match(file_name)
                || entry.relative_matcher.is_match(relative_path)
                || matches_absolute_path(&entry.absolute_matcher, path);
            let applies = if entry.negated { !matches } else { matches };

            if applies {
                ignored.union(&entry.rules)
            } else {
                ignored
            }
        })
    }
}

fn matches_absolute_path(matcher: &GlobMatcher, path: &Path) -> bool {
    matcher.is_match(path)
        || normalized_absolute_match_path(path)
            .as_deref()
            .is_some_and(|normalized| matcher.is_match(normalized))
}

fn normalized_absolute_match_path(path: &Path) -> Option<PathBuf> {
    let path = path.to_string_lossy();

    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        return Some(PathBuf::from(format!(r"\\{stripped}")));
    }

    path.strip_prefix(r"\\?\").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::*;
    use crate::RuleSet;

    #[test]
    fn default_rules_exclude_all_style_rules() {
        let defaults = LinterSettings::default_rules();

        for rule in Rule::iter().filter(|rule| matches!(rule.category(), Category::Style)) {
            assert!(
                !defaults.contains(rule),
                "{rule:?} should be disabled by default"
            );
        }
    }

    #[test]
    fn default_rules_include_non_style_rules() {
        let defaults = LinterSettings::default_rules();

        assert!(defaults.contains(Rule::UndefinedVariable));
        assert!(defaults.contains(Rule::ConstantCaseSubject));
        assert!(defaults.contains(Rule::RmGlobOnVariablePath));
        assert!(!defaults.contains(Rule::AmpersandSemicolon));
    }

    #[test]
    fn default_rules_exclude_verified_default_disabled_non_style_rules() {
        let defaults = LinterSettings::default_rules();

        for rule in DEFAULT_DISABLED_NON_STYLE_RULES {
            assert!(
                !defaults.contains(*rule),
                "{rule:?} should be excluded from the native default baseline"
            );
            assert!(
                !matches!(rule.category(), Category::Style),
                "{rule:?} must stay in the non-style default-disabled set"
            );
        }
    }

    #[test]
    fn with_analyzed_path_set_reuses_shared_set() {
        let tempdir = tempdir().unwrap();
        let script_path = tempdir.path().join("script.sh");
        std::fs::write(&script_path, "echo hi\n").unwrap();

        let analyzed_paths = LinterSettings::analyzed_path_set([script_path.clone()]);
        let settings =
            LinterSettings::default().with_analyzed_path_set(Arc::clone(&analyzed_paths));

        let stored = settings.analyzed_paths.as_ref().unwrap();
        assert!(Arc::ptr_eq(stored, &analyzed_paths));
        assert!(stored.contains(&std::fs::canonicalize(script_path).unwrap()));
    }

    #[test]
    fn matches_absolute_per_file_ignore_patterns() {
        let tempdir = tempdir().unwrap();
        let project_root = tempdir.path().to_path_buf();
        let script_path = project_root.join("nested").join("script.sh");
        let absolute_pattern = script_path
            .parent()
            .unwrap()
            .join("*.sh")
            .to_string_lossy()
            .into_owned();
        let per_file_ignores = CompiledPerFileIgnoreList::resolve(
            project_root,
            [PerFileIgnore::new(
                absolute_pattern,
                RuleSet::from_iter([Rule::UnusedAssignment]),
            )],
        )
        .unwrap();

        let ignored_rules = per_file_ignores.ignored_rules(&script_path);

        assert!(ignored_rules.contains(Rule::UnusedAssignment));
    }

    #[test]
    fn strips_windows_verbatim_disk_prefixes_for_absolute_matching() {
        assert_eq!(
            normalized_absolute_match_path(Path::new(r"\\?\C:\repo\nested\script.sh")),
            Some(PathBuf::from(r"C:\repo\nested\script.sh"))
        );
    }

    #[test]
    fn strips_windows_verbatim_unc_prefixes_for_absolute_matching() {
        assert_eq!(
            normalized_absolute_match_path(Path::new(r"\\?\UNC\server\share\script.sh")),
            Some(PathBuf::from(r"\\server\share\script.sh"))
        );
    }
}
