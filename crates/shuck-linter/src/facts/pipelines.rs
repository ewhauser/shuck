#[derive(Debug, Clone)]
pub struct PipelineSegmentFact<'a> {
    stmt: &'a Stmt,
    command_id: CommandId,
    literal_name: Option<Cow<'a, str>>,
    effective_name: Option<Cow<'a, str>>,
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


#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_pipeline_facts<'a>(
    commands: &[CommandFact<'a>],
    command_fact_indices_by_id: &[Option<usize>],
    semantic: &SemanticModel,
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
) -> Vec<PipelineFact<'a>> {
    let command_relationships = CommandRelationshipContext::new(
        commands,
        command_fact_indices_by_id,
        command_ids_by_span,
        command_child_index,
    );

    semantic
        .pipeline_commands()
        .into_iter()
        .filter_map(|pipeline| {
            let fact = command_fact_for_semantic_span_matching(
                commands,
                command_fact_indices_by_id,
                command_ids_by_span,
                pipeline.span,
                |command| matches!(command.command(), Command::Binary(_)),
            )?;
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
                return None;
            }

            Some(PipelineFact {
                key: fact.key(),
                command,
                segments: pipeline
                    .segments
                    .iter()
                    .map(|segment| {
                        build_pipeline_segment_fact(segment.command_span, command_relationships)
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                operators: pipeline
                    .segments
                    .into_iter()
                    .filter_map(|segment| segment.operator_before)
                    .map(|operator| {
                        let op = match operator.kind {
                            SemanticPipelineOperatorKind::Pipe => BinaryOp::Pipe,
                            SemanticPipelineOperatorKind::PipeAll => BinaryOp::PipeAll,
                        };
                        PipelineOperatorFact {
                            op,
                            span: operator.span,
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
        })
        .collect()
}

fn build_pipeline_segment_fact<'a>(
    command_span: Span,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> PipelineSegmentFact<'a> {
    let Some(fact) = command_fact_for_semantic_span_matching(
        command_relationships.commands,
        command_relationships.command_fact_indices_by_id,
        command_relationships.command_ids_by_span,
        command_span,
        |command| {
            !matches!(
                command.command(),
                Command::Binary(binary)
                    if matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
            )
        },
    ) else {
        unreachable!("pipeline segment should have a corresponding command fact");
    };

    PipelineSegmentFact {
        stmt: fact.stmt(),
        command_id: fact.id(),
        literal_name: fact.normalized().literal_name.clone(),
        effective_name: fact.normalized().effective_name.clone(),
    }
}
