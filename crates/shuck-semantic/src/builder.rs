use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticAssignOp, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticLvalue, ArithmeticUnaryOp, ArrayElem, ArrayExpr, ArrayKind, Assignment,
    AssignmentValue, BinaryCommand, BinaryOp, BourneParameterExpansion, BuiltinCommand, Command,
    CompoundCommand, ConditionalExpr, DeclOperand, File, FunctionDef, HeredocBody, HeredocBodyPart,
    HeredocBodyPartNode, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern,
    PatternGroupKind, PatternPart, PatternPartNode, Span, Stmt, StmtSeq, Subscript, VarRef, Word,
    WordPart, WordPartNode, ZshExpansionOperation, ZshExpansionTarget, ZshGlobSegment,
};
use shuck_indexer::Indexer;
use shuck_parser::{ShellProfile, ZshEmulationMode};
use smallvec::SmallVec;

use crate::binding::{
    AssignmentValueOrigin, Binding, BindingAttributes, BindingKind, BindingOrigin,
    BuiltinBindingTargetKind, LoopValueOrigin,
};
use crate::call_graph::{CallGraph, CallSite, OverwrittenFunction};
use crate::cfg::{
    FlowContext, IsolatedRegion, RecordedCaseArm, RecordedCommand, RecordedCommandId,
    RecordedCommandInfo, RecordedCommandKind, RecordedCommandRange, RecordedElifBranch,
    RecordedListItem, RecordedListOperator, RecordedPipelineSegment, RecordedProgram,
    RecordedZshCommandEffect, RecordedZshOptionUpdate,
};
use crate::declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
use crate::reference::{Reference, ReferenceKind};
use crate::runtime::RuntimePrelude;
use crate::source_closure::source_path_template;
use crate::source_ref::{
    SourceRef, SourceRefDiagnosticClass, SourceRefKind, SourceRefResolution,
    default_diagnostic_class,
};
use crate::{
    BindingId, FunctionScopeKind, IndirectTargetHint, ReferenceId, Scope, ScopeId, ScopeKind,
    SourceDirectiveOverride, SpanKey, TraversalObserver,
};

pub(crate) struct BuildOutput {
    pub(crate) shell_profile: ShellProfile,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) references: Vec<Reference>,
    pub(crate) reference_index: FxHashMap<Name, SmallVec<[ReferenceId; 2]>>,
    pub(crate) predefined_runtime_refs: FxHashSet<ReferenceId>,
    pub(crate) guarded_parameter_refs: FxHashSet<ReferenceId>,
    pub(crate) binding_index: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    pub(crate) resolved: FxHashMap<ReferenceId, BindingId>,
    pub(crate) unresolved: Vec<ReferenceId>,
    pub(crate) functions: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    pub(crate) call_sites: FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    pub(crate) call_graph: CallGraph,
    pub(crate) source_refs: Vec<SourceRef>,
    pub(crate) runtime: RuntimePrelude,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) indirect_target_hints: FxHashMap<BindingId, IndirectTargetHint>,
    pub(crate) indirect_expansion_refs: FxHashSet<ReferenceId>,
    pub(crate) flow_contexts: Vec<(Span, FlowContext)>,
    pub(crate) recorded_program: RecordedProgram,
    pub(crate) command_bindings: FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    pub(crate) command_references: FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    pub(crate) heuristic_unused_assignments: Vec<BindingId>,
}

