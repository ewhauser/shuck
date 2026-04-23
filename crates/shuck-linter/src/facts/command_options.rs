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
        zsh_options: Option<&ZshOptionState>,
    ) -> Self {
        Self {
            word,
            context,
            comparable_path: comparable_path(word, source, context, zsh_options),
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
}

impl ReadCommandFacts {
    pub(crate) fn target_name_uses(&self) -> &[ComparableNameUse] {
        &self.target_name_uses
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
pub struct RmCommandFacts {
    dangerous_path_spans: Box<[Span]>,
}

impl RmCommandFacts {
    pub fn dangerous_path_spans(&self) -> &[Span] {
        &self.dangerous_path_spans
    }
}

#[derive(Debug, Clone)]
pub struct SshCommandFacts {
    local_expansion_spans: Box<[Span]>,
}

impl SshCommandFacts {
    pub fn local_expansion_spans(&self) -> &[Span] {
        &self.local_expansion_spans
    }
}

#[derive(Debug, Clone)]
pub struct FindCommandFacts {
    pub has_print0: bool,
    has_formatted_output_action: bool,
    or_without_grouping_spans: Box<[Span]>,
    glob_pattern_operand_spans: Box<[Span]>,
}

impl FindCommandFacts {
    pub fn has_formatted_output_action(&self) -> bool {
        self.has_formatted_output_action
    }

    pub fn or_without_grouping_spans(&self) -> &[Span] {
        &self.or_without_grouping_spans
    }

    pub fn glob_pattern_operand_spans(&self) -> &[Span] {
        &self.glob_pattern_operand_spans
    }
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

#[derive(Debug, Clone, Copy)]
pub struct MapfileCommandFacts {
    input_fd: Option<i32>,
}

impl MapfileCommandFacts {
    pub fn input_fd(self) -> Option<i32> {
        self.input_fd
    }
}

#[derive(Debug, Clone)]
pub struct XargsCommandFacts {
    pub uses_null_input: bool,
    max_procs: Option<u64>,
    inline_replace_option_spans: Box<[Span]>,
}

impl XargsCommandFacts {
    pub fn max_procs(&self) -> Option<u64> {
        self.max_procs
    }

    pub fn inline_replace_option_spans(&self) -> &[Span] {
        &self.inline_replace_option_spans
    }
}

#[derive(Debug, Clone)]
pub struct WaitCommandFacts {
    option_spans: Box<[Span]>,
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
    required_arg_count: usize,
    uses_unprotected_positional_parameters: bool,
    resets_positional_parameters: bool,
}

impl FunctionPositionalParameterFacts {
    pub fn required_arg_count(&self) -> usize {
        self.required_arg_count
    }

    pub fn uses_positional_parameters(&self) -> bool {
        self.uses_unprotected_positional_parameters
    }

    pub fn uses_unprotected_positional_parameters(&self) -> bool {
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
    string_helper_kind: Option<ExprStringHelperKind>,
    string_helper_span: Option<Span>,
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
    status_is_static: bool,
    status_has_literal_content: bool,
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
    rm: Option<RmCommandFacts>,
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
    xargs: Option<XargsCommandFacts>,
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

impl<'a> CommandOptionFacts<'a> {
    pub fn rm(&self) -> Option<&RmCommandFacts> {
        self.rm.as_ref()
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

    pub fn xargs(&self) -> Option<&XargsCommandFacts> {
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

    fn build(command: &'a Command, normalized: &NormalizedCommand<'a>, source: &str) -> Self {
        Self {
            rm: normalized
                .literal_name
                .as_deref()
                .is_some_and(|name| name == "rm" && normalized.wrappers.is_empty())
                .then(|| parse_rm_command(normalized.body_args(), source))
                .flatten(),
            ssh: (normalized.effective_name_is("ssh") && normalized.wrappers.is_empty())
                .then(|| parse_ssh_command(normalized.body_args(), source))
                .flatten(),
            read: normalized
                .effective_name_is("read")
                .then(|| ReadCommandFacts {
                    uses_raw_input: read_uses_raw_input(normalized.body_args(), source),
                    target_name_uses: read_target_name_uses(normalized.body_args(), source),
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
            .then(|| parse_find_command(find_command_args(command, normalized, source), source)),
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
            .then(|| parse_mapfile_command(normalized.body_args(), source)),
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
                SudoFamilyCommandFacts {
                    invoker,
                }
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

fn read_uses_raw_input(args: &[&Word], source: &str) -> bool {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;
        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'r' {
                return true;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        index += 1;
    }

    false
}

fn read_target_name_uses(args: &[&Word], source: &str) -> Box<[ComparableNameUse]> {
    let mut targets = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                return Vec::new().into_boxed_slice();
            }

            for target in &args[index..] {
                targets.extend(comparable_read_target_name_uses(target, source));
            }
            break;
        };

        if text == "--" {
            for target in &args[index + 1..] {
                targets.extend(comparable_read_target_name_uses(target, source));
            }
            break;
        }

        if !text.starts_with('-') || text == "-" {
            for target in &args[index..] {
                targets.extend(comparable_read_target_name_uses(target, source));
            }
            break;
        }

        let mut chars = text[1..].char_indices().peekable();
        let mut saw_array_target = false;
        while let Some((flag_offset, flag)) = chars.next() {
            if flag == 'a' {
                let attached_start = flag_offset + 2;
                if attached_start < text.len() {
                    if let Some(target) =
                        read_attached_array_target_name_use(word, source, &text[attached_start..])
                    {
                        targets.push(target);
                    }
                } else if let Some(target) = args.get(index + 1) {
                    targets.extend(comparable_read_target_name_uses(target, source));
                    index += 1;
                }
                saw_array_target = true;
                break;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        if saw_array_target {
            break;
        }

        index += 1;
    }

    targets.into_boxed_slice()
}

fn read_attached_array_target_name_use(
    word: &Word,
    source: &str,
    target_text: &str,
) -> Option<ComparableNameUse> {
    if !comparable_name_text(target_text) {
        return None;
    }

    let target_span = word
        .span
        .slice(source)
        .rfind(target_text)
        .map(|start| {
            read_option_attached_target_span(word.span, source, start, start + target_text.len())
        })
        .unwrap_or(word.span);

    Some(ComparableNameUse {
        span: target_span,
        key: ComparableNameKey(target_text.into()),
        kind: ComparableNameUseKind::Literal,
    })
}

fn read_option_attached_target_span(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
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

fn parse_sed_command(args: &[&Word], source: &str) -> SedCommandFacts {
    SedCommandFacts {
        has_single_substitution_script: sed_has_single_substitution_script(
            args,
            source,
            SedScriptQuoteMode::ShellOnly,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SedScriptQuoteMode {
    ShellOnly,
    AllowBacktickEscapedDoubleQuotes,
}

fn sed_script_text<'a>(
    args: &[&Word],
    source: &'a str,
    quote_mode: SedScriptQuoteMode,
) -> Option<Cow<'a, str>> {
    match args {
        [script] => Some(Cow::Borrowed(strip_matching_sed_script_quotes_in_source(
            script.span.slice(source),
            quote_mode,
        ))),
        [first, .., last]
            if quote_mode == SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes
                && first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"") =>
        {
            let mut text = String::new();
            for (index, word) in args.iter().enumerate() {
                if index != 0 {
                    text.push(' ');
                }
                text.push_str(word.span.slice(source));
            }
            Some(Cow::Owned(
                strip_backtick_escaped_double_quotes_in_source(&text).to_owned(),
            ))
        }
        _ => None,
    }
}

fn sed_has_single_substitution_script(
    args: &[&Word],
    source: &str,
    quote_mode: SedScriptQuoteMode,
) -> bool {
    sed_script_text(args, source, quote_mode)
        .or_else(|| match args {
            [flag, words @ ..] if static_word_text(flag, source).as_deref() == Some("-e") => {
                sed_script_text(words, source, quote_mode)
            }
            _ => None,
        })
        .as_deref()
        .is_some_and(is_simple_sed_substitution_script)
}

fn is_echo_portability_flag(text: &str) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };

    !flags.is_empty()
        && flags
            .bytes()
            .all(|byte| matches!(byte, b'n' | b'e' | b'E' | b's'))
}

fn is_simple_sed_substitution_script(text: &str) -> bool {
    let Some(remainder) = text.strip_prefix('s') else {
        return false;
    };

    let Some(delimiter) = remainder.chars().next() else {
        return false;
    };
    if delimiter.is_whitespace() || delimiter == '\\' {
        return false;
    }

    let pattern_start = 1 + delimiter.len_utf8();
    let Some((pattern_end, pattern_has_escaped_delimiter)) =
        find_sed_substitution_section(text, pattern_start, delimiter)
    else {
        return false;
    };
    let replacement_start = pattern_end + delimiter.len_utf8();
    let Some((replacement_end, replacement_has_escaped_delimiter)) =
        find_sed_substitution_section(text, replacement_start, delimiter)
    else {
        return false;
    };

    let flags = &text[replacement_end + delimiter.len_utf8()..];
    if flags.chars().any(|ch| ch.is_whitespace() || ch == ';') {
        return false;
    }

    let pattern = &text[pattern_start..pattern_end];
    let replacement = &text[replacement_start..replacement_end];
    !pattern_has_escaped_delimiter
        && !replacement_has_escaped_delimiter
        && !uses_delimiter_sensitive_match_escape(pattern, replacement, delimiter)
}

fn find_sed_substitution_section(
    text: &str,
    start: usize,
    delimiter: char,
) -> Option<(usize, bool)> {
    let _ = text.get(start..)?;
    let mut index = start;
    let mut saw_escaped_delimiter = false;
    let mut escaped = false;
    let mut character_class_contents = None;

    while index < text.len() {
        let mut chars = text[index..].chars();
        let Some(ch) = chars.next() else {
            break;
        };
        let next = index + ch.len_utf8();

        if let Some(contents) = character_class_contents.as_mut() {
            if escaped {
                if ch == delimiter {
                    saw_escaped_delimiter = true;
                }
                *contents += 1;
                escaped = false;
            } else {
                match ch {
                    '\\' => {
                        escaped = true;
                    }
                    '^' if *contents == 0 => {}
                    ']' if *contents > 0 => {
                        character_class_contents = None;
                    }
                    _ => {
                        *contents += 1;
                    }
                }
            }
            index = next;
            continue;
        }

        if escaped {
            if ch == delimiter {
                saw_escaped_delimiter = true;
            }
            escaped = false;
            index = next;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
            }
            '[' => {
                character_class_contents = Some(0);
            }
            ch if ch == delimiter => return Some((index, saw_escaped_delimiter)),
            _ => {}
        }
        index = next;
    }

    None
}

fn uses_delimiter_sensitive_match_escape(
    pattern: &str,
    replacement: &str,
    delimiter: char,
) -> bool {
    delimiter == '/'
        && pattern.contains(delimiter)
        && is_backslash_prefixed_match_escape(replacement)
}

fn is_backslash_prefixed_match_escape(replacement: &str) -> bool {
    replacement == r"\\&"
        || replacement.strip_prefix(r"\\").is_some_and(|rest| {
            matches!(rest.as_bytes(), [b'\\', b'1'..=b'9', ..])
                && rest[1..].bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn strip_matching_sed_script_quotes_in_source(text: &str, quote_mode: SedScriptQuoteMode) -> &str {
    if quote_mode == SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes
        && text.len() >= 4
        && text.starts_with("\\\"")
        && text.ends_with("\\\"")
    {
        strip_backtick_escaped_double_quotes_in_source(text)
    } else {
        strip_shell_matching_quotes_in_source(text)
    }
}

fn strip_shell_matching_quotes_in_source(text: &str) -> &str {
    if text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')))
    {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn strip_backtick_escaped_double_quotes_in_source(text: &str) -> &str {
    debug_assert!(text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\""));
    &text[2..text.len() - 2]
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

fn word_starts_with_literal_dash(word: &Word, source: &str) -> bool {
    matches!(
        word.parts_with_spans().next(),
        Some((WordPart::Literal(text), span)) if text.as_str(source, span).starts_with('-')
    )
}

fn word_starts_with_static_or_literal_dash(word: &Word, source: &str) -> bool {
    static_word_text(word, source).is_some_and(|text| text.starts_with('-'))
        || word_starts_with_literal_dash(word, source)
}

fn parse_rm_command(args: &[&Word], source: &str) -> Option<RmCommandFacts> {
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
        .filter(|word| rm_path_is_dangerous(word, source))
        .map(|word| word.span)
        .collect::<Vec<_>>();

    (!dangerous_path_spans.is_empty()).then_some(RmCommandFacts {
        dangerous_path_spans: dangerous_path_spans.into_boxed_slice(),
    })
}

fn parse_ssh_command(args: &[&Word], source: &str) -> Option<SshCommandFacts> {
    let remote_args = ssh_remote_args(args, source)?;
    let (last_remote_arg, leading_remote_args) = remote_args.split_last()?;
    if leading_remote_args
        .iter()
        .any(|word| word_starts_with_static_or_literal_dash(word, source))
    {
        return None;
    }

    let local_expansion_spans = last_remote_arg
        .is_fully_double_quoted()
        .then(|| {
            double_quoted_expansion_part_spans(last_remote_arg)
                .into_iter()
                .next()
        })
        .flatten()
        .into_iter()
        .collect::<Vec<_>>();

    (!local_expansion_spans.is_empty()).then_some(SshCommandFacts {
        local_expansion_spans: local_expansion_spans.into_boxed_slice(),
    })
}

fn parse_su_command(args: &[&Word], source: &str) -> SuCommandFacts {
    let mut pending_option_arg = false;
    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if pending_option_arg {
                pending_option_arg = false;
            }
            continue;
        };

        if pending_option_arg {
            pending_option_arg = false;
            continue;
        }

        match text.as_ref() {
            "-" | "-l" | "--login" => {
                return SuCommandFacts {
                    has_login_flag: true,
                };
            }
            "--" => {
                break;
            }
            _ if su_long_option_takes_argument(text.as_ref()) => {
                pending_option_arg = true;
                continue;
            }
            _ => {}
        }

        if text.starts_with("--") {
            continue;
        }

        if !text.starts_with('-') {
            continue;
        }

        let mut flags = text[1..].chars().peekable();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => {
                    return SuCommandFacts {
                        has_login_flag: true,
                    }
                }
                flag if su_short_option_takes_argument(flag) => {
                    if flags.peek().is_none() {
                        pending_option_arg = true;
                    }
                    break;
                }
                _ => {}
            }
        }
    }

    SuCommandFacts {
        has_login_flag: false,
    }
}

fn su_long_option_takes_argument(text: &str) -> bool {
    matches!(
        text,
        "--command"
            | "--group"
            | "--shell"
            | "--supp-group"
            | "--whitelist-environment"
    )
}

fn su_short_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'C' | 'c' | 'g' | 'G' | 's' | 'w')
}

fn ssh_remote_args<'a>(args: &'a [&'a Word], source: &str) -> Option<&'a [&'a Word]> {
    let mut index = 0usize;
    let mut saw_local_option = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            saw_local_option = true;
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        saw_local_option = true;
        let consumes_next = ssh_option_consumes_next_argument(text.as_ref())?;
        index += 1;
        if consumes_next {
            args.get(index)?;
            index += 1;
        }
    }

    if saw_local_option {
        return None;
    }

    let _destination = args.get(index)?;
    Some(&args[index + 1..])
}

fn ssh_option_consumes_next_argument(text: &str) -> Option<bool> {
    if !text.starts_with('-') || text == "-" {
        return Some(false);
    }
    if text == "--" {
        return Some(false);
    }

    let bytes = text.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() {
        let flag = bytes[index];
        if ssh_option_requires_argument(flag) {
            return Some(index + 1 == bytes.len());
        }
        if !ssh_option_is_flag(flag) {
            return None;
        }
        index += 1;
    }

    Some(false)
}

fn ssh_option_requires_argument(flag: u8) -> bool {
    matches!(
        flag,
        b'B' | b'b'
            | b'c'
            | b'D'
            | b'E'
            | b'e'
            | b'F'
            | b'I'
            | b'i'
            | b'J'
            | b'L'
            | b'l'
            | b'm'
            | b'O'
            | b'o'
            | b'p'
            | b'P'
            | b'Q'
            | b'R'
            | b'S'
            | b'W'
            | b'w'
    )
}

fn ssh_option_is_flag(flag: u8) -> bool {
    ssh_option_requires_argument(flag)
        || matches!(
            flag,
            b'4' | b'6'
                | b'A'
                | b'a'
                | b'C'
                | b'f'
                | b'G'
                | b'g'
                | b'K'
                | b'k'
                | b'M'
                | b'N'
                | b'n'
                | b'q'
                | b's'
                | b'T'
                | b't'
                | b'V'
                | b'v'
                | b'X'
                | b'x'
                | b'Y'
                | b'y'
        )
}

fn rm_path_is_dangerous(word: &Word, source: &str) -> bool {
    let segments = rm_path_segments(word, source);
    if segments.is_empty() || !rm_path_segment_is_pure_unsafe_parameter(&segments[0]) {
        return false;
    }

    let brace_expansion_active = word.has_active_brace_expansion();
    let tail_start = segments
        .iter()
        .take_while(|segment| rm_path_segment_is_pure_unsafe_parameter(segment))
        .count();
    let tail = rm_path_tail_text(&segments[tail_start..]);

    if tail.is_empty() {
        return tail_start > 1;
    }

    rm_path_tail_is_dangerous(&tail, brace_expansion_active)
}

#[derive(Debug, Default)]
struct RmPathSegment {
    has_unsafe_param: bool,
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
        WordPart::Variable(_) => {
            current_rm_path_segment(segments).has_unsafe_param = true;
        }
        WordPart::Parameter(parameter) => {
            if rm_path_parameter_expansion_is_unsafe(parameter) {
                current_rm_path_segment(segments).has_unsafe_param = true;
            }
        }
        WordPart::ParameterExpansion {
            operator,
            colon_variant: _,
            ..
        } => {
            if rm_path_parameter_op_is_unsafe(operator) {
                current_rm_path_segment(segments).has_unsafe_param = true;
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

fn rm_path_parameter_expansion_is_unsafe(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { .. } => true,
            BourneParameterExpansion::Indirect {
                operator,
                operand: _,
                colon_variant: _,
                ..
            } => operator.as_ref().is_none_or(rm_path_parameter_op_is_unsafe),
            BourneParameterExpansion::Operation { operator, .. } => {
                rm_path_parameter_op_is_unsafe(operator)
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

fn rm_path_segment_is_pure_unsafe_parameter(segment: &RmPathSegment) -> bool {
    segment.has_unsafe_param && !segment.has_literal_text && !segment.has_other_dynamic
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
    if segment.has_unsafe_param || segment.has_other_dynamic {
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

fn rm_path_tail_is_dangerous(tail: &str, brace_expansion_active: bool) -> bool {
    if brace_expansion_active && let Some((prefix, inner, suffix)) = split_brace_expansion(tail) {
        return split_brace_alternatives(inner)
            .into_iter()
            .any(|alternative| {
                rm_path_tail_is_dangerous(&format!("{prefix}{alternative}{suffix}"), true)
            });
    }

    let tail = tail.trim_matches('/');
    if tail.is_empty() {
        return false;
    }

    let components = tail
        .split('/')
        .filter(|component| !component.is_empty())
        .map(rm_path_tail_component)
        .collect::<Vec<_>>();

    RM_DANGEROUS_LITERAL_SUFFIXES
        .iter()
        .any(|dangerous_prefix| {
            let dangerous_components = dangerous_prefix.split('/').collect::<Vec<_>>();
            rm_path_matches_exact_dangerous_prefix(&components, &dangerous_components)
                || rm_path_matches_dangerous_prefix_with_final_dynamic_or_glob(
                    &components,
                    &dangerous_components,
                )
        })
        || components.as_slice().first().is_some_and(|component| {
            components.len() == 1 && rm_tail_component_is_dangerous_wildcard(component)
        })
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

fn rm_path_matches_exact_dangerous_prefix(
    components: &[RmTailComponent<'_>],
    dangerous_components: &[&str],
) -> bool {
    components.len() == dangerous_components.len()
        && components
            .iter()
            .zip(dangerous_components.iter())
            .all(|(component, expected)| {
                rm_tail_component_matches_dangerous_literal(component, expected)
            })
}

fn rm_path_matches_dangerous_prefix_with_final_dynamic_or_glob(
    components: &[RmTailComponent<'_>],
    dangerous_components: &[&str],
) -> bool {
    components.len() == dangerous_components.len() + 1
        && components
            .iter()
            .take(dangerous_components.len())
            .zip(dangerous_components.iter())
            .all(|(component, expected)| {
                rm_tail_component_matches_dangerous_literal(component, expected)
            })
        && matches!(
            components.last(),
            Some(
                RmTailComponent::PureDynamic
                    | RmTailComponent::Literal("*")
                    | RmTailComponent::MixedDynamic("*")
            )
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

fn parse_grep_command<'a>(args: &[&'a Word], source: &str) -> Option<GrepCommandFacts<'a>> {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;
    let mut uses_only_matching = false;
    let mut uses_fixed_strings = false;
    let mut explicit_pattern_source = false;
    let mut patterns = Vec::new();

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;

        if text == "--only-matching" {
            uses_only_matching = true;
            index += 1;
            continue;
        }

        if text == "--fixed-strings" {
            uses_fixed_strings = true;
            index += 1;
            continue;
        }

        if matches!(
            text.as_ref(),
            "--basic-regexp" | "--extended-regexp" | "--perl-regexp"
        ) {
            uses_fixed_strings = false;
            index += 1;
            continue;
        }

        if text == "--regexp" {
            explicit_pattern_source = true;
            if let Some(pattern_word) = args.get(index + 1) {
                patterns.push(grep_pattern_fact(
                    pattern_word,
                    source,
                    GrepPatternSourceKind::LongOptionSeparate,
                ));
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if text.starts_with("--regexp=") {
            explicit_pattern_source = true;
            patterns.push(grep_prefixed_pattern_fact(
                word,
                source,
                "--regexp=".len(),
                GrepPatternSourceKind::LongOptionAttached,
            ));
            index += 1;
            continue;
        }

        if text == "--file" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        if text.starts_with("--file=") {
            explicit_pattern_source = true;
            index += 1;
            continue;
        }

        if text.starts_with("--") {
            index += if grep_long_option_takes_argument(text.as_ref())
                && args.get(index + 1).is_some()
            {
                2
            } else {
                1
            };
            continue;
        }

        if text == "-e" {
            explicit_pattern_source = true;
            if let Some(pattern_word) = args.get(index + 1) {
                patterns.push(grep_pattern_fact(
                    pattern_word,
                    source,
                    GrepPatternSourceKind::ShortOptionSeparate,
                ));
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if text == "-f" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == 'o' {
                uses_only_matching = true;
            }

            if flag == 'F' {
                uses_fixed_strings = true;
            }

            if matches!(flag, 'E' | 'G' | 'P') {
                uses_fixed_strings = false;
            }

            if flag == 'e' {
                explicit_pattern_source = true;
                if chars.peek().is_some() {
                    patterns.push(grep_prefixed_pattern_fact(
                        word,
                        source,
                        2,
                        GrepPatternSourceKind::ShortOptionAttached,
                    ));
                } else if let Some(pattern_word) = args.get(index + 1) {
                    patterns.push(grep_pattern_fact(
                        pattern_word,
                        source,
                        GrepPatternSourceKind::ShortOptionSeparate,
                    ));
                    consume_next_argument = true;
                }
                break;
            }

            if grep_option_takes_argument(flag) {
                if flag == 'f' {
                    explicit_pattern_source = true;
                }
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    if !explicit_pattern_source && let Some(pattern_word) = args.get(index) {
        patterns.push(grep_pattern_fact(
            pattern_word,
            source,
            GrepPatternSourceKind::ImplicitOperand,
        ));
    }

    Some(GrepCommandFacts {
        uses_only_matching,
        uses_fixed_strings,
        patterns: patterns.into_boxed_slice(),
    })
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

fn short_option_cluster_contains_flag(text: &str, flag: char) -> bool {
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

fn grep_file_operand_words<'a>(args: &[&'a Word], source: &str) -> Vec<&'a Word> {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;
    let mut explicit_pattern_source = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;

        if text == "--only-matching"
            || text == "--fixed-strings"
            || matches!(
                text.as_ref(),
                "--basic-regexp" | "--extended-regexp" | "--perl-regexp"
            )
        {
            index += 1;
            continue;
        }

        if text == "--regexp" || text == "--file" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        if text.starts_with("--regexp=") || text.starts_with("--file=") {
            explicit_pattern_source = true;
            index += 1;
            continue;
        }

        if text.starts_with("--") {
            index += if grep_long_option_takes_argument(text.as_ref())
                && args.get(index + 1).is_some()
            {
                2
            } else {
                1
            };
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == 'e' {
                explicit_pattern_source = true;
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }

            if flag == 'f' {
                explicit_pattern_source = true;
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }

            if grep_option_takes_argument(flag) {
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    if !explicit_pattern_source && args.get(index).is_some() {
        index += 1;
    }

    args.get(index..).unwrap_or(&[]).to_vec()
}

fn grep_pattern_fact<'a>(
    word: &'a Word,
    source: &str,
    source_kind: GrepPatternSourceKind,
) -> GrepPatternFact<'a> {
    grep_prefixed_pattern_fact(word, source, 0, source_kind)
}

fn grep_prefixed_pattern_fact<'a>(
    word: &'a Word,
    source: &str,
    prefix_len: usize,
    source_kind: GrepPatternSourceKind,
) -> GrepPatternFact<'a> {
    let (static_text, glob_style_star_replacement_spans) =
        cooked_static_word_text_with_source_spans(word, source)
            .and_then(|(text, source_spans)| {
                let text = text.get(prefix_len..)?.to_owned();
                let source_spans = source_spans.get(prefix_len..)?.to_vec();
                Some((text, source_spans))
            })
            .map(|(text, source_spans)| {
                let spans = grep_pattern_glob_style_star_replacement_spans(&text, &source_spans);
                (Some(text.into_boxed_str()), spans.into_boxed_slice())
            })
            .unwrap_or_else(|| (None, Box::new([])));
    let starts_with_glob_style_star = static_text
        .as_deref()
        .is_some_and(|text| text.starts_with('*') || text == "^*");
    let has_glob_style_star_confusion = !glob_style_star_replacement_spans.is_empty();

    GrepPatternFact {
        word,
        static_text,
        source_kind,
        starts_with_glob_style_star,
        has_glob_style_star_confusion,
        glob_style_star_replacement_spans,
    }
}

fn cooked_static_word_text_with_source_spans(
    word: &Word,
    source: &str,
) -> Option<(String, Vec<Span>)> {
    let mut cooked = Vec::new();
    let mut source_spans = Vec::new();
    collect_cooked_static_word_text_parts_with_source_spans(
        &word.parts,
        source,
        false,
        &mut cooked,
        &mut source_spans,
    )
    .then_some(())?;

    Some((String::from_utf8(cooked).ok()?, source_spans))
}

fn collect_cooked_static_word_text_parts_with_source_spans(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let slice = text.as_str(source, part.span);
                if in_double_quotes {
                    push_cooked_double_quoted_literal_text_with_source_spans(
                        slice,
                        part.span.start,
                        out,
                        source_spans,
                    );
                } else {
                    push_cooked_unquoted_literal_text_with_source_spans(
                        slice,
                        part.span.start,
                        out,
                        source_spans,
                    );
                }
            }
            WordPart::SingleQuoted { value, .. } => {
                let text = value.slice(source);
                push_cooked_literal_text_with_source_spans(
                    text,
                    value.span().start,
                    out,
                    source_spans,
                );
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_cooked_static_word_text_parts_with_source_spans(
                    parts,
                    source,
                    true,
                    out,
                    source_spans,
                ) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => return false,
        }
    }

    true
}

fn push_cooked_literal_text_with_source_spans(
    text: &str,
    start_position: Position,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    for (index, ch) in text.char_indices() {
        let span = Span::from_positions(
            start_position.advanced_by(&text[..index]),
            start_position.advanced_by(&text[..index + ch.len_utf8()]),
        );
        push_cooked_char_with_source_span(ch, span, out, source_spans);
    }
}

fn push_cooked_unquoted_literal_text_with_source_spans(
    text: &str,
    start_position: Position,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\\' {
            if let Some((next_index, escaped)) = chars.next()
                && escaped != '\n'
            {
                let span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..next_index + escaped.len_utf8()]),
                );
                push_cooked_char_with_source_span(escaped, span, out, source_spans);
            }
            continue;
        }

        let span = Span::from_positions(
            start_position.advanced_by(&text[..index]),
            start_position.advanced_by(&text[..index + ch.len_utf8()]),
        );
        push_cooked_char_with_source_span(ch, span, out, source_spans);
    }
}

fn push_cooked_double_quoted_literal_text_with_source_spans(
    text: &str,
    start_position: Position,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch != '\\' {
            let span = Span::from_positions(
                start_position.advanced_by(&text[..index]),
                start_position.advanced_by(&text[..index + ch.len_utf8()]),
            );
            push_cooked_char_with_source_span(ch, span, out, source_spans);
            continue;
        }

        match chars.next() {
            Some((next_index, escaped @ ('$' | '"' | '\\' | '`'))) => {
                let span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..next_index + escaped.len_utf8()]),
                );
                push_cooked_char_with_source_span(escaped, span, out, source_spans);
            }
            Some((_next_index, '\n')) => {}
            Some((next_index, other)) => {
                let backslash_span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..index + ch.len_utf8()]),
                );
                push_cooked_char_with_source_span('\\', backslash_span, out, source_spans);

                let span = Span::from_positions(
                    start_position.advanced_by(&text[..next_index]),
                    start_position.advanced_by(&text[..next_index + other.len_utf8()]),
                );
                push_cooked_char_with_source_span(other, span, out, source_spans);
            }
            None => {
                let span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..index + ch.len_utf8()]),
                );
                push_cooked_char_with_source_span('\\', span, out, source_spans);
            }
        }
    }
}

fn push_cooked_char_with_source_span(
    ch: char,
    source_span: Span,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    let mut buf = [0u8; 4];
    let encoded = ch.encode_utf8(&mut buf).as_bytes();
    out.extend_from_slice(encoded);
    source_spans.extend(std::iter::repeat_n(source_span, encoded.len()));
}

fn grep_pattern_glob_style_star_replacement_spans(text: &str, source_spans: &[Span]) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();

    if bytes.is_empty() {
        return spans;
    }

    if text.starts_with('^')
        || ends_with_unescaped_dollar(bytes)
        || bytes.contains(&b'[')
        || bytes.contains(&b'+')
    {
        return spans;
    }
    if first_unescaped_star_index(bytes).is_some_and(|index| index == 0) {
        return spans;
    }

    let mut index = 0usize;
    while let Some(star_index) = next_unescaped_star_index(bytes, index) {
        if bytes.get(star_index + 1) == Some(&b'\\') {
            index = star_index + 1;
            continue;
        }

        let Some(previous) = previous_unescaped_byte(bytes, star_index) else {
            index = star_index + 1;
            continue;
        };

        if matches!(
            previous,
            b'.' | b']' | b')' | b'*' | b'?' | b'|' | b'$' | b'{' | b'(' | b'\\'
        ) || previous.is_ascii_whitespace()
        {
            index = star_index + 1;
            continue;
        }

        if let Some(span) = source_spans.get(star_index).copied() {
            spans.push(span);
        }
        index = star_index + 1;
    }

    spans
}

fn first_unescaped_star_index(bytes: &[u8]) -> Option<usize> {
    next_unescaped_star_index(bytes, 0)
}

fn next_unescaped_star_index(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'*' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn previous_unescaped_byte(bytes: &[u8], index: usize) -> Option<u8> {
    let mut candidate = index;
    while candidate > 0 {
        candidate -= 1;
        if !is_escaped(bytes, candidate) {
            return Some(bytes[candidate]);
        }
    }
    None
}

fn ends_with_unescaped_dollar(bytes: &[u8]) -> bool {
    bytes
        .last()
        .is_some_and(|byte| *byte == b'$' && !is_escaped(bytes, bytes.len() - 1))
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}
fn grep_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'A' | 'B' | 'C' | 'D' | 'd' | 'e' | 'f' | 'm')
}

fn grep_long_option_takes_argument(option: &str) -> bool {
    let Some(name) = option.strip_prefix("--") else {
        return false;
    };
    if name.contains('=') {
        return false;
    }

    matches!(
        name,
        "after-context"
            | "before-context"
            | "binary-files"
            | "context"
            | "devices"
            | "directories"
            | "exclude"
            | "exclude-dir"
            | "exclude-from"
            | "file"
            | "group-separator"
            | "include"
            | "label"
            | "max-count"
            | "regexp"
    )
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

fn option_takes_argument(flag: char) -> bool {
    matches!(flag, 'a' | 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
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
                if text[1..].chars().any(|flag| flag == 'f') {
                    function_mode = true;
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

fn parse_find_exec_shell_command(
    command: &Command,
    source: &str,
) -> Option<FindExecShellCommandFacts> {
    let Command::Simple(command) = command else {
        return None;
    };

    let words = simple_command_body_words(command, source).collect::<Vec<_>>();
    let mut shell_command_spans = Vec::new();
    let mut index = 0usize;

    while index < words.len() {
        let Some(action) = static_word_text(words[index], source) else {
            index += 1;
            continue;
        };
        if !matches!(action.as_ref(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            index += 1;
            continue;
        }

        let Some(command_name_index) = words.get(index + 1).map(|_| index + 1) else {
            break;
        };
        let argument_start = command_name_index;
        let terminator_index = find_exec_terminator_index(&words[argument_start..], source)
            .map(|offset| argument_start + offset);
        let argument_end = terminator_index.unwrap_or(words.len());

        if matches!(action.as_ref(), "-exec" | "-execdir")
            && let Some(segment) = words.get(argument_start..argument_end)
        {
            shell_command_spans.extend(find_exec_shell_command_spans(segment, source));
        }

        index = terminator_index.map_or(words.len(), |terminator_index| terminator_index + 1);
    }

    (!shell_command_spans.is_empty()).then_some(FindExecShellCommandFacts {
        shell_command_spans: shell_command_spans.into_boxed_slice(),
    })
}

fn find_exec_shell_command_spans(args: &[&Word], source: &str) -> Vec<Span> {
    let Some(normalized) = command::normalize_command_words(args, source) else {
        return Vec::new();
    };
    if normalized.has_wrapper(WrapperKind::FindExec)
        || normalized.has_wrapper(WrapperKind::FindExecDir)
    {
        return Vec::new();
    }
    let Some(shell_name) = normalized
        .effective_name
        .as_deref()
        .map(|name| name.rsplit('/').next().unwrap_or(name))
    else {
        return Vec::new();
    };
    if !matches!(shell_name, "sh" | "bash" | "dash" | "ksh") {
        return Vec::new();
    }

    normalized
        .body_args()
        .windows(2)
        .filter_map(|pair| {
            let flag = static_word_text(pair[0], source)?;
            if !shell_flag_contains_command_string(flag.as_ref()) {
                return None;
            }
            let script = pair[1];
            script
                .span
                .slice(source)
                .contains("{}")
                .then_some(script.span)
        })
        .collect()
}

fn parse_find_exec_argument_word_spans(command: &Command, source: &str) -> Vec<Span> {
    let Command::Simple(command) = command else {
        return Vec::new();
    };

    let words = simple_command_body_words(command, source).collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < words.len() {
        let Some(action) = static_word_text(words[index], source) else {
            index += 1;
            continue;
        };
        if !matches!(action.as_ref(), "-exec" | "-ok" | "-execdir" | "-okdir") {
            index += 1;
            continue;
        }

        let Some(command_name_index) = words.get(index + 1).map(|_| index + 1) else {
            break;
        };
        let argument_start = command_name_index;
        let terminator_index = find_exec_terminator_index(&words[argument_start..], source)
            .map(|offset| argument_start + offset);
        let argument_end = terminator_index.unwrap_or(words.len());

        spans.extend(
            words[argument_start..argument_end]
                .iter()
                .map(|word| word.span),
        );

        index = terminator_index.map_or(words.len(), |terminator_index| terminator_index + 1);
    }

    spans
}

fn find_exec_terminator_index(args: &[&Word], source: &str) -> Option<usize> {
    let semicolon_terminator_index = args
        .iter()
        .position(|word| is_find_exec_semicolon_terminator(word, source));
    let plus_terminator_index = args
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            (index > 0
                && static_word_text(word, source).as_deref() == Some("+")
                && static_word_text(args[index - 1], source).as_deref() == Some("{}"))
            .then_some(index)
        })
        .next();
    match (semicolon_terminator_index, plus_terminator_index) {
        (Some(semicolon_index), Some(plus_index)) => Some(semicolon_index.min(plus_index)),
        (Some(semicolon_index), None) => Some(semicolon_index),
        (None, Some(plus_index)) => Some(plus_index),
        (None, None) => None,
    }
}

fn is_find_exec_semicolon_terminator(word: &Word, source: &str) -> bool {
    match static_word_text(word, source).as_deref() {
        Some(";") => true,
        Some("\\;") => classify_word(word, source).quote == WordQuote::Unquoted,
        _ => false,
    }
}

fn find_command_args<'a>(
    command: &'a Command,
    normalized: &'a NormalizedCommand<'a>,
    source: &'a str,
) -> impl Iterator<Item = &'a Word> + 'a {
    if normalized.literal_name.as_deref() == Some("find")
        && let Command::Simple(command) = command
    {
        return EitherFindCommandArgs::Simple(simple_command_body_words(command, source).skip(1));
    }

    EitherFindCommandArgs::Normalized(normalized.body_args().iter().copied())
}

enum EitherFindCommandArgs<I, J> {
    Simple(I),
    Normalized(J),
}

impl<'a, I, J> Iterator for EitherFindCommandArgs<I, J>
where
    I: Iterator<Item = &'a Word>,
    J: Iterator<Item = &'a Word>,
{
    type Item = &'a Word;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Simple(iter) => iter.next(),
            Self::Normalized(iter) => iter.next(),
        }
    }
}

fn parse_find_command<'a>(
    args: impl IntoIterator<Item = &'a Word>,
    source: &str,
) -> FindCommandFacts {
    let mut has_print0 = false;
    let mut has_formatted_output_action = false;
    let mut or_without_grouping_spans = Vec::new();
    let mut glob_pattern_operand_spans = Vec::new();
    let mut group_stack = vec![FindGroupState::default()];
    let mut pending_argument: Option<FindPendingArgument> = None;

    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if let Some(state) = pending_argument {
                if state.expects_pattern_operand()
                    && !word_spans::word_unquoted_glob_pattern_spans(word, source).is_empty()
                {
                    glob_pattern_operand_spans.push(word.span);
                }
                pending_argument = state.after_consuming_dynamic();
            }
            continue;
        };

        if let Some(state) = pending_argument {
            if state.expects_pattern_operand()
                && !word_spans::word_unquoted_glob_pattern_spans(word, source).is_empty()
            {
                glob_pattern_operand_spans.push(word.span);
            }
            pending_argument = state.after_consuming(text.as_ref());
            continue;
        }

        if text == "-print0" {
            has_print0 = true;
        }

        if is_find_group_open_token(text.as_ref()) {
            group_stack.push(FindGroupState::default());
            continue;
        }

        if is_find_group_close_token(text.as_ref()) {
            if let Some(child) = (group_stack.len() > 1).then(|| group_stack.pop()).flatten() {
                let Some(parent) = group_stack.last_mut() else {
                    unreachable!("group stack retains the root frame");
                };
                parent.incorporate_group(child);
            }
            continue;
        }

        let Some(state) = group_stack.last_mut() else {
            unreachable!("group stack retains the root frame");
        };

        if is_find_or_token(text.as_ref()) {
            state.note_or();
            continue;
        }

        if is_find_and_token(text.as_ref()) {
            state.note_and();
            continue;
        }

        if is_find_branch_action_token(text.as_ref()) {
            if matches!(text.as_ref(), "-fprint0" | "-printf" | "-fprintf") {
                has_formatted_output_action = true;
            }
            state.note_action(
                word.span,
                is_find_reportable_action_token(text.as_ref()),
                &mut or_without_grouping_spans,
            );
            pending_argument = find_pending_argument(text.as_ref());
            continue;
        }

        if is_find_predicate_token(text.as_ref()) {
            state.note_predicate();
            pending_argument = find_pending_argument(text.as_ref());
        }
    }

    FindCommandFacts {
        has_print0,
        has_formatted_output_action,
        or_without_grouping_spans: or_without_grouping_spans.into_boxed_slice(),
        glob_pattern_operand_spans: glob_pattern_operand_spans.into_boxed_slice(),
    }
}

