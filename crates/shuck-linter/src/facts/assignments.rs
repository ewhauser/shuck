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
}

#[derive(Debug, Clone)]
enum BindingValueKind<'a> {
    Scalar(&'a Word),
    Loop(Box<[&'a Word]>),
}

impl<'a> BindingValueFact<'a> {
    fn scalar(word: &'a Word) -> Self {
        Self::scalar_with_status_or_pid_capture(word, word_is_standalone_status_or_pid_capture(word))
    }

    fn scalar_with_status_or_pid_capture(
        word: &'a Word,
        standalone_status_or_pid_capture: bool,
    ) -> Self {
        Self {
            kind: BindingValueKind::Scalar(word),
            standalone_status_or_pid_capture,
            conditional_assignment_shortcut: false,
            one_sided_short_circuit_assignment: false,
        }
    }

    fn from_loop_words(words: Box<[&'a Word]>) -> Self {
        Self {
            kind: BindingValueKind::Loop(words),
            standalone_status_or_pid_capture: false,
            conditional_assignment_shortcut: false,
            one_sided_short_circuit_assignment: false,
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

    fn mark_conditional_assignment_shortcut(&mut self) {
        self.conditional_assignment_shortcut = true;
    }

    fn mark_one_sided_short_circuit_assignment(&mut self) {
        self.one_sided_short_circuit_assignment = true;
    }
}

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

fn build_assignment_like_command_name_spans<'a>(
    commands: &[CommandFact<'a>],
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_assignment_like_command_name_spans_in_command(fact.command(), source, &mut spans);
    }

    spans
}

fn collect_assignment_like_command_name_spans_in_command(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            collect_assignment_like_command_name_span(&command.name, source, spans);
        }
        Command::Decl(command) => {
            for operand in &command.operands {
                if let DeclOperand::Dynamic(word) = operand {
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
    if !is_bare_command_name_assignment_value(&text) {
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

fn is_bare_command_name_assignment_value(text: &str) -> bool {
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

#[derive(Debug, Default)]
struct EnvPrefixScopeSpans {
    assignment_scope_spans: Vec<Span>,
    expansion_scope_spans: Vec<Span>,
}

fn build_env_prefix_scope_spans(source: &str, commands: &[CommandFact<'_>]) -> EnvPrefixScopeSpans {
    let mut scope_spans = EnvPrefixScopeSpans::default();
    let mut seen_assignment_scope_spans = FxHashSet::default();
    let mut seen_expansion_scope_spans = FxHashSet::default();

    for command in commands {
        if command_is_assignment_only(command, source) {
            continue;
        }

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
                assignment_mentions_name_outside_nested_commands(other, &assignment.target.name)
            });
            let later_prefix_uses_name =
                assignments
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .any(|(other_index, other)| {
                        assignment_mentions_name_outside_nested_commands(
                            other,
                            &assignment.target.name,
                        ) || match (command.command(), broken_legacy_bracket_tail) {
                            (Command::Simple(simple), Some(tail))
                                if tail.assignment_index == other_index =>
                            {
                                broken_legacy_bracket_tail_mentions_name(
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

fn assignment_mentions_name_outside_nested_commands(assignment: &Assignment, name: &Name) -> bool {
    visit_assignment_reference_spans_outside_nested_commands(assignment, name, &mut |_span| {
        ControlFlow::Break(())
    })
    .is_break()
}

fn command_body_mentions_name_outside_nested_commands(
    fact: &CommandFact<'_>,
    source: &str,
    name: &Name,
) -> bool {
    visit_command_body_reference_spans_outside_nested_commands(fact, source, name, &mut |_span| {
        ControlFlow::Break(())
    })
    .is_break()
}

fn simple_command_body_words<'a>(
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
    command: &SimpleCommand,
    tail: BrokenLegacyBracketTail,
    name: &Name,
) -> bool {
    visit_broken_legacy_bracket_tail_reference_spans(command, tail, name, &mut |_span| {
        ControlFlow::Break(())
    })
    .is_break()
}

fn visit_assignment_reference_spans_outside_nested_commands(
    assignment: &Assignment,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    visit_subscript_reference_spans_outside_nested_commands(
        assignment.target.subscript.as_deref(),
        name,
        visit,
    )?;

    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            visit_word_reference_spans_outside_nested_commands(word, name, visit)
        }
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        visit_subscript_reference_spans_outside_nested_commands(
                            Some(key),
                            name,
                            visit,
                        )?;
                        visit_word_reference_spans_outside_nested_commands(value, name, visit)?;
                    }
                }
            }

            ControlFlow::Continue(())
        }
    }
}

fn visit_command_body_reference_spans_outside_nested_commands(
    fact: &CommandFact<'_>,
    source: &str,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match fact.command() {
        Command::Simple(command) => {
            for word in simple_command_body_words(command, source) {
                visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
            }
        }
        Command::Builtin(command) => {
            for word in builtin_words(command) {
                visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
            }
        }
        Command::Decl(command) => {
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
                    }
                    DeclOperand::Assignment(assignment) => {
                        visit_assignment_reference_spans_outside_nested_commands(
                            assignment, name, visit,
                        )?;
                    }
                    DeclOperand::Name(_) => {}
                }
            }
        }
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }

    for word in fact.redirects().iter().filter_map(Redirect::word_target) {
        visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
    }

    ControlFlow::Continue(())
}

fn visit_broken_legacy_bracket_tail_reference_spans(
    command: &SimpleCommand,
    tail: BrokenLegacyBracketTail,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    for word in std::iter::once(&command.name)
        .chain(command.args.iter())
        .take(tail.synthetic_word_count.saturating_sub(1))
    {
        visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
    }

    ControlFlow::Continue(())
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

fn visit_subscript_reference_spans_outside_nested_commands(
    subscript: Option<&Subscript>,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    let Some(subscript) = subscript else {
        return ControlFlow::Continue(());
    };

    if let Some(word) = subscript.word_ast() {
        visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
    }
    if let Some(expr) = subscript.arithmetic_ast.as_ref() {
        visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
    }

    ControlFlow::Continue(())
}

fn visit_word_reference_spans_outside_nested_commands(
    word: &Word,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    for part in &word.parts {
        visit_word_part_reference_spans_outside_nested_commands(part, name, visit)?;
    }

    ControlFlow::Continue(())
}

fn visit_word_part_reference_spans_outside_nested_commands(
    part: &WordPartNode,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match &part.kind {
        WordPart::Literal(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::PrefixMatch { .. } => {}
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                visit_word_part_reference_spans_outside_nested_commands(part, name, visit)?;
            }
        }
        WordPart::Variable(variable) => {
            if variable == name {
                visit(part.span)?;
            }
        }
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
        WordPart::ArithmeticExpansion {
            expression_ast,
            expression_word_ast,
            ..
        } => {
            if let Some(expr) = expression_ast.as_ref() {
                visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
            }
            visit_word_reference_spans_outside_nested_commands(expression_word_ast, name, visit)?;
        }
        WordPart::Parameter(parameter) => {
            visit_parameter_reference_spans_outside_nested_commands(
                parameter, part.span, name, visit,
            )?;
        }
        WordPart::ParameterExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            visit_var_ref_reference_spans_outside_nested_commands(
                reference, part.span, name, visit,
            )?;
            if let Some(word) = operand_word_ast.as_ref() {
                visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
            }
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            visit_var_ref_reference_spans_outside_nested_commands(
                reference, part.span, name, visit,
            )?;
        }
        WordPart::Substring {
            reference,
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        } => {
            visit_var_ref_reference_spans_outside_nested_commands(
                reference, part.span, name, visit,
            )?;
            if let Some(expr) = offset_ast.as_ref() {
                visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
            }
            visit_word_reference_spans_outside_nested_commands(offset_word_ast, name, visit)?;
            if let Some(expr) = length_ast.as_ref() {
                visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
            }
            if let Some(word) = length_word_ast.as_ref() {
                visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
            }
        }
        WordPart::IndirectExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            visit_var_ref_reference_spans_outside_nested_commands(
                reference, part.span, name, visit,
            )?;
            if let Some(word) = operand_word_ast.as_ref() {
                visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
            }
        }
    }

    ControlFlow::Continue(())
}

fn visit_parameter_reference_spans_outside_nested_commands(
    parameter: &ParameterExpansion,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                visit_var_ref_reference_spans_outside_nested_commands(
                    reference, span, name, visit,
                )?;
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansion::Operation {
                reference,
                operand_word_ast,
                ..
            } => {
                visit_var_ref_reference_spans_outside_nested_commands(
                    reference, span, name, visit,
                )?;
                if let Some(word) = operand_word_ast.as_ref() {
                    visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                visit_var_ref_reference_spans_outside_nested_commands(
                    reference, span, name, visit,
                )?;
                if let Some(expr) = offset_ast.as_ref() {
                    visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
                }
                if let Some(expr) = length_ast.as_ref() {
                    visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            visit_zsh_target_reference_spans_outside_nested_commands(
                &syntax.target,
                span,
                name,
                visit,
            )?;
        }
    }

    ControlFlow::Continue(())
}

fn visit_zsh_target_reference_spans_outside_nested_commands(
    target: &ZshExpansionTarget,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match target {
        ZshExpansionTarget::Reference(reference) => {
            visit_var_ref_reference_spans_outside_nested_commands(reference, span, name, visit)?;
        }
        ZshExpansionTarget::Nested(parameter) => {
            visit_parameter_reference_spans_outside_nested_commands(parameter, span, name, visit)?;
        }
        ZshExpansionTarget::Word(word) => {
            visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
        }
        ZshExpansionTarget::Empty => {}
    }

    ControlFlow::Continue(())
}

fn visit_var_ref_reference_spans_outside_nested_commands(
    reference: &VarRef,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    if reference.name == *name {
        visit(span)?;
    }

    visit_subscript_reference_spans_outside_nested_commands(
        reference.subscript.as_deref(),
        name,
        visit,
    )
}

fn visit_arithmetic_reference_spans_outside_nested_commands(
    expression: &ArithmeticExprNode,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match &expression.kind {
        ArithmeticExpr::Number(_) => {}
        ArithmeticExpr::Variable(variable) => {
            if variable == name {
                visit(expression.span)?;
            }
        }
        ArithmeticExpr::Indexed {
            name: variable,
            index,
        } => {
            if variable == name {
                visit(expression.span)?;
            }
            visit_arithmetic_reference_spans_outside_nested_commands(index, name, visit)?;
        }
        ArithmeticExpr::ShellWord(word) => {
            visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
        }
        ArithmeticExpr::Parenthesized { expression } => {
            visit_arithmetic_reference_spans_outside_nested_commands(expression, name, visit)?;
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visit_arithmetic_reference_spans_outside_nested_commands(expr, name, visit)?;
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visit_arithmetic_reference_spans_outside_nested_commands(left, name, visit)?;
            visit_arithmetic_reference_spans_outside_nested_commands(right, name, visit)?;
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arithmetic_reference_spans_outside_nested_commands(condition, name, visit)?;
            visit_arithmetic_reference_spans_outside_nested_commands(then_expr, name, visit)?;
            visit_arithmetic_reference_spans_outside_nested_commands(else_expr, name, visit)?;
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue_reference_spans_outside_nested_commands(
                target,
                expression.span,
                name,
                visit,
            )?;
            visit_arithmetic_reference_spans_outside_nested_commands(value, name, visit)?;
        }
    }

    ControlFlow::Continue(())
}

fn visit_arithmetic_lvalue_reference_spans_outside_nested_commands(
    target: &ArithmeticLvalue,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match target {
        ArithmeticLvalue::Variable(variable) => {
            if variable == name {
                visit(span)?;
            }
        }
        ArithmeticLvalue::Indexed {
            name: variable,
            index,
        } => {
            if variable == name {
                visit(span)?;
            }
            visit_arithmetic_reference_spans_outside_nested_commands(index, name, visit)?;
        }
    }

    ControlFlow::Continue(())
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

fn build_nonpersistent_assignment_spans(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    source: &str,
    suppress_bash_pipefail_pipeline_side_effects: bool,
    require_source_ordered_command_lookup: bool,
) -> NonpersistentAssignmentSpans {
    let scope_spans_by_id = semantic
        .scopes()
        .iter()
        .map(|scope| (scope.id, scope.span))
        .collect::<FxHashMap<_, _>>();
    let mut candidate_bindings_by_scope: FxHashMap<
        (Name, usize, usize),
        CandidateSubshellAssignment,
    > = FxHashMap::default();
    let mut persistent_reset_offsets_by_name: FxHashMap<Name, Vec<usize>> = FxHashMap::default();
    let mut command_id_query_offsets = Vec::new();
    let mut relevant_references = Vec::new();
    let mut relevant_synthetic_reads = Vec::new();
    let loop_assignment_spans = build_subshell_loop_assignment_report_spans(commands);

    for binding in semantic.bindings() {
        if !is_reportable_subshell_assignment(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }

        let Some(nonpersistent_scope) = nonpersistent_scope_span_for_assignment(
            semantic,
            binding.scope,
            &scope_spans_by_id,
            suppress_bash_pipefail_pipeline_side_effects,
        ) else {
            continue;
        };

        candidate_bindings_by_scope
            .entry((
                binding.name.clone(),
                nonpersistent_scope.span.start.offset,
                nonpersistent_scope.span.end.offset,
            ))
            .or_insert(CandidateSubshellAssignment {
                binding_id: binding.id,
                effective_local: binding_effectively_targets_local(semantic, binding),
                enclosing_function_scope: enclosing_function_scope_for_scope(semantic, binding.scope),
                assignment_span: subshell_assignment_report_span(binding, &loop_assignment_spans),
                subshell_start: nonpersistent_scope.span.start.offset,
                subshell_end: nonpersistent_scope.span.end.offset,
            });
    }

    let mut candidate_bindings_by_name: FxHashMap<Name, Vec<CandidateSubshellAssignment>> =
        FxHashMap::default();
    for ((name, _, _), candidate) in candidate_bindings_by_scope {
        candidate_bindings_by_name
            .entry(name)
            .or_default()
            .push(candidate);
    }
    for candidates in candidate_bindings_by_name.values_mut() {
        candidates.sort_by_key(|candidate| {
            (
                candidate.subshell_end,
                candidate.assignment_span.start.offset,
                candidate.assignment_span.end.offset,
            )
        });
    }

    for binding in semantic.bindings() {
        if !is_persistent_subshell_reset_binding(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }
        persistent_reset_offsets_by_name
            .entry(binding.name.clone())
            .or_default()
            .push(binding.span.start.offset);
        command_id_query_offsets.push(binding.span.start.offset);
    }

    for reference in semantic.references() {
        if matches!(
            reference.kind,
            shuck_semantic::ReferenceKind::DeclarationName
        ) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&reference.name) {
            continue;
        }
        if candidate_bindings_by_name.contains_key(&reference.name) {
            command_id_query_offsets.push(reference.span.start.offset);
            relevant_references.push(reference);
        }
    }

    for synthetic_read in semantic.synthetic_reads() {
        if !is_reportable_nonpersistent_assignment_name(synthetic_read.name()) {
            continue;
        }
        if candidate_bindings_by_name.contains_key(synthetic_read.name()) {
            command_id_query_offsets.push(synthetic_read.span().start.offset);
            relevant_synthetic_reads.push(synthetic_read);
        }
    }
    let prompt_runtime_reads = build_prompt_runtime_read_spans(commands, source);
    for read in &prompt_runtime_reads {
        if candidate_bindings_by_name.contains_key(&read.name) {
            command_id_query_offsets.push(read.span.start.offset);
        }
    }

    let innermost_command_ids_by_offset = build_innermost_command_ids_by_offset(
        commands,
        command_id_query_offsets,
        require_source_ordered_command_lookup,
    );
    let persistent_reset_offsets_by_name: FxHashMap<Name, Vec<PersistentReset>> =
        persistent_reset_offsets_by_name
            .into_iter()
            .map(|(name, offsets)| {
                let resets = offsets
                    .into_iter()
                    .map(|offset| {
                        let command_id = precomputed_command_id_for_offset(
                            &innermost_command_ids_by_offset,
                            offset,
                        );
                        let command_end_offset = command_id
                            .and_then(|id| commands.get(id.index()))
                            .map(CommandFact::span)
                            .map(|span| span.end.offset)
                            .unwrap_or(offset);

                        PersistentReset {
                            offset,
                            command_id,
                            command_end_offset,
                        }
                    })
                    .collect();
                (name, resets)
            })
            .collect();

    let mut later_use_sites = Vec::new();
    let mut assignment_sites = Vec::new();
    for reference in relevant_references {
        let Some(candidate_ids) = candidate_bindings_by_name.get(&reference.name) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(&reference.name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let event_command_id = precomputed_command_id_for_offset(
            &innermost_command_ids_by_offset,
            reference.span.start.offset,
        );
        let resolved = semantic.resolved_binding(reference.id);
        let reference_function_scope = enclosing_function_scope_for_scope(semantic, reference.scope);
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            reference.span.start.offset > candidate.subshell_end
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    reference.span.start.offset,
                    event_command_id,
                )
                && resolved_binding_allows_subshell_later_use(
                    resolved,
                    candidate,
                    reference.span.start.offset,
                    reference_function_scope,
                )
        }) {
            assignment_sites.push(NamedSpan {
                name: reference.name.clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: reference.name.clone(),
                span: reference.span,
            });
        }
    }

    for synthetic_read in relevant_synthetic_reads {
        let Some(candidate_ids) = candidate_bindings_by_name.get(synthetic_read.name()) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(synthetic_read.name())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let synthetic_command_id = precomputed_command_id_for_offset(
            &innermost_command_ids_by_offset,
            synthetic_read.span().start.offset,
        );
        let same_command_prefix_reset = synthetic_command_id
            .and_then(|id| commands.get(id.index()))
            .is_some_and(|command| {
                command_prefix_assignments_reset_name(command.command(), synthetic_read.name())
            });
        let synthetic_command_end_offset = synthetic_command_id
            .and_then(|id| commands.get(id.index()))
            .map(CommandFact::span)
            .map(|span| span.end.offset)
            .unwrap_or(synthetic_read.span().start.offset);
        let synthetic_function_scope =
            enclosing_function_scope_for_scope(semantic, synthetic_read.scope());
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            synthetic_read.span().start.offset > candidate.subshell_end
                && !same_command_prefix_reset
                && candidate_allows_unresolved_later_use(candidate, synthetic_function_scope)
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    synthetic_command_end_offset,
                    None,
                )
        }) {
            assignment_sites.push(NamedSpan {
                name: synthetic_read.name().clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: synthetic_read.name().clone(),
                span: synthetic_read.span(),
            });
        }
    }

    for read in prompt_runtime_reads {
        let Some(candidate_ids) = candidate_bindings_by_name.get(&read.name) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(&read.name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let event_command_id =
            precomputed_command_id_for_offset(&innermost_command_ids_by_offset, read.span.start.offset);
        let read_function_scope = enclosing_function_scope_for_scope(semantic, read.scope);
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            read.span.start.offset > candidate.subshell_end
                && candidate_allows_unresolved_later_use(candidate, read_function_scope)
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    read.span.start.offset,
                    event_command_id,
                )
        }) {
            assignment_sites.push(NamedSpan {
                name: read.name.clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: read.name,
                span: read.span,
            });
        }
    }

    for binding in semantic.bindings() {
        if !is_reportable_subshell_later_use_binding(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }

        let Some(candidate_ids) = candidate_bindings_by_name.get(&binding.name) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(&binding.name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let binding_function_scope = enclosing_function_scope_for_scope(semantic, binding.scope);
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            binding.span.start.offset > candidate.subshell_end
                && candidate_allows_unresolved_later_use(candidate, binding_function_scope)
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    binding.span.start.offset,
                    None,
                )
        }) {
            assignment_sites.push(NamedSpan {
                name: binding.name.clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: binding.name.clone(),
                span: binding.span,
            });
        }
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