pub(crate) struct SemanticModelBuilder<'a, 'observer> {
    source: &'a str,
    observer: &'observer mut dyn TraversalObserver,
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    reference_index: FxHashMap<Name, SmallVec<[ReferenceId; 2]>>,
    predefined_runtime_refs: FxHashSet<ReferenceId>,
    guarded_parameter_refs: FxHashSet<ReferenceId>,
    binding_index: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    call_sites: FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    source_refs: Vec<SourceRef>,
    declarations: Vec<Declaration>,
    indirect_target_hints: FxHashMap<BindingId, IndirectTargetHint>,
    indirect_expansion_refs: FxHashSet<ReferenceId>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_program: RecordedProgram,
    command_bindings: FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    source_directives: FxHashMap<usize, SourceDirectiveOverride>,
    runtime: RuntimePrelude,
    completed_scopes: FxHashSet<ScopeId>,
    deferred_functions: Vec<DeferredFunction>,
    scope_stack: Vec<ScopeId>,
    command_stack: Vec<Span>,
    guarded_parameter_operand_depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FlowState {
    in_function: bool,
    loop_depth: u32,
    in_subshell: bool,
    in_block: bool,
    exit_status_checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordVisitKind {
    Expansion,
    Conditional,
}

#[derive(Debug, Clone, Copy)]
struct DeferredFunction {
    function: *const FunctionDef,
    scope: ScopeId,
    flow: FlowState,
}

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(crate) fn build(
        file: &'a File,
        source: &'a str,
        indexer: &'a Indexer,
        observer: &'observer mut dyn TraversalObserver,
        bash_runtime_vars_enabled: bool,
        shell_profile: ShellProfile,
    ) -> BuildOutput {
        let file_scope = Scope {
            id: ScopeId(0),
            kind: ScopeKind::File,
            parent: None,
            span: file.span,
            bindings: FxHashMap::default(),
        };
        let runtime = RuntimePrelude::new(bash_runtime_vars_enabled);
        let mut builder = Self {
            source,
            observer,
            scopes: vec![file_scope],
            bindings: Vec::new(),
            references: Vec::new(),
            reference_index: FxHashMap::default(),
            predefined_runtime_refs: FxHashSet::default(),
            guarded_parameter_refs: FxHashSet::default(),
            binding_index: FxHashMap::default(),
            resolved: FxHashMap::default(),
            unresolved: Vec::new(),
            functions: FxHashMap::default(),
            call_sites: FxHashMap::default(),
            source_refs: Vec::new(),
            declarations: Vec::new(),
            indirect_target_hints: FxHashMap::default(),
            indirect_expansion_refs: FxHashSet::default(),
            flow_contexts: Vec::new(),
            recorded_program: RecordedProgram::default(),
            command_bindings: FxHashMap::default(),
            command_references: FxHashMap::default(),
            source_directives: parse_source_directives(source, indexer),
            runtime,
            completed_scopes: FxHashSet::default(),
            deferred_functions: Vec::new(),
            scope_stack: vec![ScopeId(0)],
            command_stack: Vec::new(),
            guarded_parameter_operand_depth: 0,
        };
        let file_commands = builder.visit_stmt_seq(&file.body, FlowState::default());
        builder.recorded_program.set_file_commands(file_commands);
        builder.mark_scope_completed(ScopeId(0));
        builder.drain_deferred_functions();

        let call_graph = builder.build_call_graph();
        let heuristic_unused_assignments = builder.compute_heuristic_unused_assignments();

        BuildOutput {
            shell_profile,
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: builder.references,
            reference_index: builder.reference_index,
            predefined_runtime_refs: builder.predefined_runtime_refs,
            guarded_parameter_refs: builder.guarded_parameter_refs,
            binding_index: builder.binding_index,
            resolved: builder.resolved,
            unresolved: builder.unresolved,
            functions: builder.functions,
            call_sites: builder.call_sites,
            call_graph,
            source_refs: builder.source_refs,
            runtime: builder.runtime,
            declarations: builder.declarations,
            indirect_target_hints: builder.indirect_target_hints,
            indirect_expansion_refs: builder.indirect_expansion_refs,
            flow_contexts: builder.flow_contexts,
            recorded_program: builder.recorded_program,
            command_bindings: builder.command_bindings,
            command_references: builder.command_references,
            heuristic_unused_assignments,
        }
    }

    fn flow_context(flow: FlowState) -> FlowContext {
        FlowContext {
            in_function: flow.in_function,
            loop_depth: flow.loop_depth,
            in_subshell: flow.in_subshell,
            in_block: flow.in_block,
            exit_status_checked: flow.exit_status_checked,
        }
    }

    fn record_command(
        &mut self,
        span: Span,
        nested_regions: Vec<IsolatedRegion>,
        kind: RecordedCommandKind,
    ) -> RecordedCommandId {
        let nested_regions = self.recorded_program.push_regions(nested_regions);
        self.recorded_program.push_command(RecordedCommand {
            span,
            nested_regions,
            kind,
        })
    }

    fn prepend_nested_regions(&mut self, command: RecordedCommandId, regions: Vec<IsolatedRegion>) {
        if regions.is_empty() {
            return;
        }

        let existing = self.recorded_program.command(command).nested_regions;
        let mut merged = regions;
        merged.extend_from_slice(self.recorded_program.nested_regions(existing));
        self.recorded_program.command_mut(command).nested_regions =
            self.recorded_program.push_regions(merged);
    }

    fn visit_stmt_seq(&mut self, commands: &StmtSeq, flow: FlowState) -> RecordedCommandRange {
        let mut recorded = Vec::with_capacity(commands.len());
        self.visit_stmt_seq_into(commands, flow, &mut recorded);
        self.recorded_program.push_command_ids(recorded)
    }

    fn visit_stmt_seq_into(
        &mut self,
        commands: &StmtSeq,
        flow: FlowState,
        recorded: &mut Vec<RecordedCommandId>,
    ) {
        recorded.reserve(commands.len());
        for stmt in commands.iter() {
            recorded.push(self.visit_stmt(stmt, flow));
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, flow: FlowState) -> RecordedCommandId {
        let span = stmt.span;
        let scope = self.current_scope();
        let context = Self::flow_context(flow);
        self.flow_contexts.push((span, context.clone()));
        self.observer.enter_command(&stmt.command, scope, context);
        self.command_stack.push(span);

        let recorded = self.visit_command(&stmt.command, flow);
        let redirects = self.visit_redirects(&stmt.redirects, flow);
        if !redirects.is_empty() {
            self.prepend_nested_regions(recorded, redirects);
        }
        self.recorded_program.command_mut(recorded).span = span;
        self.recorded_program.command_infos.insert(
            SpanKey::new(span),
            recorded_command_info(&stmt.command, self.source, self.runtime.bash_enabled()),
        );

        self.command_stack.pop();
        self.observer.exit_command(&stmt.command, scope);
        recorded
    }

    fn visit_command(&mut self, command: &Command, flow: FlowState) -> RecordedCommandId {
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

    fn visit_simple_command(
        &mut self,
        command: &shuck_ast::SimpleCommand,
        flow: FlowState,
    ) -> RecordedCommandId {
        let mut nested_regions = Vec::new();
        let command_has_name = simple_command_has_name(command, self.source);
        for assignment in &command.assignments {
            if command_has_name {
                self.visit_assignment_value_into(assignment, flow, &mut nested_regions);
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

        if let Some(name) = static_command_name_text(&command.name, self.source)
            && !name.is_empty()
        {
            let callee = Name::from(name.as_str());
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

            self.classify_special_simple_command(&callee, command, flow);
        }

        self.record_command(command.span, nested_regions, RecordedCommandKind::Linear)
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand, flow: FlowState) -> RecordedCommandId {
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

    fn visit_builtin_parts(
        &mut self,
        assignments: &[Assignment],
        primary_word: Option<&Word>,
        extra_words: &[Word],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for assignment in assignments {
            self.visit_assignment_into(
                assignment,
                None,
                BindingAttributes::empty(),
                flow,
                &mut nested_regions,
            );
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

    fn visit_decl(
        &mut self,
        command: &shuck_ast::DeclClause,
        flow: FlowState,
    ) -> RecordedCommandId {
        let mut nested_regions = Vec::new();
        for assignment in &command.assignments {
            self.visit_assignment_into(
                assignment,
                None,
                BindingAttributes::empty(),
                flow,
                &mut nested_regions,
            );
        }

        let builtin = declaration_builtin(&command.variant);
        let flags = declaration_flags(&command.operands, self.source);
        let global_flag_enabled =
            declaration_flag_is_enabled(&command.operands, self.source, 'g').unwrap_or(false);
        self.declarations.push(Declaration {
            builtin,
            span: command.span,
            operands: declaration_operands(&command.operands, self.source),
        });

        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                    self.visit_word_into(word, WordVisitKind::Expansion, flow, &mut nested_regions);
                }
                DeclOperand::Name(name) => {
                    self.visit_var_ref_subscript_words(
                        Some(&name.name),
                        name.subscript.as_ref(),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                    self.visit_name_only_declaration_operand(
                        builtin,
                        &flags,
                        global_flag_enabled,
                        &name.name,
                        name.span,
                    );
                }
                DeclOperand::Assignment(assignment) => {
                    let (scope, mut attributes) =
                        self.declaration_scope_and_attributes(builtin, &flags, global_flag_enabled);
                    attributes |= BindingAttributes::DECLARATION_INITIALIZED;
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

    fn visit_binary(&mut self, command: &BinaryCommand, flow: FlowState) -> RecordedCommandId {
        match command.op {
            BinaryOp::And | BinaryOp::Or => self.visit_logical_binary(command, flow),
            BinaryOp::Pipe | BinaryOp::PipeAll => self.visit_pipeline_binary(command, flow),
        }
    }

    fn visit_pipeline_binary(
        &mut self,
        command: &BinaryCommand,
        mut flow: FlowState,
    ) -> RecordedCommandId {
        flow.in_subshell = true;
        let mut commands = SmallVec::<[&Stmt; 4]>::new();
        collect_pipeline_segments(&command.left, &mut commands);
        collect_pipeline_segments(&command.right, &mut commands);

        let mut segments = Vec::with_capacity(commands.len());
        for stmt in commands {
            let scope = self.push_scope(ScopeKind::Pipeline, self.current_scope(), stmt.span);
            let recorded = self.visit_stmt(stmt, flow);
            self.pop_scope(scope);
            self.mark_scope_completed(scope);
            segments.push(RecordedPipelineSegment {
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

    fn visit_logical_binary(
        &mut self,
        command: &BinaryCommand,
        flow: FlowState,
    ) -> RecordedCommandId {
        let mut operators = SmallVec::<[RecordedListOperator; 4]>::new();
        let mut commands = SmallVec::<[&Stmt; 4]>::new();
        collect_logical_segments(&command.left, &mut commands, &mut operators);
        operators.push(recorded_list_operator(command.op));
        collect_logical_segments(&command.right, &mut commands, &mut operators);

        let mut recorded = SmallVec::<[RecordedCommandId; 4]>::with_capacity(commands.len());
        for (index, stmt) in commands.into_iter().enumerate() {
            let mut nested = flow;
            nested.exit_status_checked = operators.get(index).is_some() || flow.exit_status_checked;
            recorded.push(self.visit_stmt(stmt, nested));
        }

        let mut recorded = recorded.into_iter();
        let first = recorded
            .next()
            .expect("logical lists have at least one command");
        let rest = self.recorded_program.push_list_items(
            operators
                .into_iter()
                .zip(recorded)
                .map(|(operator, command)| RecordedListItem { operator, command })
                .collect(),
        );

        self.record_command(
            command.span,
            Vec::new(),
            RecordedCommandKind::List { first, rest },
        )
    }

    fn visit_compound(&mut self, command: &CompoundCommand, flow: FlowState) -> RecordedCommandId {
        match command {
            CompoundCommand::If(command) => {
                let condition = self.visit_stmt_seq(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let then_branch = self.visit_stmt_seq(&command.then_branch, flow);
                let elif_branches = command
                    .elif_branches
                    .iter()
                    .map(|(condition, body)| RecordedElifBranch {
                        condition: self.visit_stmt_seq(
                            condition,
                            FlowState {
                                exit_status_checked: true,
                                ..flow
                            },
                        ),
                        body: self.visit_stmt_seq(body, flow),
                    })
                    .collect();
                let elif_branches = self.recorded_program.push_elif_branches(elif_branches);
                let else_branch = command
                    .else_branch
                    .as_ref()
                    .map(|body| self.visit_stmt_seq(body, flow))
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
                        ..flow
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
                        ..flow
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
                        ..flow
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
                        ..flow
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
                        ..flow
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
                        ..flow
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
                        self.visit_stmt_seq_into(&case.body, flow, &mut commands);
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
                        ..flow
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
                let mut body = Vec::with_capacity(command.body.len() + command.always_body.len());
                self.visit_stmt_seq_into(&command.body, block_flow, &mut body);
                self.visit_stmt_seq_into(&command.always_body, block_flow, &mut body);
                let body = self.recorded_program.push_command_ids(body);
                self.record_command(
                    command.span,
                    Vec::new(),
                    RecordedCommandKind::BraceGroup { body },
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
                let nested_regions = self.visit_conditional_expr(&command.expression, flow);
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

    fn visit_function(&mut self, function: &FunctionDef, flow: FlowState) -> RecordedCommandId {
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
                    definition_span: span,
                },
                BindingAttributes::empty(),
            );
            self.recorded_program
                .function_body_scopes
                .insert(binding_id, scope);
        }
        self.deferred_functions.push(DeferredFunction {
            function: function as *const FunctionDef,
            scope,
            flow,
        });
        self.pop_scope(scope);

        self.record_command(function.span, nested_regions, RecordedCommandKind::Linear)
    }

    fn visit_anonymous_function(
        &mut self,
        function: &AnonymousFunctionCommand,
        flow: FlowState,
    ) -> RecordedCommandId {
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

    fn visit_assignment_into(
        &mut self,
        assignment: &Assignment,
        declaration_kind: Option<(BindingKind, ScopeId)>,
        mut attributes: BindingAttributes,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        self.visit_var_ref_subscript_words(
            Some(&assignment.target.name),
            assignment.target.subscript.as_ref(),
            WordVisitKind::Expansion,
            flow,
            nested_regions,
        );
        self.visit_assignment_value_into(assignment, flow, nested_regions);
        let (kind, scope) = declaration_kind.unwrap_or_else(|| {
            let kind = if assignment.append {
                BindingKind::AppendAssignment
            } else if matches!(assignment.value, AssignmentValue::Compound(_))
                || assignment.target.subscript.is_some()
            {
                BindingKind::ArrayAssignment
            } else {
                BindingKind::Assignment
            };
            (kind, self.current_scope())
        });
        attributes |= assignment_binding_attributes(assignment);
        if assignment_has_empty_initializer(assignment, self.source) {
            attributes |= BindingAttributes::EMPTY_INITIALIZER;
        }
        if assignment.target.subscript.is_some()
            && !attributes.contains(BindingAttributes::ASSOC)
            && self
                .resolve_reference(
                    &assignment.target.name,
                    self.current_scope(),
                    assignment.target.name_span.start.offset,
                )
                .map(|binding_id| {
                    self.bindings[binding_id.index()]
                        .attributes
                        .contains(BindingAttributes::ASSOC)
                })
                .unwrap_or(false)
        {
            attributes |= BindingAttributes::ARRAY | BindingAttributes::ASSOC;
        }

        let binding = self.add_binding(
            &assignment.target.name,
            kind,
            scope,
            assignment.target.name_span,
            binding_origin_for_assignment(assignment),
            attributes,
        );
        if let Some(hint) = indirect_target_hint(assignment, self.source) {
            self.indirect_target_hints.insert(binding, hint);
        }
    }

    fn visit_assignment_value_into(
        &mut self,
        assignment: &Assignment,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
            }
            AssignmentValue::Compound(array) => {
                self.visit_array_expr_into(array, WordVisitKind::Expansion, flow, nested_regions);
            }
        }
    }

    fn visit_words(
        &mut self,
        words: &[Word],
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_words_into(words, kind, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_words_into(
        &mut self,
        words: &[Word],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for word in words {
            self.visit_word_into(word, kind, flow, nested_regions);
        }
    }

    fn visit_array_expr_into(
        &mut self,
        array: &ArrayExpr,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for element in &array.elements {
            match element {
                ArrayElem::Sequential(word) => {
                    self.visit_word_into(word, kind, flow, nested_regions)
                }
                ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                    self.visit_var_ref_subscript_words(None, Some(key), kind, flow, nested_regions);
                    self.visit_word_into(value, kind, flow, nested_regions);
                }
            }
        }
    }

    fn visit_patterns(
        &mut self,
        patterns: &[Pattern],
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_patterns_into(patterns, kind, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_patterns_into(
        &mut self,
        patterns: &[Pattern],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for pattern in patterns {
            self.visit_pattern_into(pattern, kind, flow, nested_regions);
        }
    }

    fn visit_redirects(
        &mut self,
        redirects: &[shuck_ast::Redirect],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_redirects_into(redirects, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_redirects_into(
        &mut self,
        redirects: &[shuck_ast::Redirect],
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for redirect in redirects {
            match redirect.word_target() {
                Some(word) => {
                    self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions)
                }
                None => {
                    let heredoc = redirect.heredoc().expect("expected heredoc redirect");
                    if heredoc.delimiter.expands_body {
                        self.visit_heredoc_body_into(
                            &heredoc.body,
                            WordVisitKind::Expansion,
                            flow,
                            nested_regions,
                        );
                    }
                }
            }
        }
    }

    fn visit_word(
        &mut self,
        word: &Word,
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_word_into(word, kind, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_word_into(
        &mut self,
        word: &Word,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if word_is_semantically_inert(word) {
            return;
        }
        self.visit_word_part_nodes(&word.parts, kind, flow, nested_regions);
    }

    fn visit_heredoc_body_into(
        &mut self,
        body: &HeredocBody,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if !body.mode.expands() || heredoc_body_is_semantically_inert(body) {
            return;
        }
        self.visit_heredoc_body_part_nodes(&body.parts, kind, flow, nested_regions);
    }

    fn visit_pattern_into(
        &mut self,
        pattern: &Pattern,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if pattern_is_semantically_inert(pattern) {
            return;
        }
        self.visit_pattern_part_nodes(&pattern.parts, kind, flow, nested_regions);
    }

    fn visit_word_part_nodes(
        &mut self,
        parts: &[WordPartNode],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for part in parts {
            self.visit_word_part(&part.kind, part.span, kind, flow, nested_regions);
        }
    }

    fn visit_pattern_part_nodes(
        &mut self,
        parts: &[PatternPartNode],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for part in parts {
            self.visit_pattern_part(&part.kind, kind, flow, nested_regions);
        }
    }

    fn visit_heredoc_body_part_nodes(
        &mut self,
        parts: &[HeredocBodyPartNode],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for part in parts {
            self.visit_heredoc_body_part(&part.kind, part.span, kind, flow, nested_regions);
        }
    }

    fn visit_var_ref_reference(
        &mut self,
        reference: &VarRef,
        reference_kind: ReferenceKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
        span: Span,
    ) -> ReferenceId {
        let id = self.add_reference(&reference.name, reference_kind, span);
        self.visit_var_ref_subscript_words(
            Some(&reference.name),
            reference.subscript.as_ref(),
            if matches!(reference_kind, ReferenceKind::ConditionalOperand) {
                WordVisitKind::Conditional
            } else {
                WordVisitKind::Expansion
            },
            flow,
            nested_regions,
        );
        id
    }

    fn visit_var_ref_subscript_words(
        &mut self,
        owner_name: Option<&Name>,
        subscript: Option<&Subscript>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let Some(subscript) = subscript else {
            return;
        };
        if subscript.selector().is_some() {
            return;
        }
        let uses_associative_word_semantics = matches!(
            subscript.interpretation,
            shuck_ast::SubscriptInterpretation::Associative
        ) || owner_name.is_some_and(|name| {
            self.resolve_reference(name, self.current_scope(), subscript.span().start.offset)
                .map(|binding_id| {
                    self.bindings[binding_id.index()]
                        .attributes
                        .contains(BindingAttributes::ASSOC)
                })
                .unwrap_or(false)
        });
        if !uses_associative_word_semantics
            && let Some(expression) = subscript.arithmetic_ast.as_ref()
        {
            self.visit_optional_arithmetic_expr_into(Some(expression), flow, nested_regions);
            return;
        }

        self.visit_fragment_word(
            subscript.word_ast(),
            Some(subscript.syntax_source_text()),
            kind,
            flow,
            nested_regions,
        );
    }

    fn visit_word_part(
        &mut self,
        part: &WordPart,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            WordPart::ZshQualifiedGlob(glob) => {
                if zsh_qualified_glob_is_semantically_inert(glob) {
                    return;
                }
                for segment in &glob.segments {
                    if let ZshGlobSegment::Pattern(pattern) = segment {
                        self.visit_pattern_into(pattern, kind, flow, nested_regions);
                    }
                }
            }
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                if parts
                    .iter()
                    .all(|part| word_part_is_semantically_inert(&part.kind))
                {
                    return;
                }
                self.visit_word_part_nodes(parts, kind, flow, nested_regions);
            }
            WordPart::Variable(name) => {
                self.add_reference(
                    name,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::Expansion
                    },
                    span,
                );
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let mut commands = Vec::with_capacity(body.len());
                self.visit_stmt_seq_into(
                    body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                    &mut commands,
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
                nested_regions.push(IsolatedRegion {
                    scope,
                    commands: self.recorded_program.push_command_ids(commands),
                });
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_into(
                    expression_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            WordPart::Parameter(parameter) => {
                self.visit_parameter_expansion(
                    parameter,
                    kind,
                    flow,
                    nested_regions,
                    parameter.span,
                );
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let reference_id = self.visit_var_ref_reference(
                    reference,
                    if matches!(operator, ParameterOp::Error) {
                        ReferenceKind::RequiredRead
                    } else if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
                if parameter_operator_guards_unset_reference(operator) {
                    self.guarded_parameter_refs.insert(reference_id);
                }
                if matches!(operator, ParameterOp::AssignDefault) {
                    self.add_parameter_default_binding(reference);
                }
                self.visit_parameter_operator_operand(
                    operator,
                    operand.as_ref(),
                    operand_word_ast.as_ref(),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            WordPart::Length(reference) | WordPart::ArrayLength(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::Length
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::ArrayAccess(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ArrayAccess
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::ArrayIndices(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::IndirectExpansion
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::PrefixMatch { prefix, .. } => {
                self.add_reference(
                    prefix,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::IndirectExpansion
                    },
                    span,
                );
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let id = self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::IndirectExpansion
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.indirect_expansion_refs.insert(id);
                if let Some(operator) = operator {
                    self.visit_parameter_operator_operand(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_ref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
            }
            WordPart::Substring {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.visit_optional_arithmetic_expr_into(offset_ast.as_ref(), flow, nested_regions);
                self.visit_optional_arithmetic_expr_into(length_ast.as_ref(), flow, nested_regions);
            }
            WordPart::ArraySlice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.visit_optional_arithmetic_expr_into(offset_ast.as_ref(), flow, nested_regions);
                self.visit_optional_arithmetic_expr_into(length_ast.as_ref(), flow, nested_regions);
            }
            WordPart::Transformation { reference, .. } => {
                self.visit_var_ref_reference(
                    reference,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
        }
    }

    fn visit_heredoc_body_part(
        &mut self,
        part: &HeredocBodyPart,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            HeredocBodyPart::Literal(_) => {}
            HeredocBodyPart::Variable(name) => {
                self.add_reference(
                    name,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::Expansion
                    },
                    span,
                );
            }
            HeredocBodyPart::CommandSubstitution { body, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let mut commands = Vec::with_capacity(body.len());
                self.visit_stmt_seq_into(
                    body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                    &mut commands,
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
                nested_regions.push(IsolatedRegion {
                    scope,
                    commands: self.recorded_program.push_command_ids(commands),
                });
            }
            HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_into(
                    expression_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            HeredocBodyPart::Parameter(parameter) => {
                self.visit_parameter_expansion(parameter, kind, flow, nested_regions, span);
            }
        }
    }

    fn visit_parameter_expansion(
        &mut self,
        parameter: &ParameterExpansion,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
        span: Span,
    ) {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::ArrayAccess
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Length { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::Length
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Indices { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::IndirectExpansion
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Indirect {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    let id = self.visit_var_ref_reference(
                        reference,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::IndirectExpansion
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                    self.indirect_expansion_refs.insert(id);
                    if let Some(operator) = operator {
                        self.visit_parameter_operator_operand(
                            operator,
                            operand.as_ref(),
                            operand_word_ast.as_ref(),
                            kind,
                            flow,
                            nested_regions,
                        );
                    }
                }
                BourneParameterExpansion::PrefixMatch { prefix, .. } => {
                    self.add_reference(
                        prefix,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::IndirectExpansion
                        },
                        span,
                    );
                }
                BourneParameterExpansion::Slice {
                    reference,
                    offset_ast,
                    length_ast,
                    ..
                } => {
                    self.visit_var_ref_reference(
                        reference,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::ParameterExpansion
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                    self.visit_optional_arithmetic_expr_into(
                        offset_ast.as_ref(),
                        flow,
                        nested_regions,
                    );
                    self.visit_optional_arithmetic_expr_into(
                        length_ast.as_ref(),
                        flow,
                        nested_regions,
                    );
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    let reference_id = self.visit_var_ref_reference(
                        reference,
                        if matches!(operator, ParameterOp::Error) {
                            ReferenceKind::RequiredRead
                        } else if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::ParameterExpansion
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                    if parameter_operator_guards_unset_reference(operator) {
                        self.guarded_parameter_refs.insert(reference_id);
                    }
                    if matches!(operator, ParameterOp::AssignDefault) {
                        self.add_parameter_default_binding(reference);
                    }
                    self.visit_parameter_operator_operand(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_ref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
                BourneParameterExpansion::Transformation { reference, .. } => {
                    self.visit_var_ref_reference(
                        reference,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::ParameterExpansion
                        },
                        flow,
                        nested_regions,
                        span,
                    );
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        self.visit_var_ref_reference(
                            reference,
                            if matches!(kind, WordVisitKind::Conditional) {
                                ReferenceKind::ConditionalOperand
                            } else {
                                ReferenceKind::ParameterExpansion
                            },
                            flow,
                            nested_regions,
                            span,
                        );
                    }
                    ZshExpansionTarget::Word(word) => {
                        self.visit_word_into(word, kind, flow, nested_regions);
                    }
                    ZshExpansionTarget::Nested(parameter) => {
                        self.visit_parameter_expansion(parameter, kind, flow, nested_regions, span);
                    }
                    ZshExpansionTarget::Empty => {}
                }

                for modifier in &syntax.modifiers {
                    self.visit_fragment_word(
                        modifier.argument_word_ast(),
                        modifier.argument.as_ref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                }

                if let Some(operation) = &syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation { operand, .. }
                        | ZshExpansionOperation::TrimOperation { operand, .. } => self
                            .visit_fragment_word(
                                operation.operand_word_ast(),
                                Some(operand),
                                kind,
                                flow,
                                nested_regions,
                            ),
                        ZshExpansionOperation::Defaulting { operand, .. } => {
                            self.guarded_parameter_operand_depth += 1;
                            self.visit_fragment_word(
                                operation.operand_word_ast(),
                                Some(operand),
                                kind,
                                flow,
                                nested_regions,
                            );
                            self.guarded_parameter_operand_depth -= 1;
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            replacement,
                            ..
                        } => {
                            self.visit_fragment_word(
                                operation.pattern_word_ast(),
                                Some(pattern),
                                kind,
                                flow,
                                nested_regions,
                            );
                            self.visit_fragment_word(
                                operation.replacement_word_ast(),
                                replacement.as_ref(),
                                kind,
                                flow,
                                nested_regions,
                            );
                        }
                        ZshExpansionOperation::Slice { offset, length, .. } => {
                            self.visit_fragment_word(
                                operation.offset_word_ast(),
                                Some(offset),
                                kind,
                                flow,
                                nested_regions,
                            );
                            self.visit_fragment_word(
                                operation.length_word_ast(),
                                length.as_ref(),
                                kind,
                                flow,
                                nested_regions,
                            );
                        }
                        ZshExpansionOperation::Unknown { text, .. } => self.visit_fragment_word(
                            operation.operand_word_ast(),
                            Some(text),
                            kind,
                            flow,
                            nested_regions,
                        ),
                    }
                }
            }
        }
    }

    fn visit_fragment_word(
        &mut self,
        word: Option<&Word>,
        text: Option<&shuck_ast::SourceText>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let Some(word) = word else {
            debug_assert!(
                text.is_none(),
                "parser-backed fragment text should always carry a word AST"
            );
            return;
        };
        self.visit_word_into(word, kind, flow, nested_regions);
    }

    fn visit_parameter_operator_operand(
        &mut self,
        operator: &ParameterOp,
        operand: Option<&shuck_ast::SourceText>,
        operand_word_ast: Option<&Word>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                self.visit_pattern_into(pattern, kind, flow, nested_regions);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                ..
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                ..
            } => {
                self.visit_pattern_into(pattern, kind, flow, nested_regions);
                self.visit_fragment_word(
                    operator.replacement_word_ast(),
                    Some(replacement),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error => {
                self.guarded_parameter_operand_depth += 1;
                self.visit_fragment_word(operand_word_ast, operand, kind, flow, nested_regions);
                self.guarded_parameter_operand_depth -= 1;
            }
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    fn visit_pattern_part(
        &mut self,
        part: &PatternPart,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    self.visit_pattern_into(pattern, kind, flow, nested_regions);
                }
            }
            PatternPart::Word(word) => {
                self.visit_word_into(word, kind, flow, nested_regions);
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    fn visit_conditional_expr(
        &mut self,
        expression: &ConditionalExpr,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_conditional_expr_into(expression, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_conditional_expr_into(
        &mut self,
        expression: &ConditionalExpr,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.visit_conditional_expr_into(&expr.left, flow, nested_regions);
                self.visit_conditional_expr_into(&expr.right, flow, nested_regions);
            }
            ConditionalExpr::Unary(expr) => {
                self.visit_conditional_expr_into(&expr.expr, flow, nested_regions);
            }
            ConditionalExpr::Parenthesized(expr) => {
                self.visit_conditional_expr_into(&expr.expr, flow, nested_regions);
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.visit_word_into(word, WordVisitKind::Conditional, flow, nested_regions);
            }
            ConditionalExpr::Pattern(pattern) => {
                self.visit_pattern_into(pattern, WordVisitKind::Conditional, flow, nested_regions);
            }
            ConditionalExpr::VarRef(var_ref) => {
                self.visit_var_ref_reference(
                    var_ref,
                    ReferenceKind::ConditionalOperand,
                    flow,
                    nested_regions,
                    var_ref.name_span,
                );
            }
        }
    }

    fn visit_optional_arithmetic_expr(
        &mut self,
        expr: Option<&ArithmeticExprNode>,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_optional_arithmetic_expr_into(expr, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_optional_arithmetic_expr_into(
        &mut self,
        expr: Option<&ArithmeticExprNode>,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if let Some(expr) = expr {
            self.visit_arithmetic_expr_into(expr, flow, nested_regions);
        }
    }

    fn visit_arithmetic_expr_into(
        &mut self,
        expr: &ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExpr::Number(_) => {}
            ArithmeticExpr::Variable(name) => {
                self.add_reference(name, ReferenceKind::ArithmeticRead, expr.span);
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.add_reference(
                    name,
                    ReferenceKind::ArithmeticRead,
                    arithmetic_name_span(expr.span, name),
                );
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
            }
            ArithmeticExpr::ShellWord(word) => {
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
            }
            ArithmeticExpr::Parenthesized { expression } => {
                self.visit_arithmetic_expr_into(expression, flow, nested_regions);
            }
            ArithmeticExpr::Unary { op, expr: inner } => {
                if matches!(
                    op,
                    ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
                ) {
                    self.visit_arithmetic_update_into(inner, flow, nested_regions);
                } else {
                    self.visit_arithmetic_expr_into(inner, flow, nested_regions);
                }
            }
            ArithmeticExpr::Postfix { expr: inner, .. } => {
                self.visit_arithmetic_update_into(inner, flow, nested_regions);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                self.visit_arithmetic_expr_into(left, flow, nested_regions);
                self.visit_arithmetic_expr_into(right, flow, nested_regions);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_arithmetic_expr_into(condition, flow, nested_regions);
                self.visit_arithmetic_expr_into(then_expr, flow, nested_regions);
                self.visit_arithmetic_expr_into(else_expr, flow, nested_regions);
            }
            ArithmeticExpr::Assignment { target, op, value } => {
                self.visit_arithmetic_assignment_into(
                    target,
                    expr.span,
                    *op,
                    value,
                    flow,
                    nested_regions,
                );
            }
        }
    }

    fn visit_arithmetic_index_into(
        &mut self,
        owner_name: &Name,
        index: &ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if self
            .arithmetic_index_uses_associative_word_semantics(owner_name, index.span.start.offset)
        {
            self.visit_associative_arithmetic_key_into(index, flow, nested_regions);
            return;
        }

        self.visit_arithmetic_expr_into(index, flow, nested_regions);
    }

    fn arithmetic_index_uses_associative_word_semantics(
        &self,
        owner_name: &Name,
        offset: usize,
    ) -> bool {
        self.visible_binding_is_assoc(owner_name, offset)
    }

    fn visible_binding_is_assoc(&self, name: &Name, offset: usize) -> bool {
        self.resolve_reference(name, self.current_scope(), offset)
            .map(|binding_id| {
                self.bindings[binding_id.index()]
                    .attributes
                    .contains(BindingAttributes::ASSOC)
            })
            .unwrap_or(false)
    }

    fn arithmetic_binding_attributes(
        &self,
        target: &ArithmeticLvalue,
        target_offset: usize,
    ) -> BindingAttributes {
        let mut attributes = match target {
            ArithmeticLvalue::Variable(_) => BindingAttributes::empty(),
            ArithmeticLvalue::Indexed { .. } => BindingAttributes::ARRAY,
        };

        if let ArithmeticLvalue::Indexed { name, .. } = target
            && self.visible_binding_is_assoc(name, target_offset)
        {
            attributes |= BindingAttributes::ASSOC;
        }

        attributes
    }

    fn visit_associative_arithmetic_key_into(
        &mut self,
        expr: &ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => {
                self.visit_associative_arithmetic_key_into(index, flow, nested_regions);
            }
            ArithmeticExpr::ShellWord(word) => {
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
            }
            ArithmeticExpr::Parenthesized { expression } => {
                self.visit_associative_arithmetic_key_into(expression, flow, nested_regions);
            }
            ArithmeticExpr::Unary { expr: inner, .. } => {
                self.visit_associative_arithmetic_key_into(inner, flow, nested_regions);
            }
            ArithmeticExpr::Postfix { expr: inner, .. } => {
                self.visit_associative_arithmetic_key_into(inner, flow, nested_regions);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                self.visit_associative_arithmetic_key_into(left, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(right, flow, nested_regions);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_associative_arithmetic_key_into(condition, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(then_expr, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(else_expr, flow, nested_regions);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                self.visit_associative_arithmetic_lvalue_into(target, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(value, flow, nested_regions);
            }
        }
    }

    fn visit_associative_arithmetic_lvalue_into(
        &mut self,
        target: &ArithmeticLvalue,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => {
                self.visit_associative_arithmetic_key_into(index, flow, nested_regions);
            }
        }
    }

    fn visit_arithmetic_update_into(
        &mut self,
        expr: &ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExpr::Variable(name) => {
                self.add_reference(name, ReferenceKind::ArithmeticRead, expr.span);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    expr.span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: expr.span,
                    },
                    BindingAttributes::empty(),
                );
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
                let span = arithmetic_name_span(expr.span, name);
                self.add_reference(name, ReferenceKind::ArithmeticRead, span);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: span,
                    },
                    self.arithmetic_binding_attributes(
                        &ArithmeticLvalue::Indexed {
                            name: name.clone(),
                            index: index.clone(),
                        },
                        span.start.offset,
                    ),
                );
            }
            _ => {}
        }
    }

    fn visit_arithmetic_assignment_into(
        &mut self,
        target: &ArithmeticLvalue,
        target_span: Span,
        op: ArithmeticAssignOp,
        value: &ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        self.visit_arithmetic_lvalue_indices_into(target, flow, nested_regions);
        let attributes = self.arithmetic_binding_attributes(target, target_span.start.offset);
        let name = match target {
            ArithmeticLvalue::Variable(name) | ArithmeticLvalue::Indexed { name, .. } => name,
        };
        let name_span = arithmetic_name_span(target_span, name);
        if !matches!(op, ArithmeticAssignOp::Assign) {
            self.add_reference(name, ReferenceKind::ArithmeticRead, name_span);
        }
        self.visit_arithmetic_expr_into(value, flow, nested_regions);
        self.add_binding(
            name,
            BindingKind::ArithmeticAssignment,
            self.current_scope(),
            name_span,
            BindingOrigin::ArithmeticAssignment {
                definition_span: name_span,
            },
            attributes,
        );
    }

    fn visit_arithmetic_lvalue_indices_into(
        &mut self,
        target: &ArithmeticLvalue,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { name, index } => {
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
            }
        }
    }

    fn classify_special_simple_command(
        &mut self,
        name: &Name,
        command: &shuck_ast::SimpleCommand,
        _flow: FlowState,
    ) {
        match name.as_str() {
            "read" => {
                for (argument, span) in iter_read_targets(&command.args, self.source) {
                    self.add_binding(
                        &argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Read,
                        },
                        BindingAttributes::empty(),
                    );
                }
                for implicit_read in
                    self.runtime
                        .implicit_reads_for_simple_command(name, &command.args, self.source)
                {
                    let implicit_name = Name::from(*implicit_read);
                    self.add_reference_if_bound(
                        &implicit_name,
                        ReferenceKind::ImplicitRead,
                        command.span,
                    );
                }
            }
            "mapfile" | "readarray" => {
                if let Some((argument, span)) = explicit_mapfile_target(&command.args, self.source)
                {
                    self.add_binding(
                        &argument,
                        BindingKind::MapfileTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Mapfile,
                        },
                        BindingAttributes::empty(),
                    );
                }
            }
            "printf" => {
                if let Some((argument, span)) = printf_v_target(&command.args, self.source) {
                    self.add_binding(
                        &argument,
                        BindingKind::PrintfTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Printf,
                        },
                        BindingAttributes::empty(),
                    );
                }
            }
            "getopts" => {
                if let Some((argument, span)) = getopts_target(&command.args, self.source) {
                    self.add_binding(
                        &argument,
                        BindingKind::GetoptsTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Getopts,
                        },
                        BindingAttributes::empty(),
                    );
                }
            }
            "source" | "." => {
                if let Some(argument) = command.args.first() {
                    let source_span = self.command_stack.last().copied().unwrap_or(command.span);
                    let kind = self.classify_source_ref(command.span.line(), argument);
                    self.source_refs.push(SourceRef {
                        diagnostic_class: classify_source_ref_diagnostic_class(
                            argument,
                            self.source,
                            &kind,
                        ),
                        kind,
                        span: source_span,
                        path_span: argument.span,
                        resolution: SourceRefResolution::Unchecked,
                        explicitly_provided: false,
                    });
                }
            }
            _ => {}
        }
    }

    fn classify_source_ref(&self, line: usize, word: &Word) -> SourceRefKind {
        if let Some(directive) = self.source_directive_for_line(line) {
            return directive;
        }

        if let Some(text) = static_word_text(word, self.source) {
            return SourceRefKind::Literal(text);
        }

        classify_dynamic_source_word(word, self.source)
    }

    fn source_directive_for_line(&self, line: usize) -> Option<SourceRefKind> {
        if let Some(directive) = self.source_directives.get(&line) {
            return Some(directive.kind.clone());
        }

        let previous = line.checked_sub(1)?;
        self.source_directives
            .get(&previous)
            .and_then(|directive| directive.own_line.then_some(directive.kind.clone()))
    }

    fn declaration_scope_and_attributes(
        &self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
    ) -> (ScopeId, BindingAttributes) {
        let mut attributes = BindingAttributes::empty();
        if matches!(builtin, DeclarationBuiltin::Export) || flags.contains(&'x') {
            attributes |= BindingAttributes::EXPORTED;
        }
        if matches!(builtin, DeclarationBuiltin::Readonly) || flags.contains(&'r') {
            attributes |= BindingAttributes::READONLY;
        }
        if flags.contains(&'i') {
            attributes |= BindingAttributes::INTEGER;
        }
        if flags.contains(&'a') {
            attributes |= BindingAttributes::ARRAY;
        }
        if flags.contains(&'A') {
            attributes |= BindingAttributes::ASSOC;
        }
        if flags.contains(&'n') {
            attributes |= BindingAttributes::NAMEREF;
        }
        if flags.contains(&'l') {
            attributes |= BindingAttributes::LOWERCASE;
        }
        if flags.contains(&'u') {
            attributes |= BindingAttributes::UPPERCASE;
        }

        let global_like = matches!(
            builtin,
            DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset
        ) && global_flag_enabled;
        let local_like = matches!(builtin, DeclarationBuiltin::Local)
            || (matches!(
                builtin,
                DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset
            ) && self.nearest_function_scope().is_some()
                && !global_flag_enabled);

        if local_like {
            attributes |= BindingAttributes::LOCAL;
        }

        (
            if local_like {
                self.nearest_function_scope()
                    .unwrap_or_else(|| self.current_scope())
            } else if global_like {
                self.nearest_execution_scope()
            } else {
                self.current_scope()
            },
            attributes,
        )
    }

    fn visit_name_only_declaration_operand(
        &mut self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
        name: &Name,
        span: Span,
    ) {
        let (scope, attributes) =
            self.declaration_scope_and_attributes(builtin, flags, global_flag_enabled);
        let local_like = attributes.contains(BindingAttributes::LOCAL);
        let existing = self.resolve_reference(name, scope, span.start.offset);

        if let Some(existing) = existing {
            let existing_scope = self.bindings[existing.index()].scope;
            if !local_like || existing_scope == scope {
                self.add_reference(name, ReferenceKind::DeclarationName, span);
                self.bindings[existing.index()].attributes |= attributes;
                return;
            }
        }

        let kind = if attributes.contains(BindingAttributes::NAMEREF) {
            BindingKind::Nameref
        } else {
            BindingKind::Declaration(builtin)
        };
        let origin = if matches!(kind, BindingKind::Nameref) {
            BindingOrigin::Nameref {
                definition_span: span,
            }
        } else {
            BindingOrigin::Declaration {
                definition_span: span,
            }
        };
        self.add_binding(name, kind, scope, span, origin, attributes);
    }

    fn add_binding(
        &mut self,
        name: &Name,
        kind: BindingKind,
        scope: ScopeId,
        span: Span,
        origin: BindingOrigin,
        attributes: BindingAttributes,
    ) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            id,
            name: name.clone(),
            kind,
            origin,
            scope,
            span,
            references: Vec::new(),
            attributes,
        });
        self.binding_index.entry(name.clone()).or_default().push(id);
        match self.scopes[scope.index()].bindings.get_mut(name.as_str()) {
            Some(v) => v.push(id),
            None => {
                self.scopes[scope.index()]
                    .bindings
                    .insert(name.clone(), vec![id]);
            }
        }
        if matches!(kind, BindingKind::FunctionDefinition) {
            self.functions.entry(name.clone()).or_default().push(id);
        }
        if let Some(command) = self.command_stack.last().copied() {
            self.command_bindings
                .entry(SpanKey::new(command))
                .or_default()
                .push(id);
        }

        let binding = &self.bindings[id.index()];
        self.observer.record_binding(binding);
        id
    }

    fn add_reference(&mut self, name: &Name, kind: ReferenceKind, span: Span) -> ReferenceId {
        let span = self.normalize_reference_span(span);
        let id = ReferenceId(self.references.len() as u32);
        let scope = self.current_scope();
        let resolved = self.resolve_reference(name, scope, span.start.offset);
        let predefined_runtime = resolved.is_none() && self.runtime.is_preinitialized(name);

        self.references.push(Reference {
            id,
            name: name.clone(),
            kind,
            scope,
            span,
        });
        self.reference_index
            .entry(name.clone())
            .or_default()
            .push(id);
        if self.guarded_parameter_operand_depth > 0 {
            self.guarded_parameter_refs.insert(id);
        }
        if let Some(command) = self.command_stack.last().copied() {
            self.command_references
                .entry(SpanKey::new(command))
                .or_default()
                .push(id);
        }

        if let Some(binding) = resolved {
            self.resolved.insert(id, binding);
            self.bindings[binding.index()].references.push(id);
        } else if predefined_runtime {
            self.predefined_runtime_refs.insert(id);
        } else {
            self.unresolved.push(id);
        }

        let reference = &self.references[id.index()];
        let resolved_binding = resolved.map(|binding| &self.bindings[binding.index()]);
        self.observer.record_reference(reference, resolved_binding);
        id
    }

    fn normalize_reference_span(&self, span: Span) -> Span {
        if span.end.offset >= self.source.len() {
            return span;
        }

        let syntax = span.slice(self.source);
        let Some(start_rel) = syntax.find("${") else {
            return span;
        };
        if self.source.as_bytes().get(span.end.offset) != Some(&b'}') {
            return span;
        }

        let start = span.start.advanced_by(&syntax[..start_rel]);
        let end = span.end.advanced_by("}");
        if start.offset < end.offset {
            Span::from_positions(start, end)
        } else {
            span
        }
    }

    fn add_parameter_default_binding(&mut self, reference: &VarRef) {
        self.add_binding(
            &reference.name,
            BindingKind::ParameterDefaultAssignment,
            self.current_scope(),
            reference.span,
            BindingOrigin::ParameterDefaultAssignment {
                definition_span: reference.span,
            },
            BindingAttributes::empty(),
        );
    }

    fn add_reference_if_bound(&mut self, name: &Name, kind: ReferenceKind, span: Span) {
        if self
            .resolve_reference(name, self.current_scope(), span.start.offset)
            .is_some()
        {
            self.add_reference(name, kind, span);
        }
    }

    fn resolve_reference(&self, name: &Name, scope: ScopeId, offset: usize) -> Option<BindingId> {
        for scope in ancestor_scopes(&self.scopes, scope) {
            let Some(bindings) = self.scopes[scope.index()].bindings.get(name) else {
                continue;
            };

            if self.completed_scopes.contains(&scope) {
                if let Some(binding) = bindings.last().copied() {
                    return Some(binding);
                }
            } else {
                for binding in bindings.iter().rev().copied() {
                    if self.bindings[binding.index()].span.start.offset <= offset {
                        return Some(binding);
                    }
                }
            }
        }
        None
    }

    fn build_call_graph(&self) -> CallGraph {
        let mut reachable = FxHashSet::default();
        let mut worklist = self
            .call_sites
            .values()
            .flat_map(|sites| sites.iter())
            .filter(|site| !is_in_function_scope(&self.scopes, site.scope))
            .map(|site| site.callee.clone())
            .collect::<Vec<_>>();

        while let Some(name) = worklist.pop() {
            if reachable.contains(name.as_str()) {
                continue;
            }
            for sites in self.call_sites.values() {
                for site in sites {
                    if is_in_named_function_scope(&self.scopes, site.scope, &name) {
                        worklist.push(site.callee.clone());
                    }
                }
            }
            reachable.insert(name);
        }

        let uncalled = self
            .functions
            .iter()
            .filter(|(name, _)| !reachable.contains(*name))
            .flat_map(|(_, bindings)| bindings.iter().copied())
            .collect();

        let overwritten = self
            .functions
            .iter()
            .flat_map(|(name, bindings)| {
                bindings.windows(2).map(move |pair| OverwrittenFunction {
                    name: name.clone(),
                    first: pair[0],
                    second: pair[1],
                    first_called: self
                        .call_sites
                        .get(name)
                        .into_iter()
                        .flat_map(|sites| sites.iter())
                        .any(|site| {
                            let first = self.bindings[pair[0].index()].span.start.offset;
                            let second = self.bindings[pair[1].index()].span.start.offset;
                            site.span.start.offset > first && site.span.start.offset < second
                        }),
                })
            })
            .collect();

        CallGraph {
            reachable,
            uncalled,
            overwritten,
        }
    }

    fn compute_heuristic_unused_assignments(&self) -> Vec<BindingId> {
        self.bindings
            .iter()
            .filter(|binding| {
                !matches!(
                    binding.kind,
                    BindingKind::FunctionDefinition | BindingKind::Imported
                ) && binding.references.is_empty()
            })
            .map(|binding| binding.id)
            .collect()
    }

    fn push_scope(&mut self, kind: ScopeKind, parent: ScopeId, span: Span) -> ScopeId {
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

    fn pop_scope(&mut self, expected: ScopeId) {
        let popped = self.scope_stack.pop();
        debug_assert_eq!(popped, Some(expected));
    }

    fn mark_scope_completed(&mut self, scope: ScopeId) {
        self.completed_scopes.insert(scope);
    }

    fn drain_deferred_functions(&mut self) {
        while !self.deferred_functions.is_empty() {
            let deferred_functions = std::mem::take(&mut self.deferred_functions);
            for deferred in deferred_functions {
                self.rebuild_scope_stack(deferred.scope);
                // SAFETY: deferred function pointers always refer to nodes inside the borrowed AST
                // passed into `build`, and we only dereference them while that AST is still alive.
                let function = unsafe { &*deferred.function };
                let commands = self.visit_function_like_body(&function.body, deferred.flow);
                self.recorded_program
                    .set_function_body(deferred.scope, commands);
                self.mark_scope_completed(deferred.scope);
            }
        }
        self.rebuild_scope_stack(ScopeId(0));
        self.command_stack.clear();
    }

    fn visit_function_like_body(&mut self, body: &Stmt, flow: FlowState) -> RecordedCommandRange {
        let flow = FlowState {
            in_function: true,
            ..flow
        };

        match &body.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                self.visit_stmt_seq(commands, flow)
            }
            _ => {
                let command = self.visit_stmt(body, flow);
                self.recorded_program.push_command_ids(vec![command])
            }
        }
    }

    fn rebuild_scope_stack(&mut self, scope: ScopeId) {
        self.scope_stack = ancestor_scopes(&self.scopes, scope).collect::<Vec<_>>();
        self.scope_stack.reverse();
    }

    fn flatten_recorded_regions(&self, recorded: RecordedCommandId) -> Vec<IsolatedRegion> {
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

    fn current_scope(&self) -> ScopeId {
        *self.scope_stack.last().unwrap_or(&ScopeId(0))
    }

    fn nearest_function_scope(&self) -> Option<ScopeId> {
        self.scope_stack
            .iter()
            .rev()
            .copied()
            .find(|scope| matches!(self.scopes[scope.index()].kind, ScopeKind::Function(_)))
    }

    fn nearest_execution_scope(&self) -> ScopeId {
        self.scope_stack
            .iter()
            .rev()
            .copied()
            .find(|scope| !matches!(self.scopes[scope.index()].kind, ScopeKind::Function(_)))
            .unwrap_or(ScopeId(0))
    }
}

fn parameter_operator_guards_unset_reference(operator: &ParameterOp) -> bool {
    matches!(
        operator,
        ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
    )
}

fn declaration_builtin(name: &Name) -> DeclarationBuiltin {
    match name.as_str() {
        "declare" => DeclarationBuiltin::Declare,
        "local" => DeclarationBuiltin::Local,
        "export" => DeclarationBuiltin::Export,
        "readonly" => DeclarationBuiltin::Readonly,
        "typeset" => DeclarationBuiltin::Typeset,
        _ => DeclarationBuiltin::Declare,
    }
}

fn declaration_flags(operands: &[DeclOperand], source: &str) -> FxHashSet<char> {
    let mut flags = FxHashSet::default();
    for operand in operands {
        if let DeclOperand::Flag(word) = operand
            && let Some(text) = static_word_text(word, source)
        {
            for flag in text.chars().skip(1) {
                flags.insert(flag);
            }
        }
    }
    flags
}

fn declaration_flag_is_enabled(
    operands: &[DeclOperand],
    source: &str,
    target: char,
) -> Option<bool> {
    let mut enabled = None;
    for operand in operands {
        if let DeclOperand::Flag(word) = operand
            && let Some(text) = static_word_text(word, source)
        {
            let mut chars = text.chars();
            let Some(polarity) = chars.next() else {
                continue;
            };
            let enabled_for_operand = match polarity {
                '-' => true,
                '+' => false,
                _ => continue,
            };
            for flag in chars {
                if flag == target {
                    enabled = Some(enabled_for_operand);
                }
            }
        }
    }
    enabled
}

fn declaration_operands(operands: &[DeclOperand], source: &str) -> Vec<DeclarationOperand> {
    operands
        .iter()
        .map(|operand| match operand {
            DeclOperand::Flag(word) => {
                let text = static_word_text(word, source).unwrap_or_default();
                let flag = text.chars().nth(1).unwrap_or('-');
                DeclarationOperand::Flag {
                    flag,
                    span: word.span,
                }
            }
            DeclOperand::Name(name) => DeclarationOperand::Name {
                name: name.name.clone(),
                span: name.span,
            },
            DeclOperand::Assignment(assignment) => DeclarationOperand::Assignment {
                name: assignment.target.name.clone(),
                name_span: assignment.target.name_span,
                value_span: assignment_value_span(assignment),
                append: assignment.append,
            },
            DeclOperand::Dynamic(word) => DeclarationOperand::DynamicWord { span: word.span },
        })
        .collect()
}

fn binding_attributes_for_var_ref(reference: &VarRef) -> BindingAttributes {
    match reference
        .subscript
        .as_ref()
        .map(|subscript| subscript.interpretation)
    {
        Some(shuck_ast::SubscriptInterpretation::Associative) => {
            BindingAttributes::ARRAY | BindingAttributes::ASSOC
        }
        Some(_) => BindingAttributes::ARRAY,
        None => BindingAttributes::empty(),
    }
}

fn binding_attributes_for_array_expr(array: &ArrayExpr) -> BindingAttributes {
    match array.kind {
        ArrayKind::Associative => BindingAttributes::ARRAY | BindingAttributes::ASSOC,
        ArrayKind::Indexed | ArrayKind::Contextual => BindingAttributes::ARRAY,
    }
}

fn assignment_binding_attributes(assignment: &Assignment) -> BindingAttributes {
    let mut attributes = binding_attributes_for_var_ref(&assignment.target);
    if let AssignmentValue::Compound(array) = &assignment.value {
        attributes |= binding_attributes_for_array_expr(array);
    }
    attributes
}

fn assignment_value_span(assignment: &Assignment) -> Span {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word.span,
        AssignmentValue::Compound(array) => array.span,
    }
}

fn assignment_has_empty_initializer(assignment: &Assignment, source: &str) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => static_word_text(word, source).as_deref() == Some(""),
        AssignmentValue::Compound(array) => array.elements.is_empty(),
    }
}

fn indirect_target_hint(assignment: &Assignment, source: &str) -> Option<IndirectTargetHint> {
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    indirect_target_hint_from_word(word, source)
}

fn indirect_target_hint_from_word(word: &Word, source: &str) -> Option<IndirectTargetHint> {
    if let Some(text) = static_word_text(word, source) {
        let (name, array_like) = parse_indirect_target_name(&text)?;
        return Some(IndirectTargetHint::Exact {
            name: Name::from(name),
            array_like,
        });
    }

    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut saw_variable = false;
    if !collect_indirect_pattern_parts(
        &word.parts,
        source,
        &mut prefix,
        &mut suffix,
        &mut saw_variable,
    ) {
        return None;
    }

    if !saw_variable {
        return None;
    }

    let (suffix, array_like) = strip_array_like_suffix(suffix.as_str());
    if (!prefix.is_empty() && !is_name_fragment(&prefix)) || !is_name_fragment(suffix) {
        return None;
    }
    if prefix.is_empty() && suffix.is_empty() {
        return None;
    }

    Some(IndirectTargetHint::Pattern {
        prefix,
        suffix: suffix.to_string(),
        array_like,
    })
}

fn collect_indirect_pattern_parts(
    parts: &[WordPartNode],
    source: &str,
    prefix: &mut String,
    suffix: &mut String,
    saw_variable: &mut bool,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                if *saw_variable {
                    suffix.push_str(text.as_str(source, part.span));
                } else {
                    prefix.push_str(text.as_str(source, part.span));
                }
            }
            WordPart::SingleQuoted { value, .. } => {
                if *saw_variable {
                    suffix.push_str(value.slice(source));
                } else {
                    prefix.push_str(value.slice(source));
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_indirect_pattern_parts(parts, source, prefix, suffix, saw_variable) {
                    return false;
                }
            }
            WordPart::Variable(_) if !*saw_variable => *saw_variable = true,
            WordPart::Parameter(parameter)
                if !*saw_variable && parameter_is_indirect_pattern_variable(parameter) =>
            {
                *saw_variable = true;
            }
            _ => return false,
        }
    }

    true
}

fn parameter_is_indirect_pattern_variable(parameter: &ParameterExpansion) -> bool {
    matches!(
        &parameter.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.subscript.is_none()
    )
}

fn parse_indirect_target_name(text: &str) -> Option<(&str, bool)> {
    let (name, array_like) = strip_array_like_suffix(text);
    is_name(name).then_some((name, array_like))
}

fn strip_array_like_suffix(text: &str) -> (&str, bool) {
    if let Some(base) = text.strip_suffix("[@]") {
        return (base, true);
    }
    if let Some(base) = text.strip_suffix("[*]") {
        return (base, true);
    }
    (text, false)
}

fn is_name_fragment(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn iter_read_targets<'a>(
    args: &'a [Word],
    source: &'a str,
) -> impl Iterator<Item = (Name, Span)> + 'a {
    args.iter()
        .filter_map(move |word| named_target_word(word, source))
        .filter(|(name, _)| !name.as_str().starts_with('-'))
}

fn explicit_mapfile_target(args: &[Word], source: &str) -> Option<(Name, Span)> {
    args.iter().find_map(|word| {
        let target = named_target_word(word, source)?;
        (!target.0.as_str().starts_with('-')).then_some(target)
    })
}

fn printf_v_target(args: &[Word], source: &str) -> Option<(Name, Span)> {
    args.windows(2).find_map(|window| {
        (static_word_text(&window[0], source).as_deref() == Some("-v"))
            .then_some(&window[1])
            .and_then(|word| named_target_word(word, source))
    })
}

fn getopts_target(args: &[Word], source: &str) -> Option<(Name, Span)> {
    args.get(1).and_then(|word| named_target_word(word, source))
}

fn simple_command_has_name(command: &shuck_ast::SimpleCommand, source: &str) -> bool {
    !matches!(static_word_text(&command.name, source).as_deref(), Some(""))
}

fn named_target_word(word: &Word, source: &str) -> Option<(Name, Span)> {
    let text = static_word_text(word, source)?;
    is_name(&text).then_some((Name::from(text), word.span))
}

fn static_command_name_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    collect_static_command_name_parts(
        &word.parts,
        source,
        StaticCommandNameContext::Unquoted,
        &mut result,
    )
    .then_some(result)
}

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    collect_static_word_text(&word.parts, source, &mut result).then_some(result)
}

#[derive(Clone, Copy)]
enum StaticCommandNameContext {
    Unquoted,
    DoubleQuoted,
}

fn collect_static_command_name_parts(
    parts: &[WordPartNode],
    source: &str,
    context: StaticCommandNameContext,
    out: &mut String,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                decode_static_command_literal(text.as_str(source, part.span), context, out);
            }
            WordPart::SingleQuoted { value, .. } => out.push_str(value.slice(source)),
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_static_command_name_parts(
                    parts,
                    source,
                    StaticCommandNameContext::DoubleQuoted,
                    out,
                ) {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

fn decode_static_command_literal(text: &str, context: StaticCommandNameContext, out: &mut String) {
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };

        match context {
            StaticCommandNameContext::Unquoted => {
                if next != '\n' {
                    out.push(next);
                }
            }
            StaticCommandNameContext::DoubleQuoted => match next {
                '$' | '`' | '"' | '\\' => out.push(next),
                '\n' => {}
                _ => {
                    out.push('\\');
                    out.push(next);
                }
            },
        }
    }
}

fn recorded_command_info(
    command: &Command,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> RecordedCommandInfo {
    match command {
        Command::Simple(command) => {
            recorded_simple_command_info(command, source, bash_runtime_vars_enabled)
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => RecordedCommandInfo::default(),
    }
}

fn recorded_simple_command_info(
    command: &shuck_ast::SimpleCommand,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> RecordedCommandInfo {
    let words = std::iter::once(&command.name)
        .chain(command.args.iter())
        .collect::<Vec<_>>();
    let mut static_callee = static_command_name_text(&command.name, source);
    let static_args = command
        .args
        .iter()
        .map(|word| static_word_text(word, source))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let source_path_template = static_callee
        .as_deref()
        .filter(|name| matches!(*name, "source" | "."))
        .and_then(|_| command.args.first())
        .and_then(|word| source_path_template(word, source, bash_runtime_vars_enabled));

    if static_callee.as_deref() == Some("noglob") {
        static_callee = words
            .get(1)
            .and_then(|word| static_command_name_text(word, source));
    }

    let mut info = RecordedCommandInfo {
        static_callee,
        static_args,
        source_path_template,
        zsh_effects: Vec::new(),
    };
    let Some((effect_callee, effect_index)) = normalize_recorded_zsh_effect_command(&words, source)
    else {
        return info;
    };
    let args = words.get(effect_index + 1..).unwrap_or(&[]);

    match effect_callee.as_str() {
        "emulate" => info.zsh_effects = parse_emulate_effects(args, source),
        "setopt" => {
            info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                updates: parse_setopt_updates(args, source, true),
            }];
        }
        "unsetopt" => {
            info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                updates: parse_setopt_updates(args, source, false),
            }];
        }
        "set" => {
            let updates = parse_set_builtin_option_updates(args, source);
            if !updates.is_empty() {
                info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions { updates }];
            }
        }
        _ => {}
    }

    info.zsh_effects.retain(|effect| match effect {
        RecordedZshCommandEffect::Emulate { .. } => true,
        RecordedZshCommandEffect::SetOptions { updates } => !updates.is_empty(),
    });
    info
}

fn normalize_recorded_zsh_effect_command(words: &[&Word], source: &str) -> Option<(String, usize)> {
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        let text = static_word_text(word, source)?;
        if is_recorded_assignment_word(&text) {
            index += 1;
            continue;
        }

        match text.as_str() {
            "noglob" => {
                index += 1;
                continue;
            }
            "command" => {
                index = skip_recorded_command_wrapper_options(words, source, index + 1)?;
                continue;
            }
            "builtin" => {
                index = skip_recorded_wrapper_options(words, source, index + 1);
                continue;
            }
            "exec" => {
                index = skip_recorded_exec_wrapper_options(words, source, index + 1);
                continue;
            }
            _ => return Some((text, index)),
        }
    }

    None
}

fn skip_recorded_command_wrapper_options(
    words: &[&Word],
    source: &str,
    mut index: usize,
) -> Option<usize> {
    while let Some(word) = words.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        if text.starts_with('-') && text != "-" {
            if text
                .strip_prefix('-')
                .is_some_and(|flags| flags.chars().any(|flag| matches!(flag, 'v' | 'V')))
            {
                return None;
            }
            index += 1;
            continue;
        }
        break;
    }
    Some(index)
}

fn skip_recorded_wrapper_options(words: &[&Word], source: &str, mut index: usize) -> usize {
    while let Some(word) = words.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        if text.starts_with('-') && text != "-" {
            index += 1;
            continue;
        }
        break;
    }
    index
}

fn skip_recorded_exec_wrapper_options(words: &[&Word], source: &str, mut index: usize) -> usize {
    while let Some(word) = words.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        if text == "-a" {
            index = (index + 2).min(words.len());
            continue;
        }
        if text.starts_with('-') && text != "-" {
            index += 1;
            continue;
        }
        break;
    }
    index
}

fn is_recorded_assignment_word(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn parse_emulate_effects(args: &[&Word], source: &str) -> Vec<RecordedZshCommandEffect> {
    let mut local = false;
    let mut mode = None;
    let mut updates = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            index += 1;
            continue;
        };

        match text.as_str() {
            "--" => {
                break;
            }
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if let Some(option) = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                    && let Some(update) = parse_recorded_zsh_option_update(&option, enable)
                {
                    updates.push(update);
                }
                index += 2;
                continue;
            }
            _ => {}
        }

        if text.starts_with("-o") || text.starts_with("+o") {
            let enable = text.starts_with('-');
            if let Some(update) = parse_recorded_zsh_option_update(&text[2..], enable) {
                updates.push(update);
            }
            index += 1;
            continue;
        }

        if let Some(flags) = text.strip_prefix('-') {
            for flag in flags.chars() {
                match flag {
                    'L' => local = true,
                    'R' => {}
                    _ => {}
                }
            }
            index += 1;
            continue;
        }

        if mode.is_none() {
            mode = match text.to_ascii_lowercase().as_str() {
                "zsh" => Some(ZshEmulationMode::Zsh),
                "sh" => Some(ZshEmulationMode::Sh),
                "ksh" => Some(ZshEmulationMode::Ksh),
                "csh" => Some(ZshEmulationMode::Csh),
                _ => None,
            };
        }
        index += 1;
    }

    let mut effects = Vec::new();
    if let Some(mode) = mode {
        effects.push(RecordedZshCommandEffect::Emulate { mode, local });
    }
    if !updates.is_empty() {
        effects.push(RecordedZshCommandEffect::SetOptions { updates });
    }
    effects
}

fn parse_setopt_updates(
    args: &[&Word],
    source: &str,
    enable: bool,
) -> Vec<RecordedZshOptionUpdate> {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .filter(|text| text != "--")
        .filter_map(|text| parse_recorded_zsh_option_update(&text, enable))
        .collect()
}

fn parse_set_builtin_option_updates(args: &[&Word], source: &str) -> Vec<RecordedZshOptionUpdate> {
    let mut updates = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            index += 1;
            continue;
        };

        match text.as_str() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if let Some(name) = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                    && let Some(update) = parse_recorded_zsh_option_update(&name, enable)
                {
                    updates.push(update);
                }
                index += 2;
            }
            _ if text.starts_with("-o") || text.starts_with("+o") => {
                let enable = text.starts_with('-');
                if let Some(update) = parse_recorded_zsh_option_update(&text[2..], enable) {
                    updates.push(update);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    updates
}

fn parse_recorded_zsh_option_update(name: &str, enable: bool) -> Option<RecordedZshOptionUpdate> {
    let (normalized, inverted) = normalize_recorded_zsh_option_name(name)?;
    let enable = if inverted { !enable } else { enable };

    if normalized == "localoptions" {
        return Some(RecordedZshOptionUpdate::LocalOptions { enable });
    }

    Some(RecordedZshOptionUpdate::Named {
        name: normalized.into_boxed_str(),
        enable,
    })
}

fn normalize_recorded_zsh_option_name(name: &str) -> Option<(String, bool)> {
    let mut normalized = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '_' | '-') {
            continue;
        }
        normalized.push(ch.to_ascii_lowercase());
    }

    if normalized.is_empty() {
        return None;
    }

    if let Some(stripped) = normalized.strip_prefix("no")
        && !stripped.is_empty()
    {
        return Some((stripped.to_string(), true));
    }

    Some((normalized, false))
}

fn collect_static_word_text(parts: &[WordPartNode], source: &str, out: &mut String) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => out.push_str(text.as_str(source, part.span)),
            WordPart::SingleQuoted { value, .. } => out.push_str(value.slice(source)),
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_static_word_text(parts, source, out) {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

fn classify_dynamic_source_word(word: &Word, source: &str) -> SourceRefKind {
    let mut variable = None;
    let mut tail = String::new();

    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => tail.push_str(text.as_str(source, span)),
            WordPart::Variable(name) if variable.is_none() && tail.is_empty() => {
                variable = Some(name.clone());
            }
            _ => return SourceRefKind::Dynamic,
        }
    }

    if let Some(variable) = variable {
        return SourceRefKind::SingleVariableStaticTail { variable, tail };
    }

    SourceRefKind::Dynamic
}

fn classify_source_ref_diagnostic_class(
    word: &Word,
    source: &str,
    kind: &SourceRefKind,
) -> SourceRefDiagnosticClass {
    match kind {
        SourceRefKind::Literal(path)
            if literal_uses_current_user_home_tilde(word, source, path) =>
        {
            SourceRefDiagnosticClass::DynamicPath
        }
        SourceRefKind::Dynamic if dynamic_root_with_slash_tail(word, source) => {
            SourceRefDiagnosticClass::UntrackedFile
        }
        _ => default_diagnostic_class(kind),
    }
}

fn literal_uses_current_user_home_tilde(word: &Word, source: &str, path: &str) -> bool {
    if !path.starts_with("~/") {
        return false;
    }

    let Some((first, tail)) = word.parts.split_first() else {
        return false;
    };

    match &first.kind {
        WordPart::Literal(_) => {
            let text = first.span.slice(source);
            text.starts_with("~/")
                || (text == "~"
                    && static_parts_text(tail, source).is_some_and(|tail| tail.starts_with('/')))
        }
        _ => false,
    }
}

fn dynamic_root_with_slash_tail(word: &Word, source: &str) -> bool {
    let Some((root, tail)) = word.parts.split_first() else {
        return false;
    };

    match &root.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            let Some((inner_root, inner_tail)) = parts.split_first() else {
                return false;
            };

            root_word_part_is_dynamic_root(&inner_root.kind)
                && static_tail_text_starts_with_slash(inner_tail, tail, source)
        }
        _ => {
            root_word_part_is_dynamic_root(&root.kind)
                && static_tail_text_starts_with_slash(tail, &[], source)
        }
    }
}

