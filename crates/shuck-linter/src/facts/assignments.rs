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
    zsh_selectorless_subscript_value: bool,
}

#[derive(Debug, Clone)]
enum BindingValueKind<'a> {
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
            zsh_selectorless_subscript_value: word_is_zsh_selectorless_subscript_value(
                word, source,
            ),
        }
    }

    fn from_loop_words(words: Box<[&'a Word]>) -> Self {
        Self {
            kind: BindingValueKind::Loop(words),
            standalone_status_or_pid_capture: false,
            conditional_assignment_shortcut: false,
            one_sided_short_circuit_assignment: false,
            zsh_selectorless_subscript_value: false,
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
        self.zsh_selectorless_subscript_value
    }

    fn mark_conditional_assignment_shortcut(&mut self) {
        self.conditional_assignment_shortcut = true;
    }

    fn mark_one_sided_short_circuit_assignment(&mut self) {
        self.one_sided_short_circuit_assignment = true;
    }
}

#[cfg_attr(shuck_profiling, inline(never))]
fn build_bare_command_name_assignment_spans<'a>(
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
fn build_assignment_like_command_name_spans<'a>(
    commands: &[CommandFact<'a>],
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_assignment_like_command_name_spans_in_command(fact, source, &mut spans);
    }

    spans
}

