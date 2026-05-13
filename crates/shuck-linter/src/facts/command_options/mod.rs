use super::*;

mod find;
mod grep;
mod read;
mod sed;
mod set;
mod shared;
mod shell_invocation;
mod xargs;

use self::{find::*, grep::*, read::*, sed::*, set::*, shared::*, shell_invocation::*, xargs::*};
pub(crate) use self::{
    sed::{
        SedScriptQuoteMode, find_sed_substitution_section, sed_has_single_substitution_script,
        sed_script_text,
    },
    shared::word_starts_with_literal_dash,
    shell_invocation::{shell_flag_contains_command_string, ssh_option_consumes_next_argument},
    xargs::{
        XargsShortOptionArgumentStyle, xargs_long_option_requires_separate_argument,
        xargs_short_option_argument_style,
    },
};

#[derive(Debug, Clone)]
pub struct PathWordFact<'a> {
    word: &'a Word,
    context: ExpansionContext,
    comparable_path: Option<ComparablePath>,
}

impl<'a> PathWordFact<'a> {
    pub(crate) fn new(
        word: &'a Word,
        context: ExpansionContext,
        source: &str,
        behavior: &ShellBehaviorAt<'_>,
    ) -> Self {
        Self {
            word,
            context,
            comparable_path: comparable_path(word, source, context, Some(behavior)),
        }
    }

