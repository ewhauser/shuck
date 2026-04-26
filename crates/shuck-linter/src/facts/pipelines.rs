#[derive(Debug, Clone)]
pub struct PipelineSegmentFact {
    stmt_span: Span,
    command_span: Span,
    command_id: CommandId,
    arena_stmt_id: Option<AstStmtId>,
    arena_command_id: Option<AstCommandId>,
    literal_name: Option<Box<str>>,
    effective_name: Option<Box<str>>,
}

impl PipelineSegmentFact {
    pub fn stmt_span(&self) -> Span {
        self.stmt_span
    }

    pub fn command_span(&self) -> Span {
        self.command_span
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn arena_stmt_id(&self) -> Option<AstStmtId> {
        self.arena_stmt_id
    }

    pub fn arena_command_id(&self) -> Option<AstCommandId> {
        self.arena_command_id
    }

    pub fn literal_name(&self) -> Option<&str> {
        self.literal_name.as_deref()
    }

    pub fn effective_name(&self) -> Option<&str> {
        self.effective_name.as_deref()
    }

    pub fn effective_or_literal_name(&self) -> Option<&str> {
        self.effective_name().or_else(|| self.literal_name())
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.effective_name() == Some(name)
    }

    pub fn static_utility_name(&self) -> Option<&str> {
        self.effective_or_literal_name()
    }

    pub fn static_utility_name_is(&self, name: &str) -> bool {
        self.static_utility_name() == Some(name)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PipelineOperatorFact {
    op: BinaryOp,
    span: Span,
}

impl PipelineOperatorFact {
    pub fn op(&self) -> BinaryOp {
        self.op
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone)]
pub struct PipelineFact {
    key: FactSpan,
    arena_command_id: Option<AstCommandId>,
    span: Span,
    segments: Box<[PipelineSegmentFact]>,
    operators: Box<[PipelineOperatorFact]>,
}

impl PipelineFact {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn arena_command_id(&self) -> Option<AstCommandId> {
        self.arena_command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn segments(&self) -> &[PipelineSegmentFact] {
        &self.segments
    }

    pub fn operators(&self) -> &[PipelineOperatorFact] {
        &self.operators
    }

    pub fn first_segment(&self) -> Option<&PipelineSegmentFact> {
        self.segments.first()
    }

    pub fn last_segment(&self) -> Option<&PipelineSegmentFact> {
        self.segments.last()
    }
}


pub(super) fn build_pipeline_facts<'a>(
    commands: &[CommandFact<'a>],
    _command_ids_by_span: &CommandLookupIndex,
    _command_child_index: &CommandChildIndex,
    arena_file: &ArenaFile,
) -> Vec<PipelineFact> {
    let command_ids_by_arena_id = commands
        .iter()
        .filter_map(|fact| Some((fact.arena_command_id()?.index(), fact.id())))
        .collect::<FxHashMap<_, _>>();
    let mut nested_pipeline_commands = FxHashSet::default();

    for fact in commands {
        let Some(command) = fact.arena_command_id().map(|id| arena_file.store.command(id)) else {
            continue;
        };
        let Some(command) = command.binary() else {
            continue;
        };
        if !matches!(command.op(), BinaryOp::Pipe | BinaryOp::PipeAll) {
            continue;
        }

        record_nested_arena_pipeline_command(
            command.left(),
            &command_ids_by_arena_id,
            &mut nested_pipeline_commands,
        );
        record_nested_arena_pipeline_command(
            command.right(),
            &command_ids_by_arena_id,
            &mut nested_pipeline_commands,
        );
    }

    commands
        .iter()
        .filter_map(|fact| {
            let command = fact.arena_command_id().map(|id| arena_file.store.command(id))?;
            let command = command.binary()?;
            if !matches!(command.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
                || nested_pipeline_commands.contains(&fact.id())
            {
                return None;
            }

            let segments = arena_pipeline_segments(command, &command_ids_by_arena_id, commands)?;
            Some(PipelineFact {
                key: fact.key(),
                arena_command_id: fact.arena_command_id(),
                span: fact.span(),
                segments: segments.into_boxed_slice(),
                operators: arena_pipeline_operator_facts(command),
            })
        })
        .collect()
}

fn record_nested_arena_pipeline_command(
    sequence: StmtSeqView<'_>,
    command_ids_by_arena_id: &FxHashMap<usize, CommandId>,
    nested_pipeline_commands: &mut FxHashSet<CommandId>,
) {
    let Some(command) = single_arena_stmt_command(sequence) else {
        return;
    };
    if command
        .binary()
        .is_some_and(|binary| matches!(binary.op(), BinaryOp::Pipe | BinaryOp::PipeAll))
        && let Some(id) = command_ids_by_arena_id.get(&command.id().index()).copied()
    {
        nested_pipeline_commands.insert(id);
    }
}

fn arena_pipeline_segments(
    command: BinaryCommandView<'_>,
    command_ids_by_arena_id: &FxHashMap<usize, CommandId>,
    commands: &[CommandFact<'_>],
) -> Option<Vec<PipelineSegmentFact>> {
    if !matches!(command.op(), BinaryOp::Pipe | BinaryOp::PipeAll) {
        return None;
    }

    let mut segments = Vec::new();
    collect_arena_pipeline_segments(command, command_ids_by_arena_id, commands, &mut segments)?;
    Some(segments)
}

fn collect_arena_pipeline_segments(
    command: BinaryCommandView<'_>,
    command_ids_by_arena_id: &FxHashMap<usize, CommandId>,
    commands: &[CommandFact<'_>],
    segments: &mut Vec<PipelineSegmentFact>,
) -> Option<()> {
    collect_arena_pipeline_sequence_segment(command.left(), command_ids_by_arena_id, commands, segments)?;
    collect_arena_pipeline_sequence_segment(command.right(), command_ids_by_arena_id, commands, segments)?;
    Some(())
}

fn collect_arena_pipeline_sequence_segment(
    sequence: StmtSeqView<'_>,
    command_ids_by_arena_id: &FxHashMap<usize, CommandId>,
    commands: &[CommandFact<'_>],
    segments: &mut Vec<PipelineSegmentFact>,
) -> Option<()> {
    let stmt = single_arena_stmt(sequence)?;
    let command = stmt.command();
    if let Some(binary) = command.binary()
        && matches!(binary.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        return collect_arena_pipeline_segments(binary, command_ids_by_arena_id, commands, segments);
    }

    let command_id = command_ids_by_arena_id.get(&command.id().index()).copied()?;
    let fact = commands.get(command_id.index())?;
    segments.push(PipelineSegmentFact {
        stmt_span: stmt.span(),
        command_span: command.span(),
        command_id,
        arena_stmt_id: Some(stmt.id()),
        arena_command_id: Some(command.id()),
        literal_name: fact
            .literal_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
        effective_name: fact
            .effective_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
    });
    Some(())
}

fn single_arena_stmt_command(sequence: StmtSeqView<'_>) -> Option<CommandView<'_>> {
    Some(single_arena_stmt(sequence)?.command())
}

fn single_arena_stmt(sequence: StmtSeqView<'_>) -> Option<StmtView<'_>> {
    let mut stmts = sequence.stmts();
    let stmt = stmts.next()?;
    if stmts.next().is_some() {
        return None;
    }
    Some(stmt)
}

fn arena_pipeline_operator_facts(command: BinaryCommandView<'_>) -> Box<[PipelineOperatorFact]> {
    let mut operators = Vec::new();
    collect_arena_pipeline_operator_facts(command, &mut operators);
    operators.into_boxed_slice()
}

fn collect_arena_pipeline_operator_facts(
    command: BinaryCommandView<'_>,
    out: &mut Vec<PipelineOperatorFact>,
) {
    if let Some(left) = single_arena_stmt_command(command.left()).and_then(CommandView::binary)
        && matches!(left.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_arena_pipeline_operator_facts(left, out);
    }

    out.push(PipelineOperatorFact {
        op: command.op(),
        span: command.op_span(),
    });

    if let Some(right) = single_arena_stmt_command(command.right()).and_then(CommandView::binary)
        && matches!(right.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_arena_pipeline_operator_facts(right, out);
    }
}

#[cfg(test)]
fn pipeline_segment_commands(command: BinaryCommandView<'_>) -> Vec<CommandView<'_>> {
    let mut segments = Vec::new();
    collect_pipeline_segment_commands(command, &mut segments);
    segments
}

#[cfg(test)]
fn collect_pipeline_segment_commands<'a>(
    command: BinaryCommandView<'a>,
    segments: &mut Vec<CommandView<'a>>,
) {
    collect_pipeline_segment_commands_from_sequence(command.left(), segments);
    collect_pipeline_segment_commands_from_sequence(command.right(), segments);
}

#[cfg(test)]
fn collect_pipeline_segment_commands_from_sequence<'a>(
    sequence: StmtSeqView<'a>,
    segments: &mut Vec<CommandView<'a>>,
) {
    let Some(command) = single_arena_stmt_command(sequence) else {
        return;
    };
    if let Some(binary) = command.binary()
        && matches!(binary.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline_segment_commands(binary, segments);
    } else {
        segments.push(command);
    }
}

#[cfg(test)]
mod pipeline_tests {
    use shuck_ast::static_word_text_arena;
    use shuck_parser::parser::Parser;

    use super::pipeline_segment_commands;

    #[test]
    fn pipeline_segments_flattens_pipe_chains() {
        let source = "printf '%s\\n' a | command kill 0 | tee out.txt\n";
        let output = Parser::new(source).parse().unwrap();
        let command = output
            .arena_file
            .view()
            .body()
            .stmts()
            .next()
            .unwrap()
            .command()
            .binary()
            .expect("expected binary command");

        let segments = pipeline_segment_commands(command)
            .into_iter()
            .map(|command| {
                command
                    .simple()
                    .and_then(|simple| static_word_text_arena(simple.name(), source))
                    .map(|text| text.into_owned())
                    .unwrap_or_else(|| "<non-simple>".to_owned())
            })
            .collect::<Vec<_>>();

        assert_eq!(segments, vec!["printf", "command", "tee"]);
    }
}