fn root_word_part_is_dynamic_root(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(_) | WordPart::ArrayAccess(_) => true,
        WordPart::Parameter(parameter) => matches!(
            parameter.syntax,
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { .. })
        ),
        _ => false,
    }
}

fn static_parts_text(parts: &[WordPartNode], source: &str) -> Option<String> {
    let mut result = String::new();
    collect_static_word_text(parts, source, &mut result).then_some(result)
}

fn static_tail_text_starts_with_slash(
    parts: &[WordPartNode],
    trailing: &[WordPartNode],
    source: &str,
) -> bool {
    let mut result = String::new();
    collect_static_word_text(parts, source, &mut result)
        && collect_static_word_text(trailing, source, &mut result)
        && result.starts_with('/')
}

fn parse_source_directives(
    source: &str,
    indexer: &Indexer,
) -> FxHashMap<usize, SourceDirectiveOverride> {
    let mut directives = FxHashMap::default();
    let mut pending_own_line: Option<SourceDirectiveOverride> = None;
    let mut previous_comment_line = None;

    for comment in indexer.comment_index().comments() {
        if !comment.is_own_line || previous_comment_line.is_none_or(|line| comment.line != line + 1)
        {
            pending_own_line = None;
        }

        if comment.is_own_line
            && let Some(directive) = pending_own_line.as_ref()
        {
            directives
                .entry(comment.line)
                .or_insert_with(|| directive.clone());
        }

        let text = comment.range.slice(source).trim_start_matches('#').trim();
        if let Some(directive) = parse_source_directive_override(text, comment.is_own_line) {
            directives.insert(comment.line, directive.clone());
            pending_own_line = comment.is_own_line.then_some(directive);
        }

        previous_comment_line = Some(comment.line);
    }
    directives
}