    pub fn word(&self) -> &'a Word {
        self.word
    }

    pub fn context(&self) -> ExpansionContext {
        self.context
    }

    pub(crate) fn comparable_path(&self) -> Option<&ComparablePath> {
        self.comparable_path.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct ReadCommandFacts {
    pub uses_raw_input: bool,
    target_name_uses: Box<[ComparableNameUse]>,
    array_target_name_uses: Box<[ComparableNameUse]>,
}

impl ReadCommandFacts {
    pub(crate) fn target_name_uses(&self) -> &[ComparableNameUse] {
        &self.target_name_uses
    }

    pub(crate) fn array_target_name_uses(&self) -> &[ComparableNameUse] {
        &self.array_target_name_uses
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SuCommandFacts {
    has_login_flag: bool,
}

impl SuCommandFacts {
    pub fn has_login_flag(self) -> bool {
        self.has_login_flag
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EchoCommandFacts<'a> {
    portability_flag_word: Option<&'a Word>,
    uses_escape_interpreting_flag: bool,
}

impl<'a> EchoCommandFacts<'a> {
    pub fn portability_flag_word(self) -> Option<&'a Word> {
        self.portability_flag_word
    }

    pub fn uses_escape_interpreting_flag(self) -> bool {
        self.uses_escape_interpreting_flag
    }
}

#[derive(Debug, Clone)]
pub struct TrCommandFacts<'a> {
    operand_words: Box<[&'a Word]>,
}

impl<'a> TrCommandFacts<'a> {
    pub fn operand_words(&self) -> &[&'a Word] {
        &self.operand_words
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SedCommandFacts {
    has_single_substitution_script: bool,
}

impl SedCommandFacts {
    pub fn has_single_substitution_script(self) -> bool {
        self.has_single_substitution_script
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PrintfCommandFacts<'a> {
    pub format_word: Option<&'a Word>,
    pub format_word_has_literal_percent: bool,
    pub uses_q_format: bool,
}

#[derive(Debug, Clone)]
pub struct UnsetCommandFacts<'a> {
    pub function_mode: bool,
    nameref_mode: bool,
    operand_words: Box<[&'a Word]>,
    operand_facts: Box<[UnsetOperandFact<'a>]>,
    prefix_match_operand_spans: Box<[Span]>,
    options_parseable: bool,
}

impl<'a> UnsetCommandFacts<'a> {
    pub fn operand_words(&self) -> &[&'a Word] {
        &self.operand_words
    }

    pub fn prefix_match_operand_spans(&self) -> &[Span] {
        &self.prefix_match_operand_spans
    }

    pub(crate) fn operand_facts(&self) -> &[UnsetOperandFact<'a>] {
        &self.operand_facts
    }

    pub(crate) fn options_parseable(&self) -> bool {
        self.options_parseable
    }

    pub(crate) fn nameref_mode(&self) -> bool {
        self.nameref_mode
    }

    pub fn targets_function_name(&self, source: &str, target_name: &str) -> bool {
        if !self.function_mode || !self.options_parseable {
            return false;
        }

        for word in self.operand_words() {
            let Some(text) = static_word_text(word, source) else {
                return false;
            };

            if text == target_name {
                return true;
            }
        }

        false
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UnsetOperandFact<'a> {
    word: &'a Word,
    array_subscript: Option<UnsetArraySubscriptFact>,
}

impl<'a> UnsetOperandFact<'a> {
    pub(crate) fn word(&self) -> &'a Word {
        self.word
    }

    pub(crate) fn array_subscript(&self) -> Option<&UnsetArraySubscriptFact> {
        self.array_subscript.as_ref()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UnsetArraySubscriptFact;

#[derive(Debug, Clone)]
pub struct RmCommandFacts<'a> {
    dangerous_path_spans: Box<[Span]>,
    rootish_path_words: Box<[&'a Word]>,
    rootish_path_spans: OnceLock<Box<[Span]>>,
}

impl RmCommandFacts<'_> {
    pub fn dangerous_path_spans(&self) -> &[Span] {
        &self.dangerous_path_spans
    }

    pub fn rootish_path_spans(&self, source: &str) -> &[Span] {
        self.rootish_path_spans.get_or_init(|| {
            self.rootish_path_words
                .iter()
                .filter_map(|word| rm_path_is_rootish(word, source).then_some(word.span))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        })
    }
}

#[derive(Debug, Clone)]
pub struct ChmodCommandFacts<'a> {
    sensitive_path_words: Box<[&'a Word]>,
    world_writable_sensitive_path_spans: OnceLock<Box<[Span]>>,
}

impl ChmodCommandFacts<'_> {
    pub fn world_writable_sensitive_path_spans(&self, source: &str) -> &[Span] {
        self.world_writable_sensitive_path_spans.get_or_init(|| {
            self.sensitive_path_words
                .iter()
                .filter_map(|word| chmod_path_is_sensitive(word, source).then_some(word.span))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        })
    }
}

#[derive(Debug, Clone)]
pub struct SshCommandFacts {
    remote_command_arg_span: Span,
    local_expansion_spans: Box<[Span]>,
}

impl SshCommandFacts {
    pub fn remote_command_arg_span(&self) -> Span {
        self.remote_command_arg_span
    }

    pub fn local_expansion_spans(&self) -> &[Span] {
        &self.local_expansion_spans
    }
}

#[derive(Debug, Clone)]
pub struct FindCommandFacts {
    pub has_print0: bool,
    has_formatted_output_action: bool,
    or_without_grouping_spans: Box<[Span]>,
    or_without_grouping_fix_spans: Box<[FindOrWithoutGroupingFixSpan]>,
    glob_pattern_operand_spans: Box<[Span]>,
}

impl FindCommandFacts {
    pub fn has_formatted_output_action(&self) -> bool {
        self.has_formatted_output_action
    }

    pub fn or_without_grouping_spans(&self) -> &[Span] {
        &self.or_without_grouping_spans
    }

    pub fn or_without_grouping_fix_spans(&self) -> &[FindOrWithoutGroupingFixSpan] {
        &self.or_without_grouping_fix_spans
    }

    pub fn glob_pattern_operand_spans(&self) -> &[Span] {
        &self.glob_pattern_operand_spans
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FindOrWithoutGroupingFixSpan {
    pub diagnostic_span: Span,
    pub branch_start: Span,
    pub action_span: Span,
}

#[derive(Debug, Clone)]
pub struct FindExecCommandFacts {
    argument_word_spans: Box<[Span]>,
}

impl FindExecCommandFacts {
    pub fn argument_word_spans(&self) -> &[Span] {
        &self.argument_word_spans
    }
}

#[derive(Debug, Clone)]
pub struct FindExecShellCommandFacts {
    shell_command_spans: Box<[Span]>,
}

impl FindExecShellCommandFacts {
    pub fn shell_command_spans(&self) -> &[Span] {
        &self.shell_command_spans
    }
}

#[derive(Debug, Clone)]
pub struct MapfileCommandFacts {
    pub(crate) input_fd: Option<i32>,
    pub(crate) target_name_uses: Box<[ComparableNameUse]>,
}

impl MapfileCommandFacts {
    pub fn input_fd(&self) -> Option<i32> {
        self.input_fd
    }

    pub(crate) fn target_name_uses(&self) -> &[ComparableNameUse] {
        &self.target_name_uses
    }
}

#[derive(Debug, Clone)]
pub struct XargsCommandFacts<'a> {
    pub uses_null_input: bool,
    max_procs: Option<u64>,
    zero_digit_option_word: bool,
    inline_replace_options: Box<[XargsInlineReplaceOptionFact]>,
    command_operand_words: Box<[&'a Word]>,
    sc2267_default_replace_silent_shape: bool,
}

impl<'a> XargsCommandFacts<'a> {
    pub fn max_procs(&self) -> Option<u64> {
        self.max_procs
    }

    pub fn has_zero_digit_option_word(&self) -> bool {
        self.zero_digit_option_word
    }

    pub fn inline_replace_options(&self) -> &[XargsInlineReplaceOptionFact] {
        &self.inline_replace_options
    }

    pub fn inline_replace_option_spans(&self) -> impl Iterator<Item = Span> + '_ {
        self.inline_replace_options
            .iter()
            .map(XargsInlineReplaceOptionFact::span)
    }

    pub fn command_operand_words(&self) -> &[&'a Word] {
        &self.command_operand_words
    }

    pub fn has_sc2267_default_replace_silent_shape(&self) -> bool {
        self.sc2267_default_replace_silent_shape
    }
}

#[derive(Debug, Clone, Copy)]
pub struct XargsInlineReplaceOptionFact {
    span: Span,
    uses_default_replacement: bool,
}

impl XargsInlineReplaceOptionFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn uses_default_replacement(&self) -> bool {
        self.uses_default_replacement
    }
}

#[derive(Debug, Clone)]
pub struct WaitCommandFacts {
    pub(crate) option_spans: Box<[Span]>,
}

impl WaitCommandFacts {
    pub fn option_spans(&self) -> &[Span] {
        &self.option_spans
    }
}

#[derive(Debug, Clone)]
pub struct GrepCommandFacts<'a> {
    pub uses_only_matching: bool,
    pub uses_fixed_strings: bool,
    patterns: Box<[GrepPatternFact<'a>]>,
}

impl<'a> GrepCommandFacts<'a> {
    pub fn patterns(&self) -> &[GrepPatternFact<'a>] {
        &self.patterns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrepPatternSourceKind {
    ImplicitOperand,
    ShortOptionSeparate,
    ShortOptionAttached,
    LongOptionSeparate,
    LongOptionAttached,
}

impl GrepPatternSourceKind {
    pub fn uses_separate_pattern_word(self) -> bool {
        matches!(
            self,
            Self::ImplicitOperand | Self::ShortOptionSeparate | Self::LongOptionSeparate
        )
    }
}

#[derive(Debug, Clone)]
pub struct GrepPatternFact<'a> {
    word: &'a Word,
    static_text: Option<Box<str>>,
    source_kind: GrepPatternSourceKind,
    is_first_pattern: bool,
    follows_separate_option_argument: bool,
    starts_with_glob_style_star: bool,
    has_glob_style_star_confusion: bool,
    glob_style_star_replacement_spans: Box<[Span]>,
}

impl<'a> GrepPatternFact<'a> {
    pub fn word(&self) -> &'a Word {
        self.word
    }

    pub fn span(&self) -> Span {
        self.word.span
    }

    pub fn static_text(&self) -> Option<&str> {
        self.static_text.as_deref()
    }

    pub fn source_kind(&self) -> GrepPatternSourceKind {
        self.source_kind
    }

    pub fn is_first_pattern(&self) -> bool {
        self.is_first_pattern
    }

    pub fn follows_separate_option_argument(&self) -> bool {
        self.follows_separate_option_argument
    }

    pub fn starts_with_glob_style_star(&self) -> bool {
        self.starts_with_glob_style_star
    }

    pub fn has_glob_style_star_confusion(&self) -> bool {
        self.has_glob_style_star_confusion
    }

    pub fn glob_style_star_replacement_spans(&self) -> &[Span] {
        &self.glob_style_star_replacement_spans
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PsCommandFacts {
    pub has_pid_selector: bool,
}

#[derive(Debug, Clone)]
pub struct SetCommandFacts {
    pub errexit_change: Option<bool>,
    pub errtrace_change: Option<bool>,
    pub functrace_change: Option<bool>,
    pub pipefail_change: Option<bool>,
    resets_positional_parameters: bool,
    errtrace_flag_spans: Box<[Span]>,
    functrace_flag_spans: Box<[Span]>,
    pipefail_option_spans: Box<[Span]>,
    non_posix_option_spans: Box<[Span]>,
    flags_without_prefix_spans: Box<[Span]>,
}

impl SetCommandFacts {
    pub fn resets_positional_parameters(&self) -> bool {
        self.resets_positional_parameters
    }

    pub fn errtrace_flag_spans(&self) -> &[Span] {
        &self.errtrace_flag_spans
    }

    pub fn functrace_flag_spans(&self) -> &[Span] {
        &self.functrace_flag_spans
    }

    pub fn pipefail_option_spans(&self) -> &[Span] {
        &self.pipefail_option_spans
    }

    pub fn non_posix_option_spans(&self) -> &[Span] {
        &self.non_posix_option_spans
    }

    pub fn flags_without_prefix_spans(&self) -> &[Span] {
        &self.flags_without_prefix_spans
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryChangeCommandKind {
    Cd,
    Pushd,
    Popd,
}

impl DirectoryChangeCommandKind {
    pub fn command_name(self) -> &'static str {
        match self {
            Self::Cd => "cd",
            Self::Pushd => "pushd",
            Self::Popd => "popd",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DirectoryChangeCommandFacts {
    kind: DirectoryChangeCommandKind,
    plain_directory_stack_marker: bool,
    manual_restore_candidate: bool,
}

impl DirectoryChangeCommandFacts {
    pub fn kind(&self) -> DirectoryChangeCommandKind {
        self.kind
    }

    pub fn command_name(&self) -> &'static str {
        self.kind.command_name()
    }

    pub fn is_plain_directory_stack_marker(&self) -> bool {
        self.plain_directory_stack_marker
    }

    pub fn is_manual_restore_candidate(&self) -> bool {
        self.manual_restore_candidate
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FunctionPositionalParameterFacts {
    pub(crate) required_arg_count: usize,
    pub(crate) uses_unprotected_positional_parameters: bool,
    pub(crate) resets_positional_parameters: bool,
}

impl FunctionPositionalParameterFacts {
    pub fn required_arg_count(&self) -> usize {
        self.required_arg_count
    }

    pub fn uses_positional_parameters(&self) -> bool {
        self.uses_unprotected_positional_parameters
    }

    pub fn resets_positional_parameters(&self) -> bool {
        self.resets_positional_parameters
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprStringHelperKind {
    Length,
    Index,
    Match,
    Substr,
}

#[derive(Debug, Clone, Copy)]
pub struct ExprCommandFacts {
    pub uses_arithmetic_operator: bool,
    pub(crate) string_helper_kind: Option<ExprStringHelperKind>,
    pub(crate) string_helper_span: Option<Span>,
}

impl ExprCommandFacts {
    pub fn uses_arithmetic_operator(self) -> bool {
        self.uses_arithmetic_operator
    }

    pub fn string_helper_kind(self) -> Option<ExprStringHelperKind> {
        self.string_helper_kind
    }

    pub fn string_helper_span(self) -> Option<Span> {
        self.string_helper_span
    }

    pub fn uses_substr_string_form(self) -> bool {
        self.string_helper_kind == Some(ExprStringHelperKind::Substr)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExitCommandFacts<'a> {
    pub status_word: Option<&'a Word>,
    pub is_numeric_literal: bool,
    pub(crate) status_is_static: bool,
    pub(crate) status_has_literal_content: bool,
}

impl<'a> ExitCommandFacts<'a> {
    pub fn has_static_status(self) -> bool {
        self.status_is_static
    }

    pub fn has_invalid_status_argument(self) -> bool {
        self.status_word.is_some()
            && !self.is_numeric_literal
            && (self.status_is_static || self.status_has_literal_content)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SudoFamilyCommandFacts {
    pub invoker: SudoFamilyInvoker,
}

#[derive(Debug, Clone, Default)]
pub struct CommandOptionFacts<'a> {
    rm: Option<RmCommandFacts<'a>>,
    chmod: Option<ChmodCommandFacts<'a>>,
    ssh: Option<SshCommandFacts>,
    read: Option<ReadCommandFacts>,
    su: Option<SuCommandFacts>,
    echo: Option<EchoCommandFacts<'a>>,
    sed: Option<SedCommandFacts>,
    tr: Option<TrCommandFacts<'a>>,
    printf: Option<PrintfCommandFacts<'a>>,
    unset: Option<UnsetCommandFacts<'a>>,
    find: Option<FindCommandFacts>,
    find_exec: Option<FindExecCommandFacts>,
    find_exec_shell: Option<FindExecShellCommandFacts>,
    mapfile: Option<MapfileCommandFacts>,
    xargs: Option<XargsCommandFacts<'a>>,
    wait: Option<WaitCommandFacts>,
    grep: Option<GrepCommandFacts<'a>>,
    ps: Option<PsCommandFacts>,
    set: Option<SetCommandFacts>,
    directory_change: Option<DirectoryChangeCommandFacts>,
    expr: Option<ExprCommandFacts>,
    exit: Option<ExitCommandFacts<'a>>,
    sudo_family: Option<SudoFamilyCommandFacts>,
    nonportable_sh_builtin_option_span: Option<Span>,
    file_operand_words: Box<[&'a Word]>,
}

#[derive(Clone, Copy)]
pub struct CommandOptionFactsRef<'facts, 'a> {
    inner: Option<&'facts CommandOptionFacts<'a>>,
}

impl<'facts, 'a> CommandOptionFactsRef<'facts, 'a> {
    pub(crate) fn new(inner: Option<&'facts CommandOptionFacts<'a>>) -> Self {
        Self { inner }
    }

    pub fn rm(self) -> Option<&'facts RmCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::rm)
    }

    pub fn chmod(self) -> Option<&'facts ChmodCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::chmod)
    }

    pub fn ssh(self) -> Option<&'facts SshCommandFacts> {
        self.inner.and_then(CommandOptionFacts::ssh)
    }

    pub fn read(self) -> Option<&'facts ReadCommandFacts> {
        self.inner.and_then(CommandOptionFacts::read)
    }

    pub fn su(self) -> Option<&'facts SuCommandFacts> {
        self.inner.and_then(CommandOptionFacts::su)
    }

    pub fn echo(self) -> Option<&'facts EchoCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::echo)
    }

    pub fn sed(self) -> Option<&'facts SedCommandFacts> {
        self.inner.and_then(CommandOptionFacts::sed)
    }

    pub fn tr(self) -> Option<&'facts TrCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::tr)
    }

    pub fn printf(self) -> Option<&'facts PrintfCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::printf)
    }

    pub fn unset(self) -> Option<&'facts UnsetCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::unset)
    }

    pub fn find(self) -> Option<&'facts FindCommandFacts> {
        self.inner.and_then(CommandOptionFacts::find)
    }

    pub fn find_exec(self) -> Option<&'facts FindExecCommandFacts> {
        self.inner.and_then(CommandOptionFacts::find_exec)
    }

    pub fn find_exec_shell(self) -> Option<&'facts FindExecShellCommandFacts> {
        self.inner.and_then(CommandOptionFacts::find_exec_shell)
    }

    pub fn mapfile(self) -> Option<&'facts MapfileCommandFacts> {
        self.inner.and_then(CommandOptionFacts::mapfile)
    }

    pub fn xargs(self) -> Option<&'facts XargsCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::xargs)
    }

    pub fn wait(self) -> Option<&'facts WaitCommandFacts> {
        self.inner.and_then(CommandOptionFacts::wait)
    }

    pub fn grep(self) -> Option<&'facts GrepCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::grep)
    }

    pub fn ps(self) -> Option<&'facts PsCommandFacts> {
        self.inner.and_then(CommandOptionFacts::ps)
    }

    pub fn set(self) -> Option<&'facts SetCommandFacts> {
        self.inner.and_then(CommandOptionFacts::set)
    }

    pub fn directory_change(self) -> Option<&'facts DirectoryChangeCommandFacts> {
        self.inner.and_then(CommandOptionFacts::directory_change)
    }

    pub fn expr(self) -> Option<&'facts ExprCommandFacts> {
        self.inner.and_then(CommandOptionFacts::expr)
    }

    pub fn exit(self) -> Option<&'facts ExitCommandFacts<'a>> {
        self.inner.and_then(CommandOptionFacts::exit)
    }

    pub fn sudo_family(self) -> Option<&'facts SudoFamilyCommandFacts> {
        self.inner.and_then(CommandOptionFacts::sudo_family)
    }

    pub fn nonportable_sh_builtin_option_span(self) -> Option<Span> {
        self.inner
            .and_then(CommandOptionFacts::nonportable_sh_builtin_option_span)
    }

    pub fn file_operand_words(self) -> &'facts [&'a Word] {
        self.inner
            .map_or(&[], CommandOptionFacts::file_operand_words)
    }
}

