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
        Self {
            kind: BindingValueKind::Scalar(word),
            conditional_assignment_shortcut: false,
            one_sided_short_circuit_assignment: false,
        }
    }

    fn from_loop_words(words: Box<[&'a Word]>) -> Self {
        Self {
            kind: BindingValueKind::Loop(words),
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

    fn mark_conditional_assignment_shortcut(&mut self) {
        self.conditional_assignment_shortcut = true;
    }

    fn mark_one_sided_short_circuit_assignment(&mut self) {
        self.one_sided_short_circuit_assignment = true;
    }
}

fn build_bare_command_name_assignment_spans(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|command| bare_command_name_assignment_span(command, arena_file, source))
        .collect()
}

fn build_assignment_like_command_name_spans<'a>(
    commands: &[CommandFact<'a>],
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    for fact in commands {
        let Some(command_id) = fact.arena_command_id() else {
            continue;
        };
        let command = arena_file.store.command(command_id);
        if let Some(simple) = command.simple() {
            collect_assignment_like_arena_command_name_span(simple.name(), source, &mut spans);
        } else if let Some(decl) = command.decl() {
            for operand in decl.operands() {
                if let DeclOperandNode::Dynamic(word_id) = operand {
                    collect_assignment_like_arena_command_name_span(
                        arena_file.store.word(*word_id),
                        source,
                        &mut spans,
                    );
                }
            }
        }
    }

    spans
}

fn collect_assignment_like_arena_command_name_span(
    word: WordView<'_>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(span) = assignment_like_arena_command_name_span(word, source) {
        spans.push(span);
    }
}

fn assignment_like_arena_command_name_span(word: WordView<'_>, source: &str) -> Option<Span> {
    let prefix = arena_leading_literal_word_prefix(word, source);
    let target_end = prefix.find("+=").or_else(|| prefix.find('='))?;
    let target = &prefix[..target_end];
    if target.is_empty() || target.chars().any(char::is_whitespace) {
        return None;
    }

    if let Some(remainder) = target.strip_prefix('+') {
        is_shell_variable_name(remainder).then_some(word.span())
    } else {
        (!is_shell_variable_name(target)).then_some(word.span())
    }
}

fn arena_leading_literal_word_prefix(word: WordView<'_>, source: &str) -> String {
    let mut prefix = String::new();
    collect_arena_leading_literal_word_parts(word.parts(), word.store(), source, &mut prefix);
    prefix
}

fn collect_arena_leading_literal_word_parts(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    prefix: &mut String,
) -> bool {
    for part in parts {
        if !collect_arena_leading_literal_word_part(part, store, source, prefix) {
            return false;
        }
    }
    true
}

fn collect_arena_leading_literal_word_part(
    part: &WordPartArenaNode,
    store: &AstStore,
    source: &str,
    prefix: &mut String,
) -> bool {
    match &part.kind {
        WordPartArena::Literal(text) => {
            prefix.push_str(text.as_str(source, part.span));
            true
        }
        WordPartArena::SingleQuoted { value, .. } => {
            prefix.push_str(value.slice(source));
            true
        }
        WordPartArena::DoubleQuoted { parts, .. } => collect_arena_leading_literal_word_parts(
            store.word_parts(*parts),
            store,
            source,
            prefix,
        ),
        _ => false,
    }
}

