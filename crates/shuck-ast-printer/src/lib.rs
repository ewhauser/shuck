use serde_json::{Map, Number, Value};
use shuck_ast::{
    ArithmeticCommand, ArithmeticForCommand, Assignment, AssignmentValue, BuiltinCommand,
    CaseCommand, CaseItem, CaseTerminator, Command, CompoundCommand, ConditionalBinaryExpr,
    ConditionalBinaryOp, ConditionalCommand, ConditionalExpr, ConditionalParenExpr,
    ConditionalUnaryExpr, ConditionalUnaryOp, CoprocCommand, DeclClause, DeclName, DeclOperand,
    ForCommand, FunctionDef, IfCommand, ListOperator, LiteralText, ParameterOp, Pipeline,
    Position, Redirect, RedirectKind, Script, SelectCommand, SimpleCommand, SourceText, Span,
    TimeCommand, UntilCommand, WhileCommand, Word, WordPart,
};

/// Serialize a parsed Script to gbash-compatible typed JSON.
pub fn to_typed_json(script: &Script, source: &str) -> Value {
    Printer::new(source).encode_file(script).value
}

#[derive(Clone)]
struct EncodedNode {
    value: Value,
    pos: Position,
    end: Position,
}

#[derive(Clone)]
struct EncodedStmt {
    value: Value,
    pos: Position,
    end: Position,
}

#[derive(Clone)]
struct SingleStmtParts {
    position: Position,
    end: Position,
    negated: bool,
    redirs: Vec<Value>,
    cmd: EncodedNode,
}

#[derive(Clone)]
struct StmtFragment<'a> {
    base: &'a Command,
    chain: Vec<(ListOperator, &'a Command)>,
    semicolon: Option<Position>,
    background: bool,
}