impl<'a> CommandOptionFacts<'a> {
    pub fn rm(&self) -> Option<&RmCommandFacts<'a>> {
        self.rm.as_ref()
    }

    pub fn chmod(&self) -> Option<&ChmodCommandFacts<'a>> {
        self.chmod.as_ref()
    }

    pub fn ssh(&self) -> Option<&SshCommandFacts> {
        self.ssh.as_ref()
    }

    pub fn read(&self) -> Option<&ReadCommandFacts> {
        self.read.as_ref()
    }

    pub fn su(&self) -> Option<&SuCommandFacts> {
        self.su.as_ref()
    }

    pub fn echo(&self) -> Option<&EchoCommandFacts<'a>> {
        self.echo.as_ref()
    }

    pub fn sed(&self) -> Option<&SedCommandFacts> {
        self.sed.as_ref()
    }

    pub fn tr(&self) -> Option<&TrCommandFacts<'a>> {
        self.tr.as_ref()
    }

    pub fn printf(&self) -> Option<&PrintfCommandFacts<'a>> {
        self.printf.as_ref()
    }

    pub fn unset(&self) -> Option<&UnsetCommandFacts<'a>> {
        self.unset.as_ref()
    }

    pub fn find(&self) -> Option<&FindCommandFacts> {
        self.find.as_ref()
    }

    pub fn find_exec(&self) -> Option<&FindExecCommandFacts> {
        self.find_exec.as_ref()
    }

    pub fn find_exec_shell(&self) -> Option<&FindExecShellCommandFacts> {
        self.find_exec_shell.as_ref()
    }

    pub fn mapfile(&self) -> Option<&MapfileCommandFacts> {
        self.mapfile.as_ref()
    }

    pub fn xargs(&self) -> Option<&XargsCommandFacts<'a>> {
        self.xargs.as_ref()
    }

    pub fn wait(&self) -> Option<&WaitCommandFacts> {
        self.wait.as_ref()
    }

    pub fn grep(&self) -> Option<&GrepCommandFacts<'a>> {
        self.grep.as_ref()
    }

    pub fn ps(&self) -> Option<&PsCommandFacts> {
        self.ps.as_ref()
    }

    pub fn set(&self) -> Option<&SetCommandFacts> {
        self.set.as_ref()
    }

    pub fn directory_change(&self) -> Option<&DirectoryChangeCommandFacts> {
        self.directory_change.as_ref()
    }

    pub fn expr(&self) -> Option<&ExprCommandFacts> {
        self.expr.as_ref()
    }

    pub fn exit(&self) -> Option<&ExitCommandFacts<'a>> {
        self.exit.as_ref()
    }

    pub fn sudo_family(&self) -> Option<&SudoFamilyCommandFacts> {
        self.sudo_family.as_ref()
    }

    pub fn nonportable_sh_builtin_option_span(&self) -> Option<Span> {
        self.nonportable_sh_builtin_option_span
    }

    pub fn file_operand_words(&self) -> &[&'a Word] {
        &self.file_operand_words
    }

    pub(crate) fn into_sparse(self) -> Option<Box<Self>> {
        (!self.is_empty()).then(|| Box::new(self))
    }

    fn is_empty(&self) -> bool {
        self.rm.is_none()
            && self.chmod.is_none()
            && self.ssh.is_none()
            && self.read.is_none()
            && self.su.is_none()
            && self.echo.is_none()
            && self.sed.is_none()
            && self.tr.is_none()
            && self.printf.is_none()
            && self.unset.is_none()
            && self.find.is_none()
            && self.find_exec.is_none()
            && self.find_exec_shell.is_none()
            && self.mapfile.is_none()
            && self.xargs.is_none()
            && self.wait.is_none()
            && self.grep.is_none()
            && self.ps.is_none()
            && self.set.is_none()
            && self.directory_change.is_none()
            && self.expr.is_none()
            && self.exit.is_none()
            && self.sudo_family.is_none()
            && self.nonportable_sh_builtin_option_span.is_none()
            && self.file_operand_words.is_empty()
    }

    #[cfg_attr(shuck_profiling, inline(never))]
    pub(crate) fn build(
        command: &'a Command,
        normalized: &NormalizedCommand<'a>,
        semantic: &LinterSemanticArtifacts<'a>,
        source: &str,
        behavior: &ShellBehaviorAt<'_>,
    ) -> Self {
        Self {
            rm: normalized
                .literal_name
                .as_deref()
                .is_some_and(|name| name == "rm" && normalized.wrappers.is_empty())
                .then(|| parse_rm_command(normalized.body_args(), source))
                .flatten(),
            chmod: normalized
                .effective_name_is("chmod")
                .then(|| parse_chmod_command(normalized.body_args(), source))
                .flatten(),
            ssh: (normalized.effective_name_is("ssh") && normalized.wrappers.is_empty())
                .then(|| parse_ssh_command(normalized.body_args(), source))
                .flatten(),
            read: normalized
                .effective_name_is("read")
                .then(|| ReadCommandFacts {
                    uses_raw_input: read_uses_raw_input(normalized.body_args(), source),
                    target_name_uses: read_target_name_uses(
                        normalized.body_args(),
                        semantic,
                        source,
                    ),
                    array_target_name_uses: read_array_target_name_uses(
                        normalized.body_args(),
                        semantic,
                        source,
                    ),
                }),
            su: normalized
                .effective_name_is("su")
                .then(|| parse_su_command(normalized.body_args(), source)),
            echo: normalized
                .effective_basename_is("echo")
                .then(|| parse_echo_command(normalized.body_args(), source)),
            sed: normalized
                .effective_name_is("sed")
                .then(|| parse_sed_command(normalized.body_args(), source)),
            tr: (normalized.effective_name_is("tr") && normalized.wrappers.is_empty())
                .then(|| parse_tr_command(normalized.body_args(), source)),
            printf: normalized.effective_name_is("printf").then(|| {
                let format_word = printf_format_word(normalized.body_args(), source);
                PrintfCommandFacts {
                    format_word_has_literal_percent: format_word
                        .is_some_and(|word| printf_format_word_has_literal_percent(word, source)),
                    uses_q_format: format_word
                        .is_some_and(|word| printf_uses_q_format(word, source)),
                    format_word,
                }
            }),
            unset: normalized
                .effective_name_is("unset")
                .then(|| parse_unset_command(normalized.body_args(), source)),
            find: (normalized.effective_name_is("find")
                || normalized.literal_name.as_deref() == Some("find"))
            .then(|| {
                parse_find_command(
                    find_command_args(command, normalized, source),
                    source,
                    behavior,
                )
            }),
            find_exec: (normalized.has_wrapper(WrapperKind::FindExec)
                || normalized.has_wrapper(WrapperKind::FindExecDir))
            .then(|| FindExecCommandFacts {
                argument_word_spans: parse_find_exec_argument_word_spans(command, source)
                    .into_boxed_slice(),
            }),
            find_exec_shell: (normalized.has_wrapper(WrapperKind::FindExec)
                || normalized.has_wrapper(WrapperKind::FindExecDir))
            .then(|| parse_find_exec_shell_command(command, source))
            .flatten(),
            mapfile: (normalized.effective_name_is("mapfile")
                || normalized.effective_name_is("readarray"))
            .then(|| parse_mapfile_command(normalized.body_args(), semantic, source)),
            xargs: normalized
                .effective_name_is("xargs")
                .then(|| parse_xargs_command(normalized.body_args(), source)),
            wait: normalized
                .effective_name_is("wait")
                .then(|| parse_wait_command(normalized.body_args(), source)),
            grep: normalized
                .effective_name_is("grep")
                .then(|| parse_grep_command(normalized.body_args(), source))
                .flatten(),
            ps: normalized
                .effective_name_is("ps")
                .then(|| parse_ps_command(normalized.body_args(), source)),
            set: normalized
                .effective_name_is("set")
                .then(|| parse_set_command(normalized.body_args(), source)),
            directory_change: parse_directory_change_command(normalized, source),
            expr: normalized
                .effective_name_is("expr")
                .then_some(())
                .and_then(|_| parse_expr_command(normalized.body_args(), source)),
            exit: parse_exit_command(command, source),
            sudo_family: normalized.has_wrapper(WrapperKind::SudoFamily).then(|| {
                let Some(invoker) = detect_sudo_family_invoker(command, normalized, source) else {
                    unreachable!("sudo-family wrapper should preserve its invoker");
                };
                SudoFamilyCommandFacts { invoker }
            }),
            nonportable_sh_builtin_option_span: first_nonportable_sh_builtin_option_span(
                normalized, source,
            ),
            file_operand_words: same_command_file_operand_words(
                normalized.effective_or_literal_name(),
                normalized.body_args(),
                source,
            ),
        }
    }
}

fn parse_echo_command<'a>(args: &[&'a Word], source: &str) -> EchoCommandFacts<'a> {
    let mut portability_flag_word = None;
    let mut uses_escape_interpreting_flag = false;

    for word in args {
        if !classify_word(word, source).is_fixed_literal() {
            break;
        }

        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if !is_echo_portability_flag(text.as_ref()) {
            break;
        }

        portability_flag_word.get_or_insert(*word);
        uses_escape_interpreting_flag |= text.contains('e');
    }

    EchoCommandFacts {
        portability_flag_word,
        uses_escape_interpreting_flag,
    }
}

fn parse_tr_command<'a>(args: &[&'a Word], source: &str) -> TrCommandFacts<'a> {
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !is_tr_option(text.as_ref()) {
            break;
        }

        index += 1;
    }

    TrCommandFacts {
        operand_words: args[index..].iter().copied().collect(),
    }
}