fn bare_command_name_assignment_span(
    fact: &CommandFact<'_>,
    arena_file: &ArenaFile,
    source: &str,
) -> Option<Span> {
    let command = arena_file.store.command(fact.arena_command_id()?);
    let assignments = arena_command_assignments(command);
    let [assignment] = assignments else {
        return None;
    };
    let AssignmentValueNode::Scalar(word_id) = assignment.value else {
        return None;
    };
    let value = arena_file.store.word(word_id);
    let [part] = value.parts() else {
        return None;
    };
    let WordPartArena::Literal(text) = &part.kind else {
        return None;
    };
    if !is_bare_command_name_assignment_value(text.as_str(source, part.span)) {
        return None;
    }

    let command_span = trim_trailing_whitespace_span(command.span(), source);
    let stmt_span = trim_trailing_whitespace_span(fact.stmt_span(), source);
    if !command_span.slice(source).chars().any(char::is_whitespace) {
        Some(assignment_node_target_span(assignment))
    } else if stmt_span.end.offset > command_span.end.offset {
        Some(stmt_span)
    } else if command_span.end.offset <= assignment.span.end.offset {
        Some(assignment_node_target_span(assignment))
    } else {
        Some(command_span)
    }
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

fn build_env_prefix_scope_spans(
    source: &str,
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
) -> EnvPrefixScopeSpans {
    let mut scope_spans = EnvPrefixScopeSpans::default();
    let mut seen_assignment_scope_spans = FxHashSet::default();
    let mut seen_expansion_scope_spans = FxHashSet::default();

    for fact in commands {
        let Some(command_id) = fact.arena_command_id() else {
            continue;
        };
        let command = arena_file.store.command(command_id);
        let stmt = fact
            .arena_stmt_id()
            .map(|stmt_id| arena_file.store.stmt(stmt_id));
        if command_is_assignment_only(fact, command, source) {
            continue;
        }

        let assignments = arena_command_assignments(command);
        let broken_legacy_bracket_tail = command
            .simple()
            .and_then(|simple| broken_legacy_bracket_tail(simple, source));

        for (index, assignment) in assignments.iter().enumerate() {
            let span_key = FactSpan::new(assignment.target.name_span);
            let earlier_prefix_uses_name = assignments.iter().take(index).any(|other| {
                assignment_mentions_name_outside_nested_commands(
                    other,
                    &arena_file.store,
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
                            other,
                            &arena_file.store,
                            &assignment.target.name,
                        ) || command.simple().is_some_and(|simple| {
                            broken_legacy_bracket_tail.is_some_and(|tail| {
                                tail.assignment_index == other_index
                                    && broken_legacy_bracket_tail_mentions_name(
                                        simple,
                                        tail,
                                        &assignment.target.name,
                                    )
                            })
                        })
                    });
            let body_uses_name =
                command_body_mentions_name_outside_nested_commands(
                    command,
                    source,
                    &assignment.target.name,
                ) || stmt.is_some_and(|stmt| {
                    stmt_redirects_mention_name_outside_nested_commands(
                        stmt,
                        &assignment.target.name,
                    )
                });

            if (earlier_prefix_uses_name
                || later_prefix_uses_name
                || (body_uses_name
                    && !assignment_is_identity_self_copy(assignment, &arena_file.store)))
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
                    &arena_file.store,
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

                if let (Some(simple), Some(tail)) = (command.simple(), broken_legacy_bracket_tail)
                    && tail.assignment_index == other_index
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
            }

            if assignments.iter().enumerate().any(|(other_index, other)| {
                other_index != index && other.target.name == assignment.target.name
            }) {
                let _ = visit_assignment_reference_spans_outside_nested_commands(
                    assignment,
                    &arena_file.store,
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
            if let Some(stmt) = stmt {
                let _ = visit_stmt_redirect_reference_spans_outside_nested_commands(
                    stmt,
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

fn command_is_assignment_only(
    fact: &CommandFact<'_>,
    command: CommandView<'_>,
    source: &str,
) -> bool {
    if let Some(simple) = command.simple() {
        return simple.name().span().slice(source).is_empty() && simple.arg_ids().is_empty();
    }

    fact.body_word_span().is_none() && fact.effective_or_literal_name().is_none()
}

fn broken_legacy_bracket_tail(
    command: shuck_ast::SimpleCommandView<'_>,
    source: &str,
) -> Option<BrokenLegacyBracketTail> {
    let store = command.name().store();
    let assignment_index = command.assignments().len().checked_sub(1)?;
    if !assignment_is_broken_legacy_bracket_arithmetic(
        &command.assignments()[assignment_index],
        store,
    ) {
        return None;
    }

    let synthetic_word_count = std::iter::once(command.name_id())
        .chain(command.arg_ids().iter().copied())
        .position(|word_id| {
            static_word_text_arena(store.word(word_id), source).as_deref() == Some("]")
        })?
        + 1;

    Some(BrokenLegacyBracketTail {
        assignment_index,
        synthetic_word_count,
    })
}

fn assignment_is_broken_legacy_bracket_arithmetic(
    assignment: &AssignmentNode,
    store: &AstStore,
) -> bool {
    let AssignmentValueNode::Scalar(word_id) = assignment.value else {
        return false;
    };
    word_is_broken_legacy_bracket_arithmetic(store.word(word_id))
}

fn word_is_broken_legacy_bracket_arithmetic(word: WordView<'_>) -> bool {
    let [part] = word.parts() else {
        return false;
    };
    matches!(
        &part.kind,
        WordPartArena::ArithmeticExpansion {
            syntax: ArithmeticExpansionSyntax::LegacyBracket,
            expression_ast: None,
            ..
        }
    )
}

fn assignment_mentions_name_outside_nested_commands(
    assignment: &AssignmentNode,
    store: &AstStore,
    name: &Name,
) -> bool {
    visit_assignment_reference_spans_outside_nested_commands(
        assignment,
        store,
        name,
        &mut |_span| ControlFlow::Break(()),
    )
    .is_break()
}

fn command_body_mentions_name_outside_nested_commands(
    command: CommandView<'_>,
    source: &str,
    name: &Name,
) -> bool {
    visit_command_body_reference_spans_outside_nested_commands(
        command,
        source,
        name,
        &mut |_span| ControlFlow::Break(()),
    )
    .is_break()
}

fn stmt_redirects_mention_name_outside_nested_commands(
    stmt: StmtView<'_>,
    name: &Name,
) -> bool {
    visit_stmt_redirect_reference_spans_outside_nested_commands(
        stmt,
        name,
        &mut |_span| ControlFlow::Break(()),
    )
    .is_break()
}

fn simple_command_body_words<'a>(
    command: &'a SimpleCommand,
    _source: &'a str,
) -> impl Iterator<Item = &'a Word> {
    std::iter::once(&command.name).chain(command.args.iter())
}

fn broken_legacy_bracket_tail_mentions_name(
    command: shuck_ast::SimpleCommandView<'_>,
    tail: BrokenLegacyBracketTail,
    name: &Name,
) -> bool {
    visit_broken_legacy_bracket_tail_reference_spans(command, tail, name, &mut |_span| {
        ControlFlow::Break(())
    })
    .is_break()
}

fn visit_assignment_reference_spans_outside_nested_commands(
    assignment: &AssignmentNode,
    store: &AstStore,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    visit_subscript_reference_spans_outside_nested_commands(
        assignment.target.subscript.as_deref(),
        store,
        name,
        visit,
    )?;

    match &assignment.value {
        AssignmentValueNode::Scalar(word_id) => {
            visit_word_reference_spans_outside_nested_commands(store.word(*word_id), name, visit)
        }
        AssignmentValueNode::Compound(array) => {
            for element in store.array_elems(array.elements) {
                match element {
                    ArrayElemNode::Sequential(value) => {
                        visit_word_reference_spans_outside_nested_commands(
                            store.word(value.word),
                            name,
                            visit,
                        )?;
                    }
                    ArrayElemNode::Keyed { key, value }
                    | ArrayElemNode::KeyedAppend { key, value } => {
                        visit_subscript_reference_spans_outside_nested_commands(
                            Some(key),
                            store,
                            name,
                            visit,
                        )?;
                        visit_word_reference_spans_outside_nested_commands(
                            store.word(value.word),
                            name,
                            visit,
                        )?;
                    }
                }
            }

            ControlFlow::Continue(())
        }
    }
}

fn visit_command_body_reference_spans_outside_nested_commands(
    command: CommandView<'_>,
    source: &str,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    if let Some(simple) = command.simple() {
        let skip =
            broken_legacy_bracket_tail(simple, source).map_or(0, |tail| tail.synthetic_word_count);
        for word_id in std::iter::once(simple.name_id())
            .chain(simple.arg_ids().iter().copied())
            .skip(skip)
        {
            visit_word_reference_spans_outside_nested_commands(
                command.store().word(word_id),
                name,
                visit,
            )?;
        }
    } else if let Some(builtin) = command.builtin() {
        if let Some(word) = builtin.primary() {
            visit_word_reference_spans_outside_nested_commands(word, name, visit)?;
        }
        for word_id in builtin.extra_arg_ids() {
            visit_word_reference_spans_outside_nested_commands(
                command.store().word(*word_id),
                name,
                visit,
            )?;
        }
    } else if let Some(decl) = command.decl() {
        for operand in decl.operands() {
            match operand {
                DeclOperandNode::Flag(word_id) | DeclOperandNode::Dynamic(word_id) => {
                    visit_word_reference_spans_outside_nested_commands(
                        command.store().word(*word_id),
                        name,
                        visit,
                    )?;
                }
                DeclOperandNode::Name(reference) => {
                    visit_var_ref_reference_spans_outside_nested_commands(
                        reference,
                        command.store(),
                        name,
                        visit,
                    )?;
                }
                DeclOperandNode::Assignment(assignment) => {
                    visit_assignment_reference_spans_outside_nested_commands(
                        assignment,
                        command.store(),
                        name,
                        visit,
                    )?;
                }
            }
        }
    }

    ControlFlow::Continue(())
}

fn visit_stmt_redirect_reference_spans_outside_nested_commands(
    stmt: StmtView<'_>,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    for redirect in stmt.redirects() {
        match &redirect.target {
            RedirectTargetNode::Word(word_id) => {
                visit_word_reference_spans_outside_nested_commands(
                    stmt.command().store().word(*word_id),
                    name,
                    visit,
                )?;
            }
            RedirectTargetNode::Heredoc(_) => {}
        }
    }

    ControlFlow::Continue(())
}

fn visit_broken_legacy_bracket_tail_reference_spans(
    command: shuck_ast::SimpleCommandView<'_>,
    tail: BrokenLegacyBracketTail,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    let store = command.name().store();
    for word_id in std::iter::once(command.name_id())
        .chain(command.arg_ids().iter().copied())
        .take(tail.synthetic_word_count.saturating_sub(1))
    {
        visit_word_reference_spans_outside_nested_commands(store.word(word_id), name, visit)?;
    }

    ControlFlow::Continue(())
}

fn assignment_is_identity_self_copy(assignment: &AssignmentNode, store: &AstStore) -> bool {
    if assignment.append {
        return false;
    }

    let AssignmentValueNode::Scalar(word_id) = assignment.value else {
        return false;
    };
    word_is_identity_self_copy(store.word(word_id), &assignment.target.name)
}

fn word_is_identity_self_copy(word: WordView<'_>, name: &Name) -> bool {
    let [part] = word.parts() else {
        return false;
    };
    word_part_is_identity_self_copy(&part.kind, name)
}

fn word_part_is_identity_self_copy(part: &WordPartArena, name: &Name) -> bool {
    match part {
        WordPartArena::Variable(variable) => variable == name,
        WordPartArena::Parameter(parameter) => parameter_is_plain_access_to_name(parameter, name),
        _ => false,
    }
}

fn parameter_is_plain_access_to_name(parameter: &ParameterExpansionNode, name: &Name) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntaxNode::Bourne(BourneParameterExpansionNode::Access {
            reference,
        }) if reference.subscript.is_none() => &reference.name == name,
        ParameterExpansionSyntaxNode::Zsh(syntax)
            if syntax.operation.is_none()
                && matches!(&syntax.target, ZshExpansionTargetNode::Reference(reference) if reference.subscript.is_none() && &reference.name == name) =>
        {
            true
        }
        _ => false,
    }
}

fn visit_subscript_reference_spans_outside_nested_commands(
    subscript: Option<&SubscriptNode>,
    store: &AstStore,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    let Some(subscript) = subscript else {
        return ControlFlow::Continue(());
    };

    if let Some(word_id) = subscript.word_ast {
        visit_word_reference_spans_outside_nested_commands(store.word(word_id), name, visit)?;
    }
    if let Some(expr) = subscript.arithmetic_ast.as_ref() {
        visit_arithmetic_reference_spans_outside_nested_commands(expr, store, name, visit)?;
    }

    ControlFlow::Continue(())
}

fn visit_word_reference_spans_outside_nested_commands(
    word: WordView<'_>,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    for part in word.parts() {
        visit_word_part_reference_spans_outside_nested_commands(part, word.store(), name, visit)?;
    }

    ControlFlow::Continue(())
}

fn visit_word_part_reference_spans_outside_nested_commands(
    part: &WordPartArenaNode,
    store: &AstStore,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match &part.kind {
        WordPartArena::Literal(_)
        | WordPartArena::SingleQuoted { .. }
        | WordPartArena::ZshQualifiedGlob(_)
        | WordPartArena::PrefixMatch { .. } => {}
        WordPartArena::DoubleQuoted { parts, .. } => {
            for part in store.word_parts(*parts) {
                visit_word_part_reference_spans_outside_nested_commands(part, store, name, visit)?;
            }
        }
        WordPartArena::Variable(variable) => {
            if variable == name {
                visit(part.span)?;
            }
        }
        WordPartArena::CommandSubstitution { .. } | WordPartArena::ProcessSubstitution { .. } => {}
        WordPartArena::ArithmeticExpansion {
            expression_ast,
            expression_word_ast,
            ..
        } => {
            if let Some(expr) = expression_ast.as_ref() {
                visit_arithmetic_reference_spans_outside_nested_commands(expr, store, name, visit)?;
            }
            visit_word_reference_spans_outside_nested_commands(
                store.word(*expression_word_ast),
                name,
                visit,
            )?;
        }
        WordPartArena::Parameter(parameter) => {
            visit_parameter_reference_spans_outside_nested_commands(
                parameter, store, part.span, name, visit,
            )?;
        }
        WordPartArena::ParameterExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            visit_var_ref_reference_spans_outside_nested_commands(reference, store, name, visit)?;
            if let Some(word_id) = operand_word_ast {
                visit_word_reference_spans_outside_nested_commands(
                    store.word(*word_id),
                    name,
                    visit,
                )?;
            }
        }
        WordPartArena::Length(reference)
        | WordPartArena::ArrayAccess(reference)
        | WordPartArena::ArrayLength(reference)
        | WordPartArena::ArrayIndices(reference)
        | WordPartArena::Transformation { reference, .. } => {
            visit_var_ref_reference_spans_outside_nested_commands(reference, store, name, visit)?;
        }
        WordPartArena::Substring {
            reference,
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        }
        | WordPartArena::ArraySlice {
            reference,
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        } => {
            visit_var_ref_reference_spans_outside_nested_commands(reference, store, name, visit)?;
            if let Some(expr) = offset_ast.as_ref() {
                visit_arithmetic_reference_spans_outside_nested_commands(expr, store, name, visit)?;
            }
            visit_word_reference_spans_outside_nested_commands(
                store.word(*offset_word_ast),
                name,
                visit,
            )?;
            if let Some(expr) = length_ast.as_ref() {
                visit_arithmetic_reference_spans_outside_nested_commands(expr, store, name, visit)?;
            }
            if let Some(word_id) = length_word_ast {
                visit_word_reference_spans_outside_nested_commands(
                    store.word(*word_id),
                    name,
                    visit,
                )?;
            }
        }
        WordPartArena::IndirectExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            visit_var_ref_reference_spans_outside_nested_commands(reference, store, name, visit)?;
            if let Some(word_id) = operand_word_ast {
                visit_word_reference_spans_outside_nested_commands(
                    store.word(*word_id),
                    name,
                    visit,
                )?;
            }
        }
    }

    ControlFlow::Continue(())
}

