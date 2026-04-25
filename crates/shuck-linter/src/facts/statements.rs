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

pub(super) fn build_statement_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    body: &StmtSeq,
) -> Vec<StatementFact> {
    let command_relationships =
        CommandRelationshipContext::new(commands, command_ids_by_span, command_child_index);
    let mut facts = Vec::new();
    collect_statement_facts_in_stmt_seq(body, None, command_relationships, &mut facts);
    facts
}

fn collect_statement_facts_in_stmt_seq<'a>(
    body: &StmtSeq,
    parent_id: Option<CommandId>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    facts: &mut Vec<StatementFact>,
) {
    for stmt in &body.stmts {
        let stmt_fact = command_relationships.fact_for_stmt_with_parent(parent_id, stmt);
        if let Some(fact) = stmt_fact {
            facts.push(StatementFact {
                body_span: body.span,
                stmt_span: stmt.span,
                command_id: fact.id(),
            });
        }

        collect_statement_sequence_facts_in_command(
            &stmt.command,
            stmt_fact.map(CommandFact::id),
            command_relationships,
            facts,
        );
    }
}

fn collect_statement_sequence_facts_in_command<'a>(
    command: &Command,
    parent_id: Option<CommandId>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    facts: &mut Vec<StatementFact>,
) {
    match command {
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
        Command::Binary(binary) => {
            collect_statement_sequence_facts_in_stmt(
                &binary.left,
                parent_id,
                command_relationships,
                facts,
            );
            collect_statement_sequence_facts_in_stmt(
                &binary.right,
                parent_id,
                command_relationships,
                facts,
            );
        }
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.condition,
                    parent_id,
                    command_relationships,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.then_branch,
                    parent_id,
                    command_relationships,
                    facts,
                );
                for (condition, body) in &command.elif_branches {
                    collect_statement_facts_in_stmt_seq(
                        condition,
                        parent_id,
                        command_relationships,
                        facts,
                    );
                    collect_statement_facts_in_stmt_seq(
                        body,
                        parent_id,
                        command_relationships,
                        facts,
                    );
                }
                if let Some(body) = &command.else_branch {
                    collect_statement_facts_in_stmt_seq(
                        body,
                        parent_id,
                        command_relationships,
                        facts,
                    );
                }
            }
            CompoundCommand::For(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Repeat(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Foreach(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::ArithmeticFor(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::While(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.condition,
                    parent_id,
                    command_relationships,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Until(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.condition,
                    parent_id,
                    command_relationships,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    collect_statement_facts_in_stmt_seq(
                        &case.body,
                        parent_id,
                        command_relationships,
                        facts,
                    );
                }
            }
            CompoundCommand::Select(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                collect_statement_facts_in_stmt_seq(
                    body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(inner) = &command.command {
                    collect_statement_sequence_facts_in_stmt(
                        inner,
                        parent_id,
                        command_relationships,
                        facts,
                    );
                }
            }
            CompoundCommand::Coproc(command) => {
                collect_statement_sequence_facts_in_stmt(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
            CompoundCommand::Always(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    parent_id,
                    command_relationships,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.always_body,
                    parent_id,
                    command_relationships,
                    facts,
                );
            }
        },
        Command::Function(function) => {
            collect_statement_sequence_facts_in_stmt(
                &function.body,
                parent_id,
                command_relationships,
                facts,
            );
        }
        Command::AnonymousFunction(function) => {
            collect_statement_sequence_facts_in_stmt(
                &function.body,
                parent_id,
                command_relationships,
                facts,
            );
        }
    }
}

fn collect_statement_sequence_facts_in_stmt<'a>(
    stmt: &Stmt,
    parent_id: Option<CommandId>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    facts: &mut Vec<StatementFact>,
) {
    let stmt_fact = command_relationships.fact_for_stmt_with_parent(parent_id, stmt);
    collect_statement_sequence_facts_in_command(
        &stmt.command,
        stmt_fact.map(CommandFact::id),
        command_relationships,
        facts,
    );
}
