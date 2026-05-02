use crate::{Rule, RuleSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedGroup {
    Google,
}

impl NamedGroup {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Google => "google",
        }
    }

    pub const fn rule_set(self) -> RuleSet {
        match self {
            Self::Google => GOOGLE_RULE_SET,
        }
    }
}

const GOOGLE_RULES: [Rule; 17] = [
    Rule::UnquotedExpansion,
    Rule::ReadWithoutRaw,
    Rule::UnquotedCommandSubstitution,
    Rule::LegacyBackticks,
    Rule::LegacyArithmeticExpansion,
    Rule::UnquotedArrayExpansion,
    Rule::ExportCommandSubstitution,
    Rule::UnquotedArraySplit,
    Rule::AvoidLetBuiltin,
    Rule::GetoptsInvalidFlagHandler,
    Rule::UnusedAssignment,
    Rule::UncheckedDirectoryChange,
    Rule::UndefinedVariable,
    Rule::ChainedTestBranches,
    Rule::LeadingGlobArgument,
    Rule::QuotedArraySlice,
    Rule::QuotedBashSource,
];

pub(crate) const GOOGLE_RULE_SET: RuleSet = RuleSet::from_rules(&GOOGLE_RULES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_membership_matches_current_v1_subset() {
        let mut codes = NamedGroup::Google
            .rule_set()
            .iter()
            .map(Rule::code)
            .collect::<Vec<_>>();
        codes.sort_unstable();

        assert_eq!(
            codes,
            vec![
                "C001", "C004", "C006", "C010", "C012", "C099", "C100", "S001", "S002", "S004",
                "S005", "S006", "S008", "S010", "S017", "S022", "S069",
            ]
        );
    }
}