fn is_reportable_subshell_assignment(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => !attributes.contains(BindingAttributes::LOCAL),
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::Imported => false,
        BindingKind::FunctionDefinition | BindingKind::Nameref => false,
    }
}

fn is_reportable_subshell_later_use_binding(
    kind: BindingKind,
    attributes: BindingAttributes,
) -> bool {
    match kind {
        BindingKind::AppendAssignment => true,
        BindingKind::ArithmeticAssignment => true,
        BindingKind::Declaration(DeclarationBuiltin::Export) => {
            !attributes.contains(BindingAttributes::LOCAL)
        }
        BindingKind::Declaration(_) => false,
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::FunctionDefinition
        | BindingKind::Imported
        | BindingKind::Nameref => false,
    }
}

fn is_reportable_nonpersistent_assignment_name(name: &Name) -> bool {
    name.as_str() != "IFS"
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

#[derive(Debug, Clone, Copy)]
struct CandidateSubshellAssignment {
    binding_id: shuck_semantic::BindingId,
    effective_local: bool,
    enclosing_function_scope: Option<ScopeId>,
    assignment_span: Span,
    subshell_start: usize,
    subshell_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct NonpersistentScopeSpan {
    span: Span,
}


#[derive(Debug, Default)]
struct NonpersistentAssignmentSpans {
    subshell_assignment_sites: Vec<NamedSpan>,
    subshell_later_use_sites: Vec<NamedSpan>,
}

#[derive(Debug, Clone, Copy)]
struct PersistentReset {
    offset: usize,
    command_id: Option<CommandId>,
    command_end_offset: usize,
}

fn nonpersistent_scope_span_for_assignment(
    semantic: &SemanticModel,
    scope: ScopeId,
    scope_spans_by_id: &FxHashMap<ScopeId, Span>,
    suppress_bash_pipefail_pipeline_side_effects: bool,
) -> Option<NonpersistentScopeSpan> {
    semantic
        .ancestor_scopes(scope)
        .find(|scope_id| match semantic.scope_kind(*scope_id) {
            shuck_semantic::ScopeKind::Pipeline => !suppress_bash_pipefail_pipeline_side_effects,
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution => true,
            shuck_semantic::ScopeKind::Function(_) | shuck_semantic::ScopeKind::File => false,
        })
        .and_then(|scope_id| scope_spans_by_id.get(&scope_id).copied())
        .map(|span| NonpersistentScopeSpan { span })
}

fn is_persistent_subshell_reset_binding(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => !attributes.contains(BindingAttributes::LOCAL),
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::Imported => false,
        BindingKind::FunctionDefinition | BindingKind::Nameref => false,
    }
}

fn resolved_binding_allows_subshell_later_use(
    resolved: Option<&Binding>,
    candidate: &CandidateSubshellAssignment,
    reference_offset: usize,
    reference_function_scope: Option<ScopeId>,
) -> bool {
    let Some(resolved) = resolved else {
        return candidate_allows_unresolved_later_use(candidate, reference_function_scope);
    };
    if resolved.id == candidate.binding_id {
        return false;
    }
    if resolved.span.start.offset > reference_offset {
        return true;
    }
    if resolved.span.start.offset < candidate.subshell_start {
        return true;
    }

    matches!(resolved.kind, BindingKind::Declaration(_))
        && !resolved
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

fn candidate_allows_unresolved_later_use(
    candidate: &CandidateSubshellAssignment,
    later_function_scope: Option<ScopeId>,
) -> bool {
    !candidate.effective_local || later_function_scope == candidate.enclosing_function_scope
}

fn binding_effectively_targets_local(semantic: &SemanticModel, binding: &Binding) -> bool {
    if binding.attributes.contains(BindingAttributes::LOCAL) {
        return true;
    }

    let binding_function_scope = enclosing_function_scope_for_scope(semantic, binding.scope);
    semantic
        .previous_visible_binding(&binding.name, binding.span, Some(binding.span))
        .is_some_and(|previous| {
            previous.attributes.contains(BindingAttributes::LOCAL)
                && enclosing_function_scope_for_scope(semantic, previous.scope)
                    == binding_function_scope
        })
}

fn enclosing_function_scope_for_scope(semantic: &SemanticModel, scope: ScopeId) -> Option<ScopeId> {
    semantic.ancestor_scopes(scope).find(|scope| {
        matches!(
            semantic.scope_kind(*scope),
            shuck_semantic::ScopeKind::Function(_)
        )
    })
}

fn has_intervening_persistent_reset(
    resets: &[PersistentReset],
    candidate_end: usize,
    event_offset: usize,
    event_command_id: Option<CommandId>,
) -> bool {
    resets.iter().any(|reset| {
        let effective_offset = if reset.offset > candidate_end {
            reset.offset
        } else {
            reset.command_end_offset
        };

        effective_offset > candidate_end
            && effective_offset < event_offset
            && event_command_id.is_none_or(|event_id| reset.command_id != Some(event_id))
    })
}

fn command_prefix_assignments_reset_name(command: &Command, name: &Name) -> bool {
    command_assignments(command)
        .iter()
        .any(|assignment| assignment.target.name == *name)
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

fn build_innermost_command_ids_by_offset(
    commands: &[CommandFact<'_>],
    mut offsets: Vec<usize>,
    require_source_order: bool,
) -> CommandOffsetLookup {
    if offsets.is_empty() {
        return CommandOffsetLookup::default();
    }

    offsets.sort_unstable();
    offsets.dedup();

    let command_order = command_offset_order(commands, require_source_order);
    let mut entries = Vec::with_capacity(offsets.len());
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some((span, id)) = command_order.entry(commands, next_command) {
            if span.start.offset > offset {
                break;
            }

            pop_finished_commands(&mut active_commands, span.start.offset);
            active_commands.push(OpenCommand {
                end_offset: span.end.offset,
                id,
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

enum CommandOffsetOrder {
    SourceOrdered,
    Sorted(Vec<CommandId>),
}

impl CommandOffsetOrder {
    fn entry(&self, commands: &[CommandFact<'_>], index: usize) -> Option<(Span, CommandId)> {
        match self {
            Self::SourceOrdered => {
                let command = commands.get(index)?;
                Some((command.span(), command.id()))
            }
            Self::Sorted(order) => {
                let id = order.get(index).copied()?;
                Some(command_offset_entry(commands, id))
            }
        }
    }
}

fn command_offset_order(commands: &[CommandFact<'_>], require_source_order: bool) -> CommandOffsetOrder {
    if !require_source_order {
        return CommandOffsetOrder::SourceOrdered;
    }

    let mut command_order = commands.iter().map(CommandFact::id).collect::<Vec<_>>();
    command_order.sort_unstable_by(|left, right| {
        compare_command_offset_entries(
            command_offset_entry(commands, *left),
            command_offset_entry(commands, *right),
        )
    });
    CommandOffsetOrder::Sorted(command_order)
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

fn compare_command_offset_entries(
    (left_span, left_id): (Span, CommandId),
    (right_span, right_id): (Span, CommandId),
) -> std::cmp::Ordering {
    left_span
        .start
        .offset
        .cmp(&right_span.start.offset)
        .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
        .then_with(|| right_id.index().cmp(&left_id.index()))
}

fn command_offset_entry(commands: &[CommandFact<'_>], id: CommandId) -> (Span, CommandId) {
    (command_fact(commands, id).span(), id)
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
    collect_dollar_question_after_command_spans_in_seq(commands, source, true, &mut spans);

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

fn collect_dollar_question_after_command_spans_in_seq(
    commands: &StmtSeq,
    source: &str,
    mut status_available: bool,
    spans: &mut Vec<Span>,
) {
    for stmt in commands.iter() {
        collect_dollar_question_after_command_spans_in_stmt(stmt, source, status_available, spans);
        status_available = true;
    }
}

fn collect_dollar_question_after_command_spans_in_stmt(
    stmt: &Stmt,
    source: &str,
    status_available: bool,
    spans: &mut Vec<Span>,
) {
    collect_dollar_question_after_command_spans_in_command(
        &stmt.command,
        source,
        status_available,
        spans,
    );
}

fn collect_dollar_question_after_command_spans_in_command(
    command: &Command,
    source: &str,
    status_available: bool,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            if status_available {
                collect_c107_status_spans_in_simple_test(command, source, spans);
            }
        }
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.condition,
                    source,
                    status_available,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.then_branch,
                    source,
                    true,
                    spans,
                );
                for (condition, body) in &command.elif_branches {
                    collect_dollar_question_after_command_spans_in_seq(
                        condition, source, true, spans,
                    );
                    collect_dollar_question_after_command_spans_in_seq(body, source, true, spans);
                }
                if let Some(else_branch) = &command.else_branch {
                    collect_dollar_question_after_command_spans_in_seq(
                        else_branch,
                        source,
                        true,
                        spans,
                    );
                }
            }
            CompoundCommand::For(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Repeat(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Foreach(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::ArithmeticFor(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::While(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.condition,
                    source,
                    status_available,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Until(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.condition,
                    source,
                    status_available,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    collect_dollar_question_after_command_spans_in_seq(
                        &case.body, source, true, spans,
                    );
                }
            }
            CompoundCommand::Select(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                collect_dollar_question_after_command_spans_in_seq(body, source, true, spans);
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    collect_dollar_question_after_command_spans_in_stmt(
                        command,
                        source,
                        status_available,
                        spans,
                    );
                }
            }
            CompoundCommand::Conditional(command) => {
                if status_available {
                    collect_c107_status_spans_in_conditional_expr(
                        &command.expression,
                        source,
                        spans,
                    );
                }
            }
            CompoundCommand::Arithmetic(command) => {
                if status_available {
                    collect_c107_status_spans_in_arithmetic_command(command, source, spans);
                }
            }
            CompoundCommand::Coproc(command) => {
                collect_dollar_question_after_command_spans_in_stmt(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Always(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.always_body,
                    source,
                    true,
                    spans,
                );
            }
        },
        Command::Binary(command) => {
            collect_dollar_question_after_command_spans_in_stmt(
                &command.left,
                source,
                status_available,
                spans,
            );
            collect_dollar_question_after_command_spans_in_stmt(
                &command.right,
                source,
                true,
                spans,
            );
        }
        Command::AnonymousFunction(command) => {
            collect_dollar_question_after_command_spans_in_function_body(
                &command.body,
                source,
                spans,
            );
        }
        Command::Function(command) => {
            collect_dollar_question_after_command_spans_in_function_body(
                &command.body,
                source,
                spans,
            );
        }
        Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_dollar_question_after_command_spans_in_function_body(
    stmt: &Stmt,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(body))
        | Command::Compound(CompoundCommand::Subshell(body)) => {
            collect_dollar_question_after_command_spans_in_seq(body, source, false, spans);
        }
        _ => collect_dollar_question_after_command_spans_in_stmt(stmt, source, false, spans),
    }
}


fn build_declaration_assignment_probes<'a>(
    command: &'a Command,
    normalized: &NormalizedCommand<'a>,
    semantic: &SemanticModel,
    source: &str,
    zsh_options: Option<&ZshOptionState>,
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
                        zsh_options,
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

    let Some(declaration) = semantic_declaration_for_command(semantic, command) else {
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
                value_origin,
                has_command_or_process_substitution,
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
                has_command_substitution: *has_command_or_process_substitution
                    || *value_origin == AssignmentValueOrigin::CommandOrProcessSubstitution,
                status_capture: word_for_declaration_value_span(command, *value_span)
                    .is_some_and(|word| word_span_is_standalone_status_capture(word, *value_span)),
            })
        })
        .collect()
}

fn semantic_declaration_for_command<'a>(
    semantic: &'a SemanticModel,
    command: &Command,
) -> Option<&'a Declaration> {
    let span = command_span(command);
    semantic.declarations().iter().find(|declaration| {
        declaration.span.start.offset == span.start.offset
            && declaration.span.end.offset == span.end.offset
    })
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
    zsh_options: Option<&ZshOptionState>,
) -> bool {
    word_classification_from_analysis(analyze_word(word, source, zsh_options))
        .has_command_substitution()
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
        if let Some(binding_id) = binding_value_definition_id_for_name(
            semantic,
            &assignment.target.name,
            assignment.target.name_span,
        ) {
            binding_values.insert(binding_id, BindingValueFact::scalar(word));
        }
    }

    for operand in declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        if let Some(binding_id) = binding_value_definition_id_for_name(
            semantic,
            &assignment.target.name,
            assignment.target.name_span,
        ) {
            binding_values.insert(binding_id, BindingValueFact::scalar(word));
        }
    }

    if matches!(command, Command::Simple(_))
        && let Some(declaration) = semantic_declaration_for_command(semantic, command)
    {
        for operand in &declaration.operands {
            let SemanticDeclarationOperand::Assignment {
                name,
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
            if let Some(binding_id) =
                binding_value_definition_id_for_name(semantic, name, *name_span)
            {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::scalar_with_status_or_pid_capture(
                        word,
                        standalone_status_or_pid_capture,
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
                if let Some(name) = &target.name
                    && let Some(binding_id) =
                        binding_value_definition_id_for_name(semantic, name, target.span)
                {
                    binding_values.insert(
                        binding_id,
                        BindingValueFact::from_loop_words(values.clone()),
                    );
                }
            }
        }
        Command::Compound(CompoundCommand::Foreach(command)) => {
            if let Some(binding_id) = binding_value_definition_id_for_name(
                semantic,
                &command.variable,
                command.variable_span,
            ) {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::from_loop_words(
                        command.words.iter().collect::<Vec<_>>().into_boxed_slice(),
                    ),
                );
            }
        }
        Command::Compound(CompoundCommand::Select(command)) => {
            if let Some(binding_id) = binding_value_definition_id_for_name(
                semantic,
                &command.variable,
                command.variable_span,
            ) {
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

fn binding_value_definition_id_for_name(
    semantic: &SemanticModel,
    name: &Name,
    span: Span,
) -> Option<BindingId> {
    semantic
        .bindings_for(name)
        .iter()
        .rev()
        .copied()
        .find(|binding_id| semantic.binding(*binding_id).span == span)
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