fn collect_assignment_like_command_name_spans_in_command(
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

fn collect_assignment_like_command_name_span(word: &Word, source: &str, spans: &mut Vec<Span>) {
    if let Some(span) = assignment_like_command_name_span(word, source) {
        spans.push(span);
    }
}

fn assignment_like_command_name_span(word: &Word, source: &str) -> Option<Span> {
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

fn zsh_declaration_brace_assignment_target(
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

fn bare_command_name_assignment_span<'a>(
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
    if analysis.quote != WordQuote::Unquoted || analysis.literalness != WordLiteralness::FixedLiteral
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

fn anchored_assignment_command_span(
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

fn assignment_target_span(assignment: &Assignment) -> Span {
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

fn is_bare_command_name_assignment_value(text: &str, zsh_options: Option<&ZshOptionState>) -> bool {
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

fn zsh_literal_assignment_value_for_command_name_check<'a>(
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
struct EnvPrefixScopeSpans {
    assignment_scope_spans: Vec<Span>,
    expansion_scope_spans: Vec<Span>,
}

#[cfg_attr(shuck_profiling, inline(never))]
fn build_env_prefix_scope_spans(
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
                        push_fact_span(
                            span,
                            &mut scope_spans.expansion_scope_spans,
                            &mut seen_expansion_scope_spans,
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
                                push_fact_span(
                                    span,
                                    &mut scope_spans.expansion_scope_spans,
                                    &mut seen_expansion_scope_spans,
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
                        push_fact_span(
                            span,
                            &mut scope_spans.expansion_scope_spans,
                            &mut seen_expansion_scope_spans,
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
                    push_fact_span(
                        span,
                        &mut scope_spans.expansion_scope_spans,
                        &mut seen_expansion_scope_spans,
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
    scope_spans
}

#[derive(Debug, Clone, Copy)]
struct BrokenLegacyBracketTail {
    assignment_index: usize,
    synthetic_word_count: usize,
}

type EnvPrefixReferenceSpanVisitor<'a> = dyn FnMut(Span) -> ControlFlow<()> + 'a;

fn command_is_assignment_only(fact: &CommandFact<'_>, source: &str) -> bool {
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

fn broken_legacy_bracket_tail(
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

fn assignment_is_broken_legacy_bracket_arithmetic(assignment: &Assignment) -> bool {
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

fn assignment_mentions_name_outside_nested_commands(
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

fn command_body_mentions_name_outside_nested_commands(
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

pub(super) fn simple_command_body_words<'a>(
    command: &'a SimpleCommand,
    source: &'a str,
) -> impl Iterator<Item = &'a Word> {
    let skip =
        broken_legacy_bracket_tail(command, source).map_or(0, |tail| tail.synthetic_word_count);
    std::iter::once(&command.name)
        .chain(command.args.iter())
        .skip(skip)
}

fn broken_legacy_bracket_tail_mentions_name(
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

fn visit_assignment_reference_spans_outside_nested_commands(
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

fn visit_command_body_reference_spans_outside_nested_commands(
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

fn visit_broken_legacy_bracket_tail_reference_spans(
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

fn broken_legacy_bracket_tail_span(
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

fn builtin_words(command: &BuiltinCommand) -> Vec<&Word> {
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

fn visit_named_command_reference_spans_in_subspan(
    semantic: &SemanticModel,
    command_span: Span,
    subspan: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    for reference in semantic.references_in_command_span(command_span, subspan) {
        if &reference.name == name && reference_kind_counts_as_env_prefix_command_read(reference.kind)
        {
            visit(reference.span)?;
        }
    }

    ControlFlow::Continue(())
}

fn reference_kind_counts_as_env_prefix_command_read(kind: shuck_semantic::ReferenceKind) -> bool {
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

fn assignment_is_identity_self_copy(assignment: &Assignment) -> bool {
    if assignment.append {
        return false;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return false;
    };
    word_is_identity_self_copy(word, &assignment.target.name)
}

fn word_is_identity_self_copy(word: &Word, name: &Name) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_identity_self_copy(&part.kind, name)
}

fn word_part_is_identity_self_copy(part: &WordPart, name: &Name) -> bool {
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

fn parameter_is_plain_access_to_name(parameter: &ParameterExpansion, name: &Name) -> bool {
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

fn push_fact_span(span: Span, spans: &mut Vec<Span>, seen: &mut FxHashSet<FactSpan>) {
    let key = FactSpan::new(span);
    if seen.insert(key) {
        spans.push(span);
    }
}


fn build_plus_equals_assignment_spans(commands: &[CommandFact<'_>]) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_plus_equals_assignment_spans_in_command(fact.command(), &mut spans);
    }

    spans
}

fn collect_plus_equals_assignment_spans_in_command(command: &Command, spans: &mut Vec<Span>) {
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

fn collect_plus_equals_assignment_spans_in_assignments(
    assignments: &[Assignment],
    spans: &mut Vec<Span>,
) {
    for assignment in assignments {
        collect_plus_equals_assignment_span(assignment, spans);
    }
}

fn collect_plus_equals_assignment_span(assignment: &Assignment, spans: &mut Vec<Span>) {
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
fn build_nonpersistent_assignment_spans(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
    suppress_zsh_nested_subshell_noise: bool,
    suppress_bash_pipefail_pipeline_side_effects: bool,
    arithmetic_only_suppressed_subscript_spans: &[Span],
) -> NonpersistentAssignmentSpans {
    let command_contexts = build_nonpersistent_assignment_command_contexts(commands);
    let extra_reset_sites = suppress_zsh_nested_subshell_noise.then(|| {
        build_nonpersistent_assignment_extra_reset_sites(semantic, semantic_analysis, commands, source)
    });
    let prompt_runtime_reads = build_prompt_runtime_read_spans(commands, source)
        .into_iter()
        .map(|read| NonpersistentAssignmentExtraRead {
            name: read.name,
            span: read.span,
            scope: read.scope,
        })
        .collect();
    let analysis = semantic.analyze_nonpersistent_assignments(
        &NonpersistentAssignmentAnalysisContext {
            options: NonpersistentAssignmentAnalysisOptions {
                suppress_bash_pipefail_pipeline_side_effects,
                ignored_names: vec![Name::from("IFS")],
            },
            commands: command_contexts,
            extra_reads: prompt_runtime_reads,
        },
    );
    let loop_assignment_spans = build_subshell_loop_assignment_report_spans(commands);
    let mut later_use_sites = Vec::new();
    let mut assignment_sites = Vec::new();

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

fn nonpersistent_assignment_reaches_later_use(
    semantic: &SemanticModel,
    effect: &shuck_semantic::NonpersistentAssignmentEffect,
) -> bool {
    let assignment_scope = semantic.binding(effect.assignment_binding).scope;
    let assignment_transient = semantic.innermost_transient_scope_within_function(assignment_scope);
    let later_use_scope = semantic.scope_at(effect.later_use_span.start.offset);
    let later_use_transient = semantic.innermost_transient_scope_within_function(later_use_scope);
    if later_use_transient.is_some() && later_use_transient != assignment_transient {
        return false;
    }

    true
}

fn nonpersistent_reset_site_covers_later_use(
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
    let entry = semantic_analysis
        .flow_entry_block_for_binding_scopes(&[assignment_scope], effect.later_use_span.start.offset);
    later_use_blocks
        .iter()
        .copied()
        .all(|target| semantic_analysis.blocks_cover_all_paths_to_block(entry, target, &reset_blocks))
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn build_subshell_loop_assignment_report_spans(
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

fn leading_keyword_span(command_span: Span, keyword: &str) -> Span {
    Span::from_positions(command_span.start, command_span.start.advanced_by(keyword))
}

fn subshell_assignment_report_span(
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
struct NonpersistentAssignmentSpans {
    subshell_assignment_sites: Vec<NamedSpan>,
    subshell_later_use_sites: Vec<NamedSpan>,
}

#[derive(Debug, Clone)]
struct NonpersistentAssignmentExtraResetSite {
    name: Name,
    span: Span,
    flow_span: Span,
    command_id: CommandId,
}

fn build_nonpersistent_assignment_extra_reset_sites(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
) -> Vec<NonpersistentAssignmentExtraResetSite> {
    let helper_output_names_by_scope = helper_output_names_by_scope(semantic, semantic_analysis);
    let zsh_set_a_outparam_positions_by_scope =
        zsh_set_a_outparam_positions_by_scope(semantic, semantic_analysis, commands, source);
    let mut resets = Vec::new();

    for command in commands {
        if !command_runs_in_persistent_shell_context(semantic, command, source) {
            continue;
        }
        let flow_span = reset_flow_span_for_command(semantic_analysis, commands, command, source);

        let Some((callee_scope, call_name_span)) =
            resolved_function_scope_for_command(semantic, semantic_analysis, command)
        else {
            if let Some(reset_span) = zsh_reply_helper_reset_span(command) {
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

        if let Some(names) = helper_output_names_by_scope.get(&callee_scope) {
            resets.extend(names.iter().cloned().map(|name| NonpersistentAssignmentExtraResetSite {
                name,
                span: call_name_span,
                flow_span,
                command_id: command.id(),
            }));
        }

        if let Some(positions) = zsh_set_a_outparam_positions_by_scope.get(&callee_scope) {
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
            .then_with(|| left.flow_span.start.offset.cmp(&right.flow_span.start.offset))
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

fn reset_flow_span_for_command(
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    command: &CommandFact<'_>,
    source: &str,
) -> Span {
    let command_span = command.span();
    let pipeline_span = commands
        .iter()
        .filter(|candidate| candidate.id() != command.id())
        .filter(|candidate| span_contains(candidate.span(), command_span))
        .filter(|candidate| !semantic_analysis.block_ids_for_span(candidate.span()).is_empty())
        .filter_map(|candidate| {
            let Command::Binary(binary) = candidate.command() else {
                return None;
            };
            matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll).then_some(candidate.span())
        })
        .filter(|span| {
            let before_command = &source[span.start.offset..command_span.start.offset];
            let before_command = before_command.trim_end();
            before_command.ends_with('|') && !before_command.ends_with("||")
        })
        .min_by_key(|span| span.end.offset - span.start.offset)
        .unwrap_or(command_span);

    if pipeline_span != command_span {
        return pipeline_span;
    }

    command_span
}

fn zsh_reply_helper_reset_span(command: &CommandFact<'_>) -> Option<Span> {
    let name = command.effective_or_literal_name()?;
    if !zsh_helper_name_can_set_reply(name) {
        return None;
    }

    Some(command.body_name_word()?.span)
}

fn zsh_helper_name_can_set_reply(name: &str) -> bool {
    (name.starts_with('.') && name != "." && name != "..") || name.starts_with('_')
}

fn helper_output_names_by_scope(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
) -> FxHashMap<ScopeId, Vec<Name>> {
    let mut names_by_scope = FxHashMap::<ScopeId, Vec<Name>>::default();

    for binding in semantic.bindings() {
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

fn helper_binding_can_reset_parent_scope(
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
        BindingKind::Declaration(_) => {
            binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::LoopVariable
        | BindingKind::FunctionDefinition
        | BindingKind::Nameref
        | BindingKind::Imported => false,
    }
}

fn binding_command_is_unconditional_in_function(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    binding: &Binding,
) -> bool {
    let Some(current) = semantic.innermost_command_id_at(binding.span.start.offset) else {
        return true;
    };

    command_is_unconditional_in_function(semantic, semantic_analysis, current)
}

fn command_is_unconditional_in_function(
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

fn command_has_reachable_cfg_block(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    command_id: CommandId,
) -> bool {
    semantic_analysis
        .block_ids_for_span(semantic.command_syntax_span(command_id))
        .iter()
        .any(|block| !semantic_analysis.block_is_unreachable(*block))
}

fn zsh_set_a_outparam_positions_by_scope(
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

fn zsh_set_a_outparam_positions(args: &[&Word], source: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut saw_array_flag = false;

    for word in args {
        let text = word.span.slice(source);
        if !saw_array_flag {
            if static_word_text(word, source).is_some_and(|text| text == "-A" || text == "-AArray")
            {
                saw_array_flag = true;
            }
            continue;
        }

        if let Some(position) = positional_outparam_index(text) {
            positions.push(position);
        }
        break;
    }

    positions
}

fn positional_outparam_index(text: &str) -> Option<usize> {
    let parameter = text
        .strip_prefix("${")
        .and_then(|inner| inner.strip_suffix('}'))
        .or_else(|| text.strip_prefix('$'))?;
    let index = parameter.parse::<usize>().ok()?;
    (index > 0).then_some(index)
}

fn resolved_function_scope_for_command(
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
                    .find_map(|(site, binding)| (site.name_span == name_span).then_some(binding))
            })?
        })?;
    let scope = semantic_analysis.function_scope_for_binding(binding)?;
    Some((scope, name_span))
}

fn command_is_inside_function_body(semantic: &SemanticModel, scope: ScopeId) -> bool {
    semantic
        .ancestor_scopes(scope)
        .any(|scope| matches!(semantic.scope_kind(scope), ScopeKind::Function(_)))
}

fn command_runs_in_persistent_shell_context(
    semantic: &SemanticModel,
    command: &CommandFact<'_>,
    source: &str,
) -> bool {
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

fn zsh_pipeline_tail_runs_in_current_shell(
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
        .is_some_and(|options| *options == ZshOptionState::for_emulate(shuck_semantic::ZshEmulationMode::Sh))
    {
        return false;
    }

    command_is_pipeline_tail_operand(semantic, command.id(), source)
        || command_is_preceded_by_pipeline_operator(command.span(), source)
}

fn command_is_pipeline_tail_operand(
    semantic: &SemanticModel,
    mut current: CommandId,
    source: &str,
) -> bool {
    let mut saw_pipeline_parent = false;
    while let Some(parent) = semantic.syntax_backed_command_parent_id(current) {
        if !matches!(semantic.command_kind(parent), CommandKind::Binary) {
            break;
        }
        if !binary_child_is_pipe_tail_operand(semantic, parent, current, source) {
            return false;
        }
        saw_pipeline_parent = true;
        current = parent;
    }
    saw_pipeline_parent
}

fn binary_child_is_pipe_tail_operand(
    semantic: &SemanticModel,
    parent: CommandId,
    child: CommandId,
    source: &str,
) -> bool {
    let parent_span = semantic.command_syntax_span(parent);
    let child_span = semantic.command_syntax_span(child);
    if child_span.start == parent_span.start {
        return false;
    }

    let before_child = &source[parent_span.start.offset..child_span.start.offset];
    let before_child = before_child.trim_end();
    before_child.ends_with('|') && !before_child.ends_with("||")
}

fn command_is_preceded_by_pipeline_operator(command_span: Span, source: &str) -> bool {
    let before_command = &source[..command_span.start.offset];
    let before_command = before_command.trim_end();
    before_command.ends_with('|') && !before_command.ends_with("||")
}

fn reset_site_control_ancestors_contain_later_use(
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

fn reset_site_is_always_run_binary_operand(
    semantic: &SemanticModel,
    parent: CommandId,
    child: CommandId,
) -> bool {
    if !matches!(semantic.command_kind(parent), CommandKind::Binary) {
        return false;
    }

    semantic.command_syntax_span(parent).start == semantic.command_syntax_span(child).start
}

fn command_kind_may_skip_child(kind: CommandKind) -> bool {
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

fn build_nonpersistent_assignment_command_contexts(
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

struct PromptRuntimeRead {
    name: Name,
    span: Span,
    scope: ScopeId,
}

fn build_prompt_runtime_read_spans(
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

fn collect_prompt_runtime_reads_from_assignment(
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

fn escaped_braced_parameter_names(text: &str) -> Vec<String> {
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
fn build_innermost_command_ids_by_offset(
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
struct CommandOffsetLookup {
    entries: Vec<CommandOffsetLookupEntry>,
}

#[derive(Debug, Clone, Copy)]
struct CommandOffsetLookupEntry {
    offset: usize,
    id: CommandId,
}

fn precomputed_command_id_for_offset(
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
struct OpenCommand {
    end_offset: usize,
    id: CommandId,
}

fn pop_finished_commands(active_commands: &mut Vec<OpenCommand>, offset: usize) {
    while active_commands
        .last()
        .is_some_and(|command| command.end_offset < offset)
    {
        active_commands.pop();
    }
}

fn build_dollar_question_after_command_spans(commands: &StmtSeq, source: &str) -> Vec<Span> {
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


fn build_declaration_assignment_probes<'a>(
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
                    has_command_substitution: word_has_command_substitution(
                        word,
                        source,
                        behavior,
                    ),
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

fn declaration_kind_from_semantic(builtin: DeclarationBuiltin) -> DeclarationKind {
    match builtin {
        DeclarationBuiltin::Export => DeclarationKind::Export,
        DeclarationBuiltin::Local => DeclarationKind::Local,
        DeclarationBuiltin::Declare => DeclarationKind::Declare,
        DeclarationBuiltin::Typeset => DeclarationKind::Typeset,
        DeclarationBuiltin::Readonly => DeclarationKind::Other("readonly".to_owned()),
    }
}

fn semantic_declaration_readonly_flag(declaration: &Declaration) -> bool {
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

fn word_for_declaration_value_span(command: &Command, span: Span) -> Option<&Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    command
        .args
        .iter()
        .find(|word| span.start.offset >= word.span.start.offset && span.end.offset <= word.span.end.offset)
}

fn word_span_is_standalone_status_capture(word: &Word, span: Span) -> bool {
    let parts = word_parts_in_span(word, span);
    matches!(parts.as_slice(), [part] if part_is_standalone_status_capture(&part.kind))
}

fn word_span_is_standalone_status_or_pid_capture(word: &Word, span: Span) -> bool {
    let parts = word_parts_in_span(word, span);
    matches!(parts.as_slice(), [part] if part_is_standalone_status_or_pid_capture(&part.kind))
}

fn word_parts_in_span(word: &Word, span: Span) -> Vec<&WordPartNode> {
    word.parts
        .iter()
        .filter(|part| {
            span.start.offset <= part.span.start.offset && part.span.end.offset <= span.end.offset
        })
        .collect()
}

fn part_is_standalone_status_capture(part: &WordPart) -> bool {
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

fn word_is_standalone_status_or_pid_capture(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [part] if part_is_standalone_status_or_pid_capture(&part.kind))
}

fn part_is_standalone_status_or_pid_capture(part: &WordPart) -> bool {
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

fn word_has_command_substitution(
    word: &Word,
    source: &str,
    behavior: &ShellBehaviorAt<'_>,
) -> bool {
    word_classification_from_analysis(analyze_word(word, source, Some(behavior)))
        .has_command_substitution()
}

fn word_is_zsh_selectorless_subscript_value(word: &Word, source: &str) -> bool {
    let mut saw_selectorless_subscript = false;
    word_part_nodes_are_zsh_selectorless_subscript_value(
        &word.parts,
        source,
        &mut saw_selectorless_subscript,
    ) && saw_selectorless_subscript
}

fn word_part_nodes_are_zsh_selectorless_subscript_value(
    parts: &[WordPartNode],
    source: &str,
    saw_selectorless_subscript: &mut bool,
) -> bool {
    for (index, part) in parts.iter().enumerate() {
        if let WordPart::Variable(_) = &part.kind
            && parts.get(index + 1).is_some_and(|next| {
                matches!(
                    &next.kind,
                    WordPart::Literal(text) if literal_starts_with_zsh_subscript(text.as_str(source, next.span))
                )
            })
        {
            *saw_selectorless_subscript = true;
            continue;
        }
        if !word_part_is_zsh_selectorless_subscript_value(
            &part.kind,
            source,
            saw_selectorless_subscript,
        ) {
            return false;
        }
    }
    true
}

fn word_part_is_zsh_selectorless_subscript_value(
    part: &WordPart,
    source: &str,
    saw_selectorless_subscript: &mut bool,
) -> bool {
    match part {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => {
            word_part_nodes_are_zsh_selectorless_subscript_value(
                parts,
                source,
                saw_selectorless_subscript,
            )
        }
        WordPart::Parameter(parameter) => {
            parameter_is_zsh_selectorless_subscript_value(parameter, saw_selectorless_subscript)
        }
        WordPart::ArrayAccess(reference) | WordPart::ArraySlice { reference, .. } => {
            if !var_ref_has_selectorless_subscript(reference) {
                return false;
            }
            *saw_selectorless_subscript = true;
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

fn literal_starts_with_zsh_subscript(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('[') else {
        return false;
    };
    rest.contains(']')
}

fn parameter_is_zsh_selectorless_subscript_value(
    parameter: &ParameterExpansion,
    saw_selectorless_subscript: &mut bool,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
            if !var_ref_has_selectorless_subscript(reference) {
                return false;
            }
            *saw_selectorless_subscript = true;
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
                    *saw_selectorless_subscript = true;
                    true
                }
                ZshExpansionTarget::Nested(parameter) => {
                    parameter_is_zsh_selectorless_subscript_value(
                        parameter,
                        saw_selectorless_subscript,
                    )
                }
                ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => false,
            }
        }
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn var_ref_has_selectorless_subscript(reference: &VarRef) -> bool {
    reference
        .subscript
        .as_deref()
        .is_some_and(|subscript| subscript.selector().is_none())
}

fn advance_escaped_char_boundary(text: &str, start: usize) -> usize {
    let next = start + '\\'.len_utf8();
    if next >= text.len() {
        return next;
    }

    next + text[next..].chars().next().map_or(0, char::len_utf8)
}

fn collect_binding_values<'a>(
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

fn binding_value_definition_id_for_span(
    semantic: &SemanticModel,
    span: Span,
) -> Option<BindingId> {
    semantic.binding_for_definition_span(span)
}

fn binding_value_visible_id_for_name(
    semantic: &SemanticModel,
    name: &Name,
    span: Span,
) -> Option<BindingId> {
    semantic
        .visible_binding(name, span)
        .map(|binding| binding.id)
}

fn annotate_conditional_assignment_value_paths<'a>(
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

fn list_has_conditional_assignment_shortcuts(list: &ListFact<'_>) -> bool {
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
