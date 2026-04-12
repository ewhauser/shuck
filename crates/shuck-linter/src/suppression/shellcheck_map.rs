use rustc_hash::FxHashMap;

use crate::Rule;

/// Maps shellcheck SC codes to shuck rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckCodeMap {
    map: FxHashMap<u32, Rule>,
    aliases: Vec<(u32, Rule)>,
    comparison: Vec<(u32, Rule)>,
}

impl ShellCheckCodeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mappings(&self) -> impl Iterator<Item = (u32, Rule)> + '_ {
        self.map
            .iter()
            .map(|(sc_code, rule)| (*sc_code, *rule))
            .chain(self.aliases.iter().copied())
    }

    /// Mappings that are stable enough to compare in the large corpus harness.
    pub fn comparison_mappings(&self) -> impl Iterator<Item = (u32, Rule)> + '_ {
        self.comparison.iter().copied()
    }

    /// Look up a shellcheck code like `SC2086`.
    pub fn resolve(&self, sc_code: &str) -> Option<Rule> {
        self.resolve_all(sc_code).into_iter().next()
    }

    /// Look up all shellcheck mappings for a code like `SC2086`.
    pub fn resolve_all(&self, sc_code: &str) -> Vec<Rule> {
        let code = sc_code
            .strip_prefix("SC")
            .or_else(|| sc_code.strip_prefix("sc"))
            .unwrap_or(sc_code);
        let Some(number) = code.parse().ok() else {
            return Vec::new();
        };
        if number == 2262 {
            return vec![Rule::TemplateBraceInCommand];
        }
        if number == 2261 {
            return vec![Rule::NonAbsoluteShebang];
        }
        if number == 2260 {
            return vec![Rule::RedirectToCommandName];
        }
        if number == 2253 {
            return vec![Rule::StatusCaptureAfterBranchTest];
        }
        if number == 2291 {
            return vec![Rule::UnquotedVariableInSed];
        }
        if number == 2117 {
            return vec![Rule::SuWithoutFlag];
        }
        if number == 2139 {
            return vec![Rule::CommandSubstitutionInAlias];
        }
        if number == 2142 {
            return vec![Rule::FunctionInAlias];
        }
        if number == 2340 {
            return vec![Rule::DeprecatedTempfileCommand];
        }
        if number == 2186 {
            return vec![Rule::DeprecatedTempfileCommand];
        }
        if number == 2342 {
            return vec![Rule::EgrepDeprecated];
        }
        if number == 2196 {
            return vec![Rule::EgrepDeprecated];
        }
        if number == 2303 {
            return vec![Rule::UnquotedTrClass];
        }
        if number == 2060 {
            return vec![Rule::UnquotedTrClass, Rule::UnquotedTrRange];
        }
        if number == 2293 {
            return vec![Rule::LsPipedToXargs];
        }
        if number == 2294 {
            return vec![Rule::LsInSubstitution, Rule::EvalOnArray];
        }
        if number == 2281 {
            return vec![Rule::BackslashBeforeClosingBacktick];
        }
        if number == 2282 {
            return vec![Rule::BadVarName, Rule::PositionalParamAsOperator];
        }
        if number == 2283 {
            return vec![Rule::DoubleParenGrouping];
        }
        if number == 2143 {
            return vec![Rule::GrepOutputInTest];
        }
        if number == 2258 {
            return vec![Rule::BareRead];
        }
        if number == 2284 {
            return vec![Rule::UnicodeQuoteInString];
        }
        if number == 1126 {
            return vec![Rule::TrailingDirective];
        }
        if number == 2385 {
            return vec![Rule::UnicodeSingleQuoteInSingleQuotes];
        }
        let mut resolved = self
            .map
            .get(&number)
            .copied()
            .into_iter()
            .collect::<Vec<_>>();
        resolved.extend(
            self.aliases
                .iter()
                .filter_map(|(code, rule)| (*code == number).then_some(*rule)),
        );
        resolved
    }
}

