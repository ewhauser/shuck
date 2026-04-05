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
    ("C005", Category::Correctness, Severity::Warning, SingleQuotedLiteral),
    ("C006", Category::Correctness, Severity::Error, UndefinedVariable),
    ("C007", Category::Correctness, Severity::Warning, FindOutputToXargs),
    ("C008", Category::Correctness, Severity::Warning, TrapStringExpansion),
    ("C009", Category::Correctness, Severity::Warning, QuotedBashRegex),
    ("C013", Category::Correctness, Severity::Warning, FindOutputLoop),
    ("C014", Category::Correctness, Severity::Error, LocalTopLevel),
    ("C015", Category::Correctness, Severity::Warning, SudoRedirectionOrder),
    ("C017", Category::Correctness, Severity::Warning, ConstantComparisonTest),
    ("C018", Category::Correctness, Severity::Error, LoopControlOutsideLoop),
    ("C019", Category::Correctness, Severity::Warning, LiteralUnaryStringTest),
    ("C020", Category::Correctness, Severity::Warning, TruthyLiteralTest),
    ("C021", Category::Correctness, Severity::Warning, ConstantCaseSubject),
    ("C022", Category::Correctness, Severity::Error, EmptyTest),
    ("C046", Category::Correctness, Severity::Warning, PipeToKill),
    ("C063", Category::Correctness, Severity::Warning, OverwrittenFunction),
    ("C124", Category::Correctness, Severity::Warning, UnreachableAfterExit),
    ("S001", Category::Style, Severity::Warning, UnquotedExpansion),
    ("C999", Category::Correctness, Severity::Warning, NoopPlaceholder),
}

pub fn code_to_rule(code: &str) -> Option<Rule> {
    canonical_code_to_rule(code).or(match code {
        "SH-001" => Some(Rule::UnquotedExpansion),
        "SH-003" => Some(Rule::UnusedAssignment),
        "SH-036" => Some(Rule::SingleQuotedLiteral),
        "SH-039" => Some(Rule::UndefinedVariable),
        "SH-041" => Some(Rule::FindOutputToXargs),
        "SH-042" => Some(Rule::TrapStringExpansion),
        "SH-043" => Some(Rule::QuotedBashRegex),
        "SH-049" => Some(Rule::FindOutputLoop),
        "SH-052" => Some(Rule::LocalTopLevel),
        "SH-060" => Some(Rule::SudoRedirectionOrder),
        "SH-069" => Some(Rule::ConstantComparisonTest),
        "SH-070" => Some(Rule::LoopControlOutsideLoop),
        "SH-072" => Some(Rule::LiteralUnaryStringTest),
        "SH-073" => Some(Rule::TruthyLiteralTest),
        "SH-074" => Some(Rule::ConstantCaseSubject),
        "SH-075" => Some(Rule::EmptyTest),
        "SH-134" => Some(Rule::PipeToKill),
        "SH-171" => Some(Rule::OverwrittenFunction),
        "SH-293" => Some(Rule::UnreachableAfterExit),
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
        assert_eq!(code_to_rule("SH-036"), Some(Rule::SingleQuotedLiteral));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("SH-041"), Some(Rule::FindOutputToXargs));
        assert_eq!(code_to_rule("SH-042"), Some(Rule::TrapStringExpansion));
        assert_eq!(code_to_rule("SH-043"), Some(Rule::QuotedBashRegex));
        assert_eq!(code_to_rule("SH-049"), Some(Rule::FindOutputLoop));
        assert_eq!(code_to_rule("SH-052"), Some(Rule::LocalTopLevel));
        assert_eq!(code_to_rule("SH-060"), Some(Rule::SudoRedirectionOrder));
        assert_eq!(code_to_rule("SH-069"), Some(Rule::ConstantComparisonTest));
        assert_eq!(code_to_rule("SH-070"), Some(Rule::LoopControlOutsideLoop));
        assert_eq!(code_to_rule("SH-072"), Some(Rule::LiteralUnaryStringTest));
        assert_eq!(code_to_rule("SH-073"), Some(Rule::TruthyLiteralTest));
        assert_eq!(code_to_rule("SH-074"), Some(Rule::ConstantCaseSubject));
        assert_eq!(code_to_rule("SH-075"), Some(Rule::EmptyTest));
        assert_eq!(code_to_rule("SH-134"), Some(Rule::PipeToKill));
        assert_eq!(code_to_rule("SH-171"), Some(Rule::OverwrittenFunction));
        assert_eq!(code_to_rule("C006"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("C124"), Some(Rule::UnreachableAfterExit));
        assert_eq!(code_to_rule("SH-293"), Some(Rule::UnreachableAfterExit));
    }
}