struct Printer<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> Printer<'a> {
    fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (idx, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    fn encode_file(&self, script: &Script) -> EncodedNode {
        let stmts = self.encode_stmts(&script.commands);
        let pos = stmts
            .first()
            .map(|stmt| stmt.pos)
            .unwrap_or(script.span.start);
        let end = stmts.last().map(|stmt| stmt.end).unwrap_or(script.span.end);

        let mut map = self.node_object(Some("File"), pos, end);
        self.insert_array(
            &mut map,
            "Stmts",
            stmts.iter().map(|stmt| stmt.value.clone()).collect(),
        );
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn encode_stmt_values(&self, commands: &[Command]) -> Vec<Value> {
        self.encode_stmts(commands)
            .into_iter()
            .map(|stmt| stmt.value)
            .collect()
    }

    fn encode_stmt_values_before(
        &self,
        commands: &[Command],
        before: Position,
        operator: &str,
    ) -> Vec<Value> {
        let mut stmts = self.encode_stmts(commands);
        if let Some((stmt, command)) = stmts.last_mut().zip(commands.last()) {
            if let Value::Object(map) = &mut stmt.value {
                if !map.contains_key("Semicolon") {
                    if let Some(separator) = self.find_operator_after_span(
                        self.command_span(command),
                        before.offset,
                        operator,
                    ) {
                        self.insert_pos(map, "Semicolon", separator);
                        self.insert_pos(map, "End", separator.advanced_by(operator));
                    }
                }
            }
        }
        stmts.into_iter().map(|stmt| stmt.value).collect()
    }

    fn encode_stmts(&self, commands: &[Command]) -> Vec<EncodedStmt> {
        commands
            .iter()
            .flat_map(|command| self.fragments_for_command(command))
            .map(|fragment| self.encode_fragment(&fragment))
            .collect()
    }

    fn fragments_for_command<'b>(&self, command: &'b Command) -> Vec<StmtFragment<'b>> {
        let Command::List(list) = command else {
            return vec![StmtFragment {
                base: command,
                chain: Vec::new(),
                semicolon: None,
                background: false,
            }];
        };

        let mut fragments = Vec::new();
        let mut current_base: &Command = &list.first;
        let mut current_chain: Vec<(ListOperator, &Command)> = Vec::new();
        let mut current_last: &Command = &list.first;

        for (op, next) in &list.rest {
            match op {
                ListOperator::And | ListOperator::Or => {
                    current_chain.push((*op, next));
                    current_last = next;
                }
                ListOperator::Semicolon | ListOperator::Background => {
                    let search_end = if self.is_background_placeholder(next) {
                        list.span.end.offset
                    } else {
                        self.command_span(next).start.offset
                    };
                    let semicolon = self.find_operator_after_span(
                        self.command_span(current_last),
                        search_end,
                        match op {
                            ListOperator::Semicolon => ";",
                            ListOperator::Background => "&",
                            _ => unreachable!(),
                        },
                    );
                    fragments.push(StmtFragment {
                        base: current_base,
                        chain: current_chain.clone(),
                        semicolon,
                        background: matches!(op, ListOperator::Background),
                    });
                    if self.is_background_placeholder(next) {
                        return fragments;
                    }
                    current_base = next;
                    current_chain.clear();
                    current_last = next;
                }
            }
        }

        fragments.push(StmtFragment {
            base: current_base,
            chain: current_chain,
            semicolon: None,
            background: false,
        });
        fragments
    }

    fn encode_fragment(&self, fragment: &StmtFragment<'_>) -> EncodedStmt {
        if fragment.chain.is_empty() {
            let base = self.encode_single_stmt_base(fragment.base);
            return self.wrap_stmt(
                base.position,
                self.stmt_end(base.end, fragment.semicolon, fragment.background),
                base.cmd,
                base.negated,
                base.redirs,
                fragment.semicolon,
                fragment.background,
            );
        }

        let mut lhs_stmt = self.encode_stmt_without_separator(fragment.base);
        let mut lhs_cmd = fragment.base;
        let mut binary = None;

        for (op, rhs_cmd) in &fragment.chain {
            let rhs_stmt = self.encode_stmt_without_separator(rhs_cmd);
            let op_pos = self.find_operator_between_spans(
                self.command_span(lhs_cmd),
                self.command_span(rhs_cmd),
                match op {
                    ListOperator::And => "&&",
                    ListOperator::Or => "||",
                    _ => "",
                },
            );
            let current = self.encode_binary_cmd(
                lhs_stmt.clone(),
                rhs_stmt.clone(),
                self.bin_cmd_code(*op),
                op_pos,
            );
            lhs_stmt = self.wrap_stmt(
                current.pos,
                current.end,
                current,
                false,
                Vec::new(),
                None,
                false,
            );
            lhs_cmd = rhs_cmd;
            binary = Some(lhs_stmt.clone());
        }

        let inner = binary.unwrap_or(lhs_stmt);
        let cmd = self.extract_cmd_from_stmt(&inner.value);
        self.wrap_stmt(
            inner.pos,
            self.stmt_end(inner.end, fragment.semicolon, fragment.background),
            EncodedNode {
                value: cmd,
                pos: inner.pos,
                end: inner.end,
            },
            false,
            Vec::new(),
            fragment.semicolon,
            fragment.background,
        )
    }

    fn encode_stmt_without_separator(&self, command: &Command) -> EncodedStmt {
        let base = self.encode_single_stmt_base(command);
        self.wrap_stmt(
            base.position,
            base.end,
            base.cmd,
            base.negated,
            base.redirs,
            None,
            false,
        )
    }

    fn encode_single_stmt_base(&self, command: &Command) -> SingleStmtParts {
        match command {
            Command::Pipeline(pipeline) => self.encode_pipeline_stmt_base(pipeline),
            Command::Simple(simple) => {
                let cmd = self.encode_call_expr(simple);
                let redirs = self.encode_redirects(&simple.redirects);
                SingleStmtParts {
                    position: simple.span.start,
                    end: self.max_pos(cmd.end, self.last_redirect_end(&simple.redirects)),
                    negated: false,
                    redirs,
                    cmd,
                }
            }
            Command::Builtin(builtin) => {
                let (cmd, redirects, span) = self.encode_builtin_call_expr(builtin);
                SingleStmtParts {
                    position: span.start,
                    end: self.max_pos(cmd.end, self.last_redirect_end(redirects)),
                    negated: false,
                    redirs: self.encode_redirects(redirects),
                    cmd,
                }
            }
            Command::Decl(command) => {
                let cmd = self.encode_decl_clause(command);
                SingleStmtParts {
                    position: command.span.start,
                    end: self.max_pos(cmd.end, self.last_redirect_end(&command.redirects)),
                    negated: false,
                    redirs: self.encode_redirects(&command.redirects),
                    cmd,
                }
            }
            Command::Compound(compound, redirects) => {
                let cmd = self.encode_compound_command(compound);
                SingleStmtParts {
                    position: cmd.pos,
                    end: self.max_pos(cmd.end, self.last_redirect_end(redirects)),
                    negated: false,
                    redirs: self.encode_redirects(redirects),
                    cmd,
                }
            }
            Command::Function(function) => {
                let cmd = self.encode_function(function);
                SingleStmtParts {
                    position: cmd.pos,
                    end: cmd.end,
                    negated: false,
                    redirs: Vec::new(),
                    cmd,
                }
            }
            Command::List(list) => {
                let stmt = self.encode_fragment(&StmtFragment {
                    base: &list.first,
                    chain: list
                        .rest
                        .iter()
                        .filter_map(|(op, cmd)| match op {
                            ListOperator::And | ListOperator::Or => Some((*op, cmd)),
                            _ => None,
                        })
                        .collect(),
                    semicolon: None,
                    background: false,
                });
                let cmd = self.extract_cmd_from_stmt(&stmt.value);
                SingleStmtParts {
                    position: stmt.pos,
                    end: stmt.end,
                    negated: false,
                    redirs: Vec::new(),
                    cmd: EncodedNode {
                        value: cmd,
                        pos: stmt.pos,
                        end: stmt.end,
                    },
                }
            }
        }
    }

    fn encode_decl_clause(&self, command: &DeclClause) -> EncodedNode {
        let pos = command
            .assignments
            .first()
            .map(|assignment| assignment.span.start)
            .unwrap_or(command.variant_span.start);
        let end = command
            .operands
            .last()
            .map(|operand| self.decl_operand_end(operand))
            .or_else(|| {
                command
                    .assignments
                    .last()
                    .map(|assignment| assignment.span.end)
            })
            .unwrap_or(command.variant_span.end);
        let mut map = self.node_object(Some("DeclClause"), pos, end);
        self.insert_value(
            &mut map,
            "Variant",
            Some(
                self.value_lit_node(
                    &command.variant,
                    command.variant_span.start,
                    command.variant_span.end,
                )
                .value,
            ),
        );
        self.insert_array(
            &mut map,
            "Assigns",
            command
                .assignments
                .iter()
                .map(|assignment| self.encode_assignment(assignment).value)
                .collect(),
        );
        self.insert_array(
            &mut map,
            "Operands",
            command
                .operands
                .iter()
                .map(|operand| self.encode_decl_operand(operand).value)
                .collect(),
        );
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn encode_pipeline_stmt_base(&self, pipeline: &Pipeline) -> SingleStmtParts {
        if pipeline.commands.len() == 1 {
            let mut inner = self.encode_single_stmt_base(&pipeline.commands[0]);
            inner.position = pipeline.span.start;
            inner.negated = pipeline.negated || inner.negated;
            return inner;
        }

        let mut lhs = self.encode_stmt_without_separator(&pipeline.commands[0]);
        let mut last = &pipeline.commands[0];
        let mut node = None;

        for rhs_command in pipeline.commands.iter().skip(1) {
            let rhs = self.encode_stmt_without_separator(rhs_command);
            let op_pos = self.find_operator_between_spans(
                self.command_span(last),
                self.command_span(rhs_command),
                "|",
            );
            let current = self.encode_binary_cmd(lhs.clone(), rhs.clone(), 13, op_pos);
            lhs = self.wrap_stmt(
                current.pos,
                current.end,
                current,
                false,
                Vec::new(),
                None,
                false,
            );
            last = rhs_command;
            node = Some(lhs.clone());
        }

        let encoded = node.unwrap_or(lhs);
        let cmd = self.extract_cmd_from_stmt(&encoded.value);
        SingleStmtParts {
            position: pipeline.span.start,
            end: encoded.end,
            negated: pipeline.negated,
            redirs: Vec::new(),
            cmd: EncodedNode {
                value: cmd,
                pos: encoded.pos,
                end: encoded.end,
            },
        }
    }

    fn encode_builtin_call_expr<'b>(
        &self,
        builtin: &'b BuiltinCommand,
    ) -> (EncodedNode, &'b [Redirect], Span) {
        match builtin {
            BuiltinCommand::Break(command) => (
                self.encode_flow_control_call(
                    "break",
                    command.depth.as_ref(),
                    &command.extra_args,
                    &command.assignments,
                    command.span,
                ),
                &command.redirects,
                command.span,
            ),
            BuiltinCommand::Continue(command) => (
                self.encode_flow_control_call(
                    "continue",
                    command.depth.as_ref(),
                    &command.extra_args,
                    &command.assignments,
                    command.span,
                ),
                &command.redirects,
                command.span,
            ),
            BuiltinCommand::Return(command) => (
                self.encode_flow_control_call(
                    "return",
                    command.code.as_ref(),
                    &command.extra_args,
                    &command.assignments,
                    command.span,
                ),
                &command.redirects,
                command.span,
            ),
            BuiltinCommand::Exit(command) => (
                self.encode_flow_control_call(
                    "exit",
                    command.code.as_ref(),
                    &command.extra_args,
                    &command.assignments,
                    command.span,
                ),
                &command.redirects,
                command.span,
            ),
        }
    }

    fn encode_flow_control_call(
        &self,
        name: &str,
        first: Option<&Word>,
        extra_args: &[Word],
        assignments: &[Assignment],
        span: Span,
    ) -> EncodedNode {
        let mut args = vec![self.synthetic_literal_word(
            name,
            Span::from_positions(span.start, span.start.advanced_by(name)),
        )];
        if let Some(first) = first {
            args.push(first.clone());
        }
        args.extend(extra_args.iter().cloned());
        self.encode_call_expr_words(assignments, &args)
    }

    fn encode_call_expr(&self, simple: &SimpleCommand) -> EncodedNode {
        let mut args = Vec::new();
        if !self.is_empty_word(&simple.name) {
            args.push(simple.name.clone());
        }
        args.extend(simple.args.iter().cloned());
        self.encode_call_expr_words(&simple.assignments, &args)
    }

    fn encode_call_expr_words(&self, assignments: &[Assignment], args: &[Word]) -> EncodedNode {
        let pos = assignments
            .first()
            .map(|assignment| assignment.span.start)
            .or_else(|| args.first().map(|word| word.span.start))
            .unwrap_or_default();
        let end = assignments
            .last()
            .filter(|_| args.is_empty())
            .map(|assignment| assignment.span.end)
            .or_else(|| args.last().map(|word| word.span.end))
            .unwrap_or_default();
        let assigns = assignments
            .iter()
            .map(|assignment| self.encode_assignment(assignment).value)
            .collect::<Vec<_>>();
        let args = args
            .iter()
            .map(|word| self.encode_word(word).value)
            .collect::<Vec<_>>();
        let mut map = self.node_object(Some("CallExpr"), pos, end);
        self.insert_array(&mut map, "Assigns", assigns);
        self.insert_array(&mut map, "Args", args);
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn encode_function(&self, function: &FunctionDef) -> EncodedNode {
        let body_stmt = self.encode_stmt_without_separator(&function.body);
        let mut map = self.node_object(Some("FuncDecl"), function.span.start, function.span.end);
        self.insert_pos(&mut map, "Position", function.span.start);
        if let Some((rsrv_word, parens, _name_pos)) =
            self.function_surface(&function.name, function.span, body_stmt.pos)
        {
            self.insert_bool(&mut map, "RsrvWord", rsrv_word);
            self.insert_bool(&mut map, "Parens", parens);
            self.insert_value(
                &mut map,
                "Name",
                Some(
                    self.value_lit_node(
                        &function.name,
                        function.name_span.start,
                        function.name_span.end,
                    )
                    .value,
                ),
            );
        } else {
            self.insert_bool(&mut map, "Parens", true);
            self.insert_value(
                &mut map,
                "Name",
                Some(
                    self.value_lit_node(
                        &function.name,
                        function.name_span.start,
                        function.name_span.end,
                    )
                    .value,
                ),
            );
        }
        self.insert_value(&mut map, "Body", Some(body_stmt.value));
        EncodedNode {
            value: Value::Object(map),
            pos: function.span.start,
            end: function.span.end,
        }
    }

    fn encode_compound_command(&self, compound: &CompoundCommand) -> EncodedNode {
        match compound {
            CompoundCommand::If(command) => self.encode_if(command),
            CompoundCommand::For(command) => self.encode_for(command),
            CompoundCommand::ArithmeticFor(command) => self.encode_arithmetic_for(command),
            CompoundCommand::While(command) => self.encode_while(command, false),
            CompoundCommand::Until(command) => self.encode_until(command),
            CompoundCommand::Case(command) => self.encode_case(command),
            CompoundCommand::Select(command) => self.encode_select(command),
            CompoundCommand::BraceGroup(commands) => self.encode_block(commands),
            CompoundCommand::Subshell(commands) => self.encode_subshell(commands),
            CompoundCommand::Arithmetic(command) => self.encode_arithm_cmd(command),
            CompoundCommand::Conditional(command) => self.encode_test_clause(command),
            CompoundCommand::Time(command) => self.encode_time(command),
            CompoundCommand::Coproc(command) => self.encode_coproc(command),
        }
    }

    fn encode_if(&self, command: &IfCommand) -> EncodedNode {
        let fi_pos = self
            .rfind_keyword(command.span, "fi")
            .unwrap_or(command.span.end);
        self.encode_if_clause_chain(
            true,
            command.span,
            "if",
            self.find_keyword(command.span, "if")
                .unwrap_or(command.span.start),
            command.condition.as_slice(),
            command.then_branch.as_slice(),
            command.elif_branches.as_slice(),
            command.else_branch.as_deref(),
            fi_pos,
        )
    }

    fn encode_if_clause_chain(
        &self,
        typed: bool,
        span: Span,
        kind: &str,
        position: Position,
        condition: &[Command],
        then_branch: &[Command],
        elif_branches: &[(Vec<Command>, Vec<Command>)],
        else_branch: Option<&[Command]>,
        fi_pos: Position,
    ) -> EncodedNode {
        let then_pos = if kind == "else" {
            Position::default()
        } else {
            self.find_keyword_after(span, "then", position.offset)
                .unwrap_or_default()
        };
        let mut map = self.node_object(typed.then_some("IfClause"), position, span.end);
        self.insert_pos(&mut map, "Position", position);
        self.insert_string(&mut map, "Kind", kind);
        self.insert_pos(&mut map, "ThenPos", then_pos);
        self.insert_pos(&mut map, "FiPos", fi_pos);
        self.insert_array(
            &mut map,
            "Cond",
            self.encode_stmt_values_before(condition, then_pos, ";"),
        );
        self.insert_array(&mut map, "Then", self.encode_stmt_values(then_branch));

        let else_node = if let Some(((elif_cond, elif_then), rest)) = elif_branches.split_first() {
            let elif_pos = self
                .find_keyword_after(span, "elif", then_pos.offset)
                .unwrap_or_default();
            Some(
                self.encode_if_clause_chain(
                    false,
                    span,
                    "elif",
                    elif_pos,
                    elif_cond,
                    elif_then,
                    rest,
                    else_branch,
                    fi_pos,
                )
                .value,
            )
        } else if let Some(else_branch) = else_branch {
            let else_pos = self
                .find_keyword_after(span, "else", then_pos.offset)
                .unwrap_or_default();
            Some(
                self.encode_if_clause_chain(
                    false,
                    span,
                    "else",
                    else_pos,
                    &[],
                    else_branch,
                    &[],
                    None,
                    fi_pos,
                )
                .value,
            )
        } else {
            None
        };

        self.insert_value(&mut map, "Else", else_node);
        EncodedNode {
            value: Value::Object(map),
            pos: position,
            end: span.end,
        }
    }

    fn encode_for(&self, command: &ForCommand) -> EncodedNode {
        let for_pos = self
            .find_keyword(command.span, "for")
            .unwrap_or(command.span.start);
        let do_pos = self
            .find_keyword_after(command.span, "do", for_pos.offset)
            .unwrap_or_default();
        let done_pos = self
            .rfind_keyword(command.span, "done")
            .unwrap_or(command.span.end);
        let loop_end = command
            .words
            .as_ref()
            .and_then(|words| words.last())
            .map(|word| word.span.end)
            .unwrap_or(command.variable_span.end);

        let mut map = self.node_object(Some("ForClause"), for_pos, command.span.end);
        self.insert_pos(&mut map, "ForPos", for_pos);
        self.insert_pos(&mut map, "DoPos", do_pos);
        self.insert_pos(&mut map, "DonePos", done_pos);
        self.insert_value(
            &mut map,
            "Loop",
            Some(
                self.encode_word_iter(
                    &command.variable,
                    command.variable_span,
                    self.find_keyword_after(command.span, "in", command.variable_span.start.offset),
                    command.words.as_deref().unwrap_or(&[]),
                    loop_end,
                )
                .value,
            ),
        );
        self.insert_array(&mut map, "Do", self.encode_stmt_values(&command.body));
        EncodedNode {
            value: Value::Object(map),
            pos: for_pos,
            end: command.span.end,
        }
    }

    fn encode_select(&self, command: &SelectCommand) -> EncodedNode {
        let select_pos = self
            .find_keyword(command.span, "select")
            .unwrap_or(command.span.start);
        let do_pos = self
            .find_keyword_after(command.span, "do", select_pos.offset)
            .unwrap_or_default();
        let done_pos = self
            .rfind_keyword(command.span, "done")
            .unwrap_or(command.span.end);
        let loop_end = command
            .words
            .last()
            .map(|word| word.span.end)
            .unwrap_or(command.variable_span.end);

        let mut map = self.node_object(Some("ForClause"), select_pos, command.span.end);
        self.insert_pos(&mut map, "ForPos", select_pos);
        self.insert_pos(&mut map, "DoPos", do_pos);
        self.insert_pos(&mut map, "DonePos", done_pos);
        self.insert_bool(&mut map, "Select", true);
        self.insert_value(
            &mut map,
            "Loop",
            Some(
                self.encode_word_iter(
                    &command.variable,
                    command.variable_span,
                    self.find_keyword_after(command.span, "in", command.variable_span.start.offset),
                    &command.words,
                    loop_end,
                )
                .value,
            ),
        );
        self.insert_array(&mut map, "Do", self.encode_stmt_values(&command.body));
        EncodedNode {
            value: Value::Object(map),
            pos: select_pos,
            end: command.span.end,
        }
    }

    fn encode_arithmetic_for(&self, command: &ArithmeticForCommand) -> EncodedNode {
        let for_pos = self
            .find_keyword(command.span, "for")
            .unwrap_or(command.span.start);
        let do_pos = self
            .find_keyword_after(command.span, "do", for_pos.offset)
            .unwrap_or_default();
        let done_pos = self
            .rfind_keyword(command.span, "done")
            .unwrap_or(command.span.end);
        let loop_node = self.encode_c_style_loop(command);

        let mut map = self.node_object(Some("ForClause"), for_pos, command.span.end);
        self.insert_pos(&mut map, "ForPos", for_pos);
        self.insert_pos(&mut map, "DoPos", do_pos);
        self.insert_pos(&mut map, "DonePos", done_pos);
        self.insert_value(&mut map, "Loop", Some(loop_node.value));
        self.insert_array(&mut map, "Do", self.encode_stmt_values(&command.body));
        EncodedNode {
            value: Value::Object(map),
            pos: for_pos,
            end: command.span.end,
        }
    }

    fn encode_while(&self, command: &WhileCommand, until: bool) -> EncodedNode {
        let span = command.span;
        let head = if until { "until" } else { "while" };
        let while_pos = self.find_keyword(span, head).unwrap_or(span.start);
        let do_pos = self
            .find_keyword_after(span, "do", while_pos.offset)
            .unwrap_or_default();
        let done_pos = self.rfind_keyword(span, "done").unwrap_or(span.end);

        let mut map = self.node_object(Some("WhileClause"), while_pos, span.end);
        self.insert_pos(&mut map, "WhilePos", while_pos);
        self.insert_pos(&mut map, "DoPos", do_pos);
        self.insert_pos(&mut map, "DonePos", done_pos);
        self.insert_bool(&mut map, "Until", until);
        self.insert_array(
            &mut map,
            "Cond",
            self.encode_stmt_values_before(&command.condition, do_pos, ";"),
        );
        self.insert_array(&mut map, "Do", self.encode_stmt_values(&command.body));
        EncodedNode {
            value: Value::Object(map),
            pos: while_pos,
            end: span.end,
        }
    }

    fn encode_until(&self, command: &UntilCommand) -> EncodedNode {
        let synthetic = WhileCommand {
            condition: command.condition.clone(),
            body: command.body.clone(),
            span: command.span,
        };
        self.encode_while(&synthetic, true)
    }

    fn encode_case(&self, command: &CaseCommand) -> EncodedNode {
        let case_pos = self
            .find_keyword(command.span, "case")
            .unwrap_or(command.span.start);
        let in_pos = self
            .find_keyword_after(command.span, "in", command.word.span.end.offset)
            .unwrap_or_default();
        let esac_pos = self
            .rfind_keyword(command.span, "esac")
            .unwrap_or(command.span.end);

        let mut map = self.node_object(Some("CaseClause"), case_pos, command.span.end);
        self.insert_pos(&mut map, "Case", case_pos);
        self.insert_pos(&mut map, "In", in_pos);
        self.insert_pos(&mut map, "Esac", esac_pos);
        self.insert_value(
            &mut map,
            "Word",
            Some(self.encode_word(&command.word).value),
        );
        self.insert_array(
            &mut map,
            "Items",
            self.encode_case_items(&command.cases, in_pos, esac_pos),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: case_pos,
            end: command.span.end,
        }
    }

    fn encode_case_items(
        &self,
        items: &[CaseItem],
        in_pos: Position,
        esac_pos: Position,
    ) -> Vec<Value> {
        let mut cursor = in_pos.offset.saturating_add("in".len());
        items
            .iter()
            .map(|item| {
                let encoded = self.encode_case_item(item, cursor, esac_pos.offset);
                cursor = encoded.end.offset;
                encoded.value
            })
            .collect()
    }

    fn encode_case_item(
        &self,
        item: &CaseItem,
        search_start: usize,
        search_end: usize,
    ) -> EncodedNode {
        let body_end = item
            .commands
            .last()
            .map(|command| self.command_span(command).end)
            .unwrap_or_default();
        let body_start = item
            .commands
            .first()
            .map(|command| self.command_span(command).start.offset)
            .unwrap_or(search_start);
        let separator_pos = self
            .rfind_operator_between(search_start, body_start.max(search_start), ")")
            .unwrap_or_default();
        let op_str = match item.terminator {
            CaseTerminator::Break => ";;",
            CaseTerminator::FallThrough => ";&",
            CaseTerminator::Continue => ";;&",
        };
        let op_search_start = separator_pos.offset;
        let op_pos = self
            .find_operator_between(op_search_start, search_end, op_str)
            .or_else(|| self.rfind_operator_between(search_start, search_end, op_str))
            .unwrap_or_default();
        let patterns = self.encode_case_patterns(item, self.pos_at(search_start), separator_pos);
        let pos = patterns.first().map(|pattern| pattern.pos).unwrap_or_default();
        let end = if self.is_valid_pos(op_pos) {
            op_pos.advanced_by(op_str)
        } else {
            body_end
        };

        let mut map = self.node_object(None, pos, end);
        self.insert_number(
            &mut map,
            "Op",
            self.case_operator_code(item.terminator.clone()),
        );
        self.insert_pos(&mut map, "OpPos", op_pos);
        self.insert_array(
            &mut map,
            "Patterns",
            patterns.into_iter().map(|pattern| pattern.value).collect(),
        );
        self.insert_array(&mut map, "Stmts", self.encode_stmt_values(&item.commands));
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn encode_block(&self, commands: &[Command]) -> EncodedNode {
        let start = commands
            .first()
            .map(|command| {
                self.search_backward_for_token(self.command_span(command).start.offset, "{")
            })
            .flatten()
            .unwrap_or_default();
        let end = commands
            .last()
            .map(|command| {
                self.search_forward_for_token(self.command_span(command).end.offset, "}")
            })
            .flatten()
            .map(|pos| pos.advanced_by("}"))
            .unwrap_or_default();

        let mut map = self.node_object(Some("Block"), start, end);
        self.insert_pos(&mut map, "Lbrace", start);
        if self.is_valid_pos(end) {
            self.insert_pos(
                &mut map,
                "Rbrace",
                self.pos_at(end.offset.saturating_sub(1)),
            );
        }
        self.insert_array(&mut map, "Stmts", self.encode_stmt_values(commands));
        EncodedNode {
            value: Value::Object(map),
            pos: start,
            end,
        }
    }

    fn encode_subshell(&self, commands: &[Command]) -> EncodedNode {
        let start = commands
            .first()
            .map(|command| {
                self.search_backward_for_token(self.command_span(command).start.offset, "(")
            })
            .flatten()
            .unwrap_or_default();
        let end = commands
            .last()
            .map(|command| {
                self.search_forward_for_token(self.command_span(command).end.offset, ")")
            })
            .flatten()
            .map(|pos| pos.advanced_by(")"))
            .unwrap_or_default();

        let mut map = self.node_object(Some("Subshell"), start, end);
        self.insert_pos(&mut map, "Lparen", start);
        if self.is_valid_pos(end) {
            self.insert_pos(
                &mut map,
                "Rparen",
                self.pos_at(end.offset.saturating_sub(1)),
            );
        }
        self.insert_array(&mut map, "Stmts", self.encode_stmt_values(commands));
        EncodedNode {
            value: Value::Object(map),
            pos: start,
            end,
        }
    }

    fn encode_arithm_cmd(&self, command: &ArithmeticCommand) -> EncodedNode {
        let mut map = self.node_object(Some("ArithmCmd"), command.span.start, command.span.end);
        self.insert_pos(&mut map, "Left", command.left_paren_span.start);
        self.insert_pos(&mut map, "Right", command.right_paren_span.start);
        let source = command
            .expr_span
            .and_then(|span| self.slice_span(span))
            .unwrap_or_default();
        self.insert_string(&mut map, "Source", source);
        if let Some(expr_span) = command.expr_span {
            self.insert_value(
                &mut map,
                "X",
                Some(self.synthetic_expression_word(source, expr_span).value),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: command.span.start,
            end: command.span.end,
        }
    }

    fn encode_test_clause(&self, command: &ConditionalCommand) -> EncodedNode {
        let mut map = self.node_object(Some("TestClause"), command.span.start, command.span.end);
        self.insert_pos(&mut map, "Left", command.left_bracket_span.start);
        self.insert_pos(&mut map, "Right", command.right_bracket_span.start);
        let cond = self.encode_cond_expr(&command.expression);
        self.insert_value(&mut map, "X", Some(cond.value));
        EncodedNode {
            value: Value::Object(map),
            pos: command.span.start,
            end: command.span.end,
        }
    }

    fn encode_time(&self, command: &TimeCommand) -> EncodedNode {
        let time_pos = self
            .find_keyword(command.span, "time")
            .unwrap_or(command.span.start);
        let mut map = self.node_object(Some("TimeClause"), time_pos, command.span.end);
        self.insert_pos(&mut map, "Time", time_pos);
        self.insert_bool(&mut map, "PosixFormat", command.posix_format);
        self.insert_value(
            &mut map,
            "Stmt",
            command
                .command
                .as_ref()
                .map(|inner| self.encode_stmt_without_separator(inner).value),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: time_pos,
            end: command.span.end,
        }
    }

    fn encode_coproc(&self, command: &CoprocCommand) -> EncodedNode {
        let coproc_pos = self
            .find_keyword(command.span, "coproc")
            .unwrap_or(command.span.start);
        let stmt = self.encode_stmt_without_separator(&command.body);
        let mut map = self.node_object(Some("CoprocClause"), coproc_pos, command.span.end);
        self.insert_pos(&mut map, "Coproc", coproc_pos);
        if let Some(name_word) = self.explicit_coproc_name(command, stmt.pos) {
            self.insert_value(&mut map, "Name", Some(name_word.value));
        }
        self.insert_value(&mut map, "Stmt", Some(stmt.value));
        EncodedNode {
            value: Value::Object(map),
            pos: coproc_pos,
            end: command.span.end,
        }
    }

    fn explicit_coproc_name(
        &self,
        command: &CoprocCommand,
        _body_pos: Position,
    ) -> Option<EncodedNode> {
        let name_span = command.name_span?;
        Some(self.synthetic_literal_word_node(&command.name, name_span))
    }

    fn encode_binary_cmd(
        &self,
        lhs: EncodedStmt,
        rhs: EncodedStmt,
        op_code: u64,
        op_pos: Option<Position>,
    ) -> EncodedNode {
        let mut map = self.node_object(Some("BinaryCmd"), lhs.pos, rhs.end);
        self.insert_pos(&mut map, "OpPos", op_pos.unwrap_or_default());
        self.insert_number(&mut map, "Op", op_code);
        self.insert_value(&mut map, "X", Some(lhs.value));
        self.insert_value(&mut map, "Y", Some(rhs.value));
        EncodedNode {
            value: Value::Object(map),
            pos: lhs.pos,
            end: rhs.end,
        }
    }

    fn encode_word_iter(
        &self,
        variable: &str,
        name_span: Span,
        in_pos: Option<Position>,
        items: &[Word],
        end: Position,
    ) -> EncodedNode {
        let mut map = self.node_object(Some("WordIter"), name_span.start, end);
        self.insert_value(
            &mut map,
            "Name",
            Some(
                self.value_lit_node(variable, name_span.start, name_span.end)
                    .value,
            ),
        );
        self.insert_pos(&mut map, "InPos", in_pos.unwrap_or_default());
        self.insert_array(
            &mut map,
            "Items",
            items
                .iter()
                .map(|word| self.encode_word(word).value)
                .collect(),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: name_span.start,
            end,
        }
    }

    fn encode_c_style_loop(&self, command: &ArithmeticForCommand) -> EncodedNode {
        let mut map = self.node_object(
            Some("CStyleLoop"),
            command.left_paren_span.start,
            command.right_paren_span.end,
        );
        self.insert_pos(&mut map, "Lparen", command.left_paren_span.start);
        self.insert_pos(&mut map, "Rparen", command.right_paren_span.start);
        if let Some(init_span) = command.init_span {
            self.insert_value(
                &mut map,
                "Init",
                self.slice_span(init_span)
                    .map(|expr| self.synthetic_expression_word(expr, init_span).value),
            );
        }
        if let Some(condition_span) = command.condition_span {
            self.insert_value(
                &mut map,
                "Cond",
                self.slice_span(condition_span)
                    .map(|expr| self.synthetic_expression_word(expr, condition_span).value),
            );
        }
        if let Some(step_span) = command.step_span {
            self.insert_value(
                &mut map,
                "Post",
                self.slice_span(step_span)
                    .map(|expr| self.synthetic_expression_word(expr, step_span).value),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: command.left_paren_span.start,
            end: command.right_paren_span.end,
        }
    }

    fn encode_cond_expr(&self, expr: &ConditionalExpr) -> EncodedNode {
        match expr {
            ConditionalExpr::Binary(expr) => self.encode_cond_binary(expr),
            ConditionalExpr::Unary(expr) => self.encode_cond_unary(expr),
            ConditionalExpr::Parenthesized(expr) => self.encode_cond_paren(expr),
            ConditionalExpr::Word(word) => self.encode_cond_leaf("CondWord", "Word", word),
            ConditionalExpr::Pattern(word) => self.encode_cond_pattern(word),
            ConditionalExpr::Regex(word) => self.encode_cond_leaf("CondRegex", "Word", word),
        }
    }

    fn encode_cond_binary(&self, expr: &ConditionalBinaryExpr) -> EncodedNode {
        let left = self.encode_cond_expr(&expr.left);
        let right = self.encode_cond_expr(&expr.right);
        let mut map = self.node_object(Some("CondBinary"), left.pos, right.end);
        self.insert_pos(&mut map, "OpPos", expr.op_span.start);
        self.insert_number(&mut map, "Op", self.cond_binary_op_code(expr.op));
        self.insert_value(&mut map, "X", Some(left.value));
        self.insert_value(&mut map, "Y", Some(right.value));
        EncodedNode {
            value: Value::Object(map),
            pos: left.pos,
            end: right.end,
        }
    }

    fn encode_cond_unary(&self, expr: &ConditionalUnaryExpr) -> EncodedNode {
        let inner = self.encode_cond_expr(&expr.expr);
        let mut map = self.node_object(Some("CondUnary"), expr.op_span.start, inner.end);
        self.insert_pos(&mut map, "OpPos", expr.op_span.start);
        self.insert_number(&mut map, "Op", self.cond_unary_op_code(expr.op));
        self.insert_value(&mut map, "X", Some(inner.value));
        EncodedNode {
            value: Value::Object(map),
            pos: expr.op_span.start,
            end: inner.end,
        }
    }

    fn encode_cond_paren(&self, expr: &ConditionalParenExpr) -> EncodedNode {
        let inner = self.encode_cond_expr(&expr.expr);
        let mut map = self.node_object(
            Some("CondParen"),
            expr.left_paren_span.start,
            expr.right_paren_span.end,
        );
        self.insert_pos(&mut map, "Lparen", expr.left_paren_span.start);
        self.insert_pos(&mut map, "Rparen", expr.right_paren_span.start);
        self.insert_value(&mut map, "X", Some(inner.value));
        EncodedNode {
            value: Value::Object(map),
            pos: expr.left_paren_span.start,
            end: expr.right_paren_span.end,
        }
    }

    fn encode_cond_leaf(&self, ty: &str, field: &str, word: &Word) -> EncodedNode {
        let encoded = self.encode_word(word);
        let mut map = self.node_object(Some(ty), encoded.pos, encoded.end);
        self.insert_value(&mut map, field, Some(encoded.value));
        EncodedNode {
            value: Value::Object(map),
            pos: encoded.pos,
            end: encoded.end,
        }
    }

    fn encode_cond_pattern(&self, word: &Word) -> EncodedNode {
        let encoded = self.encode_pattern_word(word, None);
        let mut map = self.node_object(Some("CondPattern"), encoded.pos, encoded.end);
        self.insert_value(&mut map, "Pattern", Some(encoded.value));
        EncodedNode {
            value: Value::Object(map),
            pos: encoded.pos,
            end: encoded.end,
        }
    }

    fn encode_assignment(&self, assignment: &Assignment) -> EncodedNode {
        let mut map = self.node_object(None, assignment.span.start, assignment.span.end);
        self.insert_bool(&mut map, "Append", assignment.append);
        self.insert_value(
            &mut map,
            "Ref",
            Some(self.encode_var_ref_from_assignment(assignment).value),
        );
        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.insert_value(&mut map, "Value", Some(self.encode_word(word).value));
            }
            AssignmentValue::Array(words) => {
                self.insert_value(
                    &mut map,
                    "Array",
                    Some(self.encode_array_expr(words, assignment.span).value),
                );
            }
        }
        self.insert_value(
            &mut map,
            "Surface",
            self.assignment_surface(assignment).map(Value::Object),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: assignment.span.start,
            end: assignment.span.end,
        }
    }

    fn encode_decl_operand(&self, operand: &DeclOperand) -> EncodedNode {
        match operand {
            DeclOperand::Flag(word) => {
                let encoded_word = self.encode_word(word);
                let mut map =
                    self.node_object(Some("DeclFlag"), encoded_word.pos, encoded_word.end);
                self.insert_value(&mut map, "Word", Some(encoded_word.value));
                EncodedNode {
                    value: Value::Object(map),
                    pos: encoded_word.pos,
                    end: encoded_word.end,
                }
            }
            DeclOperand::Name(name) => {
                let encoded_ref = self.encode_decl_name_ref(name);
                let mut map = self.node_object(Some("DeclName"), name.span.start, name.span.end);
                self.insert_value(&mut map, "Ref", Some(encoded_ref.value));
                EncodedNode {
                    value: Value::Object(map),
                    pos: name.span.start,
                    end: name.span.end,
                }
            }
            DeclOperand::Assignment(assignment) => {
                let encoded_assignment = self.encode_assignment(assignment);
                let mut map = self.node_object(
                    Some("DeclAssign"),
                    assignment.span.start,
                    assignment.span.end,
                );
                self.insert_value(&mut map, "Assign", Some(encoded_assignment.value));
                EncodedNode {
                    value: Value::Object(map),
                    pos: assignment.span.start,
                    end: assignment.span.end,
                }
            }
            DeclOperand::Dynamic(word) => {
                let encoded_word = self.encode_word(word);
                let mut map =
                    self.node_object(Some("DeclDynamicWord"), encoded_word.pos, encoded_word.end);
                self.insert_value(&mut map, "Word", Some(encoded_word.value));
                EncodedNode {
                    value: Value::Object(map),
                    pos: encoded_word.pos,
                    end: encoded_word.end,
                }
            }
        }
    }

    fn encode_var_ref_from_assignment(&self, assignment: &Assignment) -> EncodedNode {
        self.encode_var_ref(
            &assignment.name,
            assignment.name_span,
            assignment.index.as_ref(),
            self.var_ref_end(assignment.name_span, assignment.index.as_ref()),
        )
    }

    fn encode_decl_name_ref(&self, name: &DeclName) -> EncodedNode {
        self.encode_var_ref(
            &name.name,
            name.name_span,
            name.index.as_ref(),
            self.var_ref_end(name.name_span, name.index.as_ref()),
        )
    }

    fn encode_var_ref(
        &self,
        name: &str,
        name_span: Span,
        index: Option<&SourceText>,
        end: Position,
    ) -> EncodedNode {
        let mut map = self.node_object(None, name_span.start, end);
        self.insert_value(
            &mut map,
            "Name",
            Some(
                self.value_lit_node(name, name_span.start, name_span.end)
                    .value,
            ),
        );
        if let Some(index) = index {
            self.insert_value(
                &mut map,
                "Index",
                Some(self.encode_subscript_text(index).value),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: name_span.start,
            end,
        }
    }

    fn encode_array_expr(&self, words: &[Word], span: Span) -> EncodedNode {
        let lparen = self
            .find_operator_between(span.start.offset, span.end.offset, "(")
            .unwrap_or_default();
        let rparen = self
            .rfind_operator_between(span.start.offset, span.end.offset, ")")
            .unwrap_or_default();
        let mut map = self.node_object(None, lparen, span.end);
        self.insert_pos(&mut map, "Lparen", lparen);
        self.insert_pos(&mut map, "Rparen", rparen);
        self.insert_array(
            &mut map,
            "Elems",
            words
                .iter()
                .map(|word| self.encode_array_elem(word).value)
                .collect(),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: lparen,
            end: span.end,
        }
    }

    fn encode_array_elem(&self, word: &Word) -> EncodedNode {
        let mut map = self.node_object(None, word.span.start, word.span.end);
        self.insert_value(&mut map, "Value", Some(self.encode_word(word).value));
        EncodedNode {
            value: Value::Object(map),
            pos: word.span.start,
            end: word.span.end,
        }
    }

    fn encode_redirects(&self, redirects: &[Redirect]) -> Vec<Value> {
        redirects
            .iter()
            .map(|redirect| self.encode_redirect(redirect).value)
            .collect()
    }

    fn encode_redirect(&self, redirect: &Redirect) -> EncodedNode {
        let op_text = self.redirect_operator_text(redirect.kind);
        let op_pos = self
            .find_operator_between(
                redirect.span.start.offset,
                redirect.span.end.offset,
                op_text,
            )
            .unwrap_or(redirect.span.start);
        let mut map = self.node_object(None, redirect.span.start, redirect.span.end);
        self.insert_pos(&mut map, "OpPos", op_pos);
        self.insert_number(&mut map, "Op", self.redirect_operator_code(redirect.kind));
        if let Some(n) = self.encode_redirect_n(redirect, op_pos) {
            self.insert_value(&mut map, "N", Some(n.value));
        }
        match redirect.kind {
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => {
                self.insert_value(
                    &mut map,
                    "Hdoc",
                    Some(self.encode_word(&redirect.target).value),
                );
            }
            _ => {
                self.insert_value(
                    &mut map,
                    "Word",
                    Some(self.encode_redirect_target_word(redirect, op_pos).value),
                );
            }
        }
        EncodedNode {
            value: Value::Object(map),
            pos: redirect.span.start,
            end: redirect.span.end,
        }
    }

    fn encode_redirect_n(&self, redirect: &Redirect, op_pos: Position) -> Option<EncodedNode> {
        if let Some(fd_var) = &redirect.fd_var {
            if let Some(span) = redirect.fd_var_span {
                return Some(self.value_lit_node(fd_var, span.start, span.end));
            }
            let start = redirect.span.start.advanced_by("{");
            let end = start.advanced_by(fd_var);
            return Some(self.value_lit_node(fd_var, start, end));
        }
        redirect.fd.map(|fd| {
            let text = fd.to_string();
            let start = self
                .find_operator_between(redirect.span.start.offset, op_pos.offset, &text)
                .unwrap_or(redirect.span.start);
            self.value_lit_node(&text, start, start.advanced_by(&text))
        })
    }

    fn encode_redirect_target_word(&self, redirect: &Redirect, op_pos: Position) -> EncodedNode {
        let encoded = self.encode_word(&redirect.target);
        if self.is_valid_pos(encoded.pos) && self.is_valid_pos(encoded.end) {
            return encoded;
        }

        let mut start_offset = op_pos
            .offset
            .saturating_add(self.redirect_operator_text(redirect.kind).len());
        while start_offset < redirect.span.end.offset {
            let Some(byte) = self.source.as_bytes().get(start_offset).copied() else {
                break;
            };
            if !byte.is_ascii_whitespace() {
                break;
            }
            start_offset += 1;
        }
        let start = self.pos_at(start_offset);
        let end = redirect.span.end;

        if redirect.target.parts.len() == 1
            && redirect
                .target
                .part_spans
                .first()
                .is_some_and(|span| !self.is_valid_pos(span.start))
            && let WordPart::Literal(value) = &redirect.target.parts[0]
        {
            return self.synthetic_literal_word_node(
                self.literal_value(value, Span::new()),
                Span::from_positions(start, end),
            );
        }

        let mut map = self.node_object(None, start, end);
        self.insert_array(
            &mut map,
            "Parts",
            redirect
                .target
                .parts_with_spans()
                .map(|(part, span)| self.encode_word_part(part, span).value)
                .collect(),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: start,
            end,
        }
    }

    fn encode_word(&self, word: &Word) -> EncodedNode {
        let pos = if self.is_valid_pos(word.span.start) {
            word.span.start
        } else {
            word.part_spans
                .iter()
                .find(|span| self.is_valid_pos(span.start))
                .map(|span| span.start)
                .unwrap_or_default()
        };
        let end = if self.is_valid_pos(word.span.end) {
            word.span.end
        } else {
            word.part_spans
                .iter()
                .rev()
                .find(|span| self.is_valid_pos(span.end))
                .map(|span| span.end)
                .unwrap_or_default()
        };
        let mut map = self.node_object(None, pos, end);
        if let Some(leading_escape) = self.leading_escape(word, pos, end) {
            self.insert_value(&mut map, "LeadingEscape", Some(leading_escape));
        }
        let parts = if let Some(wrapper) = self.quoted_wrapper(word) {
            vec![self.encode_quoted_wrapper(word, wrapper).value]
        } else if let Some(literal) = self.leading_escaped_literal(word, pos, end) {
            vec![literal.value]
        } else {
            word.parts_with_spans()
                .map(|(part, span)| self.encode_word_part(part, span).value)
                .collect::<Vec<_>>()
        };
        self.insert_array(&mut map, "Parts", parts);
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn encode_word_part(&self, part: &WordPart, span: Span) -> EncodedNode {
        match part {
            WordPart::Literal(value) => {
                self.lit_node(self.literal_value(value, span), span.start, span.end)
            }
            WordPart::Variable(name) => self.encode_simple_param_exp(name, span),
            WordPart::Length(name) => self.encode_length_param_exp(name, span),
            WordPart::ParameterExpansion {
                name,
                operator,
                operand,
                colon_variant,
            } => self.encode_parameter_expansion(name, operator, operand.as_ref(), *colon_variant, span),
            WordPart::ArrayAccess { name, index } => self.encode_array_access(name, index, span),
            WordPart::ArrayLength(name) => self.encode_array_length(name, span),
            WordPart::ArrayIndices(name) => self.encode_array_indices(name, span),
            WordPart::Substring {
                name,
                offset,
                length,
            } => self.encode_substring(name, offset, length.as_ref(), span),
            WordPart::ArraySlice {
                name,
                offset,
                length,
            } => self.encode_array_slice(name, offset, length.as_ref(), span),
            WordPart::IndirectExpansion {
                name,
                operator,
                operand,
                colon_variant,
            } => self.encode_indirect_expansion(
                name,
                operator.clone(),
                operand.as_ref(),
                *colon_variant,
                span,
            ),
            WordPart::PrefixMatch(prefix) => self.encode_prefix_match(prefix, span),
            WordPart::Transformation { name, operator } => {
                self.encode_transformation(name, *operator, span)
            }
            WordPart::CommandSubstitution(commands) => self.encode_cmd_subst(commands, span),
            WordPart::ArithmeticExpansion(expression) => self.encode_arithm_exp(expression, span),
            WordPart::ProcessSubstitution { commands, is_input } => {
                self.encode_proc_subst(commands, *is_input, span)
            }
        }
    }

    fn encode_quoted_wrapper(&self, word: &Word, wrapper: QuoteWrapper) -> EncodedNode {
        match wrapper {
            QuoteWrapper::Single { dollar } => {
                let value = word
                    .parts_with_spans()
                    .filter_map(|(part, span)| match part {
                        WordPart::Literal(value) => Some(self.literal_value(value, span)),
                        _ => None,
                    })
                    .collect::<String>();
                let right = self.pos_at(word.span.end.offset.saturating_sub(1));
                let mut map = self.node_object(Some("SglQuoted"), word.span.start, word.span.end);
                self.insert_pos(&mut map, "Left", word.span.start);
                self.insert_pos(&mut map, "Right", right);
                self.insert_bool(&mut map, "Dollar", dollar);
                self.insert_string(&mut map, "Value", &value);
                EncodedNode {
                    value: Value::Object(map),
                    pos: word.span.start,
                    end: word.span.end,
                }
            }
            QuoteWrapper::Double { dollar } => {
                let right = self.pos_at(word.span.end.offset.saturating_sub(1));
                let mut map = self.node_object(Some("DblQuoted"), word.span.start, word.span.end);
                self.insert_pos(&mut map, "Left", word.span.start);
                self.insert_pos(&mut map, "Right", right);
                self.insert_bool(&mut map, "Dollar", dollar);
                let parts = if word.parts.len() == 1
                    && word
                        .parts_with_spans()
                        .next()
                        .is_some_and(|(part, span)| {
                            matches!(part, WordPart::Literal(value) if self.literal_value(value, span).is_empty())
                        }) {
                    Vec::new()
                } else {
                    word.parts_with_spans()
                        .filter_map(|(part, span)| {
                            self.encode_quoted_part(word, part, span, dollar)
                                .map(|encoded| encoded.value)
                        })
                        .collect()
                };
                self.insert_array(&mut map, "Parts", parts);
                EncodedNode {
                    value: Value::Object(map),
                    pos: word.span.start,
                    end: word.span.end,
                }
            }
        }
    }

    fn encode_simple_param_exp(&self, name: &str, span: Span) -> EncodedNode {
        let raw = self.slice_span(span).unwrap_or_default();
        let short = !raw.starts_with("${");
        let dollar = span.start;
        let rbrace = self.param_rbrace(span);
        let param_pos = self
            .find_in_span(span, name, if short { 1 } else { 2 })
            .unwrap_or(span.start.advanced_by("$"));
        let param = self.value_lit_node(name, param_pos, param_pos.advanced_by(name));

        let mut map = self.node_object(Some("ParamExp"), span.start, span.end);
        self.insert_pos(&mut map, "Dollar", dollar);
        self.insert_pos(&mut map, "Rbrace", rbrace);
        self.insert_bool(&mut map, "Short", short);
        self.insert_value(&mut map, "Param", Some(param.value));
        EncodedNode {
            value: Value::Object(map),
            pos: span.start,
            end: span.end,
        }
    }

    fn encode_length_param_exp(&self, name: &str, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_bool(map, "Length", true);
        node
    }

    fn encode_parameter_expansion(
        &self,
        name: &str,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        colon_variant: bool,
        span: Span,
    ) -> EncodedNode {
        let raw = self.slice_span(span).unwrap_or_default();
        let short = !raw.starts_with("${");
        let dollar = span.start;
        let rbrace = self.param_rbrace(span);
        let param_pos = self
            .find_in_span(span, name, if short { 1 } else { 2 })
            .unwrap_or(span.start.advanced_by("$"));
        let param = self.value_lit_node(name, param_pos, param_pos.advanced_by(name));

        let mut map = self.node_object(Some("ParamExp"), span.start, span.end);
        self.insert_pos(&mut map, "Dollar", dollar);
        self.insert_pos(&mut map, "Rbrace", rbrace);
        self.insert_bool(&mut map, "Short", short);
        self.insert_value(&mut map, "Param", Some(param.value));

        match operator {
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
            } => {
                let mut repl = Map::new();
                self.insert_bool(
                    &mut repl,
                    "All",
                    matches!(operator, ParameterOp::ReplaceAll { .. }),
                );
                self.insert_value(
                    &mut repl,
                    "Orig",
                    Some(self.encode_pattern_source_text(pattern).value),
                );
                self.insert_value(
                    &mut repl,
                    "With",
                    Some(self.literal_word_from_source_text(replacement).value),
                );
                self.insert_value(&mut map, "Repl", Some(Value::Object(repl)));
            }
            ParameterOp::RemovePrefixShort
            | ParameterOp::RemovePrefixLong
            | ParameterOp::RemoveSuffixShort
            | ParameterOp::RemoveSuffixLong => {
                let mut exp = Map::new();
                self.insert_number(
                    &mut exp,
                    "Op",
                    self.parameter_operator_code(operator, colon_variant),
                );
                self.insert_value(
                    &mut exp,
                    "Pattern",
                    operand.map(|operand| self.encode_pattern_source_text(operand).value),
                );
                self.insert_value(&mut map, "Exp", Some(Value::Object(exp)));
            }
            _ => {
                let mut exp = Map::new();
                self.insert_number(
                    &mut exp,
                    "Op",
                    self.parameter_operator_code(operator, colon_variant),
                );
                if let Some(operand) = operand {
                    self.insert_value(
                        &mut exp,
                        "Word",
                        Some(self.literal_word_from_source_text(operand).value),
                    );
                }
                self.insert_value(&mut map, "Exp", Some(Value::Object(exp)));
            }
        }

        EncodedNode {
            value: Value::Object(map),
            pos: span.start,
            end: span.end,
        }
    }

    fn encode_array_access(&self, name: &str, index: &SourceText, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_value(
            map,
            "Index",
            Some(self.encode_subscript_text(index).value),
        );
        node
    }

    fn encode_array_length(&self, name: &str, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_bool(map, "Length", true);
        self.insert_value(
            map,
            "Index",
            Some(self.encode_all_elements_subscript(span, true).value),
        );
        node
    }

    fn encode_array_indices(&self, name: &str, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_bool(map, "Excl", true);
        self.insert_value(
            map,
            "Index",
            Some(self.encode_all_elements_subscript(span, true).value),
        );
        node
    }

    fn encode_substring(
        &self,
        name: &str,
        offset: &SourceText,
        length: Option<&SourceText>,
        span: Span,
    ) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        let mut slice = Map::new();
        self.insert_value(
            &mut slice,
            "Offset",
            Some(self.expression_word_from_source_text(offset).value),
        );
        self.insert_value(
            &mut slice,
            "Length",
            length.map(|length| self.expression_word_from_source_text(length).value),
        );
        self.insert_value(map, "Slice", Some(Value::Object(slice)));
        node
    }

    fn encode_array_slice(
        &self,
        name: &str,
        offset: &SourceText,
        length: Option<&SourceText>,
        span: Span,
    ) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_value(
            map,
            "Index",
            Some(self.encode_all_elements_subscript(span, true).value),
        );
        let mut slice = Map::new();
        self.insert_value(
            &mut slice,
            "Offset",
            Some(self.expression_word_from_source_text(offset).value),
        );
        self.insert_value(
            &mut slice,
            "Length",
            length.map(|length| self.expression_word_from_source_text(length).value),
        );
        self.insert_value(map, "Slice", Some(Value::Object(slice)));
        node
    }

    fn encode_indirect_expansion(
        &self,
        name: &str,
        operator: Option<ParameterOp>,
        operand: Option<&SourceText>,
        colon_variant: bool,
        span: Span,
    ) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_bool(map, "Excl", true);
        if let Some(operator) = operator {
            let mut exp = Map::new();
            self.insert_number(
                &mut exp,
                "Op",
                self.parameter_operator_code(&operator, colon_variant),
            );
            if let Some(operand) = operand {
                self.insert_value(
                    &mut exp,
                    "Word",
                    Some(self.literal_word_from_source_text(operand).value),
                );
            }
            self.insert_value(map, "Exp", Some(Value::Object(exp)));
        }
        node
    }

    fn encode_prefix_match(&self, prefix: &str, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(prefix, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_bool(map, "Excl", true);
        self.insert_number(map, "Names", 43);
        node
    }

    fn encode_transformation(&self, name: &str, operator: char, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        let mut exp = Map::new();
        self.insert_number(&mut exp, "Op", 100);
        self.insert_value(
            &mut exp,
            "Word",
            Some(self.literal_word_in_span(&operator.to_string(), span).value),
        );
        self.insert_value(map, "Exp", Some(Value::Object(exp)));
        node
    }

    fn encode_cmd_subst(&self, commands: &[Command], span: Span) -> EncodedNode {
        let right = self.pos_at(span.end.offset.saturating_sub(1));
        let mut map = self.node_object(Some("CmdSubst"), span.start, span.end);
        self.insert_pos(&mut map, "Left", span.start);
        self.insert_pos(&mut map, "Right", right);
        self.insert_array(&mut map, "Stmts", self.encode_stmt_values(commands));
        self.insert_pos(&mut map, "DiagnosticEnd", self.diagnostic_end(span.end));
        EncodedNode {
            value: Value::Object(map),
            pos: span.start,
            end: span.end,
        }
    }

    fn encode_arithm_exp(&self, expression: &SourceText, span: Span) -> EncodedNode {
        let expression = self.source_text_value(expression);
        let right = if span.end.offset >= 2 {
            self.pos_at(span.end.offset.saturating_sub(2))
        } else {
            Position::default()
        };
        let expr_start = if span.start.offset + 3 <= span.end.offset {
            self.pos_at(span.start.offset + 3)
        } else {
            Position::default()
        };
        let expr_end = if span.end.offset >= 2 {
            self.pos_at(span.end.offset.saturating_sub(2))
        } else {
            Position::default()
        };
        let mut map = self.node_object(Some("ArithmExp"), span.start, span.end);
        self.insert_pos(&mut map, "Left", span.start);
        self.insert_pos(&mut map, "Right", right);
        self.insert_string(&mut map, "Source", expression);
        self.insert_value(
            &mut map,
            "X",
            Some(
                self.synthetic_expression_word(
                    expression,
                    Span::from_positions(expr_start, expr_end),
                )
                .value,
            ),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: span.start,
            end: span.end,
        }
    }

    fn encode_proc_subst(&self, commands: &[Command], is_input: bool, span: Span) -> EncodedNode {
        let right = self.pos_at(span.end.offset.saturating_sub(1));
        let mut map = self.node_object(Some("ProcSubst"), span.start, span.end);
        self.insert_pos(&mut map, "OpPos", span.start);
        self.insert_pos(&mut map, "Rparen", right);
        self.insert_number(&mut map, "Op", if is_input { 78 } else { 80 });
        self.insert_array(&mut map, "Stmts", self.encode_stmt_values(commands));
        EncodedNode {
            value: Value::Object(map),
            pos: span.start,
            end: span.end,
        }
    }

    fn encode_subscript(&self, index: &str, base: Position) -> EncodedNode {
        let left = base.advanced_by("[");
        let right = left.advanced_by(index);
        let mut map = self.node_object(None, left, right.advanced_by("]"));
        self.insert_pos(&mut map, "Left", left);
        self.insert_pos(&mut map, "Right", right);
        if index == "@" {
            self.insert_number(&mut map, "Kind", 1);
        } else if index == "*" {
            self.insert_number(&mut map, "Kind", 2);
        } else if !index.is_empty() {
            self.insert_value(
                &mut map,
                "Expr",
                Some(self.synthetic_expression_word(index, Span::new()).value),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: left,
            end: right.advanced_by("]"),
        }
    }

    fn encode_subscript_with_span(&self, index: &str, span: Span) -> EncodedNode {
        let left = self.pos_at(span.start.offset.saturating_sub(1));
        let right = span.end;
        let mut map = self.node_object(None, left, right.advanced_by("]"));
        self.insert_pos(&mut map, "Left", left);
        self.insert_pos(&mut map, "Right", right);
        if index == "@" {
            self.insert_number(&mut map, "Kind", 1);
        } else if index == "*" {
            self.insert_number(&mut map, "Kind", 2);
        } else if !index.is_empty() {
            self.insert_value(
                &mut map,
                "Expr",
                Some(self.synthetic_expression_word(index, span).value),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: left,
            end: right.advanced_by("]"),
        }
    }

    fn encode_all_elements_subscript(&self, span: Span, at: bool) -> EncodedNode {
        let raw = self.slice_span(span).unwrap_or_default();
        let needle = if at { "[@]" } else { "[*]" };
        if let Some(rel) = raw.find(needle) {
            let left = self.pos_at(span.start.offset + rel);
            let right = self.pos_at(span.start.offset + rel + 2);
            let mut map = self.node_object(None, left, right.advanced_by("]"));
            self.insert_pos(&mut map, "Left", left);
            self.insert_pos(&mut map, "Right", right);
            self.insert_number(&mut map, "Kind", if at { 1 } else { 2 });
            return EncodedNode {
                value: Value::Object(map),
                pos: left,
                end: right.advanced_by("]"),
            };
        }
        self.encode_subscript(if at { "@" } else { "*" }, span.start)
    }

    fn encode_pattern_literal(&self, pattern: &str) -> EncodedNode {
        self.encode_pattern_text(pattern, None)
    }

    fn encode_pattern_literal_in_span(&self, pattern: &str, span: Span) -> EncodedNode {
        let pattern_span = self
            .rfind_in_span(span, pattern)
            .map(|start| Span::from_positions(start, start.advanced_by(pattern)));
        self.encode_pattern_text(pattern, pattern_span)
    }

    fn encode_pattern_word(&self, word: &Word, fallback_span: Option<Span>) -> EncodedNode {
        let span = if self.is_valid_pos(word.span.start) && self.is_valid_pos(word.span.end) {
            Some(word.span)
        } else {
            fallback_span
        };
        let raw = span
            .and_then(|span| self.slice_span(span))
            .map(str::to_owned)
            .unwrap_or_else(|| word.to_string());
        self.encode_pattern_text_with_word(&raw, span, Some(word))
    }

    fn encode_pattern_text(&self, pattern: &str, span: Option<Span>) -> EncodedNode {
        self.encode_pattern_text_with_word(pattern, span, None)
    }

    fn encode_pattern_text_with_word(
        &self,
        pattern: &str,
        span: Option<Span>,
        word: Option<&Word>,
    ) -> EncodedNode {
        let (pos, end) = span
            .filter(|span| self.is_valid_pos(span.start) && self.is_valid_pos(span.end))
            .map(|span| (span.start, span.end))
            .unwrap_or_default();
        let parts = if let Some(span) = span {
            self.encode_pattern_parts_from_source(span, word)
        } else {
            vec![self.lit_node(pattern, Position::default(), Position::default()).value]
        };
        let mut map = self.node_object(None, pos, end);
        self.insert_pos(&mut map, "Start", pos);
        self.insert_pos(&mut map, "EndPos", end);
        self.insert_array(&mut map, "Parts", parts);
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn lit_node(&self, value: &str, start: Position, end: Position) -> EncodedNode {
        let mut map = self.node_object(Some("Lit"), start, end);
        self.insert_pos(&mut map, "ValuePos", start);
        self.insert_pos(&mut map, "ValueEnd", end);
        self.insert_string(&mut map, "Value", value);
        EncodedNode {
            value: Value::Object(map),
            pos: start,
            end,
        }
    }

    fn value_lit_node(&self, value: &str, start: Position, end: Position) -> EncodedNode {
        let mut map = self.node_object(None, start, end);
        self.insert_pos(&mut map, "ValuePos", start);
        self.insert_pos(&mut map, "ValueEnd", end);
        self.insert_string(&mut map, "Value", value);
        EncodedNode {
            value: Value::Object(map),
            pos: start,
            end,
        }
    }

    fn synthetic_literal_word(&self, value: &str, span: Span) -> Word {
        Word::literal_with_span(value, span)
    }

    fn synthetic_literal_word_node(&self, value: &str, span: Span) -> EncodedNode {
        let word = self.synthetic_literal_word(value, span);
        self.encode_word(&word)
    }

    fn typed_word_node(&self, word: &Word) -> EncodedNode {
        let encoded = self.encode_word(word);
        let mut map = self.node_object(Some("Word"), encoded.pos, encoded.end);
        let parts = encoded
            .value
            .get("Parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.insert_array(&mut map, "Parts", parts);
        EncodedNode {
            value: Value::Object(map),
            pos: encoded.pos,
            end: encoded.end,
        }
    }

    fn literal_value<'b>(&'b self, value: &'b LiteralText, span: Span) -> &'b str {
        value.as_str(self.source, span)
    }

    fn source_text_value<'b>(&'b self, value: &'b SourceText) -> &'b str {
        value.slice(self.source)
    }

    fn literal_word_from_source_text(&self, value: &SourceText) -> EncodedNode {
        self.synthetic_literal_word_node(self.source_text_value(value), value.span())
    }

    fn expression_word_from_source_text(&self, value: &SourceText) -> EncodedNode {
        self.synthetic_expression_word(self.source_text_value(value), value.span())
    }

    fn encode_subscript_text(&self, index: &SourceText) -> EncodedNode {
        self.encode_subscript_with_span(self.source_text_value(index), index.span())
    }

    fn encode_pattern_source_text(&self, pattern: &SourceText) -> EncodedNode {
        self.encode_pattern_text(self.source_text_value(pattern), Some(pattern.span()))
    }

    fn literal_word_in_span(&self, value: &str, span: Span) -> EncodedNode {
        let word_span = self
            .rfind_in_span(span, value)
            .map(|start| Span::from_positions(start, start.advanced_by(value)))
            .unwrap_or_else(Span::new);
        self.synthetic_literal_word_node(value, word_span)
    }

    fn expression_word_in_span(&self, value: &str, span: Span) -> EncodedNode {
        self.literal_word_in_span(value, span)
    }

    fn synthetic_expression_word(&self, value: &str, span: Span) -> EncodedNode {
        let span = if self.is_valid_pos(span.start) || self.is_valid_pos(span.end) {
            span
        } else {
            Span::from_positions(Position::default(), Position::default())
        };
        let word = self.synthetic_literal_word(value, span);
        self.typed_word_node(&word)
    }

    fn wrap_stmt(
        &self,
        position: Position,
        end: Position,
        cmd: EncodedNode,
        negated: bool,
        redirs: Vec<Value>,
        semicolon: Option<Position>,
        background: bool,
    ) -> EncodedStmt {
        let mut map = self.node_object(None, position, end);
        self.insert_value(&mut map, "Cmd", Some(cmd.value));
        self.insert_pos(&mut map, "Position", position);
        self.insert_pos(&mut map, "Semicolon", semicolon.unwrap_or_default());
        self.insert_bool(&mut map, "Negated", negated);
        self.insert_bool(&mut map, "Background", background);
        self.insert_array(&mut map, "Redirs", redirs);
        EncodedStmt {
            value: Value::Object(map),
            pos: position,
            end,
        }
    }

    fn extract_cmd_from_stmt(&self, value: &Value) -> Value {
        value
            .get("Cmd")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()))
    }

    fn command_span(&self, command: &Command) -> Span {
        match command {
            Command::Simple(command) => self.span_with_redirects(command.span, &command.redirects),
            Command::Builtin(command) => {
                self.span_with_redirects(self.builtin_span(command), self.builtin_redirects(command))
            }
            Command::Decl(command) => self.span_with_redirects(command.span, &command.redirects),
            Command::Pipeline(command) => command.span,
            Command::List(command) => command.span,
            Command::Compound(command, redirects) => self.compound_span(command, redirects),
            Command::Function(command) => command.span,
        }
    }

    fn decl_operand_end(&self, operand: &DeclOperand) -> Position {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span.end,
            DeclOperand::Name(name) => name.span.end,
            DeclOperand::Assignment(assignment) => assignment.span.end,
        }
    }

    fn builtin_span(&self, builtin: &BuiltinCommand) -> Span {
        match builtin {
            BuiltinCommand::Break(command) => command.span,
            BuiltinCommand::Continue(command) => command.span,
            BuiltinCommand::Return(command) => command.span,
            BuiltinCommand::Exit(command) => command.span,
        }
    }

    fn builtin_redirects<'b>(&self, builtin: &'b BuiltinCommand) -> &'b [Redirect] {
        match builtin {
            BuiltinCommand::Break(command) => &command.redirects,
            BuiltinCommand::Continue(command) => &command.redirects,
            BuiltinCommand::Return(command) => &command.redirects,
            BuiltinCommand::Exit(command) => &command.redirects,
        }
    }

    fn compound_span(&self, compound: &CompoundCommand, redirects: &[Redirect]) -> Span {
        let core = match compound {
            CompoundCommand::If(command) => command.span,
            CompoundCommand::For(command) => command.span,
            CompoundCommand::ArithmeticFor(command) => command.span,
            CompoundCommand::While(command) => command.span,
            CompoundCommand::Until(command) => command.span,
            CompoundCommand::Case(command) => command.span,
            CompoundCommand::Select(command) => command.span,
            CompoundCommand::Time(command) => command.span,
            CompoundCommand::Coproc(command) => command.span,
            CompoundCommand::BraceGroup(commands) | CompoundCommand::Subshell(commands) => {
                self.commands_span(commands)
            }
            CompoundCommand::Conditional(command) => command.span,
            CompoundCommand::Arithmetic(command) => command.span,
        };
        if let Some(last) = redirects.last() {
            if core == Span::new() {
                Span::from_positions(last.span.start, last.span.end)
            } else {
                core.merge(last.span)
            }
        } else {
            core
        }
    }

    fn commands_span(&self, commands: &[Command]) -> Span {
        let Some(first) = commands.first() else {
            return Span::new();
        };
        let Some(last) = commands.last() else {
            return Span::new();
        };
        Span::from_positions(self.command_span(first).start, self.command_span(last).end)
    }

    fn stmt_end(
        &self,
        base_end: Position,
        semicolon: Option<Position>,
        background: bool,
    ) -> Position {
        if let Some(semicolon) = semicolon {
            return semicolon.advanced_by(if background { "&" } else { ";" });
        }
        base_end
    }

    fn assignment_surface(&self, assignment: &Assignment) -> Option<Map<String, Value>> {
        let operator = if assignment.append { "+=" } else { "=" };
        let ref_end = self.var_ref_end(assignment.name_span, assignment.index.as_ref());
        let value_pos = match &assignment.value {
            AssignmentValue::Scalar(word) => word.span.start,
            AssignmentValue::Array(_) => self
                .find_operator_between(ref_end.offset, assignment.span.end.offset, "(")
                .unwrap_or_else(|| ref_end.advanced_by(operator)),
        };
        let operator_pos = self
            .find_operator_between(ref_end.offset, value_pos.offset, operator)
            .or(Some(ref_end))?;
        let operator_end = operator_pos.advanced_by(operator);

        let mut map = Map::new();
        self.insert_pos(&mut map, "OperatorPos", operator_pos);
        self.insert_pos(&mut map, "OperatorEnd", operator_end);
        self.insert_pos(&mut map, "ValuePos", value_pos);
        Some(map)
    }

    fn var_ref_end(&self, name_span: Span, index_span: Option<&SourceText>) -> Position {
        index_span
            .map(|span| span.span().end.advanced_by("]"))
            .unwrap_or(name_span.end)
    }

    fn last_redirect_end(&self, redirects: &[Redirect]) -> Position {
        redirects
            .last()
            .map(|redirect| redirect.span.end)
            .unwrap_or_default()
    }

    fn span_with_redirects(&self, span: Span, redirects: &[Redirect]) -> Span {
        if let Some(end) = redirects.last().map(|redirect| redirect.span.end) {
            span.merge(Span::from_positions(span.start, end))
        } else {
            span
        }
    }

    fn max_pos(&self, left: Position, right: Position) -> Position {
        if right.offset > left.offset {
            right
        } else {
            left
        }
    }

    fn function_surface(
        &self,
        name: &str,
        span: Span,
        body_pos: Position,
    ) -> Option<(bool, bool, Position)> {
        let text = self.slice_offsets(span.start.offset, body_pos.offset)?;
        if let Some(rel) = text.find("function") {
            let name_rel = text[rel + "function".len()..].find(name)? + rel + "function".len();
            let name_pos = self.pos_at(span.start.offset + name_rel);
            let parens = text[name_rel..].contains("()");
            return Some((true, parens, name_pos));
        }
        let name_rel = text.find(name)?;
        Some((false, true, self.pos_at(span.start.offset + name_rel)))
    }

    fn diagnostic_end(&self, end: Position) -> Position {
        self.pos_at((end.offset + 1).min(self.source.len()))
    }

    fn adjust_quoted_part_span(&self, word: &Word, span: Span, dollar: bool) -> Span {
        if word.quoted {
            return span;
        }

        let prefix_len = if dollar { 2 } else { 1 };
        let mut start = span.start;
        if start.offset == word.span.start.offset {
            start = self.pos_at(start.offset.saturating_add(prefix_len));
        }
        let mut end = span.end;
        if span.end.offset == word.span.end.offset.saturating_sub(2) {
            end = self.pos_at(end.offset.saturating_add(1));
        }
        Span::from_positions(start, end)
    }

    fn encode_case_patterns(
        &self,
        item: &CaseItem,
        search_start: Position,
        separator_pos: Position,
    ) -> Vec<EncodedNode> {
        let Some(head) = self.slice_offsets(search_start.offset, separator_pos.offset) else {
            return item
                .patterns
                .iter()
                .map(|word| self.encode_pattern_word(word, None))
                .collect();
        };
        let raw_patterns = self.raw_case_patterns(head);
        let mut rel_cursor = 0;
        let mut patterns = Vec::new();

        for (idx, pattern) in item.patterns.iter().enumerate() {
            let text = self.case_pattern_text(pattern);
            let text = if text.is_empty() {
                raw_patterns.get(idx).cloned().unwrap_or_default()
            } else {
                text
            };
            let fallback_span = head
                .get(rel_cursor..)
                .and_then(|rest| rest.find(&text))
                .map(|found| rel_cursor + found)
                .map(|found| {
                    let start = self.pos_at(search_start.offset + found);
                    Span::from_positions(start, start.advanced_by(&text))
                });
            if let Some(span) = fallback_span {
                rel_cursor = span.end.offset.saturating_sub(search_start.offset);
            }
            patterns.push(self.encode_pattern_word(pattern, fallback_span));
        }

        patterns
    }

    fn case_pattern_text(&self, word: &Word) -> String {
        self.slice_span(word.span)
            .map(str::to_owned)
            .unwrap_or_else(|| word.to_string())
    }

    fn raw_case_patterns(&self, head: &str) -> Vec<String> {
        head.trim()
            .trim_start_matches('(')
            .split('|')
            .map(str::trim)
            .filter(|pattern| !pattern.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn encode_quoted_part(
        &self,
        word: &Word,
        part: &WordPart,
        span: Span,
        dollar: bool,
    ) -> Option<EncodedNode> {
        let span = self.adjust_quoted_part_span(word, span, dollar);
        if span.start.offset == span.end.offset && matches!(part, WordPart::Literal(_)) {
            return None;
        }
        Some(match part {
            WordPart::Literal(_) => self.quoted_literal_node(span),
            _ => self.encode_word_part(part, span),
        })
    }

    fn quoted_literal_node(&self, span: Span) -> EncodedNode {
        let value = self.slice_span(span).unwrap_or_default();
        self.lit_node(value, span.start, span.end)
    }

    fn param_rbrace(&self, span: Span) -> Position {
        let raw = self.slice_span(span).unwrap_or_default();
        if !raw.starts_with("${") {
            return Position::default();
        }
        if raw.ends_with('}') {
            return self.pos_at(span.end.offset.saturating_sub(1));
        }
        self.find_operator_between(span.end.offset, self.source.len(), "}")
            .unwrap_or_default()
    }

    fn leading_escape(&self, _word: &Word, pos: Position, end: Position) -> Option<Value> {
        let raw = self.slice_offsets(pos.offset, end.offset)?;
        if !raw.starts_with('\\') {
            return None;
        }

        let mut map = Map::new();
        self.insert_pos(&mut map, "Pos", pos);
        self.insert_pos(&mut map, "End", pos.advanced_by("\\"));
        Some(Value::Object(map))
    }

    fn leading_escaped_literal(&self, word: &Word, pos: Position, end: Position) -> Option<EncodedNode> {
        let [WordPart::Literal(_)] = word.parts.as_slice() else {
            return None;
        };
        let raw = self.slice_offsets(pos.offset, end.offset)?;
        if raw.starts_with('\\') {
            return Some(self.lit_node(raw, pos, end));
        }
        None
    }

    fn encode_pattern_parts_from_source(&self, span: Span, word: Option<&Word>) -> Vec<Value> {
        let valid_parts = word
            .map(|word| {
                word.parts_with_spans()
                    .filter(|(_, part_span)| {
                        self.is_valid_pos(part_span.start)
                            && self.is_valid_pos(part_span.end)
                            && part_span.start.offset >= span.start.offset
                            && part_span.end.offset <= span.end.offset
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let mut values = Vec::new();
        let mut cursor = span.start.offset;
        let mut part_index = 0;

        while cursor < span.end.offset {
            while valid_parts
                .get(part_index)
                .is_some_and(|(_, part_span)| part_span.end.offset <= cursor)
            {
                part_index += 1;
            }

            if let Some((quote_span, consumed_parts)) =
                self.pattern_quote_at(span, &valid_parts[part_index..], cursor)
            {
                values.push(
                    self.encode_pattern_quote_node(
                        quote_span,
                        &valid_parts[part_index..part_index + consumed_parts],
                    )
                    .value,
                );
                cursor = quote_span.end.offset;
                part_index += consumed_parts;
                continue;
            }

            if let Some(byte) = self.source.as_bytes().get(cursor).copied() {
                if matches!(byte, b'*' | b'?') {
                    let pos = self.pos_at(cursor);
                    values.push(match byte {
                        b'*' => self.pattern_any_node(pos, pos.advanced_by("*")).value,
                        b'?' => self.pattern_single_node(pos, pos.advanced_by("?")).value,
                        _ => unreachable!(),
                    });
                    cursor += 1;
                    continue;
                }
            }

            if let Some((part, part_span)) = valid_parts.get(part_index) {
                if part_span.start.offset == cursor {
                    match part {
                        WordPart::Literal(_) => {
                            let literal_end =
                                self.find_pattern_boundary(cursor, part_span.end.offset);
                            if literal_end == cursor {
                                let pos = self.pos_at(cursor);
                                let end = self.pos_at(cursor + 1);
                                values.push(
                                    self.lit_node(&self.source[cursor..cursor + 1], pos, end)
                                        .value,
                                );
                                cursor += 1;
                                continue;
                            }
                            values.extend(
                                self.encode_pattern_literals_from_source(Span::from_positions(
                                    self.pos_at(cursor),
                                    self.pos_at(literal_end),
                                ))
                                .into_iter()
                                .map(|node| node.value),
                            );
                            cursor = literal_end;
                            continue;
                        }
                        _ => {
                            values.push(self.encode_word_part(part, *part_span).value);
                            cursor = part_span.end.offset;
                            part_index += 1;
                            continue;
                        }
                    }
                }

                if part_span.start.offset > cursor {
                    let literal_end = self.find_pattern_boundary(cursor, part_span.start.offset);
                    if literal_end == cursor {
                        let pos = self.pos_at(cursor);
                        let end = self.pos_at(cursor + 1);
                        values.push(self.lit_node(&self.source[cursor..cursor + 1], pos, end).value);
                        cursor += 1;
                        continue;
                    }
                    values.extend(
                        self.encode_pattern_literals_from_source(Span::from_positions(
                            self.pos_at(cursor),
                            self.pos_at(literal_end),
                        ))
                        .into_iter()
                        .map(|node| node.value),
                    );
                    cursor = literal_end;
                    continue;
                }
            }

            let literal_end = self.find_pattern_boundary(cursor, span.end.offset);
            if literal_end == cursor {
                let pos = self.pos_at(cursor);
                let end = self.pos_at(cursor + 1);
                values.push(self.lit_node(&self.source[cursor..cursor + 1], pos, end).value);
                cursor += 1;
                continue;
            }
            values.extend(
                self.encode_pattern_literals_from_source(Span::from_positions(
                    self.pos_at(cursor),
                    self.pos_at(literal_end),
                ))
                .into_iter()
                .map(|node| node.value),
            );
            cursor = literal_end;
        }

        values
    }

    fn encode_pattern_literals_from_source(&self, span: Span) -> Vec<EncodedNode> {
        let Some(raw) = self.slice_span(span) else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        let bytes = raw.as_bytes();
        let mut rel = 0;

        while rel < bytes.len() {
            let absolute = span.start.offset + rel;
            match bytes[rel] {
                b'*' => {
                    let pos = self.pos_at(absolute);
                    nodes.push(self.pattern_any_node(pos, pos.advanced_by("*")));
                    rel += 1;
                }
                b'?' => {
                    let pos = self.pos_at(absolute);
                    nodes.push(self.pattern_single_node(pos, pos.advanced_by("?")));
                    rel += 1;
                }
                _ => {
                    let start_rel = rel;
                    while rel < bytes.len() && !matches!(bytes[rel], b'*' | b'?') {
                        rel += 1;
                    }
                    let start = self.pos_at(span.start.offset + start_rel);
                    let end = self.pos_at(span.start.offset + rel);
                    nodes.push(self.lit_node(&raw[start_rel..rel], start, end));
                }
            }
        }

        nodes
    }

    fn pattern_any_node(&self, pos: Position, end: Position) -> EncodedNode {
        let mut map = self.node_object(Some("PatternAny"), pos, end);
        self.insert_pos(&mut map, "Asterisk", pos);
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn pattern_single_node(&self, pos: Position, end: Position) -> EncodedNode {
        let mut map = self.node_object(Some("PatternSingle"), pos, end);
        self.insert_pos(&mut map, "Question", pos);
        EncodedNode {
            value: Value::Object(map),
            pos,
            end,
        }
    }

    fn encode_pattern_quote_node(
        &self,
        quote_span: Span,
        parts: &[(&WordPart, Span)],
    ) -> EncodedNode {
        let raw = self.slice_span(quote_span).unwrap_or_default();
        let (quote_type, dollar, content_start) = if raw.starts_with("$\"") {
            ("DblQuoted", true, quote_span.start.offset + 2)
        } else if raw.starts_with("$'") {
            ("SglQuoted", true, quote_span.start.offset + 2)
        } else if raw.starts_with('"') {
            ("DblQuoted", false, quote_span.start.offset + 1)
        } else {
            ("SglQuoted", false, quote_span.start.offset + 1)
        };
        let content_end = quote_span.end.offset.saturating_sub(1);
        let right = self.pos_at(quote_span.end.offset.saturating_sub(1));
        let mut map = self.node_object(Some(quote_type), quote_span.start, quote_span.end);
        self.insert_pos(&mut map, "Left", quote_span.start);
        self.insert_pos(&mut map, "Right", right);
        self.insert_bool(&mut map, "Dollar", dollar);

        if quote_type == "SglQuoted" {
            self.insert_string(
                &mut map,
                "Value",
                self.slice_offsets(content_start, content_end).unwrap_or_default(),
            );
        } else {
            let inner_parts =
                self.encode_pattern_quoted_parts(content_start, content_end, parts);
            self.insert_array(&mut map, "Parts", inner_parts);
        }

        EncodedNode {
            value: Value::Object(map),
            pos: quote_span.start,
            end: quote_span.end,
        }
    }

    fn encode_pattern_quoted_parts(
        &self,
        content_start: usize,
        content_end: usize,
        parts: &[(&WordPart, Span)],
    ) -> Vec<Value> {
        let mut values = Vec::new();
        let mut cursor = content_start;
        let mut part_index = 0;

        while cursor < content_end {
            while parts
                .get(part_index)
                .is_some_and(|(_, span)| span.end.offset <= cursor)
            {
                part_index += 1;
            }

            if self.source.as_bytes().get(cursor).copied() == Some(b'$')
                && let Some((part, _)) = parts
                    .iter()
                    .skip(part_index)
                    .find(|(part, span)| !matches!(part, WordPart::Literal(_)) && span.end.offset > cursor)
            {
                let end = self.scan_dollar_expansion_end(cursor, content_end);
                values.push(
                    self.encode_word_part(
                        part,
                        Span::from_positions(self.pos_at(cursor), self.pos_at(end)),
                    )
                    .value,
                );
                cursor = end;
                part_index += 1;
                continue;
            }

            let next_dollar = self.slice_offsets(cursor, content_end)
                .and_then(|slice| slice.find('$'))
                .map(|rel| cursor + rel)
                .unwrap_or(content_end);
            if next_dollar > cursor {
                let start = self.pos_at(cursor);
                let end = self.pos_at(next_dollar);
                values.push(self.lit_node(&self.source[cursor..next_dollar], start, end).value);
                cursor = next_dollar;
                continue;
            }

            cursor += 1;
        }

        values
    }

    fn scan_dollar_expansion_end(&self, start: usize, limit: usize) -> usize {
        let bytes = self.source.as_bytes();
        if bytes.get(start) != Some(&b'$') {
            return (start + 1).min(limit);
        }
        match (bytes.get(start + 1).copied(), bytes.get(start + 2).copied()) {
            (Some(b'{'), _) => {
                let mut idx = start + 2;
                while idx < limit {
                    if bytes.get(idx) == Some(&b'}') {
                        return idx + 1;
                    }
                    idx += 1;
                }
                limit
            }
            (Some(b'('), Some(b'(')) => {
                let mut idx = start + 3;
                while idx + 1 < limit {
                    if bytes.get(idx) == Some(&b')') && bytes.get(idx + 1) == Some(&b')') {
                        return idx + 2;
                    }
                    idx += 1;
                }
                limit
            }
            (Some(b'('), _) => {
                let mut idx = start + 2;
                let mut depth = 1usize;
                while idx < limit {
                    match bytes.get(idx).copied() {
                        Some(b'(') => depth += 1,
                        Some(b')') => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                return idx + 1;
                            }
                        }
                        _ => {}
                    }
                    idx += 1;
                }
                limit
            }
            _ => {
                let mut idx = start + 1;
                while idx < limit
                    && bytes
                        .get(idx)
                        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    idx += 1;
                }
                idx
            }
        }
    }

    fn find_pattern_boundary(&self, start_offset: usize, end_offset: usize) -> usize {
        let Some(slice) = self.slice_offsets(start_offset, end_offset) else {
            return start_offset;
        };
        slice
            .bytes()
            .position(|byte| matches!(byte, b'*' | b'?' | b'"' | b'\''))
            .map(|rel| start_offset + rel)
            .unwrap_or(end_offset)
    }

    fn pattern_quote_at(
        &self,
        span: Span,
        valid_parts: &[(&WordPart, Span)],
        cursor: usize,
    ) -> Option<(Span, usize)> {
        let raw = self.slice_span(span)?;
        let rel = cursor.checked_sub(span.start.offset)?;
        let bytes = raw.as_bytes();
        let (quote_start, quote_char, dollar) = match bytes.get(rel).copied() {
            Some(b'"') | Some(b'\'') => (cursor, bytes[rel], false),
            Some(b'$')
                if matches!(bytes.get(rel + 1).copied(), Some(b'"') | Some(b'\'')) =>
            {
                (cursor, bytes[rel + 1], true)
            }
            _ => return None,
        };
        let content_start = quote_start + if dollar { 2 } else { 1 };
        let mut scan = content_start;
        while scan < span.end.offset {
            let byte = *self.source.as_bytes().get(scan)?;
            if byte == quote_char {
                let quote_span =
                    Span::from_positions(self.pos_at(quote_start), self.pos_at(scan + 1));
                let consumed_parts = valid_parts
                    .iter()
                    .take_while(|(_, part_span)| {
                        part_span.start.offset < scan && part_span.end.offset > content_start
                    })
                    .count();
                return Some((quote_span, consumed_parts));
            }
            if quote_char == b'"' && byte == b'\\' {
                scan += 1;
            }
            scan += 1;
        }
        None
    }

    fn redirect_operator_code(&self, kind: RedirectKind) -> u64 {
        match kind {
            RedirectKind::Output => 63,
            RedirectKind::Append => 64,
            RedirectKind::Input => 65,
            RedirectKind::DupInput => 67,
            RedirectKind::DupOutput => 68,
            RedirectKind::Clobber => 69,
            RedirectKind::HereDoc => 71,
            RedirectKind::HereDocStrip => 72,
            RedirectKind::HereString => 73,
            RedirectKind::OutputBoth => 74,
        }
    }

    fn redirect_operator_text(&self, kind: RedirectKind) -> &'static str {
        match kind {
            RedirectKind::Output => ">",
            RedirectKind::Append => ">>",
            RedirectKind::Input => "<",
            RedirectKind::DupInput => "<&",
            RedirectKind::DupOutput => ">&",
            RedirectKind::Clobber => ">|",
            RedirectKind::HereDoc => "<<",
            RedirectKind::HereDocStrip => "<<-",
            RedirectKind::HereString => "<<<",
            RedirectKind::OutputBoth => "&>",
        }
    }

    fn bin_cmd_code(&self, op: ListOperator) -> u64 {
        match op {
            ListOperator::And => 11,
            ListOperator::Or => 12,
            ListOperator::Semicolon | ListOperator::Background => 0,
        }
    }

    fn cond_unary_op_code(&self, op: ConditionalUnaryOp) -> u64 {
        match op {
            ConditionalUnaryOp::Exists => 105,
            ConditionalUnaryOp::RegularFile => 106,
            ConditionalUnaryOp::Directory => 107,
            ConditionalUnaryOp::CharacterSpecial => 108,
            ConditionalUnaryOp::BlockSpecial => 109,
            ConditionalUnaryOp::NamedPipe => 110,
            ConditionalUnaryOp::Socket => 111,
            ConditionalUnaryOp::Symlink => 112,
            ConditionalUnaryOp::Sticky => 113,
            ConditionalUnaryOp::SetGroupId => 114,
            ConditionalUnaryOp::SetUserId => 115,
            ConditionalUnaryOp::GroupOwned => 116,
            ConditionalUnaryOp::UserOwned => 117,
            ConditionalUnaryOp::Modified => 118,
            ConditionalUnaryOp::Readable => 119,
            ConditionalUnaryOp::Writable => 120,
            ConditionalUnaryOp::Executable => 121,
            ConditionalUnaryOp::NonEmptyFile => 122,
            ConditionalUnaryOp::FdTerminal => 123,
            ConditionalUnaryOp::EmptyString => 124,
            ConditionalUnaryOp::NonEmptyString => 125,
            ConditionalUnaryOp::OptionSet => 126,
            ConditionalUnaryOp::VariableSet => 127,
            ConditionalUnaryOp::ReferenceVariable => 128,
            ConditionalUnaryOp::Not => 39,
        }
    }

    fn cond_binary_op_code(&self, op: ConditionalBinaryOp) -> u64 {
        match op {
            ConditionalBinaryOp::RegexMatch => 129,
            ConditionalBinaryOp::NewerThan => 130,
            ConditionalBinaryOp::OlderThan => 131,
            ConditionalBinaryOp::SameFile => 132,
            ConditionalBinaryOp::ArithmeticEq => 133,
            ConditionalBinaryOp::ArithmeticNe => 134,
            ConditionalBinaryOp::ArithmeticLe => 135,
            ConditionalBinaryOp::ArithmeticGe => 136,
            ConditionalBinaryOp::ArithmeticLt => 137,
            ConditionalBinaryOp::ArithmeticGt => 138,
            ConditionalBinaryOp::And => 11,
            ConditionalBinaryOp::Or => 12,
            ConditionalBinaryOp::PatternEqShort => 87,
            ConditionalBinaryOp::PatternEq => 45,
            ConditionalBinaryOp::PatternNe => 46,
            ConditionalBinaryOp::LexicalBefore => 65,
            ConditionalBinaryOp::LexicalAfter => 63,
        }
    }

    fn case_operator_code(&self, op: CaseTerminator) -> u64 {
        match op {
            CaseTerminator::Break => 35,
            CaseTerminator::FallThrough => 36,
            CaseTerminator::Continue => 37,
        }
    }

    fn parameter_operator_code(&self, op: &ParameterOp, colon_variant: bool) -> u64 {
        match op {
            ParameterOp::UseReplacement => {
                if colon_variant {
                    82
                } else {
                    81
                }
            }
            ParameterOp::UseDefault => {
                if colon_variant {
                    84
                } else {
                    83
                }
            }
            ParameterOp::Error => {
                if colon_variant {
                    86
                } else {
                    85
                }
            }
            ParameterOp::AssignDefault => {
                if colon_variant {
                    88
                } else {
                    87
                }
            }
            ParameterOp::RemoveSuffixShort => 89,
            ParameterOp::RemoveSuffixLong => 90,
            ParameterOp::RemovePrefixShort => 91,
            ParameterOp::RemovePrefixLong => 92,
            ParameterOp::UpperFirst => 96,
            ParameterOp::UpperAll => 97,
            ParameterOp::LowerFirst => 98,
            ParameterOp::LowerAll => 99,
            ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. } => 0,
        }
    }

    fn quoted_wrapper(&self, word: &Word) -> Option<QuoteWrapper> {
        let raw = self.slice_span(word.span)?;
        if raw.starts_with("$'") && raw.ends_with('\'') {
            return Some(QuoteWrapper::Single { dollar: true });
        }
        if raw.starts_with('\'') && raw.ends_with('\'') {
            return Some(QuoteWrapper::Single { dollar: false });
        }
        if raw.starts_with("$\"") && raw.ends_with('"') {
            return Some(QuoteWrapper::Double { dollar: true });
        }
        if raw.starts_with('"') && raw.ends_with('"') {
            return Some(QuoteWrapper::Double { dollar: false });
        }
        None
    }

    fn is_background_placeholder(&self, command: &Command) -> bool {
        let Command::Simple(simple) = command else {
            return false;
        };
        self.is_empty_word(&simple.name)
            && simple.args.is_empty()
            && simple.assignments.is_empty()
            && simple.redirects.is_empty()
    }

    fn is_empty_word(&self, word: &Word) -> bool {
        word.parts.len() == 1
            && matches!(&word.parts[0], WordPart::Literal(value) if value.is_empty())
    }

    fn node_object(
        &self,
        type_name: Option<&str>,
        pos: Position,
        end: Position,
    ) -> Map<String, Value> {
        let end = self.normalize_end(end);
        let mut map = Map::new();
        if let Some(type_name) = type_name {
            map.insert("Type".into(), Value::String(type_name.to_owned()));
        }
        self.insert_pos(&mut map, "Pos", pos);
        self.insert_pos(&mut map, "End", end);
        map
    }

    fn insert_array(&self, map: &mut Map<String, Value>, key: &str, values: Vec<Value>) {
        if !values.is_empty() {
            map.insert(key.into(), Value::Array(values));
        }
    }

    fn insert_value(&self, map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
        if let Some(value) = value {
            if !value.is_null() {
                map.insert(key.into(), value);
            }
        }
    }

    fn insert_pos(&self, map: &mut Map<String, Value>, key: &str, pos: Position) {
        if self.is_valid_pos(pos) {
            let mut value = Map::new();
            value.insert(
                "Offset".into(),
                Value::Number(Number::from(pos.offset as u64)),
            );
            value.insert("Line".into(), Value::Number(Number::from(pos.line as u64)));
            value.insert("Col".into(), Value::Number(Number::from(pos.column as u64)));
            map.insert(key.into(), Value::Object(value));
        }
    }

    fn insert_bool(&self, map: &mut Map<String, Value>, key: &str, value: bool) {
        if value {
            map.insert(key.into(), Value::Bool(true));
        }
    }

    fn insert_string(&self, map: &mut Map<String, Value>, key: &str, value: &str) {
        if !value.is_empty() {
            map.insert(key.into(), Value::String(value.to_owned()));
        }
    }

    fn insert_number(&self, map: &mut Map<String, Value>, key: &str, value: u64) {
        if value != 0 {
            map.insert(key.into(), Value::Number(Number::from(value)));
        }
    }

    fn is_valid_pos(&self, pos: Position) -> bool {
        pos.line > 0 && pos.column > 0
    }

    fn normalize_end(&self, mut end: Position) -> Position {
        while end.offset > 0 {
            let Some(byte) = self
                .source
                .as_bytes()
                .get(end.offset.saturating_sub(1))
                .copied()
            else {
                break;
            };
            if byte != b'\n' && byte != b'\r' {
                break;
            }
            end = self.pos_at(end.offset.saturating_sub(1));
        }
        end
    }

    fn pos_at(&self, offset: usize) -> Position {
        if offset > self.source.len() {
            return Position::default();
        }
        let line_index = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        Position {
            line: line_index + 1,
            column: offset.saturating_sub(line_start) + 1,
            offset,
        }
    }

    fn slice_span(&self, span: Span) -> Option<&'a str> {
        self.slice_offsets(span.start.offset, span.end.offset)
    }

    fn slice_offsets(&self, start: usize, end: usize) -> Option<&'a str> {
        if start > end || end > self.source.len() {
            return None;
        }
        self.source.get(start..end)
    }

    fn find_in_span(&self, span: Span, needle: &str, from_rel: usize) -> Option<Position> {
        let haystack = self.slice_span(span)?;
        let rel = haystack.get(from_rel..)?.find(needle)? + from_rel;
        Some(self.pos_at(span.start.offset + rel))
    }

    fn rfind_in_span(&self, span: Span, needle: &str) -> Option<Position> {
        let haystack = self.slice_span(span)?;
        let rel = haystack.rfind(needle)?;
        Some(self.pos_at(span.start.offset + rel))
    }

    fn find_operator_between(
        &self,
        start_offset: usize,
        end_offset: usize,
        operator: &str,
    ) -> Option<Position> {
        let slice = self.slice_offsets(start_offset, end_offset)?;
        let rel = slice.find(operator)?;
        Some(self.pos_at(start_offset + rel))
    }

    fn rfind_operator_between(
        &self,
        start_offset: usize,
        end_offset: usize,
        operator: &str,
    ) -> Option<Position> {
        let slice = self.slice_offsets(start_offset, end_offset)?;
        let rel = slice.rfind(operator)?;
        Some(self.pos_at(start_offset + rel))
    }

    fn find_operator_between_spans(
        &self,
        left: Span,
        right: Span,
        operator: &str,
    ) -> Option<Position> {
        self.find_operator_after_span(left, right.start.offset, operator)
    }

    fn find_operator_after_span(
        &self,
        left: Span,
        right_start_offset: usize,
        operator: &str,
    ) -> Option<Position> {
        self.find_operator_between(left.end.offset, right_start_offset, operator)
            .or_else(|| {
                self.rfind_operator_between(left.start.offset, right_start_offset, operator)
            })
    }

    fn find_keyword(&self, span: Span, keyword: &str) -> Option<Position> {
        self.find_keyword_after(span, keyword, span.start.offset.saturating_sub(1))
    }

    fn rfind_keyword(&self, span: Span, keyword: &str) -> Option<Position> {
        let slice = self.slice_span(span)?;
        let mut search_end = slice.len();
        while let Some(rel) = slice[..search_end].rfind(keyword) {
            let absolute = span.start.offset + rel;
            if self.is_keyword_boundary(absolute, keyword) {
                return Some(self.pos_at(absolute));
            }
            search_end = rel;
        }
        None
    }

    fn find_keyword_after(
        &self,
        span: Span,
        keyword: &str,
        after_offset: usize,
    ) -> Option<Position> {
        let start = after_offset.max(span.start.offset);
        let slice = self.slice_offsets(start, span.end.offset)?;
        let mut search = 0;
        while let Some(rel) = slice[search..].find(keyword) {
            let absolute = start + search + rel;
            if self.is_keyword_boundary(absolute, keyword) {
                return Some(self.pos_at(absolute));
            }
            search += rel + keyword.len();
        }
        None
    }

    fn is_keyword_boundary(&self, absolute: usize, keyword: &str) -> bool {
        let before = absolute
            .checked_sub(1)
            .and_then(|idx| self.source.as_bytes().get(idx).copied());
        let after = self
            .source
            .as_bytes()
            .get(absolute + keyword.len())
            .copied();
        let boundary = |byte: Option<u8>| match byte {
            None => true,
            Some(byte) => !(byte.is_ascii_alphanumeric() || byte == b'_'),
        };
        boundary(before) && boundary(after)
    }

    fn search_backward_for_token(&self, from_offset: usize, token: &str) -> Option<Position> {
        let slice = self.slice_offsets(0, from_offset)?;
        let rel = slice.rfind(token)?;
        Some(self.pos_at(rel))
    }

    fn search_forward_for_token(&self, from_offset: usize, token: &str) -> Option<Position> {
        let slice = self.slice_offsets(from_offset, self.source.len())?;
        let rel = slice.find(token)?;
        Some(self.pos_at(from_offset + rel))
    }
}

