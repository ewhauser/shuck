use super::*;

#[derive(Debug, Clone)]
pub struct DeclarationAssignmentProbe {
    kind: DeclarationKind,
    readonly_flag: bool,
    target_name: Box<str>,
    target_name_span: Span,
    has_command_substitution: bool,
    status_capture: bool,
}

impl DeclarationAssignmentProbe {
    pub fn kind(&self) -> &DeclarationKind {
        &self.kind
    }

    pub fn readonly_flag(&self) -> bool {
        self.readonly_flag
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub fn target_name_span(&self) -> Span {
        self.target_name_span
    }

    pub fn has_command_substitution(&self) -> bool {
        self.has_command_substitution
    }

    pub fn status_capture(&self) -> bool {
        self.status_capture
    }
}

#[derive(Debug, Clone)]
pub struct BindingValueFact<'a> {
    kind: BindingValueKind<'a>,
    standalone_status_or_pid_capture: bool,
    conditional_assignment_shortcut: bool,
    one_sided_short_circuit_assignment: bool,
    zsh_selectorless_subscript_value_base_names: Box<[Name]>,
}

#[derive(Debug, Clone)]
pub(crate) enum BindingValueKind<'a> {
    Scalar(&'a Word),
    Loop(Box<[&'a Word]>),
}

impl<'a> BindingValueFact<'a> {
    fn scalar(word: &'a Word, source: &str) -> Self {
        Self::scalar_with_status_or_pid_capture(
            word,
            word_is_standalone_status_or_pid_capture(word),
            source,
        )
    }

    fn scalar_with_status_or_pid_capture(
        word: &'a Word,
        standalone_status_or_pid_capture: bool,
        source: &str,
    ) -> Self {
        Self {
            kind: BindingValueKind::Scalar(word),
            standalone_status_or_pid_capture,
            conditional_assignment_shortcut: false,
            one_sided_short_circuit_assignment: false,
            zsh_selectorless_subscript_value_base_names:
                word_zsh_selectorless_subscript_value_base_names(word, source),
        }
    }

    fn from_loop_words(words: Box<[&'a Word]>) -> Self {
        Self {
            kind: BindingValueKind::Loop(words),
            standalone_status_or_pid_capture: false,
            conditional_assignment_shortcut: false,
            one_sided_short_circuit_assignment: false,
            zsh_selectorless_subscript_value_base_names: Box::default(),
        }
    }

    pub fn scalar_word(&self) -> Option<&'a Word> {
        match &self.kind {
            BindingValueKind::Scalar(word) => Some(*word),
            BindingValueKind::Loop(_) => None,
        }
    }

    pub fn loop_words(&self) -> Option<&[&'a Word]> {
        match &self.kind {
            BindingValueKind::Scalar(_) => None,
            BindingValueKind::Loop(words) => Some(words.as_ref()),
        }
    }

    pub fn conditional_assignment_shortcut(&self) -> bool {
        self.conditional_assignment_shortcut
    }

    pub fn one_sided_short_circuit_assignment(&self) -> bool {
        self.one_sided_short_circuit_assignment
    }

    pub fn standalone_status_or_pid_capture(&self) -> bool {
        self.standalone_status_or_pid_capture
    }

    pub fn zsh_selectorless_subscript_value(&self) -> bool {
        !self.zsh_selectorless_subscript_value_base_names.is_empty()
    }

    pub fn zsh_selectorless_subscript_value_references_base_name(&self, name: &Name) -> bool {
        self.zsh_selectorless_subscript_value_base_names
            .iter()
            .any(|base_name| base_name == name)
    }

    fn mark_conditional_assignment_shortcut(&mut self) {
        self.conditional_assignment_shortcut = true;
    }

    fn mark_one_sided_short_circuit_assignment(&mut self) {
        self.one_sided_short_circuit_assignment = true;
    }
}

pub(crate) fn command_may_have_assignment_spacing_candidate(
    command: &Command,
    source: &str,
) -> bool {
    match command {
        Command::Simple(command) => {
            simple_command_may_have_assignment_spacing_candidate(command, source)
        }
        Command::Decl(command) => {
            command
                .assignments
                .iter()
                .any(assignment_reports_assignment_spacing)
                || declaration_operands_may_have_assignment_spacing_candidate(
                    &command.operands,
                    source,
                )
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => command
                .assignments
                .iter()
                .any(assignment_reports_assignment_spacing),
            BuiltinCommand::Continue(command) => command
                .assignments
                .iter()
                .any(assignment_reports_assignment_spacing),
            BuiltinCommand::Return(command) => command
                .assignments
                .iter()
                .any(assignment_reports_assignment_spacing),
            BuiltinCommand::Exit(command) => command
                .assignments
                .iter()
                .any(assignment_reports_assignment_spacing),
        },
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn simple_command_may_have_assignment_spacing_candidate(
    command: &SimpleCommand,
    source: &str,
) -> bool {
    for (index, assignment) in command.assignments.iter().enumerate() {
        if assignment_reports_assignment_spacing(assignment)
            && (index + 1 < command.assignments.len() || !span_is_empty(command.name.span))
        {
            return true;
        }
    }

    !span_is_empty(command.name.span)
        && word_reports_assignment_spacing(&command.name, source)
        && !command.args.is_empty()
}

fn declaration_operands_may_have_assignment_spacing_candidate(
    operands: &[DeclOperand],
    source: &str,
) -> bool {
    operands.iter().enumerate().any(|(index, operand)| {
        declaration_operand_reports_assignment_spacing(operand, source)
            && index + 1 < operands.len()
    })
}

pub(crate) fn source_may_have_assignment_spacing_candidate(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'=' || !assignment_spacing_gap_can_start_at(bytes, index + 1) {
            index += 1;
            continue;
        }

        let target_end = if index > 0 && bytes[index - 1] == b'+' {
            index - 1
        } else {
            index
        };
        if target_end == 0 {
            index += 1;
            continue;
        }

        if bytes[target_end - 1] == b']' {
            return true;
        }

        let mut target_start = target_end;
        while target_start > 0 && is_shell_name_continue_byte(bytes[target_start - 1]) {
            target_start -= 1;
        }
        if target_start == target_end
            || !is_shell_name_start_byte(bytes[target_start])
            || &source[target_start..target_end] == "IFS"
        {
            index += 1;
            continue;
        }

        return true;
    }

    false
}

fn assignment_spacing_gap_can_start_at(bytes: &[u8], start: usize) -> bool {
    matches!(bytes.get(start), Some(b' ' | b'\t'))
        || matches!(
            (bytes.get(start), bytes.get(start + 1)),
            (Some(b'\\'), Some(b'\n'))
        )
        || matches!(
            (bytes.get(start), bytes.get(start + 1), bytes.get(start + 2)),
            (Some(b'\\'), Some(b'\r'), Some(b'\n'))
        )
}

fn is_shell_name_start_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_shell_name_continue_byte(byte: u8) -> bool {
    is_shell_name_start_byte(byte) || byte.is_ascii_digit()
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_assignment_spacing_spans(
    commands: &[CommandFact<'_>],
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_assignment_spacing_spans_in_command(fact.command(), source, &mut spans);
    }

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

fn collect_assignment_spacing_spans_in_command(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            collect_simple_assignment_spacing_spans(command, source, spans);
        }
        Command::Decl(command) => {
            collect_prefix_assignment_spacing_spans(
                &command.assignments,
                command.variant_span,
                source,
                spans,
            );
            collect_declaration_operand_assignment_spacing_spans(&command.operands, source, spans);
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                collect_prefix_assignment_spacing_spans(
                    &command.assignments,
                    first_word_start_after(command.span, &command.assignments, source),
                    source,
                    spans,
                );
            }
            BuiltinCommand::Continue(command) => {
                collect_prefix_assignment_spacing_spans(
                    &command.assignments,
                    first_word_start_after(command.span, &command.assignments, source),
                    source,
                    spans,
                );
            }
            BuiltinCommand::Return(command) => {
                collect_prefix_assignment_spacing_spans(
                    &command.assignments,
                    first_word_start_after(command.span, &command.assignments, source),
                    source,
                    spans,
                );
            }
            BuiltinCommand::Exit(command) => {
                collect_prefix_assignment_spacing_spans(
                    &command.assignments,
                    first_word_start_after(command.span, &command.assignments, source),
                    source,
                    spans,
                );
            }
        },
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

#[derive(Debug, Clone, Copy)]
struct AssignmentSpacingItem {
    span: Span,
    report: bool,
}

fn collect_simple_assignment_spacing_spans(
    command: &SimpleCommand,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut previous = None;
    for assignment in &command.assignments {
        let item = AssignmentSpacingItem {
            span: assignment.span,
            report: assignment_reports_assignment_spacing(assignment),
        };
        collect_assignment_spacing_gap_from_previous(previous, item, source, spans);
        previous = Some(item);
    }

    if !span_is_empty(command.name.span) {
        let name_item = AssignmentSpacingItem {
            span: command.name.span,
            report: word_reports_assignment_spacing(&command.name, source),
        };
        collect_assignment_spacing_gap_from_previous(previous, name_item, source, spans);

        if name_item.report {
            previous = Some(name_item);
            for arg in &command.args {
                let arg_item = AssignmentSpacingItem {
                    span: arg.span,
                    report: word_reports_assignment_spacing(arg, source),
                };
                collect_assignment_spacing_gap_from_previous(previous, arg_item, source, spans);
                if !arg_item.report {
                    break;
                }
                previous = Some(arg_item);
            }
        }
    }
}

fn collect_prefix_assignment_spacing_spans(
    assignments: &[Assignment],
    fallback_next_span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if assignments.is_empty() {
        return;
    }

    let mut previous = None;
    for assignment in assignments {
        let item = AssignmentSpacingItem {
            span: assignment.span,
            report: assignment_reports_assignment_spacing(assignment),
        };
        collect_assignment_spacing_gap_from_previous(previous, item, source, spans);
        previous = Some(item);
    }
    collect_assignment_spacing_gap_from_previous(
        previous,
        AssignmentSpacingItem {
            span: fallback_next_span,
            report: false,
        },
        source,
        spans,
    );
}

fn collect_declaration_operand_assignment_spacing_spans(
    operands: &[DeclOperand],
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut previous = None;
    for operand in operands {
        let item = AssignmentSpacingItem {
            span: declaration_operand_span(operand),
            report: declaration_operand_has_empty_assignment_value(operand, source),
        };
        collect_assignment_spacing_gap_from_previous(previous, item, source, spans);
        previous = Some(item);
    }
}

fn collect_assignment_spacing_gap_from_previous(
    previous: Option<AssignmentSpacingItem>,
    current: AssignmentSpacingItem,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(previous) = previous else {
        return;
    };
    if !previous.report {
        return;
    }
    if let Some(span) =
        horizontal_whitespace_span_between(previous.span.end, current.span.start, source)
    {
        spans.push(span);
    }
}

fn assignment_has_empty_scalar_value(assignment: &Assignment) -> bool {
    matches!(
        &assignment.value,
        AssignmentValue::Scalar(word) if span_is_empty(word.span)
    )
}

fn assignment_reports_assignment_spacing(assignment: &Assignment) -> bool {
    assignment_has_empty_scalar_value(assignment)
        && !is_assignment_spacing_exempt_target(assignment.target.name.as_str())
}

fn declaration_operand_has_empty_assignment_value(operand: &DeclOperand, source: &str) -> bool {
    declaration_operand_reports_assignment_spacing(operand, source)
}

fn declaration_operand_reports_assignment_spacing(operand: &DeclOperand, source: &str) -> bool {
    match operand {
        DeclOperand::Assignment(assignment) => assignment_reports_assignment_spacing(assignment),
        DeclOperand::Dynamic(word) => word_reports_assignment_spacing(word, source),
        DeclOperand::Flag(_) | DeclOperand::Name(_) => false,
    }
}

fn declaration_operand_span(operand: &DeclOperand) -> Span {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
        DeclOperand::Name(reference) => reference.span,
        DeclOperand::Assignment(assignment) => assignment.span,
    }
}

fn word_reports_assignment_spacing(word: &Word, source: &str) -> bool {
    let Some(target) = plain_literal_assignment_spacing_target(word, source) else {
        return false;
    };
    is_shell_variable_name(target) && !is_assignment_spacing_exempt_target(target)
}

fn plain_literal_assignment_spacing_target<'a>(word: &'a Word, source: &'a str) -> Option<&'a str> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    let WordPart::Literal(text) = &part.kind else {
        return None;
    };
    let text = text.as_str(source, part.span);
    let target = text.strip_suffix("+=").or_else(|| text.strip_suffix('='))?;
    (!target.is_empty() && !target.chars().any(char::is_whitespace)).then_some(target)
}

fn is_assignment_spacing_exempt_target(target: &str) -> bool {
    target == "IFS"
}

fn first_word_start_after(command_span: Span, assignments: &[Assignment], source: &str) -> Span {
    let start = assignments
        .last()
        .map_or(command_span.start.offset, |assignment| {
            assignment.span.end.offset
        });
    let end = command_span.end.offset.min(source.len());
    let next = first_command_word_offset_in_gap(source, start, end);
    let position = command_span
        .start
        .advanced_by(&source[command_span.start.offset..next]);
    Span::from_positions(position, position)
}

fn first_command_word_offset_in_gap(source: &str, start: usize, end: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = start;

    while index < end {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'\\' if index + 1 < end && bytes[index + 1] == b'\n' => index += 2,
            b'\\' if index + 2 < end && bytes[index + 1] == b'\r' && bytes[index + 2] == b'\n' => {
                index += 3;
            }
            _ => return index,
        }
    }

    end
}

fn horizontal_whitespace_span_between(
    start: Position,
    end: Position,
    source: &str,
) -> Option<Span> {
    if start.offset >= end.offset || end.offset > source.len() {
        return None;
    }
    let gap = source.get(start.offset..end.offset)?;
    if gap_is_assignment_spacing_whitespace(gap) {
        Some(Span::from_positions(start, end))
    } else {
        None
    }
}

fn gap_is_assignment_spacing_whitespace(gap: &str) -> bool {
    let bytes = gap.as_bytes();
    let mut index = 0;
    let mut saw_line_continuation = false;

    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' => index += 1,
            b'\\' if bytes.get(index + 1) == Some(&b'\n') => {
                saw_line_continuation = true;
                index += 2;
            }
            b'\\'
                if bytes.get(index + 1) == Some(&b'\r') && bytes.get(index + 2) == Some(&b'\n') =>
            {
                saw_line_continuation = true;
                index += 3;
            }
            _ => return false,
        }
    }

    !gap.is_empty()
        && (saw_line_continuation || bytes.iter().all(|byte| matches!(byte, b' ' | b'\t')))
}

