use rustc_hash::FxHashSet;
use shuck_ast::{File, Span};
use shuck_indexer::Indexer;
use shuck_semantic::{SemanticAnalysis, SemanticModel};

use crate::{Diagnostic, FileContext, LinterFacts, Rule, RuleSet, ShellDialect, Violation, rules};

pub struct Checker<'a> {
    semantic: &'a SemanticModel,
    semantic_analysis: SemanticAnalysis<'a>,
    indexer: &'a Indexer,
    file: &'a File,
    source: &'a str,
    facts: LinterFacts<'a>,
    rules: &'a RuleSet,
    shell: ShellDialect,
    file_context: &'a FileContext,
    diagnostics: Vec<Diagnostic>,
    reported: FxHashSet<DiagnosticKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DiagnosticKey {
    rule: Rule,
    start: usize,
    end: usize,
}

impl DiagnosticKey {
    fn new(rule: Rule, span: Span) -> Self {
        Self {
            rule,
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

impl<'a> Checker<'a> {
    pub fn new(
        file: &'a File,
        source: &'a str,
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        rules: &'a RuleSet,
        shell: ShellDialect,
        file_context: &'a FileContext,
    ) -> Self {
        Self {
            semantic,
            semantic_analysis: semantic.analysis(),
            indexer,
            file,
            source,
            facts: LinterFacts::build(file, source, semantic, indexer, file_context),
            rules,
            shell,
            file_context,
            diagnostics: Vec::new(),
            reported: FxHashSet::default(),
        }
    }

    pub fn semantic(&self) -> &'a SemanticModel {
        self.semantic
    }

    pub fn semantic_analysis(&self) -> &SemanticAnalysis<'a> {
        &self.semantic_analysis
    }

    pub fn indexer(&self) -> &'a Indexer {
        self.indexer
    }