fn is_tr_option(text: &str) -> bool {
    text.starts_with('-')
        && text != "-"
        && !text.starts_with("--")
        && text[1..]
            .bytes()
            .all(|byte| matches!(byte, b'c' | b'C' | b'd' | b's' | b't'))
}

fn parse_chmod_command<'a>(args: &[&'a Word], source: &str) -> Option<ChmodCommandFacts<'a>> {
    let mode_index = chmod_mode_index(args, source)?;
    let mode = static_word_text(args[mode_index], source)?;

    if !chmod_mode_makes_other_writable(mode.as_ref()) {
        return None;
    }

    let sensitive_path_words = args[mode_index + 1..]
        .iter()
        .copied()
        .filter(|word| chmod_path_may_be_sensitive(word, source))
        .collect::<Vec<_>>();

    (!sensitive_path_words.is_empty()).then_some(ChmodCommandFacts {
        sensitive_path_words: sensitive_path_words.into_boxed_slice(),
        world_writable_sensitive_path_spans: OnceLock::new(),
    })
}

fn chmod_mode_index(args: &[&Word], source: &str) -> Option<usize> {
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let text = static_word_text(word, source)?;

        if text == "--" {
            index += 1;
            break;
        }

        if text == "--reference" || text.starts_with("--reference=") {
            return None;
        }

        if chmod_text_is_option(text.as_ref()) {
            index += 1;
            continue;
        }

        break;
    }

    (index < args.len()).then_some(index)
}

fn chmod_text_is_option(text: &str) -> bool {
    matches!(
        text,
        "--changes"
            | "--silent"
            | "--quiet"
            | "--verbose"
            | "--recursive"
            | "--preserve-root"
            | "--no-preserve-root"
            | "--help"
            | "--version"
    ) || (text.starts_with('-')
        && !text.starts_with("--")
        && text.len() > 1
        && text[1..]
            .bytes()
            .all(|byte| matches!(byte, b'R' | b'c' | b'f' | b'v' | b'H' | b'L' | b'P')))
}

fn chmod_mode_makes_other_writable(mode: &str) -> bool {
    chmod_numeric_mode_makes_other_writable(mode) || chmod_symbolic_mode_makes_other_writable(mode)
}

fn chmod_numeric_mode_makes_other_writable(mode: &str) -> bool {
    let (operator, digits) = match mode.as_bytes().first().copied() {
        Some(operator @ (b'+' | b'=' | b'-')) => (Some(operator), &mode[1..]),
        Some(_) => (None, mode),
        None => return false,
    };

    if digits.is_empty() || !digits.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        return false;
    }

    let sets_other_write = digits
        .bytes()
        .last()
        .and_then(|byte| char::from(byte).to_digit(8))
        .is_some_and(|digit| digit & 0o2 != 0);

    matches!(operator, None | Some(b'+' | b'=')) && sets_other_write
}

fn chmod_symbolic_mode_makes_other_writable(mode: &str) -> bool {
    let mut state = ChmodSymbolicWriteState::default();
    for clause in mode.split(',') {
        state.apply_clause(clause);
    }

    state.other == Some(true)
}

#[derive(Debug, Clone, Copy, Default)]
struct ChmodSymbolicWriteState {
    user: Option<bool>,
    group: Option<bool>,
    other: Option<bool>,
}

impl ChmodSymbolicWriteState {
    fn apply_clause(&mut self, clause: &str) {
        let Some(operator_index) = clause.find(['+', '=', '-']) else {
            return;
        };

        let (who, rest) = clause.split_at(operator_index);
        let Some(operator) = rest.chars().next() else {
            return;
        };
        let permissions = &rest[operator.len_utf8()..];
        let targets = chmod_symbolic_targets(who);
        if !targets.targets_any() {
            return;
        }

        let write_value = self.permissions_write_value(permissions);
        match operator {
            '+' => {
                if write_value == Some(true) {
                    self.set_targets(targets, Some(true));
                }
            }
            '-' => match write_value {
                Some(true) => self.set_targets(targets, Some(false)),
                None => self.set_targets(targets, None),
                Some(false) => {}
            },
            '=' => self.set_targets(targets, write_value),
            _ => {}
        }
    }

    fn permissions_write_value(self, permissions: &str) -> Option<bool> {
        if permissions.contains('w') {
            return Some(true);
        }

        let mut saw_copy = false;
        for byte in permissions.bytes() {
            let source = match byte {
                b'u' => self.user,
                b'g' => self.group,
                b'o' => self.other,
                _ => continue,
            };
            saw_copy = true;
            match source {
                Some(true) => return Some(true),
                None => return None,
                Some(false) => {}
            }
        }

        Some(false).filter(|_| !saw_copy || !permissions.is_empty())
    }

    fn set_targets(&mut self, targets: ChmodSymbolicTargets, value: Option<bool>) {
        if targets.user {
            self.user = value;
        }
        if targets.group {
            self.group = value;
        }
        if targets.other {
            self.other = value;
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ChmodSymbolicTargets {
    user: bool,
    group: bool,
    other: bool,
}

impl ChmodSymbolicTargets {
    fn targets_any(self) -> bool {
        self.user || self.group || self.other
    }
}

fn chmod_symbolic_targets(who: &str) -> ChmodSymbolicTargets {
    let all = who.bytes().any(|byte| byte == b'a');
    ChmodSymbolicTargets {
        user: all || who.bytes().any(|byte| byte == b'u'),
        group: all || who.bytes().any(|byte| byte == b'g'),
        other: all || who.bytes().any(|byte| byte == b'o'),
    }
}

#[derive(Debug, Default)]
struct ChmodPathSegment {
    text: String,
    has_literal_text: bool,
    dynamic_count: usize,
    home_param_count: usize,
    has_other_dynamic: bool,
}

fn chmod_path_is_sensitive(word: &Word, source: &str) -> bool {
    let segments = chmod_path_segments(word, source);

    if chmod_path_has_absolute_root(&segments) {
        return chmod_absolute_path_segments_are_sensitive(&segments[1..]);
    }

    let Some((first, tail)) = segments.split_first() else {
        return false;
    };

    (chmod_segment_is_home_parameter(first)
        || chmod_segment_is_unquoted_home_tilde(first, word, source))
        && chmod_home_path_tail_is_sensitive(tail)
}

fn chmod_path_may_be_sensitive(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    text.contains('/')
        || text.contains('~')
        || text.contains("HOME")
        || text.contains(".ssh")
        || text.contains(".gnupg")
        || text.contains(".aws")
        || text.contains(".kube")
        || text.contains(".docker")
        || text.contains(".netrc")
}

fn chmod_path_segments(word: &Word, source: &str) -> Vec<ChmodPathSegment> {
    let mut segments = vec![ChmodPathSegment::default()];
    append_chmod_path_segments(&mut segments, &word.parts, source);
    segments
}

fn append_chmod_path_segments(
    segments: &mut Vec<ChmodPathSegment>,
    parts: &[WordPartNode],
    source: &str,
) {
    for part in parts {
        append_chmod_path_part(segments, &part.kind, part.span, source);
    }
}

fn append_chmod_path_part(
    segments: &mut Vec<ChmodPathSegment>,
    part: &WordPart,
    span: Span,
    source: &str,
) {
    match part {
        WordPart::Literal(text) => append_chmod_path_literal(segments, text.as_str(source, span)),
        WordPart::SingleQuoted {
            value,
            dollar: false,
        } => append_chmod_path_literal(segments, value.slice(source)),
        WordPart::DoubleQuoted { parts, .. } => append_chmod_path_segments(segments, parts, source),
        WordPart::Variable(name) => mark_chmod_path_dynamic(segments, name.as_str() == "HOME"),
        WordPart::Parameter(parameter) => {
            mark_chmod_path_dynamic(segments, chmod_parameter_is_home_access(parameter));
        }
        _ => {
            current_chmod_path_segment(segments).has_other_dynamic = true;
        }
    }
}

fn append_chmod_path_literal(segments: &mut Vec<ChmodPathSegment>, text: &str) {
    for character in text.chars() {
        if character == '/' {
            segments.push(ChmodPathSegment::default());
            continue;
        }

        let segment = current_chmod_path_segment(segments);
        segment.has_literal_text = true;
        segment.text.push(character);
    }
}

fn current_chmod_path_segment(segments: &mut [ChmodPathSegment]) -> &mut ChmodPathSegment {
    let Some(segment) = segments.last_mut() else {
        unreachable!("chmod path segments always start non-empty");
    };
    segment
}

fn mark_chmod_path_dynamic(segments: &mut [ChmodPathSegment], is_home: bool) {
    let segment = current_chmod_path_segment(segments);
    segment.dynamic_count += 1;
    segment.home_param_count += usize::from(is_home);
}

fn chmod_parameter_is_home_access(parameter: &ParameterExpansion) -> bool {
    matches!(
        &parameter.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.name.as_str() == "HOME" && reference.subscript.is_none()
    )
}

fn chmod_path_has_absolute_root(segments: &[ChmodPathSegment]) -> bool {
    segments.first().is_some_and(chmod_segment_is_empty)
}

fn chmod_segment_is_empty(segment: &ChmodPathSegment) -> bool {
    segment.text.is_empty()
        && !segment.has_literal_text
        && segment.dynamic_count == 0
        && !segment.has_other_dynamic
}

fn chmod_segment_is_home_parameter(segment: &ChmodPathSegment) -> bool {
    segment.home_param_count == 1
        && segment.dynamic_count == 1
        && !segment.has_literal_text
        && !segment.has_other_dynamic
        && segment.text.is_empty()
}

fn chmod_segment_is_unquoted_home_tilde(
    segment: &ChmodPathSegment,
    word: &Word,
    source: &str,
) -> bool {
    word.span.slice(source).starts_with('~')
        && segment.dynamic_count == 0
        && !segment.has_other_dynamic
        && chmod_tilde_prefix_is_home_root(&segment.text)
}

fn chmod_tilde_prefix_is_home_root(text: &str) -> bool {
    let Some(suffix) = text.strip_prefix('~') else {
        return false;
    };

    if suffix.starts_with(['+', '-']) {
        return false;
    }

    !suffix.contains(['*', '?', '[', ']', '{', '}'])
}

fn chmod_absolute_path_segments_are_sensitive(segments: &[ChmodPathSegment]) -> bool {
    let Some(components) = chmod_static_path_components(segments) else {
        return false;
    };

    path_components_match_any_prefix(&components, CHMOD_SENSITIVE_ABSOLUTE_PREFIXES)
}

fn chmod_home_path_tail_is_sensitive(segments: &[ChmodPathSegment]) -> bool {
    let Some(components) = chmod_static_path_components(segments) else {
        return false;
    };

    components.is_empty()
        || path_components_match_any_prefix(&components, CHMOD_SENSITIVE_HOME_PREFIXES)
}

fn chmod_static_path_components(segments: &[ChmodPathSegment]) -> Option<Vec<&str>> {
    segments
        .iter()
        .filter(|segment| !chmod_segment_is_empty(segment))
        .map(|segment| {
            (segment.dynamic_count == 0 && !segment.has_other_dynamic)
                .then_some(segment.text.as_str())
        })
        .collect()
}

const CHMOD_SENSITIVE_HOME_PREFIXES: &[&[&str]] = &[
    &[".ssh"],
    &[".gnupg"],
    &[".aws"],
    &[".kube"],
    &[".docker"],
    &[".netrc"],
    &[".config", "gh"],
    &[".config", "gcloud"],
];

const CHMOD_SENSITIVE_ABSOLUTE_PREFIXES: &[&[&str]] = &[
    &["root"],
    &["etc", "ssh"],
    &["etc", "sudoers"],
    &["etc", "sudoers.d"],
    &["etc", "passwd"],
    &["etc", "shadow"],
    &["etc", "group"],
    &["etc", "gshadow"],
    &["etc", "ssl", "private"],
    &["etc", "pki"],
];

fn path_components_match_any_prefix(components: &[&str], prefixes: &[&[&str]]) -> bool {
    prefixes
        .iter()
        .any(|prefix| path_components_match_prefix(components, prefix))
}

fn path_components_match_prefix(components: &[&str], prefix: &[&str]) -> bool {
    components.len() >= prefix.len()
        && components
            .iter()
            .take(prefix.len())
            .zip(prefix.iter())
            .all(|(actual, expected)| actual == expected)
}

fn parse_rm_command<'a>(args: &[&'a Word], source: &str) -> Option<RmCommandFacts<'a>> {
    let mut index = 0usize;
    let mut recursive = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if text == "--recursive"
            || (text.starts_with('-') && !text.starts_with("--") && text[1..].contains('r'))
            || (text.starts_with('-') && !text.starts_with("--") && text[1..].contains('R'))
        {
            recursive = true;
        }

