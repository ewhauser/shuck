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
    body: &StmtSeq,
) -> Vec<StatementFact> {
    let mut facts = Vec::new();
    collect_statement_facts_in_stmt_seq(body, commands, command_ids_by_span, &mut facts);
    facts
}

fn collect_statement_facts_in_stmt_seq<'a>(
    body: &StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    facts: &mut Vec<StatementFact>,
) {
    for stmt in &body.stmts {
        if let Some(id) = command_fact_for_stmt(stmt, commands, command_ids_by_span) {
            facts.push(StatementFact {
                body_span: body.span,
                stmt_span: stmt.span,
                command_id: id.id(),
            });
        }

        collect_statement_sequence_facts_in_command(
            &stmt.command,
            commands,
            command_ids_by_span,
            facts,
        );
    }
}

fn collect_statement_sequence_facts_in_command<'a>(
    command: &Command,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    facts: &mut Vec<StatementFact>,
) {
    match command {
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
        Command::Binary(binary) => {
            collect_statement_sequence_facts_in_stmt(
                &binary.left,
                commands,
                command_ids_by_span,
                facts,
            );
            collect_statement_sequence_facts_in_stmt(
                &binary.right,
                commands,
                command_ids_by_span,
                facts,
            );
        }
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.condition,
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.then_branch,
                    commands,
                    command_ids_by_span,
                    facts,
                );
                for (condition, body) in &command.elif_branches {
                    collect_statement_facts_in_stmt_seq(
                        condition,
                        commands,
                        command_ids_by_span,
                        facts,
                    );
                    collect_statement_facts_in_stmt_seq(body, commands, command_ids_by_span, facts);
                }
                if let Some(body) = &command.else_branch {
                    collect_statement_facts_in_stmt_seq(body, commands, command_ids_by_span, facts);
                }
            }
            CompoundCommand::For(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::Repeat(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::Foreach(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::ArithmeticFor(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::While(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.condition,
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::Until(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.condition,
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    collect_statement_facts_in_stmt_seq(
                        &case.body,
                        commands,
                        command_ids_by_span,
                        facts,
                    );
                }
            }
            CompoundCommand::Select(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                collect_statement_facts_in_stmt_seq(body, commands, command_ids_by_span, facts);
            }
            CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(inner) = &command.command {
                    collect_statement_sequence_facts_in_stmt(
                        inner,
                        commands,
                        command_ids_by_span,
                        facts,
                    );
                }
            }
            CompoundCommand::Coproc(command) => {
                collect_statement_sequence_facts_in_stmt(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommand::Always(command) => {
                collect_statement_facts_in_stmt_seq(
                    &command.body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    &command.always_body,
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
        },
        Command::Function(function) => {
            collect_statement_sequence_facts_in_stmt(
                &function.body,
                commands,
                command_ids_by_span,
                facts,
            );
        }
        Command::AnonymousFunction(function) => {
            collect_statement_sequence_facts_in_stmt(
                &function.body,
                commands,
                command_ids_by_span,
                facts,
            );
        }
    }
}

fn collect_statement_sequence_facts_in_stmt<'a>(
    stmt: &Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    facts: &mut Vec<StatementFact>,
) {
    collect_statement_sequence_facts_in_command(
        &stmt.command,
        commands,
        command_ids_by_span,
        facts,
    );
}


