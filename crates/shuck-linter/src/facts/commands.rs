#[derive(Debug, Clone)]
pub struct CommandFact<'a> {
    id: CommandId,
    key: FactSpan,
    visit: CommandVisit<'a>,
    nested_word_command: bool,
    normalized: NormalizedCommand<'a>,
    zsh_options: Option<ZshOptionState>,
    redirect_facts: Box<[RedirectFact<'a>]>,
    substitution_facts: Box<[SubstitutionFact]>,
    options: CommandOptionFacts<'a>,
    scope_read_source_words: Box<[PathWordFact<'a>]>,
    declaration_assignment_probes: Box<[DeclarationAssignmentProbe]>,
    glued_closing_bracket_operand_span: Option<Span>,
    glued_closing_bracket_insert_offset: Option<usize>,
    linebreak_in_test_anchor_span: Option<Span>,
    linebreak_in_test_insert_offset: Option<usize>,
    simple_test: Option<SimpleTestFact<'a>>,
    conditional: Option<ConditionalFact<'a>>,
}

impl<'a> CommandFact<'a> {
    pub fn id(&self) -> CommandId {
        self.id
    }

    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn stmt(&self) -> &'a Stmt {
        self.visit.stmt
    }

    pub fn command(&self) -> &'a Command {
        self.visit.command
    }

    pub fn span(&self) -> Span {
        command_span(self.visit.command)
    }

    pub fn span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.span(), source)
    }

    pub fn redirects(&self) -> &'a [Redirect] {
        self.visit.redirects
    }

    pub fn zsh_options(&self) -> Option<&ZshOptionState> {
        self.zsh_options.as_ref()
    }

    pub fn redirect_facts(&self) -> &[RedirectFact<'a>] {
        &self.redirect_facts
    }

    pub fn substitution_facts(&self) -> &[SubstitutionFact] {
        &self.substitution_facts
    }

    pub fn normalized(&self) -> &NormalizedCommand<'a> {
        &self.normalized
    }

    pub fn options(&self) -> &CommandOptionFacts<'a> {
        &self.options
    }

    pub fn scope_read_source_words(&self) -> &[PathWordFact<'a>] {
        &self.scope_read_source_words
    }

    pub fn declaration_assignment_probes(&self) -> &[DeclarationAssignmentProbe] {
        &self.declaration_assignment_probes
    }

    pub fn glued_closing_bracket_operand_span(&self) -> Option<Span> {
        self.glued_closing_bracket_operand_span
    }

    pub fn glued_closing_bracket_insert_offset(&self) -> Option<usize> {
        self.glued_closing_bracket_insert_offset
    }

    pub fn linebreak_in_test_anchor_span(&self) -> Option<Span> {
        self.linebreak_in_test_anchor_span
    }

    pub fn linebreak_in_test_insert_offset(&self) -> Option<usize> {
        self.linebreak_in_test_insert_offset
    }

    pub fn simple_test(&self) -> Option<&SimpleTestFact<'a>> {
        self.simple_test.as_ref()
    }

    pub fn conditional(&self) -> Option<&ConditionalFact<'a>> {
        self.conditional.as_ref()
    }

    pub fn literal_name(&self) -> Option<&str> {
        self.normalized.literal_name.as_deref()
    }

    pub fn effective_name(&self) -> Option<&str> {
        self.normalized.effective_name.as_deref()
    }

    pub fn effective_or_literal_name(&self) -> Option<&str> {
        self.normalized.effective_or_literal_name()
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.normalized.effective_name_is(name)
    }

    pub fn static_utility_name(&self) -> Option<&str> {
        self.effective_or_literal_name()
    }

    pub fn static_utility_name_is(&self, name: &str) -> bool {
        self.static_utility_name() == Some(name)
    }

    pub fn wrappers(&self) -> &[WrapperKind] {
        &self.normalized.wrappers
    }

    pub fn has_wrapper(&self, wrapper: WrapperKind) -> bool {
        self.normalized.has_wrapper(wrapper)
    }

    pub fn declaration(&self) -> Option<&NormalizedDeclaration<'a>> {
        self.normalized.declaration.as_ref()
    }

    pub fn body_span(&self) -> Span {
        self.normalized.body_span
    }

    pub fn body_name_word(&self) -> Option<&'a Word> {
        self.normalized.body_name_word()
    }

    pub fn body_word_span(&self) -> Option<Span> {
        self.normalized.body_word_span()
    }

    pub fn body_args(&self) -> &[&'a Word] {
        self.normalized.body_args()
    }

    pub fn file_operand_words(&self) -> &[&'a Word] {
        self.options.file_operand_words()
    }
}