#[derive(Clone, Copy)]
enum QuoteWrapper {
    Single { dollar: bool },
    Double { dollar: bool },
}

#[cfg(test)]
mod tests {
    use super::to_typed_json;
    use serde_json::json;
    use shuck_parser::parser::Parser;

    fn typed_json(source: &str) -> serde_json::Value {
        let script = Parser::new(source).parse().expect("parse should succeed");
        to_typed_json(&script, source)
    }

    #[test]
    fn serializes_simple_call_expr() {
        let actual = typed_json("echo hello\n");
        assert_eq!(actual["Type"], "File");
        assert_eq!(actual["Stmts"][0]["Cmd"]["Type"], "CallExpr");
        assert_eq!(
            actual["Stmts"][0]["Cmd"]["Args"],
            json!([
                {
                    "Pos": {"Offset": 0, "Line": 1, "Col": 1},
                    "End": {"Offset": 4, "Line": 1, "Col": 5},
                    "Parts": [{
                        "Type": "Lit",
                        "Pos": {"Offset": 0, "Line": 1, "Col": 1},
                        "End": {"Offset": 4, "Line": 1, "Col": 5},
                        "ValuePos": {"Offset": 0, "Line": 1, "Col": 1},
                        "ValueEnd": {"Offset": 4, "Line": 1, "Col": 5},
                        "Value": "echo"
                    }]
                },
                {
                    "Pos": {"Offset": 5, "Line": 1, "Col": 6},
                    "End": {"Offset": 10, "Line": 1, "Col": 11},
                    "Parts": [{
                        "Type": "Lit",
                        "Pos": {"Offset": 5, "Line": 1, "Col": 6},
                        "End": {"Offset": 10, "Line": 1, "Col": 11},
                        "ValuePos": {"Offset": 5, "Line": 1, "Col": 6},
                        "ValueEnd": {"Offset": 10, "Line": 1, "Col": 11},
                        "Value": "hello"
                    }]
                }
            ])
        );
    }