        index += 1;
    }

    if !recursive {
        return None;
    }

    let dangerous_path_spans = args[index..]
        .iter()
        .flat_map(|word| std::iter::repeat_n(word.span, rm_path_danger_count(word, source)))
        .collect::<Vec<_>>();
    let rootish_path_words = args[index..]
        .iter()
        .copied()
        .filter(|word| rm_path_may_be_rootish(word, source))
        .collect::<Vec<_>>();

    (!dangerous_path_spans.is_empty() || !rootish_path_words.is_empty()).then_some(RmCommandFacts {
        dangerous_path_spans: dangerous_path_spans.into_boxed_slice(),
        rootish_path_words: rootish_path_words.into_boxed_slice(),
        rootish_path_spans: OnceLock::new(),
    })
}

fn rm_path_danger_count(word: &Word, source: &str) -> usize {
    let segments = rm_path_segments(word, source);
    if segments.is_empty() {
        return 0;
    }

    let absolute_root = rm_path_has_absolute_root(&segments);
    let leading_dynamic_start = usize::from(absolute_root);
    let leading_dynamic_count = segments[leading_dynamic_start..]
        .iter()
        .take_while(|segment| rm_path_segment_is_pure_unsafe_parameter(segment))
        .count();

    if !absolute_root && leading_dynamic_count == 0 {
        return 0;
    }

    let brace_expansion_active = word.has_active_brace_expansion();
    let tail_start = leading_dynamic_start + leading_dynamic_count;
    let tail = rm_path_tail_text(&segments[tail_start..]);

    if tail.is_empty() {
        return usize::from(
            leading_dynamic_count > 1
                || (leading_dynamic_count > 0
                    && rm_word_has_explicit_trailing_separator(word, source)),
        );
    }

    rm_path_tail_danger_count(&tail, brace_expansion_active, leading_dynamic_count > 0)
}

#[derive(Debug, Default)]
struct RmPathSegment {
    unsafe_param_count: usize,
    unsafe_home_param_count: usize,
    home_param_count: usize,
    has_literal_text: bool,
    has_other_dynamic: bool,
    text: String,
}

fn rm_path_segments(word: &Word, source: &str) -> Vec<RmPathSegment> {
    let mut segments = vec![RmPathSegment::default()];
    append_rm_path_segments(&mut segments, &word.parts, source);
    segments
}

fn append_rm_path_segments(
    segments: &mut Vec<RmPathSegment>,
    parts: &[WordPartNode],
    source: &str,
) {
    for part in parts {
        append_rm_path_part(segments, &part.kind, part.span, source);
    }
}

fn append_rm_path_part(
    segments: &mut Vec<RmPathSegment>,
    part: &WordPart,
    span: Span,
    source: &str,
) {
    match part {
        WordPart::Literal(text) => append_rm_path_literal(segments, text.as_str(source, span)),
        WordPart::SingleQuoted {
            value,
            dollar: false,
        } => append_rm_path_literal(segments, value.slice(source)),
        WordPart::SingleQuoted { dollar: true, .. } => {
            current_rm_path_segment(segments).has_other_dynamic = true;
        }
        WordPart::DoubleQuoted { parts, .. } => append_rm_path_segments(segments, parts, source),
        WordPart::Variable(name) => {
            mark_rm_path_unsafe_parameter(segments, name.as_str() == "HOME");
        }
        WordPart::Parameter(parameter) => {
            if rm_path_parameter_expansion_is_guarded_home_access(parameter) {
                mark_rm_path_home_parameter(segments);
            } else if rm_path_parameter_expansion_is_unsafe(parameter) {
                mark_rm_path_unsafe_parameter(
                    segments,
                    rm_path_parameter_expansion_is_home_access(parameter, source),
                );
            } else {
                current_rm_path_segment(segments).has_other_dynamic = true;
            }
        }
        WordPart::ParameterExpansion {
            reference,
            operator,
            colon_variant: _,
            ..
        } => {
            let is_guarded_home = rm_path_parameter_op_is_guarded_home_access(reference, operator);
            if is_guarded_home {
                mark_rm_path_home_parameter(segments);
            } else if rm_path_parameter_op_is_unsafe(operator) {
                mark_rm_path_unsafe_parameter(
                    segments,
                    rm_var_ref_is_home(reference)
                        && rm_path_parameter_op_preserves_home_root(operator, source),
                );
            } else {
                current_rm_path_segment(segments).has_other_dynamic = true;
            }
        }
        WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ZshQualifiedGlob(_) => {
            current_rm_path_segment(segments).has_other_dynamic = true;
        }
    }
}

fn mark_rm_path_unsafe_parameter(segments: &mut [RmPathSegment], is_home: bool) {
    let segment = current_rm_path_segment(segments);
    segment.unsafe_param_count += 1;
    if is_home {
        segment.unsafe_home_param_count += 1;
        segment.home_param_count += 1;
    }
}

fn mark_rm_path_home_parameter(segments: &mut [RmPathSegment]) {
    current_rm_path_segment(segments).home_param_count += 1;
}

fn rm_path_parameter_expansion_is_unsafe(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { .. } => true,
            BourneParameterExpansion::Indirect {
                operator,
                operand: _,
                colon_variant: _,
                ..
            } => operator
                .as_deref()
                .is_none_or(rm_path_parameter_op_is_unsafe),
            BourneParameterExpansion::Operation {
                reference,
                operator,
                ..
            } => {
                rm_path_parameter_op_is_unsafe(operator)
                    && !rm_path_parameter_op_is_guarded_home_access(reference, operator)
            }
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. } => false,
        },
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn rm_path_parameter_expansion_is_home_access(
    parameter: &ParameterExpansion,
    source: &str,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
            rm_var_ref_is_home(reference)
        }
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        }) => {
            rm_path_parameter_op_is_guarded_home_access(reference, operator)
                || rm_var_ref_is_home(reference)
                    && rm_path_parameter_op_preserves_home_root(operator, source)
        }
        ParameterExpansionSyntax::Bourne(_) | ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn rm_path_parameter_expansion_is_guarded_home_access(parameter: &ParameterExpansion) -> bool {
    matches!(
        &parameter.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        }) if rm_path_parameter_op_is_guarded_home_access(reference, operator)
    )
}

fn rm_path_parameter_op_preserves_home_root(operator: &ParameterOp, source: &str) -> bool {
    match operator {
        ParameterOp::RemoveSuffixShort { pattern } | ParameterOp::RemoveSuffixLong { pattern } => {
            matches!(pattern.render(source).as_str(), "" | "/")
        }
        _ => false,
    }
}

fn rm_var_ref_is_home(reference: &VarRef) -> bool {
    reference.name.as_str() == "HOME" && reference.subscript.is_none()
}

fn rm_path_parameter_op_is_guarded_home_access(reference: &VarRef, operator: &ParameterOp) -> bool {
    rm_var_ref_is_home(reference) && matches!(operator, ParameterOp::Error)
}

fn rm_path_parameter_op_is_unsafe(operator: &ParameterOp) -> bool {
    !matches!(
        operator,
        ParameterOp::UseDefault | ParameterOp::AssignDefault | ParameterOp::Error
    )
}

fn append_rm_path_literal(segments: &mut Vec<RmPathSegment>, text: &str) {
    for character in text.chars() {
        if character == '/' {
            segments.push(RmPathSegment::default());
            continue;
        }

        let segment = current_rm_path_segment(segments);
        segment.has_literal_text = true;
        segment.text.push(character);
    }
}

fn current_rm_path_segment(segments: &mut [RmPathSegment]) -> &mut RmPathSegment {
    let Some(segment) = segments.last_mut() else {
        unreachable!("rm path segments always start non-empty");
    };
    segment
}

fn rm_path_segment_is_empty(segment: &RmPathSegment) -> bool {
    segment.unsafe_param_count == 0
        && !segment.has_literal_text
        && !segment.has_other_dynamic
        && segment.text.is_empty()
}