fn pipeline_span_with_shellcheck_tail(
    commands: &[CommandFact<'_>],
    pipeline: &PipelineFact<'_>,
    source: &str,
) -> Span {
    let Some(first_segment) = pipeline.first_segment() else {
        unreachable!("pipeline has segments");
    };
    let Some(last_segment) = pipeline.last_segment() else {
        unreachable!("pipeline has segments");
    };
    let first = command_fact(commands, first_segment.command_id());
    let last = command_fact(commands, last_segment.command_id());
    let last_end = last.span_in_source(source).end;
    let end = extend_over_shellcheck_trailing_inline_space(last_end, source);

    let Some(body_name_word) = first.body_name_word() else {
        unreachable!("plain echo command should have a body name");
    };
    Span::from_positions(
        body_name_word.span.start,
        end,
    )
}

fn command_span_with_redirects_and_shellcheck_tail(
    command: &CommandFact<'_>,
    source: &str,
) -> Option<Span> {
    let body_name = command.body_name_word()?;
    let mut end = body_name.span.end;

    for word in command.body_args() {
        if word.span.end.offset > end.offset {
            end = word.span.end;
        }
    }

    for redirect in command.redirect_facts() {
        let redirect_end = redirect.redirect().span.end;
        if redirect_end.offset > end.offset {
            end = redirect_end;
        }
    }

    Some(Span::from_positions(
        body_name.span.start,
        extend_over_shellcheck_trailing_inline_space(end, source),
    ))
}

fn effective_command_zsh_options(
    semantic: &SemanticModel,
    offset: usize,
    normalized: &NormalizedCommand<'_>,
) -> Option<ZshOptionState> {
    let mut options = semantic.zsh_options_at(offset).cloned();
    if normalized.has_wrapper(WrapperKind::Noglob)
        && let Some(options) = options.as_mut()
    {
        options.glob = shuck_semantic::OptionValue::Off;
    }
    options
}

fn extend_over_shellcheck_trailing_inline_space(end: Position, source: &str) -> Position {
    let tail = &source[end.offset..];
    let spaces_len = tail
        .char_indices()
        .take_while(|(_, ch)| matches!(ch, ' ' | '\t'))
        .last()
        .map_or(0, |(index, ch)| index + ch.len_utf8());

    if spaces_len == 0 {
        return end;
    }

    let rest = &tail[spaces_len..];
    if rest.is_empty()
        || rest.starts_with('\n')
        || rest.starts_with('\r')
        || rest.starts_with(')')
        || rest.starts_with(']')
        || rest.starts_with('}')
    {
        end.advanced_by(&tail[..spaces_len])
    } else {
        end
    }
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}


fn build_background_semicolon_spans(
    commands: &[CommandFact<'_>],
    case_items: &[CaseItemFact<'_>],
    source: &str,
) -> Vec<Span> {
    let case_terminator_starts = case_items
        .iter()
        .filter_map(CaseItemFact::terminator_span)
        .map(|span| span.start.offset)
        .collect::<FxHashSet<_>>();
    let mut spans = commands
        .iter()
        .filter_map(|command| background_semicolon_span(command, &case_terminator_starts, source))
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

fn background_semicolon_span(
    command: &CommandFact<'_>,
    case_terminator_starts: &FxHashSet<usize>,
    source: &str,
) -> Option<Span> {
    if command.stmt().terminator != Some(StmtTerminator::Background(BackgroundOperator::Plain)) {
        return None;
    }

    let terminator_span = command.stmt().terminator_span?;
    if terminator_span.slice(source) != "&" {
        return None;
    }

    let semicolon_offset = source[terminator_span.end.offset..]
        .char_indices()
        .find_map(|(relative, ch)| match ch {
            ' ' | '\t' | '\r' => None,
            '\n' | '#' => Some(None),
            ';' => Some(Some(terminator_span.end.offset + relative)),
            _ => Some(None),
        })??;

    if case_terminator_starts.contains(&semicolon_offset) {
        return None;
    }

    let start = position_at_offset(source, semicolon_offset)?;
    let end = position_at_offset(source, semicolon_offset + 1)?;
    Some(Span::from_positions(start, end))
}


fn build_scope_read_source_words<'a>(
    commands: &[CommandFact<'a>],
    pipelines: &[PipelineFact<'a>],
    if_condition_command_ids: &FxHashSet<CommandId>,
) -> Vec<Box<[PathWordFact<'a>]>> {
    let mut words_by_command = vec![Vec::new(); commands.len()];

    for command in commands {
        let mut scope_words = own_scope_read_source_words(command, if_condition_command_ids);
        if command_has_file_output_redirect(command) {
            scope_words.extend(nested_scope_read_source_words(
                commands,
                command,
                if_condition_command_ids,
            ));
        }
        dedup_path_words(&mut scope_words);
        words_by_command[command.id().index()] = scope_words;
    }

    for pipeline in pipelines {
        let writer_ids = pipeline
            .segments()
            .iter()
            .map(|segment| segment.command_id())
            .filter(|id| {
                commands
                    .get(id.index())
                    .is_some_and(command_has_file_output_redirect)
            })
            .collect::<Vec<_>>();
        if writer_ids.is_empty() {
            continue;
        }

        let mut pipeline_words = commands
            .iter()
            .filter(|command| contains_span(pipeline.span(), command.span()))
            .flat_map(|command| own_scope_read_source_words(command, if_condition_command_ids))
            .collect::<Vec<_>>();
        dedup_path_words(&mut pipeline_words);

        for writer_id in writer_ids {
            words_by_command[writer_id.index()].extend(pipeline_words.iter().copied());
            dedup_path_words(&mut words_by_command[writer_id.index()]);
        }
    }

    words_by_command
        .into_iter()
        .map(Vec::into_boxed_slice)
        .collect()
}

fn own_scope_read_source_words<'a>(
    command: &CommandFact<'a>,
    if_condition_command_ids: &FxHashSet<CommandId>,
) -> Vec<PathWordFact<'a>> {
    let mut words = command_file_operand_words(command)
        .into_iter()
        .map(|word| PathWordFact {
            word,
            context: ExpansionContext::CommandArgument,
        })
        .collect::<Vec<_>>();
    words.extend(command_redirect_read_source_words(command));
    if !if_condition_command_ids.contains(&command.id()) {
        words.extend(command_conditional_path_words(command));
    }
    words
}

fn nested_scope_read_source_words<'a>(
    commands: &[CommandFact<'a>],
    command: &CommandFact<'a>,
    if_condition_command_ids: &FxHashSet<CommandId>,
) -> Vec<PathWordFact<'a>> {
    commands
        .iter()
        .filter(|other| other.id() != command.id() && contains_span(command.span(), other.span()))
        .flat_map(|other| own_scope_read_source_words(other, if_condition_command_ids))
        .collect()
}

fn dedup_path_words(words: &mut Vec<PathWordFact<'_>>) {
    let mut seen = FxHashSet::<(FactSpan, ExpansionContext)>::default();
    words.retain(|fact| seen.insert((FactSpan::new(fact.word().span), fact.context())));
}

fn command_has_file_output_redirect(command: &CommandFact<'_>) -> bool {
    command.redirect_facts().iter().any(|redirect| {
        matches!(
            redirect.redirect().kind,
            RedirectKind::Output
                | RedirectKind::Clobber
                | RedirectKind::Append
                | RedirectKind::OutputBoth
        ) && redirect
            .analysis()
            .is_some_and(|analysis| analysis.is_file_target())
    })
}

fn command_file_operand_words<'a>(command: &CommandFact<'a>) -> Vec<&'a Word> {
    command.file_operand_words().to_vec()
}