    #[test]
    fn serializes_logical_lists_as_nested_binary_cmds() {
        let actual = typed_json("a && b || c\n");
        assert_eq!(actual["Stmts"].as_array().unwrap().len(), 1);
        assert_eq!(actual["Stmts"][0]["Cmd"]["Type"], "BinaryCmd");
        assert_eq!(actual["Stmts"][0]["Cmd"]["Op"], 12);
        assert_eq!(actual["Stmts"][0]["Cmd"]["X"]["Cmd"]["Type"], "BinaryCmd");
        assert_eq!(actual["Stmts"][0]["Cmd"]["X"]["Cmd"]["Op"], 11);
    }

    #[test]
    fn serializes_pipelines_as_nested_binary_cmds() {
        let actual = typed_json("a | b | c\n");
        assert_eq!(actual["Stmts"][0]["Cmd"]["Type"], "BinaryCmd");
        assert_eq!(actual["Stmts"][0]["Cmd"]["Op"], 13);
        assert_eq!(actual["Stmts"][0]["Cmd"]["X"]["Cmd"]["Type"], "BinaryCmd");
        assert_eq!(actual["Stmts"][0]["Cmd"]["X"]["Cmd"]["Op"], 13);
    }

    #[test]
    fn serializes_if_clause() {
        let actual = typed_json("if foo; then bar; fi\n");
        let if_clause = &actual["Stmts"][0]["Cmd"];
        assert_eq!(if_clause["Type"], "IfClause");
        assert_eq!(if_clause["Kind"], "if");
        assert_eq!(if_clause["Cond"][0]["Cmd"]["Type"], "CallExpr");
        assert_eq!(if_clause["Then"][0]["Cmd"]["Type"], "CallExpr");
    }