    pub fn ast(&self) -> &'a File {
        self.file
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn facts(&self) -> &LinterFacts<'a> {
        &self.facts
    }

    pub fn is_rule_enabled(&self, rule: Rule) -> bool {
        self.rules.contains(rule)
    }

    pub fn shell(&self) -> ShellDialect {
        self.shell
    }

    pub fn file_context(&self) -> &'a FileContext {
        self.file_context
    }

    pub fn report<V: Violation>(&mut self, violation: V, span: Span) {
        let diagnostic = Diagnostic::new(violation, span);
        self.reported
            .insert(DiagnosticKey::new(diagnostic.rule, diagnostic.span));
        self.diagnostics.push(diagnostic);
    }

    pub fn report_dedup<V: Violation>(&mut self, violation: V, span: Span) {
        let diagnostic = Diagnostic::new(violation, span);
        let key = DiagnosticKey::new(diagnostic.rule, diagnostic.span);
        if !self.reported.insert(key) {
            return;
        }
        self.diagnostics.push(diagnostic);
    }

    pub fn report_all<V: Violation>(&mut self, spans: Vec<Span>, violation: impl Fn() -> V) {
        for span in spans {
            self.report(violation(), span);
        }
    }

    pub fn report_all_dedup<V: Violation>(&mut self, spans: Vec<Span>, violation: impl Fn() -> V) {
        for span in spans {
            self.report_dedup(violation(), span);
        }
    }

    pub fn check(mut self) -> Vec<Diagnostic> {
        if self.rules.is_empty() {
            return self.diagnostics;
        }

        self.check_bindings();
        self.check_references();
        self.check_scopes();
        self.check_declarations();
        self.check_call_sites();
        self.check_source_refs();
        self.check_command_facts();
        self.check_word_and_expansion_facts();
        self.check_loop_list_and_pipeline_facts();
        self.check_redirect_and_substitution_facts();
        self.check_surface_fragment_facts();
        self.check_test_and_conditional_facts();
        self.check_flow();
        self.diagnostics
    }

    fn check_bindings(&mut self) {
        if self.is_rule_enabled(Rule::UnusedAssignment) {
            rules::correctness::unused_assignment::unused_assignment(self);
        }
        if self.is_rule_enabled(Rule::AppendToArrayAsString) {
            rules::correctness::append_to_array_as_string::append_to_array_as_string(self);
        }
        if self.is_rule_enabled(Rule::ArrayToStringConversion) {
            rules::correctness::array_to_string_conversion::array_to_string_conversion(self);
        }
        if self.is_rule_enabled(Rule::BrokenAssocKey) {
            rules::correctness::broken_assoc_key::broken_assoc_key(self);
        }
        if self.is_rule_enabled(Rule::CommaArrayElements) {
            rules::correctness::comma_array_elements::comma_array_elements(self);
        }
    }

    fn check_references(&mut self) {
        if self.is_rule_enabled(Rule::UndefinedVariable) {
            rules::correctness::undefined_variable::undefined_variable(self);
        }
    }

    fn check_scopes(&mut self) {}

    fn check_declarations(&mut self) {
        if self.is_rule_enabled(Rule::LocalTopLevel) {
            rules::correctness::script_scope_local::local_top_level(self);
        }
    }

    fn check_call_sites(&mut self) {
        if self.is_rule_enabled(Rule::OverwrittenFunction) {
            rules::correctness::overwritten_function::overwritten_function(self);
        }
        if self.is_rule_enabled(Rule::FunctionCalledWithoutArgs) {
            rules::correctness::function_called_without_args::function_called_without_args(self);
        }
        if self.is_rule_enabled(Rule::FunctionReferencesUnsetParam) {
            rules::correctness::function_references_unset_param::function_references_unset_param(
                self,
            );
        }
    }

    fn check_source_refs(&mut self) {
        if self.is_rule_enabled(Rule::DynamicSourcePath) {
            rules::correctness::dynamic_source_path::dynamic_source_path(self);
        }
        if self.is_rule_enabled(Rule::UntrackedSourceFile) {
            rules::correctness::untracked_source_file::untracked_source_file(self);
        }
    }

    fn check_command_facts(&mut self) {
        if self.is_rule_enabled(Rule::UncheckedDirectoryChange) {
            rules::correctness::unchecked_directory_change::unchecked_directory_change(self);
        }
        if self.is_rule_enabled(Rule::UncheckedDirectoryChangeInFunction) {
            rules::correctness::unchecked_directory_change_in_function::unchecked_directory_change_in_function(self);
        }
        if self.is_rule_enabled(Rule::RmGlobOnVariablePath) {
            rules::security::rm_glob_on_variable_path::rm_glob_on_variable_path(self);
        }
        if self.is_rule_enabled(Rule::SshLocalExpansion) {
            rules::security::ssh_local_expansion::ssh_local_expansion(self);
        }
        if self.is_rule_enabled(Rule::EvalOnArray) {
            rules::security::eval_on_array::eval_on_array(self);
        }
        if self.is_rule_enabled(Rule::FindExecDirWithShell) {
            rules::security::find_execdir_with_shell::find_execdir_with_shell(self);
        }
        if self.is_rule_enabled(Rule::ReadWithoutRaw) {
            rules::style::read_without_raw::read_without_raw(self);
        }
        if self.is_rule_enabled(Rule::BareRead) {
            rules::style::bare_read::bare_read(self);
        }
        if self.is_rule_enabled(Rule::AvoidLetBuiltin) {
            rules::style::avoid_let_builtin::avoid_let_builtin(self);
        }
        if self.is_rule_enabled(Rule::ArrayIndexArithmetic) {
            rules::style::array_index_arithmetic::array_index_arithmetic(self);
        }
        if self.is_rule_enabled(Rule::ArithmeticScoreLine) {
            rules::style::arithmetic_score_line::arithmetic_score_line(self);
        }
        if self.is_rule_enabled(Rule::DollarInArithmetic) {
            rules::style::dollar_in_arithmetic::dollar_in_arithmetic(self);
        }
        if self.is_rule_enabled(Rule::DollarInArithmeticContext) {
            rules::style::dollar_in_arithmetic_context::dollar_in_arithmetic_context(self);
        }
        if self.is_rule_enabled(Rule::ExprArithmetic) {
            rules::performance::expr_arithmetic::expr_arithmetic(self);
        }
        if self.is_rule_enabled(Rule::GrepCountPipeline) {
            rules::performance::grep_count_pipeline::grep_count_pipeline(self);
        }
        if self.is_rule_enabled(Rule::SingleTestSubshell) {
            rules::performance::single_test_subshell::single_test_subshell(self);
        }
        if self.is_rule_enabled(Rule::SubshellTestGroup) {
            rules::performance::subshell_test_group::subshell_test_group(self);
        }
        if self.is_rule_enabled(Rule::PrintfFormatVariable) {
            rules::style::printf_format_variable::printf_format_variable(self);
        }
        if self.is_rule_enabled(Rule::EchoedCommandSubstitution) {
            rules::style::echoed_command_substitution::echoed_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::RedundantSpacesInEcho) {
            rules::style::redundant_spaces_in_echo::redundant_spaces_in_echo(self);
        }
        if self.is_rule_enabled(Rule::UnquotedVariableInSed) {
            rules::style::unquoted_variable_in_sed::unquoted_variable_in_sed(self);
        }
        if self.is_rule_enabled(Rule::UnquotedPathInMkdir) {
            rules::style::unquoted_path_in_mkdir::unquoted_path_in_mkdir(self);
        }
        if self.is_rule_enabled(Rule::UnquotedTrClass) {
            rules::style::unquoted_tr_class::unquoted_tr_class(self);
        }
        if self.is_rule_enabled(Rule::SuWithoutFlag) {
            rules::style::su_without_flag::su_without_flag(self);
        }
        if self.is_rule_enabled(Rule::DeprecatedTempfileCommand) {
            rules::style::deprecated_tempfile_command::deprecated_tempfile_command(self);
        }
        if self.is_rule_enabled(Rule::EgrepDeprecated) {
            rules::style::egrep_deprecated::egrep_deprecated(self);
        }
        if self.is_rule_enabled(Rule::UnquotedTrRange) {
            rules::style::unquoted_tr_range::unquoted_tr_range(self);
        }
        if self.is_rule_enabled(Rule::ExportCommandSubstitution) {
            rules::style::export_command_substitution::export_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::EchoHereDoc) {
            rules::style::echo_here_doc::echo_here_doc(self);
        }
        if self.is_rule_enabled(Rule::InvalidExitStatus) {
            rules::correctness::invalid_exit_status::invalid_exit_status(self);
        }
        if self.is_rule_enabled(Rule::CStyleComment) {
            rules::correctness::c_style_comment::c_style_comment(self);
        }
        if self.is_rule_enabled(Rule::CPrototypeFragment) {
            rules::correctness::c_prototype_fragment::c_prototype_fragment(self);
        }
        if self.is_rule_enabled(Rule::BareSlashMarker) {
            rules::correctness::bare_slash_marker::bare_slash_marker(self);
        }
        if self.is_rule_enabled(Rule::StatusCaptureAfterBranchTest) {
            rules::correctness::status_capture_after_branch_test::status_capture_after_branch_test(
                self,
            );
        }
        if self.is_rule_enabled(Rule::TemplateBraceInCommand) {
            rules::correctness::template_brace_in_command::template_brace_in_command(self);
        }
        if self.is_rule_enabled(Rule::NonShellSyntaxInScript) {
            rules::correctness::non_shell_syntax_in_script::non_shell_syntax_in_script(self);
        }
        if self.is_rule_enabled(Rule::ExportWithPositionalParams) {
            rules::correctness::export_with_positional_params::export_with_positional_params(self);
        }
        if self.is_rule_enabled(Rule::SetFlagsWithoutDashes) {
            rules::correctness::set_flags_without_dashes::set_flags_without_dashes(self);
        }
        if self.is_rule_enabled(Rule::QuotedArraySlice) {
            rules::correctness::quoted_array_slice::quoted_array_slice(self);
        }
        if self.is_rule_enabled(Rule::QuotedBashSource) {
            rules::correctness::quoted_bash_source::quoted_bash_source(self);
        }
        if self.is_rule_enabled(Rule::FindOrWithoutGrouping) {
            rules::correctness::find_or_without_grouping::find_or_without_grouping(self);
        }
        if self.is_rule_enabled(Rule::UnsetAssociativeArrayElement) {
            rules::correctness::unset_associative_array_element::unset_associative_array_element(
                self,
            );
        }
        if self.is_rule_enabled(Rule::MisspelledOptionName) {
            rules::correctness::misspelled_option_name::misspelled_option_name(self);
        }
        if self.is_rule_enabled(Rule::LocalVariableInSh) {
            rules::portability::local_variable_in_sh::local_variable_in_sh(self);
        }
        if self.is_rule_enabled(Rule::FunctionKeyword) {
            rules::portability::function_keyword::function_keyword(self);
        }
        if self.is_rule_enabled(Rule::BashCaseFallthrough) {
            rules::portability::bash_case_fallthrough::bash_case_fallthrough(self);
        }
        if self.is_rule_enabled(Rule::StandaloneArithmetic) {
            rules::portability::standalone_arithmetic::standalone_arithmetic(self);
        }
        if self.is_rule_enabled(Rule::SelectLoop) {
            rules::portability::select_loop::select_loop(self);
        }
        if self.is_rule_enabled(Rule::Coproc) {
            rules::portability::coproc::coproc(self);
        }
        if self.is_rule_enabled(Rule::CStyleForInSh) {
            rules::portability::c_style_for_in_sh::c_style_for_in_sh(self);
        }
        if self.is_rule_enabled(Rule::CStyleForArithmeticInSh) {
            rules::portability::c_style_for_arithmetic_in_sh::c_style_for_arithmetic_in_sh(self);
        }
        if self.is_rule_enabled(Rule::LetCommand) {
            rules::portability::let_command::let_command(self);
        }
        if self.is_rule_enabled(Rule::DeclareCommand) {
            rules::portability::declare_command::declare_command(self);
        }
        if self.is_rule_enabled(Rule::ArrayAssignment) {
            rules::portability::array_assignment::array_assignment(self);
        }
        if self.is_rule_enabled(Rule::PlusEqualsAppend) {
            rules::portability::plus_equals_append::plus_equals_append(self);
        }
        if self.is_rule_enabled(Rule::PlusEqualsInSh) {
            rules::portability::plus_equals_in_sh::plus_equals_in_sh(self);
        }
        if self.is_rule_enabled(Rule::ArrayKeysInSh) {
            rules::portability::array_keys_in_sh::array_keys_in_sh(self);
        }
        if self.is_rule_enabled(Rule::StarGlobRemovalInSh) {
            rules::portability::star_glob_removal_in_sh::star_glob_removal_in_sh(self);
        }
        if self.is_rule_enabled(Rule::IndirectExpansion) {
            rules::portability::indirect_expansion::indirect_expansion(self);
        }
        if self.is_rule_enabled(Rule::ArrayReference) {
            rules::portability::array_reference::array_reference(self);
        }
        if self.is_rule_enabled(Rule::SubstringExpansion) {
            rules::portability::substring_expansion::substring_expansion(self);
        }
        if self.is_rule_enabled(Rule::CaseModificationExpansion) {
            rules::portability::uppercase_expansion::uppercase_expansion(self);
        }
        if self.is_rule_enabled(Rule::ReplacementExpansion) {
            rules::portability::replacement_expansion::replacement_expansion(self);
        }
        if self.is_rule_enabled(Rule::TrapErr) {
            rules::portability::trap_err::trap_err(self);
        }
        if self.is_rule_enabled(Rule::PipefailOption) {
            rules::portability::pipefail_option::pipefail_option(self);
        }
        if self.is_rule_enabled(Rule::WaitOption) {
            rules::portability::wait_option::wait_option(self);
        }
        if self.is_rule_enabled(Rule::SourceBuiltinInSh) {
            rules::portability::source_builtin_in_sh::source_builtin_in_sh(self);
        }
        if self.is_rule_enabled(Rule::PrintfQFormatInSh) {
            rules::portability::printf_q_format_in_sh::printf_q_format_in_sh(self);
        }
        if self.is_rule_enabled(Rule::ErrexitTrapInSh) {
            rules::portability::errexit_trap_in_sh::errexit_trap_in_sh(self);
        }
        if self.is_rule_enabled(Rule::SignalNameInTrap) {
            rules::portability::signal_name_in_trap::signal_name_in_trap(self);
        }
        if self.is_rule_enabled(Rule::BasePrefixInArithmetic) {
            rules::portability::base_prefix_in_arithmetic::base_prefix_in_arithmetic(self);
        }
        if self.is_rule_enabled(Rule::FunctionKeywordInSh) {
            rules::portability::function_keyword_in_sh::function_keyword_in_sh(self);
        }
        if self.is_rule_enabled(Rule::SourceInsideFunctionInSh) {
            rules::portability::source_inside_function_in_sh::source_inside_function_in_sh(self);
        }
        if self.is_rule_enabled(Rule::ZshRedirPipe) {
            rules::portability::zsh_redir_pipe::zsh_redir_pipe(self);
        }
        if self.is_rule_enabled(Rule::SourcedWithArgs) {
            rules::portability::sourced_with_args::sourced_with_args(self);
        }
        if self.is_rule_enabled(Rule::CshSyntaxInSh) {
            rules::portability::csh_syntax_in_sh::csh_syntax_in_sh(self);
        }
        if self.is_rule_enabled(Rule::ZshAssignmentToZero) {
            rules::portability::zsh_assignment_to_zero::zsh_assignment_to_zero(self);
        }
    }

    fn check_word_and_expansion_facts(&mut self) {
        if self.is_rule_enabled(Rule::UnquotedExpansion) {
            rules::style::unquoted_expansion::unquoted_expansion(self);
        }
        if self.is_rule_enabled(Rule::UnquotedDollarStar) {
            rules::style::unquoted_dollar_star::unquoted_dollar_star(self);
        }
        if self.is_rule_enabled(Rule::QuotedDollarStarLoop) {
            rules::style::quoted_dollar_star_loop::quoted_dollar_star_loop(self);
        }
        if self.is_rule_enabled(Rule::UnquotedArraySplit) {
            rules::style::unquoted_array_split::unquoted_array_split(self);
        }
        if self.is_rule_enabled(Rule::CommandOutputArraySplit) {
            rules::style::command_output_array_split::command_output_array_split(self);
        }
        if self.is_rule_enabled(Rule::PositionalArgsInString) {
            rules::style::positional_args_in_string::positional_args_in_string(self);
        }
        if self.is_rule_enabled(Rule::UnquotedWordBetweenQuotes) {
            rules::style::unquoted_word_between_quotes::unquoted_word_between_quotes(self);
        }
        if self.is_rule_enabled(Rule::DoubleQuoteNesting) {
            rules::style::double_quote_nesting::double_quote_nesting(self);
        }
        if self.is_rule_enabled(Rule::EnvPrefixQuoting) {
            rules::style::env_prefix_quoting::env_prefix_quoting(self);
        }
        if self.is_rule_enabled(Rule::MixedQuoteWord) {
            rules::style::mixed_quote_word::mixed_quote_word(self);
        }
        if self.is_rule_enabled(Rule::UnquotedPipeInEcho) {
            rules::correctness::unquoted_pipe_in_echo::unquoted_pipe_in_echo(self);
        }
        if self.is_rule_enabled(Rule::DefaultValueInColonAssign) {
            rules::style::default_value_in_colon_assign::default_value_in_colon_assign(self);
        }
        if self.is_rule_enabled(Rule::EscapedUnderscore) {
            rules::style::escaped_underscore::escaped_underscore(self);
        }
        if self.is_rule_enabled(Rule::EscapedUnderscoreLiteral) {
            rules::style::escaped_underscore_literal::escaped_underscore_literal(self);
        }
        if self.is_rule_enabled(Rule::NeedlessBackslashUnderscore) {
            rules::style::needless_backslash_underscore::needless_backslash_underscore(self);
        }
        if self.is_rule_enabled(Rule::LiteralBackslash) {
            rules::style::literal_backslash::literal_backslash(self);
        }
        if self.is_rule_enabled(Rule::BackslashBeforeCommand) {
            rules::style::backslash_before_command::backslash_before_command(self);
        }
        if self.is_rule_enabled(Rule::AmpersandSemicolon) {
            rules::style::ampersand_semicolon::ampersand_semicolon(self);
        }
        if self.is_rule_enabled(Rule::UnquotedArrayExpansion) {
            rules::style::unquoted_array_expansion::unquoted_array_expansion(self);
        }
        if self.is_rule_enabled(Rule::TrapStringExpansion) {
            rules::correctness::trap_string_expansion::trap_string_expansion(self);
        }
        if self.is_rule_enabled(Rule::ConstantCaseSubject) {
            rules::correctness::constant_case_subject::constant_case_subject(self);
        }
        if self.is_rule_enabled(Rule::CasePatternVar) {
            rules::correctness::case_pattern_var::case_pattern_var(self);
        }
        if self.is_rule_enabled(Rule::PatternWithVariable) {
            rules::correctness::pattern_with_variable::pattern_with_variable(self);
        }
        if self.is_rule_enabled(Rule::UnquotedGlobsInFind) {
            rules::correctness::unquoted_globs_in_find::unquoted_globs_in_find(self);
        }
        if self.is_rule_enabled(Rule::GlobInFindSubstitution) {
            rules::correctness::glob_in_find_substitution::glob_in_find_substitution(self);
        }
        if self.is_rule_enabled(Rule::GlobInGrepPattern) {
            rules::correctness::glob_in_grep_pattern::glob_in_grep_pattern(self);
        }
        if self.is_rule_enabled(Rule::UnquotedGrepRegex) {
            rules::correctness::unquoted_grep_regex::unquoted_grep_regex(self);
        }
        if self.is_rule_enabled(Rule::GlobWithExpansionInLoop) {
            rules::correctness::glob_with_expansion_in_loop::glob_with_expansion_in_loop(self);
        }
        if self.is_rule_enabled(Rule::GlobAssignedToVariable) {
            rules::style::glob_assigned_to_variable::glob_assigned_to_variable(self);
        }
        if self.is_rule_enabled(Rule::ZshFlagExpansion) {
            rules::portability::zsh_flag_expansion::zsh_flag_expansion(self);
        }
        if self.is_rule_enabled(Rule::NestedZshSubstitution) {
            rules::portability::nested_zsh_substitution::nested_zsh_substitution(self);
        }
        if self.is_rule_enabled(Rule::ZshPromptBracket) {
            rules::portability::zsh_prompt_bracket::zsh_prompt_bracket(self);
        }
        if self.is_rule_enabled(Rule::ZshArraySubscriptInCase) {
            rules::portability::zsh_array_subscript_in_case::zsh_array_subscript_in_case(self);
        }
        if self.is_rule_enabled(Rule::ZshParameterFlag) {
            rules::portability::zsh_parameter_flag::zsh_parameter_flag(self);
        }
        if self.is_rule_enabled(Rule::ZshParameterIndexFlag) {
            rules::portability::zsh_parameter_index_flag::zsh_parameter_index_flag(self);
        }
        if self.is_rule_enabled(Rule::ZshNestedExpansion) {
            rules::portability::zsh_nested_expansion::zsh_nested_expansion(self);
        }
        if self.is_rule_enabled(Rule::MultiVarForLoop) {
            rules::portability::multi_var_for_loop::multi_var_for_loop(self);
        }
    }

    fn check_loop_list_and_pipeline_facts(&mut self) {
        if self.is_rule_enabled(Rule::SingleIterationLoop) {
            rules::style::single_iteration_loop::single_iteration_loop(self);
        }
        if self.is_rule_enabled(Rule::ConditionalAssignmentShortcut) {
            rules::style::conditional_assignment_shortcut::conditional_assignment_shortcut(self);
        }
        if self.is_rule_enabled(Rule::LoopFromCommandOutput) {
            rules::style::loop_from_command_output::loop_from_command_output(self);
        }
        if self.is_rule_enabled(Rule::PsGrepPipeline) {
            rules::style::ps_grep_pipeline::ps_grep_pipeline(self);
        }
        if self.is_rule_enabled(Rule::LsGrepPipeline) {
            rules::style::ls_grep_pipeline::ls_grep_pipeline(self);
        }
        if self.is_rule_enabled(Rule::LsPipedToXargs) {
            rules::style::ls_piped_to_xargs::ls_piped_to_xargs(self);
        }
        if self.is_rule_enabled(Rule::LsInSubstitution) {
            rules::style::ls_in_substitution::ls_in_substitution(self);
        }
        if self.is_rule_enabled(Rule::ChainedTestBranches) {
            rules::correctness::chained_test_branches::chained_test_branches(self);
        }
        if self.is_rule_enabled(Rule::ShortCircuitFallthrough) {
            rules::correctness::short_circuit_fallthrough::short_circuit_fallthrough(self);
        }
        if self.is_rule_enabled(Rule::DefaultElseInShortCircuit) {
            rules::correctness::default_else_in_short_circuit::default_else_in_short_circuit(self);
        }
        if self.is_rule_enabled(Rule::LineOrientedInput) {
            rules::correctness::line_oriented_input::line_oriented_input(self);
        }
        if self.is_rule_enabled(Rule::LeadingGlobArgument) {
            rules::correctness::leading_glob_argument::leading_glob_argument(self);
        }
        if self.is_rule_enabled(Rule::FindOutputToXargs) {
            rules::correctness::find_output_to_xargs::find_output_to_xargs(self);
        }
        if self.is_rule_enabled(Rule::FindOutputLoop) {
            rules::correctness::find_output_loop::find_output_loop(self);
        }
        if self.is_rule_enabled(Rule::LoopControlOutsideLoop) {
            rules::correctness::loop_control_outside_loop::loop_control_outside_loop(self);
        }
        if self.is_rule_enabled(Rule::ContinueOutsideLoopInFunction) {
            rules::correctness::continue_outside_loop_in_function::continue_outside_loop_in_function(
                self,
            );
        }
        if self.is_rule_enabled(Rule::VariableAsCommandName) {
            rules::correctness::variable_as_command_name::variable_as_command_name(self);
        }
        if self.is_rule_enabled(Rule::KeywordFunctionName) {
            rules::correctness::keyword_function_name::keyword_function_name(self);
        }
        if self.is_rule_enabled(Rule::PipeToKill) {
            rules::correctness::pipe_to_kill::pipe_to_kill(self);
        }
    }

    fn check_redirect_and_substitution_facts(&mut self) {
        if self.is_rule_enabled(Rule::UnquotedCommandSubstitution) {
            rules::style::unquoted_command_substitution::unquoted_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::EchoInsideCommandSubstitution) {
            rules::style::echo_inside_command_substitution::echo_inside_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::CommandSubstitutionInAlias) {
            rules::style::command_substitution_in_alias::command_substitution_in_alias(self);
        }
        if self.is_rule_enabled(Rule::BacktickOutputToCommand) {
            rules::style::backtick_output_to_command::backtick_output_to_command(self);
        }
        if self.is_rule_enabled(Rule::FunctionInAlias) {
            rules::style::function_in_alias::function_in_alias(self);
        }
        if self.is_rule_enabled(Rule::SudoRedirectionOrder) {
            rules::correctness::sudo_redirection_order::sudo_redirection_order(self);
        }
        if self.is_rule_enabled(Rule::ArithmeticRedirectionTarget) {
            rules::correctness::arithmetic_redirection_target::arithmetic_redirection_target(self);
        }
        if self.is_rule_enabled(Rule::BadRedirectionFdOrder) {
            rules::correctness::bad_redirection_fd_order::bad_redirection_fd_order(self);
        }
        if self.is_rule_enabled(Rule::AmpersandRedirection) {
            rules::portability::ampersand_redirection::ampersand_redirection(self);
        }
        if self.is_rule_enabled(Rule::ProcessSubstitution) {
            rules::portability::process_substitution::process_substitution(self);
        }
        if self.is_rule_enabled(Rule::BashFileSlurp) {
            rules::portability::bash_file_slurp::bash_file_slurp(self);
        }
        if self.is_rule_enabled(Rule::HereString) {
            rules::portability::here_string::here_string(self);
        }
        if self.is_rule_enabled(Rule::BraceFdRedirection) {
            rules::portability::brace_fd_redirection::brace_fd_redirection(self);
        }
        if self.is_rule_enabled(Rule::AmpersandRedirectInSh) {
            rules::portability::ampersand_redirect_in_sh::ampersand_redirect_in_sh(self);
        }
        if self.is_rule_enabled(Rule::PipeStderrInSh) {
            rules::portability::pipe_stderr_in_sh::pipe_stderr_in_sh(self);
        }
        if self.is_rule_enabled(Rule::SubstWithRedirect) {
            rules::correctness::subst_with_redirect::subst_with_redirect(self);
        }
        if self.is_rule_enabled(Rule::SubstWithRedirectErr) {
            rules::correctness::subst_with_redirect_err::subst_with_redirect_err(self);
        }
        if self.is_rule_enabled(Rule::RedirectToCommandName) {
            rules::correctness::redirect_to_command_name::redirect_to_command_name(self);
        }
        if self.is_rule_enabled(Rule::MapfileProcessSubstitution) {
            rules::correctness::mapfile_process_substitution::mapfile_process_substitution(self);
        }
        if self.is_rule_enabled(Rule::UnusedHeredoc) {
            rules::correctness::unused_heredoc::unused_heredoc(self);
        }
        if self.is_rule_enabled(Rule::HeredocMissingEnd) {
            rules::correctness::heredoc_missing_end::heredoc_missing_end(self);
        }
        if self.is_rule_enabled(Rule::HeredocCloserNotAlone) {
            rules::correctness::heredoc_closer_not_alone::heredoc_closer_not_alone(self);
        }
        if self.is_rule_enabled(Rule::MisquotedHeredocClose) {
            rules::correctness::misquoted_heredoc_close::misquoted_heredoc_close(self);
        }
        if self.is_rule_enabled(Rule::HeredocEndSpace) {
            rules::style::heredoc_end_space::heredoc_end_space(self);
        }
        if self.is_rule_enabled(Rule::SpacedTabstripClose) {
            rules::style::spaced_tabstrip_close::spaced_tabstrip_close(self);
        }
    }

    fn check_surface_fragment_facts(&mut self) {
        if self.is_rule_enabled(Rule::LegacyBackticks) {
            rules::style::legacy_backticks::legacy_backticks(self);
        }
        if self.is_rule_enabled(Rule::LegacyArithmeticExpansion) {
            rules::style::legacy_arithmetic_expansion::legacy_arithmetic_expansion(self);
        }
        if self.is_rule_enabled(Rule::LiteralBraces) {
            rules::style::literal_braces::literal_braces(self);
        }
        if self.is_rule_enabled(Rule::AnsiCQuoting) {
            rules::portability::ansi_c_quoting::ansi_c_quoting(self);
        }
        if self.is_rule_enabled(Rule::DollarStringInSh) {
            rules::portability::dollar_string_in_sh::dollar_string_in_sh(self);
        }
        if self.is_rule_enabled(Rule::BraceExpansion) {
            rules::portability::brace_expansion::brace_expansion(self);
        }
        if self.is_rule_enabled(Rule::LegacyArithmeticInSh) {
            rules::portability::legacy_arithmetic_in_sh::legacy_arithmetic_in_sh(self);
        }
        if self.is_rule_enabled(Rule::SubshellInArithmetic) {
            rules::correctness::subshell_in_arithmetic::subshell_in_arithmetic(self);
        }
        if self.is_rule_enabled(Rule::SingleQuoteBackslash) {
            rules::style::single_quote_backslash::single_quote_backslash(self);
        }
        if self.is_rule_enabled(Rule::LiteralBackslashInSingleQuotes) {
            rules::style::literal_backslash_in_single_quotes::literal_backslash_in_single_quotes(
                self,
            );
        }
        if self.is_rule_enabled(Rule::IfsEqualsAmbiguity) {
            rules::style::ifs_equals_ambiguity::ifs_equals_ambiguity(self);
        }
        if self.is_rule_enabled(Rule::SingleQuotedLiteral) {
            rules::correctness::single_quoted_literal::single_quoted_literal(self);
        }
        if self.is_rule_enabled(Rule::OpenDoubleQuote) {
            rules::correctness::open_double_quote::open_double_quote(self);
        }
        if self.is_rule_enabled(Rule::SuspectClosingQuote) {
            rules::style::suspect_closing_quote::suspect_closing_quote(self);
        }
        if self.is_rule_enabled(Rule::TrailingDirective) {
            rules::style::trailing_directive::trailing_directive(self);
        }
        if self.is_rule_enabled(Rule::PositionalTenBraces) {
            rules::correctness::positional_ten_braces::positional_ten_braces(self);
        }
        if self.is_rule_enabled(Rule::NestedParameterExpansion) {
            rules::correctness::nested_parameter_expansion::nested_parameter_expansion(self);
        }
        if self.is_rule_enabled(Rule::BackslashBeforeClosingBacktick) {
            rules::correctness::backslash_before_closing_backtick::backslash_before_closing_backtick(
                self,
            );
        }
        if self.is_rule_enabled(Rule::PositionalParamAsOperator) {
            rules::correctness::positional_param_as_operator::positional_param_as_operator(self);
        }
        if self.is_rule_enabled(Rule::DoubleParenGrouping) {
            rules::correctness::double_paren_grouping::double_paren_grouping(self);
        }
        if self.is_rule_enabled(Rule::UnicodeQuoteInString) {
            rules::correctness::unicode_quote_in_string::unicode_quote_in_string(self);
        }
        if self.is_rule_enabled(Rule::AssignmentLooksLikeComparison) {
            rules::correctness::assignment_looks_like_comparison::assignment_looks_like_comparison(
                self,
            );
        }
        if self.is_rule_enabled(Rule::AssignmentToNumericVariable) {
            rules::correctness::assignment_to_numeric_variable::assignment_to_numeric_variable(
                self,
            );
        }
        if self.is_rule_enabled(Rule::PlusPrefixInAssignment) {
            rules::correctness::plus_prefix_in_assignment::plus_prefix_in_assignment(self);
        }
        if self.is_rule_enabled(Rule::IfsSetToLiteralBackslashN) {
            rules::correctness::ifs_set_to_literal_backslash_n::ifs_set_to_literal_backslash_n(
                self,
            );
        }
        if self.is_rule_enabled(Rule::AppendWithEscapedQuotes) {
            rules::correctness::append_with_escaped_quotes::append_with_escaped_quotes(self);
        }
        if self.is_rule_enabled(Rule::LocalCrossReference) {
            rules::correctness::local_cross_reference::local_cross_reference(self);
        }
        if self.is_rule_enabled(Rule::SpacedAssignment) {
            rules::correctness::spaced_assignment::spaced_assignment(self);
        }
        if self.is_rule_enabled(Rule::BadVarName) {
            rules::correctness::bad_var_name::bad_var_name(self);
        }
        if self.is_rule_enabled(Rule::CommentedContinuationLine) {
            rules::correctness::commented_continuation_line::commented_continuation_line(self);
        }
        if self.is_rule_enabled(Rule::UnicodeSingleQuoteInSingleQuotes) {
            rules::correctness::unicode_single_quote_in_single_quotes::unicode_single_quote_in_single_quotes(self);
        }
    }

    fn check_test_and_conditional_facts(&mut self) {
        if self.is_rule_enabled(Rule::DoubleBracketInSh) {
            rules::portability::conditional_portability::double_bracket_in_sh(self);
        }
        if self.is_rule_enabled(Rule::GrepOutputInTest) {
            rules::style::grep_output_in_test::grep_output_in_test(self);
        }
        if self.is_rule_enabled(Rule::GlobInStringComparison) {
            rules::correctness::glob_in_string_comparison::glob_in_string_comparison(self);
        }
        if self.is_rule_enabled(Rule::UnquotedVariableInTest) {
            rules::style::unquoted_variable_in_test::unquoted_variable_in_test(self);
        }
        if self.is_rule_enabled(Rule::TestEqualityOperator) {
            rules::portability::conditional_portability::test_equality_operator(self);
        }
        if self.is_rule_enabled(Rule::IfElifBashTest) {
            rules::portability::conditional_portability::if_elif_bash_test(self);
        }
        if self.is_rule_enabled(Rule::ExtendedGlobInTest) {
            rules::portability::conditional_portability::extended_glob_in_test(self);
        }
        if self.is_rule_enabled(Rule::ExtglobInSh) {
            rules::portability::conditional_portability::extglob_in_sh(self);
        }
        if self.is_rule_enabled(Rule::CaretNegationInBracket) {
            rules::portability::conditional_portability::caret_negation_in_bracket(self);
        }
        if self.is_rule_enabled(Rule::ArraySubscriptTest) {
            rules::portability::conditional_portability::array_subscript_test(self);
        }
        if self.is_rule_enabled(Rule::ArraySubscriptCondition) {
            rules::portability::conditional_portability::array_subscript_condition(self);
        }
        if self.is_rule_enabled(Rule::ExtglobInTest) {
            rules::portability::conditional_portability::extglob_in_test(self);
        }
        if self.is_rule_enabled(Rule::GreaterThanInDoubleBracket) {
            rules::portability::conditional_portability::greater_than_in_double_bracket(self);
        }
        if self.is_rule_enabled(Rule::RegexMatchInSh) {
            rules::portability::conditional_portability::regex_match_in_sh(self);
        }
        if self.is_rule_enabled(Rule::VTestInSh) {
            rules::portability::conditional_portability::v_test_in_sh(self);
        }
        if self.is_rule_enabled(Rule::ATestInSh) {
            rules::portability::conditional_portability::a_test_in_sh(self);
        }
        if self.is_rule_enabled(Rule::OptionTestInSh) {
            rules::portability::conditional_portability::option_test_in_sh(self);
        }
        if self.is_rule_enabled(Rule::StickyBitTestInSh) {
            rules::portability::conditional_portability::sticky_bit_test_in_sh(self);
        }
        if self.is_rule_enabled(Rule::OwnershipTestInSh) {
            rules::portability::conditional_portability::ownership_test_in_sh(self);
        }
        if self.is_rule_enabled(Rule::QuotedBashRegex) {
            rules::correctness::quoted_bash_regex::quoted_bash_regex(self);
        }
        if self.is_rule_enabled(Rule::ConstantComparisonTest) {
            rules::correctness::constant_comparison_test::constant_comparison_test(self);
        }
        if self.is_rule_enabled(Rule::AtSignInStringCompare) {
            rules::correctness::at_sign_in_string_compare::at_sign_in_string_compare(self);
        }
        if self.is_rule_enabled(Rule::ArraySliceInComparison) {
            rules::correctness::array_slice_in_comparison::array_slice_in_comparison(self);
        }
        if self.is_rule_enabled(Rule::LiteralUnaryStringTest) {
            rules::correctness::literal_unary_string_test::literal_unary_string_test(self);
        }
        if self.is_rule_enabled(Rule::TruthyLiteralTest) {
            rules::correctness::truthy_literal_test::truthy_literal_test(self);
        }
        if self.is_rule_enabled(Rule::EscapedNegationInTest) {
            rules::correctness::escaped_negation_in_test::escaped_negation_in_test(self);
        }
        if self.is_rule_enabled(Rule::GreaterThanInTest) {
            rules::correctness::greater_than_in_test::greater_than_in_test(self);
        }
        if self.is_rule_enabled(Rule::EmptyTest) {
            rules::correctness::empty_test::empty_test(self);
        }
        if self.is_rule_enabled(Rule::BrokenTestEnd) {
            rules::correctness::broken_test_end::broken_test_end(self);
        }
        if self.is_rule_enabled(Rule::BrokenTestParse) {
            rules::correctness::broken_test_parse::broken_test_parse(self);
        }
        if self.is_rule_enabled(Rule::LinebreakInTest) {
            rules::correctness::linebreak_in_test::linebreak_in_test(self);
        }
        if self.is_rule_enabled(Rule::ElseIf) {
            rules::correctness::else_if::else_if(self);
        }
    }

    fn check_flow(&mut self) {
        if self.is_rule_enabled(Rule::NonAbsoluteShebang) {
            rules::correctness::non_absolute_shebang::non_absolute_shebang(self);
        }
        if self.is_rule_enabled(Rule::IfMissingThen) {
            rules::correctness::if_missing_then::if_missing_then(self);
        }
        if self.is_rule_enabled(Rule::ElseWithoutThen) {
            rules::correctness::else_without_then::else_without_then(self);
        }
        if self.is_rule_enabled(Rule::MissingSemicolonBeforeBrace) {
            rules::correctness::missing_semicolon_before_brace::missing_semicolon_before_brace(
                self,
            );
        }
        if self.is_rule_enabled(Rule::EmptyFunctionBody) {
            rules::correctness::empty_function_body::empty_function_body(self);
        }
        if self.is_rule_enabled(Rule::BareClosingBrace) {
            rules::correctness::bare_closing_brace::bare_closing_brace(self);
        }
        if self.is_rule_enabled(Rule::UnreachableAfterExit) {
            rules::correctness::unreachable_after_exit::unreachable_after_exit(self);
        }
    }
}
