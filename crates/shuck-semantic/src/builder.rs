use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, ListOperator, Name, ParameterOp, Script, SourceText,
    Span, Word, WordPart,
};
use shuck_indexer::Indexer;

use crate::binding::{Binding, BindingAttributes, BindingKind};
use crate::call_graph::{CallGraph, CallSite, OverwrittenFunction};
use crate::cfg::FlowContext;
use crate::declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
use crate::reference::{Reference, ReferenceKind};
use crate::source_ref::{SourceRef, SourceRefKind};
use crate::{BindingId, ReferenceId, Scope, ScopeId, ScopeKind, SourceDirectiveOverride, SpanKey};

pub(crate) struct BuildOutput {
    pub(crate) scopes: Vec<Scope>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) references: Vec<Reference>,
    pub(crate) binding_index: FxHashMap<Name, Vec<BindingId>>,
    pub(crate) resolved: FxHashMap<ReferenceId, BindingId>,
    pub(crate) unresolved: Vec<ReferenceId>,
    pub(crate) functions: FxHashMap<Name, Vec<BindingId>>,
    pub(crate) call_sites: FxHashMap<Name, Vec<CallSite>>,
    pub(crate) call_graph: CallGraph,
    pub(crate) source_refs: Vec<SourceRef>,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) flow_contexts: Vec<(Span, FlowContext)>,
    pub(crate) command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    pub(crate) command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    pub(crate) heuristic_unused_assignments: Vec<BindingId>,
}

pub(crate) struct SemanticModelBuilder<'a> {
    source: &'a str,
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    binding_index: FxHashMap<Name, Vec<BindingId>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, Vec<BindingId>>,
    call_sites: FxHashMap<Name, Vec<CallSite>>,
    source_refs: Vec<SourceRef>,
    declarations: Vec<Declaration>,
    flow_contexts: Vec<(Span, FlowContext)>,
    command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    source_directives: FxHashMap<usize, SourceDirectiveOverride>,
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

#[derive(Debug, Clone)]
struct ArithmeticEvent {
    name: Name,
    span: Span,
    read: bool,
    write: bool,
}

impl<'a> SemanticModelBuilder<'a> {
    pub(crate) fn build(script: &Script, source: &'a str, indexer: &'a Indexer) -> BuildOutput {
        let file_scope = Scope {
            id: ScopeId(0),
            kind: ScopeKind::File,
            parent: None,
            span: script.span,
            bindings: FxHashMap::default(),
        };
        let mut builder = Self {
            source,
            scopes: vec![file_scope],
            bindings: Vec::new(),
            references: Vec::new(),
            binding_index: FxHashMap::default(),
            resolved: FxHashMap::default(),
            unresolved: Vec::new(),
            functions: FxHashMap::default(),
            call_sites: FxHashMap::default(),
            source_refs: Vec::new(),
            declarations: Vec::new(),
            flow_contexts: Vec::new(),
            command_bindings: FxHashMap::default(),
            command_references: FxHashMap::default(),
            source_directives: parse_source_directives(source, indexer),
            scope_stack: vec![ScopeId(0)],
            command_stack: Vec::new(),
        };
        builder.visit_commands(&script.commands, FlowState::default());
        builder.resolve_references();
        let call_graph = builder.build_call_graph();
        let heuristic_unused_assignments = builder.compute_heuristic_unused_assignments();

        BuildOutput {
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: builder.references,
            binding_index: builder.binding_index,
            resolved: builder.resolved,
            unresolved: builder.unresolved,
            functions: builder.functions,
            call_sites: builder.call_sites,
            call_graph,
            source_refs: builder.source_refs,
            declarations: builder.declarations,
            flow_contexts: builder.flow_contexts,
            command_bindings: builder.command_bindings,
            command_references: builder.command_references,
            heuristic_unused_assignments,
        }
    }

    fn visit_commands(&mut self, commands: &[Command], flow: FlowState) {
        for command in commands {
            self.visit_command(command, flow);
        }
    }