    #[test]
    fn serializes_for_clause() {
        let actual = typed_json("for i in 1 2; do echo \"$i\"; done\n");
        let for_clause = &actual["Stmts"][0]["Cmd"];
        assert_eq!(for_clause["Type"], "ForClause");
        assert_eq!(for_clause["Loop"]["Type"], "WordIter");
        assert_eq!(for_clause["Loop"]["Name"]["Value"], "i");
        assert_eq!(for_clause["Do"][0]["Cmd"]["Type"], "CallExpr");
    }

    #[test]
    fn serializes_assignments_and_redirects() {
        let actual = typed_json("FOO=bar echo hi >out\n");
        let stmt = &actual["Stmts"][0];
        assert_eq!(stmt["Cmd"]["Assigns"][0]["Ref"]["Name"]["Value"], "FOO");
        assert_eq!(stmt["Redirs"][0]["Op"], 63);
        assert_eq!(stmt["Redirs"][0]["Word"]["Parts"][0]["Value"], "out");
    }

    #[test]
    fn serializes_parameter_expansions_and_substitutions() {
        let actual = typed_json("echo ${foo:-bar} $(baz) $((1+2))\n");
        let args = actual["Stmts"][0]["Cmd"]["Args"].as_array().unwrap();
        assert_eq!(args[1]["Parts"][0]["Type"], "ParamExp");
        assert_eq!(args[1]["Parts"][0]["Exp"]["Op"], 84);
        assert_eq!(args[2]["Parts"][0]["Type"], "CmdSubst");
        assert_eq!(args[3]["Parts"][0]["Type"], "ArithmExp");
    }

