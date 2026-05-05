pub(crate) mod common;
pub mod correctness;
pub mod performance;
pub mod portability;
pub mod security;
pub mod style;

#[cfg(test)]
mod architecture_tests {
    use std::fs;
    use std::path::Path;

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

    #[test]
    fn rule_modules_avoid_direct_ast_traversal_tokens() {
        let rules_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rules");
        let mut violations = Vec::new();

        for dir in RULE_DIRS {
            let category_dir = rules_root.join(dir);
            let entries = fs::read_dir(&category_dir).unwrap_or_else(|error| {
                panic!("failed to read {}: {error}", category_dir.display())
            });

            for entry in entries {
                let entry = entry.unwrap_or_else(|error| {
                    panic!(
                        "failed to read entry in {}: {error}",
                        category_dir.display()
                    )
                });
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                if matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("mod.rs" | "syntax.rs")
                ) {
                    continue;
                }

                let source = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
                for token in FORBIDDEN_TOKENS {
                    if source.contains(token) {
                        violations.push(format!("{} contains `{token}`", path.display()));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "rule files should rely on fact/shared helper APIs instead of direct AST traversal:\n{}",
            violations.join("\n"),
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
    fn c012_rule_avoids_raw_zsh_option_state_queries() {
        let rule_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/rules/correctness/leading_glob_argument.rs");
        let source = fs::read_to_string(&rule_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", rule_path.display()));
        let forbidden_tokens = [
            "zsh_options_at",
            "zsh_options()",
            "ZshOptionState",
            "OptionValue",
            "shell_behavior_at",
        ];
        let violations = forbidden_tokens
            .iter()
            .copied()
            .filter(|token| source.contains(token))
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "C012 should consume behavior-partitioned facts instead of raw zsh option state:\n{}",
            violations.join("\n"),
        );
    }

    #[test]
    fn k001_rule_avoids_raw_zsh_option_state_queries() {
        let rule_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/rules/security/rm_glob_on_variable_path.rs");
        let source = fs::read_to_string(&rule_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", rule_path.display()));
        let forbidden_tokens = [
            "zsh_options_at",
            "zsh_options()",
            "ZshOptionState",
            "OptionValue",
            "shell_behavior_at",
        ];
        let violations = forbidden_tokens
            .iter()
            .copied()
            .filter(|token| source.contains(token))
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "K001 should consume behavior-partitioned facts instead of raw zsh option state:\n{}",
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
