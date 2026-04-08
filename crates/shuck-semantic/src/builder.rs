use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticAssignOp, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticLvalue, ArithmeticUnaryOp, ArrayElem, ArrayExpr, ArrayKind, Assignment,
    AssignmentValue, BinaryCommand, BinaryOp, BourneParameterExpansion, BuiltinCommand, Command,
    CompoundCommand, ConditionalExpr, DeclOperand, File, FunctionDef, Name, ParameterExpansion,
    ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart, PatternPartNode, Span, Stmt,
    StmtSeq, Subscript, VarRef, Word, WordPart, WordPartNode, ZshExpansionOperation,
    ZshExpansionTarget, ZshGlobSegment,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;

use crate::binding::{Binding, BindingAttributes, BindingKind};
use crate::call_graph::{CallGraph, CallSite, OverwrittenFunction};
use crate::cfg::{
    FlowContext, IsolatedRegion, RecordedCaseArm, RecordedCommand, RecordedCommandKind,
    RecordedListOperator, RecordedPipelineSegment, RecordedProgram,
};
use crate::declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
use crate::reference::{Reference, ReferenceKind};
use crate::runtime::RuntimePrelude;
use crate::source_ref::{SourceRef, SourceRefKind};
use crate::{
    BindingId, FunctionScopeKind, IndirectTargetHint, ReferenceId, Scope, ScopeId, ScopeKind,
    SourceDirectiveOverride, SpanKey, TraversalObserver,
};

pub(crate) struct BuildOutput {
    pub(crate) scopes: Vec<Scope>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) references: Vec<Reference>,
    pub(crate) predefined_runtime_refs: FxHashSet<ReferenceId>,
    pub(crate) guarded_parameter_refs: FxHashSet<ReferenceId>,
    pub(crate) binding_index: FxHashMap<Name, Vec<BindingId>>,
    pub(crate) resolved: FxHashMap<ReferenceId, BindingId>,
    pub(crate) unresolved: Vec<ReferenceId>,
    pub(crate) functions: FxHashMap<Name, Vec<BindingId>>,
    pub(crate) call_sites: FxHashMap<Name, Vec<CallSite>>,
    pub(crate) call_graph: CallGraph,
    pub(crate) source_refs: Vec<SourceRef>,
    pub(crate) runtime: RuntimePrelude,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) indirect_target_hints: FxHashMap<BindingId, IndirectTargetHint>,
    pub(crate) indirect_expansion_refs: FxHashSet<ReferenceId>,
    pub(crate) flow_contexts: Vec<(Span, FlowContext)>,
    pub(crate) recorded_program: RecordedProgram,
    pub(crate) command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    pub(crate) command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    pub(crate) heuristic_unused_assignments: Vec<BindingId>,
}