fn rm_path_segment_is_root_equivalent_tail(
    segment: &RmPathSegment,
    dotdot_is_root_equivalent: bool,
) -> bool {
    rm_path_segment_is_empty(segment)
        || (segment.unsafe_param_count == 0 && !segment.has_other_dynamic && segment.text == ".")
        || (dotdot_is_root_equivalent
            && segment.unsafe_param_count == 0
            && !segment.has_other_dynamic
            && segment.text == "..")
}

fn rm_path_has_absolute_root(segments: &[RmPathSegment]) -> bool {
    segments.first().is_some_and(rm_path_segment_is_empty)
}

fn rm_word_has_explicit_trailing_separator(word: &Word, source: &str) -> bool {
    strip_shell_matching_quotes_in_source(word.span.slice(source)).ends_with('/')
}

fn rm_path_segment_is_pure_unsafe_parameter(segment: &RmPathSegment) -> bool {
    segment.unsafe_param_count > 0 && !segment.has_literal_text && !segment.has_other_dynamic
}

fn rm_path_is_rootish(word: &Word, source: &str) -> bool {
    let segments = rm_path_segments(word, source);
    if segments.is_empty() {
        return false;
    }
    let has_unquoted_glob = rm_word_has_unquoted_glob_pattern(word, source);

    if rm_path_has_absolute_root(&segments) {
        return rm_path_tail_is_rootish(&segments[1..], has_unquoted_glob, true);
    }

    let Some((root, tail)) = segments.split_first() else {
        return false;
    };

    (rm_path_segment_is_home_parameter_root(root)
        || rm_path_segment_is_unquoted_home_tilde(root, word, source))
        && rm_path_tail_is_rootish(tail, has_unquoted_glob, false)
}

fn rm_path_may_be_rootish(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    text.contains('/') || text.contains('~') || text.contains("HOME") || text.contains('*')
}

fn rm_word_has_unquoted_glob_pattern(word: &Word, source: &str) -> bool {
    !word_spans::word_unquoted_glob_pattern_spans(word, source).is_empty()
}

fn rm_path_segment_is_home_parameter_root(segment: &RmPathSegment) -> bool {
    segment.home_param_count == 1
        && segment.unsafe_param_count == segment.unsafe_home_param_count
        && !segment.has_literal_text
        && !segment.has_other_dynamic
        && segment.text.is_empty()
}

fn rm_path_segment_is_unquoted_home_tilde(
    segment: &RmPathSegment,
    word: &Word,
    source: &str,
) -> bool {
    word.span.slice(source).starts_with('~')
        && segment.unsafe_param_count == 0
        && !segment.has_other_dynamic
        && rm_tilde_prefix_is_home_root(&segment.text)
}

fn rm_tilde_prefix_is_home_root(text: &str) -> bool {
    let Some(suffix) = text.strip_prefix('~') else {
        return false;
    };

    if suffix.starts_with(['+', '-']) {
        return false;
    }

    if suffix.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return false;
    }

    !suffix.contains(['*', '?', '[', ']', '{', '}'])
}

fn rm_path_tail_is_rootish(
    segments: &[RmPathSegment],
    has_unquoted_glob: bool,
    dotdot_is_root_equivalent: bool,
) -> bool {
    let canonical_segments = canonicalize_rm_path_tail(segments);
    let mut meaningful_segments = canonical_segments.iter().copied().filter(|segment| {
        !rm_path_segment_is_root_equivalent_tail(segment, dotdot_is_root_equivalent)
    });

    match (meaningful_segments.next(), meaningful_segments.next()) {
        (None, _) => true,
        (Some(segment), None) => rm_path_segment_is_rootish_wildcard(segment, has_unquoted_glob),
        (Some(_), Some(_)) => false,
    }
}

fn canonicalize_rm_path_tail(segments: &[RmPathSegment]) -> Vec<&RmPathSegment> {
    let mut stack = Vec::<&RmPathSegment>::new();

    for segment in segments {
        if rm_path_segment_is_static_text(segment, ".") || rm_path_segment_is_empty(segment) {
            continue;
        }

        stack.push(segment);
    }

    stack
}

fn rm_path_segment_is_static_text(segment: &RmPathSegment, text: &str) -> bool {
    segment.unsafe_param_count == 0 && !segment.has_other_dynamic && segment.text == text
}

fn rm_path_segment_is_rootish_wildcard(segment: &RmPathSegment, has_unquoted_glob: bool) -> bool {
    has_unquoted_glob
        && segment.unsafe_param_count == 0
        && !segment.has_other_dynamic
        && matches!(segment.text.as_str(), "*" | "**" | ".*")
}

fn rm_path_tail_text(segments: &[RmPathSegment]) -> String {
    segments
        .iter()
        .map(rm_path_segment_tail_pattern)
        .collect::<Vec<_>>()
        .join("/")
}

const RM_PURE_DYNAMIC_TAIL_COMPONENT: &str = "\u{1f}";
const RM_MIXED_DYNAMIC_TAIL_PREFIX: &str = "\u{1e}";

fn rm_path_segment_tail_pattern(segment: &RmPathSegment) -> String {
    if segment.unsafe_param_count > 0 || segment.has_other_dynamic {
        if segment.text.is_empty() {
            RM_PURE_DYNAMIC_TAIL_COMPONENT.to_owned()
        } else {
            format!("{RM_MIXED_DYNAMIC_TAIL_PREFIX}{}", segment.text)
        }
    } else {
        segment.text.clone()
    }
}

const RM_DANGEROUS_LITERAL_SUFFIXES: &[&str] = &[
    "bin",
    "boot",
    "dev",
    "etc",
    "home",
    "lib",
    "usr",
    "usr/bin",
    "usr/local",
    "usr/share",
    "var",
];

fn rm_path_tail_danger_count(
    tail: &str,
    brace_expansion_active: bool,
    has_leading_dynamic: bool,
) -> usize {
    if brace_expansion_active && let Some((prefix, inner, suffix)) = split_brace_expansion(tail) {
        return split_brace_alternatives(inner)
            .into_iter()
            .map(|alternative| {
                rm_path_tail_danger_count(
                    &format!("{prefix}{alternative}{suffix}"),
                    true,
                    has_leading_dynamic,
                )
            })
            .sum();
    }

    let tail = tail.trim_matches('/');
    if tail.is_empty() {
        return 0;
    }

    let components = tail
        .split('/')
        .filter(|component| !component.is_empty())
        .map(rm_path_tail_component)
        .collect::<Vec<_>>();

    let dangerous_prefix_matches = RM_DANGEROUS_LITERAL_SUFFIXES
        .iter()
        .filter(|dangerous_prefix| {
            let dangerous_components = dangerous_prefix.split('/').collect::<Vec<_>>();
            rm_path_matches_dangerous_prefix(
                &components,
                &dangerous_components,
                has_leading_dynamic,
            )
        })
        .count();

    dangerous_prefix_matches
        + usize::from(
            has_leading_dynamic
                && components.as_slice().first().is_some_and(|component| {
                    components.len() == 1 && rm_tail_component_is_dangerous_wildcard(component)
                }),
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RmTailComponent<'a> {
    Literal(&'a str),
    MixedDynamic(&'a str),
    PureDynamic,
}

fn rm_path_tail_component(component: &str) -> RmTailComponent<'_> {
    if component == RM_PURE_DYNAMIC_TAIL_COMPONENT {
        return RmTailComponent::PureDynamic;
    }

    if let Some(literal_suffix) = component.strip_prefix(RM_MIXED_DYNAMIC_TAIL_PREFIX) {
        return RmTailComponent::MixedDynamic(literal_suffix);
    }

    RmTailComponent::Literal(component)
}

fn rm_path_matches_dangerous_prefix(
    components: &[RmTailComponent<'_>],
    dangerous_components: &[&str],
    has_leading_dynamic: bool,
) -> bool {
    if components.len() < dangerous_components.len() {
        return false;
    }

    if !components
        .iter()
        .take(dangerous_components.len())
        .zip(dangerous_components.iter())
        .all(|(component, expected)| {
            rm_tail_component_matches_dangerous_literal(component, expected)
        })
    {
        return false;
    }

    let remainder = &components[dangerous_components.len()..];
    if remainder.is_empty() {
        return has_leading_dynamic;
    }

    remainder.iter().all(rm_tail_component_is_collapsible)
}

fn rm_tail_component_is_collapsible(component: &RmTailComponent<'_>) -> bool {
    matches!(
        component,
        RmTailComponent::PureDynamic
            | RmTailComponent::Literal("*")
            | RmTailComponent::MixedDynamic("*")
    )
}

fn rm_tail_component_matches_dangerous_literal(
    component: &RmTailComponent<'_>,
    expected: &str,
) -> bool {
    matches!(
        component,
        RmTailComponent::Literal(actual) | RmTailComponent::MixedDynamic(actual)
            if *actual == expected
    )
}

fn rm_tail_component_is_dangerous_wildcard(component: &RmTailComponent<'_>) -> bool {
    matches!(
        component,
        RmTailComponent::Literal("*") | RmTailComponent::MixedDynamic("*")
    )
}

fn split_brace_expansion(text: &str) -> Option<(&str, &str, &str)> {
    let bytes = text.as_bytes();
    let open = bytes.iter().position(|byte| *byte == b'{')?;
    let mut depth = 0usize;

    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((&text[..open], &text[open + 1..index], &text[index + 1..]));
                }
            }
            _ => {}
        }
    }

    None
}

fn split_brace_alternatives(text: &str) -> Vec<&str> {
    let mut alternatives = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;

    for (index, byte) in text.as_bytes().iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                alternatives.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }

    alternatives.push(&text[start..]);
    alternatives
}