fn span_is_empty(span: Span) -> bool {
    span.start.offset == span.end.offset
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_bare_command_name_assignment_spans<'a>(
    commands: &[CommandFact<'a>],
    word_nodes: &[WordNode<'a>],
    word_occurrences: &[WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    source: &str,
) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|command| {
            bare_command_name_assignment_span(
                command,
                word_nodes,
                word_occurrences,
                word_index,
                source,
            )
        })
        .collect()
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_assignment_like_command_name_spans<'a>(
    commands: &[CommandFact<'a>],
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_assignment_like_command_name_spans_in_command(fact, source, &mut spans);
    }

    spans
}

pub(crate) fn collect_assignment_like_command_name_spans_in_command(
    fact: &CommandFact<'_>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let command = fact.command();
    match command {
        Command::Simple(command) => {
            collect_assignment_like_command_name_span(&command.name, source, spans);
        }
        Command::Decl(command) => {
            for operand in &command.operands {
                if let DeclOperand::Dynamic(word) = operand {
                    if zsh_declaration_brace_assignment_target(word, source, fact.shell_behavior())
                    {
                        continue;
                    }
                    collect_assignment_like_command_name_span(word, source, spans);
                }
            }
        }
        Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

pub(crate) fn collect_assignment_like_command_name_span(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(span) = assignment_like_command_name_span(word, source) {
        spans.push(span);
    }
}

pub(crate) fn assignment_like_command_name_span(word: &Word, source: &str) -> Option<Span> {
    let prefix = leading_literal_word_prefix(word, source);
    let target_end = prefix.find("+=").or_else(|| prefix.find('='))?;
    let target = &prefix[..target_end];
    if target.is_empty() || target.chars().any(char::is_whitespace) {
        return None;
    }

    if let Some(remainder) = target.strip_prefix('+') {
        is_shell_variable_name(remainder).then_some(word.span)
    } else {
        (!is_shell_variable_name(target)).then_some(word.span)
    }
}

pub(crate) fn zsh_declaration_brace_assignment_target(
    word: &Word,
    source: &str,
    behavior: &ShellBehaviorAt<'_>,
) -> bool {
    if behavior.zsh_options().is_none() {
        return false;
    }

    let prefix = leading_literal_word_prefix(word, source);
    let Some(target_end) = prefix.find("+=").or_else(|| prefix.find('=')) else {
        return false;
    };
    let target_end_offset = word.span.start.offset + target_end;

    word.brace_syntax()
        .iter()
        .any(|brace| brace.expands() && brace.span.end.offset <= target_end_offset)
}

pub(crate) fn bare_command_name_assignment_span<'a>(
    command: &CommandFact<'a>,
    word_nodes: &[WordNode<'a>],
    word_occurrences: &[WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    source: &str,
) -> Option<Span> {
    let (assignment, anchor_full_command) = match command.command() {
        Command::Simple(simple) if simple.assignments.len() == 1 => (
            &simple.assignments[0],
            !simple.name.span.slice(source).is_empty(),
        ),
        Command::Builtin(BuiltinCommand::Break(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Builtin(BuiltinCommand::Continue(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Builtin(BuiltinCommand::Return(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Builtin(BuiltinCommand::Exit(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => return None,
    };

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    let fact = word_occurrence_with_context(
        word_nodes,
        word_occurrences,
        word_index,
        word.span,
        WordFactContext::Expansion(ExpansionContext::AssignmentValue),
    )?;
    let analysis = occurrence_analysis(word_nodes, fact);
    if analysis.quote != WordQuote::Unquoted
        || analysis.literalness != WordLiteralness::FixedLiteral
    {
        return None;
    }

    let text = occurrence_static_text(word_nodes, fact, source)?;
    if !is_bare_command_name_assignment_value(&text, command.shell_behavior().zsh_options()) {
        return None;
    }

    Some(if anchor_full_command {
        anchored_assignment_command_span(command, assignment, source)
    } else {
        assignment_target_span(assignment)
    })
}

pub(crate) fn anchored_assignment_command_span(
    command: &CommandFact<'_>,
    assignment: &Assignment,
    source: &str,
) -> Span {
    match command.command() {
        Command::Builtin(_) => return command.span_in_source(source),
        Command::Simple(simple) => {
            let end = simple
                .args
                .last()
                .map(|word| word.span.end)
                .unwrap_or(simple.name.span.end);

            return Span {
                start: assignment.span.start,
                end,
            };
        }
        Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }

    Span {
        start: assignment.span.start,
        end: assignment.span.end,
    }
}

pub(crate) fn assignment_target_span(assignment: &Assignment) -> Span {
    assignment.target.subscript.as_deref().map_or_else(
        || assignment.target.name_span,
        |subscript| {
            Span::from_positions(
                assignment.target.name_span.start,
                subscript.span().end.advanced_by("]"),
            )
        },
    )
}

pub(crate) fn is_bare_command_name_assignment_value(
    text: &str,
    zsh_options: Option<&ZshOptionState>,
) -> bool {
    let text = zsh_literal_assignment_value_for_command_name_check(text, zsh_options);
    matches!(
        text,
        "admin"
            | "alias"
            | "awk"
            | "basename"
            | "bg"
            | "break"
            | "c99"
            | "cat"
            | "cd"
            | "cflow"
            | "chmod"
            | "chown"
            | "cksum"
            | "cmp"
            | "comm"
            | "command"
            | "compress"
            | "continue"
            | "cp"
            | "csplit"
            | "ctags"
            | "cut"
            | "cxref"
            | "date"
            | "dd"
            | "delta"
            | "df"
            | "dirname"
            | "du"
            | "echo"
            | "env"
            | "eval"
            | "ex"
            | "exec"
            | "exit"
            | "expand"
            | "export"
            | "expr"
            | "file"
            | "fg"
            | "find"
            | "fold"
            | "getopts"
            | "grep"
            | "hash"
            | "head"
            | "jobs"
            | "join"
            | "kill"
            | "link"
            | "ln"
            | "ls"
            | "m4"
            | "make"
            | "mkdir"
            | "mkfifo"
            | "more"
            | "mv"
            | "nm"
            | "nice"
            | "nl"
            | "nohup"
            | "od"
            | "paste"
            | "patch"
            | "pathchk"
            | "pax"
            | "printf"
            | "pwd"
            | "read"
            | "readonly"
            | "renice"
            | "return"
            | "rm"
            | "rmdir"
            | "sed"
            | "set"
            | "shift"
            | "sh"
            | "sleep"
            | "sort"
            | "split"
            | "strings"
            | "tail"
            | "test"
            | "time"
            | "touch"
            | "tr"
            | "trap"
            | "tty"
            | "type"
            | "ulimit"
            | "umask"
            | "unalias"
            | "uname"
            | "unexpand"
            | "uniq"
            | "unlink"
            | "unset"
            | "wait"
            | "wc"
            | "xargs"
            | "zcat"
    )
}

pub(crate) fn zsh_literal_assignment_value_for_command_name_check<'a>(
    text: &'a str,
    zsh_options: Option<&ZshOptionState>,
) -> &'a str {
    let Some(candidate) = text.strip_prefix('=') else {
        return text;
    };
    let Some(options) = zsh_options else {
        return text;
    };
    if !options.equals.is_definitely_off() {
        return text;
    }
    candidate
}

#[derive(Debug, Default)]
pub(crate) struct EnvPrefixScopeSpans {
    pub(crate) assignment_scope_spans: Vec<Span>,
    pub(crate) expansion_scope_spans: Vec<Span>,
    pub(crate) expansion_fix_facts: Vec<EnvPrefixExpansionFixFact>,
}

#[derive(Debug, Clone)]
pub struct EnvPrefixExpansionFixFact {
    diagnostic_span: Span,
    assignment_spans: Vec<Span>,
    delete_span: Span,
}

impl EnvPrefixExpansionFixFact {
    pub fn diagnostic_span(&self) -> Span {
        self.diagnostic_span
    }

    pub fn assignment_spans(&self) -> &[Span] {
        &self.assignment_spans
    }

    pub fn delete_span(&self) -> Span {
        self.delete_span
    }
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_env_prefix_scope_spans(
    source: &str,
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
) -> EnvPrefixScopeSpans {
    let mut scope_spans = EnvPrefixScopeSpans::default();
    let mut seen_assignment_scope_spans = FxHashSet::default();
    let mut seen_expansion_scope_spans = FxHashSet::default();

    for command in commands {
        if command_is_assignment_only(command, source) {
            continue;
        }

        let command_span = command.span();
        let assignments = command_assignments(command.command());
        let fix_seed = env_prefix_expansion_fix_seed(source, assignments);
        let broken_legacy_bracket_tail = match command.command() {
            Command::Simple(simple) => broken_legacy_bracket_tail(simple, source),
            Command::Builtin(_)
            | Command::Decl(_)
            | Command::Binary(_)
            | Command::Compound(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => None,
        };

        for (index, assignment) in assignments.iter().enumerate() {
            let span_key = FactSpan::new(assignment.target.name_span);
            let earlier_prefix_uses_name = assignments.iter().take(index).any(|other| {
                assignment_mentions_name_outside_nested_commands(
                    semantic,
                    command_span,
                    other,
                    &assignment.target.name,
                )
            });
            let later_prefix_uses_name =
                assignments
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .any(|(other_index, other)| {
                        assignment_mentions_name_outside_nested_commands(
                            semantic,
                            command_span,
                            other,
                            &assignment.target.name,
                        ) || match (command.command(), broken_legacy_bracket_tail) {
                            (Command::Simple(simple), Some(tail))
                                if tail.assignment_index == other_index =>
                            {
                                broken_legacy_bracket_tail_mentions_name(
                                    semantic,
                                    command_span,
                                    simple,
                                    tail,
                                    &assignment.target.name,
                                )
                            }
                            (
                                Command::Builtin(_)
                                | Command::Decl(_)
                                | Command::Binary(_)
                                | Command::Compound(_)
                                | Command::Function(_)
                                | Command::AnonymousFunction(_),
                                _,
                            )
                            | (Command::Simple(_), _) => false,
                        }
                    });
            let body_uses_name = command_body_mentions_name_outside_nested_commands(
                semantic,
                command,
                source,
                &assignment.target.name,
            );

            if (earlier_prefix_uses_name
                || later_prefix_uses_name
                || (body_uses_name && !assignment_is_identity_self_copy(assignment)))
                && seen_assignment_scope_spans.insert(span_key)
            {
                scope_spans
                    .assignment_scope_spans
                    .push(assignment.target.name_span);
            }

            for (other_index, other) in assignments.iter().enumerate() {
                if other_index == index {
                    continue;
                }

                let _ = visit_assignment_reference_spans_outside_nested_commands(
                    semantic,
                    command_span,
                    other,
                    &assignment.target.name,
                    &mut |span| {
                        push_env_prefix_expansion_fact(
                            span,
                            fix_seed.as_ref(),
                            &mut scope_spans.expansion_scope_spans,
                            &mut seen_expansion_scope_spans,
                            &mut scope_spans.expansion_fix_facts,
                        );
                        ControlFlow::Continue(())
                    },
                );

                match (command.command(), broken_legacy_bracket_tail) {
                    (Command::Simple(simple), Some(tail))
                        if tail.assignment_index == other_index =>
                    {
                        let _ = visit_broken_legacy_bracket_tail_reference_spans(
                            semantic,
                            command_span,
                            simple,
                            tail,
                            &assignment.target.name,
                            &mut |span| {
                                push_env_prefix_expansion_fact(
                                    span,
                                    fix_seed.as_ref(),
                                    &mut scope_spans.expansion_scope_spans,
                                    &mut seen_expansion_scope_spans,
                                    &mut scope_spans.expansion_fix_facts,
                                );
                                ControlFlow::Continue(())
                            },
                        );
                    }
                    (
                        Command::Builtin(_)
                        | Command::Decl(_)
                        | Command::Binary(_)
                        | Command::Compound(_)
                        | Command::Function(_)
                        | Command::AnonymousFunction(_),
                        _,
                    )
                    | (Command::Simple(_), _) => {}
                }
            }

            if assignments.iter().enumerate().any(|(other_index, other)| {
                other_index != index && other.target.name == assignment.target.name
            }) {
                let _ = visit_assignment_reference_spans_outside_nested_commands(
                    semantic,
                    command_span,
                    assignment,
                    &assignment.target.name,
                    &mut |span| {
                        push_env_prefix_expansion_fact(
                            span,
                            fix_seed.as_ref(),
                            &mut scope_spans.expansion_scope_spans,
                            &mut seen_expansion_scope_spans,
                            &mut scope_spans.expansion_fix_facts,
                        );
                        ControlFlow::Continue(())
                    },
                );
            }

            let _ = visit_command_body_reference_spans_outside_nested_commands(
                semantic,
                command,
                source,
                &assignment.target.name,
                &mut |span| {
                    push_env_prefix_expansion_fact(
                        span,
                        fix_seed.as_ref(),
                        &mut scope_spans.expansion_scope_spans,
                        &mut seen_expansion_scope_spans,
                        &mut scope_spans.expansion_fix_facts,
                    );
                    ControlFlow::Continue(())
                },
            );
        }
    }

    scope_spans
        .assignment_scope_spans
        .sort_by_key(|span| (span.start.offset, span.end.offset));
    scope_spans
        .expansion_scope_spans
        .sort_by_key(|span| (span.start.offset, span.end.offset));
    scope_spans.expansion_fix_facts.sort_by_key(|fact| {
        (
            fact.diagnostic_span.start.offset,
            fact.diagnostic_span.end.offset,
        )
    });
    scope_spans
}

#[derive(Debug, Clone)]
pub(crate) struct EnvPrefixExpansionFixSeed {
    assignment_spans: Vec<Span>,
    delete_span: Span,
}

impl EnvPrefixExpansionFixSeed {
    fn for_diagnostic(&self, diagnostic_span: Span) -> EnvPrefixExpansionFixFact {
        EnvPrefixExpansionFixFact {
            diagnostic_span,
            assignment_spans: self.assignment_spans.clone(),
            delete_span: self.delete_span,
        }
    }
}

pub(crate) fn env_prefix_expansion_fix_seed(
    source: &str,
    assignments: &[Assignment],
) -> Option<EnvPrefixExpansionFixSeed> {
    let first_assignment = assignments.first()?;
    let last_assignment = assignments.last()?;
    let prefix = source.get(
        line_start_offset(source, first_assignment.span.start.offset)
            ..first_assignment.span.start.offset,
    )?;
    if !prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
        return None;
    }

    let body_start = skip_env_prefix_separator(source, last_assignment.span.end)?;
    if body_start.offset <= last_assignment.span.end.offset {
        return None;
    }

    Some(EnvPrefixExpansionFixSeed {
        assignment_spans: assignments
            .iter()
            .map(|assignment| assignment.span)
            .collect(),
        delete_span: Span::from_positions(first_assignment.span.start, body_start),
    })
}

pub(crate) fn line_start_offset(source: &str, offset: usize) -> usize {
    source[..offset]
        .rfind('\n')
        .map_or(0, |newline| newline + '\n'.len_utf8())
}

pub(crate) fn skip_env_prefix_separator(source: &str, start: Position) -> Option<Position> {
    let mut position = start;
    let mut tail = source.get(start.offset..)?;
    while let Some(ch) = tail.chars().next() {
        if matches!(ch, ' ' | '\t' | '\r' | '\n') {
            position.advance(ch);
            tail = &tail[ch.len_utf8()..];
            continue;
        }
        if ch == '\\' {
            let mut chars = tail.chars();
            let _ = chars.next();
            if matches!(chars.next(), Some('\n')) {
                position.advance('\\');
                position.advance('\n');
                tail = &tail['\\'.len_utf8() + '\n'.len_utf8()..];
                continue;
            }
        }
        break;
    }
    Some(position)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BrokenLegacyBracketTail {
    assignment_index: usize,
    synthetic_word_count: usize,
}

pub(crate) type EnvPrefixReferenceSpanVisitor<'a> = dyn FnMut(Span) -> ControlFlow<()> + 'a;

pub(crate) fn command_is_assignment_only(fact: &CommandFact<'_>, source: &str) -> bool {
    match fact.command() {
        Command::Simple(command) if !command.assignments.is_empty() => {
            fact.literal_name() == Some("")
                || broken_legacy_bracket_tail(command, source)
                    .is_some_and(|tail| tail.synthetic_word_count == command.args.len() + 1)
        }
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

pub(crate) fn broken_legacy_bracket_tail(
    command: &SimpleCommand,
    source: &str,
) -> Option<BrokenLegacyBracketTail> {
    let assignment_index = command.assignments.len().checked_sub(1)?;
    if !assignment_is_broken_legacy_bracket_arithmetic(&command.assignments[assignment_index]) {
        return None;
    }

    let synthetic_word_count = std::iter::once(&command.name)
        .chain(command.args.iter())
        .position(|word| static_word_text(word, source).as_deref() == Some("]"))?
        + 1;

    Some(BrokenLegacyBracketTail {
        assignment_index,
        synthetic_word_count,
    })
}

pub(crate) fn assignment_is_broken_legacy_bracket_arithmetic(assignment: &Assignment) -> bool {
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return false;
    };
    let [part] = word.parts.as_slice() else {
        return false;
    };
    matches!(
        &part.kind,
        WordPart::ArithmeticExpansion {
            syntax: ArithmeticExpansionSyntax::LegacyBracket,
            expression_ast: None,
            ..
        }
    )
}

pub(crate) fn assignment_mentions_name_outside_nested_commands(
    semantic: &SemanticModel,
    command_span: Span,
    assignment: &Assignment,
    name: &Name,
) -> bool {
    visit_assignment_reference_spans_outside_nested_commands(
        semantic,
        command_span,
        assignment,
        name,
        &mut |_span| ControlFlow::Break(()),
    )
    .is_break()
}

pub(crate) fn command_body_mentions_name_outside_nested_commands(
    semantic: &SemanticModel,
    fact: &CommandFact<'_>,
    source: &str,
    name: &Name,
) -> bool {
    visit_command_body_reference_spans_outside_nested_commands(
        semantic,
        fact,
        source,
        name,
        &mut |_span| ControlFlow::Break(()),
    )
    .is_break()
}

pub(crate) fn simple_command_body_words<'a>(
    command: &'a SimpleCommand,
    source: &'a str,
) -> impl Iterator<Item = &'a Word> {
    let skip =
        broken_legacy_bracket_tail(command, source).map_or(0, |tail| tail.synthetic_word_count);
    std::iter::once(&command.name)
        .chain(command.args.iter())
        .skip(skip)
}

pub(crate) fn broken_legacy_bracket_tail_mentions_name(
    semantic: &SemanticModel,
    command_span: Span,
    command: &SimpleCommand,
    tail: BrokenLegacyBracketTail,
    name: &Name,
) -> bool {
    visit_broken_legacy_bracket_tail_reference_spans(
        semantic,
        command_span,
        command,
        tail,
        name,
        &mut |_span| ControlFlow::Break(()),
    )
    .is_break()
}

pub(crate) fn visit_assignment_reference_spans_outside_nested_commands(
    semantic: &SemanticModel,
    command_span: Span,
    assignment: &Assignment,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    visit_named_command_reference_spans_in_subspan(
        semantic,
        command_span,
        assignment.span,
        name,
        visit,
    )
}

pub(crate) fn visit_command_body_reference_spans_outside_nested_commands(
    semantic: &SemanticModel,
    fact: &CommandFact<'_>,
    source: &str,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match fact.command() {
        Command::Simple(command) => {
            for word in simple_command_body_words(command, source) {
                visit_named_command_reference_spans_in_subspan(
                    semantic,
                    fact.span(),
                    word.span,
                    name,
                    visit,
                )?;
            }
        }
        Command::Builtin(command) => {
            for word in builtin_words(command) {
                visit_named_command_reference_spans_in_subspan(
                    semantic,
                    fact.span(),
                    word.span,
                    name,
                    visit,
                )?;
            }
        }
        Command::Decl(command) => {
            for operand in &command.operands {
                let span = match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
                    DeclOperand::Assignment(assignment) => assignment.span,
                    DeclOperand::Name(_) => continue,
                };
                visit_named_command_reference_spans_in_subspan(
                    semantic,
                    fact.span(),
                    span,
                    name,
                    visit,
                )?;
            }
        }
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }

    for word in fact.redirects().iter().filter_map(Redirect::word_target) {
        visit_named_command_reference_spans_in_subspan(
            semantic,
            fact.span(),
            word.span,
            name,
            visit,
        )?;
    }

    ControlFlow::Continue(())
}

pub(crate) fn visit_broken_legacy_bracket_tail_reference_spans(
    semantic: &SemanticModel,
    command_span: Span,
    command: &SimpleCommand,
    tail: BrokenLegacyBracketTail,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    let Some(span) = broken_legacy_bracket_tail_span(command, tail) else {
        return ControlFlow::Continue(());
    };

    visit_named_command_reference_spans_in_subspan(semantic, command_span, span, name, visit)
}

pub(crate) fn broken_legacy_bracket_tail_span(
    command: &SimpleCommand,
    tail: BrokenLegacyBracketTail,
) -> Option<Span> {
    let mut words = std::iter::once(&command.name)
        .chain(command.args.iter())
        .take(tail.synthetic_word_count.saturating_sub(1));
    let first = words.next()?;
    let last = words.last().unwrap_or(first);
    Some(Span::from_positions(first.span.start, last.span.end))
}

pub(crate) fn builtin_words(command: &BuiltinCommand) -> Vec<&Word> {
    let mut words = Vec::new();
    match command {
        BuiltinCommand::Break(command) => {
            if let Some(word) = &command.depth {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
        BuiltinCommand::Continue(command) => {
            if let Some(word) = &command.depth {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
        BuiltinCommand::Return(command) => {
            if let Some(word) = &command.code {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
        BuiltinCommand::Exit(command) => {
            if let Some(word) = &command.code {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
    }
    words
}

pub(crate) fn visit_named_command_reference_spans_in_subspan(
    semantic: &SemanticModel,
    command_span: Span,
    subspan: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    for reference in semantic.references_in_command_span(command_span, subspan) {
        if &reference.name == name
            && reference_kind_counts_as_env_prefix_command_read(reference.kind)
        {
            visit(reference.span)?;
        }
    }

    ControlFlow::Continue(())
}

pub(crate) fn reference_kind_counts_as_env_prefix_command_read(
    kind: shuck_semantic::ReferenceKind,
) -> bool {
    matches!(
        kind,
        shuck_semantic::ReferenceKind::Expansion
            | shuck_semantic::ReferenceKind::ParameterExpansion
            | shuck_semantic::ReferenceKind::Length
            | shuck_semantic::ReferenceKind::ArrayAccess
            | shuck_semantic::ReferenceKind::IndirectExpansion
            | shuck_semantic::ReferenceKind::ArithmeticRead
            | shuck_semantic::ReferenceKind::ParameterPattern
            | shuck_semantic::ReferenceKind::ParameterSliceArithmetic
            | shuck_semantic::ReferenceKind::ConditionalOperand
            | shuck_semantic::ReferenceKind::RequiredRead
    )
}

pub(crate) fn assignment_is_identity_self_copy(assignment: &Assignment) -> bool {
    if assignment.append {
        return false;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return false;
    };
    word_is_identity_self_copy(word, &assignment.target.name)
}

pub(crate) fn word_is_identity_self_copy(word: &Word, name: &Name) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_identity_self_copy(&part.kind, name)
}

pub(crate) fn word_part_is_identity_self_copy(part: &WordPart, name: &Name) -> bool {
    match part {
        WordPart::Variable(variable) => variable == name,
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return false;
            };
            word_part_is_identity_self_copy(&part.kind, name)
        }
        WordPart::Parameter(parameter) => parameter_is_plain_access_to_name(parameter, name),
        _ => false,
    }
}

pub(crate) fn parameter_is_plain_access_to_name(
    parameter: &ParameterExpansion,
    name: &Name,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.subscript.is_none() =>
        {
            &reference.name == name
        }
        ParameterExpansionSyntax::Zsh(syntax)
            if syntax.operation.is_none()
                && matches!(&syntax.target, ZshExpansionTarget::Reference(reference) if reference.subscript.is_none() && &reference.name == name) =>
        {
            true
        }
        _ => false,
    }
}

pub(crate) fn push_fact_span(
    span: Span,
    spans: &mut Vec<Span>,
    seen: &mut FxHashSet<FactSpan>,
) -> bool {
    let key = FactSpan::new(span);
    if seen.insert(key) {
        spans.push(span);
        true
    } else {
        false
    }
}

pub(crate) fn push_env_prefix_expansion_fact(
    span: Span,
    fix_seed: Option<&EnvPrefixExpansionFixSeed>,
    spans: &mut Vec<Span>,
    seen: &mut FxHashSet<FactSpan>,
    fix_facts: &mut Vec<EnvPrefixExpansionFixFact>,
) {
    if !push_fact_span(span, spans, seen) {
        return;
    }

    if let Some(seed) = fix_seed {
        fix_facts.push(seed.for_diagnostic(span));
    }
}

pub(crate) fn build_plus_equals_assignment_spans(commands: &[CommandFact<'_>]) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_plus_equals_assignment_spans_in_command(fact.command(), &mut spans);
    }

    spans
}

pub(crate) fn collect_plus_equals_assignment_spans_in_command(
    command: &Command,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
            BuiltinCommand::Continue(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
            BuiltinCommand::Return(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
            BuiltinCommand::Exit(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
        },
        Command::Decl(command) => {
            collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            for operand in &command.operands {
                if let DeclOperand::Assignment(assignment) = operand {
                    collect_plus_equals_assignment_span(assignment, spans);
                }
            }
        }
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

pub(crate) fn collect_plus_equals_assignment_spans_in_assignments(
    assignments: &[Assignment],
    spans: &mut Vec<Span>,
) {
    for assignment in assignments {
        collect_plus_equals_assignment_span(assignment, spans);
    }
}

pub(crate) fn collect_plus_equals_assignment_span(assignment: &Assignment, spans: &mut Vec<Span>) {
    if !assignment.append {
        return;
    }

    let target = &assignment.target;
    let end = target
        .subscript
        .as_ref()
        .map(|subscript| subscript.syntax_source_text().span().end.advanced_by("]"))
        .unwrap_or(target.name_span.end);
    spans.push(Span::from_positions(target.name_span.start, end));
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_nonpersistent_assignment_spans(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
    suppress_zsh_nested_subshell_noise: bool,
    suppress_bash_pipefail_pipeline_side_effects: bool,
    arithmetic_only_suppressed_subscript_spans: &[Span],
) -> NonpersistentAssignmentSpans {
    let command_contexts = build_nonpersistent_assignment_command_contexts(commands);
    let prompt_runtime_reads = build_prompt_runtime_read_spans(commands, source)
        .into_iter()
        .map(|read| NonpersistentAssignmentExtraRead {
            name: read.name,
            span: read.span,
            scope: read.scope,
        })
        .collect();
    let analysis =
        semantic.analyze_nonpersistent_assignments(&NonpersistentAssignmentAnalysisContext {
            options: NonpersistentAssignmentAnalysisOptions {
                suppress_bash_pipefail_pipeline_side_effects,
                require_return_use_same_execution_context: suppress_zsh_nested_subshell_noise,
                ignored_names: vec![Name::from("IFS")],
            },
            commands: command_contexts,
            extra_reads: prompt_runtime_reads,
        });
    let mut candidate_effects = Vec::new();
    for effect in analysis.effects {
        if arithmetic_only_suppressed_subscript_spans
            .iter()
            .any(|span| span_contains(*span, effect.assignment_span))
        {
            continue;
        }
        if suppress_zsh_nested_subshell_noise
            && !nonpersistent_assignment_reaches_later_use(semantic, &effect)
        {
            continue;
        }
        if suppress_zsh_nested_subshell_noise
            && zsh_later_use_is_parameter_subscript_flag(source, effect.later_use_span)
        {
            continue;
        }

        candidate_effects.push(effect);
    }

    if candidate_effects.is_empty() {
        return NonpersistentAssignmentSpans::default();
    }

    let needed_reset_names = suppress_zsh_nested_subshell_noise.then(|| {
        candidate_effects
            .iter()
            .map(|effect| effect.name.clone())
            .collect::<FxHashSet<_>>()
    });
    let extra_reset_sites = needed_reset_names.as_ref().map(|needed_names| {
        build_nonpersistent_assignment_extra_reset_sites(
            semantic,
            semantic_analysis,
            commands,
            source,
            needed_names,
        )
    });
    let mut reported_effects = Vec::new();
    for effect in candidate_effects {
        if let Some(extra_reset_sites) = &extra_reset_sites
            && extra_reset_sites.iter().any(|reset| {
                reset.name == effect.name
                    && reset.span.start.offset > effect.assignment_span.end.offset
                    && reset.flow_span.end.offset <= effect.later_use_span.start.offset
                    && nonpersistent_reset_site_covers_later_use(
                        semantic,
                        semantic_analysis,
                        &effect,
                        reset,
                    )
            })
        {
            continue;
        }

        reported_effects.push(effect);
    }

    if reported_effects.is_empty() {
        return NonpersistentAssignmentSpans::default();
    }

    let loop_assignment_spans = build_subshell_loop_assignment_report_spans(commands);
    let mut later_use_sites = Vec::new();
    let mut assignment_sites = Vec::new();

    for effect in reported_effects {
        let assignment_binding = semantic.binding(effect.assignment_binding);
        assignment_sites.push(NamedSpan {
            name: effect.name.clone(),
            span: subshell_assignment_report_span(assignment_binding, &loop_assignment_spans),
        });
        later_use_sites.push(NamedSpan {
            name: effect.name,
            span: effect.later_use_span,
        });
    }

    let mut seen = FxHashSet::default();
    later_use_sites.retain(|site| seen.insert((FactSpan::new(site.span), site.name.clone())));
    later_use_sites.sort_by_key(|site| (site.span.start.offset, site.span.end.offset));

    seen.clear();
    assignment_sites.retain(|site| seen.insert((FactSpan::new(site.span), site.name.clone())));
    assignment_sites.sort_by_key(|site| (site.span.start.offset, site.span.end.offset));

    NonpersistentAssignmentSpans {
        subshell_assignment_sites: assignment_sites,
        subshell_later_use_sites: later_use_sites,
    }
}

pub(crate) fn nonpersistent_assignment_reaches_later_use(
    semantic: &SemanticModel,
    effect: &shuck_semantic::NonpersistentAssignmentEffect,
) -> bool {
    let assignment_scope = semantic.binding(effect.assignment_binding).scope;
    let assignment_transient = semantic.innermost_transient_scope_within_function(assignment_scope);
    let later_use_scope = semantic.scope_at(effect.later_use_span.start.offset);
    let later_use_transient = semantic.innermost_transient_scope_within_function(later_use_scope);
    if let Some(later_use_transient) = later_use_transient {
        let Some(assignment_transient) = assignment_transient else {
            return false;
        };
        if later_use_transient != assignment_transient
            && !semantic
                .ancestor_scopes(assignment_transient)
                .any(|scope| scope == later_use_transient)
        {
            return false;
        }
    }

    true
}

pub(crate) fn zsh_later_use_is_parameter_subscript_flag(source: &str, span: Span) -> bool {
    if span.start.offset + 1 != span.end.offset {
        return false;
    }

    let bytes = source.as_bytes();
    let start = span.start.offset;
    let end = span.end.offset;
    if start == 0
        || end >= bytes.len()
        || bytes[start - 1] != b'('
        || bytes[end] != b')'
        || !bytes[start].is_ascii_alphabetic()
    {
        return false;
    }

    let Some(open_subscript_offset) = source[..start - 1].rfind('[') else {
        return false;
    };
    if open_subscript_offset > 0 && bytes[open_subscript_offset - 1] == b'$' {
        return false;
    }
    let subscript_prefix = &source[open_subscript_offset + 1..start - 1];
    if !subscript_prefix.is_empty() || subscript_prefix.contains(']') {
        return false;
    }

    true
}

pub(crate) fn nonpersistent_reset_site_covers_later_use(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    effect: &shuck_semantic::NonpersistentAssignmentEffect,
    reset: &NonpersistentAssignmentExtraResetSite,
) -> bool {
    let reset_blocks = semantic_analysis
        .block_ids_for_span(reset.flow_span)
        .iter()
        .copied()
        .collect::<FxHashSet<_>>();
    if reset_blocks.is_empty() {
        return false;
    }
    if !reset_site_control_ancestors_contain_later_use(
        semantic,
        reset.command_id,
        effect.later_use_span,
    ) {
        return false;
    }

    let Some(later_use_command) =
        semantic.innermost_command_id_at(effect.later_use_span.start.offset)
    else {
        return false;
    };
    let later_use_blocks =
        semantic_analysis.block_ids_for_span(semantic.command_syntax_span(later_use_command));
    if later_use_blocks.is_empty() {
        return false;
    }

    let assignment_scope = semantic.binding(effect.assignment_binding).scope;
    let entry = semantic_analysis.flow_entry_block_for_binding_scopes(
        &[assignment_scope],
        effect.later_use_span.start.offset,
    );
    later_use_blocks.iter().copied().all(|target| {
        semantic_analysis.blocks_cover_all_paths_to_block(entry, target, &reset_blocks)
    })
}

pub(crate) fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

pub(crate) fn build_subshell_loop_assignment_report_spans(
    commands: &[CommandFact<'_>],
) -> FxHashMap<FactSpan, Span> {
    let mut spans = FxHashMap::default();

    for command in commands {
        match command.command() {
            Command::Compound(CompoundCommand::For(for_command)) => {
                let keyword_span = leading_keyword_span(for_command.span, "for");
                for target in &for_command.targets {
                    if target.name.is_some() {
                        spans.insert(FactSpan::new(target.span), keyword_span);
                    }
                }
            }
            Command::Compound(CompoundCommand::Select(select_command)) => {
                spans.insert(
                    FactSpan::new(select_command.variable_span),
                    leading_keyword_span(select_command.span, "select"),
                );
            }
            _ => {}
        }
    }

    spans
}

pub(crate) fn leading_keyword_span(command_span: Span, keyword: &str) -> Span {
    Span::from_positions(command_span.start, command_span.start.advanced_by(keyword))
}

pub(crate) fn subshell_assignment_report_span(
    binding: &Binding,
    loop_assignment_spans: &FxHashMap<FactSpan, Span>,
) -> Span {
    if binding.kind == BindingKind::LoopVariable
        && let Some(span) = loop_assignment_spans.get(&FactSpan::new(binding.span))
    {
        return *span;
    }

    binding.span
}

#[derive(Debug, Default)]
pub(crate) struct NonpersistentAssignmentSpans {
    pub(crate) subshell_assignment_sites: Vec<NamedSpan>,
    pub(crate) subshell_later_use_sites: Vec<NamedSpan>,
}

#[derive(Debug, Clone)]
pub(crate) struct NonpersistentAssignmentExtraResetSite {
    name: Name,
    span: Span,
    flow_span: Span,
    command_id: CommandId,
}

pub(crate) fn build_nonpersistent_assignment_extra_reset_sites(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
    needed_names: &FxHashSet<Name>,
) -> Vec<NonpersistentAssignmentExtraResetSite> {
    let helper_output_names_by_scope =
        helper_output_names_by_scope(semantic, semantic_analysis, needed_names);
    let zsh_set_a_outparam_positions_by_scope =
        zsh_set_a_outparam_positions_by_scope(semantic, semantic_analysis, commands, source);
    let helper_function_names = helper_function_names_for_scopes(
        semantic,
        semantic_analysis,
        &helper_output_names_by_scope,
        &zsh_set_a_outparam_positions_by_scope,
    );
    let needs_reply_reset = needed_names.contains("REPLY") || needed_names.contains("reply");
    if helper_function_names.is_empty() && !needs_reply_reset {
        return Vec::new();
    }
    let reply_function_names = needs_reply_reset.then(|| function_definition_names(semantic));
    let mut resets = Vec::new();

    for command in commands {
        let Some(command_name) = command.effective_or_literal_name() else {
            continue;
        };
        let can_call_relevant_helper = helper_function_names.contains(command_name);
        let can_set_reply_by_name =
            needs_reply_reset && zsh_helper_name_can_set_reply(command_name);
        if !can_call_relevant_helper && !can_set_reply_by_name {
            continue;
        }
        if !command_runs_in_persistent_shell_context(semantic, command, source) {
            continue;
        }

        let reply_name_may_resolve_to_function = can_set_reply_by_name
            && reply_function_names
                .as_ref()
                .is_some_and(|names| names.contains(command_name));
        let resolved_function_scope = (can_call_relevant_helper
            || reply_name_may_resolve_to_function)
            .then(|| resolved_function_scope_for_command(semantic, semantic_analysis, command))
            .flatten();

        let Some((callee_scope, call_name_span)) = resolved_function_scope else {
            if let Some(reset_span) = zsh_reply_helper_reset_span(command) {
                let flow_span =
                    reset_flow_span_for_command(semantic, semantic_analysis, command, source);
                resets.push(NonpersistentAssignmentExtraResetSite {
                    name: Name::from("REPLY"),
                    span: reset_span,
                    flow_span,
                    command_id: command.id(),
                });
                resets.push(NonpersistentAssignmentExtraResetSite {
                    name: Name::from("reply"),
                    span: reset_span,
                    flow_span,
                    command_id: command.id(),
                });
            }
            continue;
        };

        let names = helper_output_names_by_scope.get(&callee_scope);
        let positions = zsh_set_a_outparam_positions_by_scope.get(&callee_scope);
        if names.is_none() && positions.is_none() {
            continue;
        }

        let flow_span = reset_flow_span_for_command(semantic, semantic_analysis, command, source);
        if let Some(names) = names {
            resets.extend(names.iter().cloned().map(|name| {
                NonpersistentAssignmentExtraResetSite {
                    name,
                    span: call_name_span,
                    flow_span,
                    command_id: command.id(),
                }
            }));
        }

        if let Some(positions) = positions {
            for position in positions {
                let Some(argument) = command.body_args().get(position.saturating_sub(1)).copied()
                else {
                    continue;
                };
                let Some(name) = static_word_text(argument, source) else {
                    continue;
                };
                if !is_shell_variable_name(&name) {
                    continue;
                }
                if !needed_names.contains(name.as_ref()) {
                    continue;
                }
                resets.push(NonpersistentAssignmentExtraResetSite {
                    name: Name::from(name.as_ref()),
                    span: argument.span,
                    flow_span,
                    command_id: command.id(),
                });
            }
        }
    }

    resets.sort_by(|left, right| {
        left.name
            .as_str()
            .cmp(right.name.as_str())
            .then_with(|| left.span.start.offset.cmp(&right.span.start.offset))
            .then_with(|| left.span.end.offset.cmp(&right.span.end.offset))
            .then_with(|| {
                left.flow_span
                    .start
                    .offset
                    .cmp(&right.flow_span.start.offset)
            })
            .then_with(|| left.flow_span.end.offset.cmp(&right.flow_span.end.offset))
            .then_with(|| left.command_id.index().cmp(&right.command_id.index()))
    });
    resets.dedup_by(|left, right| {
        left.name == right.name
            && left.span == right.span
            && left.flow_span == right.flow_span
            && left.command_id == right.command_id
    });
    resets
}

pub(crate) fn reset_flow_span_for_command(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    command: &CommandFact<'_>,
    source: &str,
) -> Span {
    let command_span = command.span();
    let mut current = command.id();
    let mut pipeline_span = None;
    while let Some(parent) = semantic.syntax_backed_command_parent_id(current) {
        if matches!(semantic.command_kind(parent), CommandKind::Binary) {
            let parent_span = semantic.command_syntax_span(parent);
            if span_contains(parent_span, command_span)
                && !semantic_analysis.block_ids_for_span(parent_span).is_empty()
            {
                let before_command = &source[parent_span.start.offset..command_span.start.offset];
                if text_ends_with_pipeline_operator(before_command.trim_end()) {
                    let previous_len = pipeline_span
                        .map(|span: Span| span.end.offset - span.start.offset)
                        .unwrap_or(usize::MAX);
                    let parent_len = parent_span.end.offset - parent_span.start.offset;
                    if parent_len < previous_len {
                        pipeline_span = Some(parent_span);
                    }
                }
            }
        }
        current = parent;
    }

    pipeline_span.unwrap_or(command_span)
}

pub(crate) fn zsh_reply_helper_reset_span(command: &CommandFact<'_>) -> Option<Span> {
    let name = command.effective_or_literal_name()?;
    if !zsh_helper_name_can_set_reply(name) {
        return None;
    }

    Some(command.body_name_word()?.span)
}

pub(crate) fn zsh_helper_name_can_set_reply(name: &str) -> bool {
    (name.starts_with('.') && name != "." && name != "..") || name.starts_with('_')
}

pub(crate) fn helper_output_names_by_scope(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    needed_names: &FxHashSet<Name>,
) -> FxHashMap<ScopeId, Vec<Name>> {
    let mut names_by_scope = FxHashMap::<ScopeId, Vec<Name>>::default();

    for binding in semantic.bindings() {
        if !needed_names.contains(binding.name.as_str()) {
            continue;
        }
        if !helper_binding_can_reset_parent_scope(semantic, semantic_analysis, binding) {
            continue;
        }
        if !matches!(semantic.scope_kind(binding.scope), ScopeKind::Function(_)) {
            continue;
        }

        names_by_scope
            .entry(binding.scope)
            .or_default()
            .push(binding.name.clone());
    }

    for names in names_by_scope.values_mut() {
        names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        names.dedup();
    }

    names_by_scope
}

pub(crate) fn helper_function_names_for_scopes(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    helper_output_names_by_scope: &FxHashMap<ScopeId, Vec<Name>>,
    zsh_set_a_outparam_positions_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
) -> FxHashSet<Name> {
    let mut helper_scopes = FxHashSet::default();
    helper_scopes.extend(helper_output_names_by_scope.keys().copied());
    helper_scopes.extend(zsh_set_a_outparam_positions_by_scope.keys().copied());
    if helper_scopes.is_empty() {
        return FxHashSet::default();
    }

    let mut function_names = FxHashSet::default();
    for binding in semantic.bindings() {
        if !matches!(binding.kind, BindingKind::FunctionDefinition) {
            continue;
        }
        let Some(function_scope) = semantic_analysis.function_scope_for_binding(binding.id) else {
            continue;
        };
        if helper_scopes.contains(&function_scope) {
            function_names.insert(binding.name.clone());
        }
    }

    function_names
}

pub(crate) fn function_definition_names(semantic: &SemanticModel) -> FxHashSet<Name> {
    semantic
        .bindings()
        .iter()
        .filter(|binding| matches!(binding.kind, BindingKind::FunctionDefinition))
        .map(|binding| binding.name.clone())
        .collect()
}

pub(crate) fn helper_binding_can_reset_parent_scope(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    binding: &Binding,
) -> bool {
    if binding.attributes.contains(BindingAttributes::LOCAL) {
        return false;
    }
    if !binding_command_is_unconditional_in_function(semantic, semantic_analysis, binding) {
        return false;
    }

    match binding.kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::Declaration(_) => binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED),
        BindingKind::LoopVariable
        | BindingKind::FunctionDefinition
        | BindingKind::Nameref
        | BindingKind::Imported => false,
    }
}

pub(crate) fn binding_command_is_unconditional_in_function(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    binding: &Binding,
) -> bool {
    let Some(current) = semantic.innermost_command_id_at(binding.span.start.offset) else {
        return true;
    };

    command_is_unconditional_in_function(semantic, semantic_analysis, current)
}

pub(crate) fn command_is_unconditional_in_function(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    mut current: CommandId,
) -> bool {
    if !command_has_reachable_cfg_block(semantic, semantic_analysis, current) {
        return false;
    }

    while let Some(parent) = semantic.syntax_backed_command_parent_id(current) {
        if matches!(semantic.command_kind(parent), CommandKind::Function) {
            return true;
        }
        if reset_site_is_always_run_binary_operand(semantic, parent, current) {
            current = parent;
            continue;
        }
        if command_kind_may_skip_child(semantic.command_kind(parent)) {
            return false;
        }
        current = parent;
    }

    true
}

pub(crate) fn command_has_reachable_cfg_block(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    command_id: CommandId,
) -> bool {
    semantic_analysis
        .block_ids_for_span(semantic.command_syntax_span(command_id))
        .iter()
        .any(|block| !semantic_analysis.block_is_unreachable(*block))
}

pub(crate) fn zsh_set_a_outparam_positions_by_scope(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
) -> FxHashMap<ScopeId, Vec<usize>> {
    let mut positions_by_scope = FxHashMap::<ScopeId, Vec<usize>>::default();

    for command in commands {
        if !command.effective_name_is("set") {
            continue;
        }
        if !command_runs_in_persistent_shell_context(semantic, command, source) {
            continue;
        }
        if !command_is_unconditional_in_function(semantic, semantic_analysis, command.id()) {
            continue;
        }
        let Some(function_scope) = semantic.enclosing_function_scope(command.scope()) else {
            continue;
        };

        positions_by_scope
            .entry(function_scope)
            .or_default()
            .extend(zsh_set_a_outparam_positions(command.body_args(), source));
    }

    for positions in positions_by_scope.values_mut() {
        positions.sort_unstable();
        positions.dedup();
    }

    positions_by_scope
}

pub(crate) fn zsh_set_a_outparam_positions(args: &[&Word], source: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut saw_array_flag = false;

    for word in args {
        let text = word.span.slice(source);
        if !saw_array_flag {
            if static_word_text(word, source).is_some_and(|text| text == "-A") {
                saw_array_flag = true;
            }
            continue;
        }

        if let Some(position) = positional_outparam_index_from_word_text(text) {
            positions.push(position);
        }
        break;
    }

    positions
}

pub(crate) fn positional_outparam_index_from_word_text(text: &str) -> Option<usize> {
    positional_outparam_index(text).or_else(|| {
        text.strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .and_then(positional_outparam_index)
    })
}

pub(crate) fn positional_outparam_index(text: &str) -> Option<usize> {
    let parameter = text
        .strip_prefix("${")
        .and_then(|inner| inner.strip_suffix('}'))
        .or_else(|| text.strip_prefix('$'))?;
    let index = parameter.parse::<usize>().ok()?;
    (index > 0).then_some(index)
}

pub(crate) fn resolved_function_scope_for_command(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    command: &CommandFact<'_>,
) -> Option<(ScopeId, Span)> {
    let name = Name::from(command.effective_or_literal_name()?);
    let name_span = command.body_name_word()?.span;
    let binding = semantic_analysis
        .visible_function_binding_at_call(&name, name_span)
        .or_else(|| {
            command_is_inside_function_body(semantic, command.scope()).then(|| {
                semantic_analysis
                    .function_call_arity_sites(&name)
                    .find_map(|(site, binding)| {
                        (site.name_span == name_span
                            && fallback_function_binding_is_callable_from_function_body(
                                semantic,
                                semantic_analysis,
                                command,
                                name_span,
                                binding,
                            ))
                        .then_some(binding)
                    })
            })?
        })?;
    let scope = semantic_analysis.function_scope_for_binding(binding)?;
    Some((scope, name_span))
}

pub(crate) fn fallback_function_binding_is_callable_from_function_body(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    command: &CommandFact<'_>,
    call_name_span: Span,
    binding: BindingId,
) -> bool {
    let binding = semantic.binding(binding);
    if binding.span.start.offset <= call_name_span.start.offset {
        return true;
    }

    let Some(command_function_scope) = enclosing_function_scope(semantic, command.scope()) else {
        return false;
    };
    if Some(command_function_scope) == enclosing_function_scope(semantic, binding.scope) {
        return false;
    }

    !function_scope_may_run_before_offset(
        semantic,
        semantic_analysis,
        command_function_scope,
        binding.span.start.offset,
    )
}

pub(crate) fn function_scope_may_run_before_offset(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    function_scope: ScopeId,
    offset: usize,
) -> bool {
    semantic_analysis
        .function_bindings_by_scope()
        .find(|(scope, _)| *scope == function_scope)
        .is_some_and(|(_, bindings)| {
            bindings.iter().copied().any(|function_binding| {
                let name = &semantic.binding(function_binding).name;
                semantic_analysis.resolved_function_call_sites(name).any(
                    |(site, resolved_binding)| {
                        resolved_binding == function_binding && site.name_span.start.offset < offset
                    },
                )
            })
        })
}

pub(crate) fn command_is_inside_function_body(semantic: &SemanticModel, scope: ScopeId) -> bool {
    enclosing_function_scope(semantic, scope).is_some()
}

pub(crate) fn enclosing_function_scope(
    semantic: &SemanticModel,
    scope: ScopeId,
) -> Option<ScopeId> {
    if matches!(semantic.scope_kind(scope), ScopeKind::Function(_)) {
        return Some(scope);
    }

    semantic
        .ancestor_scopes(scope)
        .find(|scope| matches!(semantic.scope_kind(*scope), ScopeKind::Function(_)))
}

pub(crate) fn command_runs_in_persistent_shell_context(
    semantic: &SemanticModel,
    command: &CommandFact<'_>,
    source: &str,
) -> bool {
    if matches!(
        command.stmt().terminator,
        Some(StmtTerminator::Background(_))
    ) {
        return false;
    }

    let transient_scopes = semantic
        .transient_ancestor_scopes_within_function(command.scope())
        .collect::<SmallVec<[_; 2]>>();
    if transient_scopes.is_empty() {
        return true;
    }

    transient_scopes
        .iter()
        .all(|scope| matches!(semantic.scope_kind(*scope), ScopeKind::Pipeline))
        && zsh_pipeline_tail_runs_in_current_shell(command, semantic, source)
}

pub(crate) fn zsh_pipeline_tail_runs_in_current_shell(
    command: &CommandFact<'_>,
    semantic: &SemanticModel,
    source: &str,
) -> bool {
    if command.shell_behavior().shell_dialect() != shuck_semantic::ShellDialect::Zsh {
        return false;
    }
    if command
        .shell_behavior()
        .zsh_options()
        .is_some_and(|options| {
            *options == ZshOptionState::for_emulate(shuck_semantic::ZshEmulationMode::Sh)
        })
    {
        return false;
    }

    command_is_pipeline_tail_operand(semantic, command.id(), source)
        || command_is_textual_pipeline_tail_operand(command.span(), source)
}

pub(crate) fn command_is_pipeline_tail_operand(
    semantic: &SemanticModel,
    mut current: CommandId,
    source: &str,
) -> bool {
    let mut saw_pipeline_parent = false;
    while let Some(parent) = semantic.syntax_backed_command_parent_id(current) {
        if !matches!(semantic.command_kind(parent), CommandKind::Binary) {
            break;
        }
        match binary_child_pipe_tail_relation(semantic, parent, current, source) {
            Some(true) => {
                saw_pipeline_parent = true;
                current = parent;
            }
            Some(false) => return false,
            None => break,
        }
    }
    saw_pipeline_parent
}

pub(crate) fn binary_child_pipe_tail_relation(
    semantic: &SemanticModel,
    parent: CommandId,
    child: CommandId,
    source: &str,
) -> Option<bool> {
    let parent_span = semantic.command_syntax_span(parent);
    let child_span = semantic.command_syntax_span(child);
    if child_span.start == parent_span.start {
        let after_child = &source[child_span.end.offset..parent_span.end.offset];
        return text_starts_with_pipeline_operator(after_child.trim_start()).map(|_| false);
    }

    let before_child = &source[parent_span.start.offset..child_span.start.offset];
    let before_child = before_child.trim_end();
    text_ends_with_pipeline_operator(before_child).then_some(true)
}

pub(crate) fn command_is_textual_pipeline_tail_operand(command_span: Span, source: &str) -> bool {
    let before_command = &source[..command_span.start.offset];
    let before_command = before_command.trim_end();
    if !text_ends_with_pipeline_operator(before_command) {
        return false;
    }

    let after_command = &source[command_span.end.offset..];
    let after_command = after_command.trim_start();
    !after_command.starts_with('|')
}

pub(crate) fn text_ends_with_pipeline_operator(text: &str) -> bool {
    text.ends_with("|&") || (text.ends_with('|') && !text.ends_with("||"))
}

pub(crate) fn text_starts_with_pipeline_operator(text: &str) -> Option<()> {
    (text.starts_with("|&") || (text.starts_with('|') && !text.starts_with("||"))).then_some(())
}

pub(crate) fn reset_site_control_ancestors_contain_later_use(
    semantic: &SemanticModel,
    command_id: CommandId,
    later_use_span: Span,
) -> bool {
    let mut current = command_id;
    while let Some(parent) = semantic.syntax_backed_command_parent_id(current) {
        if reset_site_is_always_run_binary_operand(semantic, parent, current) {
            current = parent;
            continue;
        }
        if matches!(semantic.command_kind(parent), CommandKind::Binary) {
            current = parent;
            continue;
        }
        if command_kind_may_skip_child(semantic.command_kind(parent))
            && !span_contains(semantic.command_syntax_span(parent), later_use_span)
        {
            return false;
        }
        current = parent;
    }
    true
}

pub(crate) fn reset_site_is_always_run_binary_operand(
    semantic: &SemanticModel,
    parent: CommandId,
    child: CommandId,
) -> bool {
    if !matches!(semantic.command_kind(parent), CommandKind::Binary) {
        return false;
    }

    semantic.command_syntax_span(parent).start == semantic.command_syntax_span(child).start
}

pub(crate) fn command_kind_may_skip_child(kind: CommandKind) -> bool {
    match kind {
        CommandKind::Binary => true,
        CommandKind::Compound(
            CompoundCommandKind::If
            | CompoundCommandKind::For
            | CompoundCommandKind::Repeat
            | CompoundCommandKind::Foreach
            | CompoundCommandKind::ArithmeticFor
            | CompoundCommandKind::While
            | CompoundCommandKind::Until
            | CompoundCommandKind::Case
            | CompoundCommandKind::Select,
        ) => true,
        CommandKind::Simple
        | CommandKind::Builtin(_)
        | CommandKind::Decl
        | CommandKind::Compound(
            CompoundCommandKind::Subshell
            | CompoundCommandKind::BraceGroup
            | CompoundCommandKind::Arithmetic
            | CompoundCommandKind::Time
            | CompoundCommandKind::Conditional
            | CompoundCommandKind::Coproc
            | CompoundCommandKind::Always,
        )
        | CommandKind::Function
        | CommandKind::AnonymousFunction => false,
    }
}

pub(crate) fn build_nonpersistent_assignment_command_contexts(
    commands: &[CommandFact<'_>],
) -> Vec<NonpersistentAssignmentCommandContext> {
    commands
        .iter()
        .map(|command| {
            let mut prefix_reset_names = command_assignments(command.command())
                .iter()
                .map(|assignment| assignment.target.name.clone())
                .collect::<Vec<_>>();
            prefix_reset_names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
            prefix_reset_names.dedup();

            NonpersistentAssignmentCommandContext {
                span: command.span(),
                prefix_reset_names,
            }
        })
        .collect()
}

pub(crate) struct PromptRuntimeRead {
    name: Name,
    span: Span,
    scope: ScopeId,
}

pub(crate) fn build_prompt_runtime_read_spans(
    commands: &[CommandFact<'_>],
    source: &str,
) -> Vec<PromptRuntimeRead> {
    let mut reads = Vec::new();

    for command in commands {
        let scope = command.scope();
        for assignment in command_assignments(command.command()) {
            collect_prompt_runtime_reads_from_assignment(assignment, scope, source, &mut reads);
        }
        for operand in declaration_operands(command.command()) {
            if let DeclOperand::Assignment(assignment) = operand {
                collect_prompt_runtime_reads_from_assignment(assignment, scope, source, &mut reads);
            }
        }
    }

    let mut seen = FxHashSet::default();
    reads.retain(|read| seen.insert((FactSpan::new(read.span), read.name.clone())));
    reads
}

pub(crate) fn collect_prompt_runtime_reads_from_assignment(
    assignment: &Assignment,
    scope: ScopeId,
    source: &str,
    reads: &mut Vec<PromptRuntimeRead>,
) {
    if assignment.target.name.as_str() != "PS4" {
        return;
    }
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return;
    };

    let target_span = assignment_target_span(assignment);
    for name in escaped_braced_parameter_names(word.span.slice(source)) {
        reads.push(PromptRuntimeRead {
            name: Name::from(name.as_str()),
            span: target_span,
            scope,
        });
    }
}

pub(crate) fn escaped_braced_parameter_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut index = 0;

    while let Some(relative) = text[index..].find(r"\${") {
        let name_start = index + relative + 3;
        let mut name_end = name_start;
        for (offset, ch) in text[name_start..].char_indices() {
            if offset == 0 {
                if !(ch == '_' || ch.is_ascii_alphabetic()) {
                    break;
                }
            } else if !(ch == '_' || ch.is_ascii_alphanumeric()) {
                break;
            }
            name_end = name_start + offset + ch.len_utf8();
        }

        if name_end > name_start {
            let name = &text[name_start..name_end];
            if is_shell_variable_name(name) {
                names.push(name.to_owned());
            }
        }
        index = name_start.max(name_end);
    }

    names
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_innermost_command_ids_by_offset(
    commands: &[CommandFact<'_>],
    mut offsets: Vec<usize>,
) -> CommandOffsetLookup {
    if offsets.is_empty() {
        return CommandOffsetLookup::default();
    }

    offsets.sort_unstable();
    offsets.dedup();

    let mut entries = Vec::with_capacity(offsets.len());
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some(command) = commands.get(next_command) {
            let span = command.span();
            if span.start.offset > offset {
                break;
            }

            pop_finished_commands(&mut active_commands, span.start.offset);
            active_commands.push(OpenCommand {
                end_offset: span.end.offset,
                id: command.id(),
            });
            next_command += 1;
        }

        pop_finished_commands(&mut active_commands, offset);
        if let Some(command) = active_commands.last() {
            entries.push(CommandOffsetLookupEntry {
                offset,
                id: command.id,
            });
        }
    }

    CommandOffsetLookup { entries }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct CommandOffsetLookup {
    entries: Vec<CommandOffsetLookupEntry>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommandOffsetLookupEntry {
    offset: usize,
    id: CommandId,
}

pub(crate) fn precomputed_command_id_for_offset(
    command_ids_by_offset: &CommandOffsetLookup,
    offset: usize,
) -> Option<CommandId> {
    command_ids_by_offset
        .entries
        .binary_search_by_key(&offset, |entry| entry.offset)
        .ok()
        .map(|index| command_ids_by_offset.entries[index].id)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenCommand {
    end_offset: usize,
    id: CommandId,
}

pub(crate) fn pop_finished_commands(active_commands: &mut Vec<OpenCommand>, offset: usize) {
    while active_commands
        .last()
        .is_some_and(|command| command.end_offset < offset)
    {
        active_commands.pop();
    }
}

pub(crate) fn build_dollar_question_after_command_spans(
    commands: &StmtSeq,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    BodyShapeAnalyzer::new(source).visit_status_available_sites(commands, true, &mut |site| {
        match site {
            StatusAvailableSite::SimpleTest(command) => {
                collect_c107_status_spans_in_simple_test(command, source, &mut spans);
            }
            StatusAvailableSite::ConditionalExpression(expression) => {
                collect_c107_status_spans_in_conditional_expr(expression, source, &mut spans);
            }
            StatusAvailableSite::ArithmeticCommand(command) => {
                collect_c107_status_spans_in_arithmetic_command(command, source, &mut spans);
            }
        }
    });

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

pub(crate) fn build_declaration_assignment_probes<'a>(
    command: &'a Command,
    normalized: &NormalizedCommand<'a>,
    semantic: &SemanticModel,
    source: &str,
    behavior: &ShellBehaviorAt<'_>,
) -> Vec<DeclarationAssignmentProbe> {
    if let Some(declaration) = normalized.declaration.as_ref() {
        return declaration
            .assignment_operands
            .iter()
            .filter_map(|assignment| {
                let AssignmentValue::Scalar(word) = &assignment.value else {
                    return None;
                };

                Some(DeclarationAssignmentProbe {
                    kind: declaration.kind.clone(),
                    readonly_flag: declaration.readonly_flag,
                    target_name: assignment.target.name.as_str().into(),
                    target_name_span: assignment.target.name_span,
                    has_command_substitution: word_has_command_substitution(word, source, behavior),
                    status_capture: word_is_standalone_status_capture(word),
                })
            })
            .collect();
    }

    let Command::Simple(_) = command else {
        return Vec::new();
    };

    if !normalized.wrappers.is_empty() {
        return Vec::new();
    }

    let Some(declaration) = semantic.declaration_for_command_span(command_span(command)) else {
        return Vec::new();
    };
    let kind = declaration_kind_from_semantic(declaration.builtin);
    let readonly_flag = semantic_declaration_readonly_flag(declaration);

    declaration
        .operands
        .iter()
        .filter_map(|operand| {
            let SemanticDeclarationOperand::Assignment {
                name,
                name_span,
                value_span,
                has_command_substitution,
                ..
            } = operand
            else {
                return None;
            };
            Some(DeclarationAssignmentProbe {
                kind: kind.clone(),
                readonly_flag,
                target_name: name.as_str().into(),
                target_name_span: *name_span,
                has_command_substitution: *has_command_substitution,
                status_capture: word_for_declaration_value_span(command, *value_span)
                    .is_some_and(|word| word_span_is_standalone_status_capture(word, *value_span)),
            })
        })
        .collect()
}

pub(crate) fn declaration_kind_from_semantic(builtin: DeclarationBuiltin) -> DeclarationKind {
    match builtin {
        DeclarationBuiltin::Export => DeclarationKind::Export,
        DeclarationBuiltin::Local => DeclarationKind::Local,
        DeclarationBuiltin::Declare => DeclarationKind::Declare,
        DeclarationBuiltin::Typeset => DeclarationKind::Typeset,
        DeclarationBuiltin::Readonly => DeclarationKind::Other("readonly".to_owned()),
    }
}

pub(crate) fn semantic_declaration_readonly_flag(declaration: &Declaration) -> bool {
    if !matches!(
        declaration.builtin,
        DeclarationBuiltin::Local | DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset
    ) {
        return false;
    }

    declaration.operands.iter().any(|operand| match operand {
        SemanticDeclarationOperand::Flag { flags, .. } => {
            flags.starts_with('-') && flags.contains('r')
        }
        SemanticDeclarationOperand::Name { .. }
        | SemanticDeclarationOperand::Assignment { .. }
        | SemanticDeclarationOperand::DynamicWord { .. } => false,
    })
}

pub(crate) fn word_for_declaration_value_span(command: &Command, span: Span) -> Option<&Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    command.args.iter().find(|word| {
        span.start.offset >= word.span.start.offset && span.end.offset <= word.span.end.offset
    })
}

pub(crate) fn word_span_is_standalone_status_capture(word: &Word, span: Span) -> bool {
    let parts = word_parts_in_span(word, span);
    matches!(parts.as_slice(), [part] if part_is_standalone_status_capture(&part.kind))
}

pub(crate) fn word_span_is_standalone_status_or_pid_capture(word: &Word, span: Span) -> bool {
    let parts = word_parts_in_span(word, span);
    matches!(parts.as_slice(), [part] if part_is_standalone_status_or_pid_capture(&part.kind))
}

pub(crate) fn word_parts_in_span(word: &Word, span: Span) -> Vec<&WordPartNode> {
    word.parts
        .iter()
        .filter(|part| {
            span.start.offset <= part.span.start.offset && part.span.end.offset <= span.end.offset
        })
        .collect()
}

pub(crate) fn part_is_standalone_status_capture(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "?",
        WordPart::DoubleQuoted { parts, .. } => {
            matches!(parts.as_slice(), [part] if part_is_standalone_status_capture(&part.kind))
        }
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if reference.name.as_str() == "?" && reference.subscript.is_none()
        ),
        _ => false,
    }
}

pub(crate) fn word_is_standalone_status_or_pid_capture(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [part] if part_is_standalone_status_or_pid_capture(&part.kind))
}

pub(crate) fn part_is_standalone_status_or_pid_capture(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => matches!(name.as_str(), "?" | "!"),
        WordPart::DoubleQuoted { parts, .. } => {
            matches!(
                parts.as_slice(),
                [part] if part_is_standalone_status_or_pid_capture(&part.kind)
            )
        }
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if matches!(reference.name.as_str(), "?" | "!") && reference.subscript.is_none()
        ),
        _ => false,
    }
}

pub(crate) fn word_has_command_substitution(
    word: &Word,
    source: &str,
    behavior: &ShellBehaviorAt<'_>,
) -> bool {
    word_classification_from_analysis(analyze_word(word, source, Some(behavior)))
        .has_command_substitution()
}

pub(crate) fn word_zsh_selectorless_subscript_value_base_names(
    word: &Word,
    source: &str,
) -> Box<[Name]> {
    let mut base_names = Vec::new();
    if word_part_nodes_are_zsh_selectorless_subscript_value(&word.parts, source, &mut base_names)
        && !base_names.is_empty()
    {
        base_names.into_boxed_slice()
    } else {
        Box::default()
    }
}

pub(crate) fn word_part_nodes_are_zsh_selectorless_subscript_value(
    parts: &[WordPartNode],
    source: &str,
    base_names: &mut Vec<Name>,
) -> bool {
    for (index, part) in parts.iter().enumerate() {
        if let WordPart::Variable(name) = &part.kind
            && parts.get(index + 1).is_some_and(|next| {
                matches!(
                    &next.kind,
                    WordPart::Literal(text) if literal_starts_with_zsh_subscript(text.as_str(source, next.span))
                )
            })
        {
            base_names.push(name.clone());
            continue;
        }
        if !word_part_is_zsh_selectorless_subscript_value(&part.kind, source, base_names) {
            return false;
        }
    }
    true
}

pub(crate) fn word_part_is_zsh_selectorless_subscript_value(
    part: &WordPart,
    source: &str,
    base_names: &mut Vec<Name>,
) -> bool {
    match part {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => {
            word_part_nodes_are_zsh_selectorless_subscript_value(parts, source, base_names)
        }
        WordPart::Parameter(parameter) => {
            parameter_is_zsh_selectorless_subscript_value(parameter, base_names)
        }
        WordPart::ArrayAccess(reference) | WordPart::ArraySlice { reference, .. } => {
            if !var_ref_has_selectorless_subscript(reference) {
                return false;
            }
            base_names.push(reference.name.clone());
            true
        }
        WordPart::ZshQualifiedGlob(_)
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

pub(crate) fn literal_starts_with_zsh_subscript(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('[') else {
        return false;
    };
    rest.contains(']')
}

pub(crate) fn parameter_is_zsh_selectorless_subscript_value(
    parameter: &ParameterExpansion,
    base_names: &mut Vec<Name>,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
            if !var_ref_has_selectorless_subscript(reference) {
                return false;
            }
            base_names.push(reference.name.clone());
            true
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::PrefixMatch { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax)
            if syntax.length_prefix.is_none()
                && syntax.operation.is_none()
                && syntax.modifiers.is_empty() =>
        {
            match &syntax.target {
                ZshExpansionTarget::Reference(reference) => {
                    if !var_ref_has_selectorless_subscript(reference) {
                        return false;
                    }
                    base_names.push(reference.name.clone());
                    true
                }
                ZshExpansionTarget::Nested(parameter) => {
                    parameter_is_zsh_selectorless_subscript_value(parameter, base_names)
                }
                ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => false,
            }
        }
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

pub(crate) fn var_ref_has_selectorless_subscript(reference: &VarRef) -> bool {
    reference
        .subscript
        .as_deref()
        .is_some_and(|subscript| subscript.selector().is_none())
}

pub(crate) fn advance_escaped_char_boundary(text: &str, start: usize) -> usize {
    let next = start + '\\'.len_utf8();
    if next >= text.len() {
        return next;
    }

    next + text[next..].chars().next().map_or(0, char::len_utf8)
}

pub(crate) fn collect_binding_values<'a>(
    command: &'a Command,
    semantic: &SemanticModel,
    source: &str,
    binding_values: &mut FxHashMap<BindingId, BindingValueFact<'a>>,
) {
    let assignments = match command {
        Command::Simple(simple) if simple.name.span.slice(source).is_empty() => &simple.assignments,
        Command::Builtin(_) | Command::Decl(_) => command_assignments(command),
        Command::Simple(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => &[],
    };

    for assignment in assignments {
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        if let Some(binding_id) =
            binding_value_definition_id_for_span(semantic, assignment.target.name_span)
        {
            binding_values.insert(binding_id, BindingValueFact::scalar(word, source));
        }
    }

    for operand in declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        if let Some(binding_id) =
            binding_value_definition_id_for_span(semantic, assignment.target.name_span)
        {
            binding_values.insert(binding_id, BindingValueFact::scalar(word, source));
        }
    }

    if matches!(command, Command::Simple(_))
        && let Some(declaration) = semantic.declaration_for_command_span(command_span(command))
    {
        for operand in &declaration.operands {
            let SemanticDeclarationOperand::Assignment {
                name: _,
                name_span,
                value_span,
                ..
            } = operand
            else {
                continue;
            };
            let Some(word) = word_for_declaration_value_span(command, *value_span) else {
                continue;
            };
            let standalone_status_or_pid_capture =
                word_span_is_standalone_status_or_pid_capture(word, *value_span);
            if let Some(binding_id) = binding_value_definition_id_for_span(semantic, *name_span) {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::scalar_with_status_or_pid_capture(
                        word,
                        standalone_status_or_pid_capture,
                        source,
                    ),
                );
            }
        }
    }

    match command {
        Command::Compound(CompoundCommand::For(command)) => {
            let Some(words) = &command.words else {
                return;
            };
            let values = words.iter().collect::<Vec<_>>().into_boxed_slice();
            for target in &command.targets {
                if target.name.is_some()
                    && let Some(binding_id) =
                        binding_value_definition_id_for_span(semantic, target.span)
                {
                    binding_values.insert(
                        binding_id,
                        BindingValueFact::from_loop_words(values.clone()),
                    );
                }
            }
        }
        Command::Compound(CompoundCommand::Foreach(command)) => {
            if let Some(binding_id) =
                binding_value_definition_id_for_span(semantic, command.variable_span)
            {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::from_loop_words(
                        command.words.iter().collect::<Vec<_>>().into_boxed_slice(),
                    ),
                );
            }
        }
        Command::Compound(CompoundCommand::Select(command)) => {
            if let Some(binding_id) =
                binding_value_definition_id_for_span(semantic, command.variable_span)
            {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::from_loop_words(
                        command.words.iter().collect::<Vec<_>>().into_boxed_slice(),
                    ),
                );
            }
        }
        _ => {}
    }
}

pub(crate) fn binding_value_definition_id_for_span(
    semantic: &SemanticModel,
    span: Span,
) -> Option<BindingId> {
    semantic.binding_for_definition_span(span)
}

pub(crate) fn binding_value_visible_id_for_name(
    semantic: &SemanticModel,
    name: &Name,
    span: Span,
) -> Option<BindingId> {
    semantic
        .visible_binding(name, span)
        .map(|binding| binding.id)
}

pub(crate) fn annotate_conditional_assignment_value_paths<'a>(
    semantic: &SemanticModel,
    lists: &[ListFact<'a>],
    binding_values: &mut FxHashMap<BindingId, BindingValueFact<'a>>,
) {
    for list in lists
        .iter()
        .filter(|list| list_has_conditional_assignment_shortcuts(list))
    {
        for segment in list.segments() {
            let Some(target) = segment.assignment_target() else {
                continue;
            };
            let Some(span) = segment.assignment_span() else {
                continue;
            };
            let Some(binding_id) =
                binding_value_visible_id_for_name(semantic, &Name::from(target), span)
            else {
                continue;
            };
            if let Some(binding_value) = binding_values.get_mut(&binding_id) {
                binding_value.mark_conditional_assignment_shortcut();
            }
        }
    }

    for list in lists
        .iter()
        .filter(|list| !list_has_conditional_assignment_shortcuts(list))
    {
        let mut prior_assignment_targets = FxHashSet::default();
        for (index, segment) in list.segments().iter().enumerate() {
            let Some(target) = segment.assignment_target() else {
                continue;
            };
            let Some(span) = segment.assignment_span() else {
                continue;
            };
            if index > 0
                && !prior_assignment_targets.contains(target)
                && let Some(binding_id) =
                    binding_value_visible_id_for_name(semantic, &Name::from(target), span)
                && let Some(binding_value) = binding_values.get_mut(&binding_id)
            {
                binding_value.mark_one_sided_short_circuit_assignment();
            }
            prior_assignment_targets.insert(target.to_owned());
        }
    }
}

pub(crate) fn list_has_conditional_assignment_shortcuts(list: &ListFact<'_>) -> bool {
    if list.mixed_short_circuit_kind() == Some(MixedShortCircuitKind::AssignmentTernary) {
        return true;
    }

    let [_, then_branch, else_branch] = list.segments() else {
        return false;
    };
    let [first_operator, second_operator] = list.operators() else {
        return false;
    };

    first_operator.op() == shuck_ast::BinaryOp::And
        && second_operator.op() == shuck_ast::BinaryOp::Or
        && then_branch.assignment_target().is_some()
        && then_branch.assignment_target() == else_branch.assignment_target()
}
