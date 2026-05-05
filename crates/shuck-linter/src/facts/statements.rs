#[derive(Debug, Clone, Copy)]
pub struct StatementFact {
    body_span: Span,
    stmt_span: Span,
    command_id: CommandId,
}

impl StatementFact {
    pub fn body_span(&self) -> Span {
        self.body_span
    }

    pub fn stmt_span(&self) -> Span {
        self.stmt_span
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_statement_facts<'a>(
    commands: &[CommandFact<'a>],
    semantic: &SemanticModel,
) -> Vec<StatementFact> {
    let command_ids_by_stmt_span = build_command_ids_by_stmt_span(commands);

    semantic
        .statement_sequence_commands()
        .iter()
        .filter_map(|statement| {
            let command_id =
                command_ids_by_stmt_span.get(&FactSpan::new(statement.stmt_span()))?;
            Some(StatementFact {
                body_span: statement.body_span(),
                stmt_span: statement.stmt_span(),
                command_id: *command_id,
            })
        })
        .collect()
}

fn build_command_ids_by_stmt_span(commands: &[CommandFact<'_>]) -> FxHashMap<FactSpan, CommandId> {
    let mut command_ids_by_stmt_span =
        FxHashMap::with_capacity_and_hasher(commands.len(), Default::default());

    for command in commands {
        if command.is_nested_word_command() {
            continue;
        }
        command_ids_by_stmt_span
            .entry(FactSpan::new(command.stmt().span))
            .or_insert_with(|| command.id());
    }

    command_ids_by_stmt_span
}