fn parse_source_directive_override(text: &str, own_line: bool) -> Option<SourceDirectiveOverride> {
    text.contains("shellcheck").then_some(())?;
    for part in text.split_whitespace() {
        if let Some(value) = part.strip_prefix("source=") {
            let kind = if value == "/dev/null" {
                SourceRefKind::DirectiveDevNull
            } else {
                SourceRefKind::Directive(value.to_string())
            };
            return Some(SourceDirectiveOverride { kind, own_line });
        }
    }

    None
}

fn arithmetic_name_span(span: Span, name: &Name) -> Span {
    Span::from_positions(span.start, span.start.advanced_by(name.as_str()))
}

fn is_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn depth_from_word(word: Option<&Word>) -> usize {
    word.and_then(single_literal_word)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|depth| *depth > 0)
        .unwrap_or(1)
}

fn single_literal_word(word: &Word) -> Option<&str> {
    match word.parts.as_slice() {
        [part] => match &part.kind {
            WordPart::Literal(
                shuck_ast::LiteralText::Owned(text) | shuck_ast::LiteralText::CookedSource(text),
            ) => Some(text.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

fn binding_origin_for_assignment(assignment: &Assignment) -> BindingOrigin {
    let value = if assignment.target.subscript.is_some() {
        AssignmentValueOrigin::ArrayOrCompound
    } else {
        match &assignment.value {
            AssignmentValue::Scalar(word) => assignment_value_origin_for_word(word),
            AssignmentValue::Compound(_) => AssignmentValueOrigin::ArrayOrCompound,
        }
    };

    BindingOrigin::Assignment {
        definition_span: assignment.target.name_span,
        value,
    }
}

fn loop_binding_origin_for_words(words: Option<&[Word]>) -> LoopValueOrigin {
    let Some(words) = words else {
        return LoopValueOrigin::ImplicitArgv;
    };

    if words.iter().all(word_is_static_binding_literal) {
        LoopValueOrigin::StaticWords
    } else {
        LoopValueOrigin::ExpandedWords
    }
}

fn assignment_value_origin_for_word(word: &Word) -> AssignmentValueOrigin {
    if !word.brace_syntax.is_empty() {
        return AssignmentValueOrigin::MixedDynamic;
    }
    if word_is_static_binding_literal(word) {
        return AssignmentValueOrigin::StaticLiteral;
    }

    let mut scan = AssignmentWordOriginScan::default();
    scan_assignment_word_parts(&word.parts, &mut scan);

    if scan.category_count() == 0 {
        return AssignmentValueOrigin::PlainScalarAccess;
    }
    if scan.mixed_dynamic || scan.category_count() > 1 {
        return AssignmentValueOrigin::MixedDynamic;
    }

    scan.primary_origin()
        .unwrap_or(AssignmentValueOrigin::Unknown)
}

#[derive(Debug, Default)]
struct AssignmentWordOriginScan {
    parameter_operator: bool,
    transformation: bool,
    indirect_expansion: bool,
    command_or_process_substitution: bool,
    array_or_compound: bool,
    mixed_dynamic: bool,
}

impl AssignmentWordOriginScan {
    fn category_count(&self) -> usize {
        [
            self.parameter_operator,
            self.transformation,
            self.indirect_expansion,
            self.command_or_process_substitution,
            self.array_or_compound,
            self.mixed_dynamic,
        ]
        .into_iter()
        .filter(|flag| *flag)
        .count()
    }

    fn primary_origin(&self) -> Option<AssignmentValueOrigin> {
        if self.parameter_operator {
            Some(AssignmentValueOrigin::ParameterOperator)
        } else if self.transformation {
            Some(AssignmentValueOrigin::Transformation)
        } else if self.indirect_expansion {
            Some(AssignmentValueOrigin::IndirectExpansion)
        } else if self.command_or_process_substitution {
            Some(AssignmentValueOrigin::CommandOrProcessSubstitution)
        } else if self.array_or_compound {
            Some(AssignmentValueOrigin::ArrayOrCompound)
        } else if self.mixed_dynamic {
            Some(AssignmentValueOrigin::MixedDynamic)
        } else {
            None
        }
    }
}

fn word_is_static_binding_literal(word: &Word) -> bool {
    word.brace_syntax.is_empty()
        && word
            .parts
            .iter()
            .all(|part| binding_literal_part_is_static(&part.kind))
}

fn binding_literal_part_is_static(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .all(|part| binding_literal_part_is_static(&part.kind)),
        WordPart::ZshQualifiedGlob(_)
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

fn scan_assignment_word_parts(parts: &[WordPartNode], scan: &mut AssignmentWordOriginScan) {
    for part in parts {
        scan_assignment_word_part(&part.kind, scan);
    }
}

fn scan_assignment_word_part(part: &WordPart, scan: &mut AssignmentWordOriginScan) {
    match part {
        WordPart::Literal(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::ArithmeticExpansion { .. } => {}
        WordPart::DoubleQuoted { parts, .. } => scan_assignment_word_parts(parts, scan),
        WordPart::Parameter(parameter) => scan_parameter_word_part(parameter, scan),
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {
            scan.command_or_process_substitution = true;
        }
        WordPart::ParameterExpansion { reference, .. } => {
            if reference.has_array_selector() {
                scan.array_or_compound = true;
            } else {
                scan.parameter_operator = true;
            }
        }
        WordPart::Length(_) | WordPart::Substring { .. } => scan.parameter_operator = true,
        WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::ArraySlice { .. } => scan.array_or_compound = true,
        WordPart::IndirectExpansion { .. } | WordPart::PrefixMatch { .. } => {
            scan.indirect_expansion = true;
        }
        WordPart::Transformation { .. } => scan.transformation = true,
        WordPart::ZshQualifiedGlob(_) => scan.mixed_dynamic = true,
    }
}

fn scan_parameter_word_part(parameter: &ParameterExpansion, scan: &mut AssignmentWordOriginScan) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
            if reference.has_array_selector() {
                scan.array_or_compound = true;
            }
        }
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { .. })
        | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation { .. })
        | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice { .. }) => {
            scan.parameter_operator = true;
        }
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indices { .. }) => {
            scan.array_or_compound = true;
        }
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indirect { .. })
        | ParameterExpansionSyntax::Bourne(BourneParameterExpansion::PrefixMatch { .. }) => {
            scan.indirect_expansion = true;
        }
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Transformation { .. }) => {
            scan.transformation = true;
        }
        ParameterExpansionSyntax::Zsh(_) => scan.mixed_dynamic = true,
    }
}

