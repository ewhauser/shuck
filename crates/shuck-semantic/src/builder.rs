use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticAssignOp, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArithmeticUnaryOp,
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, ListOperator, Name, ParameterOp, Script, Span, Word,
    WordPart, WordPartNode,
};
use shuck_indexer::Indexer;

use crate::binding::{Binding, BindingAttributes, BindingKind};
use crate::call_graph::{CallGraph, CallSite, OverwrittenFunction};
use crate::cfg::{
    FlowContext, IsolatedRegion, RecordedCaseArm, RecordedCommand, RecordedCommandKind,
    RecordedPipelineSegment, RecordedProgram,
};
use crate::declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
use crate::reference::{Reference, ReferenceKind};
use crate::runtime::RuntimePrelude;
use crate::source_ref::{SourceRef, SourceRefKind};
use crate::{
    BindingId, IndirectTargetHint, ReferenceId, Scope, ScopeId, ScopeKind, SourceDirectiveOverride,
    SpanKey, TraversalObserver,
};

pub(crate) struct BuildOutput {
    pub(crate) scopes: Vec<Scope>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) references: Vec<Reference>,
    pub(crate) predefined_runtime_refs: FxHashSet<ReferenceId>,
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
    deferred_functions: Vec<DeferredFunction<'a>>,
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
struct DeferredFunction<'a> {
    function: &'a FunctionDef,
    scope: ScopeId,
    flow: FlowState,
}

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(crate) fn build(
        script: &'a Script,
        source: &'a str,
        indexer: &'a Indexer,
        observer: &'observer mut dyn TraversalObserver,
        bash_runtime_vars_enabled: bool,
    ) -> BuildOutput {
        let file_scope = Scope {
            id: ScopeId(0),
            kind: ScopeKind::File,
            parent: None,
            span: script.span,
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
        let file_commands = builder.visit_commands(&script.commands, FlowState::default());
        builder.mark_scope_completed(ScopeId(0));
        builder.drain_deferred_functions();

        let call_graph = builder.build_call_graph();
        let heuristic_unused_assignments = builder.compute_heuristic_unused_assignments();

        BuildOutput {
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: builder.references,
            predefined_runtime_refs: builder.predefined_runtime_refs,
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

    fn visit_commands(&mut self, commands: &'a [Command], flow: FlowState) -> Vec<RecordedCommand> {
        commands
            .iter()
            .map(|command| self.visit_command(command, flow))
            .collect()
    }

    fn visit_command(&mut self, command: &'a Command, flow: FlowState) -> RecordedCommand {
        let span = command_span(command);
        let scope = self.current_scope();
        let context = Self::flow_context(flow);
        self.flow_contexts.push((span, context.clone()));
        self.observer.enter_command(command, scope, context);
        self.command_stack.push(span);

        let recorded = match command {
            Command::Simple(command) => self.visit_simple_command(command, flow),
            Command::Builtin(command) => self.visit_builtin(command, flow),
            Command::Decl(command) => self.visit_decl(command, flow),
            Command::Pipeline(command) => self.visit_pipeline(command, flow),
            Command::List(command) => self.visit_list(command, flow),
            Command::Compound(command, redirects) => self.visit_compound(command, redirects, flow),
            Command::Function(command) => self.visit_function(command, flow),
        };

        self.command_stack.pop();
        self.observer.exit_command(command, scope);
        recorded
    }

    fn visit_simple_command(
        &mut self,
        command: &'a shuck_ast::SimpleCommand,
        flow: FlowState,
    ) -> RecordedCommand {
        let mut nested_regions = Vec::new();
        let command_has_name = simple_command_has_name(command, self.source);
        for assignment in &command.assignments {
            nested_regions.extend(if command_has_name {
                self.visit_assignment_value(assignment, flow)
            } else {
                self.visit_assignment(assignment, None, BindingAttributes::empty(), flow)
            });
        }

        nested_regions.extend(self.visit_word(&command.name, WordVisitKind::Expansion, flow));
        nested_regions.extend(self.visit_words(&command.args, WordVisitKind::Expansion, flow));
        nested_regions.extend(self.visit_redirects(&command.redirects, flow));

        if let Some(name) = static_word_text(&command.name, self.source)
            && !name.is_empty()
        {
            let callee = Name::from(name.as_str());
            let scope = self.current_scope();
            self.call_sites
                .entry(callee.clone())
                .or_default()
                .push(CallSite {
                    callee: callee.clone(),
                    span: command.span,
                    scope,
                    arg_count: command.args.len(),
                });

            self.classify_special_simple_command(&callee, command, flow);
        }

        RecordedCommand {
            span: command.span,
            nested_regions,
            kind: RecordedCommandKind::Linear,
        }
    }

    fn visit_builtin(&mut self, command: &'a BuiltinCommand, flow: FlowState) -> RecordedCommand {
        match command {
            BuiltinCommand::Break(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_builtin_parts(
                    &command.assignments,
                    command.depth.as_ref(),
                    &command.extra_args,
                    &command.redirects,
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
                    &command.redirects,
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
                    &command.redirects,
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
                    &command.redirects,
                    flow,
                ),
                kind: RecordedCommandKind::Exit,
            },
        }
    }

    fn visit_builtin_parts(
        &mut self,
        assignments: &'a [Assignment],
        primary_word: Option<&'a Word>,
        extra_words: &'a [Word],
        redirects: &'a [shuck_ast::Redirect],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for assignment in assignments {
            nested_regions.extend(self.visit_assignment(
                assignment,
                None,
                BindingAttributes::empty(),
                flow,
            ));
        }
        if let Some(word) = primary_word {
            nested_regions.extend(self.visit_word(word, WordVisitKind::Expansion, flow));
        }
        nested_regions.extend(self.visit_words(extra_words, WordVisitKind::Expansion, flow));
        nested_regions.extend(self.visit_redirects(redirects, flow));
        nested_regions
    }

    fn visit_decl(
        &mut self,
        command: &'a shuck_ast::DeclClause,
        flow: FlowState,
    ) -> RecordedCommand {
        let mut nested_regions = Vec::new();
        for assignment in &command.assignments {
            nested_regions.extend(self.visit_assignment(
                assignment,
                None,
                BindingAttributes::empty(),
                flow,
            ));
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
                    nested_regions.extend(self.visit_word(word, WordVisitKind::Expansion, flow));
                }
                DeclOperand::Name(name) => {
                    nested_regions
                        .extend(self.visit_optional_arithmetic_expr(name.index_ast.as_ref(), flow));
                    self.visit_name_only_declaration_operand(
                        builtin,
                        &flags,
                        name.name.clone(),
                        name.span,
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
                    nested_regions.extend(self.visit_assignment(
                        assignment,
                        Some((kind, scope)),
                        attributes,
                        flow,
                    ));
                }
            }
        }

        nested_regions.extend(self.visit_redirects(&command.redirects, flow));

        RecordedCommand {
            span: command.span,
            nested_regions,
            kind: RecordedCommandKind::Linear,
        }
    }

    fn visit_pipeline(
        &mut self,
        pipeline: &'a shuck_ast::Pipeline,
        mut flow: FlowState,
    ) -> RecordedCommand {
        flow.in_subshell = true;
        let mut segments = Vec::with_capacity(pipeline.commands.len());
        for command in &pipeline.commands {
            let scope = self.push_scope(
                ScopeKind::Pipeline,
                self.current_scope(),
                command_span(command),
            );
            let recorded = self.visit_command(command, flow);
            self.pop_scope(scope);
            self.mark_scope_completed(scope);
            segments.push(RecordedPipelineSegment {
                scope,
                command: recorded,
            });
        }

        RecordedCommand {
            span: pipeline.span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::Pipeline { segments },
        }
    }

    fn visit_list(&mut self, list: &'a CommandList, flow: FlowState) -> RecordedCommand {
        let operators = list
            .rest
            .iter()
            .map(|item| item.operator)
            .collect::<Vec<_>>();
        let mut commands = Vec::with_capacity(list.rest.len() + 1);
        commands.push(list.first.as_ref());
        commands.extend(list.rest.iter().map(|item| &item.command));

        let mut recorded = Vec::with_capacity(commands.len());
        for (index, command) in commands.into_iter().enumerate() {
            let mut nested = flow;
            nested.exit_status_checked = matches!(
                operators.get(index).copied(),
                Some(ListOperator::And | ListOperator::Or)
            ) || flow.exit_status_checked;
            recorded.push(self.visit_command(command, nested));
        }

        let first = Box::new(recorded.remove(0));
        let rest = list
            .rest
            .iter()
            .map(|item| item.operator)
            .zip(recorded)
            .collect();

        RecordedCommand {
            span: list.span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::List { first, rest },
        }
    }

    fn visit_compound(
        &mut self,
        command: &'a CompoundCommand,
        redirects: &'a [shuck_ast::Redirect],
        flow: FlowState,
    ) -> RecordedCommand {
        match command {
            CompoundCommand::If(command) => RecordedCommand {
                span: command.span,
                nested_regions: self.visit_redirects(redirects, flow),
                kind: RecordedCommandKind::If {
                    condition: self.visit_commands(
                        &command.condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    ),
                    then_branch: self.visit_commands(&command.then_branch, flow),
                    elif_branches: command
                        .elif_branches
                        .iter()
                        .map(|(condition, body)| {
                            (
                                self.visit_commands(
                                    condition,
                                    FlowState {
                                        exit_status_checked: true,
                                        ..flow
                                    },
                                ),
                                self.visit_commands(body, flow),
                            )
                        })
                        .collect(),
                    else_branch: command
                        .else_branch
                        .as_deref()
                        .map(|body| self.visit_commands(body, flow))
                        .unwrap_or_default(),
                },
            },
            CompoundCommand::For(command) => {
                let mut nested_regions = command
                    .words
                    .as_deref()
                    .map(|words| self.visit_words(words, WordVisitKind::Expansion, flow))
                    .unwrap_or_default();
                nested_regions.extend(self.visit_redirects(redirects, flow));
                self.add_binding(
                    command.variable.clone(),
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingAttributes::empty(),
                );

                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::For {
                        body: self.visit_commands(
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
                let mut nested_regions =
                    self.visit_optional_arithmetic_expr(command.init_ast.as_ref(), flow);
                nested_regions.extend(
                    self.visit_optional_arithmetic_expr(command.condition_ast.as_ref(), flow),
                );
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(command.step_ast.as_ref(), flow));
                nested_regions.extend(self.visit_redirects(redirects, flow));
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::ArithmeticFor {
                        body: self.visit_commands(
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
                nested_regions: self.visit_redirects(redirects, flow),
                kind: RecordedCommandKind::While {
                    condition: self.visit_commands(
                        &command.condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    ),
                    body: self.visit_commands(
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
                nested_regions: self.visit_redirects(redirects, flow),
                kind: RecordedCommandKind::Until {
                    condition: self.visit_commands(
                        &command.condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    ),
                    body: self.visit_commands(
                        &command.body,
                        FlowState {
                            loop_depth: flow.loop_depth + 1,
                            ..flow
                        },
                    ),
                },
            },
            CompoundCommand::Case(command) => {
                let mut nested_regions =
                    self.visit_word(&command.word, WordVisitKind::Expansion, flow);
                nested_regions.extend(self.visit_redirects(redirects, flow));

                let arms = command
                    .cases
                    .iter()
                    .map(|case| {
                        let pattern_regions =
                            self.visit_words(&case.patterns, WordVisitKind::Conditional, flow);
                        let mut commands = self.visit_commands(&case.commands, flow);
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
                            terminator: case.terminator.clone(),
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
                let mut nested_regions =
                    self.visit_words(&command.words, WordVisitKind::Expansion, flow);
                nested_regions.extend(self.visit_redirects(redirects, flow));
                self.add_binding(
                    command.variable.clone(),
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingAttributes::empty(),
                );

                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Select {
                        body: self.visit_commands(
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
                let body = self.visit_commands(
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
                    nested_regions: self.visit_redirects(redirects, flow),
                    kind: RecordedCommandKind::Subshell { body },
                }
            }
            CompoundCommand::BraceGroup(commands) => RecordedCommand {
                span: command_span_from_compound(command),
                nested_regions: self.visit_redirects(redirects, flow),
                kind: RecordedCommandKind::BraceGroup {
                    body: self.visit_commands(
                        commands,
                        FlowState {
                            in_block: true,
                            ..flow
                        },
                    ),
                },
            },
            CompoundCommand::Arithmetic(command) => {
                let mut nested_regions =
                    self.visit_optional_arithmetic_expr(command.expr_ast.as_ref(), flow);
                nested_regions.extend(self.visit_redirects(redirects, flow));
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Linear,
                }
            }
            CompoundCommand::Time(command) => {
                let mut nested_regions = self.visit_redirects(redirects, flow);
                if let Some(command) = &command.command {
                    nested_regions.extend(Self::flatten_recorded_regions(
                        self.visit_command(command, flow),
                    ));
                }
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Linear,
                }
            }
            CompoundCommand::Conditional(command) => {
                let mut nested_regions = self.visit_conditional_expr(&command.expression, flow);
                nested_regions.extend(self.visit_redirects(redirects, flow));
                RecordedCommand {
                    span: command.span,
                    nested_regions,
                    kind: RecordedCommandKind::Linear,
                }
            }
            CompoundCommand::Coproc(command) => RecordedCommand {
                span: command.span,
                nested_regions: Self::flatten_recorded_regions(self.visit_command(
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

    fn visit_function(&mut self, function: &'a FunctionDef, flow: FlowState) -> RecordedCommand {
        self.add_binding(
            function.name.clone(),
            BindingKind::FunctionDefinition,
            self.current_scope(),
            function.name_span,
            BindingAttributes::empty(),
        );

        let scope = self.push_scope(
            ScopeKind::Function(function.name.clone()),
            self.current_scope(),
            body_span(&function.body),
        );
        self.deferred_functions.push(DeferredFunction {
            function,
            scope,
            flow,
        });
        self.pop_scope(scope);

        RecordedCommand {
            span: function.span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::Linear,
        }
    }

    fn visit_assignment(
        &mut self,
        assignment: &'a Assignment,
        declaration_kind: Option<(BindingKind, ScopeId)>,
        mut attributes: BindingAttributes,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions =
            self.visit_optional_arithmetic_expr(assignment.index_ast.as_ref(), flow);
        nested_regions.extend(self.visit_assignment_value(assignment, flow));
        let (kind, scope) = declaration_kind.unwrap_or_else(|| {
            let kind = if assignment.append {
                BindingKind::AppendAssignment
            } else if matches!(assignment.value, AssignmentValue::Array(_))
                || assignment.index.is_some()
            {
                BindingKind::ArrayAssignment
            } else {
                BindingKind::Assignment
            };
            (kind, self.current_scope())
        });
        if matches!(assignment.value, AssignmentValue::Array(_)) || assignment.index.is_some() {
            attributes |= BindingAttributes::ARRAY;
        }

        let binding = self.add_binding(
            assignment.name.clone(),
            kind,
            scope,
            assignment.name_span,
            attributes,
        );
        if let Some(hint) = indirect_target_hint(assignment, self.source) {
            self.indirect_target_hints.insert(binding, hint);
        }
        nested_regions
    }

    fn visit_assignment_value(
        &mut self,
        assignment: &'a Assignment,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                nested_regions.extend(self.visit_word(word, WordVisitKind::Expansion, flow));
            }
            AssignmentValue::Array(words) => {
                nested_regions.extend(self.visit_words(words, WordVisitKind::Expansion, flow));
            }
        }
        nested_regions
    }

    fn visit_words(
        &mut self,
        words: &'a [Word],
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for word in words {
            nested_regions.extend(self.visit_word(word, kind, flow));
        }
        nested_regions
    }

    fn visit_redirects(
        &mut self,
        redirects: &'a [shuck_ast::Redirect],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for redirect in redirects {
            let word = match redirect.word_target() {
                Some(word) => word,
                None => &redirect.heredoc().expect("expected heredoc redirect").body,
            };
            nested_regions.extend(self.visit_word(word, WordVisitKind::Expansion, flow));
        }
        nested_regions
    }

    fn visit_word(
        &mut self,
        word: &'a Word,
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_word_part_nodes(&word.parts, kind, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_word_part_nodes(
        &mut self,
        parts: &'a [WordPartNode],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for part in parts {
            self.visit_word_part(&part.kind, part.span, kind, flow, nested_regions);
        }
    }

    fn visit_word_part(
        &mut self,
        part: &'a WordPart,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                self.visit_word_part_nodes(parts, kind, flow, nested_regions);
            }
            WordPart::Variable(name) => {
                self.add_reference(
                    name.clone(),
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::Expansion
                    },
                    span,
                );
            }
            WordPart::CommandSubstitution { commands, .. }
            | WordPart::ProcessSubstitution { commands, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let commands = self.visit_commands(
                    commands,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
                nested_regions.push(IsolatedRegion { scope, commands });
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(expression_ast.as_ref(), flow));
            }
            WordPart::ParameterExpansion { name, operator, .. } => {
                self.add_reference(
                    name.clone(),
                    if matches!(operator, ParameterOp::Error) {
                        ReferenceKind::RequiredRead
                    } else if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    span,
                );
            }
            WordPart::Length(name) | WordPart::ArrayLength(name) => {
                self.add_reference(
                    name.clone(),
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::Length
                    },
                    span,
                );
            }
            WordPart::ArrayAccess {
                name, index_ast, ..
            } => {
                self.add_reference(
                    name.clone(),
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ArrayAccess
                    },
                    span,
                );
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(index_ast.as_ref(), flow));
            }
            WordPart::ArrayIndices(name) | WordPart::PrefixMatch(name) => {
                self.add_reference(
                    name.clone(),
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
                    name.clone(),
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
                name,
                offset_ast,
                length_ast,
                ..
            } => {
                self.add_reference(
                    name.clone(),
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    span,
                );
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(offset_ast.as_ref(), flow));
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(length_ast.as_ref(), flow));
            }
            WordPart::ArraySlice {
                name,
                offset_ast,
                length_ast,
                ..
            } => {
                self.add_reference(
                    name.clone(),
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    span,
                );
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(offset_ast.as_ref(), flow));
                nested_regions
                    .extend(self.visit_optional_arithmetic_expr(length_ast.as_ref(), flow));
            }
            WordPart::Transformation { name, .. } => {
                self.add_reference(
                    name.clone(),
                    if matches!(kind, WordVisitKind::Conditional) {
                        ReferenceKind::ConditionalOperand
                    } else {
                        ReferenceKind::ParameterExpansion
                    },
                    span,
                );
            }
        }
    }

    fn visit_conditional_expr(
        &mut self,
        expression: &'a ConditionalExpr,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        match expression {
            ConditionalExpr::Binary(expr) => {
                let mut nested_regions = self.visit_conditional_expr(&expr.left, flow);
                nested_regions.extend(self.visit_conditional_expr(&expr.right, flow));
                nested_regions
            }
            ConditionalExpr::Unary(expr) => self.visit_conditional_expr(&expr.expr, flow),
            ConditionalExpr::Parenthesized(expr) => self.visit_conditional_expr(&expr.expr, flow),
            ConditionalExpr::Word(word)
            | ConditionalExpr::Pattern(word)
            | ConditionalExpr::Regex(word) => {
                self.visit_word(word, WordVisitKind::Conditional, flow)
            }
        }
    }

    fn visit_optional_arithmetic_expr(
        &mut self,
        expr: Option<&'a ArithmeticExprNode>,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        expr.map(|expr| self.visit_arithmetic_expr(expr, flow))
            .unwrap_or_default()
    }

    fn visit_arithmetic_expr(
        &mut self,
        expr: &'a ArithmeticExprNode,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        match &expr.kind {
            ArithmeticExpr::Number(_) => Vec::new(),
            ArithmeticExpr::Variable(name) => {
                self.add_reference(name.clone(), ReferenceKind::ArithmeticRead, expr.span);
                Vec::new()
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.add_reference(
                    name.clone(),
                    ReferenceKind::ArithmeticRead,
                    arithmetic_name_span(expr.span, name),
                );
                self.visit_arithmetic_expr(index, flow)
            }
            ArithmeticExpr::ShellWord(word) => {
                self.visit_word(word, WordVisitKind::Expansion, flow)
            }
            ArithmeticExpr::Parenthesized { expression } => {
                self.visit_arithmetic_expr(expression, flow)
            }
            ArithmeticExpr::Unary { op, expr: inner } => {
                if matches!(
                    op,
                    ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
                ) {
                    self.visit_arithmetic_update(inner, flow)
                } else {
                    self.visit_arithmetic_expr(inner, flow)
                }
            }
            ArithmeticExpr::Postfix { expr: inner, .. } => {
                self.visit_arithmetic_update(inner, flow)
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                let mut nested_regions = self.visit_arithmetic_expr(left, flow);
                nested_regions.extend(self.visit_arithmetic_expr(right, flow));
                nested_regions
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let mut nested_regions = self.visit_arithmetic_expr(condition, flow);
                nested_regions.extend(self.visit_arithmetic_expr(then_expr, flow));
                nested_regions.extend(self.visit_arithmetic_expr(else_expr, flow));
                nested_regions
            }
            ArithmeticExpr::Assignment { target, op, value } => {
                self.visit_arithmetic_assignment(target, expr.span, *op, value, flow)
            }
        }
    }

    fn visit_arithmetic_update(
        &mut self,
        expr: &'a ArithmeticExprNode,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        match &expr.kind {
            ArithmeticExpr::Variable(name) => {
                self.add_reference(name.clone(), ReferenceKind::ArithmeticRead, expr.span);
                self.add_binding(
                    name.clone(),
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    expr.span,
                    BindingAttributes::empty(),
                );
                Vec::new()
            }
            ArithmeticExpr::Indexed { name, index } => {
                let nested_regions = self.visit_arithmetic_expr(index, flow);
                let span = arithmetic_name_span(expr.span, name);
                self.add_reference(name.clone(), ReferenceKind::ArithmeticRead, span);
                self.add_binding(
                    name.clone(),
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    span,
                    BindingAttributes::ARRAY,
                );
                nested_regions
            }
            _ => Vec::new(),
        }
    }

    fn visit_arithmetic_assignment(
        &mut self,
        target: &'a ArithmeticLvalue,
        target_span: Span,
        op: ArithmeticAssignOp,
        value: &'a ArithmeticExprNode,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = self.visit_arithmetic_lvalue_indices(target, flow);
        let (name, attributes) = match target {
            ArithmeticLvalue::Variable(name) => (name, BindingAttributes::empty()),
            ArithmeticLvalue::Indexed { name, .. } => (name, BindingAttributes::ARRAY),
        };
        let name_span = arithmetic_name_span(target_span, name);
        if !matches!(op, ArithmeticAssignOp::Assign) {
            self.add_reference(name.clone(), ReferenceKind::ArithmeticRead, name_span);
        }
        nested_regions.extend(self.visit_arithmetic_expr(value, flow));
        self.add_binding(
            name.clone(),
            BindingKind::ArithmeticAssignment,
            self.current_scope(),
            name_span,
            attributes,
        );
        nested_regions
    }

    fn visit_arithmetic_lvalue_indices(
        &mut self,
        target: &'a ArithmeticLvalue,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        match target {
            ArithmeticLvalue::Variable(_) => Vec::new(),
            ArithmeticLvalue::Indexed { index, .. } => self.visit_arithmetic_expr(index, flow),
        }
    }

    fn classify_special_simple_command(
        &mut self,
        name: &Name,
        command: &'a shuck_ast::SimpleCommand,
        _flow: FlowState,
    ) {
        match name.as_str() {
            "read" => {
                for (argument, span) in iter_read_targets(&command.args, self.source) {
                    self.add_binding(
                        argument,
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
                    self.add_reference_if_bound(
                        Name::from(*implicit_read),
                        ReferenceKind::ImplicitRead,
                        command.span,
                    );
                }
            }
            "mapfile" | "readarray" => {
                if let Some((argument, span)) = explicit_mapfile_target(&command.args, self.source)
                {
                    self.add_binding(
                        argument,
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
                        argument,
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
                        argument,
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
        name: Name,
        span: Span,
    ) {
        let (scope, attributes) = self.declaration_scope_and_attributes(builtin, flags);
        let local_like = attributes.contains(BindingAttributes::LOCAL);
        let existing = self.resolve_reference(&name, scope, span.start.offset);

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
        name: Name,
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
        self.binding_index.entry(name.clone()).or_default().push(id);
        self.scopes[scope.index()]
            .bindings
            .entry(name.clone())
            .or_default()
            .push(id);
        if matches!(kind, BindingKind::FunctionDefinition) {
            self.functions.entry(name).or_default().push(id);
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

    fn add_reference(&mut self, name: Name, kind: ReferenceKind, span: Span) -> ReferenceId {
        let id = ReferenceId(self.references.len() as u32);
        let scope = self.current_scope();
        let resolved = self.resolve_reference(&name, scope, span.start.offset);
        let predefined_runtime = resolved.is_none() && self.runtime.is_preinitialized(&name);

        self.references.push(Reference {
            id,
            name,
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

    fn add_reference_if_bound(&mut self, name: Name, kind: ReferenceKind, span: Span) {
        if self
            .resolve_reference(&name, self.current_scope(), span.start.offset)
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
            if !reachable.insert(name.clone()) {
                continue;
            }
            for sites in self.call_sites.values() {
                for site in sites {
                    if is_in_named_function_scope(&self.scopes, site.scope, &name) {
                        worklist.push(site.callee.clone());
                    }
                }
            }
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
                let commands = self.visit_function_body(deferred.function, deferred.flow);
                self.recorded_function_bodies
                    .insert(deferred.scope, commands);
                self.mark_scope_completed(deferred.scope);
            }
        }
        self.rebuild_scope_stack(ScopeId(0));
        self.command_stack.clear();
    }

    fn visit_function_body(
        &mut self,
        function: &'a FunctionDef,
        flow: FlowState,
    ) -> Vec<RecordedCommand> {
        let flow = FlowState {
            in_function: true,
            ..flow
        };

        match function.body.as_ref() {
            Command::Compound(CompoundCommand::BraceGroup(commands), _) => {
                self.visit_commands(commands, flow)
            }
            body => vec![self.visit_command(body, flow)],
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
                name: assignment.name.clone(),
                name_span: assignment.name_span,
                value_span: assignment_value_span(assignment),
                append: assignment.append,
            },
            DeclOperand::Dynamic(word) => DeclarationOperand::DynamicWord { span: word.span },
        })
        .collect()
}

fn assignment_value_span(assignment: &Assignment) -> Span {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word.span,
        AssignmentValue::Array(words) => words
            .first()
            .map(|word| word.span)
            .zip(words.last().map(|word| word.span))
            .map(|(start, end)| start.merge(end))
            .unwrap_or(assignment.span),
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
            _ => return false,
        }
    }

    true
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
    ancestor_scopes(scopes, scope)
        .any(|scope| matches!(&scopes[scope.index()].kind, ScopeKind::Function(function) if function == name))
}

fn body_span(command: &Command) -> Span {
    match command {
        Command::Compound(CompoundCommand::BraceGroup(commands), _) => commands
            .first()
            .map(command_span)
            .zip(commands.last().map(command_span))
            .map(|(start, end)| start.merge(end))
            .unwrap_or(command_span(command)),
        _ => command_span(command),
    }
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(BuiltinCommand::Break(command)) => command.span,
        Command::Builtin(BuiltinCommand::Continue(command)) => command.span,
        Command::Builtin(BuiltinCommand::Return(command)) => command.span,
        Command::Builtin(BuiltinCommand::Exit(command)) => command.span,
        Command::Decl(command) => command.span,
        Command::Pipeline(command) => command.span,
        Command::List(command) => command.span,
        Command::Compound(command, _) => command_span_from_compound(command),
        Command::Function(command) => command.span,
    }
}

fn command_span_from_compound(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .first()
            .map(command_span)
            .zip(commands.last().map(command_span))
            .map(|(start, end)| start.merge(end))
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
    }
}
