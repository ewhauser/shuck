// Shared body-shape predicates for condition/status facts.
//
// This file owns repeated command/body shape questions, but it intentionally leaves diagnostic span
// collection, word payload extraction, and rule policy in the calling fact modules.

/// Reusable predicates for status/test-like body shapes.
///
/// Keep this focused on repeated structural questions across condition/status facts. If a predicate
/// needs command-specific wording, exact fix spans, or assignment policy, expose only the shared
/// shape here and keep the payload-specific decision in the owning fact.
pub(super) struct BodyShapeAnalyzer<'source> {
    source: &'source str,
}

/// A syntax payload where the previous command status is available for inspection.
pub(super) enum StatusAvailableSite<'a> {
    /// A simple command whose test-style operands can inspect `$?`.
    SimpleTest(&'a SimpleCommand),
    /// A `[[ ... ]]` expression whose operands can inspect `$?`.
    ConditionalExpression(&'a ConditionalExpr),
    /// An arithmetic command whose expression can inspect `$?`.
    ArithmeticCommand(&'a ArithmeticCommand),
}

impl<'source> BodyShapeAnalyzer<'source> {
    /// Creates an analyzer bound to the source text used by syntax classifiers.
    pub(super) fn new(source: &'source str) -> Self {
        Self { source }
    }

    /// Returns whether any statement in `commands` or its nested bodies terminates in a test.
    pub(super) fn sequence_tail_contains_nested_test_command(&self, commands: &[Stmt]) -> bool {
        commands
            .iter()
            .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
    }

    /// Returns whether `stmt` itself or a nested statement sequence contains a terminal test.
    pub(super) fn stmt_or_nested_sequence_contains_test_command(&self, stmt: &Stmt) -> bool {
        if stmt_terminals_are_test_commands(stmt, self.source) {
            return true;
        }

        match &stmt.command {
            Command::Binary(command) => {
                self.stmt_or_nested_sequence_contains_test_command(&command.left)
                    || self.stmt_or_nested_sequence_contains_test_command(&command.right)
            }
            Command::Compound(command) => match command {
                CompoundCommand::If(command) => {
                    condition_terminals_are_test_commands(&command.condition, self.source)
                        || command
                            .then_branch
                            .iter()
                            .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                        || command.elif_branches.iter().any(|(condition, branch)| {
                            condition_terminals_are_test_commands(condition, self.source)
                                || branch.iter().any(|stmt| {
                                    self.stmt_or_nested_sequence_contains_test_command(stmt)
                                })
                        })
                        || command.else_branch.as_ref().is_some_and(|branch| {
                            branch
                                .iter()
                                .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                        })
                }
                CompoundCommand::While(command) => {
                    condition_terminals_are_test_commands(&command.condition, self.source)
                        || command
                            .body
                            .iter()
                            .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                }
                CompoundCommand::Until(command) => {
                    condition_terminals_are_test_commands(&command.condition, self.source)
                        || command
                            .body
                            .iter()
                            .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                }
                CompoundCommand::For(command) => command
                    .body
                    .iter()
                    .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt)),
                CompoundCommand::Select(command) => command
                    .body
                    .iter()
                    .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt)),
                CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => body
                    .iter()
                    .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt)),
                CompoundCommand::Time(command) => command
                    .command
                    .as_ref()
                    .is_some_and(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt)),
                CompoundCommand::Always(command) => {
                    command
                        .body
                        .iter()
                        .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                        || command
                            .always_body
                            .iter()
                            .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                }
                CompoundCommand::Case(command) => command.cases.iter().any(|case| {
                    case.body
                        .iter()
                        .any(|stmt| self.stmt_or_nested_sequence_contains_test_command(stmt))
                }),
                CompoundCommand::Conditional(_)
                | CompoundCommand::Repeat(_)
                | CompoundCommand::Foreach(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Coproc(_) => false,
            },
            Command::Function(function) => {
                self.stmt_or_nested_sequence_contains_test_command(&function.body)
            }
            Command::AnonymousFunction(function) => {
                self.stmt_or_nested_sequence_contains_test_command(&function.body)
            }
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => false,
        }
    }

    /// Collects status reads from sibling follow-up chains in `commands` and nested bodies.
    ///
    /// The analyzer owns the sibling/body traversal; the status span collector remains in
    /// `conditionals.rs` because it owns the exact payload scan and diagnostic spans.
    pub(super) fn collect_status_test_followup_chains_with_sibling_tail(
        &self,
        commands: &StmtSeq,
        spans: &mut Vec<Span>,
    ) {
        BodyTopology::new(commands).for_each_body(|body| {
            self.collect_status_test_followup_chains_in_body(body, spans);
            BodyTraversal::Descend
        });
    }

    /// Visits payloads where a previous command status is available in `commands`.
    ///
    /// The traversal owns status-availability propagation through bodies, branches, lists, and
    /// function bodies. Callers keep payload-specific span extraction local by matching on the
    /// returned site.
    pub(super) fn visit_status_available_sites<'a>(
        &self,
        commands: &'a StmtSeq,
        status_available: bool,
        visitor: &mut impl FnMut(StatusAvailableSite<'a>),
    ) {
        self.visit_status_available_sites_in_seq(commands, status_available, visitor);
    }

    /// Returns whether `stmt` is a logical chain with a status-based test followed by non-test work.
    pub(super) fn stmt_is_status_test_followup_chain(&self, stmt: &Stmt) -> bool {
        let Command::Binary(command) = &stmt.command else {
            return false;
        };
        let Some(chain) = BinaryCommandChain::logical_list(command) else {
            return false;
        };

        let mut found = false;
        chain.visit_nodes(|command| {
            found |= stmt_is_status_based_test_command(&command.left, self.source)
                && !stmt_terminals_are_test_commands(&command.right, self.source);
        });
        found
    }

    /// Returns whether `commands[index]` starts a return guard using a later status accumulator.
    pub(super) fn stmt_starts_status_accumulator_return_guard(
        &self,
        index: usize,
        commands: &[Stmt],
    ) -> bool {
        let Some(next_stmt) = commands.get(index + 1) else {
            return false;
        };
        let Some(name) = stmt_assignment_only_scalar_literal_name(next_stmt, self.source, "0")
        else {
            return false;
        };
        let later = &commands[index + 2..];
        later
            .iter()
            .any(|stmt| self.stmt_contains_status_capture_assignment_to_name(stmt, name))
            && later.iter().any(|stmt| self.stmt_returns_name(stmt, name))
    }

    /// Returns whether `stmt` assigns an unquoted `$?` capture to `name`.
    pub(super) fn stmt_contains_status_capture_assignment_to_name(
        &self,
        stmt: &Stmt,
        name: &Name,
    ) -> bool {
        match &stmt.command {
            Command::Simple(_) => stmt_plain_assignment_only_name(stmt).is_some_and(|target| {
                target == name && stmt_is_assignment_only_unquoted_status_capture(stmt)
            }),
            Command::Binary(command) => {
                self.stmt_contains_status_capture_assignment_to_name(&command.left, name)
                    || self.stmt_contains_status_capture_assignment_to_name(&command.right, name)
            }
            Command::Compound(command) => match command {
                CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => body
                    .iter()
                    .any(|stmt| self.stmt_contains_status_capture_assignment_to_name(stmt, name)),
                CompoundCommand::Time(command) => command.command.as_ref().is_some_and(|stmt| {
                    self.stmt_contains_status_capture_assignment_to_name(stmt, name)
                }),
                CompoundCommand::If(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::For(_)
                | CompoundCommand::Select(_)
                | CompoundCommand::Case(_)
                | CompoundCommand::Conditional(_)
                | CompoundCommand::Repeat(_)
                | CompoundCommand::Foreach(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Coproc(_)
                | CompoundCommand::Always(_) => false,
            },
            Command::Builtin(_)
            | Command::Decl(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => false,
        }
    }

    /// Returns whether `stmt` directly or through a transparent grouping returns `name`.
    pub(super) fn stmt_returns_name(&self, stmt: &Stmt, name: &Name) -> bool {
        match &stmt.command {
            Command::Builtin(BuiltinCommand::Return(command)) => command
                .code
                .as_ref()
                .is_some_and(|word| word_is_name_reference(word, name)),
            Command::Binary(command) => {
                self.stmt_returns_name(&command.left, name)
                    || self.stmt_returns_name(&command.right, name)
            }
            Command::Compound(command) => match command {
                CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => {
                    body.iter().any(|stmt| self.stmt_returns_name(stmt, name))
                }
                CompoundCommand::Time(command) => command
                    .command
                    .as_ref()
                    .is_some_and(|stmt| self.stmt_returns_name(stmt, name)),
                CompoundCommand::If(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::For(_)
                | CompoundCommand::Select(_)
                | CompoundCommand::Case(_)
                | CompoundCommand::Conditional(_)
                | CompoundCommand::Repeat(_)
                | CompoundCommand::Foreach(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Coproc(_)
                | CompoundCommand::Always(_) => false,
            },
            Command::Simple(_)
            | Command::Decl(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => false,
            Command::Builtin(
                BuiltinCommand::Break(_) | BuiltinCommand::Continue(_) | BuiltinCommand::Exit(_),
            ) => false,
        }
    }

    /// Collects status reads from direct sibling follow-up chains in one body.
    fn collect_status_test_followup_chains_in_body(&self, body: &StmtSeq, spans: &mut Vec<Span>) {
        for (previous, current) in BodyTopology::new(body).sibling_pairs() {
            if stmt_is_standalone_non_status_test_command(previous, self.source)
                && self.stmt_is_status_test_followup_chain(current)
            {
                collect_status_parameter_spans_in_stmt(current, self.source, spans);
            }
        }
    }

    fn visit_status_available_sites_in_seq<'a>(
        &self,
        commands: &'a StmtSeq,
        mut status_available: bool,
        visitor: &mut impl FnMut(StatusAvailableSite<'a>),
    ) {
        for stmt in commands.iter() {
            self.visit_status_available_sites_in_stmt(stmt, status_available, visitor);
            status_available = true;
        }
    }

    fn visit_status_available_sites_in_stmt<'a>(
        &self,
        stmt: &'a Stmt,
        status_available: bool,
        visitor: &mut impl FnMut(StatusAvailableSite<'a>),
    ) {
        self.visit_status_available_sites_in_command(&stmt.command, status_available, visitor);
    }

    fn visit_status_available_sites_in_command<'a>(
        &self,
        command: &'a Command,
        status_available: bool,
        visitor: &mut impl FnMut(StatusAvailableSite<'a>),
    ) {
        match command {
            Command::Simple(command) => {
                if status_available {
                    visitor(StatusAvailableSite::SimpleTest(command));
                }
            }
            Command::Compound(command) => match command {
                CompoundCommand::If(command) => {
                    self.visit_status_available_sites_in_seq(
                        &command.condition,
                        status_available,
                        visitor,
                    );
                    self.visit_status_available_sites_in_seq(&command.then_branch, true, visitor);
                    for (condition, body) in &command.elif_branches {
                        self.visit_status_available_sites_in_seq(condition, true, visitor);
                        self.visit_status_available_sites_in_seq(body, true, visitor);
                    }
                    if let Some(else_branch) = &command.else_branch {
                        self.visit_status_available_sites_in_seq(else_branch, true, visitor);
                    }
                }
                CompoundCommand::While(command) => {
                    self.visit_status_available_sites_in_seq(
                        &command.condition,
                        status_available,
                        visitor,
                    );
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::Until(command) => {
                    self.visit_status_available_sites_in_seq(
                        &command.condition,
                        status_available,
                        visitor,
                    );
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::For(command) => {
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::Repeat(command) => {
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::Foreach(command) => {
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::ArithmeticFor(command) => {
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::Case(command) => {
                    for case in &command.cases {
                        self.visit_status_available_sites_in_seq(&case.body, true, visitor);
                    }
                }
                CompoundCommand::Select(command) => {
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                }
                CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                    self.visit_status_available_sites_in_seq(body, true, visitor);
                }
                CompoundCommand::Time(command) => {
                    if let Some(command) = &command.command {
                        self.visit_status_available_sites_in_stmt(
                            command,
                            status_available,
                            visitor,
                        );
                    }
                }
                CompoundCommand::Conditional(command) => {
                    if status_available {
                        visitor(StatusAvailableSite::ConditionalExpression(
                            &command.expression,
                        ));
                    }
                }
                CompoundCommand::Arithmetic(command) => {
                    if status_available {
                        visitor(StatusAvailableSite::ArithmeticCommand(command));
                    }
                }
                CompoundCommand::Coproc(command) => {
                    self.visit_status_available_sites_in_stmt(&command.body, true, visitor);
                }
                CompoundCommand::Always(command) => {
                    self.visit_status_available_sites_in_seq(&command.body, true, visitor);
                    self.visit_status_available_sites_in_seq(&command.always_body, true, visitor);
                }
            },
            Command::Binary(command) => {
                self.visit_status_available_sites_in_stmt(
                    &command.left,
                    status_available,
                    visitor,
                );
                self.visit_status_available_sites_in_stmt(&command.right, true, visitor);
            }
            Command::Function(command) => {
                self.visit_status_available_sites_in_function_body(&command.body, visitor);
            }
            Command::AnonymousFunction(command) => {
                self.visit_status_available_sites_in_function_body(&command.body, visitor);
            }
            Command::Builtin(_) | Command::Decl(_) => {}
        }
    }

    fn visit_status_available_sites_in_function_body<'a>(
        &self,
        stmt: &'a Stmt,
        visitor: &mut impl FnMut(StatusAvailableSite<'a>),
    ) {
        match &stmt.command {
            Command::Compound(CompoundCommand::BraceGroup(body))
            | Command::Compound(CompoundCommand::Subshell(body)) => {
                self.visit_status_available_sites_in_seq(body, false, visitor);
            }
            _ => self.visit_status_available_sites_in_stmt(stmt, false, visitor),
        }
    }
}
