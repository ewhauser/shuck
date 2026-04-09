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
    ("C002", Category::Correctness, Severity::Warning, DynamicSourcePath),
    ("C003", Category::Correctness, Severity::Warning, UntrackedSourceFile),
    ("C004", Category::Correctness, Severity::Warning, UncheckedDirectoryChange),
    ("C005", Category::Correctness, Severity::Warning, SingleQuotedLiteral),
    ("C006", Category::Correctness, Severity::Error, UndefinedVariable),
    ("C007", Category::Correctness, Severity::Warning, FindOutputToXargs),
    ("C008", Category::Correctness, Severity::Warning, TrapStringExpansion),
    ("C009", Category::Correctness, Severity::Warning, QuotedBashRegex),
    ("C010", Category::Correctness, Severity::Warning, ChainedTestBranches),
    ("C011", Category::Correctness, Severity::Warning, LineOrientedInput),
    ("C012", Category::Correctness, Severity::Warning, LeadingGlobArgument),
    ("C013", Category::Correctness, Severity::Warning, FindOutputLoop),
    ("C014", Category::Correctness, Severity::Error, LocalTopLevel),
    ("C015", Category::Correctness, Severity::Warning, SudoRedirectionOrder),
    ("C017", Category::Correctness, Severity::Warning, ConstantComparisonTest),
    ("C018", Category::Correctness, Severity::Error, LoopControlOutsideLoop),
    ("C019", Category::Correctness, Severity::Warning, LiteralUnaryStringTest),
    ("C020", Category::Correctness, Severity::Warning, TruthyLiteralTest),
    ("C021", Category::Correctness, Severity::Warning, ConstantCaseSubject),
    ("C022", Category::Correctness, Severity::Error, EmptyTest),
    ("C025", Category::Correctness, Severity::Warning, PositionalTenBraces),
    ("C035", Category::Correctness, Severity::Error, MissingFi),
    ("C036", Category::Correctness, Severity::Error, BrokenTestEnd),
    ("C037", Category::Correctness, Severity::Error, BrokenTestParse),
    ("C038", Category::Correctness, Severity::Error, ElseIf),
    ("C039", Category::Correctness, Severity::Warning, OpenDoubleQuote),
    ("C040", Category::Correctness, Severity::Error, LinebreakInTest),
    ("C041", Category::Correctness, Severity::Error, CStyleComment),
    ("C042", Category::Correctness, Severity::Warning, CPrototypeFragment),
    ("C043", Category::Correctness, Severity::Warning, BadRedirectionFdOrder),
    ("C046", Category::Correctness, Severity::Warning, PipeToKill),
    ("C047", Category::Correctness, Severity::Error, InvalidExitStatus),
    ("C048", Category::Correctness, Severity::Warning, CasePatternVar),
    ("C050", Category::Correctness, Severity::Warning, ArithmeticRedirectionTarget),
    ("C054", Category::Correctness, Severity::Warning, BareSlashMarker),
    ("C055", Category::Correctness, Severity::Warning, PatternWithVariable),
    ("C056", Category::Correctness, Severity::Warning, StatusCaptureAfterBranchTest),
    ("C057", Category::Correctness, Severity::Warning, SubstWithRedirect),
    ("C058", Category::Correctness, Severity::Warning, SubstWithRedirectErr),
    ("C059", Category::Correctness, Severity::Warning, RedirectToCommandName),
    ("C060", Category::Correctness, Severity::Warning, NonAbsoluteShebang),
    ("C061", Category::Correctness, Severity::Warning, TemplateBraceInCommand),
    ("C062", Category::Correctness, Severity::Warning, NestedParameterExpansion),
    ("C063", Category::Correctness, Severity::Warning, OverwrittenFunction),
    ("C064", Category::Correctness, Severity::Warning, IfMissingThen),
    ("C065", Category::Correctness, Severity::Warning, ElseWithoutThen),
    ("C066", Category::Correctness, Severity::Warning, MissingSemicolonBeforeBrace),
    ("C067", Category::Correctness, Severity::Warning, EmptyFunctionBody),
    ("C068", Category::Correctness, Severity::Warning, BareClosingBrace),
    ("C069", Category::Correctness, Severity::Warning, BackslashBeforeClosingBacktick),
    ("C070", Category::Correctness, Severity::Warning, PositionalParamAsOperator),
    ("C071", Category::Correctness, Severity::Warning, DoubleParenGrouping),
    ("C072", Category::Correctness, Severity::Warning, UnicodeQuoteInString),
    ("C124", Category::Correctness, Severity::Warning, UnreachableAfterExit),
    ("P001", Category::Performance, Severity::Warning, ExprArithmetic),
    ("P002", Category::Performance, Severity::Warning, GrepCountPipeline),
    ("P003", Category::Performance, Severity::Warning, SingleTestSubshell),
    ("P004", Category::Performance, Severity::Warning, SubshellTestGroup),
    ("X001", Category::Portability, Severity::Warning, DoubleBracketInSh),
    ("X002", Category::Portability, Severity::Warning, TestEqualityOperator),
    ("X003", Category::Portability, Severity::Warning, LocalVariableInSh),
    ("X004", Category::Portability, Severity::Warning, FunctionKeyword),
    ("X015", Category::Portability, Severity::Warning, LetCommand),
    ("X016", Category::Portability, Severity::Warning, DeclareCommand),
    ("X031", Category::Portability, Severity::Warning, SourceBuiltinInSh),
    ("X033", Category::Portability, Severity::Warning, IfElifBashTest),
    ("X034", Category::Portability, Severity::Warning, ExtendedGlobInTest),
    ("X040", Category::Portability, Severity::Warning, ArraySubscriptTest),
    ("X041", Category::Portability, Severity::Warning, ArraySubscriptCondition),
    ("X046", Category::Portability, Severity::Warning, ExtglobInTest),
    ("X052", Category::Portability, Severity::Warning, FunctionKeywordInSh),
    ("X058", Category::Portability, Severity::Warning, GreaterThanInDoubleBracket),
    ("X059", Category::Portability, Severity::Warning, RegexMatchInSh),
    ("X060", Category::Portability, Severity::Warning, VTestInSh),
    ("X061", Category::Portability, Severity::Warning, ATestInSh),
    ("X073", Category::Portability, Severity::Warning, OptionTestInSh),
    ("X074", Category::Portability, Severity::Warning, StickyBitTestInSh),
    ("X075", Category::Portability, Severity::Warning, OwnershipTestInSh),
    ("X080", Category::Portability, Severity::Warning, SourceInsideFunctionInSh),
    ("S001", Category::Style, Severity::Warning, UnquotedExpansion),
    ("S002", Category::Style, Severity::Warning, ReadWithoutRaw),
    ("S003", Category::Style, Severity::Warning, LoopFromCommandOutput),
    ("S004", Category::Style, Severity::Warning, UnquotedCommandSubstitution),
    ("S005", Category::Style, Severity::Warning, LegacyBackticks),
    ("S006", Category::Style, Severity::Warning, LegacyArithmeticExpansion),
    ("S007", Category::Style, Severity::Warning, PrintfFormatVariable),
    ("S008", Category::Style, Severity::Warning, UnquotedArrayExpansion),
    ("S009", Category::Style, Severity::Warning, EchoedCommandSubstitution),
    ("S010", Category::Style, Severity::Warning, ExportCommandSubstitution),
}

