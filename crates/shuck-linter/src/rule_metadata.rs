use crate::{Rule, code_to_rule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellCheckLevel {
    Style,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMetadata {
    pub code: &'static str,
    pub shellcheck_level: Option<ShellCheckLevel>,
    pub description: &'static str,
    pub rationale: &'static str,
    pub fix_description: Option<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/rule_metadata_data.rs"));

/// Lifecycle of a spec in `docs/rules`, from its optional `status` field.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleSpecStatus {
    /// Default when `status` is omitted: a registered `Rule` must exist.
    Implemented,
    /// Specified but not built yet; must not be registered.
    Planned,
    /// Kept for documentation only; must not be registered.
    #[allow(dead_code)] // Only constructed by generated data once a spec is retired.
    Retired,
}

/// Registry-facing fields of a spec in `docs/rules`, used by the parity tests.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleSpec {
    pub(crate) code: &'static str,
    pub(crate) status: RuleSpecStatus,
    pub(crate) legacy_code: Option<&'static str>,
    pub(crate) aliases: &'static [&'static str],
}

#[cfg(test)]
include!(concat!(env!("OUT_DIR"), "/rule_spec_data.rs"));

pub fn rule_metadata(rule: Rule) -> Option<&'static RuleMetadata> {
    rule_metadata_by_code(rule.code())
}

pub fn rule_metadata_by_code(code: &str) -> Option<&'static RuleMetadata> {
    let rule = code_to_rule(code)?;
    RULE_METADATA
        .iter()
        .find(|metadata| metadata.code == rule.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_rule_metadata_for_known_rules() {
        let metadata = rule_metadata(Rule::UnusedAssignment).expect("metadata for C001");
        assert_eq!(metadata.code, "C001");
        assert_eq!(metadata.shellcheck_level, Some(ShellCheckLevel::Warning));
        assert!(metadata.description.contains("assigned"));
        assert!(metadata.rationale.contains("dead assignments"));
    }
}
