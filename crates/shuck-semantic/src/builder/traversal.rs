use super::zsh_effects::recorded_command_info;
use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn flow_context(flow: FlowState) -> FlowContext {
        FlowContext {
            in_function: flow.in_function,
            loop_depth: flow.loop_depth,
            in_subshell: flow.in_subshell,
            in_block: flow.in_block,
            exit_status_checked: flow.exit_status_checked,
        }
    }

    pub(super) fn record_command(
        &mut self,
        span: Span,
        nested_regions: Vec<IsolatedRegion>,
        kind: RecordedCommandKind,
    ) -> CommandId {
        let nested_regions = self.recorded_program.push_regions(nested_regions);
        self.recorded_program.push_command(RecordedCommand {
            span,
            syntax_span: span,
            syntax_kind: None,
            scope: None,
            flow_context: None,
            nested_regions,
            command_info: None,
            kind,
        })
    }

    pub(super) fn prepend_nested_regions(
        &mut self,
        command: CommandId,
        regions: Vec<IsolatedRegion>,
    ) {
        if regions.is_empty() {
            return;
        }

        let existing = self.recorded_program.command(command).nested_regions;
        let mut merged = regions;
        merged.extend_from_slice(self.recorded_program.nested_regions(existing));
        self.recorded_program.command_mut(command).nested_regions =
            self.recorded_program.push_regions(merged);
    }

    pub(super) fn visit_stmt_seq(
        &mut self,
        commands: &'a StmtSeq,
        flow: FlowState,
    ) -> RecordedCommandRange {
        let mut recorded = Vec::with_capacity(commands.len());
        self.visit_stmt_seq_into(commands, flow, &mut recorded);
        self.recorded_program.push_command_ids(recorded)
    }

    pub(super) fn visit_stmt_seq_into(
        &mut self,
        commands: &'a StmtSeq,
        mut flow: FlowState,
        recorded: &mut Vec<CommandId>,
    ) {
        recorded.reserve(commands.len());
        for stmt in commands.iter() {
            let command = self.visit_stmt(stmt, flow);
            self.recorded_program
                .push_statement_sequence_command(commands.span, stmt.span);
            self.observer
                .recorded_statement_sequence_command(commands.span, stmt.span, command);
            recorded.push(command);
            flow = self.flow_after_statement(stmt, flow);
        }
    }

    fn flow_after_statement(&self, stmt: &'a Stmt, flow: FlowState) -> FlowState {
        self.flow_after_command(&stmt.command, flow)
    }

    fn flow_after_command(&self, command: &'a Command, flow: FlowState) -> FlowState {
        match command {
            Command::Simple(_) => {
                let info = recorded_command_info(command, self.source, self.runtime.bash_enabled());
                flow_after_zsh_effects(flow, &info.zsh_effects)
            }
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                let after_block = self.flow_after_stmt_seq(
                    commands,
                    FlowState {
                        in_block: true,
                        ..flow
                    },
                );
                flow_after_nested_current_shell_effects(flow, after_block)
            }
            Command::Compound(CompoundCommand::If(command)) => {
                self.flow_after_stmt_seq(&command.condition, flow)
            }
            Command::Compound(CompoundCommand::While(command)) => {
                self.flow_after_stmt_seq(&command.condition, flow)
            }
            Command::Compound(CompoundCommand::Until(command)) => {
                self.flow_after_stmt_seq(&command.condition, flow)
            }
            Command::Compound(CompoundCommand::Always(command)) => {
                let block_flow = FlowState {
                    in_block: true,
                    ..flow
                };
                let after_body = self.flow_after_stmt_seq(&command.body, block_flow);
                let after_always = self.flow_after_stmt_seq(&command.always_body, after_body);
                flow_after_nested_current_shell_effects(flow, after_always)
            }
            Command::Compound(CompoundCommand::Time(command)) => command
                .command
                .as_deref()
                .map_or(flow, |command| self.flow_after_statement(command, flow)),
            Command::Binary(command) => match command.op {
                BinaryOp::And | BinaryOp::Or => self.flow_after_statement(&command.left, flow),
                BinaryOp::Pipe | BinaryOp::PipeAll
                    if self.shell_profile.dialect == ShellDialect::Zsh
                        && flow.pipeline_tail_runs_in_current_shell =>
                {
                    self.flow_after_statement(&command.right, flow)
                }
                BinaryOp::Pipe | BinaryOp::PipeAll => flow,
            },
            _ => flow,
        }
    }

    fn flow_after_stmt_seq(&self, commands: &'a StmtSeq, mut flow: FlowState) -> FlowState {
        for stmt in commands.iter() {
            flow = self.flow_after_statement(stmt, flow);
        }
        flow
    }

    pub(super) fn visit_stmt(&mut self, stmt: &'a Stmt, flow: FlowState) -> CommandId {
        let span = semantic_statement_span(stmt);
        let scope = self.current_scope();
        let context = Self::flow_context(flow);
        self.flow_contexts.push((span, context));
        self.observer.enter_command(&stmt.command, scope, context);
        self.command_stack.push(span);

        let recorded = self.visit_command(&stmt.command, flow);
        let redirects = self.visit_redirects(&stmt.redirects, flow);
        if !redirects.is_empty() {
            self.prepend_nested_regions(recorded, redirects);
        }
        self.recorded_program.command_mut(recorded).span = span;
        self.recorded_program.command_mut(recorded).syntax_kind =
            Some(CommandKind::from_command(&stmt.command));
        self.recorded_program.command_mut(recorded).scope = Some(scope);
        self.recorded_program.command_mut(recorded).flow_context = Some(context);
        let info = recorded_command_info(&stmt.command, self.source, self.runtime.bash_enabled());
        if !info.is_empty() {
            let info_id = self.recorded_program.push_command_info(info.clone());
            self.recorded_program.command_mut(recorded).command_info = Some(info_id);
            self.recorded_program
                .command_infos
                .entry(SpanKey::new(span))
                .and_modify(|existing| {
                    if existing.is_empty() {
                        *existing = info.clone();
                    }
                })
                .or_insert(info);
        }

        self.command_stack.pop();
        self.observer
            .recorded_command(recorded, stmt, scope, context);
        self.observer.exit_command(&stmt.command, scope);
        recorded
    }

    pub(super) fn visit_command(&mut self, command: &'a Command, flow: FlowState) -> CommandId {
        match command {
            Command::Simple(command) => self.visit_simple_command(command, flow),
            Command::Builtin(command) => self.visit_builtin(command, flow),
            Command::Decl(command) => self.visit_decl(command, flow),
            Command::Binary(command) => self.visit_binary(command, flow),
            Command::Compound(command) => self.visit_compound(command, flow),
            Command::Function(command) => self.visit_function(command, flow),
            Command::AnonymousFunction(command) => self.visit_anonymous_function(command, flow),
        }
    }

    pub(super) fn visit_simple_command(
        &mut self,
        command: &'a shuck_ast::SimpleCommand,
        flow: FlowState,
    ) -> CommandId {
        let mut nested_regions = Vec::new();
        let command_has_name = simple_command_has_name(command, self.source);
        for assignment in &command.assignments {
            if command_has_name {
                self.visit_assignment_reads_into(assignment, flow, &mut nested_regions);
            } else {
                self.visit_assignment_into(
                    assignment,
                    None,
                    BindingAttributes::empty(),
                    flow,
                    &mut nested_regions,
                );
            }
        }

        self.visit_word_into(
            &command.name,
            WordVisitKind::Expansion,
            flow,
            &mut nested_regions,
        );
        self.visit_words_into(
            &command.args,
            WordVisitKind::Expansion,
            flow,
            &mut nested_regions,
        );

        let command_words = std::iter::once(&command.name)
            .chain(command.args.iter())
            .collect::<Vec<_>>();
        let normalized = normalize_command_words(&command_words, self.source)
            .expect("simple commands always have a name");
        if !flow.in_subshell
            && let Some(collector) = self.file_entry_contract_collector.as_deref_mut()
        {
            collector.observe_simple_command(&normalized);
        }

        if let Some(name) = normalized.literal_name.as_deref()
            && !name.is_empty()
        {
            let callee = Name::from(name);
            let scope = self.current_scope();
            let call_site = CallSite {
                callee: callee.clone(),
                span: command.span,
                name_span: command.name.span,
                scope,
                arg_count: command.args.len(),
            };
            self.call_sites
                .entry(callee.clone())
                .or_default()
                .push(call_site);
            self.recorded_program.call_command_spans.insert(
                SpanKey::new(command.span),
                self.command_stack.last().copied().unwrap_or(command.span),
            );
        }

        if let Some(name) = normalized.effective_name.as_deref()
            && !name.is_empty()
        {
            let callee = Name::from(name);
            if resolved_command_can_affect_current_shell(&normalized) {
                self.classify_special_simple_command(&callee, &normalized, command.span, flow);
            }
        }

        self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
    }

    pub(super) fn visit_builtin(
        &mut self,
        command: &'a BuiltinCommand,
        flow: FlowState,
    ) -> CommandId {
        match command {
            BuiltinCommand::Break(command) => {
                let nested_regions = self.visit_builtin_parts(
                    &command.assignments,
                    command.depth.as_ref(),
                    &command.extra_args,
                    flow,
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::Break {
                        depth: depth_from_word(command.depth.as_ref()),
                    },
                )
            }
            BuiltinCommand::Continue(command) => {
                let nested_regions = self.visit_builtin_parts(
                    &command.assignments,
                    command.depth.as_ref(),
                    &command.extra_args,
                    flow,
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::Continue {
                        depth: depth_from_word(command.depth.as_ref()),
                    },
                )
            }
            BuiltinCommand::Return(command) => {
                let nested_regions = self.visit_builtin_parts(
                    &command.assignments,
                    command.code.as_ref(),
                    &command.extra_args,
                    flow,
                );
                self.record_command(command.span, nested_regions, RecordedCommandKind::Return)
            }
            BuiltinCommand::Exit(command) => {
                let nested_regions = self.visit_builtin_parts(
                    &command.assignments,
                    command.code.as_ref(),
                    &command.extra_args,
                    flow,
                );
                self.record_command(command.span, nested_regions, RecordedCommandKind::Exit)
            }
        }
    }

    pub(super) fn visit_builtin_parts(
        &mut self,
        assignments: &'a [Assignment],
        primary_word: Option<&'a Word>,
        extra_words: &'a [Word],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for assignment in assignments {
            self.visit_assignment_reads_into(assignment, flow, &mut nested_regions);
        }
        if let Some(word) = primary_word {
            self.visit_word_into(word, WordVisitKind::Expansion, flow, &mut nested_regions);
        }
        self.visit_words_into(
            extra_words,
            WordVisitKind::Expansion,
            flow,
            &mut nested_regions,
        );
        nested_regions
    }

    pub(super) fn visit_decl(
        &mut self,
        command: &'a shuck_ast::DeclClause,
        flow: FlowState,
    ) -> CommandId {
        let mut nested_regions = Vec::new();
        for assignment in &command.assignments {
            self.visit_assignment_reads_into(assignment, flow, &mut nested_regions);
        }

        let builtin = declaration_builtin(&command.variant);
        let flags = declaration_flags(command.variant.as_str(), &command.operands, self.source);
        let global_flag_enabled =
            declaration_flag_is_enabled(&command.operands, self.source, 'g').unwrap_or(false);
        self.declarations.push(Declaration {
            builtin,
            span: command.span,
            operands: declaration_operands(&command.operands, self.source),
        });

        let mut name_operands_are_function_names = false;
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) => {
                    update_declaration_function_name_mode(
                        word,
                        self.source,
                        &mut name_operands_are_function_names,
                    );
                    self.visit_word_into(word, WordVisitKind::Expansion, flow, &mut nested_regions);
                }
                DeclOperand::Dynamic(word) => {
                    self.visit_word_into(word, WordVisitKind::Expansion, flow, &mut nested_regions);
                }
                DeclOperand::Name(name) => {
                    self.visit_var_ref_subscript_words(
                        Some(&name.name),
                        name.subscript.as_deref(),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                    if !name_operands_are_function_names {
                        self.visit_name_only_declaration_operand(
                            builtin,
                            &flags,
                            global_flag_enabled,
                            &name.name,
                            name.span,
                        );
                    }
                }
                DeclOperand::Assignment(assignment) => {
                    let (scope, mut attributes) =
                        self.declaration_scope_and_attributes(builtin, &flags, global_flag_enabled);
                    attributes |= BindingAttributes::DECLARATION_INITIALIZED;
                    if flags.contains(&'p') {
                        attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
                    }
                    let kind = if attributes.contains(BindingAttributes::NAMEREF) {
                        BindingKind::Nameref
                    } else {
                        BindingKind::Declaration(builtin)
                    };
                    self.visit_assignment_into(
                        assignment,
                        Some((kind, scope)),
                        attributes,
                        flow,
                        &mut nested_regions,
                    );
                }
            }
        }

        self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
    }

    pub(super) fn visit_binary(
        &mut self,
        command: &'a BinaryCommand,
        flow: FlowState,
    ) -> CommandId {
        match command.op {
            BinaryOp::And | BinaryOp::Or => self.visit_logical_binary(command, flow),
            BinaryOp::Pipe | BinaryOp::PipeAll => self.visit_pipeline_binary(command, flow),
        }
    }

    pub(super) fn visit_pipeline_binary(
        &mut self,
        command: &'a BinaryCommand,
        flow: FlowState,
    ) -> CommandId {
        let tail_runs_in_current_shell = self.shell_profile.dialect == ShellDialect::Zsh
            && flow.pipeline_tail_runs_in_current_shell;
        let pipeline_child_flow = FlowState {
            in_subshell: true,
            pipeline_tail_runs_in_current_shell: false,
            ..flow
        };
        let mut segments = Vec::with_capacity(2);
        for (stmt, operator_before) in [
            (&command.left, None),
            (
                &command.right,
                Some(RecordedPipelineOperator {
                    operator: recorded_pipeline_operator(command.op),
                    span: command.op_span,
                }),
            ),
        ] {
            let is_tail_segment = operator_before.is_some();
            let segment_runs_in_current_shell = tail_runs_in_current_shell && is_tail_segment;
            let segment_is_nested_pipeline = matches!(
                &stmt.command,
                Command::Binary(binary) if matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
            );
            let scope = if segment_is_nested_pipeline || segment_runs_in_current_shell {
                self.current_scope()
            } else {
                self.push_scope(ScopeKind::Pipeline, self.current_scope(), stmt.span)
            };
            let segment_flow = if segment_runs_in_current_shell {
                FlowState {
                    pipeline_tail_runs_in_current_shell: true,
                    ..flow
                }
            } else {
                pipeline_child_flow
            };
            let recorded = self.visit_stmt(stmt, segment_flow);
            if !segment_is_nested_pipeline && !segment_runs_in_current_shell {
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
            }
            segments.push(RecordedPipelineSegment {
                operator_before,
                scope,
                command: recorded,
            });
        }

        let segments = self.recorded_program.push_pipeline_segments(segments);
        self.record_command(
            command.span,
            Vec::new(),
            RecordedCommandKind::Pipeline { segments },
        )
    }

    pub(super) fn visit_logical_binary(
        &mut self,
        command: &'a BinaryCommand,
        flow: FlowState,
    ) -> CommandId {
        let mut left_flow = flow;
        left_flow.exit_status_checked = true;
        let first = self.visit_stmt(&command.left, left_flow);
        let mut right_flow = flow;
        right_flow.exit_status_checked = flow.exit_status_checked;
        right_flow.conditionally_executed = true;
        let right = self.visit_stmt(&command.right, right_flow);
        let rest = self
            .recorded_program
            .push_list_items(vec![RecordedListItem {
                operator: recorded_list_operator(command.op),
                operator_span: command.op_span,
                command: right,
            }]);

        self.record_command(
            command.span,
            Vec::new(),
            RecordedCommandKind::List { first, rest },
        )
    }

    pub(super) fn visit_compound(
        &mut self,
        command: &'a CompoundCommand,
        flow: FlowState,
    ) -> CommandId {
        match command {
            CompoundCommand::If(command) => {
                let condition = self.visit_stmt_seq(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let then_branch = self.visit_stmt_seq(&command.then_branch, flow.conditional());
                let elif_branches = command
                    .elif_branches
                    .iter()
                    .map(|(condition, body)| RecordedElifBranch {
                        condition: self.visit_stmt_seq(
                            condition,
                            FlowState {
                                exit_status_checked: true,
                                ..flow.conditional()
                            },
                        ),
                        body: self.visit_stmt_seq(body, flow.conditional()),
                    })
                    .collect();
                let elif_branches = self.recorded_program.push_elif_branches(elif_branches);
                let else_branch = command
                    .else_branch
                    .as_ref()
                    .map(|body| self.visit_stmt_seq(body, flow.conditional()))
                    .unwrap_or_default();

                self.record_command(
                    command.span,
                    Vec::new(),
                    RecordedCommandKind::If {
                        condition,
                        then_branch,
                        elif_branches,
                        else_branch,
                    },
                )
            }
            CompoundCommand::For(command) => {
                let nested_regions = command
                    .words
                    .as_deref()
                    .map(|words| self.visit_words(words, WordVisitKind::Expansion, flow))
                    .unwrap_or_default();
                for target in &command.targets {
                    if let Some(name) = &target.name {
                        self.add_binding(
                            name,
                            BindingKind::LoopVariable,
                            self.current_scope(),
                            target.span,
                            BindingOrigin::LoopVariable {
                                definition_span: target.span,
                                items: loop_binding_origin_for_words(command.words.as_deref()),
                            },
                            BindingAttributes::empty(),
                        );
                    }
                }

                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::For { body },
                )
            }
            CompoundCommand::Repeat(command) => {
                let nested_regions =
                    self.visit_word(&command.count, WordVisitKind::Expansion, flow);
                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::For { body },
                )
            }
            CompoundCommand::Foreach(command) => {
                let nested_regions =
                    self.visit_words(&command.words, WordVisitKind::Expansion, flow);
                self.add_binding(
                    &command.variable,
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingOrigin::LoopVariable {
                        definition_span: command.variable_span,
                        items: loop_binding_origin_for_words(Some(&command.words)),
                    },
                    BindingAttributes::empty(),
                );

                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::For { body },
                )
            }
            CompoundCommand::ArithmeticFor(command) => {
                let mut nested_regions = Vec::new();
                self.visit_optional_arithmetic_expr_into(
                    command.init_ast.as_ref(),
                    flow,
                    &mut nested_regions,
                );
                self.visit_optional_arithmetic_expr_into(
                    command.condition_ast.as_ref(),
                    flow,
                    &mut nested_regions,
                );
                self.visit_optional_arithmetic_expr_into(
                    command.step_ast.as_ref(),
                    flow,
                    &mut nested_regions,
                );
                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::ArithmeticFor { body },
                )
            }
            CompoundCommand::While(command) => {
                let condition = self.visit_stmt_seq(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    Vec::new(),
                    RecordedCommandKind::While { condition, body },
                )
            }
            CompoundCommand::Until(command) => {
                let condition = self.visit_stmt_seq(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    Vec::new(),
                    RecordedCommandKind::Until { condition, body },
                )
            }
            CompoundCommand::Case(command) => {
                let nested_regions = self.visit_word(&command.word, WordVisitKind::Expansion, flow);

                let arms = command
                    .cases
                    .iter()
                    .map(|case| {
                        let pattern_regions =
                            self.visit_patterns(&case.patterns, WordVisitKind::Conditional, flow);
                        let mut commands = Vec::with_capacity(case.body.len());
                        self.visit_stmt_seq_into(&case.body, flow.conditional(), &mut commands);
                        if !pattern_regions.is_empty() {
                            if let Some(&first) = commands.first() {
                                self.prepend_nested_regions(first, pattern_regions);
                            } else {
                                commands.push(self.record_command(
                                    command.span,
                                    pattern_regions,
                                    RecordedCommandKind::Linear,
                                ));
                            }
                        }
                        RecordedCaseArm {
                            terminator: case.terminator,
                            matches_anything: case_arm_matches_anything(&case.patterns),
                            commands: self.recorded_program.push_command_ids(commands),
                        }
                    })
                    .collect();

                let arms = self.recorded_program.push_case_arms(arms);
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::Case { arms },
                )
            }
            CompoundCommand::Select(command) => {
                let nested_regions =
                    self.visit_words(&command.words, WordVisitKind::Expansion, flow);
                self.add_binding(
                    &command.variable,
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingOrigin::LoopVariable {
                        definition_span: command.variable_span,
                        items: loop_binding_origin_for_words(Some(&command.words)),
                    },
                    BindingAttributes::empty(),
                );

                let body = self.visit_stmt_seq(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    command.span,
                    nested_regions,
                    RecordedCommandKind::Select { body },
                )
            }
            CompoundCommand::Subshell(commands) => {
                let scope = self.push_scope(
                    ScopeKind::Subshell,
                    self.current_scope(),
                    command_span_from_compound(command),
                );
                let body = self.visit_stmt_seq(
                    commands,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);

                self.record_command(
                    command_span_from_compound(command),
                    Vec::new(),
                    RecordedCommandKind::Subshell { body },
                )
            }
            CompoundCommand::BraceGroup(commands) => {
                let body = self.visit_stmt_seq(
                    commands,
                    FlowState {
                        in_block: true,
                        ..flow
                    },
                );
                self.record_command(
                    command_span_from_compound(command),
                    Vec::new(),
                    RecordedCommandKind::BraceGroup { body },
                )
            }
            CompoundCommand::Always(command) => {
                let block_flow = FlowState {
                    in_block: true,
                    ..flow
                };
                let body = self.visit_stmt_seq(&command.body, block_flow);
                let always_body = self.visit_stmt_seq(&command.always_body, block_flow);
                self.record_command(
                    command.span,
                    Vec::new(),
                    RecordedCommandKind::Always { body, always_body },
                )
            }
            CompoundCommand::Arithmetic(command) => {
                let nested_regions =
                    self.visit_optional_arithmetic_expr(command.expr_ast.as_ref(), flow);
                self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommand::Time(command) => {
                let mut nested_regions = Vec::new();
                if let Some(command) = &command.command {
                    let command_id = self.visit_stmt(command, flow);
                    nested_regions.extend(self.flatten_recorded_regions(command_id));
                }
                self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommand::Conditional(command) => {
                let nested_regions =
                    self.visit_conditional_expr(command.span, &command.expression, flow);
                self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommand::Coproc(command) => {
                let body_command = self.visit_stmt(
                    &command.body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
                let nested_regions = self.flatten_recorded_regions(body_command);
                self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
            }
        }
    }

    pub(super) fn visit_function(
        &mut self,
        function: &'a FunctionDef,
        flow: FlowState,
    ) -> CommandId {
        let mut nested_regions = Vec::new();
        for entry in &function.header.entries {
            self.visit_word_into(
                &entry.word,
                WordVisitKind::Expansion,
                flow,
                &mut nested_regions,
            );
        }

        let parent_scope = self.current_scope();
        let scope = self.push_scope(
            ScopeKind::Function(function_scope_kind(function)),
            parent_scope,
            body_span(&function.body),
        );
        for (name, span) in function.static_name_entries() {
            let binding_id = self.add_binding(
                name,
                BindingKind::FunctionDefinition,
                parent_scope,
                span,
                BindingOrigin::FunctionDefinition {
                    definition_span: function.span,
                },
                BindingAttributes::empty(),
            );
            self.recorded_program
                .function_body_scopes
                .insert(binding_id, scope);
        }
        self.deferred_functions.push(DeferredFunction {
            function,
            scope,
            flow,
        });
        self.pop_scope(scope);

        self.record_command(function.span, nested_regions, RecordedCommandKind::Linear)
    }

    pub(super) fn visit_anonymous_function(
        &mut self,
        function: &'a AnonymousFunctionCommand,
        flow: FlowState,
    ) -> CommandId {
        let nested_regions = self.visit_words(&function.args, WordVisitKind::Expansion, flow);
        let scope = self.push_scope(
            ScopeKind::Function(FunctionScopeKind::Anonymous),
            self.current_scope(),
            body_span(&function.body),
        );
        let body = self.visit_function_like_body(&function.body, flow);
        self.pop_scope(scope);
        self.mark_scope_completed(scope);

        self.record_command(
            function.span,
            nested_regions,
            RecordedCommandKind::BraceGroup { body },
        )
    }

    pub(super) fn push_scope(&mut self, kind: ScopeKind, parent: ScopeId, span: Span) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope {
            id,
            kind,
            parent: Some(parent),
            span,
            bindings: FxHashMap::default(),
        });
        self.scope_stack.push(id);
        id
    }

    pub(super) fn pop_scope(&mut self, expected: ScopeId) {
        let popped = self.scope_stack.pop();
        debug_assert_eq!(popped, Some(expected));
    }

    pub(super) fn mark_scope_completed(&mut self, scope: ScopeId) {
        self.completed_scopes.insert(scope);
    }

    pub(super) fn drain_deferred_functions(&mut self) {
        while !self.deferred_functions.is_empty() {
            let deferred_functions = std::mem::take(&mut self.deferred_functions);
            for deferred in deferred_functions {
                self.rebuild_scope_stack(deferred.scope);
                let commands =
                    self.visit_function_like_body(&deferred.function.body, deferred.flow);
                self.recorded_program
                    .set_function_body(deferred.scope, commands);
                self.mark_scope_completed(deferred.scope);
            }
        }
        self.rebuild_scope_stack(ScopeId(0));
        self.command_stack.clear();
    }

    pub(super) fn visit_function_like_body(
        &mut self,
        body: &'a Stmt,
        flow: FlowState,
    ) -> RecordedCommandRange {
        let flow = FlowState {
            in_function: true,
            ..flow
        };

        let command = self.visit_stmt(body, flow);
        self.recorded_program.push_command_ids(vec![command])
    }

    pub(super) fn rebuild_scope_stack(&mut self, scope: ScopeId) {
        self.scope_stack = ancestor_scopes(&self.scopes, scope).collect::<Vec<_>>();
        self.scope_stack.reverse();
    }

    pub(super) fn flatten_recorded_regions(&self, recorded: CommandId) -> Vec<IsolatedRegion> {
        let recorded = self.recorded_program.command(recorded);
        let mut regions = self
            .recorded_program
            .nested_regions(recorded.nested_regions)
            .to_vec();

        match recorded.kind {
            RecordedCommandKind::Linear
            | RecordedCommandKind::Break { .. }
            | RecordedCommandKind::Continue { .. }
            | RecordedCommandKind::Return
            | RecordedCommandKind::Exit => {}
            RecordedCommandKind::List { first, rest } => {
                regions.extend(self.flatten_recorded_regions(first));
                for item in self.recorded_program.list_items(rest) {
                    regions.extend(self.flatten_recorded_regions(item.command));
                }
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                for &command in self.recorded_program.commands_in(condition) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
                for &command in self.recorded_program.commands_in(then_branch) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
                for branch in self.recorded_program.elif_branches(elif_branches) {
                    for &command in self.recorded_program.commands_in(branch.condition) {
                        regions.extend(self.flatten_recorded_regions(command));
                    }
                    for &command in self.recorded_program.commands_in(branch.body) {
                        regions.extend(self.flatten_recorded_regions(command));
                    }
                }
                for &command in self.recorded_program.commands_in(else_branch) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::While { condition, body }
            | RecordedCommandKind::Until { condition, body } => {
                for &command in self.recorded_program.commands_in(condition) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
                for &command in self.recorded_program.commands_in(body) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body }
            | RecordedCommandKind::BraceGroup { body }
            | RecordedCommandKind::Subshell { body } => {
                for &command in self.recorded_program.commands_in(body) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::Always { body, always_body } => {
                for &command in self.recorded_program.commands_in(body) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
                for &command in self.recorded_program.commands_in(always_body) {
                    regions.extend(self.flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::Case { arms } => {
                for arm in self.recorded_program.case_arms(arms) {
                    for &command in self.recorded_program.commands_in(arm.commands) {
                        regions.extend(self.flatten_recorded_regions(command));
                    }
                }
            }
            RecordedCommandKind::Pipeline { segments } => {
                for segment in self.recorded_program.pipeline_segments(segments) {
                    regions.extend(self.flatten_recorded_regions(segment.command));
                }
            }
        }

        regions
    }

    pub(super) fn current_scope(&self) -> ScopeId {
        *self.scope_stack.last().unwrap_or(&ScopeId(0))
    }

    pub(super) fn nearest_function_scope(&self) -> Option<ScopeId> {
        self.scope_stack
            .iter()
            .rev()
            .copied()
            .find(|scope| matches!(self.scopes[scope.index()].kind, ScopeKind::Function(_)))
    }

    pub(super) fn nearest_execution_scope(&self) -> ScopeId {
        self.scope_stack
            .iter()
            .rev()
            .copied()
            .find(|scope| !matches!(self.scopes[scope.index()].kind, ScopeKind::Function(_)))
            .unwrap_or(ScopeId(0))
    }
}

fn flow_after_zsh_effects(mut flow: FlowState, effects: &[RecordedZshCommandEffect]) -> FlowState {
    for effect in effects {
        match effect {
            RecordedZshCommandEffect::Emulate { mode, .. } => {
                flow = flow.with_zsh_emulation(Some(*mode));
            }
            RecordedZshCommandEffect::EmulateUnknown { .. } => {
                flow = flow.with_zsh_emulation(None);
            }
            RecordedZshCommandEffect::SetOptions { .. } => {}
        }
    }
    flow
}

fn flow_after_nested_current_shell_effects(flow: FlowState, after_nested: FlowState) -> FlowState {
    FlowState {
        pipeline_tail_runs_in_current_shell: after_nested.pipeline_tail_runs_in_current_shell,
        ..flow
    }
}
