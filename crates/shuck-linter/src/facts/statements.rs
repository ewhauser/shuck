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
    body: StmtSeqView<'_>,
) -> Vec<StatementFact> {
    let mut facts = Vec::new();
    collect_statement_facts_in_stmt_seq(body, commands, command_ids_by_span, &mut facts);
    facts
}

fn collect_statement_facts_in_stmt_seq<'a>(
    body: StmtSeqView<'_>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    facts: &mut Vec<StatementFact>,
) {
    for stmt in body.stmts() {
        if let Some(id) = command_fact_for_arena_stmt(stmt, commands, command_ids_by_span) {
            facts.push(StatementFact {
                body_span: body.span(),
                stmt_span: stmt.span(),
                command_id: id.id(),
            });
        }

        collect_statement_sequence_facts_in_command(stmt.command(), commands, command_ids_by_span, facts);
    }
}

fn collect_statement_sequence_facts_in_command<'a>(
    command: CommandView<'_>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    facts: &mut Vec<StatementFact>,
) {
    match command.kind() {
        ArenaFileCommandKind::Simple | ArenaFileCommandKind::Builtin | ArenaFileCommandKind::Decl => {}
        ArenaFileCommandKind::Binary => {
            let binary = command.binary().expect("binary command view");
            collect_statement_facts_in_stmt_seq(binary.left(), commands, command_ids_by_span, facts);
            collect_statement_facts_in_stmt_seq(binary.right(), commands, command_ids_by_span, facts);
        }
        ArenaFileCommandKind::Compound => {
            let compound = command.compound().expect("compound command view");
            match compound.node() {
            CompoundCommandNode::If { condition, then_branch, elif_branches, else_branch, .. } => {
                let store = compound.store();
                collect_statement_facts_in_stmt_seq(
                    store.stmt_seq(*condition),
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    store.stmt_seq(*then_branch),
                    commands,
                    command_ids_by_span,
                    facts,
                );
                for branch in store.elif_branches(*elif_branches) {
                    collect_statement_facts_in_stmt_seq(
                        store.stmt_seq(branch.condition),
                        commands,
                        command_ids_by_span,
                        facts,
                    );
                    collect_statement_facts_in_stmt_seq(store.stmt_seq(branch.body), commands, command_ids_by_span, facts);
                }
                if let Some(body) = else_branch {
                    collect_statement_facts_in_stmt_seq(store.stmt_seq(*body), commands, command_ids_by_span, facts);
                }
            }
            CompoundCommandNode::For { body, .. }
            | CompoundCommandNode::Repeat { body, .. }
            | CompoundCommandNode::Foreach { body, .. }
            | CompoundCommandNode::Select { body, .. } => {
                collect_statement_facts_in_stmt_seq(
                    compound.store().stmt_seq(*body),
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommandNode::ArithmeticFor(command) => {
                collect_statement_facts_in_stmt_seq(
                    compound.store().stmt_seq(command.body),
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommandNode::While { condition, body } | CompoundCommandNode::Until { condition, body } => {
                collect_statement_facts_in_stmt_seq(
                    compound.store().stmt_seq(*condition),
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    compound.store().stmt_seq(*body),
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            CompoundCommandNode::Case { cases, .. } => {
                for case in compound.store().case_items(*cases) {
                    collect_statement_facts_in_stmt_seq(
                        compound.store().stmt_seq(case.body),
                        commands,
                        command_ids_by_span,
                        facts,
                    );
                }
            }
            CompoundCommandNode::Subshell(body) | CompoundCommandNode::BraceGroup(body) => {
                collect_statement_facts_in_stmt_seq(compound.store().stmt_seq(*body), commands, command_ids_by_span, facts);
            }
            CompoundCommandNode::Arithmetic(_) | CompoundCommandNode::Conditional(_) => {}
            CompoundCommandNode::Time { command, .. } => {
                if let Some(inner) = command {
                    collect_statement_facts_in_stmt_seq(compound.store().stmt_seq(*inner), commands, command_ids_by_span, facts);
                }
            }
            CompoundCommandNode::Coproc { body, .. } => {
                collect_statement_facts_in_stmt_seq(compound.store().stmt_seq(*body), commands, command_ids_by_span, facts);
            }
            CompoundCommandNode::Always { body, always_body } => {
                collect_statement_facts_in_stmt_seq(
                    compound.store().stmt_seq(*body),
                    commands,
                    command_ids_by_span,
                    facts,
                );
                collect_statement_facts_in_stmt_seq(
                    compound.store().stmt_seq(*always_body),
                    commands,
                    command_ids_by_span,
                    facts,
                );
            }
            }
        }
        ArenaFileCommandKind::Function => {
            let function = command.function().expect("function command view");
            collect_statement_facts_in_stmt_seq(function.body(), commands, command_ids_by_span, facts);
        }
        ArenaFileCommandKind::AnonymousFunction => {
            let function = command.anonymous_function().expect("anonymous function command view");
            collect_statement_facts_in_stmt_seq(function.body(), commands, command_ids_by_span, facts);
        }
    }
}