fn command_redirect_read_source_words<'a>(command: &CommandFact<'a>) -> Vec<PathWordFact<'a>> {
    command
        .redirect_facts()
        .iter()
        .filter_map(|redirect| {
            if !matches!(
                redirect.redirect().kind,
                RedirectKind::Input | RedirectKind::ReadWrite
            ) {
                return None;
            }

            Some(PathWordFact {
                word: redirect.redirect().word_target()?,
                context: match ExpansionContext::from_redirect_kind(redirect.redirect().kind) {
                    Some(context) => context,
                    None => unreachable!("input redirects should carry a word target context"),
                },
            })
        })
        .collect()
}

fn command_conditional_path_words<'a>(command: &CommandFact<'a>) -> Vec<PathWordFact<'a>> {
    let mut words = Vec::new();

    if let Some(conditional) = command.conditional() {
        for node in conditional.nodes() {
            match node {
                ConditionalNodeFact::Binary(binary)
                    if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
                {
                    if let Some(word) = binary.left().word() {
                        words.push(PathWordFact {
                            word,
                            context: ExpansionContext::StringTestOperand,
                        });
                    }
                    if let Some(word) = binary.right().word() {
                        words.push(PathWordFact {
                            word,
                            context: ExpansionContext::StringTestOperand,
                        });
                    }
                }
                ConditionalNodeFact::Binary(_) => {}
                ConditionalNodeFact::BareWord(_) | ConditionalNodeFact::Other(_) => {}
                ConditionalNodeFact::Unary(_) => {}
            }
        }
    }

    words
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn contains_span_strictly(outer: Span, inner: Span) -> bool {
    contains_span(outer, inner)
        && (outer.start.offset < inner.start.offset || inner.end.offset < outer.end.offset)
}

