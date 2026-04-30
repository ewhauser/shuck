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
}
