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
    (
        "C076",
        Category::Correctness,
        Severity::Warning,
        CommentedContinuationLine
    ),
    ("C077", Category::Correctness, Severity::Warning, SubshellInArithmetic),
    ("C078", Category::Correctness, Severity::Warning, UnquotedGlobsInFind),
    ("C080", Category::Correctness, Severity::Warning, GlobInGrepPattern),
    ("C081", Category::Correctness, Severity::Warning, GlobInStringComparison),
    ("C083", Category::Correctness, Severity::Warning, GlobInFindSubstitution),
    ("C084", Category::Correctness, Severity::Warning, UnquotedGrepRegex),
    (
        "C079",
        Category::Correctness,
        Severity::Warning,
        ShortCircuitFallthrough
    ),
    (
        "C082",
        Category::Correctness,
        Severity::Warning,
        EscapedNegationInTest
    ),
    (
        "C086",
        Category::Correctness,
        Severity::Warning,
        GreaterThanInTest
    ),
    (
        "C087",
        Category::Correctness,
        Severity::Warning,
        StringComparisonForVersion
    ),
    (
        "C095",
        Category::Correctness,
        Severity::Warning,
        AssignmentLooksLikeComparison
    ),
    ("C096", Category::Correctness, Severity::Warning, UnquotedPipeInEcho),
    (
        "C097",
        Category::Correctness,
        Severity::Error,
        FunctionCalledWithoutArgs
    ),
    (
        "C098",
        Category::Correctness,
        Severity::Warning,
        SetFlagsWithoutDashes
    ),
    (
        "C099",
        Category::Correctness,
        Severity::Warning,
        QuotedArraySlice
    ),
    (
        "C100",
        Category::Correctness,
        Severity::Warning,
        QuotedBashSource
    ),
    (
        "C101",
        Category::Correctness,
        Severity::Warning,
        IfsSetToLiteralBackslashN
    ),
    (
        "C103",
        Category::Correctness,
        Severity::Warning,
        FindOrWithoutGrouping
    ),
    (
        "C104",
        Category::Correctness,
        Severity::Warning,
        NonShellSyntaxInScript
    ),
    (
        "C105",
        Category::Correctness,
        Severity::Warning,
        ExportWithPositionalParams
    ),
    (
        "C106",
        Category::Correctness,
        Severity::Warning,
        AppendToArrayAsString
    ),
    (
        "C108",
        Category::Correctness,
        Severity::Warning,
        UnsetAssociativeArrayElement
    ),
    (
        "C109",
        Category::Correctness,
        Severity::Warning,
        MapfileProcessSubstitution
    ),
    (
        "C111",
        Category::Correctness,
        Severity::Warning,
        AtSignInStringCompare
    ),
    (
        "C112",
        Category::Correctness,
        Severity::Warning,
        ArraySliceInComparison
    ),
    (
        "C114",
        Category::Correctness,
        Severity::Warning,
        GlobWithExpansionInLoop
    ),
    (
        "C115",
        Category::Correctness,
        Severity::Warning,
        DefaultElseInShortCircuit
    ),
    (
        "C116",
        Category::Correctness,
        Severity::Warning,
        AssignmentToNumericVariable
    ),
    (
        "C117",
        Category::Correctness,
        Severity::Warning,
        PlusPrefixInAssignment
    ),
    (
        "C123",
        Category::Correctness,
        Severity::Error,
        FunctionReferencesUnsetParam
    ),
    ("C124", Category::Correctness, Severity::Warning, UnreachableAfterExit),
    ("C127", Category::Correctness, Severity::Warning, UnusedHeredoc),
    (
        "C125",
        Category::Correctness,
        Severity::Warning,
        UncheckedDirectoryChangeInFunction
    ),
    (
        "C126",
        Category::Correctness,
        Severity::Error,
        ContinueOutsideLoopInFunction
    ),
    (
        "C130",
        Category::Correctness,
        Severity::Warning,
        AppendWithEscapedQuotes
    ),
    (
        "C131",
        Category::Correctness,
        Severity::Warning,
        VariableAsCommandName
    ),
    (
        "C132",
        Category::Correctness,
        Severity::Warning,
        MisspelledOptionName
    ),
    (
        "C133",
        Category::Correctness,
        Severity::Warning,
        ArrayToStringConversion
    ),
    (
        "C136",
        Category::Correctness,
        Severity::Warning,
        LocalCrossReference
    ),
    (
        "C137",
        Category::Correctness,
        Severity::Warning,
        UnicodeSingleQuoteInSingleQuotes
    ),
    ("C138", Category::Correctness, Severity::Warning, HeredocMissingEnd),
    ("C139", Category::Correctness, Severity::Warning, SpacedAssignment),
    ("C140", Category::Correctness, Severity::Warning, BadVarName),
    ("C141", Category::Correctness, Severity::Error, LoopWithoutEnd),
    (
        "C142",
        Category::Correctness,
        Severity::Error,
        MissingDoneInForLoop
    ),
    ("C143", Category::Correctness, Severity::Error, DanglingElse),
    (
        "C144",
        Category::Correctness,
        Severity::Warning,
        HeredocCloserNotAlone
    ),
    ("C145", Category::Correctness, Severity::Warning, MisquotedHeredocClose),
    ("C146", Category::Correctness, Severity::Error, UntilMissingDo),
    ("C147", Category::Correctness, Severity::Warning, KeywordFunctionName),
    ("C148", Category::Correctness, Severity::Warning, BrokenAssocKey),
    ("C151", Category::Correctness, Severity::Warning, CommaArrayElements),
    ("C157", Category::Correctness, Severity::Error, IfBracketGlued),
    ("P001", Category::Performance, Severity::Warning, ExprArithmetic),
    ("P002", Category::Performance, Severity::Warning, GrepCountPipeline),
    ("P003", Category::Performance, Severity::Warning, SingleTestSubshell),
    ("P004", Category::Performance, Severity::Warning, SubshellTestGroup),
    ("X001", Category::Portability, Severity::Warning, DoubleBracketInSh),
    ("X002", Category::Portability, Severity::Warning, TestEqualityOperator),
    ("X003", Category::Portability, Severity::Warning, LocalVariableInSh),
    ("X004", Category::Portability, Severity::Warning, FunctionKeyword),
    ("X005", Category::Portability, Severity::Warning, BashCaseFallthrough),
    ("X006", Category::Portability, Severity::Warning, ProcessSubstitution),
    ("X007", Category::Portability, Severity::Warning, AnsiCQuoting),
    ("X010", Category::Portability, Severity::Warning, BraceExpansion),
    ("X011", Category::Portability, Severity::Warning, HereString),
    ("X008", Category::Portability, Severity::Warning, StandaloneArithmetic),
    ("X009", Category::Portability, Severity::Warning, SelectLoop),
    ("X014", Category::Portability, Severity::Warning, Coproc),
    ("X012", Category::Portability, Severity::Warning, AmpersandRedirection),
    ("X013", Category::Portability, Severity::Warning, ArrayAssignment),
    ("X015", Category::Portability, Severity::Warning, LetCommand),
    ("X016", Category::Portability, Severity::Warning, DeclareCommand),
    ("X017", Category::Portability, Severity::Warning, TrapErr),
    ("X018", Category::Portability, Severity::Warning, IndirectExpansion),
    ("X019", Category::Portability, Severity::Warning, ArrayReference),
    ("X020", Category::Portability, Severity::Warning, BraceFdRedirection),
    ("X021", Category::Portability, Severity::Warning, PipefailOption),
    ("X022", Category::Portability, Severity::Warning, WaitOption),
    ("X023", Category::Portability, Severity::Warning, SubstringExpansion),
    ("X024", Category::Portability, Severity::Warning, CaseModificationExpansion),
    ("X025", Category::Portability, Severity::Warning, ReplacementExpansion),
    ("X026", Category::Portability, Severity::Warning, BashFileSlurp),
    ("X031", Category::Portability, Severity::Warning, SourceBuiltinInSh),
    ("X032", Category::Portability, Severity::Warning, PrintfQFormatInSh),
    ("X033", Category::Portability, Severity::Warning, IfElifBashTest),
    ("X034", Category::Portability, Severity::Warning, ExtendedGlobInTest),
    ("X037", Category::Portability, Severity::Warning, ExtglobCase),
    ("X048", Category::Portability, Severity::Warning, ExtglobInCasePattern),
    ("X054", Category::Portability, Severity::Warning, ExtglobInSh),
    ("X065", Category::Portability, Severity::Warning, CaretNegationInBracket),
    ("X036", Category::Portability, Severity::Warning, ZshRedirPipe),
    ("X038", Category::Portability, Severity::Warning, ZshBraceIf),
    ("X039", Category::Portability, Severity::Warning, ZshAlwaysBlock),
    ("X040", Category::Portability, Severity::Warning, ArraySubscriptTest),
    ("X041", Category::Portability, Severity::Warning, ArraySubscriptCondition),
    ("X042", Category::Portability, Severity::Warning, SourcedWithArgs),
    ("X043", Category::Portability, Severity::Warning, ZshFlagExpansion),
    ("X044", Category::Portability, Severity::Warning, NestedZshSubstitution),
    ("X045", Category::Portability, Severity::Warning, PlusEqualsAppend),
    ("X051", Category::Portability, Severity::Warning, ZshNestedExpansion),
    ("X047", Category::Portability, Severity::Warning, MultiVarForLoop),
    ("X049", Category::Portability, Severity::Warning, ZshPromptBracket),
    ("X050", Category::Portability, Severity::Warning, CshSyntaxInSh),
    ("X053", Category::Portability, Severity::Warning, ZshAssignmentToZero),
    ("X056", Category::Portability, Severity::Warning, CStyleForInSh),
    ("X055", Category::Portability, Severity::Warning, DollarStringInSh),
    ("X057", Category::Portability, Severity::Warning, LegacyArithmeticInSh),
    ("X062", Category::Portability, Severity::Warning, CStyleForArithmeticInSh),
    ("X064", Category::Portability, Severity::Warning, PlusEqualsInSh),
    ("X071", Category::Portability, Severity::Warning, ArrayKeysInSh),
    ("X081", Category::Portability, Severity::Warning, StarGlobRemovalInSh),
    ("X076", Category::Portability, Severity::Warning, ZshParameterFlag),
    ("X078", Category::Portability, Severity::Warning, ZshArraySubscriptInCase),
    ("X079", Category::Portability, Severity::Warning, ZshParameterIndexFlag),
    ("X046", Category::Portability, Severity::Warning, ExtglobInTest),
    ("X052", Category::Portability, Severity::Warning, FunctionKeywordInSh),
    ("X058", Category::Portability, Severity::Warning, GreaterThanInDoubleBracket),
    ("X059", Category::Portability, Severity::Warning, RegexMatchInSh),
    ("X060", Category::Portability, Severity::Warning, VTestInSh),
    ("X061", Category::Portability, Severity::Warning, ATestInSh),
    ("X063", Category::Portability, Severity::Warning, AmpersandRedirectInSh),
    ("X066", Category::Portability, Severity::Warning, PipeStderrInSh),
    ("X068", Category::Portability, Severity::Warning, ErrexitTrapInSh),
    ("X069", Category::Portability, Severity::Warning, SignalNameInTrap),
    ("X070", Category::Portability, Severity::Warning, BasePrefixInArithmetic),
    ("X073", Category::Portability, Severity::Warning, OptionTestInSh),
    ("X074", Category::Portability, Severity::Warning, StickyBitTestInSh),
    ("X075", Category::Portability, Severity::Warning, OwnershipTestInSh),
    ("X080", Category::Portability, Severity::Warning, SourceInsideFunctionInSh),
    ("K001", Category::Security, Severity::Warning, RmGlobOnVariablePath),
    ("K002", Category::Security, Severity::Warning, SshLocalExpansion),
    ("K003", Category::Security, Severity::Warning, EvalOnArray),
    ("K004", Category::Security, Severity::Warning, FindExecDirWithShell),
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
    ("S012", Category::Style, Severity::Warning, PsGrepPipeline),
    ("S013", Category::Style, Severity::Warning, LsGrepPipeline),
    ("S014", Category::Style, Severity::Warning, UnquotedDollarStar),
    ("S015", Category::Style, Severity::Warning, QuotedDollarStarLoop),
    ("S017", Category::Style, Severity::Warning, UnquotedArraySplit),
    ("S018", Category::Style, Severity::Warning, CommandOutputArraySplit),
    ("S021", Category::Style, Severity::Warning, PositionalArgsInString),
    ("S020", Category::Style, Severity::Warning, SingleIterationLoop),
    ("S032", Category::Style, Severity::Warning, ConditionalAssignmentShortcut),
    ("S036", Category::Style, Severity::Warning, BareRead),
    ("S037", Category::Style, Severity::Warning, RedundantSpacesInEcho),
    ("S044", Category::Style, Severity::Warning, UnquotedVariableInSed),
    ("S050", Category::Style, Severity::Hint, UnquotedWordBetweenQuotes),
    ("S051", Category::Style, Severity::Warning, UnquotedTrClass),
    ("S052", Category::Style, Severity::Warning, UnquotedVariableInTest),
    ("S054", Category::Style, Severity::Warning, SuWithoutFlag),
    (
        "S055",
        Category::Style,
        Severity::Warning,
        GlobAssignedToVariable
    ),
    ("S056", Category::Style, Severity::Warning, CommandSubstitutionInAlias),
    ("S057", Category::Style, Severity::Warning, FunctionInAlias),
    ("S058", Category::Style, Severity::Warning, UnquotedPathInMkdir),
    ("S059", Category::Style, Severity::Warning, DeprecatedTempfileCommand),
    ("S060", Category::Style, Severity::Warning, EgrepDeprecated),
    ("S062", Category::Style, Severity::Warning, DefaultValueInColonAssign),
    ("S067", Category::Style, Severity::Warning, BacktickOutputToCommand),
    ("S070", Category::Style, Severity::Warning, DoubleQuoteNesting),
    ("S071", Category::Style, Severity::Warning, EnvPrefixQuoting),
    ("S076", Category::Style, Severity::Warning, MixedQuoteWord),
    ("S049", Category::Style, Severity::Warning, UnquotedTrRange),
    ("S046", Category::Style, Severity::Warning, LsPipedToXargs),
    ("S047", Category::Style, Severity::Warning, LsInSubstitution),
    (
        "S016",
        Category::Style,
        Severity::Warning,
        EchoInsideCommandSubstitution
    ),
    ("S019", Category::Style, Severity::Warning, GrepOutputInTest),
    ("S022", Category::Style, Severity::Hint, AvoidLetBuiltin),
    ("S033", Category::Style, Severity::Warning, EchoHereDoc),
    ("S034", Category::Style, Severity::Warning, ArrayIndexArithmetic),
    ("S035", Category::Style, Severity::Warning, ArithmeticScoreLine),
    ("S045", Category::Style, Severity::Warning, DollarInArithmetic),
    (
        "S048",
        Category::Style,
        Severity::Warning,
        DollarInArithmeticContext
    ),
    ("S023", Category::Style, Severity::Warning, EscapedUnderscore),
    ("S024", Category::Style, Severity::Warning, SingleQuoteBackslash),
    ("S025", Category::Style, Severity::Warning, LiteralBackslash),
    (
        "S026",
        Category::Style,
        Severity::Warning,
        NeedlessBackslashUnderscore
    ),
    ("S027", Category::Style, Severity::Warning, EscapedUnderscoreLiteral),
    ("S028", Category::Style, Severity::Warning, SuspectClosingQuote),
    ("S029", Category::Style, Severity::Warning, LiteralBraces),
    ("S030", Category::Style, Severity::Warning, HeredocEndSpace),
    ("S031", Category::Style, Severity::Warning, TrailingDirective),
    (
        "S039",
        Category::Style,
        Severity::Warning,
        LiteralBackslashInSingleQuotes
    ),
    ("S040", Category::Style, Severity::Warning, BackslashBeforeCommand),
    ("S042", Category::Style, Severity::Warning, IfsEqualsAmbiguity),
    ("S072", Category::Style, Severity::Warning, LinebreakBeforeAnd),
    ("S073", Category::Style, Severity::Warning, SpacedTabstripClose),
    ("S074", Category::Style, Severity::Warning, AmpersandSemicolon),
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
        "SH-010" => Some(Rule::BashCaseFallthrough),
        "SH-011" => Some(Rule::ProcessSubstitution),
        "SH-012" => Some(Rule::AnsiCQuoting),
        "SH-015" => Some(Rule::BraceExpansion),
        "SH-016" => Some(Rule::HereString),
        "SH-018" => Some(Rule::ArrayAssignment),
        "SH-023" => Some(Rule::IndirectExpansion),
        "SH-024" => Some(Rule::ArrayReference),
        "SH-031" => Some(Rule::SubstringExpansion),
        "SH-032" => Some(Rule::CaseModificationExpansion),
        "SH-033" => Some(Rule::ReplacementExpansion),
        "SH-053" => Some(Rule::BashFileSlurp),
        "SH-013" => Some(Rule::StandaloneArithmetic),
        "SH-014" => Some(Rule::SelectLoop),
        "SH-019" => Some(Rule::Coproc),
        "SH-079" => Some(Rule::AvoidLetBuiltin),
        "SH-020" => Some(Rule::LetCommand),
        "SH-021" => Some(Rule::DeclareCommand),
        "SH-029" => Some(Rule::PipefailOption),
        "SH-030" => Some(Rule::WaitOption),
        "SH-022" => Some(Rule::TrapErr),
        "SH-080" => Some(Rule::SourceBuiltinInSh),
        "SH-081" => Some(Rule::PrintfQFormatInSh),
        "SH-226" => Some(Rule::FunctionKeywordInSh),
        "SH-234" => Some(Rule::IfsSetToLiteralBackslashN),
        "SH-304" => Some(Rule::SourceInsideFunctionInSh),
        "SH-275" => Some(Rule::ErrexitTrapInSh),
        "SH-276" => Some(Rule::SignalNameInTrap),
        "SH-277" => Some(Rule::BasePrefixInArithmetic),
        "SH-034" => Some(Rule::LegacyBackticks),
        "SH-035" => Some(Rule::LegacyArithmeticExpansion),
        "SH-157" => Some(Rule::ArrayIndexArithmetic),
        "SH-161" => Some(Rule::ArithmeticScoreLine),
        "SH-197" => Some(Rule::DollarInArithmetic),
        "SH-198" => Some(Rule::LsPipedToXargs),
        "SH-202" => Some(Rule::DollarInArithmeticContext),
        "SH-203" => Some(Rule::UnquotedTrRange),
        "SH-082" => Some(Rule::EscapedUnderscore),
        "SH-095" => Some(Rule::EscapedUnderscoreLiteral),
        "SH-208" => Some(Rule::UnquotedTrClass),
        "SH-212" => Some(Rule::UnquotedVariableInTest),
        "SH-087" => Some(Rule::SingleQuoteBackslash),
        "SH-172" => Some(Rule::LiteralBackslashInSingleQuotes),
        "SH-173" => Some(Rule::BackslashBeforeCommand),
        "SH-185" => Some(Rule::IfsEqualsAmbiguity),
        "SH-088" => Some(Rule::LiteralBackslash),
        "SH-092" => Some(Rule::NeedlessBackslashUnderscore),
        "SH-258" => Some(Rule::AssignmentToNumericVariable),
        "SH-259" => Some(Rule::PlusPrefixInAssignment),
        "SH-307" => Some(Rule::AppendWithEscapedQuotes),
        "SH-025" => Some(Rule::DynamicSourcePath),
        "SH-026" => Some(Rule::UntrackedSourceFile),
        "SH-027" => Some(Rule::UncheckedDirectoryChange),
        "SH-228" => Some(Rule::FunctionCalledWithoutArgs),
        "SH-292" => Some(Rule::FunctionReferencesUnsetParam),
        "SH-295" => Some(Rule::UncheckedDirectoryChangeInFunction),
        "SH-296" => Some(Rule::ContinueOutsideLoopInFunction),
        "SH-308" => Some(Rule::VariableAsCommandName),
        "SH-257" => Some(Rule::DefaultElseInShortCircuit),
        "SH-311" => Some(Rule::ArrayToStringConversion),
        "SH-336" => Some(Rule::KeywordFunctionName),
        "SH-337" => Some(Rule::BrokenAssocKey),
        "SH-340" => Some(Rule::CommaArrayElements),
        "SH-036" => Some(Rule::SingleQuotedLiteral),
        "SH-037" => Some(Rule::PrintfFormatVariable),
        "SH-038" => Some(Rule::UnquotedArrayExpansion),
        "SH-039" => Some(Rule::UndefinedVariable),
        "SH-040" => Some(Rule::EchoedCommandSubstitution),
        "SH-168" => Some(Rule::RedundantSpacesInEcho),
        "SH-196" => Some(Rule::UnquotedVariableInSed),
        "SH-205" => Some(Rule::UnquotedWordBetweenQuotes),
        "SH-066" => Some(Rule::EchoInsideCommandSubstitution),
        "SH-199" => Some(Rule::LsInSubstitution),
        "SH-163" => Some(Rule::BareRead),
        "SH-245" => Some(Rule::DeprecatedTempfileCommand),
        "SH-247" => Some(Rule::EgrepDeprecated),
        "SH-306" => Some(Rule::DoubleQuoteNesting),
        "SH-309" => Some(Rule::EnvPrefixQuoting),
        "SH-350" => Some(Rule::MixedQuoteWord),
        "SH-071" => Some(Rule::GrepOutputInTest),
        "SH-076" => Some(Rule::SingleIterationLoop),
        "SH-128" => Some(Rule::ConditionalAssignmentShortcut),
        "SH-041" => Some(Rule::FindOutputToXargs),
        "SH-042" => Some(Rule::TrapStringExpansion),
        "SH-043" => Some(Rule::QuotedBashRegex),
        "SH-044" => Some(Rule::RmGlobOnVariablePath),
        "SH-047" => Some(Rule::SshLocalExpansion),
        "SH-151" => Some(Rule::EvalOnArray),
        "SH-045" => Some(Rule::ChainedTestBranches),
        "SH-201" => Some(Rule::ShortCircuitFallthrough),
        "SH-046" => Some(Rule::LineOrientedInput),
        "SH-048" => Some(Rule::LeadingGlobArgument),
        "SH-049" => Some(Rule::FindOutputLoop),
        "C113" => Some(Rule::FindOutputLoop),
        "SH-050" => Some(Rule::ExportCommandSubstitution),
        "SH-135" => Some(Rule::EchoHereDoc),
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
        "SH-111" => Some(Rule::ExtglobCase),
        "SH-182" => Some(Rule::ExtglobInCasePattern),
        "SH-261" => Some(Rule::ExtglobInSh),
        "SH-272" => Some(Rule::CaretNegationInBracket),
        "SH-207" => Some(Rule::EscapedNegationInTest),
        "SH-213" => Some(Rule::GreaterThanInTest),
        "SH-214" => Some(Rule::StringComparisonForVersion),
        "SH-112" => Some(Rule::ElseIf),
        "SH-113" => Some(Rule::OpenDoubleQuote),
        "SH-114" => Some(Rule::SuspectClosingQuote),
        "SH-115" => Some(Rule::LinebreakInTest),
        "SH-116" => Some(Rule::LiteralBraces),
        "SH-119" => Some(Rule::HeredocEndSpace),
        "SH-120" => Some(Rule::TrailingDirective),
        "SH-329" => Some(Rule::LinebreakBeforeAnd),
        "SH-330" => Some(Rule::SpacedTabstripClose),
        "SH-335" => Some(Rule::AmpersandSemicolon),
        "SH-121" => Some(Rule::CStyleComment),
        "SH-123" => Some(Rule::CPrototypeFragment),
        "SH-129" => Some(Rule::BadRedirectionFdOrder),
        "SH-141" => Some(Rule::InvalidExitStatus),
        "SH-142" => Some(Rule::CasePatternVar),
        "SH-144" => Some(Rule::ArithmeticRedirectionTarget),
        "SH-148" => Some(Rule::BareSlashMarker),
        "SH-152" => Some(Rule::PatternWithVariable),
        "SH-155" => Some(Rule::StatusCaptureAfterBranchTest),
        "SH-017" => Some(Rule::AmpersandRedirection),
        "SH-028" => Some(Rule::BraceFdRedirection),
        "SH-270" => Some(Rule::AmpersandRedirectInSh),
        "SH-273" => Some(Rule::PipeStderrInSh),
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
        "SH-224" => Some(Rule::AssignmentLooksLikeComparison),
        "SH-225" => Some(Rule::UnquotedPipeInEcho),
        "SH-227" => Some(Rule::SuWithoutFlag),
        "SH-231" => Some(Rule::GlobAssignedToVariable),
        "SH-194" => Some(Rule::CommentedContinuationLine),
        "SH-195" => Some(Rule::SubshellInArithmetic),
        "SH-200" => Some(Rule::UnquotedGlobsInFind),
        "SH-204" => Some(Rule::GlobInGrepPattern),
        "SH-206" => Some(Rule::GlobInStringComparison),
        "SH-209" => Some(Rule::GlobInFindSubstitution),
        "SH-210" => Some(Rule::UnquotedGrepRegex),
        "SH-229" => Some(Rule::SetFlagsWithoutDashes),
        "SH-230" => Some(Rule::QuotedArraySlice),
        "SH-232" => Some(Rule::QuotedBashSource),
        "SH-233" => Some(Rule::CommandSubstitutionInAlias),
        "SH-235" => Some(Rule::FunctionInAlias),
        "SH-237" => Some(Rule::FindOrWithoutGrouping),
        "SH-238" => Some(Rule::NonShellSyntaxInScript),
        "SH-239" => Some(Rule::ExportWithPositionalParams),
        "SH-240" => Some(Rule::UnquotedPathInMkdir),
        "SH-249" => Some(Rule::AtSignInStringCompare),
        "SH-250" => Some(Rule::ArraySliceInComparison),
        "SH-251" => Some(Rule::DefaultValueInColonAssign),
        "SH-241" => Some(Rule::AppendToArrayAsString),
        "SH-243" => Some(Rule::UnsetAssociativeArrayElement),
        "SH-244" => Some(Rule::MapfileProcessSubstitution),
        "SH-254" => Some(Rule::GlobWithExpansionInLoop),
        "SH-253" => Some(Rule::FindOutputLoop),
        "SH-293" => Some(Rule::UnreachableAfterExit),
        "SH-298" => Some(Rule::UnusedHeredoc),
        "SH-318" => Some(Rule::HeredocMissingEnd),
        "SH-310" => Some(Rule::MisspelledOptionName),
        "SH-055" => Some(Rule::ExprArithmetic),
        "SH-056" => Some(Rule::PsGrepPipeline),
        "SH-057" => Some(Rule::LsGrepPipeline),
        "SH-062" => Some(Rule::UnquotedDollarStar),
        "SH-063" => Some(Rule::QuotedDollarStarLoop),
        "SH-067" => Some(Rule::UnquotedArraySplit),
        "SH-068" => Some(Rule::CommandOutputArraySplit),
        "SH-294" => Some(Rule::BacktickOutputToCommand),
        "SH-077" => Some(Rule::PositionalArgsInString),
        "SH-064" => Some(Rule::GrepCountPipeline),
        "SH-137" => Some(Rule::SingleTestSubshell),
        "SH-164" => Some(Rule::SubshellTestGroup),
        "SH-006" => Some(Rule::DoubleBracketInSh),
        "SH-007" => Some(Rule::TestEqualityOperator),
        "SH-093" => Some(Rule::IfElifBashTest),
        "SH-101" => Some(Rule::ExtendedGlobInTest),
        "SH-108" => Some(Rule::ZshRedirPipe),
        "SH-124" => Some(Rule::ZshBraceIf),
        "SH-125" => Some(Rule::ZshAlwaysBlock),
        "SH-140" => Some(Rule::SourcedWithArgs),
        "SH-153" => Some(Rule::ZshFlagExpansion),
        "SH-154" => Some(Rule::NestedZshSubstitution),
        "SH-158" => Some(Rule::PlusEqualsAppend),
        "SH-180" => Some(Rule::MultiVarForLoop),
        "SH-183" => Some(Rule::ZshPromptBracket),
        "SH-184" => Some(Rule::CshSyntaxInSh),
        "SH-218" => Some(Rule::ZshNestedExpansion),
        "SH-260" => Some(Rule::ZshAssignmentToZero),
        "SH-262" => Some(Rule::DollarStringInSh),
        "SH-263" => Some(Rule::CStyleForInSh),
        "SH-264" => Some(Rule::LegacyArithmeticInSh),
        "SH-269" => Some(Rule::CStyleForArithmeticInSh),
        "SH-271" => Some(Rule::PlusEqualsInSh),
        "SH-278" => Some(Rule::ArrayKeysInSh),
        "SH-305" => Some(Rule::StarGlobRemovalInSh),
        "SH-286" => Some(Rule::ZshParameterFlag),
        "SH-299" => Some(Rule::ZshArraySubscriptInCase),
        "SH-303" => Some(Rule::ZshParameterIndexFlag),
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
        "SH-283" => Some(Rule::FindExecDirWithShell),
        "SH-314" => Some(Rule::LocalCrossReference),
        "SH-315" => Some(Rule::UnicodeSingleQuoteInSingleQuotes),
        "SH-319" => Some(Rule::SpacedAssignment),
        "SH-320" => Some(Rule::BadVarName),
        "SH-321" => Some(Rule::LoopWithoutEnd),
        "SH-322" => Some(Rule::MissingDoneInForLoop),
        "SH-327" => Some(Rule::DanglingElse),
        "SH-332" => Some(Rule::HeredocCloserNotAlone),
        "SH-333" => Some(Rule::MisquotedHeredocClose),
        "SH-334" => Some(Rule::UntilMissingDo),
        "SH-353" => Some(Rule::IfBracketGlued),
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
        assert_eq!(code_to_rule("SH-010"), Some(Rule::BashCaseFallthrough));
        assert_eq!(code_to_rule("SH-013"), Some(Rule::StandaloneArithmetic));
        assert_eq!(code_to_rule("SH-014"), Some(Rule::SelectLoop));
        assert_eq!(code_to_rule("SH-019"), Some(Rule::Coproc));
        assert_eq!(code_to_rule("SH-034"), Some(Rule::LegacyBackticks));
        assert_eq!(
            code_to_rule("SH-035"),
            Some(Rule::LegacyArithmeticExpansion)
        );
        assert_eq!(code_to_rule("SH-055"), Some(Rule::ExprArithmetic));
        assert_eq!(code_to_rule("SH-056"), Some(Rule::PsGrepPipeline));
        assert_eq!(code_to_rule("SH-057"), Some(Rule::LsGrepPipeline));
        assert_eq!(code_to_rule("S014"), Some(Rule::UnquotedDollarStar));
        assert_eq!(code_to_rule("SH-062"), Some(Rule::UnquotedDollarStar));
        assert_eq!(code_to_rule("S015"), Some(Rule::QuotedDollarStarLoop));
        assert_eq!(code_to_rule("SH-063"), Some(Rule::QuotedDollarStarLoop));
        assert_eq!(code_to_rule("S017"), Some(Rule::UnquotedArraySplit));
        assert_eq!(code_to_rule("SH-067"), Some(Rule::UnquotedArraySplit));
        assert_eq!(code_to_rule("S018"), Some(Rule::CommandOutputArraySplit));
        assert_eq!(code_to_rule("SH-068"), Some(Rule::CommandOutputArraySplit));
        assert_eq!(code_to_rule("S067"), Some(Rule::BacktickOutputToCommand));
        assert_eq!(code_to_rule("SH-294"), Some(Rule::BacktickOutputToCommand));
        assert_eq!(code_to_rule("S071"), Some(Rule::EnvPrefixQuoting));
        assert_eq!(code_to_rule("SH-309"), Some(Rule::EnvPrefixQuoting));
        assert_eq!(code_to_rule("S076"), Some(Rule::MixedQuoteWord));
        assert_eq!(code_to_rule("SH-350"), Some(Rule::MixedQuoteWord));
        assert_eq!(code_to_rule("S021"), Some(Rule::PositionalArgsInString));
        assert_eq!(code_to_rule("SH-077"), Some(Rule::PositionalArgsInString));
        assert_eq!(code_to_rule("S020"), Some(Rule::SingleIterationLoop));
        assert_eq!(code_to_rule("SH-076"), Some(Rule::SingleIterationLoop));
        assert_eq!(
            code_to_rule("S032"),
            Some(Rule::ConditionalAssignmentShortcut)
        );
        assert_eq!(
            code_to_rule("SH-128"),
            Some(Rule::ConditionalAssignmentShortcut)
        );
        assert_eq!(code_to_rule("SH-064"), Some(Rule::GrepCountPipeline));
        assert_eq!(code_to_rule("SH-137"), Some(Rule::SingleTestSubshell));
        assert_eq!(code_to_rule("SH-164"), Some(Rule::SubshellTestGroup));
        assert_eq!(code_to_rule("S023"), Some(Rule::EscapedUnderscore));
        assert_eq!(code_to_rule("SH-082"), Some(Rule::EscapedUnderscore));
        assert_eq!(code_to_rule("S027"), Some(Rule::EscapedUnderscoreLiteral));
        assert_eq!(code_to_rule("SH-095"), Some(Rule::EscapedUnderscoreLiteral));
        assert_eq!(code_to_rule("S034"), Some(Rule::ArrayIndexArithmetic));
        assert_eq!(code_to_rule("SH-157"), Some(Rule::ArrayIndexArithmetic));
        assert_eq!(code_to_rule("S035"), Some(Rule::ArithmeticScoreLine));
        assert_eq!(code_to_rule("SH-161"), Some(Rule::ArithmeticScoreLine));
        assert_eq!(
            code_to_rule("SH-228"),
            Some(Rule::FunctionCalledWithoutArgs)
        );
        assert_eq!(
            code_to_rule("SH-292"),
            Some(Rule::FunctionReferencesUnsetParam)
        );
        assert_eq!(code_to_rule("S045"), Some(Rule::DollarInArithmetic));
        assert_eq!(code_to_rule("SH-197"), Some(Rule::DollarInArithmetic));
        assert_eq!(code_to_rule("S046"), Some(Rule::LsPipedToXargs));
        assert_eq!(code_to_rule("SH-198"), Some(Rule::LsPipedToXargs));
        assert_eq!(code_to_rule("S047"), Some(Rule::LsInSubstitution));
        assert_eq!(code_to_rule("SH-199"), Some(Rule::LsInSubstitution));
        assert_eq!(code_to_rule("S048"), Some(Rule::DollarInArithmeticContext));
        assert_eq!(
            code_to_rule("SH-202"),
            Some(Rule::DollarInArithmeticContext)
        );
        assert_eq!(code_to_rule("S055"), Some(Rule::GlobAssignedToVariable));
        assert_eq!(code_to_rule("SH-231"), Some(Rule::GlobAssignedToVariable));
        assert_eq!(code_to_rule("SH-203"), Some(Rule::UnquotedTrRange));
        assert_eq!(code_to_rule("S024"), Some(Rule::SingleQuoteBackslash));
        assert_eq!(code_to_rule("SH-087"), Some(Rule::SingleQuoteBackslash));
        assert_eq!(code_to_rule("S052"), Some(Rule::UnquotedVariableInTest));
        assert_eq!(code_to_rule("SH-208"), Some(Rule::UnquotedTrClass));
        assert_eq!(code_to_rule("SH-212"), Some(Rule::UnquotedVariableInTest));
        assert_eq!(code_to_rule("S058"), Some(Rule::UnquotedPathInMkdir));
        assert_eq!(code_to_rule("S062"), Some(Rule::DefaultValueInColonAssign));
        assert_eq!(
            code_to_rule("SH-251"),
            Some(Rule::DefaultValueInColonAssign)
        );
        assert_eq!(code_to_rule("SH-025"), Some(Rule::DynamicSourcePath));
        assert_eq!(
            code_to_rule("S039"),
            Some(Rule::LiteralBackslashInSingleQuotes)
        );
        assert_eq!(
            code_to_rule("SH-172"),
            Some(Rule::LiteralBackslashInSingleQuotes)
        );
        assert_eq!(code_to_rule("S040"), Some(Rule::BackslashBeforeCommand));
        assert_eq!(code_to_rule("SH-173"), Some(Rule::BackslashBeforeCommand));
        assert_eq!(code_to_rule("SH-026"), Some(Rule::UntrackedSourceFile));
        assert_eq!(code_to_rule("SH-036"), Some(Rule::SingleQuotedLiteral));
        assert_eq!(code_to_rule("SH-037"), Some(Rule::PrintfFormatVariable));
        assert_eq!(code_to_rule("SH-038"), Some(Rule::UnquotedArrayExpansion));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
        assert_eq!(
            code_to_rule("SH-040"),
            Some(Rule::EchoedCommandSubstitution)
        );
        assert_eq!(
            code_to_rule("SH-066"),
            Some(Rule::EchoInsideCommandSubstitution)
        );
        assert_eq!(code_to_rule("SH-168"), Some(Rule::RedundantSpacesInEcho));
        assert_eq!(code_to_rule("SH-196"), Some(Rule::UnquotedVariableInSed));
        assert_eq!(code_to_rule("SH-163"), Some(Rule::BareRead));
        assert_eq!(code_to_rule("SH-071"), Some(Rule::GrepOutputInTest));
        assert_eq!(code_to_rule("SH-041"), Some(Rule::FindOutputToXargs));
        assert_eq!(code_to_rule("SH-042"), Some(Rule::TrapStringExpansion));
        assert_eq!(code_to_rule("SH-043"), Some(Rule::QuotedBashRegex));
        assert_eq!(code_to_rule("SH-044"), Some(Rule::RmGlobOnVariablePath));
        assert_eq!(code_to_rule("SH-047"), Some(Rule::SshLocalExpansion));
        assert_eq!(code_to_rule("SH-045"), Some(Rule::ChainedTestBranches));
        assert_eq!(code_to_rule("SH-201"), Some(Rule::ShortCircuitFallthrough));
        assert_eq!(code_to_rule("SH-046"), Some(Rule::LineOrientedInput));
        assert_eq!(code_to_rule("SH-049"), Some(Rule::FindOutputLoop));
        assert_eq!(code_to_rule("C113"), Some(Rule::FindOutputLoop));
        assert_eq!(
            code_to_rule("SH-050"),
            Some(Rule::ExportCommandSubstitution)
        );
        assert_eq!(code_to_rule("S033"), Some(Rule::EchoHereDoc));
        assert_eq!(code_to_rule("SH-135"), Some(Rule::EchoHereDoc));
        assert_eq!(code_to_rule("SH-052"), Some(Rule::LocalTopLevel));
        assert_eq!(code_to_rule("SH-060"), Some(Rule::SudoRedirectionOrder));
        assert_eq!(code_to_rule("SH-081"), Some(Rule::PrintfQFormatInSh));
        assert_eq!(code_to_rule("SH-275"), Some(Rule::ErrexitTrapInSh));
        assert_eq!(code_to_rule("SH-276"), Some(Rule::SignalNameInTrap));
        assert_eq!(code_to_rule("SH-277"), Some(Rule::BasePrefixInArithmetic));
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
        assert_eq!(code_to_rule("SH-111"), Some(Rule::ExtglobCase));
        assert_eq!(code_to_rule("SH-182"), Some(Rule::ExtglobInCasePattern));
        assert_eq!(code_to_rule("SH-261"), Some(Rule::ExtglobInSh));
        assert_eq!(code_to_rule("SH-272"), Some(Rule::CaretNegationInBracket));
        assert_eq!(code_to_rule("C038"), Some(Rule::ElseIf));
        assert_eq!(code_to_rule("SH-112"), Some(Rule::ElseIf));
        assert_eq!(code_to_rule("C039"), Some(Rule::OpenDoubleQuote));
        assert_eq!(code_to_rule("SH-113"), Some(Rule::OpenDoubleQuote));
        assert_eq!(code_to_rule("S028"), Some(Rule::SuspectClosingQuote));
        assert_eq!(code_to_rule("SH-114"), Some(Rule::SuspectClosingQuote));
        assert_eq!(
            code_to_rule("SH-245"),
            Some(Rule::DeprecatedTempfileCommand)
        );
        assert_eq!(code_to_rule("SH-247"), Some(Rule::EgrepDeprecated));
        assert_eq!(code_to_rule("S029"), Some(Rule::LiteralBraces));
        assert_eq!(code_to_rule("SH-116"), Some(Rule::LiteralBraces));
        assert_eq!(code_to_rule("S030"), Some(Rule::HeredocEndSpace));
        assert_eq!(code_to_rule("SH-119"), Some(Rule::HeredocEndSpace));
        assert_eq!(code_to_rule("S031"), Some(Rule::TrailingDirective));
        assert_eq!(code_to_rule("SH-120"), Some(Rule::TrailingDirective));
        assert_eq!(code_to_rule("S072"), Some(Rule::LinebreakBeforeAnd));
        assert_eq!(code_to_rule("SH-329"), Some(Rule::LinebreakBeforeAnd));
        assert_eq!(code_to_rule("S073"), Some(Rule::SpacedTabstripClose));
        assert_eq!(code_to_rule("SH-330"), Some(Rule::SpacedTabstripClose));
        assert_eq!(code_to_rule("S074"), Some(Rule::AmpersandSemicolon));
        assert_eq!(code_to_rule("SH-335"), Some(Rule::AmpersandSemicolon));
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
        assert_eq!(code_to_rule("SH-151"), Some(Rule::EvalOnArray));
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
        assert_eq!(code_to_rule("SH-079"), Some(Rule::AvoidLetBuiltin));
        assert_eq!(
            code_to_rule("SH-224"),
            Some(Rule::AssignmentLooksLikeComparison)
        );
        assert_eq!(code_to_rule("C096"), Some(Rule::UnquotedPipeInEcho));
        assert_eq!(code_to_rule("SH-225"), Some(Rule::UnquotedPipeInEcho));
        assert_eq!(code_to_rule("SH-227"), Some(Rule::SuWithoutFlag));
        assert_eq!(code_to_rule("SH-240"), Some(Rule::UnquotedPathInMkdir));
        assert_eq!(code_to_rule("SH-195"), Some(Rule::SubshellInArithmetic));
        assert_eq!(code_to_rule("C078"), Some(Rule::UnquotedGlobsInFind));
        assert_eq!(code_to_rule("SH-200"), Some(Rule::UnquotedGlobsInFind));
        assert_eq!(code_to_rule("C080"), Some(Rule::GlobInGrepPattern));
        assert_eq!(code_to_rule("SH-204"), Some(Rule::GlobInGrepPattern));
        assert_eq!(code_to_rule("C081"), Some(Rule::GlobInStringComparison));
        assert_eq!(code_to_rule("SH-206"), Some(Rule::GlobInStringComparison));
        assert_eq!(code_to_rule("C083"), Some(Rule::GlobInFindSubstitution));
        assert_eq!(code_to_rule("SH-209"), Some(Rule::GlobInFindSubstitution));
        assert_eq!(code_to_rule("C084"), Some(Rule::UnquotedGrepRegex));
        assert_eq!(code_to_rule("SH-210"), Some(Rule::UnquotedGrepRegex));
        assert_eq!(code_to_rule("C006"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("SH-039"), Some(Rule::UndefinedVariable));
        assert_eq!(code_to_rule("C076"), Some(Rule::CommentedContinuationLine));
        assert_eq!(
            code_to_rule("SH-194"),
            Some(Rule::CommentedContinuationLine)
        );
        assert_eq!(code_to_rule("C098"), Some(Rule::SetFlagsWithoutDashes));
        assert_eq!(code_to_rule("SH-229"), Some(Rule::SetFlagsWithoutDashes));
        assert_eq!(code_to_rule("C099"), Some(Rule::QuotedArraySlice));
        assert_eq!(code_to_rule("SH-230"), Some(Rule::QuotedArraySlice));
        assert_eq!(code_to_rule("C100"), Some(Rule::QuotedBashSource));
        assert_eq!(code_to_rule("SH-232"), Some(Rule::QuotedBashSource));
        assert_eq!(
            code_to_rule("SH-233"),
            Some(Rule::CommandSubstitutionInAlias)
        );
        assert_eq!(code_to_rule("SH-235"), Some(Rule::FunctionInAlias));
        assert_eq!(code_to_rule("C103"), Some(Rule::FindOrWithoutGrouping));
        assert_eq!(code_to_rule("SH-237"), Some(Rule::FindOrWithoutGrouping));
        assert_eq!(code_to_rule("C104"), Some(Rule::NonShellSyntaxInScript));
        assert_eq!(code_to_rule("SH-238"), Some(Rule::NonShellSyntaxInScript));
        assert_eq!(code_to_rule("C105"), Some(Rule::ExportWithPositionalParams));
        assert_eq!(
            code_to_rule("SH-239"),
            Some(Rule::ExportWithPositionalParams)
        );
        assert_eq!(code_to_rule("C111"), Some(Rule::AtSignInStringCompare));
        assert_eq!(code_to_rule("SH-249"), Some(Rule::AtSignInStringCompare));
        assert_eq!(code_to_rule("C112"), Some(Rule::ArraySliceInComparison));
        assert_eq!(code_to_rule("SH-250"), Some(Rule::ArraySliceInComparison));
        assert_eq!(code_to_rule("C106"), Some(Rule::AppendToArrayAsString));
        assert_eq!(code_to_rule("SH-241"), Some(Rule::AppendToArrayAsString));
        assert_eq!(
            code_to_rule("C108"),
            Some(Rule::UnsetAssociativeArrayElement)
        );
        assert_eq!(
            code_to_rule("SH-243"),
            Some(Rule::UnsetAssociativeArrayElement)
        );
        assert_eq!(code_to_rule("C109"), Some(Rule::MapfileProcessSubstitution));
        assert_eq!(
            code_to_rule("SH-244"),
            Some(Rule::MapfileProcessSubstitution)
        );
        assert_eq!(code_to_rule("C114"), Some(Rule::GlobWithExpansionInLoop));
        assert_eq!(code_to_rule("SH-254"), Some(Rule::GlobWithExpansionInLoop));
        assert_eq!(code_to_rule("SH-253"), Some(Rule::FindOutputLoop));
        assert_eq!(code_to_rule("C124"), Some(Rule::UnreachableAfterExit));
        assert_eq!(code_to_rule("SH-293"), Some(Rule::UnreachableAfterExit));
        assert_eq!(code_to_rule("C127"), Some(Rule::UnusedHeredoc));
        assert_eq!(code_to_rule("SH-298"), Some(Rule::UnusedHeredoc));
        assert_eq!(code_to_rule("C138"), Some(Rule::HeredocMissingEnd));
        assert_eq!(code_to_rule("SH-318"), Some(Rule::HeredocMissingEnd));
        assert_eq!(
            code_to_rule("C125"),
            Some(Rule::UncheckedDirectoryChangeInFunction)
        );
        assert_eq!(
            code_to_rule("SH-295"),
            Some(Rule::UncheckedDirectoryChangeInFunction)
        );
        assert_eq!(
            code_to_rule("C126"),
            Some(Rule::ContinueOutsideLoopInFunction)
        );
        assert_eq!(
            code_to_rule("SH-296"),
            Some(Rule::ContinueOutsideLoopInFunction)
        );
        assert_eq!(code_to_rule("C131"), Some(Rule::VariableAsCommandName));
        assert_eq!(code_to_rule("SH-308"), Some(Rule::VariableAsCommandName));
        assert_eq!(code_to_rule("C132"), Some(Rule::MisspelledOptionName));
        assert_eq!(code_to_rule("SH-310"), Some(Rule::MisspelledOptionName));
        assert_eq!(
            code_to_rule("SH-257"),
            Some(Rule::DefaultElseInShortCircuit)
        );
        assert_eq!(code_to_rule("C133"), Some(Rule::ArrayToStringConversion));
        assert_eq!(code_to_rule("SH-311"), Some(Rule::ArrayToStringConversion));
        assert_eq!(code_to_rule("C147"), Some(Rule::KeywordFunctionName));
        assert_eq!(code_to_rule("SH-336"), Some(Rule::KeywordFunctionName));
        assert_eq!(code_to_rule("C148"), Some(Rule::BrokenAssocKey));
        assert_eq!(code_to_rule("SH-337"), Some(Rule::BrokenAssocKey));
        assert_eq!(code_to_rule("C151"), Some(Rule::CommaArrayElements));
        assert_eq!(code_to_rule("SH-340"), Some(Rule::CommaArrayElements));
        assert_eq!(code_to_rule("SH-283"), Some(Rule::FindExecDirWithShell));
        assert_eq!(
            code_to_rule("C137"),
            Some(Rule::UnicodeSingleQuoteInSingleQuotes)
        );
        assert_eq!(
            code_to_rule("SH-315"),
            Some(Rule::UnicodeSingleQuoteInSingleQuotes)
        );
        assert_eq!(code_to_rule("C141"), Some(Rule::LoopWithoutEnd));
        assert_eq!(code_to_rule("SH-321"), Some(Rule::LoopWithoutEnd));
        assert_eq!(code_to_rule("C142"), Some(Rule::MissingDoneInForLoop));
        assert_eq!(code_to_rule("SH-322"), Some(Rule::MissingDoneInForLoop));
        assert_eq!(code_to_rule("C143"), Some(Rule::DanglingElse));
        assert_eq!(code_to_rule("SH-327"), Some(Rule::DanglingElse));
        assert_eq!(code_to_rule("C144"), Some(Rule::HeredocCloserNotAlone));
        assert_eq!(code_to_rule("SH-332"), Some(Rule::HeredocCloserNotAlone));
        assert_eq!(code_to_rule("C145"), Some(Rule::MisquotedHeredocClose));
        assert_eq!(code_to_rule("SH-333"), Some(Rule::MisquotedHeredocClose));
        assert_eq!(code_to_rule("C146"), Some(Rule::UntilMissingDo));
        assert_eq!(code_to_rule("SH-334"), Some(Rule::UntilMissingDo));
        assert_eq!(code_to_rule("C157"), Some(Rule::IfBracketGlued));
        assert_eq!(code_to_rule("SH-353"), Some(Rule::IfBracketGlued));
        assert_eq!(code_to_rule("X005"), Some(Rule::BashCaseFallthrough));
        assert_eq!(code_to_rule("X008"), Some(Rule::StandaloneArithmetic));
        assert_eq!(code_to_rule("X009"), Some(Rule::SelectLoop));
        assert_eq!(code_to_rule("X014"), Some(Rule::Coproc));
        assert_eq!(code_to_rule("X036"), Some(Rule::ZshRedirPipe));
        assert_eq!(code_to_rule("SH-108"), Some(Rule::ZshRedirPipe));
        assert_eq!(code_to_rule("X038"), Some(Rule::ZshBraceIf));
        assert_eq!(code_to_rule("SH-124"), Some(Rule::ZshBraceIf));
        assert_eq!(code_to_rule("X039"), Some(Rule::ZshAlwaysBlock));
        assert_eq!(code_to_rule("SH-125"), Some(Rule::ZshAlwaysBlock));
        assert_eq!(code_to_rule("X042"), Some(Rule::SourcedWithArgs));
        assert_eq!(code_to_rule("SH-140"), Some(Rule::SourcedWithArgs));
        assert_eq!(code_to_rule("X043"), Some(Rule::ZshFlagExpansion));
        assert_eq!(code_to_rule("SH-153"), Some(Rule::ZshFlagExpansion));
        assert_eq!(code_to_rule("X044"), Some(Rule::NestedZshSubstitution));
        assert_eq!(code_to_rule("SH-154"), Some(Rule::NestedZshSubstitution));
        assert_eq!(code_to_rule("X045"), Some(Rule::PlusEqualsAppend));
        assert_eq!(code_to_rule("SH-158"), Some(Rule::PlusEqualsAppend));
        assert_eq!(code_to_rule("X047"), Some(Rule::MultiVarForLoop));
        assert_eq!(code_to_rule("SH-180"), Some(Rule::MultiVarForLoop));
        assert_eq!(code_to_rule("X049"), Some(Rule::ZshPromptBracket));
        assert_eq!(code_to_rule("SH-183"), Some(Rule::ZshPromptBracket));
        assert_eq!(code_to_rule("X050"), Some(Rule::CshSyntaxInSh));
        assert_eq!(code_to_rule("SH-184"), Some(Rule::CshSyntaxInSh));
        assert_eq!(code_to_rule("X051"), Some(Rule::ZshNestedExpansion));
        assert_eq!(code_to_rule("SH-218"), Some(Rule::ZshNestedExpansion));
        assert_eq!(code_to_rule("X053"), Some(Rule::ZshAssignmentToZero));
        assert_eq!(code_to_rule("SH-260"), Some(Rule::ZshAssignmentToZero));
        assert_eq!(code_to_rule("X055"), Some(Rule::DollarStringInSh));
        assert_eq!(code_to_rule("SH-262"), Some(Rule::DollarStringInSh));
        assert_eq!(code_to_rule("X056"), Some(Rule::CStyleForInSh));
        assert_eq!(code_to_rule("SH-263"), Some(Rule::CStyleForInSh));
        assert_eq!(code_to_rule("X057"), Some(Rule::LegacyArithmeticInSh));
        assert_eq!(code_to_rule("SH-264"), Some(Rule::LegacyArithmeticInSh));
        assert_eq!(code_to_rule("X062"), Some(Rule::CStyleForArithmeticInSh));
        assert_eq!(code_to_rule("SH-269"), Some(Rule::CStyleForArithmeticInSh));
        assert_eq!(code_to_rule("X064"), Some(Rule::PlusEqualsInSh));
        assert_eq!(code_to_rule("SH-271"), Some(Rule::PlusEqualsInSh));
        assert_eq!(code_to_rule("X071"), Some(Rule::ArrayKeysInSh));
        assert_eq!(code_to_rule("SH-278"), Some(Rule::ArrayKeysInSh));
        assert_eq!(code_to_rule("X081"), Some(Rule::StarGlobRemovalInSh));
        assert_eq!(code_to_rule("SH-305"), Some(Rule::StarGlobRemovalInSh));
        assert_eq!(code_to_rule("X076"), Some(Rule::ZshParameterFlag));
        assert_eq!(code_to_rule("SH-286"), Some(Rule::ZshParameterFlag));
        assert_eq!(code_to_rule("X078"), Some(Rule::ZshArraySubscriptInCase));
        assert_eq!(code_to_rule("SH-299"), Some(Rule::ZshArraySubscriptInCase));
        assert_eq!(code_to_rule("X079"), Some(Rule::ZshParameterIndexFlag));
        assert_eq!(code_to_rule("SH-303"), Some(Rule::ZshParameterIndexFlag));
        assert_eq!(code_to_rule("X013"), Some(Rule::ArrayAssignment));
        assert_eq!(code_to_rule("SH-018"), Some(Rule::ArrayAssignment));
        assert_eq!(code_to_rule("X018"), Some(Rule::IndirectExpansion));
        assert_eq!(code_to_rule("SH-023"), Some(Rule::IndirectExpansion));
        assert_eq!(code_to_rule("X019"), Some(Rule::ArrayReference));
        assert_eq!(code_to_rule("SH-024"), Some(Rule::ArrayReference));
        assert_eq!(code_to_rule("X023"), Some(Rule::SubstringExpansion));
        assert_eq!(code_to_rule("SH-031"), Some(Rule::SubstringExpansion));
        assert_eq!(code_to_rule("X024"), Some(Rule::CaseModificationExpansion));
        assert_eq!(
            code_to_rule("SH-032"),
            Some(Rule::CaseModificationExpansion)
        );
        assert_eq!(code_to_rule("X025"), Some(Rule::ReplacementExpansion));
        assert_eq!(code_to_rule("SH-033"), Some(Rule::ReplacementExpansion));
        assert_eq!(code_to_rule("X026"), Some(Rule::BashFileSlurp));
        assert_eq!(code_to_rule("SH-053"), Some(Rule::BashFileSlurp));
    }
}