    #[test]
    fn serializes_arithmetic_command_from_source_slices() {
        let actual = typed_json("(( 1 + 2 <= 3 ))\n");
        let arithmetic = &actual["Stmts"][0]["Cmd"];
        assert_eq!(arithmetic["Type"], "ArithmCmd");
        assert_eq!(arithmetic["Source"], " 1 + 2 <= 3 ");
        assert_eq!(arithmetic["X"]["Parts"][0]["Value"], " 1 + 2 <= 3 ");
        assert_eq!(arithmetic["X"]["Pos"]["Col"], 3);
    }

    #[test]
    fn serializes_c_style_loop_from_source_slices() {
        let actual = typed_json("for (( i = 0 ; i < 10 ; i += 2 )); do echo \"$i\"; done\n");
        let loop_node = &actual["Stmts"][0]["Cmd"]["Loop"];
        assert_eq!(loop_node["Type"], "CStyleLoop");
        assert_eq!(loop_node["Init"]["Parts"][0]["Value"], " i = 0 ");
        assert_eq!(loop_node["Cond"]["Parts"][0]["Value"], " i < 10 ");
        assert_eq!(loop_node["Post"]["Parts"][0]["Value"], " i += 2 ");
        assert_eq!(loop_node["Lparen"]["Col"], 5);
    }

