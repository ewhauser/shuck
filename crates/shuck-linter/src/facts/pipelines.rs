#[derive(Debug, Clone)]
pub struct PipelineSegmentFact<'a> {
    stmt: &'a Stmt,
    command_id: CommandId,
    literal_name: Option<Box<str>>,
    effective_name: Option<Box<str>>,
}

impl<'a> PipelineSegmentFact<'a> {
    pub fn stmt(&self) -> &'a Stmt {
        self.stmt
    }

    pub fn command(&self) -> &'a Command {
        &self.stmt.command
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
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
pub struct PipelineFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    segments: Box<[PipelineSegmentFact<'a>]>,
    operators: Box<[PipelineOperatorFact]>,
}

impl<'a> PipelineFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn command(&self) -> &'a BinaryCommand {
        self.command
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn segments(&self) -> &[PipelineSegmentFact<'a>] {
        &self.segments
    }

    pub fn operators(&self) -> &[PipelineOperatorFact] {
        &self.operators
    }

    pub fn first_segment(&self) -> Option<&PipelineSegmentFact<'a>> {
        self.segments.first()
    }

    pub fn last_segment(&self) -> Option<&PipelineSegmentFact<'a>> {
        self.segments.last()
    }
}


pub(super) fn build_pipeline_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Vec<PipelineFact<'a>> {
    let mut nested_pipeline_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
            continue;
        }

        if matches!(
            &command.left.command,
            Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        ) && let Some(id) = command_id_for_command(&command.left.command, command_ids_by_span)
        {
            nested_pipeline_commands.insert(id);
        }
        if matches!(
            &command.right.command,
            Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        ) && let Some(id) = command_id_for_command(&command.right.command, command_ids_by_span)
        {
            nested_pipeline_commands.insert(id);
        }
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll)
                || nested_pipeline_commands.contains(&fact.id())
            {
                return None;
            }

            let segments = query::pipeline_segments(fact.command())?;
            Some(PipelineFact {
                key: fact.key(),
                command,
                segments: segments
                    .into_iter()
                    .map(|stmt| build_pipeline_segment_fact(stmt, commands, command_ids_by_span))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                operators: pipeline_operator_facts(command),
            })
        })
        .collect()
}

fn pipeline_operator_facts(command: &BinaryCommand) -> Box<[PipelineOperatorFact]> {
    let mut operators = Vec::new();
    collect_pipeline_operator_facts(command, &mut operators);
    operators.into_boxed_slice()
}

fn collect_pipeline_operator_facts(command: &BinaryCommand, out: &mut Vec<PipelineOperatorFact>) {
    if let Command::Binary(left) = &command.left.command
        && matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline_operator_facts(left, out);
    }

    out.push(PipelineOperatorFact {
        op: command.op,
        span: command.op_span,
    });

    if let Command::Binary(right) = &command.right.command
        && matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline_operator_facts(right, out);
    }
}

fn build_pipeline_segment_fact<'a>(
    stmt: &'a Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> PipelineSegmentFact<'a> {
    let Some(fact) = command_fact_for_stmt(stmt, commands, command_ids_by_span) else {
        unreachable!("pipeline segment should have a corresponding command fact");
    };

    PipelineSegmentFact {
        stmt,
        command_id: fact.id(),
        literal_name: fact
            .literal_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
        effective_name: fact
            .effective_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
    }
}