fn build_backtick_command_name_spans(commands: &[CommandFact<'_>]) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Simple(command) if command.args.is_empty() => {
                plain_backtick_command_name_span(&command.name)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

fn plain_backtick_command_name_span(word: &Word) -> Option<Span> {
    let [part] = word.parts.as_slice() else {
        return None;
    };

    match &part.kind {
        WordPart::CommandSubstitution {
            syntax: CommandSubstitutionSyntax::Backtick,
            ..
        } => Some(part.span),
        _ => None,
    }
}


fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => builtin_span(command),
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(command) => compound_span(command),
        Command::Function(command) => command.span,
        Command::AnonymousFunction(command) => command.span,
    }
}

fn command_lookup_kind(command: &Command) -> CommandLookupKind {
    match command {
        Command::Simple(_) => CommandLookupKind::Simple,
        Command::Builtin(command) => CommandLookupKind::Builtin(match command {
            BuiltinCommand::Break(_) => BuiltinLookupKind::Break,
            BuiltinCommand::Continue(_) => BuiltinLookupKind::Continue,
            BuiltinCommand::Return(_) => BuiltinLookupKind::Return,
            BuiltinCommand::Exit(_) => BuiltinLookupKind::Exit,
        }),
        Command::Decl(_) => CommandLookupKind::Decl,
        Command::Binary(_) => CommandLookupKind::Binary,
        Command::Compound(command) => CommandLookupKind::Compound(match command {
            CompoundCommand::If(_) => CompoundLookupKind::If,
            CompoundCommand::For(_) => CompoundLookupKind::For,
            CompoundCommand::Repeat(_) => CompoundLookupKind::Repeat,
            CompoundCommand::Foreach(_) => CompoundLookupKind::Foreach,
            CompoundCommand::ArithmeticFor(_) => CompoundLookupKind::ArithmeticFor,
            CompoundCommand::While(_) => CompoundLookupKind::While,
            CompoundCommand::Until(_) => CompoundLookupKind::Until,
            CompoundCommand::Case(_) => CompoundLookupKind::Case,
            CompoundCommand::Select(_) => CompoundLookupKind::Select,
            CompoundCommand::Subshell(_) => CompoundLookupKind::Subshell,
            CompoundCommand::BraceGroup(_) => CompoundLookupKind::BraceGroup,
            CompoundCommand::Arithmetic(_) => CompoundLookupKind::Arithmetic,
            CompoundCommand::Time(_) => CompoundLookupKind::Time,
            CompoundCommand::Conditional(_) => CompoundLookupKind::Conditional,
            CompoundCommand::Coproc(_) => CompoundLookupKind::Coproc,
            CompoundCommand::Always(_) => CompoundLookupKind::Always,
        }),
        Command::Function(_) => CommandLookupKind::Function,
        Command::AnonymousFunction(_) => CommandLookupKind::AnonymousFunction,
    }
}