pub fn code_to_rule(code: &str) -> Option<Rule> {
    canonical_code_to_rule(code).or(match code {
        "SH-001" => Some(Rule::UnquotedExpansion),
        "SH-002" => Some(Rule::ReadWithoutRaw),
        "SH-003" => Some(Rule::UnusedAssignment),
        "SH-004" => Some(Rule::LoopFromCommandOutput),
        "SH-005" => Some(Rule::UnquotedCommandSubstitution),
        "SH-008" => Some(Rule::LocalVariableInSh),
        "SH-009" => Some(Rule::FunctionKeyword),
        "SH-020" => Some(Rule::LetCommand),
        "SH-021" => Some(Rule::DeclareCommand),
        "SH-080" => Some(Rule::SourceBuiltinInSh),
        "SH-226" => Some(Rule::FunctionKeywordInSh),
        "SH-304" => Some(Rule::SourceInsideFunctionInSh),
        "SH-034" => Some(Rule::LegacyBackticks),
        "SH-035" => Some(Rule::LegacyArithmeticExpansion),
        "SH-025" => Some(Rule::DynamicSourcePath),
        "SH-026" => Some(Rule::UntrackedSourceFile),
        "SH-027" => Some(Rule::UncheckedDirectoryChange),
        "SH-036" => Some(Rule::SingleQuotedLiteral),
        "SH-037" => Some(Rule::PrintfFormatVariable),
        "SH-038" => Some(Rule::UnquotedArrayExpansion),
        "SH-039" => Some(Rule::UndefinedVariable),
        "SH-040" => Some(Rule::EchoedCommandSubstitution),
        "SH-041" => Some(Rule::FindOutputToXargs),
        "SH-042" => Some(Rule::TrapStringExpansion),
        "SH-043" => Some(Rule::QuotedBashRegex),
        "SH-045" => Some(Rule::ChainedTestBranches),
        "SH-046" => Some(Rule::LineOrientedInput),
        "SH-048" => Some(Rule::LeadingGlobArgument),
        "SH-049" => Some(Rule::FindOutputLoop),
        "SH-050" => Some(Rule::ExportCommandSubstitution),
        "SH-052" => Some(Rule::LocalTopLevel),
        "SH-060" => Some(Rule::SudoRedirectionOrder),
        "SH-069" => Some(Rule::ConstantComparisonTest),
        "SH-070" => Some(Rule::LoopControlOutsideLoop),
        "SH-072" => Some(Rule::LiteralUnaryStringTest),
        "SH-073" => Some(Rule::TruthyLiteralTest),
        "SH-074" => Some(Rule::ConstantCaseSubject),
        "SH-075" => Some(Rule::EmptyTest),
        "SH-134" => Some(Rule::PipeToKill),
        "SH-086" => Some(Rule::PositionalTenBraces),
        "SH-106" => Some(Rule::MissingFi),
        "SH-109" => Some(Rule::BrokenTestEnd),
        "SH-110" => Some(Rule::BrokenTestParse),
        "SH-112" => Some(Rule::ElseIf),
        "SH-113" => Some(Rule::OpenDoubleQuote),
        "SH-115" => Some(Rule::LinebreakInTest),
        "SH-121" => Some(Rule::CStyleComment),
        "SH-123" => Some(Rule::CPrototypeFragment),
        "SH-129" => Some(Rule::BadRedirectionFdOrder),
        "SH-141" => Some(Rule::InvalidExitStatus),
        "SH-142" => Some(Rule::CasePatternVar),
        "SH-144" => Some(Rule::ArithmeticRedirectionTarget),
        "SH-148" => Some(Rule::BareSlashMarker),
        "SH-152" => Some(Rule::PatternWithVariable),
        "SH-155" => Some(Rule::StatusCaptureAfterBranchTest),
        "SH-159" => Some(Rule::SubstWithRedirect),
        "SH-160" => Some(Rule::SubstWithRedirectErr),
        "SH-165" => Some(Rule::RedirectToCommandName),
        "SH-166" => Some(Rule::NonAbsoluteShebang),
        "SH-167" => Some(Rule::TemplateBraceInCommand),
        "SH-169" => Some(Rule::NestedParameterExpansion),
        "SH-171" => Some(Rule::OverwrittenFunction),
        "SH-175" => Some(Rule::IfMissingThen),
        "SH-176" => Some(Rule::ElseWithoutThen),
        "SH-177" => Some(Rule::MissingSemicolonBeforeBrace),
        "SH-178" => Some(Rule::EmptyFunctionBody),
        "SH-179" => Some(Rule::BareClosingBrace),
        "SH-186" => Some(Rule::BackslashBeforeClosingBacktick),
        "SH-187" => Some(Rule::PositionalParamAsOperator),
        "SH-188" => Some(Rule::DoubleParenGrouping),
        "SH-189" => Some(Rule::UnicodeQuoteInString),
        "SH-293" => Some(Rule::UnreachableAfterExit),
        "SH-055" => Some(Rule::ExprArithmetic),
        "SH-064" => Some(Rule::GrepCountPipeline),
        "SH-137" => Some(Rule::SingleTestSubshell),
        "SH-164" => Some(Rule::SubshellTestGroup),
        "SH-006" => Some(Rule::DoubleBracketInSh),
        "SH-007" => Some(Rule::TestEqualityOperator),
        "SH-093" => Some(Rule::IfElifBashTest),
        "SH-101" => Some(Rule::ExtendedGlobInTest),
        "SH-126" => Some(Rule::ArraySubscriptTest),
        "SH-127" => Some(Rule::ArraySubscriptCondition),
        "SH-174" => Some(Rule::ExtglobInTest),
        "SH-265" => Some(Rule::GreaterThanInDoubleBracket),
        "SH-266" => Some(Rule::RegexMatchInSh),
        "SH-267" => Some(Rule::VTestInSh),
        "SH-268" => Some(Rule::ATestInSh),
        "SH-280" => Some(Rule::OptionTestInSh),
        "SH-281" => Some(Rule::StickyBitTestInSh),
        "SH-282" => Some(Rule::OwnershipTestInSh),
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
        assert_eq!(code_to_rule("SH-002"), Some(Rule::ReadWithoutRaw));
        assert_eq!(code_to_rule("SH-003"), Some(Rule::UnusedAssignment));
        assert_eq!(code_to_rule("SH-004"), Some(Rule::LoopFromCommandOutput));
        assert_eq!(
            code_to_rule("SH-005"),
            Some(Rule::UnquotedCommandSubstitution)
        );
        assert_eq!(code_to_rule("SH-034"), Some(Rule::LegacyBackticks));
        assert_eq!(
            code_to_rule("SH-035"),
            Some(Rule::LegacyArithmeticExpansion)
        );
        assert_eq!(code_to_rule("SH-055"), Some(Rule::ExprArithmetic));
        assert_eq!(code_to_rule("SH-064"), Some(Rule::GrepCountPipeline));
        assert_eq!(code_to_rule("SH-137"), Some(Rule::SingleTestSubshell));
        assert_eq!(code_to_rule("SH-164"), Some(Rule::SubshellTestGroup));
        assert_eq!(code_to_rule("SH-025"), Some(Rule::DynamicSourcePath));
        assert_eq!(code_to_rule("SH-026"), Some(Rule::UntrackedSourceFile));
        assert_eq!(code_to_rule("SH-036"), Some(Rule::SingleQuotedLiteral));
        assert_eq!(code_to_rule("SH-037"), Some(Rule::PrintfFormatVariable));
        assert_eq!(code_to_rule("SH-038"), Some(Rule::UnquotedArrayExpansion));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
        assert_eq!(
            code_to_rule("SH-040"),
            Some(Rule::EchoedCommandSubstitution)
        );
        assert_eq!(code_to_rule("SH-041"), Some(Rule::FindOutputToXargs));
        assert_eq!(code_to_rule("SH-042"), Some(Rule::TrapStringExpansion));
        assert_eq!(code_to_rule("SH-043"), Some(Rule::QuotedBashRegex));
        assert_eq!(code_to_rule("SH-045"), Some(Rule::ChainedTestBranches));
        assert_eq!(code_to_rule("SH-046"), Some(Rule::LineOrientedInput));
        assert_eq!(code_to_rule("SH-049"), Some(Rule::FindOutputLoop));
        assert_eq!(
            code_to_rule("SH-050"),
            Some(Rule::ExportCommandSubstitution)
        );
        assert_eq!(code_to_rule("SH-052"), Some(Rule::LocalTopLevel));
        assert_eq!(code_to_rule("SH-060"), Some(Rule::SudoRedirectionOrder));
        assert_eq!(code_to_rule("SH-069"), Some(Rule::ConstantComparisonTest));
        assert_eq!(code_to_rule("SH-070"), Some(Rule::LoopControlOutsideLoop));
        assert_eq!(code_to_rule("SH-072"), Some(Rule::LiteralUnaryStringTest));
        assert_eq!(code_to_rule("SH-073"), Some(Rule::TruthyLiteralTest));
        assert_eq!(code_to_rule("SH-074"), Some(Rule::ConstantCaseSubject));
        assert_eq!(code_to_rule("SH-075"), Some(Rule::EmptyTest));
        assert_eq!(code_to_rule("SH-134"), Some(Rule::PipeToKill));
        assert_eq!(code_to_rule("SH-086"), Some(Rule::PositionalTenBraces));
        assert_eq!(code_to_rule("C035"), Some(Rule::MissingFi));
        assert_eq!(code_to_rule("SH-106"), Some(Rule::MissingFi));
        assert_eq!(code_to_rule("C036"), Some(Rule::BrokenTestEnd));
        assert_eq!(code_to_rule("SH-109"), Some(Rule::BrokenTestEnd));
        assert_eq!(code_to_rule("C037"), Some(Rule::BrokenTestParse));
        assert_eq!(code_to_rule("SH-110"), Some(Rule::BrokenTestParse));
        assert_eq!(code_to_rule("C038"), Some(Rule::ElseIf));
        assert_eq!(code_to_rule("SH-112"), Some(Rule::ElseIf));
        assert_eq!(code_to_rule("C039"), Some(Rule::OpenDoubleQuote));
        assert_eq!(code_to_rule("SH-113"), Some(Rule::OpenDoubleQuote));
        assert_eq!(code_to_rule("C040"), Some(Rule::LinebreakInTest));
        assert_eq!(code_to_rule("SH-115"), Some(Rule::LinebreakInTest));
        assert_eq!(code_to_rule("C041"), Some(Rule::CStyleComment));
        assert_eq!(code_to_rule("SH-121"), Some(Rule::CStyleComment));
        assert_eq!(code_to_rule("C042"), Some(Rule::CPrototypeFragment));
        assert_eq!(code_to_rule("SH-123"), Some(Rule::CPrototypeFragment));
        assert_eq!(code_to_rule("C043"), Some(Rule::BadRedirectionFdOrder));
        assert_eq!(code_to_rule("SH-129"), Some(Rule::BadRedirectionFdOrder));
        assert_eq!(code_to_rule("SH-141"), Some(Rule::InvalidExitStatus));
        assert_eq!(code_to_rule("SH-142"), Some(Rule::CasePatternVar));
        assert_eq!(
            code_to_rule("SH-144"),
            Some(Rule::ArithmeticRedirectionTarget)
        );
        assert_eq!(code_to_rule("SH-148"), Some(Rule::BareSlashMarker));
        assert_eq!(code_to_rule("SH-152"), Some(Rule::PatternWithVariable));
        assert_eq!(
            code_to_rule("SH-155"),
            Some(Rule::StatusCaptureAfterBranchTest)
        );
        assert_eq!(code_to_rule("SH-159"), Some(Rule::SubstWithRedirect));
        assert_eq!(code_to_rule("SH-160"), Some(Rule::SubstWithRedirectErr));
        assert_eq!(code_to_rule("SH-165"), Some(Rule::RedirectToCommandName));
        assert_eq!(code_to_rule("SH-166"), Some(Rule::NonAbsoluteShebang));
        assert_eq!(code_to_rule("SH-167"), Some(Rule::TemplateBraceInCommand));
        assert_eq!(code_to_rule("SH-169"), Some(Rule::NestedParameterExpansion));
        assert_eq!(code_to_rule("SH-171"), Some(Rule::OverwrittenFunction));
        assert_eq!(code_to_rule("SH-175"), Some(Rule::IfMissingThen));
        assert_eq!(code_to_rule("SH-176"), Some(Rule::ElseWithoutThen));
        assert_eq!(
            code_to_rule("SH-177"),
            Some(Rule::MissingSemicolonBeforeBrace)
        );
        assert_eq!(code_to_rule("SH-178"), Some(Rule::EmptyFunctionBody));
        assert_eq!(code_to_rule("SH-179"), Some(Rule::BareClosingBrace));
        assert_eq!(
            code_to_rule("SH-186"),
            Some(Rule::BackslashBeforeClosingBacktick)
        );
        assert_eq!(
            code_to_rule("SH-187"),
            Some(Rule::PositionalParamAsOperator)
        );
        assert_eq!(code_to_rule("SH-188"), Some(Rule::DoubleParenGrouping));
        assert_eq!(code_to_rule("SH-189"), Some(Rule::UnicodeQuoteInString));
        assert_eq!(code_to_rule("C006"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("C124"), Some(Rule::UnreachableAfterExit));
        assert_eq!(code_to_rule("SH-293"), Some(Rule::UnreachableAfterExit));
    }
}