impl Default for ShellCheckCodeMap {
    fn default() -> Self {
        let comparison = vec![
            (2003, Rule::ExprArithmetic),
            (2005, Rule::EchoedCommandSubstitution),
            (2116, Rule::EchoInsideCommandSubstitution),
            (2143, Rule::GrepOutputInTest),
            (2145, Rule::PositionalArgsInString),
            (2198, Rule::AtSignInStringCompare),
            (2199, Rule::ArraySliceInComparison),
            // Modern ShellCheck reuses SC2320 for a different echo/printf `$?` warning.
            // Keep the historical SC2320 comparison slot for C096 and review known corpus deltas.
            (2320, Rule::UnquotedPipeInEcho),
            // ShellCheck 0.11.0 reports quoted array-to-scalar assignments as SC2124.
            // Keep SC2325 as a suppression alias for the authored C099 rule code.
            (2124, Rule::QuotedArraySlice),
            // ShellCheck 0.11.0 reports quoted unindexed BASH_SOURCE expansions as SC2128.
            // Keep SC2327 as a suppression alias for the authored C100 rule code.
            (2128, Rule::QuotedBashSource),
            (2006, Rule::LegacyBackticks),
            (2007, Rule::LegacyArithmeticExpansion),
            (2009, Rule::PsGrepPipeline),
            (2010, Rule::LsGrepPipeline),
            (2293, Rule::LsPipedToXargs),
            (2294, Rule::LsInSubstitution),
            (2263, Rule::RedundantSpacesInEcho),
            // ShellCheck 0.11.0 reports single-quoted split-word cases as SC2026.
            // Keep SC2300 as a suppression alias for the authored S050 rule code.
            (2026, Rule::UnquotedWordBetweenQuotes),
            (2291, Rule::UnquotedVariableInSed),
            (2117, Rule::SuWithoutFlag),
            // ShellCheck 0.11.0 reports tempfile deprecation warnings as SC2186.
            // Keep SC2340 as a suppression alias for the authored S059 rule code.
            (2186, Rule::DeprecatedTempfileCommand),
            // ShellCheck 0.11.0 reports egrep deprecation warnings as SC2196.
            // Keep SC2342 as a suppression alias for the authored S060 rule code.
            (2196, Rule::EgrepDeprecated),
            // ShellCheck 0.11.0 reports command substitutions inside alias definitions as SC2139.
            // Keep SC2328 as a suppression alias for historical compatibility.
            (2139, Rule::CommandSubstitutionInAlias),
            // ShellCheck 0.11.0 reports alias function-definition issues as SC2142.
            // Keep SC2330 as a suppression alias for the authored S057 rule code.
            (2142, Rule::FunctionInAlias),
            // ShellCheck 0.11.0 reports split double-quoted strings around variables as SC2027.
            // Keep SC2376 as a suppression alias for the authored S070 rule code.
            (2027, Rule::DoubleQuoteNesting),
            // SC2379 remains the authored compatibility code for env-prefix quoting checks.
            (2379, Rule::EnvPrefixQuoting),
            (2140, Rule::MixedQuoteWord),
            // ShellCheck 0.11.0 reports `tr [:upper:] [:lower:]`-style class warnings as SC2060.
            // Keep SC2303 as a suppression alias for the authored S051 rule code.
            (2060, Rule::UnquotedTrClass),
            (2335, Rule::UnquotedPathInMkdir),
            // ShellCheck 0.11.0 reports unquoted `-n` test operands as SC2070.
            // Keep SC2307 as a suppression alias for the authored S052 rule code.
            (2070, Rule::UnquotedVariableInTest),
            // ShellCheck 0.11.0 reports unquoted default-assignment expansions as SC2223.
            // Keep SC2346 as a suppression alias for the authored S062 rule code.
            (2223, Rule::DefaultValueInColonAssign),
            (2021, Rule::UnquotedTrRange),
            (2283, Rule::DoubleParenGrouping),
            (1037, Rule::PositionalTenBraces),
            // ShellCheck 0.11.0 reports space-indented `<<-` close candidates as SC1040.
            // Keep SC2393 as a suppression alias for compatibility metadata.
            (1040, Rule::SpacedTabstripClose),
            // ShellCheck 0.11.0 reports trailing whitespace after heredoc terminators as SC1118.
            // Keep SC1040 as a suppression alias for compatibility metadata.
            (1118, Rule::HeredocEndSpace),
            // ShellCheck 0.11.0 reports near-match heredoc closers as SC1042.
            // Keep SC2395 as a suppression alias for compatibility metadata.
            (1042, Rule::MisquotedHeredocClose),
            // ShellCheck 0.11.0 reports heredoc closers mixed with line content as SC1041.
            // Keep SC2394 as a suppression alias for compatibility metadata.
            (1041, Rule::HeredocCloserNotAlone),
            // ShellCheck 0.11.0 reports missing heredoc terminators as SC1044.
            // Keep SC2386 as a suppression alias for compatibility metadata.
            (1044, Rule::HeredocMissingEnd),
            // ShellCheck 0.11.0 reports `foo &;` as SC1045.
            // Keep SC2397 as a suppression alias for historical compatibility.
            (1045, Rule::AmpersandSemicolon),
            // ShellCheck 0.11.0 reports case-pattern shadowing as SC2221.
            // Keep SC2373 as a suppression alias for the authored C128 rule code.
            (2221, Rule::CaseGlobReachability),
            // ShellCheck 0.11.0 reports the shadowed case pattern as SC2222.
            // Keep SC2374 as a suppression alias for the authored C129 rule code.
            (2222, Rule::CaseDefaultBeforeGlob),
            // ShellCheck 0.11.0 reports missing getopts case arms as SC2213.
            // Keep SC2382 as a suppression alias for the authored C134 rule code.
            (2213, Rule::GetoptsOptionNotInCase),
            // ShellCheck 0.11.0 reports undeclared getopts case arms as SC2214.
            // Keep SC2383 as a suppression alias for the authored C135 rule code.
            (2214, Rule::CaseArmNotInGetopts),
            // Keep the historical SC2372 comparison slot for the authored S069 rule code.
            (2372, Rule::SingleLetterCaseLabel),
            (1047, Rule::MissingFi),
            (1065, Rule::FunctionParamsInSh),
            (1069, Rule::IfBracketGlued),
            (1072, Rule::BrokenTestParse),
            (1073, Rule::BrokenTestEnd),
            (1075, Rule::ElseIf),
            (1078, Rule::OpenDoubleQuote),
            (1079, Rule::SuspectClosingQuote),
            (1080, Rule::LinebreakInTest),
            (1083, Rule::LiteralBraces),
            (1090, Rule::DynamicSourcePath),
            (1091, Rule::UntrackedSourceFile),
            (1097, Rule::IfsEqualsAmbiguity),
            (1101, Rule::BackslashBeforeClosingBacktick),
            (1102, Rule::PositionalParamAsOperator),
            (1110, Rule::UnicodeQuoteInString),
            (1126, Rule::TrailingDirective),
            (1113, Rule::TrailingDirective),
            (1127, Rule::CStyleComment),
            (1132, Rule::CPrototypeFragment),
            (2164, Rule::UncheckedDirectoryChange),
            (2016, Rule::SingleQuotedLiteral),
            (2013, Rule::LineOrientedInput),
            (2015, Rule::ChainedTestBranches),
            // ShellCheck 0.11.0 reports `find -exec` pre-expansion warnings as SC2014.
            // Keep SC2295 as a suppression alias for authored C078 metadata.
            (2014, Rule::UnquotedGlobsInFind),
            (2296, Rule::ShortCircuitFallthrough),
            // ShellCheck 0.11.0 reports loop-list glob+expansion mixes as SC2231.
            // Keep SC2349 as a suppression alias for authored C114 metadata.
            (2231, Rule::GlobWithExpansionInLoop),
            // ShellCheck 0.11.0 reports grep-pattern glob/regex confusion as SC2022.
            // Keep SC2299 as a suppression alias for authored C080 metadata.
            (2022, Rule::GlobInGrepPattern),
            // ShellCheck 0.11.0 reports unquoted grep regex shell expansion as SC2062.
            // Keep SC2305 as a suppression alias for authored C084 metadata.
            (2062, Rule::UnquotedGrepRegex),
            // ShellCheck 0.11.0 reports unquoted variable-backed `[[ ... == ... ]]` patterns as SC2053.
            // Keep SC2301 as a suppression alias for authored C081 metadata.
            (2053, Rule::GlobInStringComparison),
            // ShellCheck 0.11.0 reports unquoted glob operands in `find` predicates as SC2061.
            // Keep SC2304 as a suppression alias for authored C083 metadata.
            (2061, Rule::GlobInFindSubstitution),
            // ShellCheck 0.11.0 reports unquoted glob assignment values as SC2125.
            // Keep SC2326 as a suppression alias for authored S055 metadata.
            (2125, Rule::GlobAssignedToVariable),
            (1019, Rule::EmptyTest),
            (2024, Rule::SudoRedirectionOrder),
            (2034, Rule::UnusedAssignment),
            (2035, Rule::LeadingGlobArgument),
            (2143, Rule::GrepOutputInTest),
            // The pinned ShellCheck oracle reports single-item `for ... in ...` loops as SC2043.
            // Keep SC2165 as a suppression alias for the authored S020 metadata.
            (2043, Rule::SingleIterationLoop),
            // The pinned ShellCheck oracle reports conditional-assignment shortcuts as SC2209.
            // Keep SC2114 as a suppression alias for the authored S032 metadata.
            (2209, Rule::ConditionalAssignmentShortcut),
            // ShellCheck 0.11.0 reports `find` output-in-loop warnings as SC2044.
            // Keep SC2348 as a suppression alias for historical compatibility.
            (2044, Rule::FindOutputLoop),
            (2380, Rule::MisspelledOptionName),
            (2045, Rule::LoopFromCommandOutput),
            (2046, Rule::UnquotedCommandSubstitution),
            (2048, Rule::UnquotedDollarStar),
            (2198, Rule::AtSignInStringCompare),
            (2199, Rule::AtSignInStringCompare),
            (2059, Rule::PrintfFormatVariable),
            (2029, Rule::SshLocalExpansion),
            (2352, Rule::DefaultElseInShortCircuit),
            // The pinned ShellCheck oracle reports `+=` assignment portability findings as SC3024.
            // Keep SC3055/SC3071 as suppression aliases for compatibility with older rule metadata.
            (3024, Rule::PlusEqualsAppend),
            (3024, Rule::PlusEqualsInSh),
            (3002, Rule::ExtglobInSh),
            // The pinned ShellCheck oracle reports `$(< file)` as SC3034.
            // Keep SC3024 as a legacy alias for suppression compatibility.
            (3034, Rule::BashFileSlurp),
            (3037, Rule::EchoFlags),
            (2018, Rule::TrLowerRange),
            (2019, Rule::TrUpperRange),
            (2028, Rule::EchoBackslashEscapes),
            (3025, Rule::PrintfQFormatInSh),
            (3026, Rule::CaretNegationInBracket),
            (3077, Rule::BasePrefixInArithmetic),
            (3043, Rule::LocalVariableInSh),
            (3001, Rule::ProcessSubstitution),
            (3003, Rule::AnsiCQuoting),
            // The pinned ShellCheck oracle reports `$\"...\"` portability findings as SC3004.
            // Keep SC3062 as a suppression alias for compatibility with older rule metadata.
            (3004, Rule::DollarStringInSh),
            (3009, Rule::BraceExpansion),
            (3011, Rule::HereString),
            (3030, Rule::ArrayAssignment),
            (3053, Rule::IndirectExpansion),
            (3079, Rule::UnsetPatternInSh),
            (3083, Rule::NestedDefaultExpansion),
            // The pinned ShellCheck oracle reports `${!arr[*]}` portability findings as SC3055.
            // Keep SC3078 as a suppression alias for compatibility with older rule metadata.
            (3055, Rule::ArrayKeysInSh),
            (2219, Rule::AvoidLetBuiltin),
            // ShellCheck 0.11.0 reports array references as SC3054.
            // Keep SC3028 as a suppression alias, but prefer the current code for comparisons.
            (3054, Rule::ArrayReference),
            (3057, Rule::SubstringExpansion),
            (3059, Rule::CaseModificationExpansion),
            (3060, Rule::ReplacementExpansion),
            (2038, Rule::FindOutputToXargs),
            (2064, Rule::TrapStringExpansion),
            (2066, Rule::QuotedDollarStarLoop),
            (2206, Rule::UnquotedArraySplit),
            (2207, Rule::CommandOutputArraySplit),
            (2366, Rule::BacktickOutputToCommand),
            (2068, Rule::UnquotedArrayExpansion),
            (2076, Rule::QuotedBashRegex),
            (2086, Rule::UnquotedExpansion),
            // ShellCheck 0.11.0 reports ungrouped `find ... -o ...` actions as SC2146.
            // Keep SC2332 as a suppression alias for historical compatibility.
            (2146, Rule::FindOrWithoutGrouping),
            // ShellCheck 0.11.0 reports `set` flag-prefix issues as SC2121.
            // Keep SC2324 as a suppression alias for historical compatibility.
            (2121, Rule::SetFlagsWithoutDashes),
            // ShellCheck 0.11.0 reports quoted associative-array unset keys as SC2184.
            // Keep SC2338 as a suppression alias for historical compatibility.
            (2184, Rule::UnsetAssociativeArrayElement),
            // ShellCheck 0.11.0 reports array-to-scalar rebinding as SC2178.
            // Keep SC2381 as a suppression alias for authored C133 metadata.
            (2178, Rule::ArrayToStringConversion),
            (2054, Rule::CommaArrayElements),
            (2336, Rule::AppendToArrayAsString),
            (2339, Rule::MapfileProcessSubstitution),
            (2115, Rule::RmGlobOnVariablePath),
            (2104, Rule::LoopControlOutsideLoop),
            (2126, Rule::GrepCountPipeline),
            (2112, Rule::FunctionKeyword),
            (2216, Rule::PipeToKill),
            (2156, Rule::FindExecDirWithShell),
            // ShellCheck 0.11.0 reports redirecting heredoc/stdin input into `echo` as SC2217.
            // The legacy S033 metadata still references SC2127.
            (2217, Rule::EchoHereDoc),
            // ShellCheck 0.11.0 reports C-style `for ((...))` loop portability warnings as SC3005.
            // Keep SC3063 as a suppression alias, but prefer the current code for comparisons.
            (3005, Rule::CStyleForInSh),
            (2233, Rule::SingleTestSubshell),
            (2235, Rule::SubshellTestGroup),
            (3006, Rule::StandaloneArithmetic),
            // ShellCheck 0.11.0 reports legacy `$[...]` arithmetic portability warnings as SC3007.
            // Keep SC3064 as a suppression alias, but prefer the current code for comparisons.
            (3007, Rule::LegacyArithmeticInSh),
            (3008, Rule::SelectLoop),
            // ShellCheck 0.11.0 reports C-style `for ((...))` arithmetic operator findings as SC3018.
            // Keep SC3069 as a suppression alias, but prefer the current code for comparisons.
            (3018, Rule::CStyleForArithmeticInSh),
            (3032, Rule::Coproc),
            // ShellCheck 0.11.0 reports `let` portability warnings as SC3039.
            // Keep SC3042 as a suppression alias, but prefer the current code for comparisons.
            (3042, Rule::LetCommand),
            (3039, Rule::LetCommand),
            (3040, Rule::PipefailOption),
            (3047, Rule::TrapErr),
            (3048, Rule::WaitOption),
            (3046, Rule::SourceBuiltinInSh),
            (3050, Rule::BraceFdRedirection),
            (3052, Rule::AmpersandRedirection),
            // ShellCheck 0.11.0 surfaces `;&` / `;;&` portability findings as SC2127.
            // Keep SC3058 as a suppression alias, but prefer the current code for comparisons.
            (2127, Rule::BashCaseFallthrough),
            (3058, Rule::BashCaseFallthrough),
            // The pinned ShellCheck oracle reports `${*%%pattern}` portability findings as SC3058.
            // Keep SC3085 as a suppression alias for compatibility with older rule metadata.
            (3058, Rule::StarGlobRemovalInSh),
            (3075, Rule::ErrexitTrapInSh),
            (3076, Rule::SignalNameInTrap),
            // ShellCheck 0.11.0 reports `source` inside functions as SC3051.
            (3051, Rule::SourceInsideFunctionInSh),
            (3070, Rule::AmpersandRedirectInSh),
            (3073, Rule::PipeStderrInSh),
            (2155, Rule::ExportCommandSubstitution),
            (2156, Rule::FindExecDirWithShell),
            (2157, Rule::ConstantComparisonTest),
            (2158, Rule::LiteralUnaryStringTest),
            (2162, Rule::ReadWithoutRaw),
            (2078, Rule::TruthyLiteralTest),
            (2168, Rule::LocalTopLevel),
            (2194, Rule::ConstantCaseSubject),
            (2210, Rule::BadRedirectionFdOrder),
            (2154, Rule::UndefinedVariable),
            // ShellCheck 0.11.0 reports missing shebangs on comment-first scripts as SC2148.
            // Keep SC2285 as a suppression alias for the authored S043 rule code.
            (2148, Rule::MissingShebangLine),
            (2239, Rule::NonAbsoluteShebang),
            (2286, Rule::IndentedShebang),
            (2287, Rule::SpaceAfterHashBang),
            (2288, Rule::TemplateBraceInCommand),
            (2289, Rule::CommentedContinuationLine),
            (1133, Rule::LinebreakBeforeAnd),
            // ShellCheck 0.11.0 reuses SC2290 for declaration spacing warnings.
            // Keep SC2290 assigned to C077 in comparisons so large-corpus attribution stays stable.
            (2290, Rule::SubshellInArithmetic),
            (2292, Rule::DollarInArithmetic),
            (2297, Rule::DollarInArithmeticContext),
            (2302, Rule::EscapedNegationInTest),
            (2308, Rule::GreaterThanInTest),
            (2333, Rule::NonShellSyntaxInScript),
            // ShellCheck 0.11.0 reports `export "$@"`-style findings as SC2163.
            // Keep SC2334 as a suppression alias for the authored C105 rule code.
            (2163, Rule::ExportWithPositionalParams),
            (2389, Rule::LoopWithoutEnd),
            (2390, Rule::MissingDoneInForLoop),
            (2391, Rule::DanglingElse),
            (2399, Rule::BrokenAssocKey),
            (2392, Rule::LinebreakBeforeAnd),
            (2396, Rule::UntilMissingDo),
            (2397, Rule::AmpersandSemicolon),
            (2241, Rule::InvalidExitStatus),
            (2242, Rule::CasePatternVar),
            (2248, Rule::BareSlashMarker),
            (2258, Rule::BareRead),
            (2257, Rule::ArithmeticRedirectionTarget),
            (2004, Rule::DollarInArithmetic),
            (2321, Rule::ArrayIndexArithmetic),
            (2323, Rule::ArithmeticScoreLine),
            (2264, Rule::NestedParameterExpansion),
            (2250, Rule::PatternWithVariable),
            (2255, Rule::SubstWithRedirect),
            (2256, Rule::SubstWithRedirectErr),
            (2089, Rule::AppendWithEscapedQuotes),
            // ShellCheck 0.11.0 reports declaration cross-reference warnings as SC2318.
            // Keep SC2384 as a suppression alias, but prefer the current code for comparisons.
            (2318, Rule::LocalCrossReference),
            (2276, Rule::PlusPrefixInAssignment),
            // ShellCheck 0.11.0 reports digit-prefixed assignment names as SC2282.
            // Keep SC2388 as a suppression alias, but prefer the current code for comparisons.
            (2282, Rule::BadVarName),
            (2238, Rule::RedirectToCommandName),
            (2259, Rule::SubshellTestGroup),
            (2266, Rule::OverwrittenFunction),
            (2270, Rule::AssignmentToNumericVariable),
            (2271, Rule::ElseWithoutThen),
            (2272, Rule::MissingSemicolonBeforeBrace),
            (2273, Rule::EmptyFunctionBody),
            (2274, Rule::BareClosingBrace),
            (2120, Rule::FunctionCalledWithoutArgs),
            (2364, Rule::FunctionReferencesUnsetParam),
            (2277, Rule::ExtglobInCasePattern),
            (2030, Rule::SubshellSideEffect),
            (2031, Rule::SubshellLocalAssignment),
            (2153, Rule::PossibleVariableMisspelling),
            (2100, Rule::AssignmentLooksLikeComparison),
            (2319, Rule::StatusCaptureAfterBranchTest),
            (2337, Rule::DollarQuestionAfterCommand),
            (2141, Rule::IfsSetToLiteralBackslashN),
            (2365, Rule::UnreachableAfterExit),
            (2370, Rule::UnusedHeredoc),
            (3010, Rule::DoubleBracketInSh),
            (3012, Rule::GreaterThanInDoubleBracket),
            (3014, Rule::TestEqualityOperator),
            (3015, Rule::RegexMatchInSh),
            (3016, Rule::VTestInSh),
            (3017, Rule::ATestInSh),
            (3062, Rule::OptionTestInSh),
            (3065, Rule::StickyBitTestInSh),
            (3067, Rule::OwnershipTestInSh),
        ];

        Self {
            map: {
                let mut map = FxHashMap::default();
                // Preserve the first-listed rule as the primary suppression target when
                // multiple rules intentionally share a ShellCheck code.
                for (sc_code, rule) in [
                    (1001, Rule::EscapedUnderscore),
                    (1002, Rule::EscapedUnderscoreLiteral),
                    (1003, Rule::SingleQuoteBackslash),
                    (1004, Rule::LiteralBackslash),
                    (1012, Rule::NeedlessBackslashUnderscore),
                    (2267, Rule::LiteralBackslashInSingleQuotes),
                    (2005, Rule::EchoedCommandSubstitution),
                    (2116, Rule::EchoInsideCommandSubstitution),
                    (2006, Rule::LegacyBackticks),
                    (2007, Rule::LegacyArithmeticExpansion),
                    (2003, Rule::ExprArithmetic),
                    (2126, Rule::GrepCountPipeline),
                    (2009, Rule::PsGrepPipeline),
                    (2010, Rule::LsGrepPipeline),
                    (2293, Rule::LsPipedToXargs),
                    (2117, Rule::SuWithoutFlag),
                    (2196, Rule::EgrepDeprecated),
                    (2139, Rule::CommandSubstitutionInAlias),
                    (2142, Rule::FunctionInAlias),
                    (2283, Rule::DoubleParenGrouping),
                    (2233, Rule::SingleTestSubshell),
                    (2235, Rule::SubshellTestGroup),
                    (2259, Rule::SubshellTestGroup),
                    (1037, Rule::PositionalTenBraces),
                    (1040, Rule::SpacedTabstripClose),
                    (1118, Rule::HeredocEndSpace),
                    (1042, Rule::MisquotedHeredocClose),
                    (1041, Rule::HeredocCloserNotAlone),
                    (1044, Rule::HeredocMissingEnd),
                    (1045, Rule::AmpersandSemicolon),
                    (1047, Rule::MissingFi),
                    (1065, Rule::FunctionParamsInSh),
                    (1069, Rule::IfBracketGlued),
                    (1070, Rule::ZshRedirPipe),
                    (1072, Rule::BrokenTestParse),
                    (1073, Rule::BrokenTestEnd),
                    (1075, Rule::ElseIf),
                    (1078, Rule::OpenDoubleQuote),
                    (1079, Rule::SuspectClosingQuote),
                    (1080, Rule::LinebreakInTest),
                    (1083, Rule::LiteralBraces),
                    (1090, Rule::DynamicSourcePath),
                    (1091, Rule::UntrackedSourceFile),
                    (1097, Rule::IfsEqualsAmbiguity),
                    (1101, Rule::BackslashBeforeClosingBacktick),
                    (1102, Rule::PositionalParamAsOperator),
                    (1110, Rule::UnicodeQuoteInString),
                    (1126, Rule::TrailingDirective),
                    (1113, Rule::TrailingDirective),
                    (2385, Rule::UnicodeSingleQuoteInSingleQuotes),
                    (2386, Rule::HeredocMissingEnd),
                    (2394, Rule::HeredocCloserNotAlone),
                    (2395, Rule::MisquotedHeredocClose),
                    (3002, Rule::ExtglobInSh),
                    (3061, Rule::ExtglobInSh),
                    (3026, Rule::CaretNegationInBracket),
                    (1127, Rule::CStyleComment),
                    (1129, Rule::ZshBraceIf),
                    (1130, Rule::ZshAlwaysBlock),
                    (1132, Rule::CPrototypeFragment),
                    (2254, Rule::ArrayIndexArithmetic),
                    (2297, Rule::DollarInArithmeticContext),
                    (2240, Rule::SourcedWithArgs),
                    (2251, Rule::ZshFlagExpansion),
                    (2252, Rule::NestedZshSubstitution),
                    (2313, Rule::ZshNestedExpansion),
                    (2275, Rule::MultiVarForLoop),
                    (2278, Rule::ZshPromptBracket),
                    (2279, Rule::CshSyntaxInSh),
                    (2282, Rule::BadVarName),
                    (2355, Rule::ZshAssignmentToZero),
                    (2359, Rule::ZshParameterFlag),
                    (2371, Rule::ZshArraySubscriptInCase),
                    (2375, Rule::ZshParameterIndexFlag),
                    (2164, Rule::UncheckedDirectoryChange),
                    (2263, Rule::RedundantSpacesInEcho),
                    (2143, Rule::GrepOutputInTest),
                    (2145, Rule::PositionalArgsInString),
                    (2198, Rule::AtSignInStringCompare),
                    (2199, Rule::ArraySliceInComparison),
                    (2124, Rule::QuotedArraySlice),
                    (2128, Rule::QuotedBashSource),
                    (2294, Rule::LsInSubstitution),
                    (2291, Rule::UnquotedVariableInSed),
                    (2026, Rule::UnquotedWordBetweenQuotes),
                    (2027, Rule::DoubleQuoteNesting),
                    (2379, Rule::EnvPrefixQuoting),
                    (2140, Rule::MixedQuoteWord),
                    (2060, Rule::UnquotedTrClass),
                    (2335, Rule::UnquotedPathInMkdir),
                    (2070, Rule::UnquotedVariableInTest),
                    (2223, Rule::DefaultValueInColonAssign),
                    (2021, Rule::UnquotedTrRange),
                    (2186, Rule::DeprecatedTempfileCommand),
                    (2258, Rule::BareRead),
                    (3024, Rule::PlusEqualsAppend),
                    (3071, Rule::PlusEqualsInSh),
                    (3034, Rule::BashFileSlurp),
                    (3037, Rule::EchoFlags),
                    (2018, Rule::TrLowerRange),
                    (2019, Rule::TrUpperRange),
                    (2028, Rule::EchoBackslashEscapes),
                    (3025, Rule::PrintfQFormatInSh),
                    (3052, Rule::AmpersandRedirection),
                    (3050, Rule::BraceFdRedirection),
                    (3077, Rule::BasePrefixInArithmetic),
                    (3070, Rule::AmpersandRedirectInSh),
                    (3073, Rule::PipeStderrInSh),
                    (2016, Rule::SingleQuotedLiteral),
                    (2030, Rule::SubshellSideEffect),
                    (2031, Rule::SubshellLocalAssignment),
                    (2153, Rule::PossibleVariableMisspelling),
                    (2013, Rule::LineOrientedInput),
                    (2015, Rule::ChainedTestBranches),
                    (2014, Rule::UnquotedGlobsInFind),
                    (2296, Rule::ShortCircuitFallthrough),
                    (2231, Rule::GlobWithExpansionInLoop),
                    (2022, Rule::GlobInGrepPattern),
                    (2062, Rule::UnquotedGrepRegex),
                    (2053, Rule::GlobInStringComparison),
                    (2061, Rule::GlobInFindSubstitution),
                    (1019, Rule::EmptyTest),
                    (2024, Rule::SudoRedirectionOrder),
                    (2034, Rule::UnusedAssignment),
                    (2035, Rule::LeadingGlobArgument),
                    (2043, Rule::SingleIterationLoop),
                    (2209, Rule::ConditionalAssignmentShortcut),
                    (2044, Rule::FindOutputLoop),
                    (2348, Rule::FindOutputLoop),
                    (2380, Rule::MisspelledOptionName),
                    (2045, Rule::LoopFromCommandOutput),
                    (2046, Rule::UnquotedCommandSubstitution),
                    (2048, Rule::UnquotedDollarStar),
                    (2059, Rule::PrintfFormatVariable),
                    (2029, Rule::SshLocalExpansion),
                    (2352, Rule::DefaultElseInShortCircuit),
                    (3043, Rule::LocalVariableInSh),
                    (3001, Rule::ProcessSubstitution),
                    (3003, Rule::AnsiCQuoting),
                    (3004, Rule::DollarStringInSh),
                    (3009, Rule::BraceExpansion),
                    (3011, Rule::HereString),
                    (3030, Rule::ArrayAssignment),
                    (3053, Rule::IndirectExpansion),
                    (3079, Rule::UnsetPatternInSh),
                    (3083, Rule::NestedDefaultExpansion),
                    (3078, Rule::ArrayKeysInSh),
                    (2219, Rule::AvoidLetBuiltin),
                    (2320, Rule::UnquotedPipeInEcho),
                    (2321, Rule::ArrayIndexArithmetic),
                    (2323, Rule::ArithmeticScoreLine),
                    (3028, Rule::ArrayReference),
                    (3054, Rule::ArrayReference),
                    (3057, Rule::SubstringExpansion),
                    (3059, Rule::CaseModificationExpansion),
                    (3060, Rule::ReplacementExpansion),
                    (2038, Rule::FindOutputToXargs),
                    (2064, Rule::TrapStringExpansion),
                    (2066, Rule::QuotedDollarStarLoop),
                    (2206, Rule::UnquotedArraySplit),
                    (2207, Rule::CommandOutputArraySplit),
                    (2366, Rule::BacktickOutputToCommand),
                    (2068, Rule::UnquotedArrayExpansion),
                    (2076, Rule::QuotedBashRegex),
                    (2086, Rule::UnquotedExpansion),
                    (2146, Rule::FindOrWithoutGrouping),
                    (2332, Rule::FindOrWithoutGrouping),
                    (2121, Rule::SetFlagsWithoutDashes),
                    (2324, Rule::SetFlagsWithoutDashes),
                    (2184, Rule::UnsetAssociativeArrayElement),
                    (2178, Rule::ArrayToStringConversion),
                    (2115, Rule::RmGlobOnVariablePath),
                    (2104, Rule::LoopControlOutsideLoop),
                    (2112, Rule::FunctionKeyword),
                    (2125, Rule::GlobAssignedToVariable),
                    (2216, Rule::PipeToKill),
                    (2148, Rule::MissingShebangLine),
                    (2285, Rule::MissingShebangLine),
                    (3005, Rule::CStyleForInSh),
                    (3006, Rule::StandaloneArithmetic),
                    (3007, Rule::LegacyArithmeticInSh),
                    (3008, Rule::SelectLoop),
                    (3018, Rule::CStyleForArithmeticInSh),
                    (3032, Rule::Coproc),
                    (3033, Rule::SelectLoop),
                    (3039, Rule::LetCommand),
                    (3042, Rule::LetCommand),
                    (3047, Rule::TrapErr),
                    (3040, Rule::PipefailOption),
                    (3048, Rule::WaitOption),
                    (3044, Rule::DeclareCommand),
                    (3046, Rule::SourceBuiltinInSh),
                    (3058, Rule::BashCaseFallthrough),
                    (2127, Rule::BashCaseFallthrough),
                    (3085, Rule::StarGlobRemovalInSh),
                    (3063, Rule::CStyleForInSh),
                    (3064, Rule::LegacyArithmeticInSh),
                    (3069, Rule::CStyleForArithmeticInSh),
                    (3075, Rule::ErrexitTrapInSh),
                    (3076, Rule::SignalNameInTrap),
                    (2323, Rule::ArithmeticScoreLine),
                    (2399, Rule::BrokenAssocKey),
                    (2054, Rule::CommaArrayElements),
                    (2336, Rule::AppendToArrayAsString),
                    (2339, Rule::MapfileProcessSubstitution),
                    (3051, Rule::SourceInsideFunctionInSh),
                    (3084, Rule::SourceInsideFunctionInSh),
                    (2155, Rule::ExportCommandSubstitution),
                    (2156, Rule::FindExecDirWithShell),
                    (2157, Rule::ConstantComparisonTest),
                    (2158, Rule::LiteralUnaryStringTest),
                    (2162, Rule::ReadWithoutRaw),
                    (2078, Rule::TruthyLiteralTest),
                    (2168, Rule::LocalTopLevel),
                    (2194, Rule::ConstantCaseSubject),
                    (2210, Rule::BadRedirectionFdOrder),
                    (2217, Rule::EchoHereDoc),
                    (2154, Rule::UndefinedVariable),
                    (2148, Rule::MissingShebangLine),
                    (2239, Rule::NonAbsoluteShebang),
                    (2286, Rule::IndentedShebang),
                    (2287, Rule::SpaceAfterHashBang),
                    (2288, Rule::TemplateBraceInCommand),
                    (2289, Rule::CommentedContinuationLine),
                    (1133, Rule::LinebreakBeforeAnd),
                    (2290, Rule::SubshellInArithmetic),
                    (2292, Rule::DollarInArithmetic),
                    (2302, Rule::EscapedNegationInTest),
                    (2308, Rule::GreaterThanInTest),
                    (2309, Rule::StringComparisonForVersion),
                    (2310, Rule::MixedAndOrInCondition),
                    (2311, Rule::QuotedCommandInTest),
                    (2312, Rule::GlobInTestComparison),
                    (2314, Rule::TildeInStringComparison),
                    (2315, Rule::IfDollarCommand),
                    (2316, Rule::BacktickInCommandPosition),
                    (2294, Rule::EvalOnArray),
                    (2333, Rule::NonShellSyntaxInScript),
                    (2163, Rule::ExportWithPositionalParams),
                    (2334, Rule::ExportWithPositionalParams),
                    (2389, Rule::LoopWithoutEnd),
                    (2390, Rule::MissingDoneInForLoop),
                    (2391, Rule::DanglingElse),
                    (2392, Rule::LinebreakBeforeAnd),
                    (2393, Rule::SpacedTabstripClose),
                    (2396, Rule::UntilMissingDo),
                    (2397, Rule::AmpersandSemicolon),
                    (2221, Rule::CaseGlobReachability),
                    (2222, Rule::CaseDefaultBeforeGlob),
                    (2213, Rule::GetoptsOptionNotInCase),
                    (2214, Rule::CaseArmNotInGetopts),
                    (2241, Rule::InvalidExitStatus),
                    (2242, Rule::CasePatternVar),
                    (2248, Rule::BareSlashMarker),
                    (2257, Rule::ArithmeticRedirectionTarget),
                    (2264, Rule::NestedParameterExpansion),
                    (2250, Rule::PatternWithVariable),
                    (2255, Rule::SubstWithRedirect),
                    (2256, Rule::SubstWithRedirectErr),
                    (2089, Rule::AppendWithEscapedQuotes),
                    (2276, Rule::PlusPrefixInAssignment),
                    (2238, Rule::RedirectToCommandName),
                    (2266, Rule::OverwrittenFunction),
                    (2270, Rule::AssignmentToNumericVariable),
                    (2318, Rule::LocalCrossReference),
                    (2271, Rule::ElseWithoutThen),
                    (2272, Rule::MissingSemicolonBeforeBrace),
                    (2273, Rule::EmptyFunctionBody),
                    (2274, Rule::BareClosingBrace),
                    (2120, Rule::FunctionCalledWithoutArgs),
                    (2364, Rule::FunctionReferencesUnsetParam),
                    (2277, Rule::ExtglobInCasePattern),
                    (2100, Rule::AssignmentLooksLikeComparison),
                    (2319, Rule::StatusCaptureAfterBranchTest),
                    (2337, Rule::DollarQuestionAfterCommand),
                    (2141, Rule::IfsSetToLiteralBackslashN),
                    (2365, Rule::UnreachableAfterExit),
                    (2370, Rule::UnusedHeredoc),
                    (3010, Rule::DoubleBracketInSh),
                    (3012, Rule::GreaterThanInDoubleBracket),
                    (3014, Rule::TestEqualityOperator),
                    (3015, Rule::RegexMatchInSh),
                    (3016, Rule::VTestInSh),
                    (3017, Rule::ATestInSh),
                    (2338, Rule::UnsetAssociativeArrayElement),
                    (2381, Rule::ArrayToStringConversion),
                    (3062, Rule::OptionTestInSh),
                    (3065, Rule::StickyBitTestInSh),
                    (3067, Rule::OwnershipTestInSh),
                    (3074, Rule::HyphenatedFunctionName),
                ] {
                    map.entry(sc_code).or_insert(rule);
                }
                map
            },
            aliases: vec![
                (1040, Rule::HeredocEndSpace),
                (1075, Rule::ExtglobCase),
                (2321, Rule::FunctionKeywordInSh),
                (3024, Rule::BashFileSlurp),
                (3055, Rule::PlusEqualsAppend),
                (3055, Rule::ArrayKeysInSh),
                (3058, Rule::StarGlobRemovalInSh),
                (3024, Rule::PlusEqualsInSh),
                (3062, Rule::DollarStringInSh),
                (3056, Rule::UnsetPatternInSh),
                (3072, Rule::CaretNegationInBracket),
                (3033, Rule::HyphenatedFunctionName),
                (2009, Rule::DoubleParenGrouping),
                (2294, Rule::LsInSubstitution),
                (2294, Rule::EvalOnArray),
                (2373, Rule::CaseGlobReachability),
                (2374, Rule::CaseDefaultBeforeGlob),
                (2372, Rule::SingleLetterCaseLabel),
                (2382, Rule::GetoptsOptionNotInCase),
                (2383, Rule::CaseArmNotInGetopts),
                // The pinned ShellCheck oracle still reports ordinary `A && B || C`
                // fallthrough chains as SC2015. Keep that older code as a
                // compatibility alias so targeted large-corpus validation can
                // compare C079 against the actual oracle output.
                (2015, Rule::ShortCircuitFallthrough),
                (2114, Rule::ConditionalAssignmentShortcut),
                (2165, Rule::SingleIterationLoop),
                (2322, Rule::SuWithoutFlag),
                (2340, Rule::DeprecatedTempfileCommand),
                (2342, Rule::EgrepDeprecated),
                (2328, Rule::CommandSubstitutionInAlias),
                (2330, Rule::FunctionInAlias),
                (2376, Rule::DoubleQuoteNesting),
                (2298, Rule::UnquotedTrRange),
                (2303, Rule::UnquotedTrClass),
                // ShellCheck 0.11.0 currently emits SC2057 for leading `\!` test operators.
                // Keep SC2302 as the authored rule code and accept the oracle's live code too.
                (2057, Rule::EscapedNegationInTest),
                (2073, Rule::GreaterThanInTest),
                (2072, Rule::StringComparisonForVersion),
                (2081, Rule::GlobInTestComparison),
                (2088, Rule::TildeInStringComparison),
                (2091, Rule::IfDollarCommand),
                (2092, Rule::BacktickInCommandPosition),
                (2307, Rule::UnquotedVariableInTest),
                (2300, Rule::UnquotedWordBetweenQuotes),
                (2346, Rule::DefaultValueInColonAssign),
                (2319, Rule::AssignmentLooksLikeComparison),
                (2329, Rule::IfsSetToLiteralBackslashN),
                (2353, Rule::AssignmentToNumericVariable),
                (2354, Rule::PlusPrefixInAssignment),
                (2377, Rule::AppendWithEscapedQuotes),
                (2367, Rule::UncheckedDirectoryChangeInFunction),
                (2368, Rule::ContinueOutsideLoopInFunction),
                (2378, Rule::VariableAsCommandName),
                (2384, Rule::LocalCrossReference),
                (2387, Rule::SpacedAssignment),
                (2388, Rule::BadVarName),
                (2344, Rule::AtSignInStringCompare),
                (2345, Rule::ArraySliceInComparison),
                (2325, Rule::QuotedArraySlice),
                (2327, Rule::QuotedBashSource),
                (2398, Rule::KeywordFunctionName),
                // Preserve SC2290 for suppressing C139 without taking over the large-corpus
                // comparison slot that already belongs to C077.
                (2290, Rule::SpacedAssignment),
                (2295, Rule::UnquotedGlobsInFind),
                (2299, Rule::GlobInGrepPattern),
                (2301, Rule::GlobInStringComparison),
                (2304, Rule::GlobInFindSubstitution),
                (2305, Rule::UnquotedGrepRegex),
                (2349, Rule::GlobWithExpansionInLoop),
                (2326, Rule::GlobAssignedToVariable),
                (2060, Rule::UnquotedTrRange),
                (2280, Rule::IfsEqualsAmbiguity),
                (2270, Rule::IfMissingThen),
                (2127, Rule::EchoHereDoc),
            ],
            comparison,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_codes_and_ignores_unknown_ones() {
        let map = ShellCheckCodeMap::default();

        assert_eq!(map.resolve("SC2034"), Some(Rule::UnusedAssignment));
        assert_eq!(map.resolve("SC2005"), Some(Rule::EchoedCommandSubstitution));
        assert_eq!(
            map.resolve("SC2116"),
            Some(Rule::EchoInsideCommandSubstitution)
        );
        assert_eq!(map.resolve("SC2143"), Some(Rule::GrepOutputInTest));
        assert_eq!(map.resolve("SC2145"), Some(Rule::PositionalArgsInString));
        assert_eq!(map.resolve("SC2006"), Some(Rule::LegacyBackticks));
        assert_eq!(map.resolve("SC2007"), Some(Rule::LegacyArithmeticExpansion));
        assert_eq!(map.resolve("SC2003"), Some(Rule::ExprArithmetic));
        assert_eq!(map.resolve("SC2219"), Some(Rule::AvoidLetBuiltin));
        assert_eq!(map.resolve("SC2126"), Some(Rule::GrepCountPipeline));
        assert_eq!(map.resolve("SC2009"), Some(Rule::PsGrepPipeline));
        assert_eq!(map.resolve("SC2010"), Some(Rule::LsGrepPipeline));
        assert_eq!(map.resolve("SC2293"), Some(Rule::LsPipedToXargs));
        assert_eq!(map.resolve("SC2294"), Some(Rule::LsInSubstitution));
        assert_eq!(map.resolve("SC2048"), Some(Rule::UnquotedDollarStar));
        assert_eq!(map.resolve("2048"), Some(Rule::UnquotedDollarStar));
        assert_eq!(map.resolve("SC2198"), Some(Rule::AtSignInStringCompare));
        assert_eq!(map.resolve("SC2199"), Some(Rule::ArraySliceInComparison));
        assert_eq!(map.resolve("SC2344"), Some(Rule::AtSignInStringCompare));
        assert_eq!(map.resolve("SC2345"), Some(Rule::ArraySliceInComparison));
        assert_eq!(map.resolve("SC2124"), Some(Rule::QuotedArraySlice));
        assert_eq!(map.resolve("SC2325"), Some(Rule::QuotedArraySlice));
        assert_eq!(map.resolve("SC2128"), Some(Rule::QuotedBashSource));
        assert_eq!(map.resolve("SC2327"), Some(Rule::QuotedBashSource));
        assert_eq!(
            map.resolve_all("SC2009"),
            vec![Rule::PsGrepPipeline, Rule::DoubleParenGrouping]
        );
        assert_eq!(map.resolve("SC2233"), Some(Rule::SingleTestSubshell));
        assert_eq!(map.resolve("SC2235"), Some(Rule::SubshellTestGroup));
        assert_eq!(map.resolve("SC2259"), Some(Rule::SubshellTestGroup));
        assert_eq!(map.resolve("SC1037"), Some(Rule::PositionalTenBraces));
        assert_eq!(map.resolve("SC1040"), Some(Rule::SpacedTabstripClose));
        assert_eq!(map.resolve("SC2393"), Some(Rule::SpacedTabstripClose));
        assert_eq!(
            map.resolve_all("SC1040"),
            vec![Rule::SpacedTabstripClose, Rule::HeredocEndSpace]
        );
        assert_eq!(map.resolve("SC1118"), Some(Rule::HeredocEndSpace));
        assert_eq!(map.resolve("SC1042"), Some(Rule::MisquotedHeredocClose));
        assert_eq!(map.resolve("SC1041"), Some(Rule::HeredocCloserNotAlone));
        assert_eq!(map.resolve("SC1044"), Some(Rule::HeredocMissingEnd));
        assert_eq!(map.resolve("SC1001"), Some(Rule::EscapedUnderscore));
        assert_eq!(map.resolve("SC1002"), Some(Rule::EscapedUnderscoreLiteral));
        assert_eq!(map.resolve("SC1003"), Some(Rule::SingleQuoteBackslash));
        assert_eq!(map.resolve("SC1004"), Some(Rule::LiteralBackslash));
        assert_eq!(
            map.resolve("SC1012"),
            Some(Rule::NeedlessBackslashUnderscore)
        );
        assert_eq!(
            map.resolve("SC2267"),
            Some(Rule::LiteralBackslashInSingleQuotes)
        );
        assert_eq!(map.resolve("SC1047"), Some(Rule::MissingFi));
        assert_eq!(map.resolve("SC1069"), Some(Rule::IfBracketGlued));
        assert_eq!(map.resolve("SC1070"), Some(Rule::ZshRedirPipe));
        assert_eq!(map.resolve("SC1072"), Some(Rule::BrokenTestParse));
        assert_eq!(map.resolve("SC1073"), Some(Rule::BrokenTestEnd));
        assert_eq!(map.resolve("SC1075"), Some(Rule::ElseIf));
        assert_eq!(
            map.resolve_all("SC1075"),
            vec![Rule::ElseIf, Rule::ExtglobCase]
        );
        assert_eq!(map.resolve("SC3002"), Some(Rule::ExtglobInSh));
        assert_eq!(map.resolve("SC3061"), Some(Rule::ExtglobInSh));
        assert_eq!(map.resolve("SC2258"), Some(Rule::BareRead));
        assert_eq!(map.resolve("SC2291"), Some(Rule::UnquotedVariableInSed));
        assert_eq!(map.resolve("SC2322"), Some(Rule::SuWithoutFlag));
        assert_eq!(map.resolve("SC2117"), Some(Rule::SuWithoutFlag));
        assert_eq!(map.resolve("SC2340"), Some(Rule::DeprecatedTempfileCommand));
        assert_eq!(map.resolve("SC2186"), Some(Rule::DeprecatedTempfileCommand));
        assert_eq!(map.resolve("SC2342"), Some(Rule::EgrepDeprecated));
        assert_eq!(map.resolve("SC2196"), Some(Rule::EgrepDeprecated));
        assert_eq!(
            map.resolve("SC2139"),
            Some(Rule::CommandSubstitutionInAlias)
        );
        assert_eq!(map.resolve("SC2142"), Some(Rule::FunctionInAlias));
        assert_eq!(map.resolve("SC2303"), Some(Rule::UnquotedTrClass));
        assert_eq!(map.resolve("SC2335"), Some(Rule::UnquotedPathInMkdir));
        assert_eq!(map.resolve("SC2307"), Some(Rule::UnquotedVariableInTest));
        assert_eq!(map.resolve("SC2379"), Some(Rule::EnvPrefixQuoting));
        assert_eq!(map.resolve("SC2140"), Some(Rule::MixedQuoteWord));
        assert_eq!(map.resolve("SC2030"), Some(Rule::SubshellSideEffect));
        assert_eq!(map.resolve("SC2031"), Some(Rule::SubshellLocalAssignment));
        assert_eq!(map.resolve("SC2320"), Some(Rule::UnquotedPipeInEcho));
        assert_eq!(
            map.resolve("SC2337"),
            Some(Rule::DollarQuestionAfterCommand)
        );
        assert_eq!(map.resolve("SC2223"), Some(Rule::DefaultValueInColonAssign));
        assert_eq!(map.resolve("SC2346"), Some(Rule::DefaultValueInColonAssign));
        assert_eq!(map.resolve("SC2298"), Some(Rule::UnquotedTrRange));
        assert_eq!(map.resolve("SC2021"), Some(Rule::UnquotedTrRange));
        assert_eq!(map.resolve("SC2060"), Some(Rule::UnquotedTrClass));
        assert_eq!(map.resolve("SC2070"), Some(Rule::UnquotedVariableInTest));
        assert_eq!(map.resolve("SC2066"), Some(Rule::QuotedDollarStarLoop));
        assert_eq!(map.resolve("SC2206"), Some(Rule::UnquotedArraySplit));
        assert_eq!(map.resolve("SC2207"), Some(Rule::CommandOutputArraySplit));
        assert_eq!(map.resolve("SC2366"), Some(Rule::BacktickOutputToCommand));
        assert_eq!(map.resolve_all("SC2198"), vec![Rule::AtSignInStringCompare]);
        assert_eq!(
            map.resolve_all("SC2199"),
            vec![Rule::ArraySliceInComparison]
        );
        assert_eq!(map.resolve_all("SC2344"), vec![Rule::AtSignInStringCompare]);
        assert_eq!(
            map.resolve_all("SC2345"),
            vec![Rule::ArraySliceInComparison]
        );
        assert_eq!(map.resolve_all("SC2124"), vec![Rule::QuotedArraySlice]);
        assert_eq!(map.resolve_all("SC2325"), vec![Rule::QuotedArraySlice]);
        assert_eq!(map.resolve_all("SC2128"), vec![Rule::QuotedBashSource]);
        assert_eq!(map.resolve_all("SC2327"), vec![Rule::QuotedBashSource]);
        assert_eq!(map.resolve("SC2293"), Some(Rule::LsPipedToXargs));
        assert_eq!(map.resolve("SC2294"), Some(Rule::LsInSubstitution));
        assert_eq!(map.resolve("SC2263"), Some(Rule::RedundantSpacesInEcho));
        assert_eq!(map.resolve("SC3026"), Some(Rule::CaretNegationInBracket));
        assert_eq!(map.resolve("SC3072"), Some(Rule::CaretNegationInBracket));
        assert_eq!(map.resolve_all("SC3002"), vec![Rule::ExtglobInSh]);
        assert_eq!(map.resolve_all("SC3061"), vec![Rule::ExtglobInSh]);
        assert_eq!(map.resolve_all("SC2258"), vec![Rule::BareRead]);
        assert_eq!(map.resolve_all("SC2291"), vec![Rule::UnquotedVariableInSed]);
        assert_eq!(
            map.resolve_all("SC2145"),
            vec![Rule::PositionalArgsInString]
        );
        assert_eq!(map.resolve_all("SC2320"), vec![Rule::UnquotedPipeInEcho]);
        assert_eq!(map.resolve_all("SC2322"), vec![Rule::SuWithoutFlag]);
        assert_eq!(map.resolve_all("SC2117"), vec![Rule::SuWithoutFlag]);
        assert_eq!(
            map.resolve_all("SC2340"),
            vec![Rule::DeprecatedTempfileCommand]
        );
        assert_eq!(
            map.resolve_all("SC2186"),
            vec![Rule::DeprecatedTempfileCommand]
        );
        assert_eq!(map.resolve_all("SC2342"), vec![Rule::EgrepDeprecated]);
        assert_eq!(map.resolve_all("SC2196"), vec![Rule::EgrepDeprecated]);
        assert_eq!(
            map.resolve_all("SC2139"),
            vec![Rule::CommandSubstitutionInAlias]
        );
        assert_eq!(map.resolve_all("SC2142"), vec![Rule::FunctionInAlias]);
        assert_eq!(
            map.resolve_all("SC2328"),
            vec![Rule::CommandSubstitutionInAlias]
        );
        assert_eq!(map.resolve_all("SC2330"), vec![Rule::FunctionInAlias]);
        assert_eq!(map.resolve_all("SC2303"), vec![Rule::UnquotedTrClass]);
        assert_eq!(map.resolve_all("SC2335"), vec![Rule::UnquotedPathInMkdir]);
        assert_eq!(
            map.resolve_all("SC2307"),
            vec![Rule::UnquotedVariableInTest]
        );
        assert_eq!(map.resolve_all("SC2379"), vec![Rule::EnvPrefixQuoting]);
        assert_eq!(map.resolve_all("SC2140"), vec![Rule::MixedQuoteWord]);
        assert_eq!(
            map.resolve_all("SC2223"),
            vec![Rule::DefaultValueInColonAssign]
        );
        assert_eq!(
            map.resolve_all("SC2346"),
            vec![Rule::DefaultValueInColonAssign]
        );
        assert_eq!(map.resolve_all("SC2298"), vec![Rule::UnquotedTrRange]);
        assert_eq!(map.resolve_all("SC2021"), vec![Rule::UnquotedTrRange]);
        assert_eq!(
            map.resolve_all("SC2060"),
            vec![Rule::UnquotedTrClass, Rule::UnquotedTrRange]
        );
        assert_eq!(
            map.resolve_all("SC2070"),
            vec![Rule::UnquotedVariableInTest]
        );
        assert_eq!(map.resolve_all("SC2066"), vec![Rule::QuotedDollarStarLoop]);
        assert_eq!(map.resolve_all("SC2206"), vec![Rule::UnquotedArraySplit]);
        assert_eq!(
            map.resolve_all("SC2207"),
            vec![Rule::CommandOutputArraySplit]
        );
        assert_eq!(
            map.resolve_all("SC2366"),
            vec![Rule::BacktickOutputToCommand]
        );
        assert_eq!(map.resolve_all("SC2293"), vec![Rule::LsPipedToXargs]);
        assert_eq!(
            map.resolve_all("SC2294"),
            vec![Rule::LsInSubstitution, Rule::EvalOnArray]
        );
        assert_eq!(map.resolve_all("SC2048"), vec![Rule::UnquotedDollarStar]);
        assert_eq!(map.resolve_all("2048"), vec![Rule::UnquotedDollarStar]);
        assert_eq!(map.resolve_all("SC2263"), vec![Rule::RedundantSpacesInEcho]);
        assert_eq!(
            map.resolve_all("SC3026"),
            vec![Rule::CaretNegationInBracket]
        );
        assert_eq!(
            map.resolve_all("SC3072"),
            vec![Rule::CaretNegationInBracket]
        );
        assert_eq!(map.resolve("SC1078"), Some(Rule::OpenDoubleQuote));
        assert_eq!(map.resolve("SC1079"), Some(Rule::SuspectClosingQuote));
        assert_eq!(map.resolve("SC1083"), Some(Rule::LiteralBraces));
        assert_eq!(map.resolve("SC1080"), Some(Rule::LinebreakInTest));
        assert_eq!(map.resolve("SC1090"), Some(Rule::DynamicSourcePath));
        assert_eq!(map.resolve("SC1091"), Some(Rule::UntrackedSourceFile));
        assert_eq!(map.resolve("SC1097"), Some(Rule::IfsEqualsAmbiguity));
        assert_eq!(
            map.resolve("SC1101"),
            Some(Rule::BackslashBeforeClosingBacktick)
        );
        assert_eq!(map.resolve("SC1102"), Some(Rule::PositionalParamAsOperator));
        assert_eq!(map.resolve("SC1110"), Some(Rule::UnicodeQuoteInString));
        assert_eq!(map.resolve("SC1113"), Some(Rule::TrailingDirective));
        assert_eq!(map.resolve("SC1126"), Some(Rule::TrailingDirective));
        assert_eq!(map.resolve("SC2290"), Some(Rule::SubshellInArithmetic));
        assert_eq!(
            map.resolve_all("SC2290"),
            vec![Rule::SubshellInArithmetic, Rule::SpacedAssignment]
        );
        assert_eq!(
            map.resolve("SC2385"),
            Some(Rule::UnicodeSingleQuoteInSingleQuotes)
        );
        assert_eq!(map.resolve("SC2386"), Some(Rule::HeredocMissingEnd));
        assert_eq!(map.resolve("SC1129"), Some(Rule::ZshBraceIf));
        assert_eq!(map.resolve("SC1127"), Some(Rule::CStyleComment));
        assert_eq!(map.resolve("SC1130"), Some(Rule::ZshAlwaysBlock));
        assert_eq!(map.resolve("SC1132"), Some(Rule::CPrototypeFragment));
        assert_eq!(map.resolve("SC2240"), Some(Rule::SourcedWithArgs));
        assert_eq!(map.resolve("SC2251"), Some(Rule::ZshFlagExpansion));
        assert_eq!(map.resolve("SC2252"), Some(Rule::NestedZshSubstitution));
        assert_eq!(map.resolve("SC2313"), Some(Rule::ZshNestedExpansion));
        assert_eq!(map.resolve("SC2275"), Some(Rule::MultiVarForLoop));
        assert_eq!(map.resolve("SC2278"), Some(Rule::ZshPromptBracket));
        assert_eq!(map.resolve("SC2279"), Some(Rule::CshSyntaxInSh));
        assert_eq!(map.resolve("SC2355"), Some(Rule::ZshAssignmentToZero));
        assert_eq!(map.resolve("SC2359"), Some(Rule::ZshParameterFlag));
        assert_eq!(map.resolve("SC2371"), Some(Rule::ZshArraySubscriptInCase));
        assert_eq!(map.resolve("SC2375"), Some(Rule::ZshParameterIndexFlag));
        assert_eq!(map.resolve("SC2164"), Some(Rule::UncheckedDirectoryChange));
        assert_eq!(
            map.resolve("SC2367"),
            Some(Rule::UncheckedDirectoryChangeInFunction)
        );
        assert_eq!(
            map.resolve("SC2368"),
            Some(Rule::ContinueOutsideLoopInFunction)
        );
        assert_eq!(map.resolve("SC3052"), Some(Rule::AmpersandRedirection));
        assert_eq!(map.resolve("SC3058"), Some(Rule::BashCaseFallthrough));
        assert_eq!(map.resolve("SC2127"), Some(Rule::BashCaseFallthrough));
        assert_eq!(
            map.resolve_all("SC2127"),
            vec![Rule::BashCaseFallthrough, Rule::EchoHereDoc]
        );
        assert_eq!(map.resolve("SC3085"), Some(Rule::StarGlobRemovalInSh));
        assert_eq!(
            map.resolve_all("SC3058"),
            vec![Rule::BashCaseFallthrough, Rule::StarGlobRemovalInSh]
        );
        assert_eq!(map.resolve("SC3050"), Some(Rule::BraceFdRedirection));
        assert_eq!(map.resolve("SC3070"), Some(Rule::AmpersandRedirectInSh));
        assert_eq!(map.resolve("SC3073"), Some(Rule::PipeStderrInSh));
        assert_eq!(map.resolve("SC2016"), Some(Rule::SingleQuotedLiteral));
        assert_eq!(map.resolve("SC2013"), Some(Rule::LineOrientedInput));
        assert_eq!(map.resolve("SC2015"), Some(Rule::ChainedTestBranches));
        assert_eq!(map.resolve("SC2014"), Some(Rule::UnquotedGlobsInFind));
        assert_eq!(map.resolve("SC2295"), Some(Rule::UnquotedGlobsInFind));
        assert_eq!(map.resolve("SC2231"), Some(Rule::GlobWithExpansionInLoop));
        assert_eq!(map.resolve("SC2349"), Some(Rule::GlobWithExpansionInLoop));
        assert_eq!(map.resolve("SC2022"), Some(Rule::GlobInGrepPattern));
        assert_eq!(map.resolve("SC2299"), Some(Rule::GlobInGrepPattern));
        assert_eq!(map.resolve("SC2062"), Some(Rule::UnquotedGrepRegex));
        assert_eq!(map.resolve("SC2305"), Some(Rule::UnquotedGrepRegex));
        assert_eq!(map.resolve("SC2053"), Some(Rule::GlobInStringComparison));
        assert_eq!(map.resolve("SC2301"), Some(Rule::GlobInStringComparison));
        assert_eq!(map.resolve("SC2061"), Some(Rule::GlobInFindSubstitution));
        assert_eq!(map.resolve("SC2304"), Some(Rule::GlobInFindSubstitution));
        assert_eq!(map.resolve("SC2125"), Some(Rule::GlobAssignedToVariable));
        assert_eq!(map.resolve("SC2326"), Some(Rule::GlobAssignedToVariable));
        assert_eq!(map.resolve_all("SC2014"), vec![Rule::UnquotedGlobsInFind]);
        assert_eq!(map.resolve_all("SC2295"), vec![Rule::UnquotedGlobsInFind]);
        assert_eq!(
            map.resolve_all("SC2015"),
            vec![Rule::ChainedTestBranches, Rule::ShortCircuitFallthrough]
        );
        assert_eq!(map.resolve("SC2296"), Some(Rule::ShortCircuitFallthrough));
        assert_eq!(
            map.resolve_all("SC2231"),
            vec![Rule::GlobWithExpansionInLoop]
        );
        assert_eq!(
            map.resolve_all("SC2349"),
            vec![Rule::GlobWithExpansionInLoop]
        );
        assert_eq!(map.resolve_all("SC2022"), vec![Rule::GlobInGrepPattern]);
        assert_eq!(map.resolve_all("SC2299"), vec![Rule::GlobInGrepPattern]);
        assert_eq!(map.resolve_all("SC2062"), vec![Rule::UnquotedGrepRegex]);
        assert_eq!(map.resolve_all("SC2305"), vec![Rule::UnquotedGrepRegex]);
        assert_eq!(
            map.resolve_all("SC2053"),
            vec![Rule::GlobInStringComparison]
        );
        assert_eq!(
            map.resolve_all("SC2301"),
            vec![Rule::GlobInStringComparison]
        );
        assert_eq!(
            map.resolve_all("SC2061"),
            vec![Rule::GlobInFindSubstitution]
        );
        assert_eq!(
            map.resolve_all("SC2304"),
            vec![Rule::GlobInFindSubstitution]
        );
        assert_eq!(
            map.resolve_all("SC2125"),
            vec![Rule::GlobAssignedToVariable]
        );
        assert_eq!(
            map.resolve_all("SC2326"),
            vec![Rule::GlobAssignedToVariable]
        );
        assert_eq!(map.resolve("SC1019"), Some(Rule::EmptyTest));
        assert_eq!(map.resolve("SC1045"), Some(Rule::AmpersandSemicolon));
        assert_eq!(map.resolve("SC2024"), Some(Rule::SudoRedirectionOrder));
        assert_eq!(map.resolve("SC2035"), Some(Rule::LeadingGlobArgument));
        assert_eq!(map.resolve("SC2043"), Some(Rule::SingleIterationLoop));
        assert_eq!(
            map.resolve("SC2209"),
            Some(Rule::ConditionalAssignmentShortcut)
        );
        assert_eq!(map.resolve("SC2044"), Some(Rule::FindOutputLoop));
        assert_eq!(map.resolve("SC2348"), Some(Rule::FindOutputLoop));
        assert_eq!(map.resolve("SC2380"), Some(Rule::MisspelledOptionName));
        assert_eq!(map.resolve("SC2045"), Some(Rule::LoopFromCommandOutput));
        assert_eq!(
            map.resolve("SC2046"),
            Some(Rule::UnquotedCommandSubstitution)
        );
        assert_eq!(map.resolve("SC2048"), Some(Rule::UnquotedDollarStar));
        assert_eq!(map.resolve("SC2059"), Some(Rule::PrintfFormatVariable));
        assert_eq!(
            map.resolve("SC2114"),
            Some(Rule::ConditionalAssignmentShortcut)
        );
        assert_eq!(map.resolve("SC2165"), Some(Rule::SingleIterationLoop));
        assert_eq!(map.resolve("SC2352"), Some(Rule::DefaultElseInShortCircuit));
        assert_eq!(map.resolve("SC3025"), Some(Rule::PrintfQFormatInSh));
        assert_eq!(map.resolve("SC3034"), Some(Rule::BashFileSlurp));
        assert_eq!(map.resolve("SC3037"), Some(Rule::EchoFlags));
        assert_eq!(map.resolve("SC2018"), Some(Rule::TrLowerRange));
        assert_eq!(map.resolve("SC2019"), Some(Rule::TrUpperRange));
        assert_eq!(map.resolve("SC2028"), Some(Rule::EchoBackslashEscapes));
        assert_eq!(map.resolve("SC3024"), Some(Rule::PlusEqualsAppend));
        assert_eq!(map.resolve("SC3055"), Some(Rule::PlusEqualsAppend));
        assert_eq!(map.resolve("SC3071"), Some(Rule::PlusEqualsInSh));
        assert_eq!(
            map.resolve("SC2184"),
            Some(Rule::UnsetAssociativeArrayElement)
        );
        assert_eq!(
            map.resolve("SC2338"),
            Some(Rule::UnsetAssociativeArrayElement)
        );
        assert_eq!(map.resolve("SC2178"), Some(Rule::ArrayToStringConversion));
        assert_eq!(map.resolve("SC2381"), Some(Rule::ArrayToStringConversion));
        assert_eq!(
            map.resolve_all("SC3024"),
            vec![
                Rule::PlusEqualsAppend,
                Rule::BashFileSlurp,
                Rule::PlusEqualsInSh
            ]
        );
        assert_eq!(map.resolve_all("SC3071"), vec![Rule::PlusEqualsInSh]);
        assert_eq!(map.resolve("SC3001"), Some(Rule::ProcessSubstitution));
        assert_eq!(map.resolve("SC3003"), Some(Rule::AnsiCQuoting));
        assert_eq!(map.resolve("SC3004"), Some(Rule::DollarStringInSh));
        assert_eq!(map.resolve("SC3009"), Some(Rule::BraceExpansion));
        assert_eq!(map.resolve("SC3011"), Some(Rule::HereString));
        assert_eq!(map.resolve("SC3028"), Some(Rule::ArrayReference));
        assert_eq!(map.resolve("SC3030"), Some(Rule::ArrayAssignment));
        assert_eq!(map.resolve("SC3053"), Some(Rule::IndirectExpansion));
        assert_eq!(map.resolve("SC3079"), Some(Rule::UnsetPatternInSh));
        assert_eq!(map.resolve_all("SC3056"), vec![Rule::UnsetPatternInSh]);
        assert_eq!(map.resolve("SC3078"), Some(Rule::ArrayKeysInSh));
        assert_eq!(
            map.resolve_all("SC3055"),
            vec![Rule::PlusEqualsAppend, Rule::ArrayKeysInSh]
        );
        assert_eq!(map.resolve("SC3054"), Some(Rule::ArrayReference));
        assert_eq!(map.resolve("SC3057"), Some(Rule::SubstringExpansion));
        assert_eq!(map.resolve("SC3059"), Some(Rule::CaseModificationExpansion));
        assert_eq!(map.resolve("SC3060"), Some(Rule::ReplacementExpansion));
        assert_eq!(map.resolve("SC2038"), Some(Rule::FindOutputToXargs));
        assert_eq!(map.resolve("SC2064"), Some(Rule::TrapStringExpansion));
        assert_eq!(map.resolve("SC2066"), Some(Rule::QuotedDollarStarLoop));
        assert_eq!(map.resolve("SC2206"), Some(Rule::UnquotedArraySplit));
        assert_eq!(map.resolve("SC2207"), Some(Rule::CommandOutputArraySplit));
        assert_eq!(map.resolve("SC2366"), Some(Rule::BacktickOutputToCommand));
        assert_eq!(map.resolve("SC2068"), Some(Rule::UnquotedArrayExpansion));
        assert_eq!(map.resolve("SC2076"), Some(Rule::QuotedBashRegex));
        assert_eq!(map.resolve("SC2086"), Some(Rule::UnquotedExpansion));
        assert_eq!(map.resolve("SC2146"), Some(Rule::FindOrWithoutGrouping));
        assert_eq!(map.resolve("SC2332"), Some(Rule::FindOrWithoutGrouping));
        assert_eq!(map.resolve("SC2121"), Some(Rule::SetFlagsWithoutDashes));
        assert_eq!(map.resolve("SC2324"), Some(Rule::SetFlagsWithoutDashes));
        assert_eq!(map.resolve("SC2115"), Some(Rule::RmGlobOnVariablePath));
        assert_eq!(map.resolve("SC2104"), Some(Rule::LoopControlOutsideLoop));
        assert_eq!(
            map.resolve("SC2368"),
            Some(Rule::ContinueOutsideLoopInFunction)
        );
        assert_eq!(map.resolve("SC2378"), Some(Rule::VariableAsCommandName));
        assert_eq!(map.resolve("SC2398"), Some(Rule::KeywordFunctionName));
        assert_eq!(map.resolve("SC2112"), Some(Rule::FunctionKeyword));
        assert_eq!(map.resolve("SC2216"), Some(Rule::PipeToKill));
        assert_eq!(map.resolve("SC2217"), Some(Rule::EchoHereDoc));
        assert_eq!(map.resolve("SC3005"), Some(Rule::CStyleForInSh));
        assert_eq!(map.resolve("SC3006"), Some(Rule::StandaloneArithmetic));
        assert_eq!(map.resolve("SC3007"), Some(Rule::LegacyArithmeticInSh));
        assert_eq!(map.resolve("SC3008"), Some(Rule::SelectLoop));
        assert_eq!(map.resolve("SC3063"), Some(Rule::CStyleForInSh));
        assert_eq!(map.resolve("SC3064"), Some(Rule::LegacyArithmeticInSh));
        assert_eq!(map.resolve("SC3018"), Some(Rule::CStyleForArithmeticInSh));
        assert_eq!(map.resolve("SC3069"), Some(Rule::CStyleForArithmeticInSh));
        assert_eq!(map.resolve("SC3032"), Some(Rule::Coproc));
        assert_eq!(map.resolve("SC3033"), Some(Rule::SelectLoop));
        assert_eq!(
            map.resolve_all("SC3033"),
            vec![Rule::SelectLoop, Rule::HyphenatedFunctionName]
        );
        assert_eq!(map.resolve("SC3074"), Some(Rule::HyphenatedFunctionName));
        assert_eq!(map.resolve("SC3039"), Some(Rule::LetCommand));
        assert_eq!(map.resolve("SC3042"), Some(Rule::LetCommand));
        assert_eq!(map.resolve("SC3040"), Some(Rule::PipefailOption));
        assert_eq!(map.resolve("SC3062"), Some(Rule::OptionTestInSh));
        assert_eq!(
            map.resolve_all("SC3062"),
            vec![Rule::OptionTestInSh, Rule::DollarStringInSh]
        );
        assert_eq!(map.resolve("SC3048"), Some(Rule::WaitOption));
        assert_eq!(map.resolve("SC3044"), Some(Rule::DeclareCommand));
        assert_eq!(map.resolve("SC3046"), Some(Rule::SourceBuiltinInSh));
        assert_eq!(map.resolve("SC2254"), Some(Rule::ArrayIndexArithmetic));
        assert_eq!(map.resolve("SC3077"), Some(Rule::BasePrefixInArithmetic));
        assert_eq!(map.resolve("SC3075"), Some(Rule::ErrexitTrapInSh));
        assert_eq!(map.resolve("SC3076"), Some(Rule::SignalNameInTrap));
        assert_eq!(map.resolve("SC2321"), Some(Rule::ArrayIndexArithmetic));
        assert_eq!(
            map.resolve_all("SC2321"),
            vec![Rule::ArrayIndexArithmetic, Rule::FunctionKeywordInSh]
        );
        assert_eq!(map.resolve("SC2120"), Some(Rule::FunctionCalledWithoutArgs));
        assert_eq!(
            map.resolve("SC2364"),
            Some(Rule::FunctionReferencesUnsetParam)
        );
        assert_eq!(map.resolve("SC2323"), Some(Rule::ArithmeticScoreLine));
        assert_eq!(map.resolve("SC2054"), Some(Rule::CommaArrayElements));
        assert_eq!(map.resolve("SC2336"), Some(Rule::AppendToArrayAsString));
        assert_eq!(
            map.resolve("SC2339"),
            Some(Rule::MapfileProcessSubstitution)
        );
        assert_eq!(map.resolve("SC2399"), Some(Rule::BrokenAssocKey));
        assert_eq!(map.resolve("SC2333"), Some(Rule::NonShellSyntaxInScript));
        assert_eq!(
            map.resolve("SC2163"),
            Some(Rule::ExportWithPositionalParams)
        );
        assert_eq!(
            map.resolve("SC2334"),
            Some(Rule::ExportWithPositionalParams)
        );
        assert_eq!(map.resolve("SC2370"), Some(Rule::UnusedHeredoc));
        assert_eq!(map.resolve("SC2389"), Some(Rule::LoopWithoutEnd));
        assert_eq!(map.resolve("SC2390"), Some(Rule::MissingDoneInForLoop));
        assert_eq!(map.resolve("SC2391"), Some(Rule::DanglingElse));
        assert_eq!(map.resolve("SC2396"), Some(Rule::UntilMissingDo));
        assert_eq!(map.resolve("SC3051"), Some(Rule::SourceInsideFunctionInSh));
        assert_eq!(map.resolve("SC3084"), Some(Rule::SourceInsideFunctionInSh));
        assert_eq!(map.resolve("SC3083"), Some(Rule::NestedDefaultExpansion));
        assert_eq!(map.resolve("SC2155"), Some(Rule::ExportCommandSubstitution));
        assert_eq!(map.resolve("SC2156"), Some(Rule::FindExecDirWithShell));
        assert_eq!(map.resolve("SC2157"), Some(Rule::ConstantComparisonTest));
        assert_eq!(map.resolve("SC2158"), Some(Rule::LiteralUnaryStringTest));
        assert_eq!(map.resolve("SC2057"), Some(Rule::EscapedNegationInTest));
        assert_eq!(map.resolve("SC2078"), Some(Rule::TruthyLiteralTest));
        assert_eq!(map.resolve("SC2162"), Some(Rule::ReadWithoutRaw));
        assert_eq!(map.resolve("SC2168"), Some(Rule::LocalTopLevel));
        assert_eq!(map.resolve("SC2194"), Some(Rule::ConstantCaseSubject));
        assert_eq!(map.resolve("SC2210"), Some(Rule::BadRedirectionFdOrder));
        assert_eq!(
            map.resolve("SC2153"),
            Some(Rule::PossibleVariableMisspelling)
        );
        assert_eq!(map.resolve("sc2154"), Some(Rule::UndefinedVariable));
        assert_eq!(map.resolve("SC2241"), Some(Rule::InvalidExitStatus));
        assert_eq!(map.resolve("SC2242"), Some(Rule::CasePatternVar));
        assert_eq!(map.resolve("SC2248"), Some(Rule::BareSlashMarker));
        assert_eq!(
            map.resolve("SC2253"),
            Some(Rule::StatusCaptureAfterBranchTest)
        );
        assert_eq!(
            map.resolve("SC2100"),
            Some(Rule::AssignmentLooksLikeComparison)
        );
        assert_eq!(
            map.resolve("SC2319"),
            Some(Rule::StatusCaptureAfterBranchTest)
        );
        assert_eq!(
            map.resolve("SC2337"),
            Some(Rule::DollarQuestionAfterCommand)
        );
        assert_eq!(
            map.resolve_all("SC2319"),
            vec![
                Rule::StatusCaptureAfterBranchTest,
                Rule::AssignmentLooksLikeComparison,
            ]
        );
        assert_eq!(
            map.resolve("SC2257"),
            Some(Rule::ArithmeticRedirectionTarget)
        );
        assert_eq!(map.resolve("SC2292"), Some(Rule::DollarInArithmetic));
        assert_eq!(map.resolve("SC2297"), Some(Rule::DollarInArithmeticContext));
        assert_eq!(map.resolve("SC2250"), Some(Rule::PatternWithVariable));
        assert_eq!(map.resolve("SC2255"), Some(Rule::SubstWithRedirect));
        assert_eq!(map.resolve("SC2256"), Some(Rule::SubstWithRedirectErr));
        assert_eq!(map.resolve("SC2238"), Some(Rule::RedirectToCommandName));
        assert_eq!(map.resolve("SC2268"), None);
        assert_eq!(map.resolve("SC2239"), Some(Rule::NonAbsoluteShebang));
        assert_eq!(map.resolve("SC2260"), Some(Rule::RedirectToCommandName));
        assert_eq!(map.resolve("SC2261"), Some(Rule::NonAbsoluteShebang));
        assert_eq!(map.resolve("SC2262"), Some(Rule::TemplateBraceInCommand));
        assert_eq!(map.resolve("SC2264"), Some(Rule::NestedParameterExpansion));
        assert_eq!(map.resolve("SC2089"), Some(Rule::AppendWithEscapedQuotes));
        assert_eq!(map.resolve("SC2318"), Some(Rule::LocalCrossReference));
        assert_eq!(map.resolve("SC2276"), Some(Rule::PlusPrefixInAssignment));
        assert_eq!(
            map.resolve("SC2270"),
            Some(Rule::AssignmentToNumericVariable)
        );
        assert_eq!(map.resolve("SC2271"), Some(Rule::ElseWithoutThen));
        assert_eq!(
            map.resolve("SC2272"),
            Some(Rule::MissingSemicolonBeforeBrace)
        );
        assert_eq!(map.resolve("SC2273"), Some(Rule::EmptyFunctionBody));
        assert_eq!(map.resolve("SC2274"), Some(Rule::BareClosingBrace));
        assert_eq!(map.resolve("SC2277"), Some(Rule::ExtglobInCasePattern));
        assert_eq!(
            map.resolve("SC2281"),
            Some(Rule::BackslashBeforeClosingBacktick)
        );
        assert_eq!(map.resolve("SC2282"), Some(Rule::BadVarName));
        assert_eq!(
            map.resolve_all("SC2282"),
            vec![Rule::BadVarName, Rule::PositionalParamAsOperator]
        );
        assert_eq!(map.resolve("SC2283"), Some(Rule::DoubleParenGrouping));
        assert_eq!(map.resolve("SC2284"), Some(Rule::UnicodeQuoteInString));
        assert_eq!(map.resolve("SC2148"), Some(Rule::MissingShebangLine));
        assert_eq!(map.resolve("SC2285"), Some(Rule::MissingShebangLine));
        assert_eq!(map.resolve("SC2286"), Some(Rule::IndentedShebang));
        assert_eq!(map.resolve("SC2287"), Some(Rule::SpaceAfterHashBang));
        assert_eq!(map.resolve("SC2288"), Some(Rule::TemplateBraceInCommand));
        assert_eq!(map.resolve("SC2289"), Some(Rule::CommentedContinuationLine));
        assert_eq!(map.resolve("SC2333"), Some(Rule::NonShellSyntaxInScript));
        assert_eq!(map.resolve("SC1133"), Some(Rule::LinebreakBeforeAnd));
        assert_eq!(map.resolve("SC2389"), Some(Rule::LoopWithoutEnd));
        assert_eq!(map.resolve("SC2390"), Some(Rule::MissingDoneInForLoop));
        assert_eq!(map.resolve("SC2391"), Some(Rule::DanglingElse));
        assert_eq!(map.resolve("SC2392"), Some(Rule::LinebreakBeforeAnd));
        assert_eq!(map.resolve("SC2394"), Some(Rule::HeredocCloserNotAlone));
        assert_eq!(map.resolve("SC2395"), Some(Rule::MisquotedHeredocClose));
        assert_eq!(map.resolve("SC2396"), Some(Rule::UntilMissingDo));
        assert_eq!(map.resolve("SC2397"), Some(Rule::AmpersandSemicolon));
        assert_eq!(map.resolve("SC2221"), Some(Rule::CaseGlobReachability));
        assert_eq!(map.resolve("SC2373"), Some(Rule::CaseGlobReachability));
        assert_eq!(map.resolve("SC2222"), Some(Rule::CaseDefaultBeforeGlob));
        assert_eq!(map.resolve("SC2374"), Some(Rule::CaseDefaultBeforeGlob));
        assert_eq!(map.resolve("SC2372"), Some(Rule::SingleLetterCaseLabel));
        assert_eq!(map.resolve("SC2213"), Some(Rule::GetoptsOptionNotInCase));
        assert_eq!(map.resolve("SC2382"), Some(Rule::GetoptsOptionNotInCase));
        assert_eq!(map.resolve("SC2214"), Some(Rule::CaseArmNotInGetopts));
        assert_eq!(map.resolve("SC2383"), Some(Rule::CaseArmNotInGetopts));
        assert_eq!(map.resolve("SC2280"), Some(Rule::IfsEqualsAmbiguity));
        assert_eq!(map.resolve("SC2266"), Some(Rule::OverwrittenFunction));
        assert_eq!(map.resolve("SC2365"), Some(Rule::UnreachableAfterExit));
        assert_eq!(
            map.resolve("SC2353"),
            Some(Rule::AssignmentToNumericVariable)
        );
        assert_eq!(map.resolve("SC2354"), Some(Rule::PlusPrefixInAssignment));
        assert_eq!(map.resolve("SC2387"), Some(Rule::SpacedAssignment));
        assert_eq!(map.resolve("SC2388"), Some(Rule::BadVarName));
        assert_eq!(map.resolve("SC2384"), Some(Rule::LocalCrossReference));
        assert_eq!(map.resolve("SC2377"), Some(Rule::AppendWithEscapedQuotes));
        assert_eq!(
            map.resolve_all("SC2270"),
            vec![Rule::AssignmentToNumericVariable, Rule::IfMissingThen]
        );
        assert_eq!(map.resolve_all("SC2384"), vec![Rule::LocalCrossReference]);
        assert_eq!(map.resolve("SC2141"), Some(Rule::IfsSetToLiteralBackslashN));
        assert_eq!(map.resolve("SC2329"), Some(Rule::IfsSetToLiteralBackslashN));
        assert_eq!(map.resolve("SC7777"), None);
    }

    #[test]
    fn shared_comparison_codes_keep_legacy_primary_suppressions() {
        let map = ShellCheckCodeMap::default();

        assert_eq!(map.resolve("SC3024"), Some(Rule::PlusEqualsAppend));
        assert_eq!(
            map.resolve_all("SC3024"),
            vec![
                Rule::PlusEqualsAppend,
                Rule::BashFileSlurp,
                Rule::PlusEqualsInSh
            ]
        );

        assert_eq!(map.resolve("SC3058"), Some(Rule::BashCaseFallthrough));
        assert_eq!(
            map.resolve_all("SC3058"),
            vec![Rule::BashCaseFallthrough, Rule::StarGlobRemovalInSh]
        );
    }

    #[test]
    fn exposes_all_mappings() {
        let mappings = ShellCheckCodeMap::default()
            .mappings()
            .collect::<std::collections::HashSet<_>>();
        let expected = vec![
            (1001, Rule::EscapedUnderscore),
            (1002, Rule::EscapedUnderscoreLiteral),
            (1003, Rule::SingleQuoteBackslash),
            (1004, Rule::LiteralBackslash),
            (1012, Rule::NeedlessBackslashUnderscore),
            (1019, Rule::EmptyTest),
            (1037, Rule::PositionalTenBraces),
            (1040, Rule::SpacedTabstripClose),
            (1040, Rule::HeredocEndSpace),
            (1118, Rule::HeredocEndSpace),
            (1042, Rule::MisquotedHeredocClose),
            (1041, Rule::HeredocCloserNotAlone),
            (1044, Rule::HeredocMissingEnd),
            (1045, Rule::AmpersandSemicolon),
            (1047, Rule::MissingFi),
            (1065, Rule::FunctionParamsInSh),
            (1069, Rule::IfBracketGlued),
            (1070, Rule::ZshRedirPipe),
            (1072, Rule::BrokenTestParse),
            (1073, Rule::BrokenTestEnd),
            (1075, Rule::ElseIf),
            (1075, Rule::ExtglobCase),
            (1078, Rule::OpenDoubleQuote),
            (1079, Rule::SuspectClosingQuote),
            (1080, Rule::LinebreakInTest),
            (1083, Rule::LiteralBraces),
            (1090, Rule::DynamicSourcePath),
            (1091, Rule::UntrackedSourceFile),
            (1097, Rule::IfsEqualsAmbiguity),
            (1101, Rule::BackslashBeforeClosingBacktick),
            (1102, Rule::PositionalParamAsOperator),
            (1110, Rule::UnicodeQuoteInString),
            (1133, Rule::LinebreakBeforeAnd),
            (1113, Rule::TrailingDirective),
            (1126, Rule::TrailingDirective),
            (1127, Rule::CStyleComment),
            (1129, Rule::ZshBraceIf),
            (1130, Rule::ZshAlwaysBlock),
            (1132, Rule::CPrototypeFragment),
            (2003, Rule::ExprArithmetic),
            (2005, Rule::EchoedCommandSubstitution),
            (2116, Rule::EchoInsideCommandSubstitution),
            (2006, Rule::LegacyBackticks),
            (2007, Rule::LegacyArithmeticExpansion),
            (2009, Rule::PsGrepPipeline),
            (2009, Rule::DoubleParenGrouping),
            (2010, Rule::LsGrepPipeline),
            (2293, Rule::LsPipedToXargs),
            (2294, Rule::LsInSubstitution),
            (2263, Rule::RedundantSpacesInEcho),
            (2026, Rule::UnquotedWordBetweenQuotes),
            (2027, Rule::DoubleQuoteNesting),
            (2143, Rule::GrepOutputInTest),
            (2145, Rule::PositionalArgsInString),
            (2198, Rule::AtSignInStringCompare),
            (2199, Rule::ArraySliceInComparison),
            (2320, Rule::UnquotedPipeInEcho),
            (2124, Rule::QuotedArraySlice),
            (2128, Rule::QuotedBashSource),
            (2291, Rule::UnquotedVariableInSed),
            (2117, Rule::SuWithoutFlag),
            (2186, Rule::DeprecatedTempfileCommand),
            (2196, Rule::EgrepDeprecated),
            (2139, Rule::CommandSubstitutionInAlias),
            (2142, Rule::FunctionInAlias),
            (2021, Rule::UnquotedTrRange),
            (2060, Rule::UnquotedTrClass),
            (2335, Rule::UnquotedPathInMkdir),
            (2070, Rule::UnquotedVariableInTest),
            (2223, Rule::DefaultValueInColonAssign),
            (2060, Rule::UnquotedTrRange),
            (2303, Rule::UnquotedTrClass),
            (2335, Rule::UnquotedPathInMkdir),
            (2307, Rule::UnquotedVariableInTest),
            (2379, Rule::EnvPrefixQuoting),
            (2140, Rule::MixedQuoteWord),
            (2300, Rule::UnquotedWordBetweenQuotes),
            (2346, Rule::DefaultValueInColonAssign),
            (2184, Rule::UnsetAssociativeArrayElement),
            (2178, Rule::ArrayToStringConversion),
            (2322, Rule::SuWithoutFlag),
            (2340, Rule::DeprecatedTempfileCommand),
            (2342, Rule::EgrepDeprecated),
            (2344, Rule::AtSignInStringCompare),
            (2345, Rule::ArraySliceInComparison),
            (2325, Rule::QuotedArraySlice),
            (2327, Rule::QuotedBashSource),
            (2328, Rule::CommandSubstitutionInAlias),
            (2330, Rule::FunctionInAlias),
            (2376, Rule::DoubleQuoteNesting),
            (2298, Rule::UnquotedTrRange),
            (2057, Rule::EscapedNegationInTest),
            (2258, Rule::BareRead),
            (2013, Rule::LineOrientedInput),
            (2015, Rule::ChainedTestBranches),
            (2015, Rule::ShortCircuitFallthrough),
            (2296, Rule::ShortCircuitFallthrough),
            (2016, Rule::SingleQuotedLiteral),
            (2014, Rule::UnquotedGlobsInFind),
            (2231, Rule::GlobWithExpansionInLoop),
            (2022, Rule::GlobInGrepPattern),
            (2062, Rule::UnquotedGrepRegex),
            (2053, Rule::GlobInStringComparison),
            (2061, Rule::GlobInFindSubstitution),
            (2024, Rule::SudoRedirectionOrder),
            (2034, Rule::UnusedAssignment),
            (2035, Rule::LeadingGlobArgument),
            (2038, Rule::FindOutputToXargs),
            (2043, Rule::SingleIterationLoop),
            (2044, Rule::FindOutputLoop),
            (2348, Rule::FindOutputLoop),
            (2380, Rule::MisspelledOptionName),
            (2045, Rule::LoopFromCommandOutput),
            (2046, Rule::UnquotedCommandSubstitution),
            (2048, Rule::UnquotedDollarStar),
            (2059, Rule::PrintfFormatVariable),
            (2029, Rule::SshLocalExpansion),
            (2209, Rule::ConditionalAssignmentShortcut),
            (2352, Rule::DefaultElseInShortCircuit),
            (2072, Rule::StringComparisonForVersion),
            (2073, Rule::GreaterThanInTest),
            (2081, Rule::GlobInTestComparison),
            (2088, Rule::TildeInStringComparison),
            (2091, Rule::IfDollarCommand),
            (2092, Rule::BacktickInCommandPosition),
            (2064, Rule::TrapStringExpansion),
            (2066, Rule::QuotedDollarStarLoop),
            (2206, Rule::UnquotedArraySplit),
            (2207, Rule::CommandOutputArraySplit),
            (2366, Rule::BacktickOutputToCommand),
            (2068, Rule::UnquotedArrayExpansion),
            (2076, Rule::QuotedBashRegex),
            (2078, Rule::TruthyLiteralTest),
            (2089, Rule::AppendWithEscapedQuotes),
            (2086, Rule::UnquotedExpansion),
            (2146, Rule::FindOrWithoutGrouping),
            (2114, Rule::ConditionalAssignmentShortcut),
            (2121, Rule::SetFlagsWithoutDashes),
            (2165, Rule::SingleIterationLoop),
            (2399, Rule::BrokenAssocKey),
            (2054, Rule::CommaArrayElements),
            (2336, Rule::AppendToArrayAsString),
            (2339, Rule::MapfileProcessSubstitution),
            (2104, Rule::LoopControlOutsideLoop),
            (2112, Rule::FunctionKeyword),
            (2398, Rule::KeywordFunctionName),
            (2115, Rule::RmGlobOnVariablePath),
            (2125, Rule::GlobAssignedToVariable),
            (2126, Rule::GrepCountPipeline),
            (2127, Rule::BashCaseFallthrough),
            (2127, Rule::EchoHereDoc),
            (2154, Rule::UndefinedVariable),
            (2155, Rule::ExportCommandSubstitution),
            (2156, Rule::FindExecDirWithShell),
            (2157, Rule::ConstantComparisonTest),
            (2158, Rule::LiteralUnaryStringTest),
            (2162, Rule::ReadWithoutRaw),
            (2163, Rule::ExportWithPositionalParams),
            (2164, Rule::UncheckedDirectoryChange),
            (2168, Rule::LocalTopLevel),
            (2194, Rule::ConstantCaseSubject),
            (2219, Rule::AvoidLetBuiltin),
            (2210, Rule::BadRedirectionFdOrder),
            (2217, Rule::EchoHereDoc),
            (2216, Rule::PipeToKill),
            (2221, Rule::CaseGlobReachability),
            (2222, Rule::CaseDefaultBeforeGlob),
            (2213, Rule::GetoptsOptionNotInCase),
            (2214, Rule::CaseArmNotInGetopts),
            (2233, Rule::SingleTestSubshell),
            (2235, Rule::SubshellTestGroup),
            (2238, Rule::RedirectToCommandName),
            (2239, Rule::NonAbsoluteShebang),
            (2240, Rule::SourcedWithArgs),
            (2241, Rule::InvalidExitStatus),
            (2242, Rule::CasePatternVar),
            (2248, Rule::BareSlashMarker),
            (2254, Rule::ArrayIndexArithmetic),
            (2250, Rule::PatternWithVariable),
            (2251, Rule::ZshFlagExpansion),
            (2252, Rule::NestedZshSubstitution),
            (2255, Rule::SubstWithRedirect),
            (2256, Rule::SubstWithRedirectErr),
            (2276, Rule::PlusPrefixInAssignment),
            (2257, Rule::ArithmeticRedirectionTarget),
            (2290, Rule::SubshellInArithmetic),
            (2295, Rule::UnquotedGlobsInFind),
            (2299, Rule::GlobInGrepPattern),
            (2301, Rule::GlobInStringComparison),
            (2304, Rule::GlobInFindSubstitution),
            (2305, Rule::UnquotedGrepRegex),
            (2349, Rule::GlobWithExpansionInLoop),
            (2292, Rule::DollarInArithmetic),
            (2297, Rule::DollarInArithmeticContext),
            (2259, Rule::SubshellTestGroup),
            (2264, Rule::NestedParameterExpansion),
            (2266, Rule::OverwrittenFunction),
            (2267, Rule::LiteralBackslashInSingleQuotes),
            (2270, Rule::AssignmentToNumericVariable),
            (2270, Rule::IfMissingThen),
            (2318, Rule::LocalCrossReference),
            (2290, Rule::SpacedAssignment),
            (2384, Rule::LocalCrossReference),
            (2387, Rule::SpacedAssignment),
            (2271, Rule::ElseWithoutThen),
            (2272, Rule::MissingSemicolonBeforeBrace),
            (2273, Rule::EmptyFunctionBody),
            (2274, Rule::BareClosingBrace),
            (2275, Rule::MultiVarForLoop),
            (2278, Rule::ZshPromptBracket),
            (2279, Rule::CshSyntaxInSh),
            (2280, Rule::IfsEqualsAmbiguity),
            (2282, Rule::BadVarName),
            (2283, Rule::DoubleParenGrouping),
            (2148, Rule::MissingShebangLine),
            (2285, Rule::MissingShebangLine),
            (2286, Rule::IndentedShebang),
            (2287, Rule::SpaceAfterHashBang),
            (2288, Rule::TemplateBraceInCommand),
            (2289, Rule::CommentedContinuationLine),
            (2294, Rule::EvalOnArray),
            (2030, Rule::SubshellSideEffect),
            (2031, Rule::SubshellLocalAssignment),
            (2153, Rule::PossibleVariableMisspelling),
            (2302, Rule::EscapedNegationInTest),
            (2308, Rule::GreaterThanInTest),
            (2309, Rule::StringComparisonForVersion),
            (2310, Rule::MixedAndOrInCondition),
            (2311, Rule::QuotedCommandInTest),
            (2312, Rule::GlobInTestComparison),
            (2314, Rule::TildeInStringComparison),
            (2315, Rule::IfDollarCommand),
            (2316, Rule::BacktickInCommandPosition),
            (2313, Rule::ZshNestedExpansion),
            (2319, Rule::StatusCaptureAfterBranchTest),
            (2337, Rule::DollarQuestionAfterCommand),
            (2120, Rule::FunctionCalledWithoutArgs),
            (2141, Rule::IfsSetToLiteralBackslashN),
            (2364, Rule::FunctionReferencesUnsetParam),
            (2367, Rule::UncheckedDirectoryChangeInFunction),
            (2368, Rule::ContinueOutsideLoopInFunction),
            (2378, Rule::VariableAsCommandName),
            (2353, Rule::AssignmentToNumericVariable),
            (2354, Rule::PlusPrefixInAssignment),
            (2377, Rule::AppendWithEscapedQuotes),
            (2329, Rule::IfsSetToLiteralBackslashN),
            (2320, Rule::UnquotedPipeInEcho),
            (2321, Rule::ArrayIndexArithmetic),
            (2323, Rule::ArithmeticScoreLine),
            (2321, Rule::FunctionKeywordInSh),
            (2120, Rule::FunctionCalledWithoutArgs),
            (2323, Rule::ArithmeticScoreLine),
            (2332, Rule::FindOrWithoutGrouping),
            (2399, Rule::BrokenAssocKey),
            (2054, Rule::CommaArrayElements),
            (2339, Rule::MapfileProcessSubstitution),
            (2324, Rule::SetFlagsWithoutDashes),
            (2326, Rule::GlobAssignedToVariable),
            (2333, Rule::NonShellSyntaxInScript),
            (2334, Rule::ExportWithPositionalParams),
            (2389, Rule::LoopWithoutEnd),
            (2390, Rule::MissingDoneInForLoop),
            (2391, Rule::DanglingElse),
            (2392, Rule::LinebreakBeforeAnd),
            (2393, Rule::SpacedTabstripClose),
            (2394, Rule::HeredocCloserNotAlone),
            (2395, Rule::MisquotedHeredocClose),
            (2396, Rule::UntilMissingDo),
            (2397, Rule::AmpersandSemicolon),
            (2221, Rule::CaseGlobReachability),
            (2373, Rule::CaseGlobReachability),
            (2222, Rule::CaseDefaultBeforeGlob),
            (2374, Rule::CaseDefaultBeforeGlob),
            (2372, Rule::SingleLetterCaseLabel),
            (2213, Rule::GetoptsOptionNotInCase),
            (2382, Rule::GetoptsOptionNotInCase),
            (2214, Rule::CaseArmNotInGetopts),
            (2383, Rule::CaseArmNotInGetopts),
            (2355, Rule::ZshAssignmentToZero),
            (2359, Rule::ZshParameterFlag),
            (2365, Rule::UnreachableAfterExit),
            (2370, Rule::UnusedHeredoc),
            (2371, Rule::ZshArraySubscriptInCase),
            (2375, Rule::ZshParameterIndexFlag),
            (2385, Rule::UnicodeSingleQuoteInSingleQuotes),
            (2388, Rule::BadVarName),
            (2386, Rule::HeredocMissingEnd),
            (2394, Rule::HeredocCloserNotAlone),
            (2395, Rule::MisquotedHeredocClose),
            (3001, Rule::ProcessSubstitution),
            (3002, Rule::ExtglobInSh),
            (3003, Rule::AnsiCQuoting),
            (3004, Rule::DollarStringInSh),
            (3005, Rule::CStyleForInSh),
            (3006, Rule::StandaloneArithmetic),
            (3007, Rule::LegacyArithmeticInSh),
            (3008, Rule::SelectLoop),
            (3009, Rule::BraceExpansion),
            (3010, Rule::DoubleBracketInSh),
            (3011, Rule::HereString),
            (3012, Rule::GreaterThanInDoubleBracket),
            (3014, Rule::TestEqualityOperator),
            (3015, Rule::RegexMatchInSh),
            (3016, Rule::VTestInSh),
            (3017, Rule::ATestInSh),
            (3018, Rule::CStyleForArithmeticInSh),
            (3024, Rule::PlusEqualsAppend),
            (3071, Rule::PlusEqualsInSh),
            (3024, Rule::BashFileSlurp),
            (3024, Rule::PlusEqualsInSh),
            (3034, Rule::BashFileSlurp),
            (3037, Rule::EchoFlags),
            (2018, Rule::TrLowerRange),
            (2019, Rule::TrUpperRange),
            (2028, Rule::EchoBackslashEscapes),
            (3025, Rule::PrintfQFormatInSh),
            (3026, Rule::CaretNegationInBracket),
            (3028, Rule::ArrayReference),
            (3030, Rule::ArrayAssignment),
            (3032, Rule::Coproc),
            (3033, Rule::SelectLoop),
            (3033, Rule::HyphenatedFunctionName),
            (3074, Rule::HyphenatedFunctionName),
            (2338, Rule::UnsetAssociativeArrayElement),
            (2381, Rule::ArrayToStringConversion),
            (3039, Rule::LetCommand),
            (3040, Rule::PipefailOption),
            (3042, Rule::LetCommand),
            (3043, Rule::LocalVariableInSh),
            (3044, Rule::DeclareCommand),
            (3046, Rule::SourceBuiltinInSh),
            (3047, Rule::TrapErr),
            (3048, Rule::WaitOption),
            (3050, Rule::BraceFdRedirection),
            (3051, Rule::SourceInsideFunctionInSh),
            (3052, Rule::AmpersandRedirection),
            (3053, Rule::IndirectExpansion),
            (3056, Rule::UnsetPatternInSh),
            (3079, Rule::UnsetPatternInSh),
            (3083, Rule::NestedDefaultExpansion),
            (3078, Rule::ArrayKeysInSh),
            (3054, Rule::ArrayReference),
            (3055, Rule::PlusEqualsAppend),
            (3055, Rule::ArrayKeysInSh),
            (3057, Rule::SubstringExpansion),
            (3058, Rule::BashCaseFallthrough),
            (3085, Rule::StarGlobRemovalInSh),
            (3058, Rule::StarGlobRemovalInSh),
            (3059, Rule::CaseModificationExpansion),
            (3060, Rule::ReplacementExpansion),
            (3061, Rule::ExtglobInSh),
            (3062, Rule::OptionTestInSh),
            (3062, Rule::DollarStringInSh),
            (3063, Rule::CStyleForInSh),
            (3064, Rule::LegacyArithmeticInSh),
            (3065, Rule::StickyBitTestInSh),
            (3067, Rule::OwnershipTestInSh),
            (3069, Rule::CStyleForArithmeticInSh),
            (3070, Rule::AmpersandRedirectInSh),
            (3072, Rule::CaretNegationInBracket),
            (3073, Rule::PipeStderrInSh),
            (3075, Rule::ErrexitTrapInSh),
            (3076, Rule::SignalNameInTrap),
            (3077, Rule::BasePrefixInArithmetic),
            (3084, Rule::SourceInsideFunctionInSh),
            (2100, Rule::AssignmentLooksLikeComparison),
            (2319, Rule::AssignmentLooksLikeComparison),
            (2148, Rule::MissingShebangLine),
            (2286, Rule::IndentedShebang),
            (2287, Rule::SpaceAfterHashBang),
            (2277, Rule::ExtglobInCasePattern),
        ]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();

        assert_eq!(
            mappings.len(),
            expected.len(),
            "missing from expected: {:?}\nextra in expected: {:?}",
            mappings.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&mappings).collect::<Vec<_>>()
        );
        assert_eq!(mappings, expected);
    }

    #[test]
    fn every_real_rule_has_a_primary_shellcheck_mapping() {
        let map = ShellCheckCodeMap::default();
        let mapped_rules: std::collections::HashSet<Rule> =
            map.mappings().map(|(_, rule)| rule).collect();
        let allowed_unmapped = std::collections::HashSet::from([
            Rule::IfElifBashTest,
            Rule::ExtendedGlobInTest,
            Rule::ArraySubscriptTest,
            Rule::ArraySubscriptCondition,
            Rule::ExtglobInTest,
            Rule::BackslashBeforeCommand,
            Rule::ShebangNotOnFirstLine,
        ]);

        let unmapped: Vec<&str> = Rule::iter()
            .filter(|r| !mapped_rules.contains(r) && !allowed_unmapped.contains(r))
            .map(|r| r.code())
            .collect();

        assert!(
            unmapped.is_empty(),
            "rules without a shellcheck mapping: {unmapped:?}"
        );
    }

    #[test]
    fn comparison_mappings_skip_ambiguous_codes() {
        let comparison = ShellCheckCodeMap::default()
            .comparison_mappings()
            .collect::<Vec<_>>();

        assert!(comparison.contains(&(3002, Rule::ExtglobInSh)));
        assert!(comparison.contains(&(3004, Rule::DollarStringInSh)));
        assert!(comparison.contains(&(3006, Rule::StandaloneArithmetic)));
        assert!(comparison.contains(&(3007, Rule::LegacyArithmeticInSh)));
        assert!(comparison.contains(&(3008, Rule::SelectLoop)));
        assert!(comparison.contains(&(3018, Rule::CStyleForArithmeticInSh)));
        assert!(comparison.contains(&(3005, Rule::CStyleForInSh)));
        assert!(comparison.contains(&(3032, Rule::Coproc)));
        assert!(comparison.contains(&(3026, Rule::CaretNegationInBracket)));
        assert!(comparison.contains(&(3039, Rule::LetCommand)));
        assert!(comparison.contains(&(3042, Rule::LetCommand)));
        assert!(comparison.contains(&(2009, Rule::PsGrepPipeline)));
        assert!(comparison.contains(&(2010, Rule::LsGrepPipeline)));
        assert!(comparison.contains(&(2014, Rule::UnquotedGlobsInFind)));
        assert!(comparison.contains(&(2231, Rule::GlobWithExpansionInLoop)));
        assert!(comparison.contains(&(2022, Rule::GlobInGrepPattern)));
        assert!(comparison.contains(&(2062, Rule::UnquotedGrepRegex)));
        assert!(comparison.contains(&(2053, Rule::GlobInStringComparison)));
        assert!(comparison.contains(&(2061, Rule::GlobInFindSubstitution)));
        assert!(comparison.contains(&(2293, Rule::LsPipedToXargs)));
        assert!(comparison.contains(&(2294, Rule::LsInSubstitution)));
        assert!(!comparison.contains(&(2294, Rule::EvalOnArray)));
        assert!(!comparison.contains(&(2295, Rule::UnquotedGlobsInFind)));
        assert!(!comparison.contains(&(2299, Rule::GlobInGrepPattern)));
        assert!(!comparison.contains(&(2301, Rule::GlobInStringComparison)));
        assert!(!comparison.contains(&(2304, Rule::GlobInFindSubstitution)));
        assert!(!comparison.contains(&(2305, Rule::UnquotedGrepRegex)));
        assert!(!comparison.contains(&(2349, Rule::GlobWithExpansionInLoop)));
        assert!(comparison.contains(&(2263, Rule::RedundantSpacesInEcho)));
        assert!(comparison.contains(&(2291, Rule::UnquotedVariableInSed)));
        assert!(comparison.contains(&(2026, Rule::UnquotedWordBetweenQuotes)));
        assert!(comparison.contains(&(2117, Rule::SuWithoutFlag)));
        assert!(comparison.contains(&(2186, Rule::DeprecatedTempfileCommand)));
        assert!(comparison.contains(&(2196, Rule::EgrepDeprecated)));
        assert!(comparison.contains(&(2184, Rule::UnsetAssociativeArrayElement)));
        assert!(comparison.contains(&(2178, Rule::ArrayToStringConversion)));
        assert!(comparison.contains(&(2139, Rule::CommandSubstitutionInAlias)));
        assert!(comparison.contains(&(2142, Rule::FunctionInAlias)));
        assert!(comparison.contains(&(2027, Rule::DoubleQuoteNesting)));
        assert!(comparison.contains(&(2379, Rule::EnvPrefixQuoting)));
        assert!(comparison.contains(&(2140, Rule::MixedQuoteWord)));
        assert!(!comparison.contains(&(2322, Rule::SuWithoutFlag)));
        assert!(!comparison.contains(&(2340, Rule::DeprecatedTempfileCommand)));
        assert!(!comparison.contains(&(2342, Rule::EgrepDeprecated)));
        assert!(!comparison.contains(&(2328, Rule::CommandSubstitutionInAlias)));
        assert!(!comparison.contains(&(2330, Rule::FunctionInAlias)));
        assert!(!comparison.contains(&(2376, Rule::DoubleQuoteNesting)));
        assert!(comparison.contains(&(2258, Rule::BareRead)));
        assert!(comparison.contains(&(2060, Rule::UnquotedTrClass)));
        assert!(comparison.contains(&(2335, Rule::UnquotedPathInMkdir)));
        assert!(comparison.contains(&(2070, Rule::UnquotedVariableInTest)));
        assert!(comparison.contains(&(2320, Rule::UnquotedPipeInEcho)));
        assert!(comparison.contains(&(2223, Rule::DefaultValueInColonAssign)));
        assert!(comparison.contains(&(2021, Rule::UnquotedTrRange)));
        assert!(comparison.contains(&(2018, Rule::TrLowerRange)));
        assert!(comparison.contains(&(2019, Rule::TrUpperRange)));
        assert!(comparison.contains(&(2028, Rule::EchoBackslashEscapes)));
        assert!(!comparison.contains(&(2300, Rule::UnquotedWordBetweenQuotes)));
        assert!(!comparison.contains(&(2307, Rule::UnquotedVariableInTest)));
        assert!(!comparison.contains(&(2346, Rule::DefaultValueInColonAssign)));
        assert!(!comparison.contains(&(2298, Rule::UnquotedTrRange)));
        assert!(!comparison.contains(&(2060, Rule::UnquotedTrRange)));
        assert!(comparison.contains(&(2143, Rule::GrepOutputInTest)));
        assert!(comparison.contains(&(2145, Rule::PositionalArgsInString)));
        assert!(comparison.contains(&(2198, Rule::AtSignInStringCompare)));
        assert!(comparison.contains(&(2199, Rule::ArraySliceInComparison)));
        assert!(comparison.contains(&(2124, Rule::QuotedArraySlice)));
        assert!(!comparison.contains(&(2325, Rule::QuotedArraySlice)));
        assert!(comparison.contains(&(2128, Rule::QuotedBashSource)));
        assert!(!comparison.contains(&(2327, Rule::QuotedBashSource)));
        assert!(comparison.contains(&(2283, Rule::DoubleParenGrouping)));
        assert!(comparison.contains(&(2219, Rule::AvoidLetBuiltin)));
        assert!(comparison.contains(&(2066, Rule::QuotedDollarStarLoop)));
        assert!(comparison.contains(&(2206, Rule::UnquotedArraySplit)));
        assert!(comparison.contains(&(2207, Rule::CommandOutputArraySplit)));
        assert!(comparison.contains(&(2366, Rule::BacktickOutputToCommand)));
        assert!(comparison.contains(&(2048, Rule::UnquotedDollarStar)));
        assert!(comparison.contains(&(2127, Rule::BashCaseFallthrough)));
        assert!(comparison.contains(&(2146, Rule::FindOrWithoutGrouping)));
        assert!(comparison.contains(&(2121, Rule::SetFlagsWithoutDashes)));
        assert!(comparison.contains(&(2125, Rule::GlobAssignedToVariable)));
        assert!(comparison.contains(&(2054, Rule::CommaArrayElements)));
        assert!(comparison.contains(&(2399, Rule::BrokenAssocKey)));
        assert!(!comparison.contains(&(2326, Rule::GlobAssignedToVariable)));
        assert!(comparison.contains(&(2336, Rule::AppendToArrayAsString)));
        assert!(comparison.contains(&(2339, Rule::MapfileProcessSubstitution)));
        assert!(comparison.contains(&(2380, Rule::MisspelledOptionName)));
        assert!(!comparison.contains(&(2348, Rule::FindOutputLoop)));
        assert!(comparison.contains(&(3058, Rule::BashCaseFallthrough)));
        assert!(comparison.contains(&(3058, Rule::StarGlobRemovalInSh)));
        assert!(comparison.contains(&(3040, Rule::PipefailOption)));
        assert!(comparison.contains(&(3025, Rule::PrintfQFormatInSh)));
        assert!(comparison.contains(&(3048, Rule::WaitOption)));
        assert!(comparison.contains(&(3037, Rule::EchoFlags)));
        assert!(comparison.contains(&(2217, Rule::EchoHereDoc)));
        assert!(comparison.contains(&(3046, Rule::SourceBuiltinInSh)));
        assert!(comparison.contains(&(3024, Rule::PlusEqualsAppend)));
        assert!(comparison.contains(&(3024, Rule::PlusEqualsInSh)));
        assert!(comparison.contains(&(3034, Rule::BashFileSlurp)));
        assert!(comparison.contains(&(3011, Rule::HereString)));
        assert!(comparison.contains(&(3030, Rule::ArrayAssignment)));
        assert!(comparison.contains(&(3053, Rule::IndirectExpansion)));
        assert!(comparison.contains(&(3055, Rule::ArrayKeysInSh)));
        assert!(comparison.contains(&(3054, Rule::ArrayReference)));
        assert!(comparison.contains(&(3057, Rule::SubstringExpansion)));
        assert!(comparison.contains(&(3059, Rule::CaseModificationExpansion)));
        assert!(comparison.contains(&(3060, Rule::ReplacementExpansion)));
        assert!(comparison.contains(&(3083, Rule::NestedDefaultExpansion)));
        assert!(comparison.contains(&(2277, Rule::ExtglobInCasePattern)));
        assert!(comparison.contains(&(3077, Rule::BasePrefixInArithmetic)));
        assert!(comparison.contains(&(3075, Rule::ErrexitTrapInSh)));
        assert!(comparison.contains(&(3076, Rule::SignalNameInTrap)));
        assert!(comparison.contains(&(3050, Rule::BraceFdRedirection)));
        assert!(comparison.contains(&(3052, Rule::AmpersandRedirection)));
        assert!(comparison.contains(&(3051, Rule::SourceInsideFunctionInSh)));
        assert!(comparison.contains(&(3070, Rule::AmpersandRedirectInSh)));
        assert!(comparison.contains(&(3073, Rule::PipeStderrInSh)));
        assert!(comparison.contains(&(2004, Rule::DollarInArithmetic)));
        assert!(comparison.contains(&(2320, Rule::UnquotedPipeInEcho)));
        assert!(comparison.contains(&(2321, Rule::ArrayIndexArithmetic)));
        assert!(comparison.contains(&(2120, Rule::FunctionCalledWithoutArgs)));
        assert!(comparison.contains(&(2364, Rule::FunctionReferencesUnsetParam)));
        assert!(comparison.contains(&(2323, Rule::ArithmeticScoreLine)));
        assert!(comparison.contains(&(1041, Rule::HeredocCloserNotAlone)));
        assert!(comparison.contains(&(1042, Rule::MisquotedHeredocClose)));
        assert!(comparison.contains(&(2370, Rule::UnusedHeredoc)));
        assert!(comparison.contains(&(1044, Rule::HeredocMissingEnd)));
        assert!(comparison.contains(&(1118, Rule::HeredocEndSpace)));
        assert!(comparison.contains(&(1040, Rule::SpacedTabstripClose)));
        assert!(comparison.contains(&(2372, Rule::SingleLetterCaseLabel)));
        assert!(!comparison.contains(&(2009, Rule::DoubleParenGrouping)));
        assert!(!comparison.contains(&(2004, Rule::DollarInArithmeticContext)));
        assert!(comparison.contains(&(2141, Rule::IfsSetToLiteralBackslashN)));
        assert!(comparison.contains(&(1097, Rule::IfsEqualsAmbiguity)));
        assert!(comparison.contains(&(2089, Rule::AppendWithEscapedQuotes)));
        assert!(comparison.contains(&(2318, Rule::LocalCrossReference)));
        assert!(comparison.contains(&(2290, Rule::SubshellInArithmetic)));
        assert!(comparison.contains(&(2282, Rule::BadVarName)));
        assert!(comparison.contains(&(2270, Rule::AssignmentToNumericVariable)));
        assert!(comparison.contains(&(2276, Rule::PlusPrefixInAssignment)));
        assert!(!comparison.contains(&(2338, Rule::UnsetAssociativeArrayElement)));
        assert!(!comparison.contains(&(2381, Rule::ArrayToStringConversion)));
        assert!(!comparison.contains(&(2353, Rule::AssignmentToNumericVariable)));
        assert!(!comparison.contains(&(2354, Rule::PlusPrefixInAssignment)));
        assert!(!comparison.contains(&(2290, Rule::SpacedAssignment)));
        assert!(!comparison.contains(&(2387, Rule::SpacedAssignment)));
        assert!(!comparison.contains(&(2280, Rule::IfsEqualsAmbiguity)));
        assert!(!comparison.contains(&(2388, Rule::BadVarName)));
        assert!(!comparison.contains(&(2384, Rule::LocalCrossReference)));
        assert!(!comparison.contains(&(2377, Rule::AppendWithEscapedQuotes)));
        assert!(!comparison.contains(&(1075, Rule::ExtglobCase)));
        assert!(!comparison.contains(&(2321, Rule::FunctionKeywordInSh)));
        assert!(!comparison.contains(&(3061, Rule::ExtglobInSh)));
        assert!(!comparison.contains(&(3061, Rule::BareRead)));
        assert!(!comparison.contains(&(3062, Rule::DollarStringInSh)));
        assert!(!comparison.contains(&(3072, Rule::CaretNegationInBracket)));
        assert!(!comparison.contains(&(3055, Rule::PlusEqualsAppend)));
        assert!(!comparison.contains(&(3078, Rule::ArrayKeysInSh)));
        assert!(!comparison.contains(&(3085, Rule::StarGlobRemovalInSh)));
        assert!(!comparison.contains(&(3071, Rule::PlusEqualsInSh)));
        assert!(!comparison.contains(&(3024, Rule::BashFileSlurp)));
        assert!(!comparison.contains(&(3084, Rule::SourceInsideFunctionInSh)));
        assert!(!comparison.contains(&(3044, Rule::DeclareCommand)));
        assert!(!comparison.contains(&(3063, Rule::CStyleForInSh)));
        assert!(!comparison.contains(&(3064, Rule::LegacyArithmeticInSh)));
        assert!(!comparison.contains(&(3069, Rule::CStyleForArithmeticInSh)));
        assert!(!comparison.contains(&(2394, Rule::HeredocCloserNotAlone)));
        assert!(!comparison.contains(&(2395, Rule::MisquotedHeredocClose)));
        assert!(!comparison.contains(&(2386, Rule::HeredocMissingEnd)));
        assert!(!comparison.contains(&(2393, Rule::SpacedTabstripClose)));
        assert!(!comparison.contains(&(1040, Rule::HeredocEndSpace)));
    }
}
