pub(crate) mod common;
pub mod correctness;
pub mod performance;
pub mod portability;
pub mod security;
pub mod style;

#[cfg(test)]
mod parity_tests;

#[cfg(test)]
mod architecture_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    const RULE_DIRS: &[&str] = &[
        "correctness",
        "performance",
        "portability",
        "security",
        "style",
    ];
    const FORBIDDEN_TOKENS: &[&str] = &[
        "WordPart",
        "WordPartNode",
        "ConditionalExpr",
        "PatternPart",
        "ParameterExpansionSyntax",
        "ZshExpansionTarget",
        "ConditionalCommand",
        "BourneParameterExpansion",
        "iter_commands",
        "query::",
    ];

    /// Rule-local helper files that predate the facts layer and still reach into AST
    /// traversal types. Each entry is debt: the goal is to move the structural work into
    /// `src/facts/` and delete the entry. Paths are relative to `src/rules/`.
    const TRAVERSAL_TOKEN_ALLOWLIST: &[&str] = &[
        // S001's safe-value classifier walks word parts, parameter expansions, and
        // conditional operands to decide whether an unquoted expansion can split or glob.
        // It should become a word-safety fact under `src/facts/` that S001 (and any other
        // quoting rule) consumes, at which point this entry goes away.
        "style/unquoted_expansion/safe_value.rs",
    ];

    fn collect_rule_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|error| {
                    panic!("failed to read entry in {}: {error}", dir.display())
                })
                .path();
            if path.is_dir() {
                collect_rule_sources(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn rule_modules_avoid_direct_ast_traversal_tokens() {
        let rules_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rules");
        let mut violations = Vec::new();
        let mut unused_allowlist = TRAVERSAL_TOKEN_ALLOWLIST.to_vec();

        for dir in RULE_DIRS {
            let category_dir = rules_root.join(dir);
            let mut paths = Vec::new();
            collect_rule_sources(&category_dir, &mut paths);
            paths.sort();

            for path in paths {
                // The category `mod.rs` only declares modules and fixture test cases.
                if path == category_dir.join("mod.rs") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&rules_root)
                    .expect("rule source lives under src/rules")
                    .to_string_lossy()
                    .replace('\\', "/");

                let source = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
                let found = FORBIDDEN_TOKENS
                    .iter()
                    .copied()
                    .filter(|token| source.contains(token))
                    .collect::<Vec<_>>();

                if TRAVERSAL_TOKEN_ALLOWLIST.contains(&relative.as_str()) {
                    // An allowlisted file that has been cleaned up should drop its entry.
                    if !found.is_empty() {
                        unused_allowlist.retain(|entry| *entry != relative);
                    }
                    continue;
                }
                for token in found {
                    violations.push(format!("{} contains `{token}`", path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "rule files should rely on fact/shared helper APIs instead of direct AST traversal:\n{}",
            violations.join("\n"),
        );
        assert!(
            unused_allowlist.is_empty(),
            "stale TRAVERSAL_TOKEN_ALLOWLIST entries (file moved or no longer needs an exemption):\n{}",
            unused_allowlist.join("\n"),
        );
    }

    #[test]
    fn rules_common_has_no_query_module() {
        let query_module = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rules/common/query.rs");

        assert!(
            !query_module.exists(),
            "rule-facing traversal helpers must live in facts, not rules/common/query.rs",
        );
    }

    #[test]
    fn facts_traversal_helpers_stay_private() {
        let traversal_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/facts/traversal.rs");
        let source = fs::read_to_string(&traversal_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", traversal_path.display()));
        let forbidden_visibility = [
            "pub(crate) struct CommandVisit",
            "pub(crate) fn visit_arithmetic_words",
            "pub(crate) fn visit_var_ref_subscript_words",
            "pub(crate) fn visit_subscript_words",
        ];
        let violations = forbidden_visibility
            .iter()
            .copied()
            .filter(|token| source.contains(token))
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "facts traversal helpers should stay private to the facts module:\n{}",
            violations.join("\n"),
        );
    }

    #[test]
    fn c100_rule_avoids_raw_zsh_option_state_queries() {
        let rule_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/rules/correctness/quoted_bash_source.rs");
        let source = fs::read_to_string(&rule_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", rule_path.display()));
        let forbidden_tokens = [
            "zsh_options_at",
            "zsh_ksh_arrays_runtime_state_at",
            "ZshOptionState",
            "OptionValue",
            "shell_behavior_at",
            "ArrayReferencePolicy",
        ];
        let violations = forbidden_tokens
            .iter()
            .copied()
            .filter(|token| source.contains(token))
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "C100 should consume behavior-partitioned facts instead of raw zsh option state:\n{}",
            violations.join("\n"),
        );
    }

    #[test]
    fn glob_sensitive_rules_avoid_raw_zsh_option_state_queries() {
        let rule_paths = [
            ("C012", "correctness/leading_glob_argument.rs"),
            ("K001", "security/rm_glob_on_variable_path.rs"),
            ("S001", "style/unquoted_expansion.rs"),
            ("S001", "style/unquoted_expansion/safe_value.rs"),
            ("S020", "style/single_iteration_loop.rs"),
            ("C055", "correctness/pattern_with_variable.rs"),
            ("C059", "correctness/redirect_to_command_name.rs"),
            ("C078", "correctness/unquoted_globs_in_find.rs"),
            ("C083", "correctness/glob_in_find_substitution.rs"),
            ("C084", "correctness/unquoted_grep_regex.rs"),
            ("C090", "correctness/glob_in_test_comparison.rs"),
            ("C094", "correctness/redirect_clobbers_input.rs"),
            ("C102", "correctness/glob_in_test_directory.rs"),
            ("C114", "correctness/glob_with_expansion_in_loop.rs"),
            ("C128", "correctness/case_glob_reachability.rs"),
            ("C129", "correctness/case_default_before_glob.rs"),
        ];
        let forbidden_tokens = [
            "zsh_options_at",
            "zsh_options()",
            "ZshOptionState",
            "OptionValue",
            "shell_behavior_at",
        ];
        let mut violations = Vec::new();

        for (rule, relative_path) in rule_paths {
            let rule_path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/rules")
                .join(relative_path);
            let source = fs::read_to_string(&rule_path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", rule_path.display()));
            violations.extend(
                forbidden_tokens
                    .iter()
                    .copied()
                    .filter(|token| source.contains(token))
                    .map(|token| format!("{rule} {} contains `{token}`", rule_path.display())),
            );
        }

        assert!(
            violations.is_empty(),
            "glob-sensitive rules should consume behavior-partitioned facts instead of raw zsh option state:\n{}",
            violations.join("\n"),
        );
    }

    #[test]
    fn brace_and_bracket_rules_avoid_raw_zsh_option_state_queries() {
        let rule_paths = [
            "src/rules/correctness/suspicious_bracket_glob.rs",
            "src/rules/portability/brace_expansion.rs",
        ];
        let forbidden_tokens = [
            "zsh_options_at",
            "zsh_options()",
            "ZshOptionState",
            "OptionValue",
            "shell_behavior_at",
            "BraceCharacterClassBehavior",
        ];
        let mut violations = Vec::new();
        for path in rule_paths {
            let rule_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
            let source = fs::read_to_string(&rule_path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", rule_path.display()));
            violations.extend(
                forbidden_tokens
                    .iter()
                    .copied()
                    .filter(|token| source.contains(token))
                    .map(|token| format!("{path}: {token}")),
            );
        }

        assert!(
            violations.is_empty(),
            "brace/glob rules should consume option-aware facts instead of raw zsh option state:\n{}",
            violations.join("\n"),
        );
    }
}