fn command_id_for_command(
    command: &Command,
    command_ids_by_span: &CommandLookupIndex,
) -> Option<CommandId> {
    command_ids_by_span
        .get(&FactSpan::new(command_span(command)))
        .and_then(|entries| {
            let kind = command_lookup_kind(command);
            entries
                .iter()
                .find(|entry| entry.kind == kind)
                .map(|entry| entry.id)
        })
}

fn command_fact<'a>(commands: &'a [CommandFact<'a>], id: CommandId) -> &'a CommandFact<'a> {
    &commands[id.index()]
}

fn build_command_parent_ids(commands: &[CommandFact<'_>]) -> Vec<Option<CommandId>> {
    let mut command_spans = commands
        .iter()
        .map(|command| (command.span(), command.id()))
        .collect::<Vec<_>>();
    if command_spans
        .windows(2)
        .any(|window| compare_command_offset_entries(window[0], window[1]).is_gt())
    {
        command_spans.sort_unstable_by(|left, right| compare_command_offset_entries(*left, *right));
    }

    let mut parent_ids = vec![None; commands.len()];
    let mut active_commands = Vec::<OpenParentCommand>::new();

    for (span, id) in command_spans {
        while active_commands
            .last()
            .is_some_and(|candidate| candidate.end_offset < span.end.offset)
        {
            active_commands.pop();
        }

        parent_ids[id.index()] = active_commands.last().map(|command| command.id);
        active_commands.push(OpenParentCommand {
            id,
            end_offset: span.end.offset,
        });
    }

    parent_ids
}

fn build_command_dominance_barrier_flags(commands: &[CommandFact<'_>]) -> Vec<bool> {
    commands
        .iter()
        .map(|fact| match fact.command() {
            Command::Binary(_) => true,
            Command::Compound(compound) => !matches!(
                compound,
                CompoundCommand::BraceGroup(_)
                    | CompoundCommand::Arithmetic(_)
                    | CompoundCommand::Time(_)
            ),
            Command::Simple(_)
            | Command::Builtin(_)
            | Command::Decl(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => false,
        })
        .collect()
}

fn sort_and_dedup_spans(spans: &mut Vec<Span>) {
    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
}

#[derive(Debug, Clone, Copy)]
struct OpenParentCommand {
    id: CommandId,
    end_offset: usize,
}

fn trim_trailing_whitespace_span(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let trimmed = text.trim_end_matches(char::is_whitespace);
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}

fn command_fact_for_command<'a>(
    command: &Command,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<&'a CommandFact<'a>> {
    command_id_for_command(command, command_ids_by_span).map(|id| command_fact(commands, id))
}

fn command_fact_for_stmt<'a>(
    stmt: &Stmt,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<&'a CommandFact<'a>> {
    command_fact_for_command(&stmt.command, commands, command_ids_by_span)
}

fn builtin_span(command: &BuiltinCommand) -> Span {
    match command {
        BuiltinCommand::Break(command) => command.span,
        BuiltinCommand::Continue(command) => command.span,
        BuiltinCommand::Return(command) => command.span,
        BuiltinCommand::Exit(command) => command.span,
    }
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            commands.span
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}
