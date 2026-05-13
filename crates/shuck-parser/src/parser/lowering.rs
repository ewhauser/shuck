use super::*;

impl<'a> Parser<'a> {
    pub(super) fn compound_span(compound: &CompoundCommand) -> Span {
        match compound {
            CompoundCommand::If(command) => command.span,
            CompoundCommand::For(command) => command.span,
            CompoundCommand::Repeat(command) => command.span,
            CompoundCommand::Foreach(command) => command.span,
            CompoundCommand::ArithmeticFor(command) => command.span,
            CompoundCommand::While(command) => command.span,
            CompoundCommand::Until(command) => command.span,
            CompoundCommand::Case(command) => command.span,
            CompoundCommand::Select(command) => command.span,
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => body.span,
            CompoundCommand::Arithmetic(command) => command.span,
            CompoundCommand::Time(command) => command.span,
            CompoundCommand::Conditional(command) => command.span,
            CompoundCommand::Coproc(command) => command.span,
            CompoundCommand::Always(command) => command.span,
        }
    }

    pub(super) fn stmt_seq_with_span(span: Span, stmts: Vec<Stmt>) -> StmtSeq {
        StmtSeq {
            leading_comments: Vec::new(),
            stmts,
            trailing_comments: Vec::new(),
            span,
        }
    }

    pub(super) fn binary_stmt(left: Stmt, op: BinaryOp, op_span: Span, right: Stmt) -> Stmt {
        let span = left.span.merge(right.span);
        Stmt {
            leading_comments: Vec::new(),
            command: AstCommand::Binary(BinaryCommand {
                left: Box::new(left),
                op,
                op_span,
                right: Box::new(right),
                span,
            }),
            negated: false,
            redirects: Box::default(),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span,
        }
    }

    pub(super) fn lower_builtin_command(
        builtin: BuiltinCommand,
    ) -> (AstBuiltinCommand, SmallVec<[Redirect; 1]>, Span) {
        match builtin {
            BuiltinCommand::Break(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Break(AstBreakCommand {
                        depth: command.depth,
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_boxed_slice(),
                        span,
                    }),
                    redirects,
                    span,
                )
            }
            BuiltinCommand::Continue(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Continue(AstContinueCommand {
                        depth: command.depth,
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_boxed_slice(),
                        span,
                    }),
                    redirects,
                    span,
                )
            }
            BuiltinCommand::Return(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Return(AstReturnCommand {
                        code: command.code,
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_boxed_slice(),
                        span,
                    }),
                    redirects,
                    span,
                )
            }
            BuiltinCommand::Exit(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Exit(AstExitCommand {
                        code: command.code,
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_boxed_slice(),
                        span,
                    }),
                    redirects,
                    span,
                )
            }
        }
    }

    pub(super) fn lower_non_sequence_command_to_stmt(command: Command) -> Stmt {
        match command {
            Command::Simple(command) => Stmt {
                leading_comments: Vec::new(),
                command: AstCommand::Simple(AstSimpleCommand {
                    name: command.name,
                    args: command.args.into_vec(),
                    assignments: command.assignments.into_boxed_slice(),
                    span: command.span,
                }),
                negated: false,
                redirects: command.redirects.into_boxed_slice(),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
                span: command.span,
            },
            Command::Builtin(command) => {
                let (command, redirects, span) = Self::lower_builtin_command(command);
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Builtin(command),
                    negated: false,
                    redirects: redirects.into_boxed_slice(),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span,
                }
            }
            Command::Decl(command) => {
                let command = *command;
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Decl(AstDeclClause {
                        variant: command.variant,
                        variant_span: command.variant_span,
                        operands: command.operands.into_vec(),
                        assignments: command.assignments.into_boxed_slice(),
                        span: command.span,
                    }),
                    negated: false,
                    redirects: command.redirects.into_boxed_slice(),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span: command.span,
                }
            }
            Command::Compound(compound, redirects) => {
                let span = Self::compound_span(&compound);
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Compound(*compound),
                    negated: false,
                    redirects: redirects.into_boxed_slice(),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span,
                }
            }
            Command::Function(function) => Stmt {
                leading_comments: Vec::new(),
                span: function.span,
                command: AstCommand::Function(function),
                negated: false,
                redirects: Box::default(),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
            },
            Command::AnonymousFunction(function, redirects) => Stmt {
                leading_comments: Vec::new(),
                span: function.span,
                command: AstCommand::AnonymousFunction(function),
                negated: false,
                redirects: redirects.into_boxed_slice(),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
            },
        }
    }
}
