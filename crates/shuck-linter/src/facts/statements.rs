use super::*;

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
pub(crate) fn build_statement_facts<'a>(
    commands: &[CommandFact<'a>],
    semantic: &SemanticModel,
) -> Vec<StatementFact> {
    let command_ids_by_stmt_span = build_command_ids_by_stmt_span(commands);

    semantic
        .statement_sequence_commands()
        .iter()
        .filter_map(|statement| {
            let command_id = command_ids_by_stmt_span.get(&FactSpan::new(statement.stmt_span()))?;
            Some(StatementFact {
                body_span: statement.body_span(),
                stmt_span: statement.stmt_span(),
                command_id: *command_id,
            })
        })
        .collect()
}

pub(crate) fn build_command_ids_by_stmt_span(
    commands: &[CommandFact<'_>],
) -> FxHashMap<FactSpan, CommandId> {
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

/// The edit that removes one statement without leaving broken syntax behind.
///
/// Statements in a sequence are removed together with their own `;` or `&`
/// terminator, and the right operand of `&&` / `||` is removed together with
/// its operator. Statements whose removal would orphan a heredoc body are not
/// described here, so callers simply have no safe deletion for them.
#[derive(Debug, Clone, Copy)]
pub struct StatementDeletionFact {
    delete_span: Span,
    sequence: Option<StatementSequenceSlot>,
}

/// The statement sequence a deletable statement belongs to.
#[derive(Debug, Clone, Copy)]
pub struct StatementSequenceSlot {
    body_span: Span,
    statement_count: usize,
    may_be_empty: bool,
}

impl StatementDeletionFact {
    /// Span to delete so the surrounding code stays well-formed.
    pub fn delete_span(&self) -> Span {
        self.delete_span
    }

    /// Sequence membership, or `None` for the right operand of `&&` / `||`.
    pub fn sequence(&self) -> Option<StatementSequenceSlot> {
        self.sequence
    }
}

impl StatementSequenceSlot {
    pub fn body_span(&self) -> Span {
        self.body_span
    }

    /// Number of statements directly in the sequence.
    pub fn statement_count(&self) -> usize {
        self.statement_count
    }

    /// Whether the sequence stays valid with every statement removed. Only the
    /// file body qualifies; `{ }`, `then fi`, `do done`, and similar bodies
    /// need at least one command.
    pub fn may_be_empty(&self) -> bool {
        self.may_be_empty
    }
}

/// Builds deletion edits keyed by each statement's semantic span (the
/// statement without its terminator, extended over its redirects), which is
/// the span the semantic model reports for unreachable commands.
#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_statement_deletion_facts(
    commands: &[CommandFact<'_>],
    command_fact_indices_by_id: &[Option<usize>],
    statement_facts: &[StatementFact],
    file_body_span: Span,
    source: &str,
) -> FxHashMap<FactSpan, StatementDeletionFact> {
    let heredoc_ranges = commands
        .iter()
        .flat_map(|command| command.redirects().iter())
        .filter_map(|redirect| {
            let heredoc = redirect.heredoc()?;
            Some((redirect.span.start.offset(), heredoc.body.span.end.offset()))
        })
        .collect::<Vec<_>>();
    let orphans_heredoc = |span: Span| {
        heredoc_ranges.iter().any(|&(operator_start, body_end)| {
            span.start.offset() <= operator_start
                && operator_start < span.end.offset()
                && body_end > span.end.offset()
        })
    };

    // Statement spans from the parser can run past the statement's own syntax
    // into a following `&&`, `||`, `|`, `;;`, or newline. Those separator
    // tokens carry exact spans, so index them by end offset and peel them off.
    let mut separators = FxHashMap::<usize, Position>::default();
    for command in commands {
        let stmt = command.stmt();
        if let Some(terminator) = stmt.terminator_span {
            separators.insert(terminator.end.offset(), terminator.start);
        }
        match command.command() {
            Command::Binary(binary) => {
                separators.insert(binary.op_span.end.offset(), binary.op_span.start);
            }
            Command::Compound(CompoundCommand::Case(case)) => {
                for terminator in case.cases.iter().filter_map(|item| item.terminator_span) {
                    separators.insert(terminator.end.offset(), terminator.start);
                }
            }
            _ => {}
        }
    }
    let statement_end = |span: Span| tight_statement_end(span, &separators, source);

    let mut sequence_lengths = FxHashMap::<FactSpan, usize>::default();
    for statement in statement_facts {
        *sequence_lengths
            .entry(FactSpan::new(statement.body_span()))
            .or_default() += 1;
    }

    let command_fact = |id: CommandId| {
        command_fact_indices_by_id
            .get(id.index())
            .copied()
            .flatten()
            .map(|index| &commands[index])
    };

    let mut deletions = FxHashMap::default();
    for statement in statement_facts {
        let Some(command) = command_fact(statement.command_id()) else {
            continue;
        };
        let stmt = command.stmt();
        let key = semantic_statement_span(stmt);
        let end = own_terminator_span(stmt, source)
            .map_or_else(|| statement_end(key), |terminator| terminator.end);
        let end = skip_inline_blanks(end, source);
        let delete_span = Span::from_positions(stmt.span.start, end);
        if orphans_heredoc(delete_span) {
            continue;
        }
        let body_key = FactSpan::new(statement.body_span());
        deletions.insert(
            FactSpan::new(key),
            StatementDeletionFact {
                delete_span,
                sequence: Some(StatementSequenceSlot {
                    body_span: statement.body_span(),
                    statement_count: sequence_lengths.get(&body_key).copied().unwrap_or(1),
                    may_be_empty: statement.body_span() == file_body_span,
                }),
            },
        );
    }

    for command in commands {
        let Command::Binary(binary) = command.command() else {
            continue;
        };
        if !matches!(binary.op, BinaryOp::And | BinaryOp::Or) {
            continue;
        }
        let key = semantic_statement_span(&binary.right);
        let delete_span = Span::from_positions(
            statement_end(semantic_statement_span(&binary.left)),
            statement_end(key),
        );
        if orphans_heredoc(delete_span) {
            continue;
        }
        deletions
            .entry(FactSpan::new(key))
            .or_insert(StatementDeletionFact {
                delete_span,
                sequence: None,
            });
    }

    deletions
}

/// Mirrors the span the semantic model records for a statement: the statement
/// without a trailing terminator, extended over any redirects.
fn semantic_statement_span(stmt: &Stmt) -> Span {
    let mut end = stmt
        .terminator_span
        .filter(|terminator| terminator.end.offset() == stmt.span.end.offset())
        .map_or(stmt.span.end, |terminator| terminator.start);
    for redirect in stmt.redirects.iter() {
        if redirect.span.end.offset() > end.offset() {
            end = redirect.span.end;
        }
    }
    Span::from_positions(stmt.span.start, end)
}

/// Returns the statement's own `;` or `&` terminator. Case-arm terminators
/// such as `;;` belong to the arm and must survive the deletion.
fn own_terminator_span(stmt: &Stmt, source: &str) -> Option<Span> {
    let terminator = stmt.terminator_span?;
    match stmt.terminator? {
        StmtTerminator::Background(_) => Some(terminator),
        StmtTerminator::Semicolon => (terminator.slice(source) == ";").then_some(terminator),
    }
}

/// End of a statement's own syntax: `span` with trailing whitespace and any
/// trailing separator tokens (list operators, terminators, case-arm `;;`)
/// removed.
fn tight_statement_end(
    span: Span,
    separators: &FxHashMap<usize, Position>,
    source: &str,
) -> Position {
    let mut span = trim_trailing_whitespace_span(span, source);
    while let Some(start) = separators.get(&span.end.offset())
        && start.offset() >= span.start.offset()
        && start.offset() < span.end.offset()
    {
        span = trim_trailing_whitespace_span(Span::from_positions(span.start, *start), source);
    }
    span.end
}

fn skip_inline_blanks(position: Position, source: &str) -> Position {
    let rest = source.get(position.offset()..).unwrap_or_default();
    let blanks = &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()];
    position.advanced_by(blanks)
}
