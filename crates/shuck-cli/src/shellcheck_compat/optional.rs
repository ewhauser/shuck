use shuck_linter::Rule;
use shuck_linter::RuleSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionalCheckBehavior {
    None,
    ReportEnvironmentStyleNames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionalCheck {
    pub name: &'static str,
    pub description: &'static str,
    pub example: &'static str,
    pub guidance: &'static str,
    pub enable_rules: &'static [Rule],
    pub default_disabled_rules: &'static [Rule],
    pub behavior: OptionalCheckBehavior,
    pub supported: bool,
}

impl OptionalCheck {
    pub fn enabled_rule_set(self) -> RuleSet {
        self.enable_rules.iter().copied().collect()
    }
}

pub const OPTIONAL_CHECKS: &[OptionalCheck] = &[
    OptionalCheck {
        name: "add-default-case",
        description: "Reports case statements that omit a fallback branch.",
        example: "case $? in 0) echo ok ;; esac",
        guidance: "Add a catch-all branch when the script should handle unexpected values.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "avoid-negated-conditions",
        description: "Prefers direct comparison operators over leading negation in tests.",
        example: "[ ! \"$value\" -eq 1 ]",
        guidance: "Rewrite the operator so the intent stays positive without a leading !.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "avoid-nullary-conditions",
        description: "Flags bare [ \"$var\" ] checks that rely on implicit non-empty semantics.",
        example: "[ \"$var\" ]",
        guidance: "Use an explicit string or numeric operator for the condition you mean.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "check-extra-masked-returns",
        description: "Looks for command substitutions whose failures are easy to miss.",
        example: "rm -r \"$(helper)/home\"",
        guidance: "Split the substitution into a checked step before using the result.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "check-set-e-suppressed",
        description: "Notes call sites where set -e is neutralized by the surrounding construct.",
        example: "set -e; build && echo ok",
        guidance: "Run the function as its own command when failures should still abort the script.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "check-unassigned-uppercase",
        description: "Adds uppercase-variable coverage to unset-variable style checks.",
        example: "echo $VAR",
        guidance: "Initialize the uppercase name before reading it when it is not inherited from the environment.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::ReportEnvironmentStyleNames,
        supported: true,
    },
    OptionalCheck {
        name: "deprecate-which",
        description: "Discourages the non-portable which utility in favor of shell builtins.",
        example: "which javac",
        guidance: "Prefer command -v when you only need command lookup.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "quote-safe-variables",
        description: "Requests quotes even for scalar variables that look safe today.",
        example: "name=hello; echo $name",
        guidance: "Wrap the expansion in double quotes when consistency matters more than brevity.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "require-double-brackets",
        description: "Requires [[ ... ]] in shells where that test form is available.",
        example: "[ -e /etc/issue ]",
        guidance: "Use the double-bracket form only when the selected shell actually supports it.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "require-variable-braces",
        description: "Prefers ${name} over bare $name references.",
        example: "name=hello; echo $name",
        guidance: "Add braces when you want every variable reference to follow the same house style.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
    OptionalCheck {
        name: "useless-use-of-cat",
        description: "Looks for cat pipelines that can be replaced by a direct file operand.",
        example: "cat foo | grep bar",
        guidance: "Pass the file to the downstream command directly when it reads files itself.",
        enable_rules: &[],
        default_disabled_rules: &[],
        behavior: OptionalCheckBehavior::None,
        supported: false,
    },
];

pub fn compat_default_disabled_rules() -> RuleSet {
    OPTIONAL_CHECKS
        .iter()
        .filter(|check| check.supported)
        .flat_map(|check| check.default_disabled_rules.iter().copied())
        .collect()
}

pub fn supported_optional_checks() -> impl Iterator<Item = &'static OptionalCheck> {
    OPTIONAL_CHECKS.iter().filter(|check| check.supported)
}

pub fn find_optional_check(name: &str) -> Option<&'static OptionalCheck> {
    OPTIONAL_CHECKS.iter().find(|check| check.name == name)
}