#[derive(Debug, Clone, Copy)]
enum FindPendingArgument {
    Words {
        remaining: usize,
        pattern_operand: bool,
    },
    UntilExecTerminator,
}

impl FindPendingArgument {
    fn after_consuming(self, token: &str) -> Option<Self> {
        match self {
            Self::Words {
                remaining,
                pattern_operand: _,
            } => remaining.checked_sub(1).and_then(|next| {
                (next > 0).then_some(Self::Words {
                    remaining: next,
                    pattern_operand: false,
                })
            }),
            Self::UntilExecTerminator => {
                (!matches!(token, ";" | "\\;" | "+")).then_some(Self::UntilExecTerminator)
            }
        }
    }

    fn after_consuming_dynamic(self) -> Option<Self> {
        match self {
            Self::Words {
                remaining,
                pattern_operand: _,
            } => remaining.checked_sub(1).and_then(|next| {
                (next > 0).then_some(Self::Words {
                    remaining: next,
                    pattern_operand: false,
                })
            }),
            Self::UntilExecTerminator => Some(Self::UntilExecTerminator),
        }
    }

    fn expects_pattern_operand(self) -> bool {
        matches!(
            self,
            Self::Words {
                pattern_operand: true,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FindGroupState {
    saw_or: bool,
    saw_action_before_current_branch: bool,
    current_branch_has_predicate: bool,
    current_branch_has_explicit_and: bool,
    has_any_predicate: bool,
    has_any_action: bool,
}

impl FindGroupState {
    fn current_branch_can_bind_action(&self) -> bool {
        !self.current_branch_has_explicit_and && self.current_branch_has_predicate
    }

    fn note_or(&mut self) {
        self.saw_or = true;
        self.current_branch_has_predicate = false;
        self.current_branch_has_explicit_and = false;
    }

    fn note_and(&mut self) {
        self.current_branch_has_explicit_and = true;
    }

    fn note_predicate(&mut self) {
        self.current_branch_has_predicate = true;
        self.has_any_predicate = true;
    }

    fn note_action(&mut self, span: Span, reportable: bool, spans: &mut Vec<Span>) {
        if reportable
            && self.saw_or
            && !self.saw_action_before_current_branch
            && self.current_branch_can_bind_action()
        {
            spans.push(span);
        }

        self.saw_action_before_current_branch = true;
        self.has_any_action = true;
    }

    fn incorporate_group(&mut self, child: Self) {
        if child.has_any_predicate {
            self.note_predicate();
        }

        if child.has_any_action {
            self.saw_action_before_current_branch = true;
            self.has_any_action = true;
        }
    }
}

fn is_find_group_open_token(token: &str) -> bool {
    matches!(token, "(" | "\\(" | "-(")
}

fn is_find_group_close_token(token: &str) -> bool {
    matches!(token, ")" | "\\)" | "-)")
}

fn is_find_or_token(token: &str) -> bool {
    matches!(token, "-o" | "-or")
}

fn is_find_and_token(token: &str) -> bool {
    matches!(token, "-a" | "-and" | ",")
}

fn is_find_action_token(token: &str) -> bool {
    matches!(
        token,
        "-delete"
            | "-exec"
            | "-execdir"
            | "-ok"
            | "-okdir"
            | "-print"
            | "-print0"
            | "-printf"
            | "-ls"
            | "-fls"
            | "-fprint"
            | "-fprint0"
            | "-fprintf"
    )
}

fn is_find_branch_action_token(token: &str) -> bool {
    is_find_reportable_action_token(token) || matches!(token, "-prune" | "-quit")
}

fn is_find_reportable_action_token(token: &str) -> bool {
    is_find_action_token(token)
}

fn find_pending_argument(token: &str) -> Option<FindPendingArgument> {
    match token {
        "-fls" | "-fprint" | "-fprint0" | "-printf" => Some(FindPendingArgument::Words {
            remaining: 1,
            pattern_operand: false,
        }),
        "-fprintf" => Some(FindPendingArgument::Words {
            remaining: 2,
            pattern_operand: false,
        }),
        "-exec" | "-execdir" | "-ok" | "-okdir" => Some(FindPendingArgument::UntilExecTerminator),
        "-amin" | "-anewer" | "-atime" | "-cmin" | "-cnewer" | "-context" | "-fstype" | "-gid"
        | "-group" | "-ilname" | "-iname" | "-inum" | "-ipath" | "-iregex" | "-links"
        | "-lname" | "-maxdepth" | "-mindepth" | "-mmin" | "-mtime" | "-name" | "-newer"
        | "-path" | "-perm" | "-regex" | "-samefile" | "-size" | "-type" | "-uid" | "-used"
        | "-user" | "-wholename" | "-iwholename" | "-xtype" | "-files0-from" => {
            Some(FindPendingArgument::Words {
                remaining: 1,
                pattern_operand: is_find_pattern_predicate_token(token),
            })
        }
        token if token.starts_with("-newer") && token.len() > "-newer".len() => {
            Some(FindPendingArgument::Words {
                remaining: 1,
                pattern_operand: false,
            })
        }
        _ => None,
    }
}

fn is_find_pattern_predicate_token(token: &str) -> bool {
    matches!(
        token,
        "-name"
            | "-iname"
            | "-path"
            | "-ipath"
            | "-regex"
            | "-iregex"
            | "-lname"
            | "-ilname"
            | "-wholename"
            | "-iwholename"
    )
}

fn is_find_predicate_token(token: &str) -> bool {
    token.starts_with('-')
        && !is_find_branch_action_token(token)
        && !is_find_or_token(token)
        && !is_find_and_token(token)
        && !is_find_group_open_token(token)
        && !is_find_group_close_token(token)
        && !matches!(token, "-not")
}

fn shell_flag_contains_command_string(flag: &str) -> bool {
    let Some(cluster) = flag.strip_prefix('-') else {
        return false;
    };
    !cluster.is_empty()
        && !cluster.starts_with('-')
        && cluster.bytes().all(shell_short_flag_is_clusterable)
        && cluster.bytes().any(|byte| byte == b'c')
}

fn shell_short_flag_is_clusterable(flag: u8) -> bool {
    matches!(
        flag,
        b'a' | b'b'
            | b'c'
            | b'e'
            | b'f'
            | b'h'
            | b'i'
            | b'k'
            | b'l'
            | b'm'
            | b'n'
            | b'p'
            | b'r'
            | b's'
            | b't'
            | b'u'
            | b'v'
            | b'x'
    )
}

fn parse_set_command(args: &[&Word], source: &str) -> SetCommandFacts {
    let mut errexit_change = None;
    let mut errtrace_change = None;
    let mut functrace_change = None;
    let mut pipefail_change = None;
    let mut resets_positional_parameters = false;
    let mut errtrace_flag_spans = Vec::new();
    let mut functrace_flag_spans = Vec::new();
    let mut pipefail_option_spans = Vec::new();
    let mut flags_without_prefix_spans = Vec::new();
    let mut index = 0usize;

    if args.len() >= 2
        && let Some(first_word) = args.first().copied()
        && classify_word(first_word, source).quote == WordQuote::Unquoted
        && let Some(first_text) = static_word_text(first_word, source)
        && first_text != "--"
        && !first_text.starts_with('-')
        && !first_text.starts_with('+')
        && is_shell_variable_name(first_text.as_ref())
    {
        flags_without_prefix_spans.push(first_word.span);
    }

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            resets_positional_parameters = true;
            break;
        };

        if text == "--" {
            resets_positional_parameters = true;
            break;
        }

        match text.as_ref() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if text.starts_with('+') {
                    resets_positional_parameters = true;
                }
                let Some(name_word) = args.get(index + 1) else {
                    break;
                };
                let Some(name) = static_word_text(name_word, source) else {
                    break;
                };

                if name == "errexit" {
                    errexit_change = Some(enable);
                } else if name == "errtrace" {
                    errtrace_change = Some(enable);
                } else if name == "functrace" {
                    functrace_change = Some(enable);
                } else if name == "pipefail" {
                    pipefail_change = Some(enable);
                    pipefail_option_spans.push(name_word.span);
                }
                index += 2;
                continue;
            }
            _ => {}
        }

        let Some(flags) = text.strip_prefix('-').or_else(|| text.strip_prefix('+')) else {
            resets_positional_parameters = true;
            break;
        };
        if flags.is_empty() {
            break;
        }
        if text.starts_with('+') {
            resets_positional_parameters = true;
        }

        if flags.chars().any(|flag| flag == 'e') {
            errexit_change = Some(text.starts_with('-'));
        }
        if flags.chars().any(|flag| flag == 'E') {
            errtrace_change = Some(text.starts_with('-'));
            errtrace_flag_spans.push(word.span);
        }
        if flags.chars().any(|flag| flag == 'T') {
            functrace_change = Some(text.starts_with('-'));
            functrace_flag_spans.push(word.span);
        }

        if flags.chars().any(|flag| flag == 'o') {
            let enable = text.starts_with('-');
            let Some(name_word) = args.get(index + 1) else {
                break;
            };
            let Some(name) = static_word_text(name_word, source) else {
                break;
            };

            if name == "errtrace" {
                errtrace_change = Some(enable);
            } else if name == "functrace" {
                functrace_change = Some(enable);
            } else if name == "pipefail" {
                pipefail_change = Some(enable);
                pipefail_option_spans.push(name_word.span);
            }
            index += 2;
            continue;
        }

        index += 1;
    }

    SetCommandFacts {
        errexit_change,
        errtrace_change,
        functrace_change,
        pipefail_change,
        resets_positional_parameters,
        errtrace_flag_spans: errtrace_flag_spans.into_boxed_slice(),
        functrace_flag_spans: functrace_flag_spans.into_boxed_slice(),
        pipefail_option_spans: pipefail_option_spans.into_boxed_slice(),
        flags_without_prefix_spans: flags_without_prefix_spans.into_boxed_slice(),
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