pub(crate) struct SemanticModelBuilder<'a, 'observer> {
    source: &'a str,
    observer: &'observer mut dyn TraversalObserver,
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    predefined_runtime_refs: FxHashSet<ReferenceId>,
    guarded_parameter_refs: FxHashSet<ReferenceId>,
    binding_index: FxHashMap<Name, Vec<BindingId>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, Vec<BindingId>>,
    call_sites: FxHashMap<Name, Vec<CallSite>>,
    source_refs: Vec<SourceRef>,
    declarations: Vec<Declaration>,
    indirect_target_hints: FxHashMap<BindingId, IndirectTargetHint>,
    indirect_expansion_refs: FxHashSet<ReferenceId>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_function_bodies: FxHashMap<ScopeId, Vec<RecordedCommand>>,
    command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    source_directives: FxHashMap<usize, SourceDirectiveOverride>,
    runtime: RuntimePrelude,
    completed_scopes: FxHashSet<ScopeId>,
    deferred_functions: Vec<DeferredFunction>,
    scope_stack: Vec<ScopeId>,
    command_stack: Vec<Span>,
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
            recorded_function_bodies: FxHashMap::default(),
            command_bindings: FxHashMap::default(),
            command_references: FxHashMap::default(),
            source_directives: parse_source_directives(source, indexer),
            runtime,
            completed_scopes: FxHashSet::default(),
            deferred_functions: Vec::new(),
            scope_stack: vec![ScopeId(0)],
            command_stack: Vec::new(),
        };
        let file_commands = builder.visit_stmt_seq(&file.body, FlowState::default());
        builder.mark_scope_completed(ScopeId(0));
        builder.drain_deferred_functions();

        let call_graph = builder.build_call_graph();
        let heuristic_unused_assignments = builder.compute_heuristic_unused_assignments();

        BuildOutput {
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: builder.references,
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
            recorded_program: RecordedProgram {
                file_commands,
                function_bodies: builder.recorded_function_bodies,
            },
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

    fn visit_stmt_seq(&mut self, commands: &StmtSeq, flow: FlowState) -> Vec<RecordedCommand> {
        let mut recorded = Vec::with_capacity(commands.len());
        self.visit_stmt_seq_into(commands, flow, &mut recorded);
        recorded
    }

    fn visit_stmt_seq_into(
        &mut self,
        commands: &StmtSeq,
        flow: FlowState,
        recorded: &mut Vec<RecordedCommand>,
    ) {
        recorded.reserve(commands.len());
        for stmt in commands.iter() {
            recorded.push(self.visit_stmt(stmt, flow));
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, flow: FlowState) -> RecordedCommand {
        let span = stmt.span;
        let scope = self.current_scope();
        let context = Self::flow_context(flow);
        self.flow_contexts.push((span, context.clone()));
        self.observer.enter_command(&stmt.command, scope, context);
        self.command_stack.push(span);

        let mut recorded = self.visit_command(&stmt.command, flow);
        let redirects = self.visit_redirects(&stmt.redirects, flow);
        if !redirects.is_empty() {
            recorded.nested_regions.splice(0..0, redirects);
        }
        recorded.span = span;

        self.command_stack.pop();
        self.observer.exit_command(&stmt.command, scope);
        recorded
    }

    fn visit_command(&mut self, command: &Command, flow: FlowState) -> RecordedCommand {
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
    ) -> RecordedCommand {
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

        if let Some(name) = static_word_text(&command.name, self.source)
            && !name.is_empty()
        {
            let callee = Name::from(name.as_str());
            let scope = self.current_scope();
            let call_site = CallSite {
                callee: callee.clone(),
                span: command.span,
                scope,
                arg_count: command.args.len(),
            };
            match self.call_sites.get_mut(callee.as_str()) {
                Some(v) => v.push(call_site),
                None => {
                    self.call_sites.insert(callee.clone(), vec![call_site]);
                }
            }

            self.classify_special_simple_command(&callee, command, flow);
        }

        RecordedCommand {
            span: command.span,
            nested_regions,
            kind: RecordedCommandKind::Linear,
        }
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand, flow: FlowState) -> RecordedCommand {
        match command {
            BuiltinCommand::Break(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_builtin_parts(
                    &command.assignments,
                    command.depth.as_ref(),
                    &command.extra_args,
                    flow,
                ),
                kind: RecordedCommandKind::Break {
                    depth: depth_from_word(command.depth.as_ref()),
                },
            },
            BuiltinCommand::Continue(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_builtin_parts(
                    &command.assignments,
                    command.depth.as_ref(),
                    &command.extra_args,
                    flow,
                ),
                kind: RecordedCommandKind::Continue {
                    depth: depth_from_word(command.depth.as_ref()),
                },
            },
            BuiltinCommand::Return(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_builtin_parts(
                    &command.assignments,
                    command.code.as_ref(),
                    &command.extra_args,
                    flow,
                ),
                kind: RecordedCommandKind::Return,
            },
            BuiltinCommand::Exit(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_builtin_parts(
                    &command.assignments,
                    command.code.as_ref(),
                    &command.extra_args,
                    flow,
                ),
                kind: RecordedCommandKind::Exit,
            },
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

    fn visit_decl(&mut self, command: &shuck_ast::DeclClause, flow: FlowState) -> RecordedCommand {
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
                        builtin, &flags, &name.name, name.span,
                    );
                }
                DeclOperand::Assignment(assignment) => {
                    let (scope, mut attributes) =
                        self.declaration_scope_and_attributes(builtin, &flags);
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

        RecordedCommand {
            span: command.span,
            nested_regions,
            kind: RecordedCommandKind::Linear,
        }
    }

    fn visit_binary(&mut self, command: &BinaryCommand, flow: FlowState) -> RecordedCommand {
        match command.op {
            BinaryOp::And | BinaryOp::Or => self.visit_logical_binary(command, flow),
            BinaryOp::Pipe | BinaryOp::PipeAll => self.visit_pipeline_binary(command, flow),
        }
    }

    fn visit_pipeline_binary(
        &mut self,
        command: &BinaryCommand,
        mut flow: FlowState,
    ) -> RecordedCommand {
        flow.in_subshell = true;
        let mut commands = Vec::new();
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

        RecordedCommand {
            span: command.span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::Pipeline { segments },
        }
    }

    fn visit_logical_binary(
        &mut self,
        command: &BinaryCommand,
        flow: FlowState,
    ) -> RecordedCommand {
        let mut operators = Vec::new();
        let mut commands = Vec::new();
        collect_logical_segments(&command.left, &mut commands, &mut operators);
        operators.push(recorded_list_operator(command.op));
        collect_logical_segments(&command.right, &mut commands, &mut operators);

        let mut recorded = Vec::with_capacity(commands.len());
        for (index, stmt) in commands.into_iter().enumerate() {
            let mut nested = flow;
            nested.exit_status_checked = operators.get(index).is_some() || flow.exit_status_checked;
            recorded.push(self.visit_stmt(stmt, nested));
        }

        let mut recorded = recorded.into_iter();
        let first = Box::new(
            recorded
                .next()
                .expect("logical lists have at least one command"),
        );
        let rest = operators.into_iter().zip(recorded).collect();

        RecordedCommand {
            span: command.span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::List { first, rest },
        }
    }

    fn visit_compound(&mut self, command: &CompoundCommand, flow: FlowState) -> RecordedCommand {
        match command {
            CompoundCommand::If(command) => RecordedCommand {
                span: command.span,
                nested_regions: Vec::new(),
                kind: RecordedCommandKind::If {
                    condition: self.visit_stmt_seq(
                        &command.condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    ),
                    then_branch: self.visit_stmt_seq(&command.then_branch, flow),
                    elif_branches: command
                        .elif_branches
                        .iter()
                        .map(|(condition, body)| {
                            (
                                self.visit_stmt_seq(
                                    condition,
                                    FlowState {
                                        exit_status_checked: true,
                                        ..flow
                                    },
                                ),
                                self.visit_stmt_seq(body, flow),
                            )
                        })
                        .collect(),
                    else_branch: command
                        .else_branch
                        .as_ref()
                        .map(|body| self.visit_stmt_seq(body, flow))
                        .unwrap_or_default(),
                },
            },
            CompoundCommand::For(command) => {
                let nested_regions = command
                    .words
                    .as_deref()
                    .map(|words| self.visit_words(words, WordVisitKind::Expansion, flow))
                    .unwrap_or_default();
                for target in &command.targets {
                    self.add_binding(
                        &target.name,
                        BindingKind::LoopVariable,
                        self.current_scope(),
                        target.span,
                        BindingAttributes::empty(),
                    );
                }

                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::For {
                        body: self.visit_stmt_seq(
                            &command.body,
                            FlowState {
                                loop_depth: flow.loop_depth + 1,
                                ..flow
                            },
                        ),
                    },
                }
            }
            CompoundCommand::Repeat(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_word(&command.count, WordVisitKind::Expansion, flow),
                kind: RecordedCommandKind::For {
                    body: self.visit_stmt_seq(
                        &command.body,
                        FlowState {
                            loop_depth: flow.loop_depth + 1,
                            ..flow
                        },
                    ),
                },
            },
            CompoundCommand::Foreach(command) => {
                let nested_regions =
                    self.visit_words(&command.words, WordVisitKind::Expansion, flow);
                self.add_binding(
                    &command.variable,
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingAttributes::empty(),
                );

                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::For {
                        body: self.visit_stmt_seq(
                            &command.body,
                            FlowState {
                                loop_depth: flow.loop_depth + 1,
                                ..flow
                            },
                        ),
                    },
                }
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
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::ArithmeticFor {
                        body: self.visit_stmt_seq(
                            &command.body,
                            FlowState {
                                loop_depth: flow.loop_depth + 1,
                                ..flow
                            },
                        ),
                    },
                }
            }
            CompoundCommand::While(command) => RecordedCommand {
                span: command.span,
                nested_regions: Vec::new(),
                kind: RecordedCommandKind::While {
                    condition: self.visit_stmt_seq(
                        &command.condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    ),
                    body: self.visit_stmt_seq(
                        &command.body,
                        FlowState {
                            loop_depth: flow.loop_depth + 1,
                            ..flow
                        },
                    ),
                },
            },
            CompoundCommand::Until(command) => RecordedCommand {
                span: command.span,
                nested_regions: Vec::new(),
                kind: RecordedCommandKind::Until {
                    condition: self.visit_stmt_seq(
                        &command.condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    ),
                    body: self.visit_stmt_seq(
                        &command.body,
                        FlowState {
                            loop_depth: flow.loop_depth + 1,
                            ..flow
                        },
                    ),
                },
            },
            CompoundCommand::Case(command) => {
                let nested_regions = self.visit_word(&command.word, WordVisitKind::Expansion, flow);

                let arms = command
                    .cases
                    .iter()
                    .map(|case| {
                        let pattern_regions =
                            self.visit_patterns(&case.patterns, WordVisitKind::Conditional, flow);
                        let mut commands = self.visit_stmt_seq(&case.body, flow);
                        if !pattern_regions.is_empty() {
                            if let Some(first) = commands.first_mut() {
                                first.nested_regions.splice(0..0, pattern_regions);
                            } else {
                                commands.push(RecordedCommand {
                                    span: command.span,
                                    nested_regions: pattern_regions,
                                    kind: RecordedCommandKind::Linear,
                                });
                            }
                        }
                        RecordedCaseArm {
                            terminator: case.terminator,
                            commands,
                        }
                    })
                    .collect();

                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Case { arms },
                }
            }
            CompoundCommand::Select(command) => {
                let nested_regions =
                    self.visit_words(&command.words, WordVisitKind::Expansion, flow);
                self.add_binding(
                    &command.variable,
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingAttributes::empty(),
                );

                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Select {
                        body: self.visit_stmt_seq(
                            &command.body,
                            FlowState {
                                loop_depth: flow.loop_depth + 1,
                                ..flow
                            },
                        ),
                    },
                }
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

                RecordedCommand {
                    span: command_span_from_compound(command),
                    nested_regions: Vec::new(),
                    kind: RecordedCommandKind::Subshell { body },
                }
            }
            CompoundCommand::BraceGroup(commands) => RecordedCommand {
                span: command_span_from_compound(command),
                nested_regions: Vec::new(),
                kind: RecordedCommandKind::BraceGroup {
                    body: self.visit_stmt_seq(
                        commands,
                        FlowState {
                            in_block: true,
                            ..flow
                        },
                    ),
                },
            },
            CompoundCommand::Always(command) => RecordedCommand {
                span: command.span,
                nested_regions: Vec::new(),
                kind: RecordedCommandKind::BraceGroup {
                    body: {
                        let block_flow = FlowState {
                            in_block: true,
                            ..flow
                        };
                        let mut body =
                            Vec::with_capacity(command.body.len() + command.always_body.len());
                        self.visit_stmt_seq_into(&command.body, block_flow, &mut body);
                        self.visit_stmt_seq_into(&command.always_body, block_flow, &mut body);
                        body
                    },
                },
            },
            CompoundCommand::Arithmetic(command) => {
                let nested_regions =
                    self.visit_optional_arithmetic_expr(command.expr_ast.as_ref(), flow);
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Linear,
                }
            }
            CompoundCommand::Time(command) => {
                let mut nested_regions = Vec::new();
                if let Some(command) = &command.command {
                    nested_regions.extend(Self::flatten_recorded_regions(
                        self.visit_stmt(command, flow),
                    ));
                }
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Linear,
                }
            }
            CompoundCommand::Conditional(command) => {
                let nested_regions = self.visit_conditional_expr(&command.expression, flow);
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Linear,
                }
            }
            CompoundCommand::Coproc(command) => RecordedCommand {
                span: command.span,
                nested_regions: Self::flatten_recorded_regions(self.visit_stmt(
                    &command.body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                )),
                kind: RecordedCommandKind::Linear,
            },
        }
    }

    fn visit_function(&mut self, function: &FunctionDef, flow: FlowState) -> RecordedCommand {
        let mut nested_regions = Vec::new();
        for entry in &function.header.entries {
            self.visit_word_into(
                &entry.word,
                WordVisitKind::Expansion,
                flow,
                &mut nested_regions,
            );
        }

        for (name, span) in function.static_name_entries() {
            self.add_binding(
                name,
                BindingKind::FunctionDefinition,
                self.current_scope(),
                span,
                BindingAttributes::empty(),
            );
        }

        let scope = self.push_scope(
            ScopeKind::Function(function_scope_kind(function)),
            self.current_scope(),
            body_span(&function.body),
        );
        self.deferred_functions.push(DeferredFunction {
            function: function as *const FunctionDef,
            scope,
            flow,
        });
        self.pop_scope(scope);

        RecordedCommand {
            span: function.span,
            nested_regions,
            kind: RecordedCommandKind::Linear,
        }
    }

    fn visit_anonymous_function(
        &mut self,
        function: &AnonymousFunctionCommand,
        flow: FlowState,
    ) -> RecordedCommand {
        let nested_regions = self.visit_words(&function.args, WordVisitKind::Expansion, flow);
        let scope = self.push_scope(
            ScopeKind::Function(FunctionScopeKind::Anonymous),
            self.current_scope(),
            body_span(&function.body),
        );
        let body = self.visit_function_like_body(&function.body, flow);
        self.pop_scope(scope);
        self.mark_scope_completed(scope);

        RecordedCommand {
            span: function.span,
            nested_regions,
            kind: RecordedCommandKind::BraceGroup { body },
        }
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
            let word = match redirect.word_target() {
                Some(word) => word,
                None => &redirect.heredoc().expect("expected heredoc redirect").body,
            };
            self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
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
        self.visit_word_part_nodes(&word.parts, kind, flow, nested_regions);
    }

    fn visit_pattern_into(
        &mut self,
        pattern: &Pattern,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
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

        let text = subscript.syntax_source_text();
        let word = Parser::parse_word_fragment(self.source, text.slice(self.source), text.span());
        self.visit_word_into(&word, kind, flow, nested_regions);
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
                for segment in &glob.segments {
                    if let ZshGlobSegment::Pattern(pattern) = segment {
                        self.visit_pattern_into(pattern, kind, flow, nested_regions);
                    }
                }
            }
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
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
                nested_regions.push(IsolatedRegion { scope, commands });
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_into(
                    expression_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            WordPart::Parameter(parameter) => {
                self.visit_parameter_expansion(parameter, kind, flow, nested_regions, span);
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
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
                match operator {
                    ParameterOp::RemovePrefixShort { pattern }
                    | ParameterOp::RemovePrefixLong { pattern }
                    | ParameterOp::RemoveSuffixShort { pattern }
                    | ParameterOp::RemoveSuffixLong { pattern }
                    | ParameterOp::ReplaceFirst { pattern, .. }
                    | ParameterOp::ReplaceAll { pattern, .. } => {
                        self.visit_pattern_into(pattern, kind, flow, nested_regions);
                    }
                    ParameterOp::UseDefault
                    | ParameterOp::AssignDefault
                    | ParameterOp::UseReplacement
                    | ParameterOp::Error
                    | ParameterOp::UpperFirst
                    | ParameterOp::UpperAll
                    | ParameterOp::LowerFirst
                    | ParameterOp::LowerAll => {}
                }
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
                    span,
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
                    span,
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
                    span,
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
            WordPart::IndirectExpansion { name, .. } => {
                let id = self.add_reference(
                    name,
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::IndirectExpansion
                    },
                    span,
                );
                self.indirect_expansion_refs.insert(id);
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
                    span,
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
                    span,
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
                    span,
                );
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
                BourneParameterExpansion::Indirect { name, .. } => {
                    let id = self.add_reference(
                        name,
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::IndirectExpansion
                        },
                        span,
                    );
                    self.indirect_expansion_refs.insert(id);
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
                    match operator {
                        ParameterOp::RemovePrefixShort { pattern }
                        | ParameterOp::RemovePrefixLong { pattern }
                        | ParameterOp::RemoveSuffixShort { pattern }
                        | ParameterOp::RemoveSuffixLong { pattern }
                        | ParameterOp::ReplaceFirst { pattern, .. }
                        | ParameterOp::ReplaceAll { pattern, .. } => {
                            self.visit_pattern_into(pattern, kind, flow, nested_regions);
                        }
                        ParameterOp::UseDefault
                        | ParameterOp::AssignDefault
                        | ParameterOp::UseReplacement
                        | ParameterOp::Error
                        | ParameterOp::UpperFirst
                        | ParameterOp::UpperAll
                        | ParameterOp::LowerFirst
                        | ParameterOp::LowerAll => {}
                    }
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
                    ZshExpansionTarget::Nested(parameter) => {
                        self.visit_parameter_expansion(parameter, kind, flow, nested_regions, span);
                    }
                    ZshExpansionTarget::Empty => {}
                }

                for modifier in &syntax.modifiers {
                    if let Some(argument) = &modifier.argument {
                        self.visit_source_text_as_word(argument, kind, flow, nested_regions);
                    }
                }

                if let Some(operation) = &syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation { operand, .. }
                        | ZshExpansionOperation::Defaulting { operand, .. }
                        | ZshExpansionOperation::TrimOperation { operand, .. }
                        | ZshExpansionOperation::Unknown(operand) => {
                            self.visit_source_text_as_word(operand, kind, flow, nested_regions);
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            replacement,
                            ..
                        } => {
                            self.visit_source_text_as_word(pattern, kind, flow, nested_regions);
                            if let Some(replacement) = replacement {
                                self.visit_source_text_as_word(
                                    replacement,
                                    kind,
                                    flow,
                                    nested_regions,
                                );
                            }
                        }
                        ZshExpansionOperation::Slice { offset, length } => {
                            self.visit_source_text_as_word(offset, kind, flow, nested_regions);
                            if let Some(length) = length {
                                self.visit_source_text_as_word(length, kind, flow, nested_regions);
                            }
                        }
                    }
                }
            }
        }
    }

    fn visit_source_text_as_word(
        &mut self,
        text: &shuck_ast::SourceText,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let word = Parser::parse_word_fragment(self.source, text.slice(self.source), text.span());
        self.visit_word_into(&word, kind, flow, nested_regions);
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
                self.visit_arithmetic_expr_into(index, flow, nested_regions);
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
                    BindingAttributes::empty(),
                );
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.visit_arithmetic_expr_into(index, flow, nested_regions);
                let span = arithmetic_name_span(expr.span, name);
                self.add_reference(name, ReferenceKind::ArithmeticRead, span);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    span,
                    BindingAttributes::ARRAY,
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
        let (name, attributes) = match target {
            ArithmeticLvalue::Variable(name) => (name, BindingAttributes::empty()),
            ArithmeticLvalue::Indexed { name, .. } => (name, BindingAttributes::ARRAY),
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
            ArithmeticLvalue::Indexed { index, .. } => {
                self.visit_arithmetic_expr_into(index, flow, nested_regions);
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
                        BindingAttributes::empty(),
                    );
                }
            }
            "source" | "." => {
                if let Some(argument) = command.args.first() {
                    self.source_refs.push(SourceRef {
                        kind: self.classify_source_ref(command.span.line(), argument),
                        span: command.span,
                        path_span: argument.span,
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

        let local_like = matches!(builtin, DeclarationBuiltin::Local)
            || (matches!(
                builtin,
                DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset
            ) && self.nearest_function_scope().is_some()
                && !flags.contains(&'g'));

        if local_like {
            attributes |= BindingAttributes::LOCAL;
        }

        (
            if local_like {
                self.nearest_function_scope()
                    .unwrap_or_else(|| self.current_scope())
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
        name: &Name,
        span: Span,
    ) {
        let (scope, attributes) = self.declaration_scope_and_attributes(builtin, flags);
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
        self.add_binding(name, kind, scope, span, attributes);
    }

    fn add_binding(
        &mut self,
        name: &Name,
        kind: BindingKind,
        scope: ScopeId,
        span: Span,
        attributes: BindingAttributes,
    ) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            id,
            name: name.clone(),
            kind,
            scope,
            span,
            references: Vec::new(),
            attributes,
        });
        match self.binding_index.get_mut(name.as_str()) {
            Some(v) => v.push(id),
            None => {
                self.binding_index.insert(name.clone(), vec![id]);
            }
        }
        match self.scopes[scope.index()].bindings.get_mut(name.as_str()) {
            Some(v) => v.push(id),
            None => {
                self.scopes[scope.index()]
                    .bindings
                    .insert(name.clone(), vec![id]);
            }
        }
        if matches!(kind, BindingKind::FunctionDefinition) {
            match self.functions.get_mut(name.as_str()) {
                Some(v) => v.push(id),
                None => {
                    self.functions.insert(name.clone(), vec![id]);
                }
            }
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

    fn add_parameter_default_binding(&mut self, reference: &VarRef) {
        self.add_binding(
            &reference.name,
            BindingKind::ParameterDefaultAssignment,
            self.current_scope(),
            reference.name_span,
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
                self.recorded_function_bodies
                    .insert(deferred.scope, commands);
                self.mark_scope_completed(deferred.scope);
            }
        }
        self.rebuild_scope_stack(ScopeId(0));
        self.command_stack.clear();
    }

    fn visit_function_like_body(&mut self, body: &Stmt, flow: FlowState) -> Vec<RecordedCommand> {
        let flow = FlowState {
            in_function: true,
            ..flow
        };

        match &body.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                self.visit_stmt_seq(commands, flow)
            }
            _ => vec![self.visit_stmt(body, flow)],
        }
    }

    fn rebuild_scope_stack(&mut self, scope: ScopeId) {
        self.scope_stack = ancestor_scopes(&self.scopes, scope).collect::<Vec<_>>();
        self.scope_stack.reverse();
    }

    fn flatten_recorded_regions(recorded: RecordedCommand) -> Vec<IsolatedRegion> {
        let RecordedCommand {
            nested_regions,
            kind,
            ..
        } = recorded;
        let mut regions = nested_regions;

        match kind {
            RecordedCommandKind::Linear
            | RecordedCommandKind::Break { .. }
            | RecordedCommandKind::Continue { .. }
            | RecordedCommandKind::Return
            | RecordedCommandKind::Exit => {}
            RecordedCommandKind::List { first, rest } => {
                regions.extend(Self::flatten_recorded_regions(*first));
                for (_, command) in rest {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                for command in condition {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
                for command in then_branch {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
                for (condition, branch) in elif_branches {
                    for command in condition {
                        regions.extend(Self::flatten_recorded_regions(command));
                    }
                    for command in branch {
                        regions.extend(Self::flatten_recorded_regions(command));
                    }
                }
                for command in else_branch {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::While { condition, body }
            | RecordedCommandKind::Until { condition, body } => {
                for command in condition {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
                for command in body {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body }
            | RecordedCommandKind::BraceGroup { body }
            | RecordedCommandKind::Subshell { body } => {
                for command in body {
                    regions.extend(Self::flatten_recorded_regions(command));
                }
            }
            RecordedCommandKind::Case { arms } => {
                for arm in arms {
                    for command in arm.commands {
                        regions.extend(Self::flatten_recorded_regions(command));
                    }
                }
            }
            RecordedCommandKind::Pipeline { segments } => {
                for segment in segments {
                    regions.extend(Self::flatten_recorded_regions(segment.command));
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

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    collect_static_word_text(&word.parts, source, &mut result).then_some(result)
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

fn parse_source_directives(
    source: &str,
    indexer: &Indexer,
) -> FxHashMap<usize, SourceDirectiveOverride> {
    let mut directives = FxHashMap::default();
    for comment in indexer.comment_index().comments() {
        let text = comment.range.slice(source).trim_start_matches('#').trim();
        if !text.contains("shellcheck") {
            continue;
        }
        for part in text.split_whitespace() {
            if let Some(value) = part.strip_prefix("source=") {
                let kind = if value == "/dev/null" {
                    SourceRefKind::DirectiveDevNull
                } else {
                    SourceRefKind::Directive(value.to_string())
                };
                directives.insert(
                    comment.line,
                    SourceDirectiveOverride {
                        kind,
                        own_line: comment.is_own_line,
                    },
                );
            }
        }
    }
    directives
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
            WordPart::Literal(shuck_ast::LiteralText::Owned(text)) => Some(text.as_ref()),
            _ => None,
        },
        _ => None,
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

fn collect_pipeline_segments<'a>(stmt: &'a Stmt, out: &mut Vec<&'a Stmt>) {
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
    commands: &mut Vec<&'a Stmt>,
    operators: &mut Vec<RecordedListOperator>,
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