    #[test]
    fn serializes_identifier_positions_from_exact_spans() {
        let actual = typed_json("foo[10]=bar\nexec {myfd}>&-\n");
        assert_eq!(
            actual["Stmts"][0]["Cmd"]["Assigns"][0]["Ref"]["Name"]["Value"],
            "foo"
        );
        assert_eq!(
            actual["Stmts"][0]["Cmd"]["Assigns"][0]["Ref"]["Name"]["ValuePos"]["Col"],
            1
        );
        assert_eq!(
            actual["Stmts"][0]["Cmd"]["Assigns"][0]["Ref"]["Index"]["Expr"]["Parts"][0]["Pos"]["Col"],
            5
        );
        assert_eq!(actual["Stmts"][1]["Redirs"][0]["N"]["Value"], "myfd");
        assert_eq!(actual["Stmts"][1]["Redirs"][0]["N"]["ValuePos"]["Col"], 7);
    }

    #[test]
    fn serializes_structured_test_clause() {
        let actual = typed_json("[[ ! (foo && bar) ]]\n");
        let clause = &actual["Stmts"][0]["Cmd"];
        assert_eq!(clause["Type"], "TestClause");
        assert_eq!(clause["Left"]["Col"], 1);
        assert_eq!(clause["Right"]["Col"], 19);
        assert_eq!(clause["X"]["Type"], "CondUnary");
        assert_eq!(clause["X"]["Op"], 39);
        assert_eq!(clause["X"]["X"]["Type"], "CondParen");
        assert_eq!(clause["X"]["X"]["X"]["Type"], "CondBinary");
        assert_eq!(clause["X"]["X"]["X"]["Op"], 11);
    }