fn case_arm_matches_anything(patterns: &[Pattern]) -> bool {
    patterns.iter().any(pattern_matches_anything)
}

fn pattern_matches_anything(pattern: &Pattern) -> bool {
    !pattern.parts.is_empty()
        && pattern
            .parts
            .iter()
            .all(|part| pattern_part_can_match_empty(&part.kind))
        && pattern
            .parts
            .iter()
            .any(|part| pattern_part_matches_anything(&part.kind))
}

fn pattern_can_match_empty(pattern: &Pattern) -> bool {
    pattern
        .parts
        .iter()
        .all(|part| pattern_part_can_match_empty(&part.kind))
}

fn pattern_part_matches_anything(part: &PatternPart) -> bool {
    match part {
        PatternPart::AnyString => true,
        PatternPart::Group { kind, patterns } => pattern_group_matches_anything(*kind, patterns),
        PatternPart::Literal(_)
        | PatternPart::AnyChar
        | PatternPart::CharClass(_)
        | PatternPart::Word(_) => false,
    }
}

fn pattern_part_can_match_empty(part: &PatternPart) -> bool {
    match part {
        PatternPart::AnyString => true,
        PatternPart::Group { kind, patterns } => pattern_group_can_match_empty(*kind, patterns),
        PatternPart::Literal(_)
        | PatternPart::AnyChar
        | PatternPart::CharClass(_)
        | PatternPart::Word(_) => false,
    }
}