fn same_command_file_operand_words<'a>(
    command_name: Option<&str>,
    args: &[&'a Word],
    source: &str,
) -> Box<[&'a Word]> {
    match command_name {
        Some("grep") => grep_file_operand_words(args, source).into_boxed_slice(),
        Some("sed") => {
            let skip_initial_positionals =
                usize::from(!sed_has_explicit_script_source(args, source));
            collect_file_operand_words_after_prefix(
                args,
                source,
                skip_initial_positionals,
                |text| match text {
                    "-e" | "-f" | "--expression" | "--file" => Some(OperandArgAction::SkipNext),
                    _ if text.starts_with("--expression=") || text.starts_with("--file=") => None,
                    _ => None,
                },
            )
            .into_boxed_slice()
        }
        Some("awk") => {
            let skip_initial_positionals = usize::from(!awk_has_file_program_source(args, source));
            collect_file_operand_words_after_prefix(
                args,
                source,
                skip_initial_positionals,
                |text| match text {
                    "-f" | "-v" | "--file" | "--assign" => Some(OperandArgAction::SkipNext),
                    _ if text.starts_with("--file=") || text.starts_with("--assign=") => None,
                    _ => None,
                },
            )
            .into_boxed_slice()
        }
        Some("unzip") => {
            collect_file_operand_words_after_prefix(args, source, 1, |text| match text {
                "-d" | "--d" | "--directory" => Some(OperandArgAction::SkipNext),
                _ if text.starts_with("--directory=") => None,
                _ => None,
            })
            .into_boxed_slice()
        }
        Some("sort") => {
            collect_file_operand_words_after_prefix(args, source, 0, |text| match text {
                "-o" | "--output" => Some(OperandArgAction::SkipNext),
                _ if text.starts_with("--output=") => None,
                _ => None,
            })
            .into_boxed_slice()
        }
        Some("jq") => jq_file_operand_words(args, source).into_boxed_slice(),
        Some("bsdtar") | Some("tar") => {
            collect_file_operand_words_after_prefix(args, source, 0, |text| match text {
                "--exclude" => Some(OperandArgAction::IncludeNext),
                _ if text.starts_with("--exclude=") => None,
                _ => None,
            })
            .into_boxed_slice()
        }
        Some(
            "cat" | "cp" | "mv" | "head" | "tail" | "cut" | "uniq" | "comm" | "join" | "paste",
        ) => collect_file_operand_words_after_prefix(args, source, 0, |_| None).into_boxed_slice(),
        _ => Vec::new().into_boxed_slice(),
    }
}

fn sed_has_explicit_script_source(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|text| {
            matches!(text.as_ref(), "-e" | "-f" | "--expression" | "--file")
                || text.starts_with("--expression=")
                || text.starts_with("--file=")
                || short_option_cluster_contains_flag(text.as_ref(), 'e')
                || short_option_cluster_contains_flag(text.as_ref(), 'f')
        })
}

fn awk_has_file_program_source(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|text| {
            matches!(text.as_ref(), "-f" | "--file")
                || text.starts_with("--file=")
                || short_option_cluster_contains_flag(text.as_ref(), 'f')
        })
}

pub(crate) fn short_option_cluster_contains_flag(text: &str, flag: char) -> bool {
    let Some(cluster) = text.strip_prefix('-') else {
        return false;
    };

    !cluster.is_empty() && !cluster.starts_with('-') && cluster.contains(flag)
}

enum OperandArgAction {
    IncludeNext,
    SkipNext,
}

fn collect_file_operand_words_after_prefix<'a>(
    args: &[&'a Word],
    source: &str,
    skip_initial_positionals: usize,
    mut option_arg_action: impl FnMut(&str) -> Option<OperandArgAction>,
) -> Vec<&'a Word> {
    let mut operands = Vec::new();
    let mut index = 0usize;
    let mut options_open = true;
    let mut pending_option_arg_action: Option<OperandArgAction> = None;
    let mut remaining_prefix_words = skip_initial_positionals;

    while let Some(word) = args.get(index) {
        if let Some(action) = pending_option_arg_action.take() {
            if matches!(action, OperandArgAction::IncludeNext) {
                operands.push(*word);
            }
            index += 1;
            continue;
        }

        let Some(text) = static_word_text(word, source) else {
            if options_open && word_starts_with_literal_dash(word, source) {
                index += 1;
                continue;
            }

            if options_open {
                options_open = false;
            }

            if remaining_prefix_words > 0 {
                remaining_prefix_words -= 1;
                index += 1;
                continue;
            }

            operands.push(*word);
            index += 1;
            continue;
        };

        if options_open && text == "--" {
            options_open = false;
            index += 1;
            continue;
        }

        if options_open && text.starts_with('-') && text != "-" {
            if let Some(action) = option_arg_action(text.as_ref()) {
                pending_option_arg_action = Some(action);
            }
            index += 1;
            continue;
        }

        options_open = false;
        if remaining_prefix_words > 0 {
            remaining_prefix_words -= 1;
            index += 1;
            continue;
        }

        operands.push(*word);
        index += 1;
    }

    operands
}

fn jq_file_operand_words<'a>(args: &[&'a Word], source: &str) -> Vec<&'a Word> {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PendingOptionArgs {
        Skip(usize),
        NamedFileSource { seen_name: bool },
    }

    let mut operands = Vec::new();
    let mut index = 0usize;
    let mut options_open = true;
    let mut pending_args: Option<PendingOptionArgs> = None;
    let mut filter_from_file = false;
    let mut null_input = false;
    let mut consumed_filter = false;
    let mut positional_args_mode = false;

    while let Some(word) = args.get(index) {
        if let Some(pending) = pending_args {
            match pending {
                PendingOptionArgs::Skip(remaining) => {
                    if remaining > 1 {
                        pending_args = Some(PendingOptionArgs::Skip(remaining - 1));
                    } else {
                        pending_args = None;
                    }
                    index += 1;
                    continue;
                }
                PendingOptionArgs::NamedFileSource { seen_name: false } => {
                    pending_args = Some(PendingOptionArgs::NamedFileSource { seen_name: true });
                    index += 1;
                    continue;
                }
                PendingOptionArgs::NamedFileSource { seen_name: true } => {
                    operands.push(*word);
                    pending_args = None;
                    index += 1;
                    continue;
                }
            }
        }

        let Some(text) = static_word_text(word, source) else {
            if options_open && word_starts_with_literal_dash(word, source) {
                index += 1;
                continue;
            }

            options_open = false;
            if !consumed_filter && !filter_from_file {
                consumed_filter = true;
                index += 1;
                continue;
            }

            if !null_input && !positional_args_mode {
                operands.push(*word);
            }
            index += 1;
            continue;
        };

        if options_open && text == "--" {
            options_open = false;
            index += 1;
            continue;
        }

        if options_open && text.starts_with('-') && text != "-" {
            if !text.starts_with("--")
                && let Some(cluster) = text.strip_prefix('-')
            {
                let mut cluster_chars = cluster.chars().peekable();
                while let Some(flag) = cluster_chars.next() {
                    match flag {
                        'n' => {
                            null_input = true;
                        }
                        'f' => {
                            filter_from_file = true;
                            consumed_filter = true;
                            if cluster_chars.peek().is_none() {
                                pending_args = Some(PendingOptionArgs::Skip(1));
                            }
                            break;
                        }
                        'L' => {
                            if cluster_chars.peek().is_none() {
                                pending_args = Some(PendingOptionArgs::Skip(1));
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }

            match text.as_ref() {
                "-n" | "--null-input" => {
                    null_input = true;
                }
                "-f" | "--from-file" => {
                    filter_from_file = true;
                    consumed_filter = true;
                    pending_args = Some(PendingOptionArgs::Skip(1));
                }
                "--arg" | "--argjson" => {
                    pending_args = Some(PendingOptionArgs::Skip(2));
                }
                "--rawfile" | "--slurpfile" | "--argfile" => {
                    pending_args = Some(PendingOptionArgs::NamedFileSource { seen_name: false });
                }
                "--indent" => {
                    pending_args = Some(PendingOptionArgs::Skip(1));
                }
                "-L" | "--library-path" => {
                    pending_args = Some(PendingOptionArgs::Skip(1));
                }
                "--args" | "--jsonargs" => {
                    positional_args_mode = true;
                }
                _ if text.starts_with("--from-file=") => {
                    filter_from_file = true;
                    consumed_filter = true;
                }
                _ if text.starts_with("--library-path=")
                    || text.starts_with("--arg=")
                    || text.starts_with("--argjson=")
                    || text.starts_with("--args=")
                    || text.starts_with("--jsonargs=") => {}
                _ => {}
            }
            index += 1;
            continue;
        }

        options_open = false;
        if !consumed_filter && !filter_from_file {
            consumed_filter = true;
            index += 1;
            continue;
        }

        if !null_input && !positional_args_mode {
            operands.push(*word);
        }
        index += 1;
    }

    operands
}

fn parse_ps_command(args: &[&Word], source: &str) -> PsCommandFacts {
    let mut has_pid_selector = false;
    let mut pending_option_arg = false;
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if let Some(expects_argument) = dynamic_ps_pid_selector(word, source) {
                has_pid_selector = true;
                pending_option_arg = expects_argument;
                index += 1;
                continue;
            }

            if word_starts_with_literal_dash(word, source) {
                pending_option_arg = true;
                index += 1;
                continue;
            }

            if pending_option_arg {
                pending_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            break;
        }

        if matches!(text.as_ref(), "p" | "q") {
            has_pid_selector = true;
            pending_option_arg = true;
            index += 1;
            continue;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_option_arg {
                pending_option_arg = false;
                index += 1;
                continue;
            }

            if text != "-" && ps_bare_pid_selector(text.as_ref()) {
                has_pid_selector = true;
                index += 1;
                continue;
            }

            if ps_bsd_option_cluster(text.as_ref()) {
                index += 1;
                continue;
            }

            break;
        }

        pending_option_arg = false;

        if text == "-p"
            || text == "-q"
            || text == "--pid"
            || text == "--ppid"
            || text == "--quick-pid"
        {
            has_pid_selector = true;
            pending_option_arg = true;
            index += 1;
            continue;
        }

        if text.starts_with("--pid=")
            || text.starts_with("--ppid=")
            || text.starts_with("--quick-pid=")
            || (text.starts_with("-p") && text.len() > 2)
            || (text.starts_with("-q") && text.len() > 2)
        {
            has_pid_selector = true;
            index += 1;
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'p' || flag == 'q' {
                has_pid_selector = true;
            }

            if ps_option_takes_argument(flag) {
                if chars.peek().is_none() {
                    pending_option_arg = true;
                }
                break;
            }
        }

        index += 1;
    }

    PsCommandFacts { has_pid_selector }
}

fn dynamic_ps_pid_selector(word: &Word, source: &str) -> Option<bool> {
    let prefix = leading_literal_word_prefix(word, source);
    if prefix.is_empty() {
        return None;
    }

    let has_attached_value = word.span.slice(source).len() > prefix.len();

    match prefix.as_str() {
        "p" | "q" | "-p" | "-q" | "--pid" | "--ppid" | "--quick-pid" => Some(!has_attached_value),
        _ if prefix.starts_with("--pid=")
            || prefix.starts_with("--ppid=")
            || prefix.starts_with("--quick-pid=")
            || (prefix.starts_with("-p") && prefix.len() > 2)
            || (prefix.starts_with("-q") && prefix.len() > 2) =>
        {
            Some(false)
        }
        _ => None,
    }
}

fn ps_bare_pid_selector(text: &str) -> bool {
    text.split(',')
        .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn ps_bsd_option_cluster(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|ch| {
            matches!(
                ch,
                'A' | 'a'
                    | 'C'
                    | 'c'
                    | 'E'
                    | 'e'
                    | 'f'
                    | 'g'
                    | 'h'
                    | 'j'
                    | 'l'
                    | 'L'
                    | 'M'
                    | 'm'
                    | 'r'
                    | 'S'
                    | 'T'
                    | 'u'
                    | 'v'
                    | 'w'
                    | 'X'
                    | 'x'
            )
        })
}

fn ps_option_takes_argument(flag: char) -> bool {
    matches!(
        flag,
        'C' | 'G' | 'N' | 'O' | 'U' | 'g' | 'o' | 'p' | 'q' | 't' | 'u'
    )
}

#[derive(Debug, Clone, Copy)]
enum ShBuiltinOptionPolicy {
    Any,
    AllowOnly(&'static str),
}

fn first_nonportable_sh_builtin_option_span(
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<Span> {
    match normalized.effective_name.as_deref()? {
        "read" => first_nonportable_sh_option_span_in_words(
            normalized.body_args(),
            source,
            ShBuiltinOptionPolicy::AllowOnly("r"),
        ),
        "export" => normalized
            .declaration
            .as_ref()
            .and_then(|declaration| {
                matches!(declaration.kind, command::DeclarationKind::Export).then(|| ())
            })
            .map_or_else(
                || {
                    first_nonportable_sh_option_span_in_words(
                        normalized.body_args(),
                        source,
                        ShBuiltinOptionPolicy::AllowOnly("p"),
                    )
                },
                |_| {
                    let Some(declaration) = normalized.declaration.as_ref() else {
                        unreachable!("checked export declaration");
                    };
                    let operands = declaration.operands;
                    first_nonportable_sh_export_option_span(operands, source)
                },
            ),
        "ulimit" => first_nonportable_sh_option_span_in_words(
            normalized.body_args(),
            source,
            ShBuiltinOptionPolicy::AllowOnly("f"),
        ),
        "printf" | "trap" | "type" | "wait" => first_nonportable_sh_option_span_in_words(
            normalized.body_args(),
            source,
            ShBuiltinOptionPolicy::Any,
        ),
        _ => None,
    }
}

fn first_nonportable_sh_export_option_span(operands: &[DeclOperand], source: &str) -> Option<Span> {
    for operand in operands {
        match operand {
            DeclOperand::Flag(word) => {
                let text = static_word_text(word, source)?;

                if text == "--" {
                    return None;
                }

                if !text.starts_with('-') || text == "-" {
                    return None;
                }

                if sh_builtin_option_word_is_portable(
                    text.as_ref(),
                    ShBuiltinOptionPolicy::AllowOnly("p"),
                ) {
                    continue;
                }

                return Some(word.span);
            }
            DeclOperand::Dynamic(word) => {
                return word_starts_with_literal_dash(word, source).then_some(word.span);
            }
            DeclOperand::Name(_) | DeclOperand::Assignment(_) => return None,
        }
    }

    None
}

fn first_nonportable_sh_option_span_in_words(
    args: &[&Word],
    source: &str,
    policy: ShBuiltinOptionPolicy,
) -> Option<Span> {
    for word in args {
        let Some(text) = static_word_text(word, source) else {
            return word_starts_with_literal_dash(word, source).then_some(word.span);
        };

        if text == "--" {
            return None;
        }

        if !text.starts_with('-') || text == "-" {
            return None;
        }

        if sh_builtin_option_word_is_portable(text.as_ref(), policy) {
            continue;
        }

        return Some(word.span);
    }

    None
}

fn sh_builtin_option_word_is_portable(text: &str, policy: ShBuiltinOptionPolicy) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };

    if flags.is_empty() {
        return false;
    }

    match policy {
        ShBuiltinOptionPolicy::Any => false,
        ShBuiltinOptionPolicy::AllowOnly(allowed_flags) => {
            flags.chars().all(|flag| allowed_flags.contains(flag))
        }
    }
}

fn printf_format_word<'a>(args: &[&'a Word], source: &str) -> Option<&'a Word> {
    let mut index = 0usize;

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    if let Some(option) = args
        .get(index)
        .and_then(|word| static_word_text(word, source))
    {
        if option == "-v" {
            index += 2;
        } else if option.starts_with("-v") && option.len() > 2 {
            index += 1;
        }
    }

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    args.get(index).copied()
}