    #[test]
    fn serializes_pattern_and_regex_operands_in_conditionals() {
        let pattern = typed_json("[[ foo == (bar|baz)* ]]\n");
        assert_eq!(pattern["Stmts"][0]["Cmd"]["X"]["Type"], "CondBinary");
        assert_eq!(pattern["Stmts"][0]["Cmd"]["X"]["Op"], 45);
        assert_eq!(pattern["Stmts"][0]["Cmd"]["X"]["Y"]["Type"], "CondPattern");
        assert_eq!(
            pattern["Stmts"][0]["Cmd"]["X"]["Y"]["Pattern"]["Parts"][0]["Value"],
            "("
        );

        let regex = typed_json("[[ foo =~ [ab](c|d) ]]\n");
        assert_eq!(regex["Stmts"][0]["Cmd"]["X"]["Type"], "CondBinary");
        assert_eq!(regex["Stmts"][0]["Cmd"]["X"]["Op"], 129);
        assert_eq!(regex["Stmts"][0]["Cmd"]["X"]["Y"]["Type"], "CondRegex");
    }

    #[test]
    fn serializes_decl_clause_with_typed_operands() {
        let actual = typed_json("FOO=1 declare -a arr=(\"hello world\" two) foo\n");
        let clause = &actual["Stmts"][0]["Cmd"];

        assert_eq!(clause["Type"], "DeclClause");
        assert_eq!(clause["Variant"]["Value"], "declare");
        assert_eq!(clause["Assigns"][0]["Ref"]["Name"]["Value"], "FOO");
        assert_eq!(clause["Operands"][0]["Type"], "DeclFlag");
        assert_eq!(clause["Operands"][0]["Word"]["Parts"][0]["Value"], "-a");
        assert_eq!(clause["Operands"][1]["Type"], "DeclAssign");
        assert_eq!(
            clause["Operands"][1]["Assign"]["Ref"]["Name"]["Value"],
            "arr"
        );
        assert_eq!(
            clause["Operands"][1]["Assign"]["Array"]["Elems"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(clause["Operands"][2]["Type"], "DeclName");
        assert_eq!(clause["Operands"][2]["Ref"]["Name"]["Value"], "foo");
    }
}
