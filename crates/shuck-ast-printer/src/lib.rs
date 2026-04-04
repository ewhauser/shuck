use serde_json::{Map, Number, Value};
use shuck_ast::{
    ArithmeticForCommand, Assignment, AssignmentValue, BuiltinCommand, CaseCommand, CaseItem,
    CaseTerminator, Command, CompoundCommand, ConditionalBinaryExpr, ConditionalBinaryOp,
    ConditionalCommand, ConditionalExpr, ConditionalParenExpr, ConditionalUnaryExpr,
    ConditionalUnaryOp, CoprocCommand, ForCommand, FunctionDef, IfCommand, ListOperator,
    ParameterOp, Pipeline, Position, Redirect, RedirectKind, Script, SelectCommand, SimpleCommand,
    Span, TimeCommand, UntilCommand, WhileCommand, Word, WordPart,
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
        let mut map = self.node_object(Some("File"), script.span.start, script.span.end);
        self.insert_array(&mut map, "Stmts", self.encode_stmt_values(&script.commands));
        EncodedNode {
            value: Value::Object(map),
            pos: script.span.start,
            end: script.span.end,
        }
    }

    fn encode_stmt_values(&self, commands: &[Command]) -> Vec<Value> {
        commands
            .iter()
            .flat_map(|command| self.fragments_for_command(command))
            .map(|fragment| self.encode_fragment(&fragment).value)
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
                    let semicolon = self.find_operator_between(
                        self.command_span(current_last).end.offset,
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
            let op_pos = self.find_operator_between(
                self.command_span(lhs_cmd).end.offset,
                self.command_span(rhs_cmd).start.offset,
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
            let op_pos = self.find_operator_between(
                self.command_span(last).end.offset,
                self.command_span(rhs_command).start.offset,
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
        if let Some((rsrv_word, parens, name_pos)) =
            self.function_surface(&function.name, function.span, body_stmt.pos)
        {
            self.insert_bool(&mut map, "RsrvWord", rsrv_word);
            self.insert_bool(&mut map, "Parens", parens);
            self.insert_value(
                &mut map,
                "Name",
                Some(
                    self.lit_node(
                        &function.name,
                        name_pos,
                        name_pos.advanced_by(&function.name),
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
                    self.lit_node(
                        &function.name,
                        function.span.start,
                        function.span.start.advanced_by(&function.name),
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
            CompoundCommand::Arithmetic(expression) => self.encode_arithm_cmd(expression),
            CompoundCommand::Conditional(command) => self.encode_test_clause(command),
            CompoundCommand::Time(command) => self.encode_time(command),
            CompoundCommand::Coproc(command) => self.encode_coproc(command),
        }
    }

    fn encode_if(&self, command: &IfCommand) -> EncodedNode {
        let fi_pos = self
            .find_keyword(command.span, "fi")
            .unwrap_or(command.span.end);
        self.encode_if_clause_chain(
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
        let mut map = self.node_object(Some("IfClause"), position, span.end);
        self.insert_pos(&mut map, "Position", position);
        self.insert_string(&mut map, "Kind", kind);
        self.insert_pos(&mut map, "ThenPos", then_pos);
        self.insert_pos(&mut map, "FiPos", fi_pos);
        self.insert_array(&mut map, "Cond", self.encode_stmt_values(condition));
        self.insert_array(&mut map, "Then", self.encode_stmt_values(then_branch));

        let else_node = if let Some(((elif_cond, elif_then), rest)) = elif_branches.split_first() {
            let elif_pos = self
                .find_keyword_after(span, "elif", then_pos.offset)
                .unwrap_or_default();
            Some(
                self.encode_if_clause_chain(
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
            .find_keyword(command.span, "done")
            .unwrap_or(command.span.end);
        let name_pos = self
            .find_name_after_keyword(command.span, "for", &command.variable)
            .unwrap_or(command.span.start);
        let loop_end = command
            .words
            .as_ref()
            .and_then(|words| words.last())
            .map(|word| word.span.end)
            .unwrap_or_else(|| name_pos.advanced_by(&command.variable));

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
                    name_pos,
                    self.find_keyword_after(command.span, "in", name_pos.offset),
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
            .find_keyword(command.span, "done")
            .unwrap_or(command.span.end);
        let name_pos = self
            .find_name_after_keyword(command.span, "select", &command.variable)
            .unwrap_or(command.span.start);
        let loop_end = command
            .words
            .last()
            .map(|word| word.span.end)
            .unwrap_or_else(|| name_pos.advanced_by(&command.variable));

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
                    name_pos,
                    self.find_keyword_after(command.span, "in", name_pos.offset),
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
            .find_keyword(command.span, "done")
            .unwrap_or(command.span.end);
        let lparen = self
            .find_operator_between(for_pos.offset, do_pos.offset, "((")
            .unwrap_or_default();
        let rparen = self
            .rfind_operator_between(for_pos.offset, do_pos.offset, "))")
            .unwrap_or_default();
        let loop_node = self.encode_c_style_loop(command, lparen, rparen);

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
        let done_pos = self.find_keyword(span, "done").unwrap_or(span.end);

        let mut map = self.node_object(Some("WhileClause"), while_pos, span.end);
        self.insert_pos(&mut map, "WhilePos", while_pos);
        self.insert_pos(&mut map, "DoPos", do_pos);
        self.insert_pos(&mut map, "DonePos", done_pos);
        self.insert_bool(&mut map, "Until", until);
        self.insert_array(
            &mut map,
            "Cond",
            self.encode_stmt_values(&command.condition),
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
            .find_keyword(command.span, "esac")
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
            self.encode_case_items(&command.cases, command.span, esac_pos),
        );
        EncodedNode {
            value: Value::Object(map),
            pos: case_pos,
            end: command.span.end,
        }
    }

    fn encode_case_items(&self, items: &[CaseItem], span: Span, esac_pos: Position) -> Vec<Value> {
        items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let next_start = items
                    .get(idx + 1)
                    .and_then(|next| next.patterns.first())
                    .map(|word| word.span.start.offset)
                    .unwrap_or(esac_pos.offset);
                self.encode_case_item(item, next_start, span).value
            })
            .collect()
    }

    fn encode_case_item(&self, item: &CaseItem, next_start: usize, _span: Span) -> EncodedNode {
        let pos = item
            .patterns
            .first()
            .map(|word| word.span.start)
            .unwrap_or_default();
        let body_end = item
            .commands
            .last()
            .map(|command| self.command_span(command).end)
            .or_else(|| item.patterns.last().map(|word| word.span.end))
            .unwrap_or(pos);
        let op_str = match item.terminator {
            CaseTerminator::Break => ";;",
            CaseTerminator::FallThrough => ";&",
            CaseTerminator::Continue => ";;&",
        };
        let op_pos = self
            .find_operator_between(body_end.offset, next_start, op_str)
            .unwrap_or_default();
        let end = if self.is_valid_pos(op_pos) {
            op_pos.advanced_by(op_str)
        } else {
            body_end
        };

        let mut map = self.node_object(Some("CaseItem"), pos, end);
        self.insert_number(
            &mut map,
            "Op",
            self.case_operator_code(item.terminator.clone()),
        );
        self.insert_pos(&mut map, "OpPos", op_pos);
        self.insert_array(
            &mut map,
            "Patterns",
            item.patterns
                .iter()
                .map(|word| self.encode_word(word).value)
                .collect(),
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

    fn encode_arithm_cmd(&self, expression: &str) -> EncodedNode {
        let pos = Position::default();
        let mut map = self.node_object(Some("ArithmCmd"), pos, Position::default());
        self.insert_string(&mut map, "Source", expression);
        if !expression.is_empty() {
            self.insert_value(
                &mut map,
                "X",
                Some(
                    self.synthetic_expression_word(expression, Span::new())
                        .value,
                ),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos,
            end: Position::default(),
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
        body_pos: Position,
    ) -> Option<EncodedNode> {
        let between = self.slice_offsets(command.span.start.offset, body_pos.offset)?;
        let mut tokens = between.split_whitespace();
        let first = tokens.next()?;
        if first != "coproc" {
            return None;
        }
        let second = tokens.next()?;
        if second == "coproc" || second.is_empty() {
            return None;
        }
        if command.name == "COPROC" && second != "COPROC" {
            return None;
        }
        let name_offset = between.find(second)? + command.span.start.offset;
        Some(self.synthetic_literal_word_node(
            second,
            Span::from_positions(
                self.pos_at(name_offset),
                self.pos_at(name_offset).advanced_by(second),
            ),
        ))
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
        name_pos: Position,
        in_pos: Option<Position>,
        items: &[Word],
        end: Position,
    ) -> EncodedNode {
        let name_end = name_pos.advanced_by(variable);
        let mut map = self.node_object(Some("WordIter"), name_pos, end);
        self.insert_value(
            &mut map,
            "Name",
            Some(self.lit_node(variable, name_pos, name_end).value),
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
            pos: name_pos,
            end,
        }
    }

    fn encode_c_style_loop(
        &self,
        command: &ArithmeticForCommand,
        lparen: Position,
        rparen: Position,
    ) -> EncodedNode {
        let mut map = self.node_object(
            Some("CStyleLoop"),
            lparen,
            if self.is_valid_pos(rparen) {
                rparen.advanced_by("))")
            } else {
                Position::default()
            },
        );
        self.insert_pos(&mut map, "Lparen", lparen);
        self.insert_pos(&mut map, "Rparen", rparen);
        if !command.init.is_empty() {
            self.insert_value(
                &mut map,
                "Init",
                Some(
                    self.synthetic_expression_word(&command.init, Span::new())
                        .value,
                ),
            );
        }
        if !command.condition.is_empty() {
            self.insert_value(
                &mut map,
                "Cond",
                Some(
                    self.synthetic_expression_word(&command.condition, Span::new())
                        .value,
                ),
            );
        }
        if !command.step.is_empty() {
            self.insert_value(
                &mut map,
                "Post",
                Some(
                    self.synthetic_expression_word(&command.step, Span::new())
                        .value,
                ),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: lparen,
            end: if self.is_valid_pos(rparen) {
                rparen.advanced_by("))")
            } else {
                Position::default()
            },
        }
    }

    fn encode_cond_expr(&self, expr: &ConditionalExpr) -> EncodedNode {
        match expr {
            ConditionalExpr::Binary(expr) => self.encode_cond_binary(expr),
            ConditionalExpr::Unary(expr) => self.encode_cond_unary(expr),
            ConditionalExpr::Parenthesized(expr) => self.encode_cond_paren(expr),
            ConditionalExpr::Word(word) => self.encode_cond_leaf("CondWord", "Word", word),
            ConditionalExpr::Pattern(word) => self.encode_cond_leaf("CondPattern", "Word", word),
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
        EncodedNode {
            value: Value::Object(map),
            pos: assignment.span.start,
            end: assignment.span.end,
        }
    }

    fn encode_var_ref_from_assignment(&self, assignment: &Assignment) -> EncodedNode {
        let name_pos = assignment.span.start;
        let name_end = name_pos.advanced_by(&assignment.name);
        let mut map = self.node_object(Some("VarRef"), name_pos, assignment.span.end);
        self.insert_value(
            &mut map,
            "Name",
            Some(self.lit_node(&assignment.name, name_pos, name_end).value),
        );
        if let Some(index) = &assignment.index {
            self.insert_value(
                &mut map,
                "Index",
                Some(self.encode_subscript(index, name_end).value),
            );
        }
        EncodedNode {
            value: Value::Object(map),
            pos: name_pos,
            end: assignment.span.end,
        }
    }

    fn encode_array_expr(&self, words: &[Word], span: Span) -> EncodedNode {
        let lparen = self
            .find_operator_between(span.start.offset, span.end.offset, "(")
            .unwrap_or_default();
        let rparen = self
            .rfind_operator_between(span.start.offset, span.end.offset, ")")
            .unwrap_or_default();
        let mut map = self.node_object(Some("ArrayExpr"), lparen, span.end);
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
        let mut map = self.node_object(Some("ArrayElem"), word.span.start, word.span.end);
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
                    Some(self.encode_word(&redirect.target).value),
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
            let start = redirect.span.start.advanced_by("{");
            let end = start.advanced_by(fd_var);
            return Some(self.lit_node(fd_var, start, end));
        }
        redirect.fd.map(|fd| {
            let text = fd.to_string();
            let start = self
                .find_operator_between(redirect.span.start.offset, op_pos.offset, &text)
                .unwrap_or(redirect.span.start);
            self.lit_node(&text, start, start.advanced_by(&text))
        })
    }

    fn encode_word(&self, word: &Word) -> EncodedNode {
        let mut map = self.node_object(None, word.span.start, word.span.end);
        let parts = if let Some(wrapper) = self.quoted_wrapper(word) {
            vec![self.encode_quoted_wrapper(word, wrapper).value]
        } else {
            word.parts_with_spans()
                .map(|(part, span)| self.encode_word_part(part, span).value)
                .collect::<Vec<_>>()
        };
        self.insert_array(&mut map, "Parts", parts);
        EncodedNode {
            value: Value::Object(map),
            pos: word.span.start,
            end: word.span.end,
        }
    }

    fn encode_word_part(&self, part: &WordPart, span: Span) -> EncodedNode {
        match part {
            WordPart::Literal(value) => self.lit_node(value, span.start, span.end),
            WordPart::Variable(name) => self.encode_simple_param_exp(name, span),
            WordPart::Length(name) => self.encode_length_param_exp(name, span),
            WordPart::ParameterExpansion {
                name,
                operator,
                operand,
                colon_variant,
            } => self.encode_parameter_expansion(name, operator, operand, *colon_variant, span),
            WordPart::ArrayAccess { name, index } => self.encode_array_access(name, index, span),
            WordPart::ArrayLength(name) => self.encode_array_length(name, span),
            WordPart::ArrayIndices(name) => self.encode_array_indices(name, span),
            WordPart::Substring {
                name,
                offset,
                length,
            } => self.encode_substring(name, offset, length.as_deref(), span),
            WordPart::ArraySlice {
                name,
                offset,
                length,
            } => self.encode_array_slice(name, offset, length.as_deref(), span),
            WordPart::IndirectExpansion {
                name,
                operator,
                operand,
                colon_variant,
            } => self.encode_indirect_expansion(
                name,
                operator.clone(),
                operand,
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
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        WordPart::Literal(value) => Some(value.as_str()),
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
                self.insert_array(
                    &mut map,
                    "Parts",
                    word.parts_with_spans()
                        .map(|(part, span)| self.encode_word_part(part, span).value)
                        .collect(),
                );
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
        let rbrace = if raw.ends_with('}') {
            self.pos_at(span.end.offset.saturating_sub(1))
        } else {
            Position::default()
        };
        let param_pos = self
            .find_in_span(span, name, if short { 1 } else { 2 })
            .unwrap_or(span.start.advanced_by("$"));
        let param = self.lit_node(name, param_pos, param_pos.advanced_by(name));

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
        operand: &str,
        colon_variant: bool,
        span: Span,
    ) -> EncodedNode {
        let raw = self.slice_span(span).unwrap_or_default();
        let short = !raw.starts_with("${");
        let dollar = span.start;
        let rbrace = if raw.ends_with('}') {
            self.pos_at(span.end.offset.saturating_sub(1))
        } else {
            Position::default()
        };
        let param_pos = self
            .find_in_span(span, name, if short { 1 } else { 2 })
            .unwrap_or(span.start.advanced_by("$"));
        let param = self.lit_node(name, param_pos, param_pos.advanced_by(name));

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
                let mut repl = self.node_object(None, span.start, span.end);
                self.insert_bool(
                    &mut repl,
                    "All",
                    matches!(operator, ParameterOp::ReplaceAll { .. }),
                );
                self.insert_value(
                    &mut repl,
                    "Orig",
                    Some(self.encode_pattern_literal(pattern).value),
                );
                self.insert_value(
                    &mut repl,
                    "With",
                    Some(
                        self.synthetic_literal_word_node(replacement, Span::new())
                            .value,
                    ),
                );
                self.insert_value(&mut map, "Repl", Some(Value::Object(repl)));
            }
            ParameterOp::RemovePrefixShort
            | ParameterOp::RemovePrefixLong
            | ParameterOp::RemoveSuffixShort
            | ParameterOp::RemoveSuffixLong => {
                let mut exp = self.node_object(None, span.start, span.end);
                self.insert_number(
                    &mut exp,
                    "Op",
                    self.parameter_operator_code(operator, colon_variant),
                );
                self.insert_value(
                    &mut exp,
                    "Pattern",
                    Some(self.encode_pattern_literal(operand).value),
                );
                self.insert_value(&mut map, "Exp", Some(Value::Object(exp)));
            }
            _ => {
                let mut exp = self.node_object(None, span.start, span.end);
                self.insert_number(
                    &mut exp,
                    "Op",
                    self.parameter_operator_code(operator, colon_variant),
                );
                if !operand.is_empty() {
                    self.insert_value(
                        &mut exp,
                        "Word",
                        Some(self.synthetic_literal_word_node(operand, Span::new()).value),
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

    fn encode_array_access(&self, name: &str, index: &str, span: Span) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_value(
            map,
            "Index",
            Some(self.encode_subscript(index, span.start).value),
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
        offset: &str,
        length: Option<&str>,
        span: Span,
    ) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        let mut slice = self.node_object(None, span.start, span.end);
        self.insert_value(
            &mut slice,
            "Offset",
            Some(self.synthetic_expression_word(offset, Span::new()).value),
        );
        self.insert_value(
            &mut slice,
            "Length",
            length.map(|length| self.synthetic_expression_word(length, Span::new()).value),
        );
        self.insert_value(map, "Slice", Some(Value::Object(slice)));
        node
    }

    fn encode_array_slice(
        &self,
        name: &str,
        offset: &str,
        length: Option<&str>,
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
        let mut slice = self.node_object(None, span.start, span.end);
        self.insert_value(
            &mut slice,
            "Offset",
            Some(self.synthetic_expression_word(offset, Span::new()).value),
        );
        self.insert_value(
            &mut slice,
            "Length",
            length.map(|length| self.synthetic_expression_word(length, Span::new()).value),
        );
        self.insert_value(map, "Slice", Some(Value::Object(slice)));
        node
    }

    fn encode_indirect_expansion(
        &self,
        name: &str,
        operator: Option<ParameterOp>,
        operand: &str,
        colon_variant: bool,
        span: Span,
    ) -> EncodedNode {
        let mut node = self.encode_simple_param_exp(name, span);
        let Value::Object(map) = &mut node.value else {
            unreachable!()
        };
        self.insert_bool(map, "Excl", true);
        if let Some(operator) = operator {
            let mut exp = self.node_object(None, span.start, span.end);
            self.insert_number(
                &mut exp,
                "Op",
                self.parameter_operator_code(&operator, colon_variant),
            );
            if !operand.is_empty() {
                self.insert_value(
                    &mut exp,
                    "Word",
                    Some(self.synthetic_literal_word_node(operand, Span::new()).value),
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
        let mut exp = self.node_object(None, span.start, span.end);
        self.insert_number(&mut exp, "Op", 100);
        self.insert_value(
            &mut exp,
            "Word",
            Some(
                self.synthetic_literal_word_node(&operator.to_string(), Span::new())
                    .value,
            ),
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
        EncodedNode {
            value: Value::Object(map),
            pos: span.start,
            end: span.end,
        }
    }

    fn encode_arithm_exp(&self, expression: &str, span: Span) -> EncodedNode {
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
        let mut map = self.node_object(Some("Subscript"), left, right.advanced_by("]"));
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

    fn encode_all_elements_subscript(&self, span: Span, at: bool) -> EncodedNode {
        let raw = self.slice_span(span).unwrap_or_default();
        let needle = if at { "[@]" } else { "[*]" };
        if let Some(rel) = raw.find(needle) {
            let left = self.pos_at(span.start.offset + rel);
            let right = self.pos_at(span.start.offset + rel + 2);
            let mut map = self.node_object(Some("Subscript"), left, right.advanced_by("]"));
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
        let lit = self.lit_node(pattern, Position::default(), Position::default());
        let mut map = self.node_object(Some("Pattern"), Position::default(), Position::default());
        self.insert_array(&mut map, "Parts", vec![lit.value]);
        EncodedNode {
            value: Value::Object(map),
            pos: Position::default(),
            end: Position::default(),
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

    fn synthetic_literal_word(&self, value: &str, span: Span) -> Word {
        Word::literal_with_span(value, span)
    }

    fn synthetic_literal_word_node(&self, value: &str, span: Span) -> EncodedNode {
        let word = self.synthetic_literal_word(value, span);
        self.encode_word(&word)
    }

    fn synthetic_expression_word(&self, value: &str, span: Span) -> EncodedNode {
        let span = if self.is_valid_pos(span.start) || self.is_valid_pos(span.end) {
            span
        } else {
            Span::from_positions(Position::default(), Position::default())
        };
        self.synthetic_literal_word_node(value, span)
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
            Command::Simple(command) => command.span,
            Command::Builtin(command) => self.builtin_span(command),
            Command::Pipeline(command) => command.span,
            Command::List(command) => command.span,
            Command::Compound(command, redirects) => self.compound_span(command, redirects),
            Command::Function(command) => command.span,
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
            CompoundCommand::Arithmetic(_) => Span::new(),
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

    fn last_redirect_end(&self, redirects: &[Redirect]) -> Position {
        redirects
            .last()
            .map(|redirect| redirect.span.end)
            .unwrap_or_default()
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
        if !word.quoted {
            return None;
        }
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

    fn find_keyword(&self, span: Span, keyword: &str) -> Option<Position> {
        self.find_keyword_after(span, keyword, span.start.offset.saturating_sub(1))
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

    fn find_name_after_keyword(&self, span: Span, keyword: &str, name: &str) -> Option<Position> {
        let keyword_pos = self.find_keyword(span, keyword)?;
        let start = keyword_pos.offset + keyword.len();
        let slice = self.slice_offsets(start, span.end.offset)?;
        let rel = slice.find(name)?;
        Some(self.pos_at(start + rel))
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
            pattern["Stmts"][0]["Cmd"]["X"]["Y"]["Word"]["Parts"][0]["Value"],
            "("
        );

        let regex = typed_json("[[ foo =~ [ab](c|d) ]]\n");
        assert_eq!(regex["Stmts"][0]["Cmd"]["X"]["Type"], "CondBinary");
        assert_eq!(regex["Stmts"][0]["Cmd"]["X"]["Op"], 129);
        assert_eq!(regex["Stmts"][0]["Cmd"]["X"]["Y"]["Type"], "CondRegex");
    }
}