fn pattern_group_matches_anything(kind: PatternGroupKind, patterns: &[Pattern]) -> bool {
    match kind {
        PatternGroupKind::ZeroOrOne
        | PatternGroupKind::ZeroOrMore
        | PatternGroupKind::OneOrMore
        | PatternGroupKind::ExactlyOne => patterns.iter().any(pattern_matches_anything),
        PatternGroupKind::NoneOf => false,
    }
}

fn pattern_group_can_match_empty(kind: PatternGroupKind, patterns: &[Pattern]) -> bool {
    match kind {
        PatternGroupKind::ZeroOrOne | PatternGroupKind::ZeroOrMore => true,
        PatternGroupKind::OneOrMore | PatternGroupKind::ExactlyOne => {
            patterns.iter().any(pattern_can_match_empty)
        }
        PatternGroupKind::NoneOf => false,
    }
}

fn word_is_semantically_inert(word: &Word) -> bool {
    word.parts
        .iter()
        .all(|part| word_part_is_semantically_inert(&part.kind))
}

fn heredoc_body_is_semantically_inert(body: &HeredocBody) -> bool {
    body.parts
        .iter()
        .all(|part| heredoc_body_part_is_semantically_inert(&part.kind))
}

fn word_part_is_semantically_inert(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::ZshQualifiedGlob(glob) => zsh_qualified_glob_is_semantically_inert(glob),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .all(|part| word_part_is_semantically_inert(&part.kind)),
        WordPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_none(),
        WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

fn heredoc_body_part_is_semantically_inert(part: &HeredocBodyPart) -> bool {
    match part {
        HeredocBodyPart::Literal(_) => true,
        HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_none(),
        HeredocBodyPart::Variable(_)
        | HeredocBodyPart::CommandSubstitution { .. }
        | HeredocBodyPart::Parameter(_) => false,
    }
}

fn zsh_qualified_glob_is_semantically_inert(glob: &shuck_ast::ZshQualifiedGlob) -> bool {
    glob.segments.iter().all(|segment| match segment {
        ZshGlobSegment::Pattern(pattern) => pattern_is_semantically_inert(pattern),
        ZshGlobSegment::InlineControl(_) => true,
    })
}

fn pattern_is_semantically_inert(pattern: &Pattern) -> bool {
    pattern
        .parts
        .iter()
        .all(|part| pattern_part_is_semantically_inert(&part.kind))
}

fn pattern_part_is_semantically_inert(part: &PatternPart) -> bool {
    match part {
        PatternPart::Literal(_)
        | PatternPart::AnyString
        | PatternPart::AnyChar
        | PatternPart::CharClass(_) => true,
        PatternPart::Group { patterns, .. } => patterns.iter().all(pattern_is_semantically_inert),
        PatternPart::Word(word) => word_is_semantically_inert(word),
    }
}

fn ancestor_scopes(scopes: &[Scope], start: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
    std::iter::successors(Some(start), move |scope| scopes[scope.index()].parent)
}

fn is_in_function_scope(scopes: &[Scope], scope: ScopeId) -> bool {
    ancestor_scopes(scopes, scope)
        .skip(1)
        .any(|scope| matches!(scopes[scope.index()].kind, ScopeKind::Function(_)))
}

fn is_in_named_function_scope(scopes: &[Scope], scope: ScopeId, name: &Name) -> bool {
    ancestor_scopes(scopes, scope).any(|scope| {
        matches!(
            &scopes[scope.index()].kind,
            ScopeKind::Function(function) if function.contains_name(name)
        )
    })
}

fn function_scope_kind(function: &FunctionDef) -> FunctionScopeKind {
    let names = function.static_names().cloned().collect::<Vec<_>>();
    if names.is_empty() {
        FunctionScopeKind::Dynamic
    } else {
        FunctionScopeKind::Named(names)
    }
}

fn body_span(command: &Stmt) -> Span {
    match &command.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) if !commands.is_empty() => {
            commands.span
        }
        _ => command.span,
    }
}

