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
    command_child_index: &CommandChildIndex,
) -> Vec<PipelineFact<'a>> {
    let command_relationships =
        CommandRelationshipContext::new(commands, command_ids_by_span, command_child_index);
    let mut nested_pipeline_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
            continue;
        }

        for child_id in command_relationships.child_ids(fact.id()) {
            let child = command_relationships.fact(*child_id);
            if matches!(
                child.command(),
                Command::Binary(child) if matches!(child.op, BinaryOp::Pipe | BinaryOp::PipeAll)
            ) {
                nested_pipeline_commands.insert(*child_id);
            }
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

            let segments = pipeline_segments(fact.command())?;
            Some(PipelineFact {
                key: fact.key(),
                command,
                segments: segments
                    .into_iter()
                    .map(|stmt| {
                        build_pipeline_segment_fact(
                            stmt,
                            command_relationships,
                            fact.id(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                operators: pipeline_operator_facts(command),
            })
        })
        .collect()
}

fn pipeline_segments(command: &Command) -> Option<Vec<&Stmt>> {
    let Command::Binary(command) = command else {
        return None;
    };
    if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
        return None;
    }

    let mut segments = Vec::new();
    collect_pipeline_segments(command, &mut segments);
    Some(segments)
}

fn collect_pipeline_segments<'a>(command: &'a BinaryCommand, segments: &mut Vec<&'a Stmt>) {
    match &command.left.command {
        Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(left, segments);
        }
        _ => segments.push(&command.left),
    }

    match &command.right.command {
        Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(right, segments);
        }
        _ => segments.push(&command.right),
    }
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
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
) -> PipelineSegmentFact<'a> {
    let Some(fact) = command_relationships.child_or_lookup_fact(parent_id, stmt) else {
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

#[cfg(test)]
mod pipeline_tests {
    use shuck_ast::{Command, StmtSeq, Word};
    use shuck_parser::parser::Parser;

    use super::pipeline_segments;

    fn parse_commands(source: &str) -> StmtSeq {
        let output = Parser::new(source).parse().unwrap();
        output.file.body
    }

    fn static_word_owned_text(word: &Word, source: &str) -> Option<String> {
        word.try_static_text(source).map(|text| text.into_owned())
    }

    #[test]
    fn pipeline_segments_flattens_pipe_chains() {
        let source = "printf '%s\\n' a | command kill 0 | tee out.txt\n";
        let commands = parse_commands(source);
        let Command::Binary(command) = &commands[0].command else {
            panic!("expected binary command");
        };

        let segments = pipeline_segments(&Command::Binary(command.clone()))
            .expect("expected pipeline segments")
            .into_iter()
            .map(|stmt| match &stmt.command {
                Command::Simple(command) => static_word_owned_text(&command.name, source).unwrap(),
                _ => "<non-simple>".to_owned(),
            })
            .collect::<Vec<_>>();

        assert_eq!(segments, vec!["printf", "command", "tee"]);
    }
}