fn visit_parameter_reference_spans_outside_nested_commands(
    parameter: &ParameterExpansionNode,
    store: &AstStore,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match &parameter.syntax {
        ParameterExpansionSyntaxNode::Bourne(syntax) => match syntax {
            BourneParameterExpansionNode::Access { reference }
            | BourneParameterExpansionNode::Length { reference }
            | BourneParameterExpansionNode::Indices { reference }
            | BourneParameterExpansionNode::Transformation { reference, .. } => {
                visit_var_ref_reference_spans_outside_nested_commands(
                    reference, store, name, visit,
                )?;
            }
            BourneParameterExpansionNode::Indirect {
                reference,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansionNode::Operation {
                reference,
                operand_word_ast,
                ..
            } => {
                visit_var_ref_reference_spans_outside_nested_commands(
                    reference, store, name, visit,
                )?;
                if let Some(word_id) = operand_word_ast {
                    visit_word_reference_spans_outside_nested_commands(
                        store.word(*word_id),
                        name,
                        visit,
                    )?;
                }
            }
            BourneParameterExpansionNode::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                visit_var_ref_reference_spans_outside_nested_commands(
                    reference, store, name, visit,
                )?;
                if let Some(expr) = offset_ast.as_ref() {
                    visit_arithmetic_reference_spans_outside_nested_commands(
                        expr, store, name, visit,
                    )?;
                }
                visit_word_reference_spans_outside_nested_commands(
                    store.word(*offset_word_ast),
                    name,
                    visit,
                )?;
                if let Some(expr) = length_ast.as_ref() {
                    visit_arithmetic_reference_spans_outside_nested_commands(
                        expr, store, name, visit,
                    )?;
                }
                if let Some(word_id) = length_word_ast {
                    visit_word_reference_spans_outside_nested_commands(
                        store.word(*word_id),
                        name,
                        visit,
                    )?;
                }
            }
            BourneParameterExpansionNode::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntaxNode::Zsh(syntax) => {
            visit_zsh_target_reference_spans_outside_nested_commands(
                &syntax.target,
                store,
                span,
                name,
                visit,
            )?;
        }
    }

    ControlFlow::Continue(())
}

fn visit_zsh_target_reference_spans_outside_nested_commands(
    target: &ZshExpansionTargetNode,
    store: &AstStore,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match target {
        ZshExpansionTargetNode::Reference(reference) => {
            visit_var_ref_reference_spans_outside_nested_commands(reference, store, name, visit)?;
        }
        ZshExpansionTargetNode::Nested(parameter) => {
            visit_parameter_reference_spans_outside_nested_commands(
                parameter, store, span, name, visit,
            )?;
        }
        ZshExpansionTargetNode::Word(word_id) => {
            visit_word_reference_spans_outside_nested_commands(store.word(*word_id), name, visit)?;
        }
        ZshExpansionTargetNode::Empty => {}
    }

    ControlFlow::Continue(())
}

fn visit_var_ref_reference_spans_outside_nested_commands(
    reference: &VarRefNode,
    store: &AstStore,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    if reference.name == *name {
        visit(reference.span)?;
    }

    visit_subscript_reference_spans_outside_nested_commands(
        reference.subscript.as_deref(),
        store,
        name,
        visit,
    )
}

fn visit_arithmetic_reference_spans_outside_nested_commands(
    expression: &ArithmeticExprArenaNode,
    store: &AstStore,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match &expression.kind {
        ArithmeticExprArena::Number(_) => {}
        ArithmeticExprArena::Variable(variable) => {
            if variable == name {
                visit(expression.span)?;
            }
        }
        ArithmeticExprArena::Indexed {
            name: variable,
            index,
        } => {
            if variable == name {
                visit(expression.span)?;
            }
            visit_arithmetic_reference_spans_outside_nested_commands(index, store, name, visit)?;
        }
        ArithmeticExprArena::ShellWord(word_id) => {
            visit_word_reference_spans_outside_nested_commands(store.word(*word_id), name, visit)?;
        }
        ArithmeticExprArena::Parenthesized { expression } => {
            visit_arithmetic_reference_spans_outside_nested_commands(
                expression, store, name, visit,
            )?;
        }
        ArithmeticExprArena::Unary { expr, .. } | ArithmeticExprArena::Postfix { expr, .. } => {
            visit_arithmetic_reference_spans_outside_nested_commands(expr, store, name, visit)?;
        }
        ArithmeticExprArena::Binary { left, right, .. } => {
            visit_arithmetic_reference_spans_outside_nested_commands(left, store, name, visit)?;
            visit_arithmetic_reference_spans_outside_nested_commands(right, store, name, visit)?;
        }
        ArithmeticExprArena::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arithmetic_reference_spans_outside_nested_commands(
                condition, store, name, visit,
            )?;
            visit_arithmetic_reference_spans_outside_nested_commands(
                then_expr, store, name, visit,
            )?;
            visit_arithmetic_reference_spans_outside_nested_commands(
                else_expr, store, name, visit,
            )?;
        }
        ArithmeticExprArena::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue_reference_spans_outside_nested_commands(
                target,
                store,
                expression.span,
                name,
                visit,
            )?;
            visit_arithmetic_reference_spans_outside_nested_commands(value, store, name, visit)?;
        }
    }

    ControlFlow::Continue(())
}