    fn visit_command(&mut self, command: &Command, flow: FlowState) {
        let span = command_span(command);
        self.flow_contexts.push((
            span,
            FlowContext {
                in_function: flow.in_function,
                loop_depth: flow.loop_depth,
                in_subshell: flow.in_subshell,
                in_block: flow.in_block,
                exit_status_checked: flow.exit_status_checked,
            },
        ));
        self.command_stack.push(span);

        match command {
            Command::Simple(command) => self.visit_simple_command(command, flow),
            Command::Builtin(command) => self.visit_builtin(command, flow),
            Command::Decl(command) => self.visit_decl(command, flow),
            Command::Pipeline(command) => self.visit_pipeline(command, flow),
            Command::List(command) => self.visit_list(command, flow),
            Command::Compound(command, redirects) => self.visit_compound(command, redirects, flow),
            Command::Function(command) => self.visit_function(command, flow),
        }

        self.command_stack.pop();
    }

    fn visit_simple_command(&mut self, command: &shuck_ast::SimpleCommand, flow: FlowState) {
        for assignment in &command.assignments {
            self.visit_assignment(assignment, None, BindingAttributes::empty());
        }

        self.visit_word(&command.name, WordVisitKind::Expansion, flow);
        for argument in &command.args {
            self.visit_word(argument, WordVisitKind::Expansion, flow);
        }
        for redirect in &command.redirects {
            self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
        }

        if let Some(name) = static_word_text(&command.name, self.source) {
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
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand, flow: FlowState) {
        match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment, None, BindingAttributes::empty());
                }
                if let Some(depth) = &command.depth {
                    self.visit_word(depth, WordVisitKind::Expansion, flow);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, WordVisitKind::Expansion, flow);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment, None, BindingAttributes::empty());
                }
                if let Some(depth) = &command.depth {
                    self.visit_word(depth, WordVisitKind::Expansion, flow);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, WordVisitKind::Expansion, flow);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment, None, BindingAttributes::empty());
                }
                if let Some(code) = &command.code {
                    self.visit_word(code, WordVisitKind::Expansion, flow);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, WordVisitKind::Expansion, flow);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment, None, BindingAttributes::empty());
                }
                if let Some(code) = &command.code {
                    self.visit_word(code, WordVisitKind::Expansion, flow);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument, WordVisitKind::Expansion, flow);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
                }
            }
        }
    }

    fn visit_decl(&mut self, command: &shuck_ast::DeclClause, flow: FlowState) {
        for assignment in &command.assignments {
            self.visit_assignment(assignment, None, BindingAttributes::empty());
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
                DeclOperand::Flag(word) => self.visit_word(word, WordVisitKind::Expansion, flow),
                DeclOperand::Dynamic(word) => self.visit_word(word, WordVisitKind::Expansion, flow),
                DeclOperand::Name(name) => {
                    let (scope, attributes) =
                        self.declaration_scope_and_attributes(builtin, &flags);
                    let kind = if attributes.contains(BindingAttributes::NAMEREF) {
                        BindingKind::Nameref
                    } else {
                        BindingKind::Declaration(builtin)
                    };
                    self.add_binding(name.name.clone(), kind, scope, name.span, attributes);
                }
                DeclOperand::Assignment(assignment) => {
                    let (scope, attributes) =
                        self.declaration_scope_and_attributes(builtin, &flags);
                    let kind = if attributes.contains(BindingAttributes::NAMEREF) {
                        BindingKind::Nameref
                    } else {
                        BindingKind::Declaration(builtin)
                    };
                    self.visit_assignment(assignment, Some((kind, scope)), attributes);
                }
            }
        }

        for redirect in &command.redirects {
            self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
        }
    }

    fn visit_pipeline(&mut self, pipeline: &shuck_ast::Pipeline, mut flow: FlowState) {
        flow.in_subshell = true;
        for command in &pipeline.commands {
            let scope = self.push_scope(
                ScopeKind::Pipeline,
                self.current_scope(),
                command_span(command),
            );
            self.visit_command(command, flow);
            self.pop_scope(scope);
        }
    }

    fn visit_list(&mut self, list: &CommandList, flow: FlowState) {
        let operators = list.rest.iter().map(|(op, _)| *op).collect::<Vec<_>>();
        let mut commands = Vec::with_capacity(list.rest.len() + 1);
        commands.push(list.first.as_ref());
        commands.extend(list.rest.iter().map(|(_, command)| command));

        for (index, command) in commands.into_iter().enumerate() {
            let mut nested = flow;
            nested.exit_status_checked = matches!(
                operators.get(index).copied(),
                Some(ListOperator::And | ListOperator::Or)
            ) || flow.exit_status_checked;
            self.visit_command(command, nested);
        }
    }

    fn visit_compound(
        &mut self,
        command: &CompoundCommand,
        redirects: &[shuck_ast::Redirect],
        flow: FlowState,
    ) {
        match command {
            CompoundCommand::If(command) => {
                self.visit_commands(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                self.visit_commands(&command.then_branch, flow);
                for (condition, body) in &command.elif_branches {
                    self.visit_commands(
                        condition,
                        FlowState {
                            exit_status_checked: true,
                            ..flow
                        },
                    );
                    self.visit_commands(body, flow);
                }
                if let Some(body) = &command.else_branch {
                    self.visit_commands(body, flow);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        self.visit_word(word, WordVisitKind::Expansion, flow);
                    }
                }
                self.add_binding(
                    command.variable.clone(),
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingAttributes::empty(),
                );
                self.visit_commands(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow
                    },
                );
            }
            CompoundCommand::ArithmeticFor(command) => {
                if let Some(span) = command.init_span {
                    self.visit_arithmetic_span(span);
                }
                if let Some(span) = command.condition_span {
                    self.visit_arithmetic_span(span);
                }
                if let Some(span) = command.step_span {
                    self.visit_arithmetic_span(span);
                }
                self.visit_commands(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow
                    },
                );
            }
            CompoundCommand::While(command) => {
                self.visit_commands(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                self.visit_commands(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow
                    },
                );
            }
            CompoundCommand::Until(command) => {
                self.visit_commands(
                    &command.condition,
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                self.visit_commands(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow
                    },
                );
            }
            CompoundCommand::Case(command) => {
                self.visit_word(&command.word, WordVisitKind::Expansion, flow);
                for case in &command.cases {
                    for pattern in &case.patterns {
                        self.visit_word(pattern, WordVisitKind::Conditional, flow);
                    }
                    self.visit_commands(&case.commands, flow);
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    self.visit_word(word, WordVisitKind::Expansion, flow);
                }
                self.add_binding(
                    command.variable.clone(),
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    command.variable_span,
                    BindingAttributes::empty(),
                );
                self.visit_commands(
                    &command.body,
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow
                    },
                );
            }
            CompoundCommand::Subshell(commands) => {
                let scope = self.push_scope(
                    ScopeKind::Subshell,
                    self.current_scope(),
                    command_span_from_compound(command),
                );
                self.visit_commands(
                    commands,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
                self.pop_scope(scope);
            }
            CompoundCommand::BraceGroup(commands) => {
                self.visit_commands(
                    commands,
                    FlowState {
                        in_block: true,
                        ..flow
                    },
                );
            }
            CompoundCommand::Arithmetic(command) => {
                if let Some(span) = command.expr_span {
                    self.visit_arithmetic_span(span);
                }
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.visit_command(command, flow);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.visit_conditional_expr(&command.expression, flow);
            }
            CompoundCommand::Coproc(command) => {
                self.visit_command(
                    &command.body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
            }
        }

        for redirect in redirects {
            self.visit_word(&redirect.target, WordVisitKind::Expansion, flow);
        }
    }

    fn visit_function(&mut self, function: &FunctionDef, flow: FlowState) {
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
        self.visit_command(
            &function.body,
            FlowState {
                in_function: true,
                ..flow
            },
        );
        self.pop_scope(scope);
    }

    fn visit_assignment(
        &mut self,
        assignment: &Assignment,
        declaration_kind: Option<(BindingKind, ScopeId)>,
        attributes: BindingAttributes,
    ) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.visit_word(word, WordVisitKind::Expansion, FlowState::default())
            }
            AssignmentValue::Array(words) => {
                for word in words {
                    self.visit_word(word, WordVisitKind::Expansion, FlowState::default());
                }
            }
        }

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

        self.add_binding(
            assignment.name.clone(),
            kind,
            scope,
            assignment.span,
            attributes,
        );
    }

    fn visit_word(&mut self, word: &Word, kind: WordVisitKind, flow: FlowState) {
        for (part, span) in word.parts_with_spans() {
            match part {
                WordPart::Literal(_) => {}
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
                WordPart::CommandSubstitution(commands)
                | WordPart::ProcessSubstitution { commands, .. } => {
                    let scope =
                        self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                    self.visit_commands(
                        commands,
                        FlowState {
                            in_subshell: true,
                            ..flow
                        },
                    );
                    self.pop_scope(scope);
                }
                WordPart::ArithmeticExpansion(text) => self.visit_arithmetic_source_text(text),
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
                WordPart::ArrayAccess { name, .. } => {
                    self.add_reference(
                        name.clone(),
                        if matches!(kind, WordVisitKind::Conditional) {
                            ReferenceKind::ConditionalOperand
                        } else {
                            ReferenceKind::ArrayAccess
                        },
                        span,
                    );
                }
                WordPart::ArrayIndices(name)
                | WordPart::IndirectExpansion { name, .. }
                | WordPart::PrefixMatch(name) => {
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
                WordPart::Substring { name, .. }
                | WordPart::ArraySlice { name, .. }
                | WordPart::Transformation { name, .. } => {
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
    }

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr, flow: FlowState) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.visit_conditional_expr(&expr.left, flow);
                self.visit_conditional_expr(&expr.right, flow);
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

    fn visit_arithmetic_span(&mut self, span: Span) {
        let text = span.slice(self.source);
        self.visit_arithmetic_text(text, span);
    }

    fn visit_arithmetic_source_text(&mut self, text: &SourceText) {
        let source = text.slice(self.source);
        self.visit_arithmetic_text(source, text.span());
    }

    fn visit_arithmetic_text(&mut self, text: &str, base: Span) {
        for event in scan_arithmetic(text, base) {
            if event.read {
                self.add_reference(
                    event.name.clone(),
                    ReferenceKind::ArithmeticRead,
                    event.span,
                );
            }
            if event.write {
                self.add_binding(
                    event.name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    event.span,
                    BindingAttributes::empty(),
                );
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
                for argument in iter_read_targets(&command.args, self.source) {
                    self.add_binding(
                        argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        command.span,
                        BindingAttributes::empty(),
                    );
                }
            }
            "mapfile" | "readarray" => {
                if let Some(argument) = explicit_mapfile_target(&command.args, self.source) {
                    self.add_binding(
                        argument,
                        BindingKind::MapfileTarget,
                        self.current_scope(),
                        command.span,
                        BindingAttributes::empty(),
                    );
                }
            }
            "printf" => {
                if let Some(argument) = printf_v_target(&command.args, self.source) {
                    self.add_binding(
                        argument,
                        BindingKind::PrintfTarget,
                        self.current_scope(),
                        command.span,
                        BindingAttributes::empty(),
                    );
                }
            }
            "getopts" => {
                if let Some(argument) = getopts_target(&command.args, self.source) {
                    self.add_binding(
                        argument,
                        BindingKind::GetoptsTarget,
                        self.current_scope(),
                        command.span,
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
        id
    }

    fn add_reference(&mut self, name: Name, kind: ReferenceKind, span: Span) -> ReferenceId {
        let id = ReferenceId(self.references.len() as u32);
        self.references.push(Reference {
            id,
            name,
            kind,
            scope: self.current_scope(),
            span,
        });
        if let Some(command) = self.command_stack.last().copied() {
            self.command_references
                .entry(SpanKey::new(command))
                .or_default()
                .push(id);
        }
        id
    }

    fn resolve_references(&mut self) {
        for index in 0..self.references.len() {
            let reference = self.references[index].clone();
            let mut resolved = None;
            for scope in ancestor_scopes(&self.scopes, reference.scope) {
                if let Some(bindings) = self.scopes[scope.index()].bindings.get(&reference.name) {
                    for binding in bindings.iter().rev().copied() {
                        if self.bindings[binding.index()].span.start.offset
                            <= reference.span.start.offset
                        {
                            resolved = Some(binding);
                            break;
                        }
                    }
                }
                if resolved.is_some() {
                    break;
                }
            }

            if let Some(binding) = resolved {
                self.resolved.insert(reference.id, binding);
                self.bindings[binding.index()].references.push(reference.id);
            } else {
                self.unresolved.push(reference.id);
            }
        }
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

fn iter_read_targets<'a>(args: &'a [Word], source: &'a str) -> impl Iterator<Item = Name> + 'a {
    args.iter()
        .filter_map(move |word| static_word_text(word, source))
        .filter(|text| !text.starts_with('-'))
        .filter(|text| is_name(text))
        .map(Name::from)
}

fn explicit_mapfile_target(args: &[Word], source: &str) -> Option<Name> {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .filter(|text| !text.starts_with('-'))
        .find(|text| is_name(text))
        .map(Name::from)
}

fn printf_v_target(args: &[Word], source: &str) -> Option<Name> {
    args.windows(2).find_map(|window| {
        (static_word_text(&window[0], source).as_deref() == Some("-v"))
            .then(|| static_word_text(&window[1], source))
            .flatten()
            .filter(|text| is_name(text))
            .map(Name::from)
    })
}

fn getopts_target(args: &[Word], source: &str) -> Option<Name> {
    args.get(1)
        .and_then(|word| static_word_text(word, source))
        .filter(|text| is_name(text))
        .map(Name::from)
}

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
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

fn scan_arithmetic(text: &str, base: Span) -> Vec<ArithmeticEvent> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut events = Vec::new();

    while index < bytes.len() {
        let byte = bytes[index];
        if is_ident_start(byte) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_ident_continue(bytes[index]) {
                index += 1;
            }
            let name = &text[start..index];
            let before = prev_non_whitespace(text, start);
            let after = next_non_whitespace(text, index);
            let (read, write) = classify_arithmetic_usage(before, after);
            let span = relative_span(base, text, start, index);
            events.push(ArithmeticEvent {
                name: Name::from(name),
                span,
                read,
                write,
            });
        } else {
            index += 1;
        }
    }

    events
}

fn classify_arithmetic_usage(before: &str, after: &str) -> (bool, bool) {
    if before.ends_with("++") || before.ends_with("--") {
        return (true, true);
    }
    if after.starts_with("++") || after.starts_with("--") {
        return (true, true);
    }
    if after.starts_with("+=")
        || after.starts_with("-=")
        || after.starts_with("*=")
        || after.starts_with("/=")
        || after.starts_with("%=")
        || after.starts_with("&=")
        || after.starts_with("|=")
        || after.starts_with("^=")
        || after.starts_with("<<=")
        || after.starts_with(">>=")
    {
        return (true, true);
    }
    if after.starts_with('=') && !after.starts_with("==") {
        return (false, true);
    }
    (true, false)
}

fn prev_non_whitespace(text: &str, index: usize) -> &str {
    let mut cursor = index;
    while cursor > 0 && text.as_bytes()[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    &text[..cursor]
}

fn next_non_whitespace(text: &str, index: usize) -> &str {
    let mut cursor = index;
    while cursor < text.len() && text.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    &text[cursor..]
}

fn relative_span(base: Span, source: &str, start: usize, end: usize) -> Span {
    let start_position = base.start.advanced_by(&source[..start]);
    let end_position = base.start.advanced_by(&source[..end]);
    Span::from_positions(start_position, end_position)
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
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