fn command_span_from_compound(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            commands.span
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}

fn collect_pipeline_segments<'a>(stmt: &'a Stmt, out: &mut SmallVec<[&'a Stmt; 4]>) {
    match &stmt.command {
        Command::Binary(command) if matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(&command.left, out);
            collect_pipeline_segments(&command.right, out);
        }
        _ => out.push(stmt),
    }
}

fn collect_logical_segments<'a>(
    stmt: &'a Stmt,
    commands: &mut SmallVec<[&'a Stmt; 4]>,
    operators: &mut SmallVec<[RecordedListOperator; 4]>,
) {
    match &stmt.command {
        Command::Binary(command) if matches!(command.op, BinaryOp::And | BinaryOp::Or) => {
            collect_logical_segments(&command.left, commands, operators);
            operators.push(recorded_list_operator(command.op));
            collect_logical_segments(&command.right, commands, operators);
        }
        _ => commands.push(stmt),
    }
}

fn recorded_list_operator(op: BinaryOp) -> RecordedListOperator {
    match op {
        BinaryOp::And => RecordedListOperator::And,
        BinaryOp::Or => RecordedListOperator::Or,
        BinaryOp::Pipe | BinaryOp::PipeAll => {
            unreachable!("pipeline operators are not valid in logical lists")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_ast::{LiteralText, SourceText};

    fn word(parts: Vec<WordPart>) -> Word {
        let span = Span::new();
        Word {
            parts: parts
                .into_iter()
                .map(|part| WordPartNode::new(part, span))
                .collect(),
            span,
            brace_syntax: Vec::new(),
        }
    }

    fn pattern(parts: Vec<PatternPart>) -> Pattern {
        let span = Span::new();
        Pattern {
            parts: parts
                .into_iter()
                .map(|part| PatternPartNode::new(part, span))
                .collect(),
            span,
        }
    }

    #[test]
    fn inert_word_short_circuits_literal_shapes() {
        let word = word(vec![
            WordPart::Literal(LiteralText::owned("plain")),
            WordPart::DoubleQuoted {
                parts: vec![WordPartNode::new(
                    WordPart::Literal(LiteralText::owned("quoted")),
                    Span::new(),
                )],
                dollar: false,
            },
            WordPart::SingleQuoted {
                value: SourceText::from("single"),
                dollar: false,
            },
        ]);

        assert!(word_is_semantically_inert(&word));
    }

    #[test]
    fn word_with_variable_expansion_is_not_inert() {
        let word = word(vec![
            WordPart::Literal(LiteralText::owned("prefix")),
            WordPart::Variable("HOME".into()),
        ]);

        assert!(!word_is_semantically_inert(&word));
    }

    #[test]
    fn word_with_nested_command_substitution_is_not_inert() {
        let word = word(vec![WordPart::DoubleQuoted {
            parts: vec![WordPartNode::new(
                WordPart::CommandSubstitution {
                    body: StmtSeq {
                        leading_comments: Vec::new(),
                        stmts: Vec::new(),
                        trailing_comments: Vec::new(),
                        span: Span::new(),
                    },
                    syntax: shuck_ast::CommandSubstitutionSyntax::DollarParen,
                },
                Span::new(),
            )],
            dollar: false,
        }]);

        assert!(!word_is_semantically_inert(&word));
    }

    #[test]
    fn inert_zsh_qualified_glob_short_circuits() {
        let word = word(vec![WordPart::ZshQualifiedGlob(
            shuck_ast::ZshQualifiedGlob {
                span: Span::new(),
                segments: vec![
                    ZshGlobSegment::Pattern(pattern(vec![
                        PatternPart::Literal(LiteralText::owned("foo")),
                        PatternPart::AnyString,
                        PatternPart::Group {
                            kind: PatternGroupKind::ExactlyOne,
                            patterns: vec![pattern(vec![PatternPart::CharClass(
                                SourceText::from("[ab]"),
                            )])],
                        },
                    ])),
                    ZshGlobSegment::InlineControl(shuck_ast::ZshInlineGlobControl::StartAnchor {
                        span: Span::new(),
                    }),
                ],
                qualifiers: None,
            },
        )]);

        assert!(word_is_semantically_inert(&word));
    }

    #[test]
    fn pattern_with_expanding_word_is_not_inert() {
        let pattern = pattern(vec![PatternPart::Word(word(vec![
            WordPart::ParameterExpansion {
                reference: VarRef {
                    name: "name".into(),
                    name_span: Span::new(),
                    subscript: None,
                    span: Span::new(),
                },
                operator: ParameterOp::UseDefault,
                operand: Some(SourceText::from("fallback")),
                operand_word_ast: Some(Word::literal("fallback")),
                colon_variant: true,
            },
        ]))]);

        assert!(!pattern_is_semantically_inert(&pattern));
    }
}