fn visit_arithmetic_lvalue_reference_spans_outside_nested_commands(
    target: &ArithmeticLvalueArena,
    store: &AstStore,
    span: Span,
    name: &Name,
    visit: &mut EnvPrefixReferenceSpanVisitor<'_>,
) -> ControlFlow<()> {
    match target {
        ArithmeticLvalueArena::Variable(variable) => {
            if variable == name {
                visit(span)?;
            }
        }
        ArithmeticLvalueArena::Indexed {
            name: variable,
            index,
        } => {
            if variable == name {
                visit(span)?;
            }
            visit_arithmetic_reference_spans_outside_nested_commands(index, store, name, visit)?;
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

fn build_plus_equals_assignment_spans(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
) -> Vec<Span> {
    let mut spans = Vec::new();
    for fact in commands {
        let Some(command_id) = fact.arena_command_id() else {
            continue;
        };
        let command = arena_file.store.command(command_id);
        for assignment in arena_command_assignments(command) {
            collect_plus_equals_arena_assignment_span(assignment, &mut spans);
        }
        for operand in arena_declaration_operands(command) {
            if let DeclOperandNode::Assignment(assignment) = operand {
                collect_plus_equals_arena_assignment_span(assignment, &mut spans);
            }
        }
    }
    spans
}

fn collect_plus_equals_arena_assignment_span(assignment: &AssignmentNode, spans: &mut Vec<Span>) {
    if !assignment.append {
        return;
    }

    spans.push(assignment_node_target_span(assignment));
}

fn assignment_node_target_span(assignment: &AssignmentNode) -> Span {
    assignment.target.subscript.as_deref().map_or_else(
        || assignment.target.name_span,
        |subscript| {
            let subscript_span = subscript.raw.as_ref().unwrap_or(&subscript.text).span();
            Span::from_positions(
                assignment.target.name_span.start,
                subscript_span.end.advanced_by("]"),
            )
        },
    )
}

fn build_nonpersistent_assignment_spans(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
    suppress_bash_pipefail_pipeline_side_effects: bool,
    _require_source_ordered_command_lookup: bool,
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
    let loop_assignment_spans = build_subshell_loop_assignment_report_spans(commands, arena_file);

    for binding in semantic.bindings() {
        if !is_reportable_subshell_assignment(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }

        let Some(nonpersistent_scope) = nonpersistent_scope_span_for_assignment(
            semantic,
            binding.span.start.offset,
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
                enclosing_function_scope: enclosing_function_scope_for_scope(
                    semantic,
                    binding.scope,
                ),
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
    let prompt_runtime_reads = build_prompt_runtime_read_spans(commands, arena_file, source);
    for read in &prompt_runtime_reads {
        if candidate_bindings_by_name.contains_key(&read.name) {
            command_id_query_offsets.push(read.span.start.offset);
        }
    }

    let innermost_command_ids_by_offset =
        build_innermost_command_ids_by_offset(commands, command_id_query_offsets, false);
    let persistent_reset_offsets_by_name: FxHashMap<Name, Vec<PersistentReset>> =
        persistent_reset_offsets_by_name
            .into_iter()
            .map(|(name, offsets)| {
                let resets = offsets
                    .into_iter()
                    .map(|offset| {
                        let command_id =
                            precomputed_command_id_for_offset(&innermost_command_ids_by_offset, offset)
                                .or_else(|| {
                                    command_id_for_prefix_assignment_offset(commands, offset)
                                });
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
        let reference_function_scope =
            enclosing_function_scope_for_scope(semantic, reference.scope);
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
        let same_command_prefix_reset =
            synthetic_command_id.is_some_and(|synthetic_command_id| {
                reset_offsets.iter().any(|reset| {
                    reset.command_id == Some(synthetic_command_id)
                }) || command_has_assignment_to_name(
                    commands,
                    arena_file,
                    synthetic_command_id,
                    synthetic_read.name(),
                )
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
        let event_command_id = precomputed_command_id_for_offset(
            &innermost_command_ids_by_offset,
            read.span.start.offset,
        );
        let read_function_scope =
            enclosing_function_scope_for_scope(semantic, semantic.scope_at(read.span.start.offset));
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
            later_use_sites.push(read);
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
    arena_file: &ArenaFile,
) -> FxHashMap<FactSpan, Span> {
    let mut spans = FxHashMap::default();

    for fact in commands {
        let Some(command_id) = fact.arena_command_id() else {
            continue;
        };
        let command = arena_file.store.command(command_id);
        let Some(compound) = command.compound() else {
            continue;
        };
        match compound.node() {
            CompoundCommandNode::For { targets, .. } => {
                let keyword_span = leading_keyword_span(command.span(), "for");
                for target in arena_file.store.for_targets(*targets) {
                    if target.name.is_some() {
                        spans.insert(FactSpan::new(target.span), keyword_span);
                    }
                }
            }
            CompoundCommandNode::Select { variable_span, .. } => {
                spans.insert(
                    FactSpan::new(*variable_span),
                    leading_keyword_span(command.span(), "select"),
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
    offset: usize,
    scope_spans_by_id: &FxHashMap<ScopeId, Span>,
    suppress_bash_pipefail_pipeline_side_effects: bool,
) -> Option<NonpersistentScopeSpan> {
    semantic
        .ancestor_scopes(semantic.scope_at(offset))
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

fn build_prompt_runtime_read_spans(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<NamedSpan> {
    let mut reads = Vec::new();
    for fact in commands {
        let Some(command_id) = fact.arena_command_id() else {
            continue;
        };
        let command = arena_file.store.command(command_id);
        for assignment in arena_command_assignments(command) {
            collect_prompt_runtime_reads_from_assignment(
                assignment,
                &arena_file.store,
                source,
                &mut reads,
            );
        }
        for operand in arena_declaration_operands(command) {
            if let DeclOperandNode::Assignment(assignment) = operand {
                collect_prompt_runtime_reads_from_assignment(
                    assignment,
                    &arena_file.store,
                    source,
                    &mut reads,
                );
            }
        }
    }
    reads
}

fn collect_prompt_runtime_reads_from_assignment(
    assignment: &AssignmentNode,
    store: &AstStore,
    source: &str,
    reads: &mut Vec<NamedSpan>,
) {
    if assignment.target.name.as_str() != "PS4" {
        return;
    }
    let AssignmentValueNode::Scalar(word_id) = assignment.value else {
        return;
    };
    let word = store.word(word_id);

    let target_span = assignment_node_target_span(assignment);
    for name in escaped_braced_parameter_names(word.span().slice(source)) {
        reads.push(NamedSpan {
            name: Name::from(name.as_str()),
            span: target_span,
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
    _require_source_order: bool,
) -> CommandOffsetLookup {
    if offsets.is_empty() {
        return CommandOffsetLookup::default();
    }

    offsets.sort_unstable();
    offsets.dedup();

    let mut command_order = commands.iter().map(CommandFact::id).collect::<Vec<_>>();
    if command_order.windows(2).any(|window| {
        compare_command_offset_entries(
            command_offset_entry(commands, window[0]),
            command_offset_entry(commands, window[1]),
        )
        .is_gt()
    }) {
        command_order.sort_unstable_by(|left, right| {
            compare_command_offset_entries(
                command_offset_entry(commands, *left),
                command_offset_entry(commands, *right),
            )
        });
    }

    let mut entries = Vec::with_capacity(offsets.len());
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some(id) = command_order.get(next_command).copied() {
            let span = command_fact(commands, id).span();
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

fn command_id_for_prefix_assignment_offset(
    commands: &[CommandFact<'_>],
    offset: usize,
) -> Option<CommandId> {
    commands
        .iter()
        .filter(|command| {
            let stmt_span = command.stmt_span();
            stmt_span.start.offset <= offset
                && offset <= stmt_span.end.offset
                && command.span().start.offset >= offset
        })
        .min_by_key(|command| {
            let span = command.span();
            let stmt_span = command.stmt_span();
            (
                span.start.offset,
                stmt_span.end.offset.saturating_sub(stmt_span.start.offset),
                command.id().index(),
            )
        })
        .map(CommandFact::id)
}

fn command_has_assignment_to_name(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    command_id: CommandId,
    name: &Name,
) -> bool {
    let Some(command) = commands.get(command_id.index()) else {
        return false;
    };
    let Some(arena_command_id) = command.arena_command_id() else {
        return false;
    };
    arena_command_assignments(arena_file.store.command(arena_command_id))
        .iter()
        .any(|assignment| assignment.target.name.as_str() == name.as_str())
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

    build_simple_command_declaration_assignment_probes(command, normalized, source, zsh_options)
}

fn build_simple_command_declaration_assignment_probes<'a>(
    command: &'a Command,
    normalized: &NormalizedCommand<'a>,
    source: &str,
    zsh_options: Option<&ZshOptionState>,
) -> Vec<DeclarationAssignmentProbe> {
    let Command::Simple(_) = command else {
        return Vec::new();
    };

    if !normalized.wrappers.is_empty() {
        return Vec::new();
    }

    let Some(kind) = simple_command_declaration_kind(normalized.effective_or_literal_name()) else {
        return Vec::new();
    };
    let word_groups = contiguous_word_groups(normalized.body_args());
    let readonly_flag = matches!(
        kind,
        DeclarationKind::Local | DeclarationKind::Declare | DeclarationKind::Typeset
    ) && simple_command_declaration_readonly_flag(&word_groups, source);

    word_groups
        .iter()
        .filter_map(|words| {
            let first = *words.first()?;
            let text = words
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<String>();
            let parsed = parse_assignment_word(&text)?;
            let value_text = &text[parsed.value_offset..];
            Some(DeclarationAssignmentProbe {
                kind: kind.clone(),
                readonly_flag,
                target_name: parsed.name.into(),
                target_name_span: Span::from_positions(
                    first.span.start,
                    first.span.start.advanced_by(parsed.name),
                ),
                has_command_substitution: parsed_assignment_value_has_command_substitution(
                    value_text,
                    zsh_options,
                ),
                status_capture: assignment_value_text_is_standalone_status_capture(value_text),
            })
        })
        .collect()
}

fn assignment_value_text_is_standalone_status_capture(text: &str) -> bool {
    matches!(text, "$?" | "${?}" | "\"$?\"" | "\"${?}\"")
}

fn contiguous_word_groups<'a>(words: &'a [&'a Word]) -> Vec<&'a [&'a Word]> {
    let mut groups = Vec::new();
    let mut start = 0usize;

    while start < words.len() {
        let mut end = start + 1;
        while let Some(next) = words.get(end).copied() {
            if words[end - 1].span.end.offset != next.span.start.offset {
                break;
            }
            end += 1;
        }
        groups.push(&words[start..end]);
        start = end;
    }

    groups
}

fn simple_command_declaration_kind(name: Option<&str>) -> Option<DeclarationKind> {
    match name? {
        "export" => Some(DeclarationKind::Export),
        "local" => Some(DeclarationKind::Local),
        "declare" => Some(DeclarationKind::Declare),
        "typeset" => Some(DeclarationKind::Typeset),
        "readonly" => Some(DeclarationKind::Other("readonly".to_owned())),
        _ => None,
    }
}

fn simple_command_declaration_readonly_flag(word_groups: &[&[&Word]], source: &str) -> bool {
    let mut readonly_flag = false;

    for words in word_groups {
        let [word] = words else {
            break;
        };
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        // Bash stops parsing declaration options after the first name[=value] operand,
        // so later "-r" words must not retroactively mark earlier assignments readonly.
        if text == "--" {
            break;
        }

        if !simple_command_declaration_option_word(&text) {
            break;
        }

        if declaration_flag_sets_readonly_text(&text) {
            readonly_flag = true;
        }
    }

    readonly_flag
}

fn simple_command_declaration_option_word(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(prefix) = chars.next() else {
        return false;
    };

    if !matches!(prefix, '-' | '+') {
        return false;
    }

    let rest = chars.as_str();
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn declaration_flag_sets_readonly_text(text: &str) -> bool {
    text.starts_with('-') && text.contains('r')
}

#[derive(Debug, Clone, Copy)]
struct ParsedAssignmentWord<'a> {
    name: &'a str,
    value_offset: usize,
}

fn parse_assignment_word(word: &str) -> Option<ParsedAssignmentWord<'_>> {
    if !word.contains('=') {
        return None;
    }

    let mut chars = word.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }

    let mut ident_end = first.len_utf8();
    for (index, ch) in chars {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ident_end = index + ch.len_utf8();
        } else {
            break;
        }
    }

    let name = &word[..ident_end];
    let mut cursor = ident_end;

    if word[cursor..].starts_with('[') {
        let bytes = word.as_bytes();
        let mut close_index = None;
        let mut bracket_depth = 0usize;
        let mut index = cursor + 1;

        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = advance_escaped_char_boundary(word, index);
                continue;
            }

            if index + 2 < bytes.len()
                && is_unescaped_dollar(bytes, index)
                && bytes[index + 1] == b'('
                && bytes[index + 2] == b'('
            {
                index = find_wrapped_arithmetic_end(bytes, index)?;
                continue;
            }

            if index + 1 < bytes.len()
                && is_unescaped_dollar(bytes, index)
                && bytes[index + 1] == b'('
            {
                index = find_command_substitution_end(bytes, index)?;
                continue;
            }

            if index + 1 < bytes.len()
                && is_unescaped_dollar(bytes, index)
                && bytes[index + 1] == b'{'
            {
                index = find_runtime_parameter_closing_brace(word, index)?;
                continue;
            }

            if index + 1 < bytes.len()
                && matches!(bytes[index], b'<' | b'>')
                && bytes[index + 1] == b'('
            {
                index = find_process_substitution_end(bytes, index)?;
                continue;
            }

            match bytes[index] {
                b'\'' => index = skip_single_quoted(bytes, index + 1)?,
                b'"' => index = skip_double_quoted(bytes, index + 1)?,
                b'`' => index = skip_backticks(bytes, index + 1)?,
                b'[' => {
                    bracket_depth += 1;
                    index += 1;
                }
                b']' if bracket_depth == 0 => {
                    close_index = Some(index);
                    break;
                }
                b']' => {
                    bracket_depth -= 1;
                    index += 1;
                }
                _ => {
                    index += word[index..].chars().next()?.len_utf8();
                }
            }
        }

        cursor = close_index? + 1;
    }

    if word[cursor..].starts_with("+=") || word[cursor..].starts_with('=') {
        Some(ParsedAssignmentWord {
            name,
            value_offset: cursor
                + if word[cursor..].starts_with("+=") {
                    2
                } else {
                    1
                },
        })
    } else {
        None
    }
}

fn advance_escaped_char_boundary(text: &str, start: usize) -> usize {
    let next = start + '\\'.len_utf8();
    if next >= text.len() {
        return next;
    }

    next + text[next..].chars().next().map_or(0, char::len_utf8)
}

fn word_has_command_substitution(
    word: &Word,
    source: &str,
    zsh_options: Option<&ZshOptionState>,
) -> bool {
    word_classification_from_analysis(analyze_word(word, source, zsh_options))
        .has_command_substitution()
}

fn parsed_assignment_value_has_command_substitution(
    value_text: &str,
    zsh_options: Option<&ZshOptionState>,
) -> bool {
    if value_text.is_empty() {
        return false;
    }

    let word = Parser::parse_word_string(value_text);
    word_classification_from_analysis(analyze_word(&word, value_text, zsh_options))
        .has_command_substitution()
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
    lists: &[ListFact],
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

fn list_has_conditional_assignment_shortcuts(list: &ListFact) -> bool {
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
