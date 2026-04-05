use crate::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Correctness,
    Style,
    Performance,
    Portability,
    Security,
}

impl Category {
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Correctness => "C",
            Self::Style => "S",
            Self::Performance => "P",
            Self::Portability => "X",
            Self::Security => "K",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "C" => Some(Self::Correctness),
            "S" => Some(Self::Style),
            "P" => Some(Self::Performance),
            "X" => Some(Self::Portability),
            "K" => Some(Self::Security),
            _ => None,
        }
    }
}

macro_rules! declare_rules {
    (@sub $name:ident) => {
        ()
    };
    (@count $($name:ident),+ $(,)?) => {
        <[()]>::len(&[$(declare_rules!(@sub $name)),+])
    };
    ($(
        ($code:literal, $category:expr, $severity:expr, $name:ident),
    )+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum Rule {
            $($name,)*
        }

        pub const ALL_RULES: [Rule; declare_rules!(@count $($name),+)] = [
            $(Rule::$name,)*
        ];

        impl Rule {
            pub const COUNT: usize = ALL_RULES.len();

            pub fn iter() -> impl ExactSizeIterator<Item = Self> + DoubleEndedIterator + Clone {
                ALL_RULES.into_iter()
            }

            pub const fn code(self) -> &'static str {
                match self {
                    $(Self::$name => $code,)*
                }
            }

            pub const fn category(self) -> Category {
                match self {
                    $(Self::$name => $category,)*
                }
            }

            pub const fn default_severity(self) -> Severity {
                match self {
                    $(Self::$name => $severity,)*
                }
            }
        }

        fn canonical_code_to_rule(code: &str) -> Option<Rule> {
            match code {
                $($code => Some(Rule::$name),)*
                _ => None,
            }
        }
    };
}

declare_rules! {
    ("C001", Category::Correctness, Severity::Warning, UnusedAssignment),
    ("C002", Category::Correctness, Severity::Error, UndefinedVariable),
    ("C014", Category::Correctness, Severity::Error, LocalTopLevel),
    ("S001", Category::Style, Severity::Warning, UnquotedExpansion),
    ("C999", Category::Correctness, Severity::Warning, NoopPlaceholder),
}

pub fn code_to_rule(code: &str) -> Option<Rule> {
    canonical_code_to_rule(code).or(match code {
        "SH-001" => Some(Rule::UnquotedExpansion),
        "SH-003" => Some(Rule::UnusedAssignment),
        "SH-052" => Some(Rule::LocalTopLevel),
        "SH-039" => Some(Rule::UndefinedVariable),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn rule_codes_are_unique() {
        let codes = Rule::iter().map(Rule::code).collect::<BTreeSet<_>>();
        assert_eq!(codes.len(), Rule::COUNT);
    }

    #[test]
    fn resolves_legacy_shuck_aliases() {
        assert_eq!(code_to_rule("SH-001"), Some(Rule::UnquotedExpansion));
        assert_eq!(code_to_rule("SH-003"), Some(Rule::UnusedAssignment));
        assert_eq!(code_to_rule("SH-052"), Some(Rule::LocalTopLevel));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
    }
}