fn printf_format_word_has_literal_percent(word: &Word, source: &str) -> bool {
    word_parts_have_literal_percent(&word.parts, source)
}

fn word_parts_have_literal_percent(parts: &[WordPartNode], source: &str) -> bool {
    parts
        .iter()
        .any(|part| word_part_has_literal_percent(part, source))
}

fn word_part_has_literal_percent(part: &WordPartNode, source: &str) -> bool {
    match &part.kind {
        WordPart::Literal(text) => text.as_str(source, part.span).contains('%'),
        WordPart::ZshQualifiedGlob(_) => part.span.slice(source).contains('%'),
        WordPart::SingleQuoted { value, .. } => value.slice(source).contains('%'),
        WordPart::DoubleQuoted { parts, .. } => word_parts_have_literal_percent(parts, source),
        WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

fn printf_uses_q_format(word: &Word, source: &str) -> bool {
    let Some(text) = static_word_text(word, source) else {
        return false;
    };

    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }

        index += 1;
        if index >= bytes.len() {
            break;
        }

        if bytes[index] == b'%' {
            index += 1;
            continue;
        }

        while index < bytes.len() && matches!(bytes[index], b'-' | b'+' | b' ' | b'#' | b'0') {
            index += 1;
        }

        if index < bytes.len() && bytes[index] == b'*' {
            index += 1;
        } else {
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
        }

        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            if index < bytes.len() && bytes[index] == b'*' {
                index += 1;
            } else {
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }

        if index + 1 < bytes.len()
            && ((bytes[index] == b'h' && bytes[index + 1] == b'h')
                || (bytes[index] == b'l' && bytes[index + 1] == b'l'))
        {
            index += 2;
        } else if index < bytes.len()
            && matches!(bytes[index], b'h' | b'l' | b'j' | b'z' | b't' | b'L')
        {
            index += 1;
        }

        if index < bytes.len() && bytes[index] == b'q' {
            return true;
        }
    }

    false
}

fn parse_unset_command<'a>(args: &[&'a Word], source: &str) -> UnsetCommandFacts<'a> {
    let mut function_mode = false;
    let mut nameref_mode = false;
    let mut parsing_options = true;
    let mut options_parseable = true;
    let mut operands = Vec::new();
    let mut operand_facts = Vec::new();
    let mut prefix_match_operand_spans = Vec::new();

    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if parsing_options {
                options_parseable = false;
                parsing_options = false;
            }

            collect_word_prefix_match_spans(word, &mut prefix_match_operand_spans);
            operands.push(*word);
            operand_facts.push(parse_unset_operand_fact(word, source));
            continue;
        };

        if parsing_options {
            if text == "--" {
                parsing_options = false;
                continue;
            }

            if text.starts_with('-') && text != "-" {
                for flag in text[1..].chars() {
                    match flag {
                        'f' => function_mode = true,
                        'n' => nameref_mode = true,
                        'v' => {}
                        _ => options_parseable = false,
                    }
                }
                continue;
            }

            parsing_options = false;
        }

        collect_word_prefix_match_spans(word, &mut prefix_match_operand_spans);
        operands.push(*word);
        operand_facts.push(parse_unset_operand_fact(word, source));
    }

    UnsetCommandFacts {
        function_mode,
        nameref_mode,
        operand_words: operands.into_boxed_slice(),
        operand_facts: operand_facts.into_boxed_slice(),
        prefix_match_operand_spans: prefix_match_operand_spans.into_boxed_slice(),
        options_parseable,
    }
}

fn parse_unset_operand_fact<'a>(word: &'a Word, source: &str) -> UnsetOperandFact<'a> {
    UnsetOperandFact {
        word,
        array_subscript: parse_unset_array_subscript(word.span.slice(source)),
    }
}

fn parse_unset_array_subscript(text: &str) -> Option<UnsetArraySubscriptFact> {
    let (name, key_with_bracket) = text.split_once('[')?;
    key_with_bracket.strip_suffix(']')?;
    is_shell_variable_name(name).then_some(UnsetArraySubscriptFact)
}

fn collect_word_prefix_match_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_prefix_match_spans(&word.parts, spans);
}

fn collect_prefix_match_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_prefix_match_spans(parts, spans),
            WordPart::PrefixMatch { .. } => spans.push(part.span),
            WordPart::Parameter(parameter)
                if matches!(
                    parameter.bourne(),
                    Some(BourneParameterExpansion::PrefixMatch { .. })
                ) =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

fn parse_directory_change_command(
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<DirectoryChangeCommandFacts> {
    let kind = match normalized.effective_name.as_deref() {
        Some("cd") => DirectoryChangeCommandKind::Cd,
        Some("pushd") => DirectoryChangeCommandKind::Pushd,
        Some("popd") => DirectoryChangeCommandKind::Popd,
        _ => return None,
    };

    let target = normalized
        .body_args()
        .first()
        .and_then(|word| static_word_text(word, source));

    let plain_directory_stack_marker = matches!(
        kind,
        DirectoryChangeCommandKind::Cd | DirectoryChangeCommandKind::Pushd
    ) && normalized.wrappers.is_empty()
        && target
            .as_ref()
            .is_some_and(|target| is_directory_stack_marker(target.as_ref()));

    let manual_restore_candidate = kind == DirectoryChangeCommandKind::Cd
        && target
            .as_ref()
            .is_some_and(|target| matches!(target.as_ref(), ".." | "-"));

    Some(DirectoryChangeCommandFacts {
        kind,
        plain_directory_stack_marker,
        manual_restore_candidate,
    })
}

fn is_directory_stack_marker(target: &str) -> bool {
    if !target.is_empty() && target.chars().all(|ch| ch == '/') {
        return true;
    }

    let mut saw_segment = false;
    for segment in target.split('/').filter(|segment| !segment.is_empty()) {
        saw_segment = true;
        if segment != "." && segment != ".." {
            return false;
        }
    }

    saw_segment
}
