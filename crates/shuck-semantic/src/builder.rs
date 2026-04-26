use std::{borrow::Cow, collections::BTreeMap};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    AnonymousFunctionCommand, ArenaFile, ArenaFileCommandKind, ArenaHeredocBodyPart,
    ArithmeticAssignOp, ArithmeticExpr, ArithmeticExprArena, ArithmeticExprArenaNode,
    ArithmeticExprNode, ArithmeticLvalue, ArithmeticLvalueArena, ArithmeticUnaryOp, ArrayElem,
    ArrayExpr, ArrayKind, Assignment, AssignmentNode, AssignmentValue, AssignmentValueNode,
    AstStore, BinaryCommand, BinaryOp, BourneParameterExpansion, BourneParameterExpansionNode,
    BuiltinCommand, BuiltinCommandNodeKind, Command, CommandView, CompoundCommand,
    CompoundCommandNode, ConditionalBinaryOp, ConditionalExpr, ConditionalExprArena,
    ConditionalUnaryOp, DeclOperand, DeclOperandNode, FunctionDef, HeredocBody, HeredocBodyPart,
    HeredocBodyPartNode, LiteralText, Name, NormalizedCommand, ParameterExpansion,
    ParameterExpansionNode, ParameterExpansionSyntax, ParameterExpansionSyntaxNode, ParameterOp,
    Pattern, PatternGroupKind, PatternNode, PatternPart, PatternPartArena, PatternPartNode,
    Position, RedirectNode, RedirectTargetNode, SourceText, Span, StaticCommandWrapperTarget, Stmt,
    StmtSeq, StmtSeqId, StmtSeqView, StmtView, Subscript, SubscriptNode, VarRef, VarRefNode, Word,
    WordId, WordPart, WordPartArena, WordPartArenaNode, WordPartNode, WrapperKind,
    ZshExpansionOperation, ZshExpansionOperationNode, ZshExpansionTarget, ZshExpansionTargetNode,
    ZshGlobSegment, normalize_command_words, static_command_name_text,
    static_command_wrapper_target_index, static_word_text, try_static_word_parts_text,
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
use crate::source_closure::{SourcePathTemplate, TemplatePart};
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
    pub(crate) parameter_guard_flow_refs: FxHashSet<ReferenceId>,
    pub(crate) defaulting_parameter_operand_refs: FxHashSet<ReferenceId>,
    pub(crate) self_referential_assignment_refs: FxHashSet<ReferenceId>,
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
    pub(crate) cleared_variables: FxHashMap<(ScopeId, Name), SmallVec<[usize; 2]>>,
    pub(crate) heuristic_unused_assignments: Vec<BindingId>,
}

pub(crate) struct SemanticModelBuilder<'src, 'ast, 'observer> {
    source: &'src str,
    arena_store: Option<&'ast AstStore>,
    line_start_offsets: Vec<usize>,
    shell_profile: ShellProfile,
    observer: &'observer mut dyn TraversalObserver,
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    reference_index: FxHashMap<Name, SmallVec<[ReferenceId; 2]>>,
    predefined_runtime_refs: FxHashSet<ReferenceId>,
    guarded_parameter_refs: FxHashSet<ReferenceId>,
    parameter_guard_flow_refs: FxHashSet<ReferenceId>,
    defaulting_parameter_operand_refs: FxHashSet<ReferenceId>,
    self_referential_assignment_refs: FxHashSet<ReferenceId>,
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
    source_directives: BTreeMap<usize, SourceDirectiveOverride>,
    cleared_variables: FxHashMap<(ScopeId, Name), SmallVec<[usize; 2]>>,
    runtime: RuntimePrelude,
    completed_scopes: FxHashSet<ScopeId>,
    deferred_functions: Vec<DeferredFunction>,
    scope_stack: Vec<ScopeId>,
    command_stack: Vec<Span>,
    guarded_parameter_operand_depth: u32,
    defaulting_parameter_operand_depth: u32,
    short_circuit_condition_depth: u32,
    arithmetic_reference_kind: ReferenceKind,
    word_reference_kind_override: Option<ReferenceKind>,
}

fn semantic_statement_span(stmt: &Stmt) -> Span {
    let mut end = stmt
        .terminator_span
        .filter(|terminator| terminator.end.offset == stmt.span.end.offset)
        .map_or(stmt.span.end, |terminator| terminator.start);

    for redirect in stmt.redirects.iter() {
        if redirect.span.end.offset > end.offset {
            end = redirect.span.end;
        }
    }

    Span::from_positions(stmt.span.start, end)
}

fn semantic_statement_span_arena(stmt: StmtView<'_>) -> Span {
    let mut end = stmt
        .terminator_span()
        .filter(|terminator| terminator.end.offset == stmt.span().end.offset)
        .map_or(stmt.span().end, |terminator| terminator.start);

    for redirect in stmt.redirects() {
        if redirect.span.end.offset > end.offset {
            end = redirect.span.end;
        }
    }

    Span::from_positions(stmt.span().start, end)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FlowState {
    in_function: bool,
    loop_depth: u32,
    in_subshell: bool,
    in_block: bool,
    exit_status_checked: bool,
    conditionally_executed: bool,
}

impl FlowState {
    fn conditional(self) -> Self {
        Self {
            conditionally_executed: true,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordVisitKind {
    Expansion,
    Conditional,
    ParameterPattern,
}

#[derive(Debug, Clone)]
struct ArenaNormalizedCommand<'a> {
    literal_name: Option<Cow<'a, str>>,
    effective_name: Option<Cow<'a, str>>,
    wrappers: Vec<WrapperKind>,
    body_word_span: Option<Span>,
    body_words: Vec<WordId>,
    command_span: Span,
}

impl ArenaNormalizedCommand<'_> {
    fn body_word_span(&self) -> Option<Span> {
        self.body_word_span
    }

    fn body_args(&self) -> &[WordId] {
        self.body_words.split_first().map_or(&[], |(_, rest)| rest)
    }
}

#[derive(Debug, Clone)]
struct DeferredFunction {
    body: DeferredFunctionBody,
    scope: ScopeId,
    flow: FlowState,
}

#[derive(Debug, Clone)]
enum DeferredFunctionBody {
    Recursive(FunctionDef),
    Arena(StmtSeqId),
}

impl<'src, 'ast, 'observer> SemanticModelBuilder<'src, 'ast, 'observer> {
    pub(crate) fn build_arena(
        file: &'ast ArenaFile,
        source: &'src str,
        indexer: &Indexer,
        observer: &'observer mut dyn TraversalObserver,
        bash_runtime_vars_enabled: bool,
        shell_profile: ShellProfile,
    ) -> BuildOutput {
        let arena_store = &file.store;
        let file = file.view();
        let file_scope = Scope {
            id: ScopeId(0),
            kind: ScopeKind::File,
            parent: None,
            span: file.span(),
            bindings: FxHashMap::default(),
        };
        let runtime = RuntimePrelude::new(bash_runtime_vars_enabled);
        let mut builder = Self {
            source,
            arena_store: Some(arena_store),
            line_start_offsets: source_line_start_offsets(source),
            shell_profile: shell_profile.clone(),
            observer,
            scopes: vec![file_scope],
            bindings: Vec::new(),
            references: Vec::new(),
            reference_index: FxHashMap::default(),
            predefined_runtime_refs: FxHashSet::default(),
            guarded_parameter_refs: FxHashSet::default(),
            parameter_guard_flow_refs: FxHashSet::default(),
            defaulting_parameter_operand_refs: FxHashSet::default(),
            self_referential_assignment_refs: FxHashSet::default(),
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
            cleared_variables: FxHashMap::default(),
            runtime,
            completed_scopes: FxHashSet::default(),
            deferred_functions: Vec::new(),
            scope_stack: vec![ScopeId(0)],
            command_stack: Vec::new(),
            guarded_parameter_operand_depth: 0,
            defaulting_parameter_operand_depth: 0,
            short_circuit_condition_depth: 0,
            arithmetic_reference_kind: ReferenceKind::ArithmeticRead,
            word_reference_kind_override: None,
        };
        let file_commands = builder.visit_stmt_seq_arena(file.body(), FlowState::default());
        builder.recorded_program.set_file_commands(file_commands);
        builder.mark_scope_completed(ScopeId(0));
        builder.drain_deferred_functions();

        let call_graph = builder.build_call_graph();
        builder.mark_local_declarations_visible_to_later_calls();
        let heuristic_unused_assignments = builder.compute_heuristic_unused_assignments();

        BuildOutput {
            shell_profile,
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: builder.references,
            reference_index: builder.reference_index,
            predefined_runtime_refs: builder.predefined_runtime_refs,
            guarded_parameter_refs: builder.guarded_parameter_refs,
            parameter_guard_flow_refs: builder.parameter_guard_flow_refs,
            defaulting_parameter_operand_refs: builder.defaulting_parameter_operand_refs,
            self_referential_assignment_refs: builder.self_referential_assignment_refs,
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
            cleared_variables: builder.cleared_variables,
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

    fn arena_store(&self) -> &'ast AstStore {
        self.arena_store
            .expect("arena visitor requires arena storage")
    }

    fn arena_stmt_seq(&self, id: StmtSeqId) -> StmtSeqView<'ast> {
        self.arena_store().stmt_seq(id)
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

    fn visit_stmt_seq_arena(
        &mut self,
        commands: StmtSeqView<'_>,
        flow: FlowState,
    ) -> RecordedCommandRange {
        let mut recorded = Vec::with_capacity(commands.stmt_ids().len());
        self.visit_stmt_seq_arena_into(commands, flow, &mut recorded);
        self.recorded_program.push_command_ids(recorded)
    }

    fn visit_stmt_seq_arena_into(
        &mut self,
        commands: StmtSeqView<'_>,
        flow: FlowState,
        recorded: &mut Vec<RecordedCommandId>,
    ) {
        recorded.reserve(commands.stmt_ids().len());
        for stmt in commands.stmts() {
            recorded.push(self.visit_stmt_arena(stmt, flow));
        }
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
        let span = semantic_statement_span(stmt);
        let context = Self::flow_context(flow);
        self.flow_contexts.push((span, context));
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
        recorded
    }

    fn visit_stmt_arena(&mut self, stmt: StmtView<'_>, flow: FlowState) -> RecordedCommandId {
        let span = semantic_statement_span_arena(stmt);
        let scope = self.current_scope();
        let context = Self::flow_context(flow);
        self.flow_contexts.push((span, context));
        self.command_stack.push(span);

        let recorded = self.visit_command_arena(stmt.command(), flow);
        let redirects = self.visit_redirects_arena(stmt.redirects(), flow);
        if !redirects.is_empty() {
            self.prepend_nested_regions(recorded, redirects);
        }
        self.recorded_program.command_mut(recorded).span = span;
        self.recorded_program.command_infos.insert(
            SpanKey::new(span),
            recorded_command_info_arena(stmt.command(), self.source, self.runtime.bash_enabled()),
        );

        self.command_stack.pop();
        let _ = scope;
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

    fn visit_command_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        match command.kind() {
            ArenaFileCommandKind::Simple => self.visit_simple_command_arena(command, flow),
            ArenaFileCommandKind::Builtin => self.visit_builtin_arena(command, flow),
            ArenaFileCommandKind::Decl => self.visit_decl_arena(command, flow),
            ArenaFileCommandKind::Binary => self.visit_binary_arena(command, flow),
            ArenaFileCommandKind::Compound => self.visit_compound_arena(command, flow),
            ArenaFileCommandKind::Function => self.visit_function_arena(command, flow),
            ArenaFileCommandKind::AnonymousFunction => {
                self.visit_anonymous_function_arena(command, flow)
            }
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

        let command_words = std::iter::once(&command.name)
            .chain(command.args.iter())
            .collect::<Vec<_>>();
        let normalized = normalize_command_words(&command_words, self.source)
            .expect("simple commands always have a name");

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

    fn visit_simple_command_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        let command_span = command.span();
        let command = command
            .simple()
            .expect("simple command kind should expose simple payload");
        let mut nested_regions = Vec::new();
        let command_has_name = !matches!(
            static_word_text_arena(command.name(), self.source).as_deref(),
            Some("")
        );
        for assignment in command.assignments() {
            if command_has_name {
                self.visit_assignment_value_arena_into(assignment, flow, &mut nested_regions);
            } else {
                self.visit_assignment_arena_into(
                    assignment,
                    None,
                    BindingAttributes::empty(),
                    flow,
                    &mut nested_regions,
                );
            }
        }

        self.visit_word_arena_into(
            command.name(),
            WordVisitKind::Expansion,
            flow,
            &mut nested_regions,
        );
        for arg in command.args() {
            self.visit_word_arena_into(arg, WordVisitKind::Expansion, flow, &mut nested_regions);
        }

        let words = std::iter::once(command.name_id())
            .chain(command.arg_ids().iter().copied())
            .collect::<Vec<_>>();
        let normalized =
            normalize_command_words_arena(self.arena_store(), &words, command_span, self.source)
                .expect("simple commands always have a name");

        if let Some(name) = normalized.literal_name.as_deref()
            && !name.is_empty()
        {
            let callee = Name::from(name);
            let scope = self.current_scope();
            let call_site = CallSite {
                callee: callee.clone(),
                span: normalized.command_span,
                name_span: command.name().span(),
                scope,
                arg_count: command.arg_ids().len(),
            };
            self.call_sites
                .entry(callee.clone())
                .or_default()
                .push(call_site);
            self.recorded_program.call_command_spans.insert(
                SpanKey::new(normalized.command_span),
                self.command_stack
                    .last()
                    .copied()
                    .unwrap_or(normalized.command_span),
            );
        }

        if let Some(name) = normalized.effective_name.as_deref()
            && !name.is_empty()
        {
            let callee = Name::from(name);
            if resolved_command_can_affect_current_shell_arena(&normalized) {
                self.classify_special_simple_command_arena(&callee, &normalized, flow);
            }
        }

        self.record_command(
            normalized.command_span,
            nested_regions,
            RecordedCommandKind::Linear,
        )
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

    fn visit_builtin_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        let span = command.span();
        let command = command
            .builtin()
            .expect("builtin command kind should expose builtin payload");
        let mut nested_regions = Vec::new();
        for assignment in command.assignments() {
            self.visit_assignment_value_arena_into(assignment, flow, &mut nested_regions);
        }
        if let Some(primary) = command.primary() {
            self.visit_word_arena_into(
                primary,
                WordVisitKind::Expansion,
                flow,
                &mut nested_regions,
            );
        }
        for arg in command.extra_args() {
            self.visit_word_arena_into(arg, WordVisitKind::Expansion, flow, &mut nested_regions);
        }

        let kind = match command.kind() {
            BuiltinCommandNodeKind::Break => RecordedCommandKind::Break {
                depth: depth_from_static_text(
                    command
                        .primary()
                        .and_then(|word| static_word_text_arena(word, self.source)),
                ),
            },
            BuiltinCommandNodeKind::Continue => RecordedCommandKind::Continue {
                depth: depth_from_static_text(
                    command
                        .primary()
                        .and_then(|word| static_word_text_arena(word, self.source)),
                ),
            },
            BuiltinCommandNodeKind::Return => RecordedCommandKind::Return,
            BuiltinCommandNodeKind::Exit => RecordedCommandKind::Exit,
        };
        self.record_command(span, nested_regions, kind)
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
            self.visit_assignment_value_into(assignment, flow, &mut nested_regions);
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
            self.visit_assignment_value_into(assignment, flow, &mut nested_regions);
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

    fn visit_decl_arena(&mut self, command: CommandView<'_>, flow: FlowState) -> RecordedCommandId {
        let span = command.span();
        let command = command
            .decl()
            .expect("decl command kind should expose declaration payload");
        let mut nested_regions = Vec::new();
        for assignment in command.assignments() {
            self.visit_assignment_value_arena_into(assignment, flow, &mut nested_regions);
        }

        let builtin = declaration_builtin(command.variant());
        let flags = declaration_flags_arena(command.operands(), self.arena_store(), self.source);
        let global_flag_enabled = declaration_flag_is_enabled_arena(
            command.operands(),
            self.arena_store(),
            self.source,
            'g',
        )
        .unwrap_or(false);
        self.declarations.push(Declaration {
            builtin,
            span,
            operands: declaration_operands_arena(
                command.operands(),
                self.arena_store(),
                self.source,
            ),
        });

        let mut name_operands_are_function_names = false;
        for operand in command.operands() {
            match operand {
                DeclOperandNode::Flag(word) => {
                    update_declaration_function_name_mode_arena(
                        self.arena_store().word(*word),
                        self.source,
                        &mut name_operands_are_function_names,
                    );
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                }
                DeclOperandNode::Dynamic(word) => {
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                }
                DeclOperandNode::Name(name) => {
                    self.visit_var_ref_subscript_words_arena(
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
                DeclOperandNode::Assignment(assignment) => {
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
                    self.visit_assignment_arena_into(
                        assignment,
                        Some((kind, scope)),
                        attributes,
                        flow,
                        &mut nested_regions,
                    );
                }
            }
        }

        self.record_command(span, nested_regions, RecordedCommandKind::Linear)
    }

    fn visit_binary(&mut self, command: &BinaryCommand, flow: FlowState) -> RecordedCommandId {
        match command.op {
            BinaryOp::And | BinaryOp::Or => self.visit_logical_binary(command, flow),
            BinaryOp::Pipe | BinaryOp::PipeAll => self.visit_pipeline_binary(command, flow),
        }
    }

    fn visit_binary_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        let span = command.span();
        let command = command
            .binary()
            .expect("binary command kind should expose binary payload");
        match command.op() {
            BinaryOp::And | BinaryOp::Or => {
                let mut operators = SmallVec::<[RecordedListOperator; 4]>::new();
                let mut commands = SmallVec::<[StmtView<'_>; 4]>::new();
                collect_logical_segments_arena(command.left(), &mut commands, &mut operators);
                operators.push(recorded_list_operator(command.op()));
                collect_logical_segments_arena(command.right(), &mut commands, &mut operators);

                let mut recorded =
                    SmallVec::<[RecordedCommandId; 4]>::with_capacity(commands.len());
                for (index, stmt) in commands.into_iter().enumerate() {
                    let mut nested = flow;
                    nested.exit_status_checked =
                        operators.get(index).is_some() || flow.exit_status_checked;
                    if index > 0 {
                        nested.conditionally_executed = true;
                    }
                    recorded.push(self.visit_stmt_arena(stmt, nested));
                }

                let mut recorded = recorded.into_iter();
                let Some(first) = recorded.next() else {
                    unreachable!("logical lists have at least one command");
                };
                let rest = self.recorded_program.push_list_items(
                    operators
                        .into_iter()
                        .zip(recorded)
                        .map(|(operator, command)| RecordedListItem { operator, command })
                        .collect(),
                );
                self.record_command(span, Vec::new(), RecordedCommandKind::List { first, rest })
            }
            BinaryOp::Pipe | BinaryOp::PipeAll => {
                let mut flow = flow;
                flow.in_subshell = true;
                let mut commands = SmallVec::<[StmtView<'_>; 4]>::new();
                collect_pipeline_segments_arena(command.left(), &mut commands);
                collect_pipeline_segments_arena(command.right(), &mut commands);

                let mut segments = Vec::with_capacity(commands.len());
                for stmt in commands {
                    let scope =
                        self.push_scope(ScopeKind::Pipeline, self.current_scope(), stmt.span());
                    let recorded = self.visit_stmt_arena(stmt, flow);
                    self.pop_scope(scope);
                    self.mark_scope_completed(scope);
                    segments.push(RecordedPipelineSegment {
                        scope,
                        command: recorded,
                    });
                }
                let segments = self.recorded_program.push_pipeline_segments(segments);
                self.record_command(span, Vec::new(), RecordedCommandKind::Pipeline { segments })
            }
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
            if index > 0 {
                nested.conditionally_executed = true;
            }
            recorded.push(self.visit_stmt(stmt, nested));
        }

        let mut recorded = recorded.into_iter();
        let Some(first) = recorded.next() else {
            unreachable!("logical lists have at least one command");
        };
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

    fn visit_compound_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        let span = command.span();
        let node = command
            .compound()
            .expect("compound command kind should expose compound payload")
            .node();
        match node {
            CompoundCommandNode::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
                ..
            } => {
                let condition = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*condition),
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let then_branch = self
                    .visit_stmt_seq_arena(self.arena_stmt_seq(*then_branch), flow.conditional());
                let branches = self.arena_store().elif_branches(*elif_branches).to_vec();
                let elif_branches = branches
                    .into_iter()
                    .map(|branch| RecordedElifBranch {
                        condition: self.visit_stmt_seq_arena(
                            self.arena_stmt_seq(branch.condition),
                            FlowState {
                                exit_status_checked: true,
                                ..flow.conditional()
                            },
                        ),
                        body: self.visit_stmt_seq_arena(
                            self.arena_stmt_seq(branch.body),
                            flow.conditional(),
                        ),
                    })
                    .collect();
                let elif_branches = self.recorded_program.push_elif_branches(elif_branches);
                let else_branch = else_branch
                    .map(|body| {
                        self.visit_stmt_seq_arena(self.arena_stmt_seq(body), flow.conditional())
                    })
                    .unwrap_or_default();
                self.record_command(
                    span,
                    Vec::new(),
                    RecordedCommandKind::If {
                        condition,
                        then_branch,
                        elif_branches,
                        else_branch,
                    },
                )
            }
            CompoundCommandNode::While { condition, body } => {
                let condition = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*condition),
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    span,
                    Vec::new(),
                    RecordedCommandKind::While { condition, body },
                )
            }
            CompoundCommandNode::Until { condition, body } => {
                let condition = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*condition),
                    FlowState {
                        exit_status_checked: true,
                        ..flow
                    },
                );
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    span,
                    Vec::new(),
                    RecordedCommandKind::Until { condition, body },
                )
            }
            CompoundCommandNode::For {
                targets,
                words,
                body,
                ..
            } => {
                let word_ids = words
                    .map(|range| self.arena_store().word_ids(range))
                    .unwrap_or(&[]);
                let mut nested_regions = Vec::new();
                for word in word_ids {
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                }
                let items = if words.is_some() {
                    loop_binding_origin_for_static_texts(word_ids.iter().map(|word| {
                        static_word_text_arena(self.arena_store().word(*word), self.source)
                    }))
                } else {
                    LoopValueOrigin::ImplicitArgv
                };
                for target in self.arena_store().for_targets(*targets).to_vec() {
                    if let Some(name) = &target.name {
                        self.add_binding(
                            name,
                            BindingKind::LoopVariable,
                            self.current_scope(),
                            target.span,
                            BindingOrigin::LoopVariable {
                                definition_span: target.span,
                                items,
                            },
                            BindingAttributes::empty(),
                        );
                    }
                }
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(span, nested_regions, RecordedCommandKind::For { body })
            }
            CompoundCommandNode::Subshell(body) => {
                let scope = self.push_scope(ScopeKind::Subshell, self.current_scope(), span);
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
                self.record_command(span, Vec::new(), RecordedCommandKind::Subshell { body })
            }
            CompoundCommandNode::BraceGroup(body) => {
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        in_block: true,
                        ..flow
                    },
                );
                self.record_command(span, Vec::new(), RecordedCommandKind::BraceGroup { body })
            }
            CompoundCommandNode::Always { body, always_body } => {
                let block_flow = FlowState {
                    in_block: true,
                    ..flow
                };
                let mut body_commands = Vec::new();
                self.visit_stmt_seq_arena_into(
                    self.arena_stmt_seq(*body),
                    block_flow,
                    &mut body_commands,
                );
                self.visit_stmt_seq_arena_into(
                    self.arena_stmt_seq(*always_body),
                    block_flow,
                    &mut body_commands,
                );
                let body = self.recorded_program.push_command_ids(body_commands);
                self.record_command(span, Vec::new(), RecordedCommandKind::BraceGroup { body })
            }
            CompoundCommandNode::Repeat { count, body, .. } => {
                let mut nested_regions = Vec::new();
                self.visit_word_arena_into(
                    self.arena_store().word(*count),
                    WordVisitKind::Expansion,
                    flow,
                    &mut nested_regions,
                );
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(span, nested_regions, RecordedCommandKind::For { body })
            }
            CompoundCommandNode::Foreach {
                variable,
                variable_span,
                words,
                body,
                ..
            }
            | CompoundCommandNode::Select {
                variable,
                variable_span,
                words,
                body,
            } => {
                let word_ids = self.arena_store().word_ids(*words).to_vec();
                let mut nested_regions = Vec::new();
                for word in &word_ids {
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                }
                let items = loop_binding_origin_for_static_texts(word_ids.iter().map(|word| {
                    static_word_text_arena(self.arena_store().word(*word), self.source)
                }));
                self.add_binding(
                    variable,
                    BindingKind::LoopVariable,
                    self.current_scope(),
                    *variable_span,
                    BindingOrigin::LoopVariable {
                        definition_span: *variable_span,
                        items,
                    },
                    BindingAttributes::empty(),
                );
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                let kind = if matches!(node, CompoundCommandNode::Select { .. }) {
                    RecordedCommandKind::Select { body }
                } else {
                    RecordedCommandKind::For { body }
                };
                self.record_command(span, nested_regions, kind)
            }
            CompoundCommandNode::Time { command, .. } => {
                let mut nested_regions = Vec::new();
                if let Some(command) = command {
                    let commands = self.visit_stmt_seq_arena(self.arena_stmt_seq(*command), flow);
                    let ids = self.recorded_program.commands_in(commands).to_vec();
                    for command_id in ids {
                        nested_regions.extend(self.flatten_recorded_regions(command_id));
                    }
                }
                self.record_command(span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommandNode::Coproc { body, .. } => {
                let commands = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(*body),
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                );
                let ids = self.recorded_program.commands_in(commands).to_vec();
                let mut nested_regions = Vec::new();
                for command_id in ids {
                    nested_regions.extend(self.flatten_recorded_regions(command_id));
                }
                self.record_command(span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommandNode::ArithmeticFor(command) => {
                let mut nested_regions = Vec::new();
                self.visit_optional_arithmetic_expr_arena_into(
                    command.init_ast.as_ref(),
                    flow,
                    &mut nested_regions,
                );
                self.visit_optional_arithmetic_expr_arena_into(
                    command.condition_ast.as_ref(),
                    flow,
                    &mut nested_regions,
                );
                self.visit_optional_arithmetic_expr_arena_into(
                    command.step_ast.as_ref(),
                    flow,
                    &mut nested_regions,
                );
                let body = self.visit_stmt_seq_arena(
                    self.arena_stmt_seq(command.body),
                    FlowState {
                        loop_depth: flow.loop_depth + 1,
                        ..flow.conditional()
                    },
                );
                self.record_command(
                    span,
                    nested_regions,
                    RecordedCommandKind::ArithmeticFor { body },
                )
            }
            CompoundCommandNode::Arithmetic(command) => {
                let nested_regions =
                    self.visit_optional_arithmetic_expr_arena(command.expr_ast.as_ref(), flow);
                self.record_command(span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommandNode::Conditional(command) => {
                let nested_regions = self.visit_conditional_expr_arena(&command.expression, flow);
                self.record_command(span, nested_regions, RecordedCommandKind::Linear)
            }
            CompoundCommandNode::Case { word, cases } => {
                let mut nested_regions = Vec::new();
                self.visit_word_arena_into(
                    self.arena_store().word(*word),
                    WordVisitKind::Expansion,
                    flow,
                    &mut nested_regions,
                );
                let cases = self.arena_store().case_items(*cases).to_vec();
                let arms = cases
                    .into_iter()
                    .map(|case| {
                        let pattern_regions = self.visit_patterns_arena(
                            self.arena_store().patterns(case.patterns),
                            WordVisitKind::Conditional,
                            flow,
                        );
                        let mut commands = Vec::new();
                        self.visit_stmt_seq_arena_into(
                            self.arena_stmt_seq(case.body),
                            flow.conditional(),
                            &mut commands,
                        );
                        if !pattern_regions.is_empty() {
                            if let Some(&first) = commands.first() {
                                self.prepend_nested_regions(first, pattern_regions);
                            } else {
                                commands.push(self.record_command(
                                    span,
                                    pattern_regions,
                                    RecordedCommandKind::Linear,
                                ));
                            }
                        }
                        RecordedCaseArm {
                            terminator: case.terminator,
                            matches_anything: case_arm_matches_anything_arena(
                                self.arena_store().patterns(case.patterns),
                                self.arena_store(),
                            ),
                            commands: self.recorded_program.push_command_ids(commands),
                        }
                    })
                    .collect();
                let arms = self.recorded_program.push_case_arms(arms);
                self.record_command(span, nested_regions, RecordedCommandKind::Case { arms })
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
                    definition_span: function.span,
                },
                BindingAttributes::empty(),
            );
            self.recorded_program
                .function_body_scopes
                .insert(binding_id, scope);
        }
        self.deferred_functions.push(DeferredFunction {
            body: DeferredFunctionBody::Recursive(function.clone()),
            scope,
            flow,
        });
        self.pop_scope(scope);

        self.record_command(function.span, nested_regions, RecordedCommandKind::Linear)
    }

    fn visit_function_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        let span = command.span();
        let function = command
            .function()
            .expect("function command kind should expose function payload");
        let mut nested_regions = Vec::new();
        for entry in function.entries() {
            self.visit_word_arena_into(
                self.arena_store().word(entry.word),
                WordVisitKind::Expansion,
                flow,
                &mut nested_regions,
            );
        }

        let parent_scope = self.current_scope();
        let names = function
            .entries()
            .iter()
            .filter_map(|entry| entry.static_name.clone())
            .collect::<Vec<_>>();
        let scope = self.push_scope(
            ScopeKind::Function(if names.is_empty() {
                FunctionScopeKind::Dynamic
            } else {
                FunctionScopeKind::Named(names)
            }),
            parent_scope,
            function.body().span(),
        );
        for entry in function.entries() {
            let Some(name) = &entry.static_name else {
                continue;
            };
            let binding_id = self.add_binding(
                name,
                BindingKind::FunctionDefinition,
                parent_scope,
                self.arena_store().word(entry.word).span(),
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
            body: DeferredFunctionBody::Arena(function.body_id()),
            scope,
            flow,
        });
        self.pop_scope(scope);

        self.record_command(span, nested_regions, RecordedCommandKind::Linear)
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

    fn visit_anonymous_function_arena(
        &mut self,
        command: CommandView<'_>,
        flow: FlowState,
    ) -> RecordedCommandId {
        let span = command.span();
        let function = command
            .anonymous_function()
            .expect("anonymous function kind should expose function payload");
        let mut nested_regions = Vec::new();
        for arg in function.args() {
            self.visit_word_arena_into(arg, WordVisitKind::Expansion, flow, &mut nested_regions);
        }
        let scope = self.push_scope(
            ScopeKind::Function(FunctionScopeKind::Anonymous),
            self.current_scope(),
            function.body().span(),
        );
        let body = self.visit_function_like_body_arena(function.body_id(), flow);
        self.pop_scope(scope);
        self.mark_scope_completed(scope);

        self.record_command(
            span,
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
        let reference_start = self.references.len();
        self.visit_var_ref_subscript_words(
            Some(&assignment.target.name),
            assignment.target.subscript.as_deref(),
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
        let self_referential_refs =
            self.newly_added_reference_ids_reading_name(&assignment.target.name, reference_start);
        if !self_referential_refs.is_empty() {
            attributes |= BindingAttributes::SELF_REFERENTIAL_READ;
            self.self_referential_assignment_refs
                .extend(self_referential_refs);
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
            binding_origin_for_assignment(assignment, self.source),
            attributes,
        );
        self.record_prompt_assignment_references(assignment);
        if let Some(hint) = indirect_target_hint(assignment, self.source) {
            self.indirect_target_hints.insert(binding, hint);
        }
    }

    fn visit_assignment_arena_into(
        &mut self,
        assignment: &AssignmentNode,
        declaration_kind: Option<(BindingKind, ScopeId)>,
        mut attributes: BindingAttributes,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let reference_start = self.references.len();
        self.visit_var_ref_subscript_words_arena(
            Some(&assignment.target.name),
            assignment.target.subscript.as_deref(),
            WordVisitKind::Expansion,
            flow,
            nested_regions,
        );
        self.visit_assignment_value_arena_into(assignment, flow, nested_regions);
        let (kind, scope) = declaration_kind.unwrap_or_else(|| {
            let kind = if assignment.append {
                BindingKind::AppendAssignment
            } else if matches!(assignment.value, AssignmentValueNode::Compound(_))
                || assignment.target.subscript.is_some()
            {
                BindingKind::ArrayAssignment
            } else {
                BindingKind::Assignment
            };
            (kind, self.current_scope())
        });
        attributes |= assignment_binding_attributes_arena(assignment);
        if assignment_has_empty_initializer_arena(assignment, self.arena_store(), self.source) {
            attributes |= BindingAttributes::EMPTY_INITIALIZER;
        }
        let self_referential_refs =
            self.newly_added_reference_ids_reading_name(&assignment.target.name, reference_start);
        if !self_referential_refs.is_empty() {
            attributes |= BindingAttributes::SELF_REFERENTIAL_READ;
            self.self_referential_assignment_refs
                .extend(self_referential_refs);
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
            binding_origin_for_assignment_arena(assignment, self.arena_store(), self.source),
            attributes,
        );
        self.record_prompt_assignment_references_arena(assignment);
        if let Some(hint) = indirect_target_hint_arena(assignment, self.arena_store(), self.source)
        {
            self.indirect_target_hints.insert(binding, hint);
        }
    }

    fn record_prompt_assignment_references(&mut self, assignment: &Assignment) {
        let AssignmentValue::Scalar(word) = &assignment.value else {
            return;
        };

        match assignment.target.name.as_str() {
            "PS1" => {
                for (name, span) in prompt_assignment_reference_names(word, self.source) {
                    self.add_reference(&name, ReferenceKind::ImplicitRead, span);
                }
            }
            "PS4" => {
                for name in escaped_prompt_assignment_reference_names(word, self.source) {
                    self.add_reference(
                        &name,
                        ReferenceKind::PromptExpansion,
                        assignment.target.name_span,
                    );
                }
            }
            _ => {}
        }
    }

    fn record_prompt_assignment_references_arena(&mut self, assignment: &AssignmentNode) {
        let AssignmentValueNode::Scalar(word) = &assignment.value else {
            return;
        };
        let word = self.arena_store().word(*word);

        match assignment.target.name.as_str() {
            "PS1" => {
                for (name, span) in prompt_assignment_reference_names_arena(word, self.source) {
                    self.add_reference(&name, ReferenceKind::ImplicitRead, span);
                }
            }
            "PS4" => {
                for name in escaped_prompt_assignment_reference_names_arena(word, self.source) {
                    self.add_reference(
                        &name,
                        ReferenceKind::PromptExpansion,
                        assignment.target.name_span,
                    );
                }
            }
            _ => {}
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

    fn visit_assignment_value_arena_into(
        &mut self,
        assignment: &AssignmentNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &assignment.value {
            AssignmentValueNode::Scalar(word) => {
                self.visit_word_arena_into(
                    self.arena_store().word(*word),
                    WordVisitKind::Expansion,
                    flow,
                    nested_regions,
                );
            }
            AssignmentValueNode::Compound(array) => {
                for element in self.arena_store().array_elems(array.elements).to_vec() {
                    match element {
                        shuck_ast::ArrayElemNode::Sequential(value) => self.visit_word_arena_into(
                            self.arena_store().word(value.word),
                            WordVisitKind::Expansion,
                            flow,
                            nested_regions,
                        ),
                        shuck_ast::ArrayElemNode::Keyed { key, value }
                        | shuck_ast::ArrayElemNode::KeyedAppend { key, value } => {
                            self.visit_var_ref_subscript_words_arena(
                                None,
                                Some(&key),
                                WordVisitKind::Expansion,
                                flow,
                                nested_regions,
                            );
                            self.visit_word_arena_into(
                                self.arena_store().word(value.word),
                                WordVisitKind::Expansion,
                                flow,
                                nested_regions,
                            );
                        }
                    }
                }
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
                    self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
                    self.add_redirect_fd_var_binding(redirect);
                }
                None => {
                    self.add_redirect_fd_var_binding(redirect);
                    let Some(heredoc) = redirect.heredoc() else {
                        continue;
                    };
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

    fn visit_redirects_arena(
        &mut self,
        redirects: &[RedirectNode],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for redirect in redirects {
            match &redirect.target {
                RedirectTargetNode::Word(word) => {
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        WordVisitKind::Expansion,
                        flow,
                        &mut nested_regions,
                    );
                    self.add_redirect_fd_var_binding_arena(redirect);
                }
                RedirectTargetNode::Heredoc(heredoc) => {
                    self.add_redirect_fd_var_binding_arena(redirect);
                    if heredoc.delimiter.expands_body {
                        self.visit_heredoc_body_arena_into(
                            &heredoc.body,
                            WordVisitKind::Expansion,
                            flow,
                            &mut nested_regions,
                        );
                    }
                }
            }
        }
        nested_regions
    }

    fn add_redirect_fd_var_binding(&mut self, redirect: &shuck_ast::Redirect) {
        if let (Some(name), Some(span)) = (&redirect.fd_var, redirect.fd_var_span) {
            self.add_binding(
                name,
                BindingKind::Assignment,
                self.current_scope(),
                span,
                BindingOrigin::Assignment {
                    definition_span: span,
                    value: AssignmentValueOrigin::StaticLiteral,
                },
                BindingAttributes::INTEGER,
            );
        }
    }

    fn add_redirect_fd_var_binding_arena(&mut self, redirect: &RedirectNode) {
        if let (Some(name), Some(span)) = (&redirect.fd_var, redirect.fd_var_span) {
            self.add_binding(
                name,
                BindingKind::Assignment,
                self.current_scope(),
                span,
                BindingOrigin::Assignment {
                    definition_span: span,
                    value: AssignmentValueOrigin::StaticLiteral,
                },
                BindingAttributes::INTEGER,
            );
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

    fn visit_word_arena_into(
        &mut self,
        word: shuck_ast::WordView<'_>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if word_is_semantically_inert_arena(word, self.arena_store()) {
            return;
        }
        for part in word.parts() {
            self.visit_word_part_arena(&part.kind, part.span, kind, flow, nested_regions);
        }
    }

    fn visit_heredoc_body_into(
        &mut self,
        body: &HeredocBody,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if !body.mode.expands() || heredoc_body_is_semantically_inert(body, self.source) {
            return;
        }
        self.visit_heredoc_body_part_nodes(&body.parts, kind, flow, nested_regions);
    }

    fn visit_heredoc_body_arena_into(
        &mut self,
        body: &shuck_ast::HeredocBodyNode,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if !body.mode.expands() {
            return;
        }
        for part in self.arena_store().heredoc_body_parts(body.parts).to_vec() {
            self.visit_heredoc_body_part_arena(&part.kind, part.span, kind, flow, nested_regions);
        }
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

    fn visit_patterns_arena(
        &mut self,
        patterns: &[PatternNode],
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        for pattern in patterns {
            self.visit_pattern_arena_into(pattern, kind, flow, &mut nested_regions);
        }
        nested_regions
    }

    fn visit_pattern_arena_into(
        &mut self,
        pattern: &PatternNode,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if pattern_is_semantically_inert_arena(pattern, self.arena_store()) {
            return;
        }
        for part in self.arena_store().pattern_parts(pattern.parts).to_vec() {
            self.visit_pattern_part_arena(&part.kind, kind, flow, nested_regions);
        }
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
        let reference_kind = self.word_reference_kind_override.unwrap_or(reference_kind);
        let id = self.add_reference(&reference.name, reference_kind, span);
        self.visit_var_ref_subscript_words(
            Some(&reference.name),
            reference.subscript.as_deref(),
            word_visit_kind_for_reference_kind(reference_kind),
            flow,
            nested_regions,
        );
        id
    }

    fn visit_var_ref_reference_arena(
        &mut self,
        reference: &VarRefNode,
        reference_kind: ReferenceKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
        span: Span,
    ) -> ReferenceId {
        let reference_kind = self.word_reference_kind_override.unwrap_or(reference_kind);
        let id = self.add_reference(&reference.name, reference_kind, span);
        self.visit_var_ref_subscript_words_arena(
            Some(&reference.name),
            reference.subscript.as_deref(),
            word_visit_kind_for_reference_kind(reference_kind),
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

        if !uses_associative_word_semantics {
            self.visit_unparsed_arithmetic_subscript_references(subscript);
        }

        self.visit_fragment_word(
            subscript.word_ast(),
            Some(subscript.syntax_source_text()),
            kind,
            flow,
            nested_regions,
        );
    }

    fn visit_var_ref_subscript_words_arena(
        &mut self,
        owner_name: Option<&Name>,
        subscript: Option<&SubscriptNode>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let Some(subscript) = subscript else {
            return;
        };
        if subscript_selector_arena(subscript).is_some() {
            return;
        }
        let uses_associative_word_semantics = matches!(
            subscript.interpretation,
            shuck_ast::SubscriptInterpretation::Associative
        ) || owner_name.is_some_and(|name| {
            self.resolve_reference(
                name,
                self.current_scope(),
                subscript_span_arena(subscript).start.offset,
            )
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
            self.visit_optional_arithmetic_expr_arena_into(Some(expression), flow, nested_regions);
            return;
        }

        if !uses_associative_word_semantics {
            for (name, span) in unparsed_arithmetic_subscript_reference_names(
                subscript_syntax_source_text_arena(subscript),
                self.source,
            ) {
                self.add_reference(&name, ReferenceKind::ArithmeticRead, span);
            }
        }

        if let Some(word) = subscript.word_ast {
            self.visit_word_arena_into(self.arena_store().word(word), kind, flow, nested_regions);
        }
    }

    fn visit_unparsed_arithmetic_subscript_references(&mut self, subscript: &Subscript) {
        for (name, span) in unparsed_arithmetic_subscript_reference_names(
            subscript.syntax_source_text(),
            self.source,
        ) {
            self.add_reference(&name, ReferenceKind::ArithmeticRead, span);
        }
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
                    self.word_reference_kind_override
                        .unwrap_or(reference_kind_for_word_visit(
                            kind,
                            ReferenceKind::Expansion,
                        )),
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
                    parameter_operation_reference_kind(kind, operator),
                    flow,
                    nested_regions,
                    reference.span,
                );
                if parameter_operator_guards_unset_reference(operator) {
                    self.record_guarded_parameter_reference(reference_id);
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
                    reference_kind_for_word_visit(kind, ReferenceKind::Length),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::ArrayAccess(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ArrayAccess),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::ArrayIndices(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::PrefixMatch { .. } => {}
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let id = self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
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
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
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
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
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
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
        }
    }

    fn visit_word_part_arena(
        &mut self,
        part: &WordPartArena,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            WordPartArena::ZshQualifiedGlob(glob) => {
                for segment in self.arena_store().zsh_glob_segments(glob.segments).to_vec() {
                    if let shuck_ast::ZshGlobSegmentNode::Pattern(pattern) = segment {
                        self.visit_pattern_arena_into(&pattern, kind, flow, nested_regions);
                    }
                }
            }
            WordPartArena::Literal(_) | WordPartArena::SingleQuoted { .. } => {}
            WordPartArena::DoubleQuoted { parts, .. } => {
                for part in self.arena_store().word_parts(*parts).to_vec() {
                    self.visit_word_part_arena(&part.kind, part.span, kind, flow, nested_regions);
                }
            }
            WordPartArena::Variable(name) => {
                self.add_reference(
                    name,
                    self.word_reference_kind_override
                        .unwrap_or(reference_kind_for_word_visit(
                            kind,
                            ReferenceKind::Expansion,
                        )),
                    span,
                );
            }
            WordPartArena::CommandSubstitution { body, .. }
            | WordPartArena::ProcessSubstitution { body, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let mut commands = Vec::new();
                self.visit_stmt_seq_arena_into(
                    self.arena_stmt_seq(*body),
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
            WordPartArena::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_arena_into(
                    expression_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            WordPartArena::Parameter(parameter) => {
                self.visit_parameter_expansion_arena(
                    parameter,
                    kind,
                    flow,
                    nested_regions,
                    parameter.span,
                );
            }
            WordPartArena::ParameterExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                let reference_id = self.visit_var_ref_reference_arena(
                    reference,
                    parameter_operation_reference_kind(kind, operator),
                    flow,
                    nested_regions,
                    reference.span,
                );
                if parameter_operator_guards_unset_reference(operator) {
                    self.record_guarded_parameter_reference(reference_id);
                }
                if matches!(operator, ParameterOp::AssignDefault) {
                    self.add_parameter_default_binding_arena(reference);
                }
                self.visit_parameter_operator_operand_arena(
                    operator,
                    *operand_word_ast,
                    kind,
                    flow,
                    nested_regions,
                );
            }
            WordPartArena::Length(reference) | WordPartArena::ArrayLength(reference) => {
                self.visit_var_ref_reference_arena(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::Length),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPartArena::ArrayAccess(reference) => {
                self.visit_var_ref_reference_arena(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ArrayAccess),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPartArena::ArrayIndices(reference) => {
                self.visit_var_ref_reference_arena(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPartArena::PrefixMatch { .. } => {}
            WordPartArena::IndirectExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                let id = self.visit_var_ref_reference_arena(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.indirect_expansion_refs.insert(id);
                if let Some(operator) = operator {
                    self.visit_parameter_operator_operand_arena(
                        operator,
                        *operand_word_ast,
                        kind,
                        flow,
                        nested_regions,
                    );
                }
            }
            WordPartArena::Substring {
                reference,
                offset_ast,
                length_ast,
                ..
            }
            | WordPartArena::ArraySlice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                self.visit_var_ref_reference_arena(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.visit_optional_arithmetic_expr_arena_into(
                    offset_ast.as_ref(),
                    flow,
                    nested_regions,
                );
                self.visit_optional_arithmetic_expr_arena_into(
                    length_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            WordPartArena::Transformation { reference, .. } => {
                self.visit_var_ref_reference_arena(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
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
            HeredocBodyPart::Literal(text) => {
                self.visit_escaped_braced_literal_references(text, span, kind);
            }
            HeredocBodyPart::Variable(name) => {
                self.add_reference(
                    name,
                    reference_kind_for_word_visit(kind, ReferenceKind::Expansion),
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

    fn visit_heredoc_body_part_arena(
        &mut self,
        part: &ArenaHeredocBodyPart,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            ArenaHeredocBodyPart::Literal(text) => {
                self.visit_escaped_braced_literal_references(text, span, kind);
            }
            ArenaHeredocBodyPart::Variable(name) => {
                self.add_reference(
                    name,
                    reference_kind_for_word_visit(kind, ReferenceKind::Expansion),
                    span,
                );
            }
            ArenaHeredocBodyPart::CommandSubstitution { body, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let mut commands = Vec::new();
                self.visit_stmt_seq_arena_into(
                    self.arena_stmt_seq(*body),
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
            ArenaHeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_arena_into(
                    expression_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            ArenaHeredocBodyPart::Parameter(parameter) => {
                self.visit_parameter_expansion_arena(parameter, kind, flow, nested_regions, span);
            }
        }
    }

    fn visit_escaped_braced_literal_references(
        &mut self,
        text: &LiteralText,
        span: Span,
        kind: WordVisitKind,
    ) {
        if !text.is_source_backed() {
            return;
        }

        for (name, span) in
            escaped_braced_literal_reference_names(text.syntax_str(self.source, span), span)
        {
            self.add_reference(
                &name,
                reference_kind_for_word_visit(kind, ReferenceKind::Expansion),
                span,
            );
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
                        reference_kind_for_word_visit(kind, ReferenceKind::ArrayAccess),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Length { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::Length),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Indices { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
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
                        reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
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
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset_ast,
                    length_ast,
                    ..
                } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                    self.visit_parameter_slice_arithmetic_expr_into(
                        offset_ast.as_ref(),
                        flow,
                        nested_regions,
                    );
                    self.visit_parameter_slice_arithmetic_expr_into(
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
                        parameter_operation_reference_kind(kind, operator),
                        flow,
                        nested_regions,
                        span,
                    );
                    if parameter_operator_guards_unset_reference(operator) {
                        self.record_guarded_parameter_reference(reference_id);
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
                        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh {
                            self.visit_var_ref_reference(
                                reference,
                                reference_kind_for_word_visit(
                                    kind,
                                    ReferenceKind::ParameterExpansion,
                                ),
                                flow,
                                nested_regions,
                                span,
                            );
                        }
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
                            self.defaulting_parameter_operand_depth += 1;
                            self.visit_fragment_word(
                                operation.operand_word_ast(),
                                Some(operand),
                                kind,
                                flow,
                                nested_regions,
                            );
                            self.guarded_parameter_operand_depth -= 1;
                            self.defaulting_parameter_operand_depth -= 1;
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            replacement,
                            ..
                        } => {
                            self.visit_fragment_word(
                                operation.pattern_word_ast(),
                                Some(pattern),
                                WordVisitKind::ParameterPattern,
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

    fn visit_parameter_expansion_arena(
        &mut self,
        parameter: &ParameterExpansionNode,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
        span: Span,
    ) {
        match &parameter.syntax {
            ParameterExpansionSyntaxNode::Bourne(syntax) => match syntax {
                BourneParameterExpansionNode::Access { reference } => {
                    self.visit_var_ref_reference_arena(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ArrayAccess),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansionNode::Length { reference } => {
                    self.visit_var_ref_reference_arena(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::Length),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansionNode::Indices { reference } => {
                    self.visit_var_ref_reference_arena(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansionNode::Indirect {
                    reference,
                    operator,
                    operand_word_ast,
                    ..
                } => {
                    let id = self.visit_var_ref_reference_arena(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                    self.indirect_expansion_refs.insert(id);
                    if let Some(operator) = operator {
                        self.visit_parameter_operator_operand_arena(
                            operator,
                            *operand_word_ast,
                            kind,
                            flow,
                            nested_regions,
                        );
                    }
                }
                BourneParameterExpansionNode::PrefixMatch { .. } => {}
                BourneParameterExpansionNode::Slice {
                    reference,
                    offset_ast,
                    length_ast,
                    ..
                } => {
                    self.visit_var_ref_reference_arena(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                    self.visit_parameter_slice_arithmetic_expr_arena_into(
                        offset_ast.as_ref(),
                        flow,
                        nested_regions,
                    );
                    self.visit_parameter_slice_arithmetic_expr_arena_into(
                        length_ast.as_ref(),
                        flow,
                        nested_regions,
                    );
                }
                BourneParameterExpansionNode::Operation {
                    reference,
                    operator,
                    operand_word_ast,
                    ..
                } => {
                    let reference_id = self.visit_var_ref_reference_arena(
                        reference,
                        parameter_operation_reference_kind(kind, operator),
                        flow,
                        nested_regions,
                        span,
                    );
                    if parameter_operator_guards_unset_reference(operator) {
                        self.record_guarded_parameter_reference(reference_id);
                    }
                    if matches!(operator, ParameterOp::AssignDefault) {
                        self.add_parameter_default_binding_arena(reference);
                    }
                    self.visit_parameter_operator_operand_arena(
                        operator,
                        *operand_word_ast,
                        kind,
                        flow,
                        nested_regions,
                    );
                }
                BourneParameterExpansionNode::Transformation { reference, .. } => {
                    self.visit_var_ref_reference_arena(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                }
            },
            ParameterExpansionSyntaxNode::Zsh(syntax) => {
                match &syntax.target {
                    ZshExpansionTargetNode::Reference(reference) => {
                        if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh {
                            self.visit_var_ref_reference_arena(
                                reference,
                                reference_kind_for_word_visit(
                                    kind,
                                    ReferenceKind::ParameterExpansion,
                                ),
                                flow,
                                nested_regions,
                                span,
                            );
                        }
                    }
                    ZshExpansionTargetNode::Word(word) => {
                        self.visit_word_arena_into(
                            self.arena_store().word(*word),
                            kind,
                            flow,
                            nested_regions,
                        );
                    }
                    ZshExpansionTargetNode::Nested(parameter) => {
                        self.visit_parameter_expansion_arena(
                            parameter,
                            kind,
                            flow,
                            nested_regions,
                            span,
                        );
                    }
                    ZshExpansionTargetNode::Empty => {}
                }
                for modifier in self.arena_store().zsh_modifiers(syntax.modifiers).to_vec() {
                    if let Some(word) = modifier.argument_word_ast {
                        self.visit_word_arena_into(
                            self.arena_store().word(word),
                            kind,
                            flow,
                            nested_regions,
                        );
                    }
                }
                if let Some(operation) = &syntax.operation {
                    self.visit_zsh_operation_arena(operation, kind, flow, nested_regions);
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
                self.visit_pattern_into(
                    pattern,
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
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
                self.visit_pattern_into(
                    pattern,
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
                self.visit_fragment_word(
                    operator.replacement_word_ast(),
                    Some(replacement),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            ParameterOp::UseDefault | ParameterOp::UseReplacement => {
                self.guarded_parameter_operand_depth += 1;
                self.defaulting_parameter_operand_depth += 1;
                self.visit_fragment_word(operand_word_ast, operand, kind, flow, nested_regions);
                self.guarded_parameter_operand_depth -= 1;
                self.defaulting_parameter_operand_depth -= 1;
            }
            ParameterOp::AssignDefault | ParameterOp::Error => {
                self.defaulting_parameter_operand_depth += 1;
                self.visit_fragment_word(operand_word_ast, operand, kind, flow, nested_regions);
                self.defaulting_parameter_operand_depth -= 1;
            }
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    fn visit_parameter_operator_operand_arena(
        &mut self,
        operator: &ParameterOp,
        operand_word_ast: Option<WordId>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                self.visit_pattern_into(
                    pattern,
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
            }
            ParameterOp::ReplaceFirst { pattern, .. } | ParameterOp::ReplaceAll { pattern, .. } => {
                self.visit_pattern_into(
                    pattern,
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
                if let Some(word) = operator.replacement_word_ast() {
                    self.visit_word_into(word, kind, flow, nested_regions);
                } else if let Some(word) = operand_word_ast {
                    self.visit_word_arena_into(
                        self.arena_store().word(word),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
            }
            ParameterOp::UseDefault | ParameterOp::UseReplacement => {
                self.guarded_parameter_operand_depth += 1;
                self.defaulting_parameter_operand_depth += 1;
                if let Some(word) = operand_word_ast {
                    self.visit_word_arena_into(
                        self.arena_store().word(word),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
                self.guarded_parameter_operand_depth -= 1;
                self.defaulting_parameter_operand_depth -= 1;
            }
            ParameterOp::AssignDefault | ParameterOp::Error => {
                self.defaulting_parameter_operand_depth += 1;
                if let Some(word) = operand_word_ast {
                    self.visit_word_arena_into(
                        self.arena_store().word(word),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
                self.defaulting_parameter_operand_depth -= 1;
            }
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    fn visit_zsh_operation_arena(
        &mut self,
        operation: &ZshExpansionOperationNode,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match operation {
            ZshExpansionOperationNode::PatternOperation {
                operand_word_ast, ..
            }
            | ZshExpansionOperationNode::TrimOperation {
                operand_word_ast, ..
            }
            | ZshExpansionOperationNode::Unknown {
                word_ast: operand_word_ast,
                ..
            } => {
                self.visit_word_arena_into(
                    self.arena_store().word(*operand_word_ast),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            ZshExpansionOperationNode::Defaulting {
                operand_word_ast, ..
            } => {
                self.guarded_parameter_operand_depth += 1;
                self.defaulting_parameter_operand_depth += 1;
                self.visit_word_arena_into(
                    self.arena_store().word(*operand_word_ast),
                    kind,
                    flow,
                    nested_regions,
                );
                self.guarded_parameter_operand_depth -= 1;
                self.defaulting_parameter_operand_depth -= 1;
            }
            ZshExpansionOperationNode::ReplacementOperation {
                pattern_word_ast,
                replacement_word_ast,
                ..
            } => {
                self.visit_word_arena_into(
                    self.arena_store().word(*pattern_word_ast),
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
                if let Some(word) = replacement_word_ast {
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
            }
            ZshExpansionOperationNode::Slice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.visit_word_arena_into(
                    self.arena_store().word(*offset_word_ast),
                    kind,
                    flow,
                    nested_regions,
                );
                if let Some(word) = length_word_ast {
                    self.visit_word_arena_into(
                        self.arena_store().word(*word),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
            }
        }
    }

    fn record_guarded_parameter_reference(&mut self, reference_id: ReferenceId) {
        self.guarded_parameter_refs.insert(reference_id);
        if self.defaulting_parameter_operand_depth == 0 && self.short_circuit_condition_depth == 0 {
            self.parameter_guard_flow_refs.insert(reference_id);
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

    fn visit_pattern_part_arena(
        &mut self,
        part: &PatternPartArena,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            PatternPartArena::Group { patterns, .. } => {
                for pattern in self.arena_store().patterns(*patterns).to_vec() {
                    self.visit_pattern_arena_into(&pattern, kind, flow, nested_regions);
                }
            }
            PatternPartArena::Word(word) => {
                self.visit_word_arena_into(
                    self.arena_store().word(*word),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            PatternPartArena::Literal(_)
            | PatternPartArena::AnyString
            | PatternPartArena::AnyChar
            | PatternPartArena::CharClass(_) => {}
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

    fn visit_conditional_expr_arena(
        &mut self,
        expression: &ConditionalExprArena,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_conditional_expr_arena_into(expression, flow, &mut nested_regions);
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
                if conditional_binary_op_uses_arithmetic_operands(expr.op) {
                    self.visit_conditional_arithmetic_operand_into(
                        &expr.left,
                        flow,
                        nested_regions,
                    );
                    self.visit_conditional_arithmetic_operand_into(
                        &expr.right,
                        flow,
                        nested_regions,
                    );
                } else if matches!(expr.op, ConditionalBinaryOp::And | ConditionalBinaryOp::Or) {
                    self.visit_conditional_expr_into(&expr.left, flow, nested_regions);
                    self.short_circuit_condition_depth += 1;
                    self.visit_conditional_expr_into(&expr.right, flow, nested_regions);
                    self.short_circuit_condition_depth -= 1;
                } else {
                    self.visit_conditional_expr_into(&expr.left, flow, nested_regions);
                    self.visit_conditional_expr_into(&expr.right, flow, nested_regions);
                }
            }
            ConditionalExpr::Unary(expr) => {
                if expr.op == ConditionalUnaryOp::VariableSet
                    && let Some((name, span)) =
                        variable_set_test_operand_name(&expr.expr, self.source)
                {
                    self.add_reference_if_bound(&name, ReferenceKind::ConditionalOperand, span);
                }
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

    fn visit_conditional_arithmetic_operand_into(
        &mut self,
        expression: &ConditionalExpr,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if let Some((name, span)) = conditional_arithmetic_operand_name(expression, self.source) {
            self.add_reference(&name, ReferenceKind::ArithmeticRead, span);
            return;
        }

        self.visit_conditional_expr_into(expression, flow, nested_regions);
    }

    fn visit_conditional_expr_arena_into(
        &mut self,
        expression: &ConditionalExprArena,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match expression {
            ConditionalExprArena::Binary {
                left, op, right, ..
            } => {
                if conditional_binary_op_uses_arithmetic_operands(*op) {
                    self.visit_conditional_arithmetic_operand_arena_into(
                        left,
                        flow,
                        nested_regions,
                    );
                    self.visit_conditional_arithmetic_operand_arena_into(
                        right,
                        flow,
                        nested_regions,
                    );
                } else if matches!(op, ConditionalBinaryOp::And | ConditionalBinaryOp::Or) {
                    self.visit_conditional_expr_arena_into(left, flow, nested_regions);
                    self.short_circuit_condition_depth += 1;
                    self.visit_conditional_expr_arena_into(right, flow, nested_regions);
                    self.short_circuit_condition_depth -= 1;
                } else {
                    self.visit_conditional_expr_arena_into(left, flow, nested_regions);
                    self.visit_conditional_expr_arena_into(right, flow, nested_regions);
                }
            }
            ConditionalExprArena::Unary { op, expr, .. } => {
                if *op == ConditionalUnaryOp::VariableSet
                    && let Some((name, span)) =
                        variable_set_test_operand_name_arena(expr, self.arena_store(), self.source)
                {
                    self.add_reference_if_bound(&name, ReferenceKind::ConditionalOperand, span);
                }
                self.visit_conditional_expr_arena_into(expr, flow, nested_regions);
            }
            ConditionalExprArena::Parenthesized { expr, .. } => {
                self.visit_conditional_expr_arena_into(expr, flow, nested_regions);
            }
            ConditionalExprArena::Word(word) | ConditionalExprArena::Regex(word) => {
                self.visit_word_arena_into(
                    self.arena_store().word(*word),
                    WordVisitKind::Conditional,
                    flow,
                    nested_regions,
                );
            }
            ConditionalExprArena::Pattern(pattern) => {
                self.visit_pattern_arena_into(
                    pattern,
                    WordVisitKind::Conditional,
                    flow,
                    nested_regions,
                );
            }
            ConditionalExprArena::VarRef(var_ref) => {
                self.visit_var_ref_reference_arena(
                    var_ref,
                    ReferenceKind::ConditionalOperand,
                    flow,
                    nested_regions,
                    var_ref.name_span,
                );
            }
        }
    }

    fn visit_conditional_arithmetic_operand_arena_into(
        &mut self,
        expression: &ConditionalExprArena,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if let Some((name, span)) =
            conditional_arithmetic_operand_name_arena(expression, self.arena_store(), self.source)
        {
            self.add_reference(&name, ReferenceKind::ArithmeticRead, span);
            return;
        }

        self.visit_conditional_expr_arena_into(expression, flow, nested_regions);
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

    fn visit_parameter_slice_arithmetic_expr_into(
        &mut self,
        expr: Option<&ArithmeticExprNode>,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let previous_kind = self.arithmetic_reference_kind;
        self.arithmetic_reference_kind = ReferenceKind::ParameterSliceArithmetic;
        self.visit_optional_arithmetic_expr_into(expr, flow, nested_regions);
        self.arithmetic_reference_kind = previous_kind;
    }

    fn visit_optional_arithmetic_expr_arena(
        &mut self,
        expr: Option<&ArithmeticExprArenaNode>,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_optional_arithmetic_expr_arena_into(expr, flow, &mut nested_regions);
        nested_regions
    }

    fn visit_optional_arithmetic_expr_arena_into(
        &mut self,
        expr: Option<&ArithmeticExprArenaNode>,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if let Some(expr) = expr {
            self.visit_arithmetic_expr_arena_into(expr, flow, nested_regions);
        }
    }

    fn visit_parameter_slice_arithmetic_expr_arena_into(
        &mut self,
        expr: Option<&ArithmeticExprArenaNode>,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let previous_kind = self.arithmetic_reference_kind;
        self.arithmetic_reference_kind = ReferenceKind::ParameterSliceArithmetic;
        self.visit_optional_arithmetic_expr_arena_into(expr, flow, nested_regions);
        self.arithmetic_reference_kind = previous_kind;
    }

    fn visit_arithmetic_expr_arena_into(
        &mut self,
        expr: &ArithmeticExprArenaNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExprArena::Number(_) => {}
            ArithmeticExprArena::Variable(name) => {
                self.add_reference(name, self.arithmetic_reference_kind, expr.span);
            }
            ArithmeticExprArena::Indexed { name, index } => {
                self.add_reference(
                    name,
                    self.arithmetic_reference_kind,
                    arithmetic_name_span(expr.span, name),
                );
                self.visit_arithmetic_index_arena_into(name, index, flow, nested_regions);
            }
            ArithmeticExprArena::ShellWord(word) => {
                let previous_kind =
                    if self.arithmetic_reference_kind == ReferenceKind::ParameterSliceArithmetic {
                        self.word_reference_kind_override
                            .replace(ReferenceKind::ParameterSliceArithmetic)
                    } else {
                        None
                    };
                self.visit_word_arena_into(
                    self.arena_store().word(*word),
                    WordVisitKind::Expansion,
                    flow,
                    nested_regions,
                );
                if self.arithmetic_reference_kind == ReferenceKind::ParameterSliceArithmetic {
                    self.word_reference_kind_override = previous_kind;
                }
            }
            ArithmeticExprArena::Parenthesized { expression } => {
                self.visit_arithmetic_expr_arena_into(expression, flow, nested_regions);
            }
            ArithmeticExprArena::Unary { op, expr: inner } => {
                if matches!(
                    op,
                    ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
                ) {
                    self.visit_arithmetic_update_arena_into(inner, flow, nested_regions);
                } else {
                    self.visit_arithmetic_expr_arena_into(inner, flow, nested_regions);
                }
            }
            ArithmeticExprArena::Postfix { expr: inner, .. } => {
                self.visit_arithmetic_update_arena_into(inner, flow, nested_regions);
            }
            ArithmeticExprArena::Binary { left, right, .. } => {
                self.visit_arithmetic_expr_arena_into(left, flow, nested_regions);
                self.visit_arithmetic_expr_arena_into(right, flow, nested_regions);
            }
            ArithmeticExprArena::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_arithmetic_expr_arena_into(condition, flow, nested_regions);
                self.visit_arithmetic_expr_arena_into(then_expr, flow, nested_regions);
                self.visit_arithmetic_expr_arena_into(else_expr, flow, nested_regions);
            }
            ArithmeticExprArena::Assignment { target, op, value } => {
                self.visit_arithmetic_assignment_arena_into(
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

    fn visit_arithmetic_index_arena_into(
        &mut self,
        owner_name: &Name,
        index: &ArithmeticExprArenaNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if self
            .arithmetic_index_uses_associative_word_semantics(owner_name, index.span.start.offset)
        {
            self.visit_associative_arithmetic_key_arena_into(index, flow, nested_regions);
            return;
        }

        self.visit_arithmetic_expr_arena_into(index, flow, nested_regions);
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
                self.add_reference(name, self.arithmetic_reference_kind, expr.span);
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.add_reference(
                    name,
                    self.arithmetic_reference_kind,
                    arithmetic_name_span(expr.span, name),
                );
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
            }
            ArithmeticExpr::ShellWord(word) => {
                let previous_kind =
                    if self.arithmetic_reference_kind == ReferenceKind::ParameterSliceArithmetic {
                        self.word_reference_kind_override
                            .replace(ReferenceKind::ParameterSliceArithmetic)
                    } else {
                        None
                    };
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
                if self.arithmetic_reference_kind == ReferenceKind::ParameterSliceArithmetic {
                    self.word_reference_kind_override = previous_kind;
                }
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
                let reference_id =
                    self.add_reference(name, self.arithmetic_reference_kind, expr.span);
                self.self_referential_assignment_refs.insert(reference_id);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    expr.span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: expr.span,
                        target_span: arithmetic_lvalue_span(
                            &ArithmeticLvalue::Variable(name.clone()),
                            expr.span,
                        ),
                    },
                    BindingAttributes::SELF_REFERENTIAL_READ,
                );
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
                let span = arithmetic_name_span(expr.span, name);
                let reference_id = self.add_reference(name, self.arithmetic_reference_kind, span);
                self.self_referential_assignment_refs.insert(reference_id);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: span,
                        target_span: arithmetic_lvalue_span(
                            &ArithmeticLvalue::Indexed {
                                name: name.clone(),
                                index: index.clone(),
                            },
                            expr.span,
                        ),
                    },
                    self.arithmetic_binding_attributes(
                        &ArithmeticLvalue::Indexed {
                            name: name.clone(),
                            index: index.clone(),
                        },
                        span.start.offset,
                    ) | BindingAttributes::SELF_REFERENTIAL_READ,
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
        let name = match target {
            ArithmeticLvalue::Variable(name) | ArithmeticLvalue::Indexed { name, .. } => name,
        };
        let name_span = arithmetic_name_span(target_span, name);
        let reference_start = self.references.len();
        self.visit_arithmetic_lvalue_indices_into(target, flow, nested_regions);
        let mut attributes = self.arithmetic_binding_attributes(target, target_span.start.offset);
        if !matches!(op, ArithmeticAssignOp::Assign) {
            self.add_reference(name, self.arithmetic_reference_kind, name_span);
        }
        self.visit_arithmetic_expr_into(value, flow, nested_regions);
        let self_referential_refs =
            self.newly_added_reference_ids_reading_name(name, reference_start);
        if !self_referential_refs.is_empty() {
            attributes |= BindingAttributes::SELF_REFERENTIAL_READ;
            self.self_referential_assignment_refs
                .extend(self_referential_refs);
        }
        self.add_binding(
            name,
            BindingKind::ArithmeticAssignment,
            self.current_scope(),
            name_span,
            BindingOrigin::ArithmeticAssignment {
                definition_span: name_span,
                target_span: arithmetic_lvalue_span(target, target_span),
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

    fn arithmetic_binding_attributes_arena(
        &self,
        target: &ArithmeticLvalueArena,
        target_offset: usize,
    ) -> BindingAttributes {
        let mut attributes = match target {
            ArithmeticLvalueArena::Variable(_) => BindingAttributes::empty(),
            ArithmeticLvalueArena::Indexed { .. } => BindingAttributes::ARRAY,
        };

        if let ArithmeticLvalueArena::Indexed { name, .. } = target
            && self.visible_binding_is_assoc(name, target_offset)
        {
            attributes |= BindingAttributes::ASSOC;
        }

        attributes
    }

    fn visit_associative_arithmetic_key_arena_into(
        &mut self,
        expr: &ArithmeticExprArenaNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExprArena::Number(_) | ArithmeticExprArena::Variable(_) => {}
            ArithmeticExprArena::Indexed { index, .. } => {
                self.visit_associative_arithmetic_key_arena_into(index, flow, nested_regions);
            }
            ArithmeticExprArena::ShellWord(word) => {
                self.visit_word_arena_into(
                    self.arena_store().word(*word),
                    WordVisitKind::Expansion,
                    flow,
                    nested_regions,
                );
            }
            ArithmeticExprArena::Parenthesized { expression } => {
                self.visit_associative_arithmetic_key_arena_into(expression, flow, nested_regions);
            }
            ArithmeticExprArena::Unary { expr: inner, .. }
            | ArithmeticExprArena::Postfix { expr: inner, .. } => {
                self.visit_associative_arithmetic_key_arena_into(inner, flow, nested_regions);
            }
            ArithmeticExprArena::Binary { left, right, .. } => {
                self.visit_associative_arithmetic_key_arena_into(left, flow, nested_regions);
                self.visit_associative_arithmetic_key_arena_into(right, flow, nested_regions);
            }
            ArithmeticExprArena::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_associative_arithmetic_key_arena_into(condition, flow, nested_regions);
                self.visit_associative_arithmetic_key_arena_into(then_expr, flow, nested_regions);
                self.visit_associative_arithmetic_key_arena_into(else_expr, flow, nested_regions);
            }
            ArithmeticExprArena::Assignment { target, value, .. } => {
                self.visit_associative_arithmetic_lvalue_arena_into(target, flow, nested_regions);
                self.visit_associative_arithmetic_key_arena_into(value, flow, nested_regions);
            }
        }
    }

    fn visit_associative_arithmetic_lvalue_arena_into(
        &mut self,
        target: &ArithmeticLvalueArena,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match target {
            ArithmeticLvalueArena::Variable(_) => {}
            ArithmeticLvalueArena::Indexed { index, .. } => {
                self.visit_associative_arithmetic_key_arena_into(index, flow, nested_regions);
            }
        }
    }

    fn visit_arithmetic_update_arena_into(
        &mut self,
        expr: &ArithmeticExprArenaNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExprArena::Variable(name) => {
                let reference_id =
                    self.add_reference(name, self.arithmetic_reference_kind, expr.span);
                self.self_referential_assignment_refs.insert(reference_id);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    expr.span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: expr.span,
                        target_span: arithmetic_lvalue_span_arena(
                            &ArithmeticLvalueArena::Variable(name.clone()),
                            expr.span,
                        ),
                    },
                    BindingAttributes::SELF_REFERENTIAL_READ,
                );
            }
            ArithmeticExprArena::Indexed { name, index } => {
                self.visit_arithmetic_index_arena_into(name, index, flow, nested_regions);
                let span = arithmetic_name_span(expr.span, name);
                let reference_id = self.add_reference(name, self.arithmetic_reference_kind, span);
                self.self_referential_assignment_refs.insert(reference_id);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: span,
                        target_span: arithmetic_lvalue_span_arena(
                            &ArithmeticLvalueArena::Indexed {
                                name: name.clone(),
                                index: index.clone(),
                            },
                            expr.span,
                        ),
                    },
                    self.arithmetic_binding_attributes_arena(
                        &ArithmeticLvalueArena::Indexed {
                            name: name.clone(),
                            index: index.clone(),
                        },
                        span.start.offset,
                    ) | BindingAttributes::SELF_REFERENTIAL_READ,
                );
            }
            _ => {}
        }
    }

    fn visit_arithmetic_assignment_arena_into(
        &mut self,
        target: &ArithmeticLvalueArena,
        target_span: Span,
        op: ArithmeticAssignOp,
        value: &ArithmeticExprArenaNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let name = match target {
            ArithmeticLvalueArena::Variable(name) | ArithmeticLvalueArena::Indexed { name, .. } => {
                name
            }
        };
        let name_span = arithmetic_name_span(target_span, name);
        let reference_start = self.references.len();
        self.visit_arithmetic_lvalue_indices_arena_into(target, flow, nested_regions);
        let mut attributes =
            self.arithmetic_binding_attributes_arena(target, target_span.start.offset);
        if !matches!(op, ArithmeticAssignOp::Assign) {
            self.add_reference(name, self.arithmetic_reference_kind, name_span);
        }
        self.visit_arithmetic_expr_arena_into(value, flow, nested_regions);
        let self_referential_refs =
            self.newly_added_reference_ids_reading_name(name, reference_start);
        if !self_referential_refs.is_empty() {
            attributes |= BindingAttributes::SELF_REFERENTIAL_READ;
            self.self_referential_assignment_refs
                .extend(self_referential_refs);
        }
        self.add_binding(
            name,
            BindingKind::ArithmeticAssignment,
            self.current_scope(),
            name_span,
            BindingOrigin::ArithmeticAssignment {
                definition_span: name_span,
                target_span: arithmetic_lvalue_span_arena(target, target_span),
            },
            attributes,
        );
    }

    fn visit_arithmetic_lvalue_indices_arena_into(
        &mut self,
        target: &ArithmeticLvalueArena,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match target {
            ArithmeticLvalueArena::Variable(_) => {}
            ArithmeticLvalueArena::Indexed { name, index } => {
                self.visit_arithmetic_index_arena_into(name, index, flow, nested_regions);
            }
        }
    }

    fn classify_special_simple_command(
        &mut self,
        name: &Name,
        normalized: &NormalizedCommand<'_>,
        command_span: Span,
        flow: FlowState,
    ) {
        let args = normalized.body_args();
        let name_span = normalized.body_word_span().unwrap_or(command_span);
        match name.as_str() {
            "read" => {
                let read_assigns_array = read_assigns_array(args, self.source);
                for (target_index, (argument, span)) in
                    iter_read_targets(args, self.source).into_iter().enumerate()
                {
                    let target_attributes = if read_assigns_array && target_index == 0 {
                        BindingAttributes::ARRAY
                    } else {
                        BindingAttributes::empty()
                    };
                    self.add_binding(
                        &argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Read,
                        },
                        target_attributes,
                    );
                }
                for implicit_read in self.runtime.implicit_reads_for_simple_command(name) {
                    let implicit_name = Name::from(*implicit_read);
                    self.add_reference_if_bound(
                        &implicit_name,
                        ReferenceKind::ImplicitRead,
                        command_span,
                    );
                }
            }
            "mapfile" | "readarray" => match mapfile_target(args, self.source) {
                Some(MapfileTarget::Explicit(argument, span)) => {
                    self.add_binding(
                        &argument,
                        BindingKind::MapfileTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Mapfile,
                        },
                        BindingAttributes::ARRAY,
                    );
                }
                Some(MapfileTarget::Implicit) => {
                    self.add_binding(
                        &Name::from("MAPFILE"),
                        BindingKind::MapfileTarget,
                        self.current_scope(),
                        name_span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: name_span,
                            kind: BuiltinBindingTargetKind::Mapfile,
                        },
                        BindingAttributes::ARRAY,
                    );
                }
                None => {}
            },
            "printf" => {
                if let Some((argument, span)) = printf_v_target(args, self.source) {
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
                if let Some((argument, span)) = getopts_target(args, self.source) {
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
            "let" => self.record_let_arithmetic_assignment_targets(args),
            "eval" => self.record_eval_argument_references(args),
            "trap" => self.record_trap_action_references(args),
            "source" | "." => {
                if normalized.wrappers.is_empty()
                    && let Some(argument) = args.first().copied()
                {
                    let source_span = self.command_stack.last().copied().unwrap_or(command_span);
                    let kind = self.classify_source_ref(command_span.line(), argument);
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
            "unset" => self.record_unset_variable_targets(args, flow),
            "export" | "local" | "declare" | "typeset" | "readonly" => {
                self.visit_simple_declaration_command(name.as_str(), args, command_span, flow);
            }
            _ if name.as_str().starts_with("DEFINE_") => {
                self.visit_command_defined_variable(args);
            }
            _ => {}
        }
    }

    fn classify_special_simple_command_arena(
        &mut self,
        name: &Name,
        normalized: &ArenaNormalizedCommand<'_>,
        flow: FlowState,
    ) {
        let args = normalized.body_args();
        let command_span = normalized.command_span;
        let name_span = normalized.body_word_span().unwrap_or(command_span);
        match name.as_str() {
            "read" => {
                let read_assigns_array =
                    read_assigns_array_arena(args, self.arena_store(), self.source);
                for (target_index, (argument, span)) in
                    iter_read_targets_arena(args, self.arena_store(), self.source)
                        .into_iter()
                        .enumerate()
                {
                    let target_attributes = if read_assigns_array && target_index == 0 {
                        BindingAttributes::ARRAY
                    } else {
                        BindingAttributes::empty()
                    };
                    self.add_binding(
                        &argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Read,
                        },
                        target_attributes,
                    );
                }
                let implicit_name = Name::from("IFS");
                self.add_reference_if_bound(
                    &implicit_name,
                    ReferenceKind::ImplicitRead,
                    command_span,
                );
            }
            "mapfile" | "readarray" => {
                match mapfile_target_arena(args, self.arena_store(), self.source) {
                    Some(MapfileTarget::Explicit(argument, span)) => {
                        self.add_binding(
                            &argument,
                            BindingKind::MapfileTarget,
                            self.current_scope(),
                            span,
                            BindingOrigin::BuiltinTarget {
                                definition_span: span,
                                kind: BuiltinBindingTargetKind::Mapfile,
                            },
                            BindingAttributes::ARRAY,
                        );
                    }
                    Some(MapfileTarget::Implicit) => {
                        self.add_binding(
                            &Name::from("MAPFILE"),
                            BindingKind::MapfileTarget,
                            self.current_scope(),
                            name_span,
                            BindingOrigin::BuiltinTarget {
                                definition_span: name_span,
                                kind: BuiltinBindingTargetKind::Mapfile,
                            },
                            BindingAttributes::ARRAY,
                        );
                    }
                    None => {}
                }
            }
            "printf" => {
                if let Some((argument, span)) =
                    printf_v_target_arena(args, self.arena_store(), self.source)
                {
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
                if let Some((argument, span)) =
                    getopts_target_arena(args, self.arena_store(), self.source)
                {
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
            "let" => self.record_let_arithmetic_assignment_targets_arena(args),
            "eval" => self.record_eval_argument_references_arena(args),
            "trap" => self.record_trap_action_references_arena(args),
            "source" | "." => {
                if normalized.wrappers.is_empty()
                    && let Some(argument) = args.first().copied()
                {
                    let word = self.arena_store().word(argument);
                    let source_span = self.command_stack.last().copied().unwrap_or(command_span);
                    let kind = self.classify_source_ref_arena(command_span.line(), word);
                    self.source_refs.push(SourceRef {
                        diagnostic_class: classify_source_ref_diagnostic_class_arena(
                            word,
                            self.source,
                            &kind,
                        ),
                        kind,
                        span: source_span,
                        path_span: word.span(),
                        resolution: SourceRefResolution::Unchecked,
                        explicitly_provided: false,
                    });
                }
            }
            "unset" => self.record_unset_variable_targets_arena(args, flow),
            "export" | "local" | "declare" | "typeset" | "readonly" => {
                self.visit_simple_declaration_command_arena(
                    name.as_str(),
                    args,
                    command_span,
                    flow,
                );
            }
            _ if name.as_str().starts_with("DEFINE_") => {
                self.visit_command_defined_variable_arena(args);
            }
            _ => {}
        }
        let _ = name_span;
    }

    fn record_trap_action_references(&mut self, args: &[&Word]) {
        let Some(argument) = trap_action_argument(args, self.source) else {
            return;
        };

        let mut seen = FxHashSet::default();
        for name in trap_action_reference_names(argument, self.source) {
            if seen.insert(name.clone()) {
                self.add_reference(&name, ReferenceKind::TrapAction, argument.span);
            }
        }
    }

    fn record_trap_action_references_arena(&mut self, args: &[WordId]) {
        let Some(argument) = trap_action_argument_arena(args, self.arena_store(), self.source)
        else {
            return;
        };
        let argument = self.arena_store().word(argument);

        let mut seen = FxHashSet::default();
        for name in trap_action_reference_names_arena(argument, self.source) {
            if seen.insert(name.clone()) {
                self.add_reference(&name, ReferenceKind::TrapAction, argument.span());
            }
        }
    }

    fn record_let_arithmetic_assignment_targets(&mut self, args: &[&Word]) {
        for argument in args {
            let Some((name, span)) = let_arithmetic_assignment_target(argument, self.source) else {
                continue;
            };
            self.add_binding(
                &name,
                BindingKind::ArithmeticAssignment,
                self.current_scope(),
                span,
                BindingOrigin::ArithmeticAssignment {
                    definition_span: span,
                    target_span: span,
                },
                BindingAttributes::empty(),
            );
        }
    }

    fn record_let_arithmetic_assignment_targets_arena(&mut self, args: &[WordId]) {
        for argument in args {
            let word = self.arena_store().word(*argument);
            let Some((name, span)) = let_arithmetic_assignment_target_arena(word, self.source)
            else {
                continue;
            };
            self.add_binding(
                &name,
                BindingKind::ArithmeticAssignment,
                self.current_scope(),
                span,
                BindingOrigin::ArithmeticAssignment {
                    definition_span: span,
                    target_span: span,
                },
                BindingAttributes::empty(),
            );
        }
    }

    fn visit_simple_declaration_command(
        &mut self,
        command_name: &str,
        args: &[&Word],
        command_span: Span,
        flow: FlowState,
    ) {
        let Some(builtin) = declaration_builtin_name(command_name) else {
            return;
        };

        let mut flags = FxHashSet::default();
        let mut global_flag_enabled = false;
        let mut name_operands_are_function_names = false;
        let mut parsing_options = true;
        let mut operands = Vec::new();

        for argument in args.iter().copied() {
            if parsing_options {
                if let Some(text) = static_word_text(argument, self.source) {
                    if text == "--" {
                        parsing_options = false;
                        continue;
                    }

                    if simple_declaration_option_word(&text) {
                        update_simple_declaration_flags(
                            &text,
                            &mut flags,
                            &mut global_flag_enabled,
                            &mut name_operands_are_function_names,
                        );
                        operands.push(simple_declaration_flag_operand(argument, text.as_ref()));
                        continue;
                    }
                }

                parsing_options = false;
            }

            if name_operands_are_function_names {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument.span,
                });
                continue;
            }

            let assignment_text = declaration_assignment_text(argument, self.source);
            if let Some(assignment) =
                parse_simple_declaration_assignment(argument, assignment_text.as_ref(), self.source)
            {
                let (scope, mut attributes) = self.simple_declaration_scope_and_attributes(
                    builtin,
                    &flags,
                    global_flag_enabled,
                    flow,
                );
                attributes |= BindingAttributes::DECLARATION_INITIALIZED;
                if assignment.array_like {
                    attributes |= BindingAttributes::ARRAY;
                }
                if flags.contains(&'p') {
                    attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
                }
                let kind = if attributes.contains(BindingAttributes::NAMEREF) {
                    BindingKind::Nameref
                } else {
                    BindingKind::Declaration(builtin)
                };
                let origin = BindingOrigin::Assignment {
                    definition_span: assignment.target_span,
                    value: assignment.value_origin,
                };
                self.add_binding(
                    &assignment.name,
                    kind,
                    scope,
                    assignment.name_span,
                    origin,
                    attributes,
                );
                operands.push(DeclarationOperand::Assignment {
                    name: assignment.name,
                    name_span: assignment.name_span,
                    value_span: assignment.value_span,
                    append: assignment.append,
                });
                continue;
            }

            if static_word_text(argument, self.source).is_none() {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument.span,
                });
                continue;
            }

            if let Some((name, span)) = named_target_word(argument, self.source) {
                self.visit_simple_name_only_declaration_operand(
                    builtin,
                    &flags,
                    global_flag_enabled,
                    flow,
                    &name,
                    span,
                );
                operands.push(DeclarationOperand::Name { name, span });
            } else {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument.span,
                });
            }
        }

        self.declarations.push(Declaration {
            builtin,
            span: command_span,
            operands,
        });
    }

    fn visit_simple_declaration_command_arena(
        &mut self,
        command_name: &str,
        args: &[WordId],
        command_span: Span,
        flow: FlowState,
    ) {
        let Some(builtin) = declaration_builtin_name(command_name) else {
            return;
        };

        let mut flags = FxHashSet::default();
        let mut global_flag_enabled = false;
        let mut name_operands_are_function_names = false;
        let mut parsing_options = true;
        let mut operands = Vec::new();

        for argument_id in args.iter().copied() {
            let argument = self.arena_store().word(argument_id);
            if parsing_options {
                if let Some(text) = static_word_text_arena(argument, self.source) {
                    if text == "--" {
                        parsing_options = false;
                        continue;
                    }

                    if simple_declaration_option_word(&text) {
                        update_simple_declaration_flags(
                            &text,
                            &mut flags,
                            &mut global_flag_enabled,
                            &mut name_operands_are_function_names,
                        );
                        operands.push(simple_declaration_flag_operand_arena(
                            argument,
                            text.as_ref(),
                        ));
                        continue;
                    }
                }

                parsing_options = false;
            }

            if name_operands_are_function_names {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument.span(),
                });
                continue;
            }

            let text = static_word_text_arena(argument, self.source)
                .unwrap_or_else(|| Cow::Borrowed(argument.span().slice(self.source)));

            if let Some(parsed) =
                parse_simple_declaration_assignment_from_text(argument.span(), &text, self.source)
            {
                let (scope, mut attributes) = self.simple_declaration_scope_and_attributes(
                    builtin,
                    &flags,
                    global_flag_enabled,
                    flow,
                );
                attributes |= BindingAttributes::DECLARATION_INITIALIZED;
                if parsed.array_like {
                    attributes |= BindingAttributes::ARRAY;
                }
                if flags.contains(&'p') {
                    attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
                }
                let kind = if attributes.contains(BindingAttributes::NAMEREF) {
                    BindingKind::Nameref
                } else {
                    BindingKind::Declaration(builtin)
                };
                self.add_binding(
                    &parsed.name,
                    kind,
                    scope,
                    parsed.name_span,
                    BindingOrigin::Assignment {
                        definition_span: parsed.target_span,
                        value: parsed.value_origin,
                    },
                    attributes,
                );
                operands.push(DeclarationOperand::Assignment {
                    name: parsed.name,
                    name_span: parsed.name_span,
                    value_span: parsed.value_span,
                    append: parsed.append,
                });
                continue;
            }

            if let Some((name, span)) = named_target_word_arena(argument, self.source) {
                self.visit_simple_name_only_declaration_operand(
                    builtin,
                    &flags,
                    global_flag_enabled,
                    flow,
                    &name,
                    span,
                );
                operands.push(DeclarationOperand::Name { name, span });
            } else {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument.span(),
                });
            }
        }

        self.declarations.push(Declaration {
            builtin,
            span: command_span,
            operands,
        });
    }

    fn visit_command_defined_variable(&mut self, args: &[&Word]) {
        let Some((flag_name, span)) = args
            .first()
            .copied()
            .and_then(|word| named_target_word(word, self.source))
        else {
            return;
        };
        let generated = Name::from(format!("FLAGS_{}", flag_name.as_str()));
        self.add_binding(
            &generated,
            BindingKind::Declaration(DeclarationBuiltin::Declare),
            self.current_scope(),
            span,
            BindingOrigin::Declaration {
                definition_span: span,
            },
            BindingAttributes::empty(),
        );
    }

    fn visit_command_defined_variable_arena(&mut self, args: &[WordId]) {
        let Some((flag_name, span)) = args
            .first()
            .copied()
            .and_then(|word| named_target_word_arena(self.arena_store().word(word), self.source))
        else {
            return;
        };
        let generated = Name::from(format!("FLAGS_{}", flag_name.as_str()));
        self.add_binding(
            &generated,
            BindingKind::Declaration(DeclarationBuiltin::Declare),
            self.current_scope(),
            span,
            BindingOrigin::Declaration {
                definition_span: span,
            },
            BindingAttributes::empty(),
        );
    }

    fn record_eval_argument_references(&mut self, args: &[&Word]) {
        for argument in args.iter().copied() {
            for (name, span) in eval_argument_reference_names(argument, self.source) {
                self.add_reference_if_bound(&name, ReferenceKind::ImplicitRead, span);
            }
        }
    }

    fn record_eval_argument_references_arena(&mut self, args: &[WordId]) {
        for argument in args.iter().copied() {
            for (name, span) in
                eval_argument_reference_names_arena(self.arena_store().word(argument), self.source)
            {
                self.add_reference_if_bound(&name, ReferenceKind::ImplicitRead, span);
            }
        }
    }

    fn record_unset_variable_targets(&mut self, args: &[&Word], flow: FlowState) {
        if flow.conditionally_executed {
            return;
        }

        let mut function_flag_seen = false;
        let mut variable_flag_seen = false;
        let mut nameref_mode = false;
        let mut parsing_options = true;

        for argument in args.iter().copied() {
            let Some(text) = static_word_text(argument, self.source) else {
                if parsing_options {
                    return;
                }
                parsing_options = false;
                continue;
            };

            if parsing_options {
                if text == "--" {
                    parsing_options = false;
                    continue;
                }

                if text.starts_with('-') && text != "-" {
                    let flags = text.trim_start_matches('-');
                    if !unset_flags_are_valid(flags) {
                        return;
                    }
                    for flag in flags.chars() {
                        match flag {
                            'f' => {
                                if variable_flag_seen {
                                    return;
                                }
                                function_flag_seen = true;
                            }
                            'v' => {
                                if function_flag_seen {
                                    return;
                                }
                                variable_flag_seen = true;
                            }
                            'n' => {
                                nameref_mode = true;
                            }
                            _ => unreachable!("invalid unset flag already filtered"),
                        }
                    }
                    continue;
                }

                parsing_options = false;
            }

            if function_flag_seen || !is_name(&text) {
                continue;
            }

            if nameref_mode {
                let name = Name::from(text.as_ref());
                let Some(binding_id) =
                    self.resolve_reference(&name, self.current_scope(), argument.span.start.offset)
                else {
                    continue;
                };
                let binding = &self.bindings[binding_id.index()];
                if !binding.attributes.contains(BindingAttributes::NAMEREF)
                    && !matches!(binding.kind, BindingKind::Nameref)
                {
                    continue;
                }
            }

            self.cleared_variables
                .entry((self.current_scope(), Name::from(text.as_ref())))
                .or_default()
                .push(argument.span.start.offset);
        }
    }

    fn record_unset_variable_targets_arena(&mut self, args: &[WordId], flow: FlowState) {
        if flow.conditionally_executed {
            return;
        }

        let mut function_flag_seen = false;
        let mut variable_flag_seen = false;
        let mut nameref_mode = false;
        let mut parsing_options = true;

        for argument_id in args.iter().copied() {
            let argument = self.arena_store().word(argument_id);
            let Some(text) = static_word_text_arena(argument, self.source) else {
                if parsing_options {
                    return;
                }
                parsing_options = false;
                continue;
            };

            if parsing_options {
                if text == "--" {
                    parsing_options = false;
                    continue;
                }

                if text.starts_with('-') && text != "-" {
                    let flags = text.trim_start_matches('-');
                    if !unset_flags_are_valid(flags) {
                        return;
                    }
                    for flag in flags.chars() {
                        match flag {
                            'f' => {
                                if variable_flag_seen {
                                    return;
                                }
                                function_flag_seen = true;
                            }
                            'v' => {
                                if function_flag_seen {
                                    return;
                                }
                                variable_flag_seen = true;
                            }
                            'n' => {
                                nameref_mode = true;
                            }
                            _ => unreachable!("invalid unset flag already filtered"),
                        }
                    }
                    continue;
                }

                parsing_options = false;
            }

            if function_flag_seen || !is_name(&text) {
                continue;
            }

            if nameref_mode {
                let name = Name::from(text.as_ref());
                let Some(binding_id) = self.resolve_reference(
                    &name,
                    self.current_scope(),
                    argument.span().start.offset,
                ) else {
                    continue;
                };
                let binding = &self.bindings[binding_id.index()];
                if !binding.attributes.contains(BindingAttributes::NAMEREF)
                    && !matches!(binding.kind, BindingKind::Nameref)
                {
                    continue;
                }
            }

            self.cleared_variables
                .entry((self.current_scope(), Name::from(text.as_ref())))
                .or_default()
                .push(argument.span().start.offset);
        }
    }

    fn classify_source_ref(&self, line: usize, word: &Word) -> SourceRefKind {
        if let Some(directive) = self.source_directive_for_line(line) {
            return directive;
        }

        if let Some(text) = static_word_text(word, self.source) {
            return SourceRefKind::Literal(text.into_owned());
        }

        classify_dynamic_source_word(word, self.source)
    }

    fn classify_source_ref_arena(
        &self,
        line: usize,
        word: shuck_ast::WordView<'_>,
    ) -> SourceRefKind {
        if let Some(directive) = self.source_directive_for_line(line) {
            return directive;
        }

        if let Some(text) = static_word_text_arena(word, self.source) {
            return SourceRefKind::Literal(text.into_owned());
        }

        classify_dynamic_source_word_arena(word, self.source)
    }

    fn source_directive_for_line(&self, line: usize) -> Option<SourceRefKind> {
        if let Some(directive) = self.source_directives.get(&line) {
            return Some(directive.kind.clone());
        }

        if let Some(previous) = line.checked_sub(1)
            && let Some(directive) = self.source_directives.get(&previous)
            && directive.own_line
        {
            return Some(directive.kind.clone());
        }

        let directive = self
            .source_directives
            .range(..line)
            .rev()
            .find(|(_, directive)| directive.own_line)
            .map(|(_, directive)| directive)?;

        match directive.kind {
            SourceRefKind::DirectiveDevNull => Some(SourceRefKind::DirectiveDevNull),
            _ => None,
        }
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

    fn simple_declaration_scope_and_attributes(
        &self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
        flow: FlowState,
    ) -> (ScopeId, BindingAttributes) {
        let (scope, mut attributes) =
            self.declaration_scope_and_attributes(builtin, flags, global_flag_enabled);
        if flow.in_subshell && attributes.contains(BindingAttributes::LOCAL) {
            attributes.remove(BindingAttributes::LOCAL);
            return (self.current_scope(), attributes);
        }

        (scope, attributes)
    }

    fn visit_simple_name_only_declaration_operand(
        &mut self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
        flow: FlowState,
        name: &Name,
        span: Span,
    ) {
        if flow.in_subshell {
            let (scope, attributes) = self.simple_declaration_scope_and_attributes(
                builtin,
                flags,
                global_flag_enabled,
                flow,
            );
            self.add_binding(
                name,
                BindingKind::Declaration(builtin),
                scope,
                span,
                BindingOrigin::Declaration {
                    definition_span: span,
                },
                attributes,
            );
            return;
        }

        self.visit_name_only_declaration_operand(builtin, flags, global_flag_enabled, name, span);
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

        let reuse_existing = existing.is_some_and(|existing| {
            let existing_binding = &self.bindings[existing.index()];

            !local_like
                || (existing_binding.scope == scope
                    && self.has_uncleared_local_binding_in_scope(name, scope, span.start.offset))
        });

        if reuse_existing {
            let existing = existing.expect("existing binding already checked");
            self.add_reference(name, ReferenceKind::DeclarationName, span);
            self.bindings[existing.index()].attributes |= attributes;
            return;
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

    fn binding_was_cleared_in_scope_after(
        &self,
        name: &Name,
        scope: ScopeId,
        binding_offset: usize,
    ) -> bool {
        self.cleared_variables
            .get(&(scope, name.clone()))
            .is_some_and(|cleared_offsets| {
                cleared_offsets
                    .iter()
                    .any(|cleared_offset| *cleared_offset > binding_offset)
            })
    }

    fn binding_was_cleared_in_scope_between(
        &self,
        name: &Name,
        scope: ScopeId,
        binding_offset: usize,
        lookup_offset: usize,
    ) -> bool {
        self.cleared_variables
            .get(&(scope, name.clone()))
            .is_some_and(|cleared_offsets| {
                cleared_offsets.iter().any(|cleared_offset| {
                    *cleared_offset > binding_offset && *cleared_offset < lookup_offset
                })
            })
    }

    fn binding_was_cleared_before_lookup(
        &self,
        binding: &Binding,
        lookup_scope: ScopeId,
        lookup_offset: usize,
    ) -> bool {
        for scope in ancestor_scopes(&self.scopes, lookup_scope) {
            let clear_lower_bound = if scope == binding.scope {
                binding.span.start.offset
            } else {
                0
            };
            let clear_upper_bound = if self.completed_scopes.contains(&scope) {
                usize::MAX
            } else {
                lookup_offset
            };
            if self.binding_was_cleared_in_scope_between(
                &binding.name,
                scope,
                clear_lower_bound,
                clear_upper_bound,
            ) {
                return true;
            }
            if scope == binding.scope {
                break;
            }
        }
        false
    }

    fn has_uncleared_local_binding_in_scope(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> bool {
        self.scopes[scope.index()]
            .bindings
            .get(name)
            .and_then(|bindings| {
                bindings.iter().rev().copied().find(|binding_id| {
                    let binding = &self.bindings[binding_id.index()];
                    binding.span.start.offset <= offset
                        && binding.attributes.contains(BindingAttributes::LOCAL)
                })
            })
            .is_some_and(|binding_id| {
                !self.binding_was_cleared_in_scope_after(
                    name,
                    scope,
                    self.bindings[binding_id.index()].span.start.offset,
                )
            })
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
        let span = self.normalize_reference_span(name, kind, span);
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
        if self.defaulting_parameter_operand_depth > 0 {
            self.defaulting_parameter_operand_refs.insert(id);
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

    fn normalize_reference_span(&self, name: &Name, kind: ReferenceKind, span: Span) -> Span {
        if span.end.offset >= self.source.len() {
            return span;
        }

        let syntax = span.slice(self.source);
        if matches!(kind, ReferenceKind::Expansion)
            && unbraced_parameter_reference_matches(syntax, name.as_str())
        {
            return span;
        }
        if !reference_kind_uses_braced_parameter_syntax(kind) {
            return span;
        }
        if let Some(start_rel) = syntax.find('$') {
            let candidate = &syntax[start_rel..];
            if unbraced_parameter_reference_matches(candidate, name.as_str()) {
                let start_offset = span.start.offset + start_rel;
                let end_offset = start_offset + '$'.len_utf8() + name.as_str().len();
                if let Some((start, end)) =
                    self.source_positions_for_offsets(start_offset, end_offset)
                    && start.offset < end.offset
                {
                    return Span::from_positions(start, end);
                }
            }
        }
        let Some(start_rel) = syntax.find("${") else {
            return self
                .recover_unbraced_reference_span(name, span)
                .or_else(|| self.recover_braced_reference_span(name, span))
                .unwrap_or(span);
        };
        if self.source.as_bytes().get(span.end.offset) != Some(&b'}') {
            return self
                .recover_braced_reference_span(name, span)
                .unwrap_or(span);
        }

        let start_offset = span.start.offset + start_rel;
        let end_offset = span.end.offset + '}'.len_utf8();
        let Some((start, end)) = self.source_positions_for_offsets(start_offset, end_offset) else {
            return span;
        };
        if start.offset < end.offset {
            Span::from_positions(start, end)
        } else {
            span
        }
    }

    fn recover_braced_reference_span(&self, name: &Name, span: Span) -> Option<Span> {
        if name.is_empty() || span.start.offset >= self.source.len() {
            return None;
        }

        let name = name.as_str();
        let search_end = self
            .source
            .get(span.start.offset..)?
            .find('\n')
            .map(|relative| span.start.offset + relative)
            .unwrap_or(self.source.len());
        let search = self.source.get(span.start.offset..search_end)?;
        let needle = format!("${{{name}");
        for (start_rel, _) in search.match_indices(&needle) {
            let start_offset = span.start.offset + start_rel;
            if braced_parameter_start_matches(self.source, start_offset, name)
                && let Some(end_offset) =
                    braced_parameter_end_offset(self.source, start_offset, search_end)
                && let Some((start, end)) =
                    self.source_positions_for_offsets(start_offset, end_offset)
                && start.offset < end.offset
            {
                return Some(Span::from_positions(start, end));
            }
        }

        self.recover_braced_reference_span_on_line(&needle, span)
    }

    fn recover_unbraced_reference_span(&self, name: &Name, span: Span) -> Option<Span> {
        if name.is_empty() || span.start.offset >= self.source.len() {
            return None;
        }

        let (line_start_offset, line) = source_line(self.source, span.start.line)?;
        let name = name.as_str();
        let mut best = None::<(usize, usize, usize)>;
        for (start, _) in line.match_indices('$') {
            if !unbraced_parameter_start_matches(line, start, name) {
                continue;
            }
            let end = start + '$'.len_utf8() + name.len();
            let column = line.get(..start)?.chars().count() + 1;
            let distance = column.abs_diff(span.start.column);
            if best
                .as_ref()
                .is_none_or(|(_, _, best_distance)| distance < *best_distance)
            {
                best = Some((start, end, distance));
            }
        }

        let (start, end, _) = best?;
        let start_offset = line_start_offset + start;
        let end_offset = line_start_offset + end;
        let (start, end) = self.source_positions_for_offsets(start_offset, end_offset)?;
        (start.offset < end.offset).then(|| Span::from_positions(start, end))
    }

    fn recover_braced_reference_span_on_line(&self, needle: &str, span: Span) -> Option<Span> {
        let (line_start_offset, line) = source_line(self.source, span.start.line)?;
        let mut best = None::<(usize, usize, usize)>;
        let name = needle.strip_prefix("${").unwrap_or(needle);
        for (start, _) in line.match_indices(needle) {
            if !braced_parameter_start_matches(line, start, name) {
                continue;
            }
            let Some(end) = braced_parameter_end_offset(line, start, line.len()) else {
                continue;
            };
            let column = line.get(..start)?.chars().count() + 1;
            let distance = column.abs_diff(span.start.column);
            if best
                .as_ref()
                .is_none_or(|(_, _, best_distance)| distance < *best_distance)
            {
                best = Some((start, end, distance));
            }
        }

        let (start, end, _) = best?;
        let start_offset = line_start_offset + start;
        let end_offset = line_start_offset + end;
        let (start, end) = self.source_positions_for_offsets(start_offset, end_offset)?;
        (start.offset < end.offset).then(|| Span::from_positions(start, end))
    }

    fn source_positions_for_offsets(
        &self,
        start: usize,
        end: usize,
    ) -> Option<(Position, Position)> {
        if start > end || end > self.source.len() {
            return None;
        }
        Some((
            self.source_position_at_offset(start)?,
            self.source_position_at_offset(end)?,
        ))
    }

    fn source_position_at_offset(&self, offset: usize) -> Option<Position> {
        source_position_at_offset(self.source, &self.line_start_offsets, offset)
    }

    fn add_parameter_default_binding(&mut self, reference: &VarRef) {
        let mut attributes = binding_attributes_for_var_ref(reference);
        if reference.subscript.is_some()
            && !attributes.contains(BindingAttributes::ASSOC)
            && self
                .resolve_reference(
                    &reference.name,
                    self.current_scope(),
                    reference.name_span.start.offset,
                )
                .map(|binding_id| {
                    let binding = &self.bindings[binding_id.index()];
                    binding.attributes.contains(BindingAttributes::ASSOC)
                        && !self.binding_was_cleared_before_lookup(
                            binding,
                            self.current_scope(),
                            reference.name_span.start.offset,
                        )
                })
                .unwrap_or(false)
        {
            attributes |= BindingAttributes::ARRAY | BindingAttributes::ASSOC;
        }

        self.add_binding(
            &reference.name,
            BindingKind::ParameterDefaultAssignment,
            self.current_scope(),
            reference.span,
            BindingOrigin::ParameterDefaultAssignment {
                definition_span: reference.span,
            },
            attributes,
        );
    }

    fn add_parameter_default_binding_arena(&mut self, reference: &VarRefNode) {
        let mut attributes = binding_attributes_for_var_ref_arena(reference);
        if reference.subscript.is_some()
            && !attributes.contains(BindingAttributes::ASSOC)
            && self
                .resolve_reference(
                    &reference.name,
                    self.current_scope(),
                    reference.name_span.start.offset,
                )
                .map(|binding_id| {
                    let binding = &self.bindings[binding_id.index()];
                    binding.attributes.contains(BindingAttributes::ASSOC)
                        && !self.binding_was_cleared_before_lookup(
                            binding,
                            self.current_scope(),
                            reference.name_span.start.offset,
                        )
                })
                .unwrap_or(false)
        {
            attributes |= BindingAttributes::ARRAY | BindingAttributes::ASSOC;
        }

        self.add_binding(
            &reference.name,
            BindingKind::ParameterDefaultAssignment,
            self.current_scope(),
            reference.span,
            BindingOrigin::ParameterDefaultAssignment {
                definition_span: reference.span,
            },
            attributes,
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

    fn newly_added_reference_ids_reading_name(
        &self,
        name: &Name,
        start: usize,
    ) -> Vec<ReferenceId> {
        self.references[start..]
            .iter()
            .filter(|reference| reference.name == *name)
            .map(|reference| reference.id)
            .collect()
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

    fn mark_local_declarations_visible_to_later_calls(&mut self) {
        let later_call_scopes = self
            .call_sites
            .iter()
            .filter(|(callee, _)| self.functions.contains_key(callee.as_str()))
            .flat_map(|(_, sites)| sites.iter())
            .map(|site| (site.scope, site.span.start.offset))
            .collect::<Vec<_>>();

        for binding in &mut self.bindings {
            if !matches!(binding.kind, BindingKind::Declaration(_))
                || !binding.attributes.contains(BindingAttributes::LOCAL)
                || !binding.references.is_empty()
            {
                continue;
            }
            if later_call_scopes
                .iter()
                .any(|(scope, offset)| *scope == binding.scope && *offset > binding.span.end.offset)
            {
                binding.attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
            }
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
                    && !binding
                        .attributes
                        .contains(BindingAttributes::SELF_REFERENTIAL_READ)
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
                let commands = match deferred.body {
                    DeferredFunctionBody::Recursive(function) => {
                        self.visit_function_like_body(&function.body, deferred.flow)
                    }
                    DeferredFunctionBody::Arena(body) => {
                        self.visit_function_like_body_arena(body, deferred.flow)
                    }
                };
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

    fn visit_function_like_body_arena(
        &mut self,
        body: StmtSeqId,
        flow: FlowState,
    ) -> RecordedCommandRange {
        let flow = FlowState {
            in_function: true,
            ..flow
        };
        let body = self.arena_stmt_seq(body);
        if body.stmt_ids().len() == 1 {
            let stmt = body.stmts().next().expect("single body statement");
            if let Some(compound) = stmt.command().compound()
                && let CompoundCommandNode::BraceGroup(inner) = compound.node()
            {
                return self.visit_stmt_seq_arena(self.arena_stmt_seq(*inner), flow);
            }
        }
        self.visit_stmt_seq_arena(body, flow)
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

fn reference_kind_for_word_visit(
    kind: WordVisitKind,
    expansion_kind: ReferenceKind,
) -> ReferenceKind {
    match kind {
        WordVisitKind::Expansion => expansion_kind,
        WordVisitKind::Conditional => ReferenceKind::ConditionalOperand,
        WordVisitKind::ParameterPattern => ReferenceKind::ParameterPattern,
    }
}

fn parameter_operation_reference_kind(
    kind: WordVisitKind,
    operator: &ParameterOp,
) -> ReferenceKind {
    if matches!(kind, WordVisitKind::ParameterPattern) {
        ReferenceKind::ParameterPattern
    } else if matches!(operator, ParameterOp::Error) {
        ReferenceKind::RequiredRead
    } else {
        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion)
    }
}

fn word_visit_kind_for_reference_kind(kind: ReferenceKind) -> WordVisitKind {
    match kind {
        ReferenceKind::ConditionalOperand => WordVisitKind::Conditional,
        ReferenceKind::ParameterPattern => WordVisitKind::ParameterPattern,
        _ => WordVisitKind::Expansion,
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

fn declaration_builtin_name(name: &str) -> Option<DeclarationBuiltin> {
    match name {
        "declare" => Some(DeclarationBuiltin::Declare),
        "local" => Some(DeclarationBuiltin::Local),
        "export" => Some(DeclarationBuiltin::Export),
        "readonly" => Some(DeclarationBuiltin::Readonly),
        "typeset" => Some(DeclarationBuiltin::Typeset),
        _ => None,
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

fn declaration_flags_arena(
    operands: &[DeclOperandNode],
    store: &AstStore,
    source: &str,
) -> FxHashSet<char> {
    let mut flags = FxHashSet::default();
    for operand in operands {
        if let DeclOperandNode::Flag(word) = operand
            && let Some(text) = static_word_text_arena(store.word(*word), source)
        {
            for flag in text.chars().skip(1) {
                flags.insert(flag);
            }
        }
    }
    flags
}

fn simple_declaration_option_word(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(polarity) = chars.next() else {
        return false;
    };
    matches!(polarity, '-' | '+')
        && !matches!(text, "-" | "+")
        && !text.starts_with("--")
        && chars.all(|flag| flag.is_ascii_alphabetic())
}

fn update_simple_declaration_flags(
    text: &str,
    flags: &mut FxHashSet<char>,
    global_flag_enabled: &mut bool,
    function_name_mode: &mut bool,
) {
    let enabled_for_operand = text.starts_with('-');
    for flag in text.chars().skip(1) {
        if enabled_for_operand {
            flags.insert(flag);
        } else {
            flags.remove(&flag);
        }

        if flag == 'g' {
            *global_flag_enabled = enabled_for_operand;
        }
        if matches!(flag, 'f' | 'F') {
            *function_name_mode = enabled_for_operand;
        }
    }
}

fn simple_declaration_flag_operand(word: &Word, text: &str) -> DeclarationOperand {
    DeclarationOperand::Flag {
        flag: text.chars().nth(1).unwrap_or('-'),
        flags: text.to_owned(),
        span: word.span,
    }
}

fn simple_declaration_flag_operand_arena(
    word: shuck_ast::WordView<'_>,
    text: &str,
) -> DeclarationOperand {
    DeclarationOperand::Flag {
        flag: text.chars().nth(1).unwrap_or('-'),
        flags: text.to_owned(),
        span: word.span(),
    }
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

fn declaration_flag_is_enabled_arena(
    operands: &[DeclOperandNode],
    store: &AstStore,
    source: &str,
    target: char,
) -> Option<bool> {
    let mut enabled = None;
    for operand in operands {
        if let DeclOperandNode::Flag(word) = operand
            && let Some(text) = static_word_text_arena(store.word(*word), source)
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

fn update_declaration_function_name_mode(word: &Word, source: &str, function_name_mode: &mut bool) {
    let Some(text) = static_word_text(word, source) else {
        return;
    };
    let mut chars = text.chars();
    let Some(polarity) = chars.next() else {
        return;
    };
    let enabled_for_operand = match polarity {
        '-' => true,
        '+' => false,
        _ => return,
    };
    for flag in chars {
        if matches!(flag, 'f' | 'F') {
            *function_name_mode = enabled_for_operand;
        }
    }
}

fn update_declaration_function_name_mode_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
    function_name_mode: &mut bool,
) {
    let Some(text) = static_word_text_arena(word, source) else {
        return;
    };
    let mut chars = text.chars();
    let Some(polarity) = chars.next() else {
        return;
    };
    let enabled_for_operand = match polarity {
        '-' => true,
        '+' => false,
        _ => return,
    };
    for flag in chars {
        if matches!(flag, 'f' | 'F') {
            *function_name_mode = enabled_for_operand;
        }
    }
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
                    flags: text.into_owned(),
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

fn declaration_operands_arena(
    operands: &[DeclOperandNode],
    store: &AstStore,
    source: &str,
) -> Vec<DeclarationOperand> {
    operands
        .iter()
        .map(|operand| match operand {
            DeclOperandNode::Flag(word) => {
                let word = store.word(*word);
                let text = static_word_text_arena(word, source).unwrap_or_default();
                let flag = text.chars().nth(1).unwrap_or('-');
                DeclarationOperand::Flag {
                    flag,
                    flags: text.into_owned(),
                    span: word.span(),
                }
            }
            DeclOperandNode::Name(name) => DeclarationOperand::Name {
                name: name.name.clone(),
                span: name.span,
            },
            DeclOperandNode::Assignment(assignment) => DeclarationOperand::Assignment {
                name: assignment.target.name.clone(),
                name_span: assignment.target.name_span,
                value_span: assignment_value_span_arena(assignment, store),
                append: assignment.append,
            },
            DeclOperandNode::Dynamic(word) => DeclarationOperand::DynamicWord {
                span: store.word(*word).span(),
            },
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

fn binding_attributes_for_var_ref_arena(reference: &VarRefNode) -> BindingAttributes {
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

fn assignment_binding_attributes_arena(assignment: &AssignmentNode) -> BindingAttributes {
    let mut attributes = binding_attributes_for_var_ref_arena(&assignment.target);
    if let AssignmentValueNode::Compound(array) = &assignment.value {
        attributes |= match array.kind {
            ArrayKind::Associative => BindingAttributes::ARRAY | BindingAttributes::ASSOC,
            ArrayKind::Indexed | ArrayKind::Contextual => BindingAttributes::ARRAY,
        };
    }
    attributes
}

fn assignment_value_span(assignment: &Assignment) -> Span {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word.span,
        AssignmentValue::Compound(array) => array.span,
    }
}

fn assignment_value_span_arena(assignment: &AssignmentNode, store: &AstStore) -> Span {
    match &assignment.value {
        AssignmentValueNode::Scalar(word) => store.word(*word).span(),
        AssignmentValueNode::Compound(array) => array.span,
    }
}

fn assignment_has_empty_initializer(assignment: &Assignment, source: &str) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => static_word_text(word, source).as_deref() == Some(""),
        AssignmentValue::Compound(array) => array.elements.is_empty(),
    }
}

fn assignment_has_empty_initializer_arena(
    assignment: &AssignmentNode,
    store: &AstStore,
    source: &str,
) -> bool {
    match &assignment.value {
        AssignmentValueNode::Scalar(word) => {
            static_word_text_arena(store.word(*word), source).as_deref() == Some("")
        }
        AssignmentValueNode::Compound(array) => store.array_elems(array.elements).is_empty(),
    }
}

fn indirect_target_hint(assignment: &Assignment, source: &str) -> Option<IndirectTargetHint> {
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    indirect_target_hint_from_word(word, source)
}

fn indirect_target_hint_arena(
    assignment: &AssignmentNode,
    store: &AstStore,
    source: &str,
) -> Option<IndirectTargetHint> {
    let AssignmentValueNode::Scalar(word) = &assignment.value else {
        return None;
    };
    indirect_target_hint_from_word_arena(store.word(*word), store, source)
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

fn indirect_target_hint_from_word_arena(
    word: shuck_ast::WordView<'_>,
    store: &AstStore,
    source: &str,
) -> Option<IndirectTargetHint> {
    if let Some(text) = static_word_text_arena(word, source) {
        let (name, array_like) = parse_indirect_target_name(&text)?;
        return Some(IndirectTargetHint::Exact {
            name: Name::from(name),
            array_like,
        });
    }

    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut saw_variable = false;
    if !collect_indirect_pattern_parts_arena(
        word.parts(),
        store,
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

fn collect_indirect_pattern_parts_arena(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    prefix: &mut String,
    suffix: &mut String,
    saw_variable: &mut bool,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPartArena::Literal(text) => {
                if *saw_variable {
                    suffix.push_str(text.as_str(source, part.span));
                } else {
                    prefix.push_str(text.as_str(source, part.span));
                }
            }
            WordPartArena::SingleQuoted { value, .. } => {
                if *saw_variable {
                    suffix.push_str(value.slice(source));
                } else {
                    prefix.push_str(value.slice(source));
                }
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                if !collect_indirect_pattern_parts_arena(
                    store.word_parts(*parts),
                    store,
                    source,
                    prefix,
                    suffix,
                    saw_variable,
                ) {
                    return false;
                }
            }
            WordPartArena::Variable(_) if !*saw_variable => *saw_variable = true,
            WordPartArena::Parameter(parameter)
                if !*saw_variable && parameter_is_indirect_pattern_variable_arena(parameter) =>
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

fn parameter_is_indirect_pattern_variable_arena(parameter: &ParameterExpansionNode) -> bool {
    matches!(
        &parameter.syntax,
        ParameterExpansionSyntaxNode::Bourne(BourneParameterExpansionNode::Access { reference })
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

fn iter_read_targets(args: &[&Word], source: &str) -> Vec<(Name, Span)> {
    let options = parse_read_options(args, source);
    let mut targets = Vec::new();

    if let Some(array_target) = options.array_target {
        targets.push(array_target);
    }

    if options.assigns_array {
        return targets;
    }

    targets.extend(
        args[options.target_start_index..]
            .iter()
            .filter_map(|word| named_target_word(word, source)),
    );
    targets
}

fn read_assigns_array(args: &[&Word], source: &str) -> bool {
    parse_read_options(args, source).assigns_array
}

fn iter_read_targets_arena(args: &[WordId], store: &AstStore, source: &str) -> Vec<(Name, Span)> {
    let options = parse_read_options_arena(args, store, source);
    let mut targets = Vec::new();

    if let Some(array_target) = options.array_target {
        targets.push(array_target);
    }

    if options.assigns_array {
        return targets;
    }

    targets.extend(
        args[options.target_start_index..]
            .iter()
            .filter_map(|word| named_target_word_arena(store.word(*word), source)),
    );
    targets
}

fn read_assigns_array_arena(args: &[WordId], store: &AstStore, source: &str) -> bool {
    parse_read_options_arena(args, store, source).assigns_array
}

#[derive(Debug, Clone)]
struct ParsedReadOptions {
    assigns_array: bool,
    target_start_index: usize,
    array_target: Option<(Name, Span)>,
}

fn parse_read_options(args: &[&Word], source: &str) -> ParsedReadOptions {
    let mut assigns_array = false;
    let mut array_target = None;
    let mut index = 0;
    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        let Some(flags) = text.strip_prefix('-') else {
            break;
        };
        if flags.is_empty() || flags.starts_with('-') {
            break;
        }

        let mut stop_after_array_target = false;
        for (offset, flag) in flags.char_indices() {
            if flag == 'a' {
                assigns_array = true;
                let attached_offset = offset + flag.len_utf8();
                if attached_offset < flags.len() {
                    array_target =
                        read_attached_array_target(word, source, &flags[attached_offset..]);
                } else if let Some(target) = args
                    .get(index + 1)
                    .and_then(|word| named_target_word(word, source))
                {
                    array_target = Some(target);
                    index += 1;
                }
                stop_after_array_target = true;
                break;
            }
            if read_flag_takes_value(flag) {
                if offset + flag.len_utf8() == flags.len() {
                    index += 1;
                }
                break;
            }
        }
        index += 1;
        if stop_after_array_target {
            break;
        }
    }

    ParsedReadOptions {
        assigns_array,
        target_start_index: index.min(args.len()),
        array_target,
    }
}

fn parse_read_options_arena(args: &[WordId], store: &AstStore, source: &str) -> ParsedReadOptions {
    let mut assigns_array = false;
    let mut array_target = None;
    let mut index = 0;
    while let Some(word_id) = args.get(index) {
        let word = store.word(*word_id);
        let Some(text) = static_word_text_arena(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        let Some(flags) = text.strip_prefix('-') else {
            break;
        };
        if flags.is_empty() || flags.starts_with('-') {
            break;
        }

        let mut stop_after_array_target = false;
        for (offset, flag) in flags.char_indices() {
            if flag == 'a' {
                assigns_array = true;
                let attached_offset = offset + flag.len_utf8();
                if attached_offset < flags.len() {
                    array_target =
                        read_attached_array_target_arena(word, source, &flags[attached_offset..]);
                } else if let Some(target) = args
                    .get(index + 1)
                    .and_then(|word| named_target_word_arena(store.word(*word), source))
                {
                    array_target = Some(target);
                    index += 1;
                }
                stop_after_array_target = true;
                break;
            }
            if read_flag_takes_value(flag) {
                if offset + flag.len_utf8() == flags.len() {
                    index += 1;
                }
                break;
            }
        }
        index += 1;
        if stop_after_array_target {
            break;
        }
    }

    ParsedReadOptions {
        assigns_array,
        target_start_index: index.min(args.len()),
        array_target,
    }
}

fn read_flag_takes_value(flag: char) -> bool {
    matches!(flag, 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
}

#[derive(Debug, Clone)]
enum MapfileTarget {
    Explicit(Name, Span),
    Implicit,
}

fn mapfile_target(args: &[&Word], source: &str) -> Option<MapfileTarget> {
    let mut index = 0;
    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        let Some(flags) = text.strip_prefix('-') else {
            break;
        };
        if flags.is_empty() || flags.starts_with('-') {
            break;
        }
        for (offset, flag) in flags.char_indices() {
            if mapfile_flag_takes_value(flag) {
                if offset + flag.len_utf8() == flags.len() {
                    index += 1;
                }
                break;
            }
        }
        index += 1;
    }

    if let Some((name, span)) = args[index..]
        .iter()
        .find_map(|word| named_target_word(word, source))
    {
        return Some(MapfileTarget::Explicit(name, span));
    }

    args.get(index).is_none().then_some(MapfileTarget::Implicit)
}

fn mapfile_target_arena(args: &[WordId], store: &AstStore, source: &str) -> Option<MapfileTarget> {
    let mut index = 0;
    while let Some(word_id) = args.get(index) {
        let word = store.word(*word_id);
        let Some(text) = static_word_text_arena(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        let Some(flags) = text.strip_prefix('-') else {
            break;
        };
        if flags.is_empty() || flags.starts_with('-') {
            break;
        }
        for (offset, flag) in flags.char_indices() {
            if mapfile_flag_takes_value(flag) {
                if offset + flag.len_utf8() == flags.len() {
                    index += 1;
                }
                break;
            }
        }
        index += 1;
    }

    if let Some((name, span)) = args[index..]
        .iter()
        .find_map(|word| named_target_word_arena(store.word(*word), source))
    {
        return Some(MapfileTarget::Explicit(name, span));
    }

    args.get(index).is_none().then_some(MapfileTarget::Implicit)
}

fn mapfile_flag_takes_value(flag: char) -> bool {
    matches!(flag, 'C' | 'c' | 'd' | 'n' | 'O' | 's' | 'u')
}

fn printf_v_target(args: &[&Word], source: &str) -> Option<(Name, Span)> {
    args.windows(2).find_map(|window| {
        (static_word_text(window[0], source).as_deref() == Some("-v"))
            .then_some(window[1])
            .and_then(|word| named_target_word(word, source))
    })
}

fn printf_v_target_arena(args: &[WordId], store: &AstStore, source: &str) -> Option<(Name, Span)> {
    args.windows(2).find_map(|window| {
        (static_word_text_arena(store.word(window[0]), source).as_deref() == Some("-v"))
            .then_some(window[1])
            .and_then(|word| named_target_word_arena(store.word(word), source))
    })
}

fn getopts_target(args: &[&Word], source: &str) -> Option<(Name, Span)> {
    args.get(1).and_then(|word| named_target_word(word, source))
}

fn getopts_target_arena(args: &[WordId], store: &AstStore, source: &str) -> Option<(Name, Span)> {
    args.get(1)
        .and_then(|word| named_target_word_arena(store.word(*word), source))
}

fn variable_set_test_operand_name(
    expression: &ConditionalExpr,
    source: &str,
) -> Option<(Name, Span)> {
    match expression {
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            variable_name_operand_from_source(word.span.slice(source), word.span)
        }
        ConditionalExpr::Pattern(pattern) => {
            variable_name_operand_from_source(pattern.span.slice(source), pattern.span)
        }
        ConditionalExpr::VarRef(reference) => Some((reference.name.clone(), reference.name_span)),
        ConditionalExpr::Parenthesized(expression) => {
            variable_set_test_operand_name(&expression.expr, source)
        }
        ConditionalExpr::Unary(_) | ConditionalExpr::Binary(_) => None,
    }
}

fn conditional_binary_op_uses_arithmetic_operands(op: ConditionalBinaryOp) -> bool {
    matches!(
        op,
        ConditionalBinaryOp::ArithmeticEq
            | ConditionalBinaryOp::ArithmeticNe
            | ConditionalBinaryOp::ArithmeticLe
            | ConditionalBinaryOp::ArithmeticGe
            | ConditionalBinaryOp::ArithmeticLt
            | ConditionalBinaryOp::ArithmeticGt
    )
}

fn unparsed_arithmetic_subscript_reference_names(
    source_text: &SourceText,
    source: &str,
) -> Vec<(Name, Span)> {
    if !source_text.is_source_backed() {
        return Vec::new();
    }

    let text = source_text.slice(source);
    let Some((leading, _)) = text.split_once(':') else {
        return Vec::new();
    };

    let mut references = Vec::new();
    let mut chars = leading.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !is_name_start_character(ch) || text[..start].ends_with('$') {
            continue;
        }

        let mut end = start + ch.len_utf8();
        while let Some((next_index, next)) = chars.peek().copied() {
            if !is_name_character(next) {
                break;
            }
            chars.next();
            end = next_index + next.len_utf8();
        }

        let name = &leading[start..end];
        let start_position = source_text.span().start.advanced_by(&text[..start]);
        references.push((
            Name::from(name),
            Span::from_positions(start_position, start_position.advanced_by(name)),
        ));
    }

    references
}

fn escaped_braced_literal_reference_names(text: &str, span: Span) -> Vec<(Name, Span)> {
    let mut references = Vec::new();
    let mut search_start = 0;

    while let Some(start_rel) = text[search_start..].find("\\${") {
        let start = search_start + start_rel;
        let mut cursor = start + "\\${".len();
        let mut depth = 1usize;
        let mut escaped = false;

        while cursor < text.len() {
            let Some(ch) = text[cursor..].chars().next() else {
                break;
            };
            let next = cursor + ch.len_utf8();

            if escaped {
                escaped = false;
                cursor = next;
                continue;
            }

            if ch == '\\' {
                escaped = true;
                cursor = next;
                continue;
            }

            if ch == '$' {
                let after_dollar = next;
                if text[after_dollar..].starts_with('{') {
                    depth += 1;
                }
                if let Some((name_start, name_end)) =
                    parameter_name_bounds_after_dollar(text, after_dollar)
                {
                    let name = &text[name_start..name_end];
                    let mut reference_end = name_end;
                    if text[after_dollar..].starts_with('{') && text[name_end..].starts_with('}') {
                        reference_end += '}'.len_utf8();
                    }
                    let start_position = span.start.advanced_by(&text[..cursor]);
                    references.push((
                        Name::from(name),
                        Span::from_positions(
                            start_position,
                            start_position.advanced_by(&text[cursor..reference_end]),
                        ),
                    ));
                }
                cursor = next;
                continue;
            }

            if ch == '}' {
                depth = depth.saturating_sub(1);
                cursor = next;
                if depth == 0 {
                    break;
                }
                continue;
            }

            cursor = next;
        }

        search_start = cursor;
    }

    references
}

fn escaped_braced_literal_may_contain_reference(text: &str) -> bool {
    text.contains("\\${")
}

fn conditional_arithmetic_operand_name(
    expression: &ConditionalExpr,
    source: &str,
) -> Option<(Name, Span)> {
    match strip_parenthesized_conditional(expression) {
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            static_word_text(word, source).and_then(|text| {
                is_name(text.as_ref()).then(|| (Name::from(text.as_ref()), word.span))
            })
        }
        ConditionalExpr::Pattern(pattern) => {
            let text = pattern.span.slice(source).trim();
            is_name(text).then(|| (Name::from(text), pattern.span))
        }
        ConditionalExpr::VarRef(_)
        | ConditionalExpr::Unary(_)
        | ConditionalExpr::Binary(_)
        | ConditionalExpr::Parenthesized(_) => None,
    }
}

fn conditional_arithmetic_operand_name_arena(
    expression: &ConditionalExprArena,
    store: &AstStore,
    source: &str,
) -> Option<(Name, Span)> {
    match strip_parenthesized_conditional_arena(expression) {
        ConditionalExprArena::Word(word) | ConditionalExprArena::Regex(word) => {
            let word = store.word(*word);
            conditional_static_word_text_arena(word, source).and_then(|text| {
                is_name(text.as_ref()).then(|| (Name::from(text.as_ref()), word.span()))
            })
        }
        ConditionalExprArena::Pattern(pattern) => {
            let text = pattern.span.slice(source).trim();
            is_name(text).then(|| (Name::from(text), pattern.span))
        }
        ConditionalExprArena::VarRef(_)
        | ConditionalExprArena::Unary { .. }
        | ConditionalExprArena::Binary { .. }
        | ConditionalExprArena::Parenthesized { .. } => None,
    }
}

fn strip_parenthesized_conditional(expression: &ConditionalExpr) -> &ConditionalExpr {
    let mut current = expression;
    while let ConditionalExpr::Parenthesized(paren) = current {
        current = &paren.expr;
    }
    current
}

fn strip_parenthesized_conditional_arena(
    expression: &ConditionalExprArena,
) -> &ConditionalExprArena {
    let mut current = expression;
    while let ConditionalExprArena::Parenthesized { expr, .. } = current {
        current = expr;
    }
    current
}

fn variable_set_test_operand_name_arena(
    expression: &ConditionalExprArena,
    store: &AstStore,
    source: &str,
) -> Option<(Name, Span)> {
    match strip_parenthesized_conditional_arena(expression) {
        ConditionalExprArena::Word(word) | ConditionalExprArena::Regex(word) => {
            let word = store.word(*word);
            variable_name_operand_from_source(word.span().slice(source), word.span())
        }
        ConditionalExprArena::Pattern(pattern) => {
            variable_name_operand_from_source(pattern.span.slice(source), pattern.span)
        }
        ConditionalExprArena::VarRef(reference) => {
            Some((reference.name.clone(), reference.name_span))
        }
        ConditionalExprArena::Unary { .. }
        | ConditionalExprArena::Binary { .. }
        | ConditionalExprArena::Parenthesized { .. } => None,
    }
}

fn variable_name_operand_from_source(text: &str, span: Span) -> Option<(Name, Span)> {
    let leading_whitespace = text.len() - text.trim_start().len();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (operand, operand_start) = unquote_variable_test_operand(trimmed, leading_whitespace)?;
    let name_end = direct_variable_test_name_end(operand)?;
    let name = &operand[..name_end];
    let start_position = span.start.advanced_by(&text[..operand_start]);
    Some((
        Name::from(name),
        Span::from_positions(start_position, start_position.advanced_by(name)),
    ))
}

fn unquote_variable_test_operand(text: &str, base_offset: usize) -> Option<(&str, usize)> {
    let Some(quote) = text.chars().next().filter(|ch| matches!(ch, '"' | '\'')) else {
        return Some((text, base_offset));
    };
    let quote_width = quote.len_utf8();
    if text.len() <= quote_width || !text.ends_with(quote) {
        return None;
    }
    Some((
        &text[quote_width..text.len() - quote_width],
        base_offset + quote_width,
    ))
}

fn direct_variable_test_name_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !is_name_start_character(first) {
        return None;
    }

    let mut end = first.len_utf8();
    for (index, ch) in chars {
        if !is_name_character(ch) {
            break;
        }
        end = index + ch.len_utf8();
    }

    let trailing = &text[end..];
    if trailing.is_empty() || valid_direct_variable_subscript(trailing) {
        Some(end)
    } else {
        None
    }
}

fn valid_direct_variable_subscript(text: &str) -> bool {
    text.starts_with('[') && text.ends_with(']') && text.len() > 2
}

fn eval_argument_reference_names(word: &Word, source: &str) -> Vec<(Name, Span)> {
    let source_text = word.span.slice(source);
    let decoded = decode_eval_word_text(source_text);
    scan_parameter_reference_names(
        &decoded.text,
        source_text,
        &decoded.source_offsets,
        word.span,
    )
}

fn eval_argument_reference_names_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
) -> Vec<(Name, Span)> {
    let span = word.span();
    let source_text = span.slice(source);
    let decoded = decode_eval_word_text(source_text);
    scan_parameter_reference_names(&decoded.text, source_text, &decoded.source_offsets, span)
}

fn trap_action_argument<'a>(args: &[&'a Word], source: &str) -> Option<&'a Word> {
    let argument = *args.first()?;
    let text = static_word_text(argument, source)?;

    if text == "--" {
        return args.get(1).copied();
    }
    if is_trap_inspection_option(&text) {
        return None;
    }

    Some(argument)
}

fn trap_action_argument_arena(args: &[WordId], store: &AstStore, source: &str) -> Option<WordId> {
    let argument = *args.first()?;
    let text = static_word_text_arena(store.word(argument), source)?;

    if text == "--" {
        return args.get(1).copied();
    }
    if is_trap_inspection_option(&text) {
        return None;
    }

    Some(argument)
}

fn is_trap_inspection_option(text: &str) -> bool {
    text.len() > 1
        && text.starts_with('-')
        && text[1..].chars().all(|flag| matches!(flag, 'l' | 'p'))
}

fn trap_action_reference_names(word: &Word, source: &str) -> Vec<Name> {
    let Some(text) = static_word_text(word, source) else {
        return Vec::new();
    };

    scan_parameter_reference_name_ranges(&text)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn trap_action_reference_names_arena(word: shuck_ast::WordView<'_>, source: &str) -> Vec<Name> {
    let Some(text) = static_word_text_arena(word, source) else {
        return Vec::new();
    };

    scan_parameter_reference_name_ranges(&text)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn prompt_assignment_reference_names(word: &Word, source: &str) -> Vec<(Name, Span)> {
    let Some(text) = static_word_text(word, source) else {
        return Vec::new();
    };
    scan_prompt_parameter_reference_names(text.as_ref(), word.span)
}

fn prompt_assignment_reference_names_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
) -> Vec<(Name, Span)> {
    let Some(text) = static_word_text_arena(word, source) else {
        return Vec::new();
    };
    scan_prompt_parameter_reference_names(text.as_ref(), word.span())
}

fn escaped_prompt_assignment_reference_names(word: &Word, source: &str) -> Vec<Name> {
    if static_word_text(word, source).is_none() {
        return Vec::new();
    }

    let text = word.span.slice(source);
    let mut names = Vec::new();
    let mut search_start = 0;

    while let Some(start_rel) = text[search_start..].find("\\${") {
        let start = search_start + start_rel;
        let after_dollar = start + "\\$".len();
        if let Some((name_start, name_end)) = parameter_name_bounds_after_dollar(text, after_dollar)
        {
            names.push(Name::from(&text[name_start..name_end]));
            search_start = name_end;
        } else {
            search_start = start + "\\${".len();
        }
    }

    names
}

fn escaped_prompt_assignment_reference_names_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
) -> Vec<Name> {
    let text = word.span().slice(source);
    let mut names = Vec::new();
    let mut search_start = 0;

    while let Some(start_rel) = text[search_start..].find("\\${") {
        let start = search_start + start_rel;
        let after_dollar = start + "\\$".len();
        if let Some((name_start, name_end)) = parameter_name_bounds_after_dollar(text, after_dollar)
        {
            names.push(Name::from(&text[name_start..name_end]));
            search_start = name_end;
        } else {
            search_start = start + "\\${".len();
        }
    }
    names
}

fn scan_prompt_parameter_reference_names(text: &str, span: Span) -> Vec<(Name, Span)> {
    let mut references = Vec::new();
    for (index, ch) in text.char_indices() {
        if ch != '$' {
            continue;
        }

        let after_dollar = index + ch.len_utf8();
        let Some((name_start, name_end)) = parameter_name_bounds_after_dollar(text, after_dollar)
        else {
            continue;
        };
        references.push((Name::from(&text[name_start..name_end]), span));
    }
    references
}

struct DecodedEvalText {
    text: String,
    source_offsets: Vec<usize>,
}

impl DecodedEvalText {
    fn push(&mut self, ch: char, source_offset: usize) {
        self.text.push(ch);
        self.source_offsets
            .extend(std::iter::repeat_n(source_offset, ch.len_utf8()));
    }
}

fn decode_eval_word_text(source_text: &str) -> DecodedEvalText {
    let mut decoded = DecodedEvalText {
        text: String::new(),
        source_offsets: Vec::new(),
    };
    let mut chars = source_text.char_indices().peekable();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;

    while let Some((index, ch)) = chars.next() {
        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            continue;
        }

        if in_double_quotes {
            match ch {
                '"' => in_double_quotes = false,
                '\\' => {
                    if let Some(&(next_index, next_ch)) = chars.peek()
                        && matches!(next_ch, '$' | '`' | '"' | '\\' | '\n')
                    {
                        chars.next();
                        if next_ch != '\n' {
                            decoded.push(next_ch, next_index);
                        }
                    } else {
                        decoded.push(ch, index);
                    }
                }
                _ => decoded.push(ch, index),
            }
            continue;
        }

        match ch {
            '\'' => in_single_quotes = true,
            '"' => in_double_quotes = true,
            '\\' => {
                if let Some((next_index, next_ch)) = chars.next() {
                    if next_ch != '\n' {
                        decoded.push(next_ch, next_index);
                    }
                } else {
                    decoded.push(ch, index);
                }
            }
            _ => decoded.push(ch, index),
        }
    }

    decoded
}

fn scan_parameter_reference_names(
    text: &str,
    source_text: &str,
    source_offsets: &[usize],
    span: Span,
) -> Vec<(Name, Span)> {
    scan_parameter_reference_name_ranges(text)
        .into_iter()
        .map(|(name, (name_start, _name_end))| {
            let source_name_start = source_offsets[name_start];
            let source_name_end = source_name_start + name.as_str().len();
            let start = span.start.advanced_by(&source_text[..source_name_start]);
            (
                name,
                Span::from_positions(
                    start,
                    start.advanced_by(&source_text[source_name_start..source_name_end]),
                ),
            )
        })
        .collect()
}

fn scan_parameter_reference_name_ranges(text: &str) -> Vec<(Name, (usize, usize))> {
    let mut references = Vec::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut in_comment = false;
    let mut escaped = false;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
            }
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            continue;
        }

        if ch == '\'' && !in_double_quotes {
            in_single_quotes = true;
            continue;
        }
        if ch == '"' {
            in_double_quotes = !in_double_quotes;
            continue;
        }
        if ch == '\\' {
            if in_double_quotes {
                if chars
                    .peek()
                    .is_some_and(|(_, next_ch)| matches!(next_ch, '$' | '`' | '"' | '\\' | '\n'))
                {
                    escaped = true;
                }
            } else {
                escaped = true;
            }
            continue;
        }
        if !in_double_quotes && ch == '#' && hash_starts_eval_comment(text, index) {
            in_comment = true;
            continue;
        }
        if ch != '$' {
            continue;
        }
        if chars.peek().is_some_and(|(_, next_ch)| *next_ch == '$') {
            chars.next();
            continue;
        }

        let after_dollar = index + ch.len_utf8();
        let Some((name_start, name_end)) = parameter_name_bounds_after_dollar(text, after_dollar)
        else {
            continue;
        };
        let name = &text[name_start..name_end];
        references.push((Name::from(name), (name_start, name_end)));
    }
    references
}

fn hash_starts_eval_comment(text: &str, hash_offset: usize) -> bool {
    if let Some(ch) = text[..hash_offset].chars().next_back() {
        return ch == '\n' || ch.is_whitespace() || matches!(ch, ';' | '&' | '|');
    }
    true
}

fn parameter_name_bounds_after_dollar(text: &str, after_dollar: usize) -> Option<(usize, usize)> {
    let mut chars = text[after_dollar..].char_indices();
    let (_, first) = chars.next()?;
    let name_start = if first == '{' {
        after_dollar + first.len_utf8()
    } else if is_name_start_character(first) {
        after_dollar
    } else {
        return None;
    };

    let mut name_chars = text[name_start..].char_indices();
    let (_, first_name) = name_chars.next()?;
    if !is_name_start_character(first_name) {
        return None;
    }

    let mut name_end = name_start + first_name.len_utf8();
    for (index, ch) in name_chars {
        if !is_name_character(ch) {
            break;
        }
        name_end = name_start + index + ch.len_utf8();
    }

    Some((name_start, name_end))
}

fn is_name_start_character(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_name_character(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn simple_command_has_name(command: &shuck_ast::SimpleCommand, source: &str) -> bool {
    !matches!(static_word_text(&command.name, source).as_deref(), Some(""))
}

fn resolved_command_can_affect_current_shell(command: &NormalizedCommand<'_>) -> bool {
    command.wrappers.iter().all(|wrapper| {
        matches!(
            wrapper,
            WrapperKind::Command | WrapperKind::Builtin | WrapperKind::Noglob
        )
    })
}

fn normalize_command_words_arena<'a>(
    store: &'a AstStore,
    words: &[WordId],
    command_span: Span,
    source: &'a str,
) -> Option<ArenaNormalizedCommand<'a>> {
    let first_id = words.first().copied()?;
    let first_word = store.word(first_id);
    let literal_name = static_command_name_text_arena(first_word, source);
    let mut effective_name = literal_name.clone();
    let mut wrappers = Vec::new();
    let mut body_word_span = Some(first_word.span());
    let mut body_start = literal_name.as_ref().map(|_| 0usize);
    let mut current_index = 0usize;

    while let Some(current_name) = effective_name.as_deref() {
        match static_command_wrapper_target_index(
            words.len(),
            current_index,
            current_name,
            |word_index| static_word_text_arena(store.word(words[word_index]), source),
        ) {
            StaticCommandWrapperTarget::NotWrapper => break,
            StaticCommandWrapperTarget::Wrapper { target_index } => {
                let kind = match current_name {
                    "noglob" => WrapperKind::Noglob,
                    "command" => WrapperKind::Command,
                    "builtin" => WrapperKind::Builtin,
                    "exec" => WrapperKind::Exec,
                    _ => WrapperKind::Command,
                };
                wrappers.push(kind);
                let Some(target_index) = target_index else {
                    effective_name = None;
                    body_word_span = None;
                    body_start = None;
                    break;
                };
                let target = store.word(words[target_index]);
                body_word_span = Some(target.span());
                effective_name = static_command_name_text_arena(target, source);
                body_start = effective_name.as_ref().map(|_| target_index);
                current_index = target_index;
                if effective_name.is_none() {
                    break;
                }
            }
        }
    }

    Some(ArenaNormalizedCommand {
        literal_name,
        effective_name,
        wrappers,
        body_word_span,
        body_words: body_start.map_or_else(Vec::new, |start| words[start..].to_vec()),
        command_span,
    })
}

fn static_word_text_arena<'a>(
    word: shuck_ast::WordView<'_>,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    try_static_word_parts_text_arena(word.parts(), word.store(), source)
}

fn conditional_static_word_text_arena<'a>(
    word: shuck_ast::WordView<'_>,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    static_word_text_arena(word, source).or_else(|| {
        let text = word.span().slice(source);
        (text.len() >= 2 && text.starts_with('"') && text.ends_with('"'))
            .then(|| Cow::Owned(text[1..text.len() - 1].to_owned()))
    })
}

fn static_command_name_text_arena<'a>(
    word: shuck_ast::WordView<'_>,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    try_static_command_name_parts_text_arena(word.parts(), source, StaticNameContext::Unquoted)
}

#[derive(Clone, Copy)]
enum StaticNameContext {
    Unquoted,
}

fn try_static_word_parts_text_arena<'a>(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    if parts.is_empty() {
        return Some(Cow::Borrowed(""));
    }
    let mut owned = None::<String>;
    for part in parts {
        let text = match &part.kind {
            WordPartArena::Literal(text) => text.as_str(source, part.span),
            WordPartArena::SingleQuoted { value, .. } => value.slice(source),
            WordPartArena::DoubleQuoted { parts, .. } => {
                let text =
                    try_static_word_parts_text_arena(store.word_parts(*parts), store, source)?;
                owned.get_or_insert_with(String::new).push_str(&text);
                continue;
            }
            _ => return None,
        };
        owned.get_or_insert_with(String::new).push_str(text);
    }
    owned.map(Cow::Owned)
}

fn try_static_command_name_parts_text_arena<'a>(
    parts: &[WordPartArenaNode],
    source: &'a str,
    context: StaticNameContext,
) -> Option<Cow<'a, str>> {
    if parts.is_empty() {
        return Some(Cow::Borrowed(""));
    }
    let mut result = String::new();
    for part in parts {
        match &part.kind {
            WordPartArena::Literal(text) => {
                append_decoded_static_command_literal_arena(
                    text.as_str(source, part.span),
                    context,
                    &mut result,
                );
            }
            WordPartArena::SingleQuoted { value, .. } => result.push_str(value.slice(source)),
            WordPartArena::DoubleQuoted { .. } => return None,
            _ => return None,
        }
    }
    Some(Cow::Owned(result))
}

fn append_decoded_static_command_literal_arena(
    text: &str,
    context: StaticNameContext,
    out: &mut String,
) {
    let mut chars = text.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some((_, next)) = chars.next() else {
            out.push('\\');
            return;
        };
        match context {
            StaticNameContext::Unquoted => {
                if next != '\n' {
                    out.push(next);
                }
            }
        }
    }
}

fn resolved_command_can_affect_current_shell_arena(command: &ArenaNormalizedCommand<'_>) -> bool {
    command.wrappers.iter().all(|wrapper| {
        matches!(
            wrapper,
            WrapperKind::Command | WrapperKind::Builtin | WrapperKind::Noglob
        )
    })
}

fn named_target_word(word: &Word, source: &str) -> Option<(Name, Span)> {
    let text = static_word_text(word, source)?;
    is_name(&text).then_some((Name::from(text.as_ref()), word.span))
}

fn named_target_word_arena(word: shuck_ast::WordView<'_>, source: &str) -> Option<(Name, Span)> {
    let text = static_word_text_arena(word, source)?;
    is_name(&text).then_some((Name::from(text.as_ref()), word.span()))
}

fn declaration_assignment_text<'a>(word: &'a Word, source: &'a str) -> Cow<'a, str> {
    static_word_text(word, source).unwrap_or_else(|| Cow::Borrowed(word.span.slice(source)))
}

#[derive(Debug, Clone)]
struct SimpleDeclarationAssignment {
    name: Name,
    name_span: Span,
    target_span: Span,
    value_span: Span,
    append: bool,
    array_like: bool,
    value_origin: AssignmentValueOrigin,
}

fn parse_simple_declaration_assignment(
    word: &Word,
    text: &str,
    source: &str,
) -> Option<SimpleDeclarationAssignment> {
    parse_simple_declaration_assignment_from_span(word.span, text, source)
}

fn parse_simple_declaration_assignment_from_text(
    span: Span,
    text: &str,
    source: &str,
) -> Option<SimpleDeclarationAssignment> {
    parse_simple_declaration_assignment_from_span(span, text, source)
}

fn parse_simple_declaration_assignment_from_span(
    span: Span,
    text: &str,
    source: &str,
) -> Option<SimpleDeclarationAssignment> {
    let name_end = variable_name_end(text)?;
    let name = &text[..name_end];
    let mut index = name_end;
    let mut array_like = false;

    if text.as_bytes().get(index) == Some(&b'[') {
        let subscript_end = text[index..].find(']')? + index + 1;
        index = subscript_end;
        array_like = true;
    }

    let append = if text.as_bytes().get(index) == Some(&b'+') {
        index += 1;
        true
    } else {
        false
    };

    if text.as_bytes().get(index) != Some(&b'=') {
        return None;
    }

    let value_start = index + 1;
    let name_span = word_text_offset_span(span, source, 0, name_end);
    let target_span =
        word_text_offset_span(span, source, 0, if append { index - 1 } else { index });
    let value_span = word_text_offset_span(span, source, value_start, text.len());
    let value_origin = if text[value_start..].trim_start().starts_with('(') {
        AssignmentValueOrigin::ArrayOrCompound
    } else {
        AssignmentValueOrigin::Unknown
    };

    Some(SimpleDeclarationAssignment {
        name: Name::from(name),
        name_span,
        target_span,
        value_span,
        append,
        array_like,
        value_origin,
    })
}

fn let_arithmetic_assignment_target(word: &Word, source: &str) -> Option<(Name, Span)> {
    let text = word.span.slice(source);
    let name_end = variable_name_end(text)?;
    let rest = text[name_end..].trim_start();
    arithmetic_assignment_operator(rest)?;

    Some((
        Name::from(&text[..name_end]),
        word_text_offset_span(word.span, source, 0, name_end),
    ))
}

fn let_arithmetic_assignment_target_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
) -> Option<(Name, Span)> {
    let span = word.span();
    let text = span.slice(source);
    let name_end = variable_name_end(text)?;
    let rest = text[name_end..].trim_start();
    arithmetic_assignment_operator(rest)?;

    Some((
        Name::from(&text[..name_end]),
        word_text_offset_span(span, source, 0, name_end),
    ))
}

fn arithmetic_assignment_operator(text: &str) -> Option<&'static str> {
    const ASSIGNMENT_OPERATORS: &[&str] = &[
        "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
    ];

    ASSIGNMENT_OPERATORS.iter().copied().find(|&operator| {
        text.starts_with(operator) && !(operator == "=" && text.as_bytes().get(1) == Some(&b'='))
    })
}

fn variable_name_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !is_name_start_character(first) {
        return None;
    }
    let mut end = first.len_utf8();
    for (index, ch) in chars {
        if !is_name_character(ch) {
            break;
        }
        end = index + ch.len_utf8();
    }
    Some(end)
}

fn word_text_offset_span(span: Span, source: &str, start: usize, end: usize) -> Span {
    let source_text = span.slice(source);
    let start = start.min(source_text.len());
    let end = end.min(source_text.len()).max(start);
    let start = span.start.advanced_by(&source_text[..start]);
    let end = span.start.advanced_by(&source_text[..end]);
    Span::from_positions(start, end)
}

fn read_attached_array_target(
    word: &Word,
    source: &str,
    target_text: &str,
) -> Option<(Name, Span)> {
    if !is_name(target_text) {
        return None;
    }

    let target_span = word
        .span
        .slice(source)
        .rfind(target_text)
        .map(|start| {
            read_option_attached_target_span(word.span, source, start, start + target_text.len())
        })
        .unwrap_or(word.span);

    Some((Name::from(target_text), target_span))
}

fn read_attached_array_target_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
    target_text: &str,
) -> Option<(Name, Span)> {
    if !is_name(target_text) {
        return None;
    }

    let span = word.span();
    let target_span = span
        .slice(source)
        .rfind(target_text)
        .map(|start| {
            read_option_attached_target_span(span, source, start, start + target_text.len())
        })
        .unwrap_or(span);

    Some((Name::from(target_text), target_span))
}

fn read_option_attached_target_span(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
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

fn recorded_command_info_arena(
    command: CommandView<'_>,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> RecordedCommandInfo {
    let command_span = command.span();
    let Some(command) = command.simple() else {
        let text = command.span().slice(source);
        let words = text
            .split_whitespace()
            .map(|word| Some(word.to_owned()))
            .collect::<Vec<_>>();
        let Some(Some(static_callee)) = words.first() else {
            return RecordedCommandInfo::default();
        };
        if !matches!(
            static_callee.as_str(),
            "emulate" | "setopt" | "unsetopt" | "set"
        ) {
            return RecordedCommandInfo::default();
        }
        let static_args = words
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let mut info = RecordedCommandInfo {
            static_callee: Some(static_callee.clone()),
            static_args,
            source_path_template: None,
            zsh_effects: Vec::new(),
        };
        let args = words.get(1..).unwrap_or(&[]);
        match static_callee.as_str() {
            "emulate" => info.zsh_effects = parse_emulate_effects_static(args),
            "setopt" => {
                info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                    updates: parse_setopt_updates_static(args, true),
                }];
            }
            "unsetopt" => {
                info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                    updates: parse_setopt_updates_static(args, false),
                }];
            }
            _ => {}
        }
        info.zsh_effects.retain(|effect| match effect {
            RecordedZshCommandEffect::Emulate { .. } => true,
            RecordedZshCommandEffect::SetOptions { updates } => !updates.is_empty(),
        });
        return info;
    };
    let word_ids = std::iter::once(command.name().id())
        .chain(command.args().map(|word| word.id()))
        .collect::<Vec<_>>();
    let normalized =
        normalize_command_words_arena(command.name().store(), &word_ids, command_span, source);
    let mut static_callee = normalized
        .as_ref()
        .and_then(|command| command.effective_name.as_ref())
        .map(|name| name.to_string())
        .or_else(|| {
            static_command_name_text_arena(command.name(), source).map(|name| name.into_owned())
        });
    let body_words = normalized
        .as_ref()
        .map(|command| command.body_words.as_slice())
        .unwrap_or(word_ids.as_slice());
    let static_args = body_words
        .iter()
        .skip(1)
        .map(|word| {
            static_word_text_arena(command.name().store().word(*word), source)
                .map(|text| text.into_owned())
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let source_path_template = static_callee
        .as_deref()
        .filter(|name| matches!(*name, "source" | "."))
        .and_then(|_| body_words.get(1).copied())
        .map(|word| command.name().store().word(word))
        .and_then(|word| source_path_template_arena(word, source, bash_runtime_vars_enabled));

    if static_callee.as_deref() == Some("noglob") {
        static_callee = command.args().next().and_then(|word| {
            static_command_name_text_arena(word, source).map(|name| name.into_owned())
        });
    }

    let mut info = RecordedCommandInfo {
        static_callee,
        static_args,
        source_path_template,
        zsh_effects: Vec::new(),
    };

    let static_words = std::iter::once(command.name())
        .chain(command.args())
        .map(|word| static_word_text_arena(word, source).map(|text| text.into_owned()))
        .collect::<Vec<_>>();
    let Some((effect_callee, effect_index)) =
        normalize_recorded_zsh_effect_command_arena(&static_words)
    else {
        return info;
    };
    let args = static_words.get(effect_index + 1..).unwrap_or(&[]);
    match effect_callee.as_str() {
        "emulate" => info.zsh_effects = parse_emulate_effects_static(args),
        "setopt" => {
            info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                updates: parse_setopt_updates_static(args, true),
            }];
        }
        "unsetopt" => {
            info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                updates: parse_setopt_updates_static(args, false),
            }];
        }
        "set" => {
            let updates = parse_set_builtin_option_updates_static(args);
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

fn recorded_simple_command_info(
    command: &shuck_ast::SimpleCommand,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> RecordedCommandInfo {
    let words = std::iter::once(&command.name)
        .chain(command.args.iter())
        .collect::<Vec<_>>();
    let mut static_callee =
        static_command_name_text(&command.name, source).map(|name| name.into_owned());
    let static_args = command
        .args
        .iter()
        .map(|word| static_word_text(word, source).map(|text| text.into_owned()))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let _ = bash_runtime_vars_enabled;

    if static_callee.as_deref() == Some("noglob") {
        static_callee = words
            .get(1)
            .and_then(|word| static_command_name_text(word, source).map(|name| name.into_owned()));
    }

    let mut info = RecordedCommandInfo {
        static_callee,
        static_args,
        source_path_template: None,
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

fn source_path_template_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> Option<SourcePathTemplate> {
    if static_word_text_arena(word, source).is_some() {
        return None;
    }

    let mut parts = Vec::new();
    let mut ignored_root = false;
    let mut saw_dynamic = false;

    collect_source_template_parts_arena(
        word.parts(),
        word.store(),
        source,
        bash_runtime_vars_enabled,
        &mut parts,
        &mut ignored_root,
        &mut saw_dynamic,
    )
    .then_some(())?;

    (saw_dynamic && !parts.is_empty()).then_some(SourcePathTemplate::Interpolated(parts))
}

fn collect_source_template_parts_arena(
    word_parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    bash_runtime_vars_enabled: bool,
    parts: &mut Vec<TemplatePart>,
    ignored_root: &mut bool,
    saw_dynamic: &mut bool,
) -> bool {
    for part in word_parts {
        match &part.kind {
            WordPartArena::Literal(text) => {
                let text = text.as_str(source, part.span);
                if !text.is_empty() {
                    push_source_template_literal_arena(parts, text.to_owned());
                }
            }
            WordPartArena::SingleQuoted { value, .. } => {
                let text = value.slice(source);
                if !text.is_empty() {
                    push_source_template_literal_arena(parts, text.to_owned());
                }
            }
            WordPartArena::DoubleQuoted { parts: inner, .. } => {
                if !collect_source_template_parts_arena(
                    store.word_parts(*inner),
                    store,
                    source,
                    bash_runtime_vars_enabled,
                    parts,
                    ignored_root,
                    saw_dynamic,
                ) {
                    return false;
                }
            }
            WordPartArena::Variable(name) => {
                if let Some(index) = source_template_positional_index_arena(name) {
                    *saw_dynamic = true;
                    parts.push(TemplatePart::Arg(index));
                } else if bash_runtime_vars_enabled
                    && source_template_is_bash_source_var_arena(name)
                {
                    *saw_dynamic = true;
                    parts.push(TemplatePart::SourceFile);
                } else if !*ignored_root && parts.is_empty() {
                    *ignored_root = true;
                    *saw_dynamic = true;
                } else {
                    return false;
                }
            }
            WordPartArena::Parameter(parameter)
                if bash_runtime_vars_enabled
                    && source_template_parameter_is_current_source_file_arena(
                        parameter, source,
                    ) =>
            {
                *saw_dynamic = true;
                parts.push(TemplatePart::SourceFile);
            }
            WordPartArena::ArrayAccess(reference)
                if bash_runtime_vars_enabled
                    && source_template_is_bash_source_index_ref_arena(reference, source) =>
            {
                *saw_dynamic = true;
                parts.push(TemplatePart::SourceFile);
            }
            WordPartArena::CommandSubstitution { body, .. } => {
                if bash_runtime_vars_enabled
                    && let Some(template_part) =
                        dirname_source_template_part_arena(store.stmt_seq(*body), store, source)
                {
                    *saw_dynamic = true;
                    parts.push(template_part);
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

fn push_source_template_literal_arena(parts: &mut Vec<TemplatePart>, text: String) {
    if let Some(TemplatePart::Literal(existing)) = parts.last_mut() {
        existing.push_str(&text);
    } else {
        parts.push(TemplatePart::Literal(text));
    }
}

fn source_template_positional_index_arena(name: &Name) -> Option<usize> {
    name.as_str().parse().ok()
}

fn source_template_is_bash_source_var_arena(name: &Name) -> bool {
    name.as_str() == "BASH_SOURCE"
}

fn source_template_parameter_is_current_source_file_arena(
    parameter: &ParameterExpansionNode,
    source: &str,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntaxNode::Bourne(BourneParameterExpansionNode::Access {
            reference,
        }) => source_template_is_current_source_reference_arena(reference, source),
        ParameterExpansionSyntaxNode::Bourne(
            BourneParameterExpansionNode::Length { .. }
            | BourneParameterExpansionNode::Indices { .. }
            | BourneParameterExpansionNode::Indirect { .. }
            | BourneParameterExpansionNode::PrefixMatch { .. }
            | BourneParameterExpansionNode::Slice { .. }
            | BourneParameterExpansionNode::Operation { .. }
            | BourneParameterExpansionNode::Transformation { .. },
        )
        | ParameterExpansionSyntaxNode::Zsh(_) => false,
    }
}

fn source_template_is_current_source_reference_arena(reference: &VarRefNode, source: &str) -> bool {
    source_template_is_bash_source_var_arena(&reference.name)
        && reference.subscript.as_ref().is_none_or(|subscript| {
            source_template_subscript_is_semantic_zero_arena(subscript, None, source)
        })
}

fn source_template_is_bash_source_index_ref_arena(reference: &VarRefNode, source: &str) -> bool {
    source_template_is_bash_source_var_arena(&reference.name)
        && reference.subscript.as_ref().is_some_and(|subscript| {
            source_template_subscript_is_semantic_zero_arena(subscript, None, source)
        })
}

fn source_template_subscript_is_semantic_zero_arena(
    subscript: &SubscriptNode,
    store: Option<&AstStore>,
    source: &str,
) -> bool {
    subscript.arithmetic_ast.as_ref().is_some_and(|expr| {
        source_template_arithmetic_expr_is_semantic_zero_arena(expr, store, source)
    })
}

fn source_template_arithmetic_expr_is_semantic_zero_arena(
    expr: &ArithmeticExprArenaNode,
    store: Option<&AstStore>,
    source: &str,
) -> bool {
    match &expr.kind {
        ArithmeticExprArena::Number(text) => {
            source_template_shell_zero_literal_arena(text.slice(source))
        }
        ArithmeticExprArena::ShellWord(word) => store
            .map(|store| {
                source_template_word_is_semantic_zero_arena(store.word(*word), store, source)
            })
            .unwrap_or(false),
        ArithmeticExprArena::Parenthesized { expression } => {
            source_template_arithmetic_expr_is_semantic_zero_arena(expression, store, source)
        }
        ArithmeticExprArena::Unary { expr, .. } => {
            source_template_arithmetic_expr_is_semantic_zero_arena(expr, store, source)
        }
        _ => false,
    }
}

fn source_template_word_is_semantic_zero_arena(
    word: shuck_ast::WordView<'_>,
    store: &AstStore,
    source: &str,
) -> bool {
    matches!(
        word.parts(),
        [part] if source_template_word_part_is_semantic_zero_arena(part, store, source)
    )
}

fn source_template_word_part_is_semantic_zero_arena(
    part: &WordPartArenaNode,
    store: &AstStore,
    source: &str,
) -> bool {
    match &part.kind {
        WordPartArena::Literal(text) => {
            source_template_shell_zero_literal_arena(text.as_str(source, part.span))
        }
        WordPartArena::SingleQuoted { value, .. } => {
            source_template_shell_zero_literal_arena(value.slice(source))
        }
        WordPartArena::DoubleQuoted { parts, .. } => {
            matches!(
                store.word_parts(*parts),
                [part] if source_template_word_part_is_semantic_zero_arena(part, store, source)
            )
        }
        WordPartArena::ArithmeticExpansion {
            expression_ast: Some(expr),
            ..
        } => source_template_arithmetic_expr_is_semantic_zero_arena(expr, Some(store), source),
        _ => false,
    }
}

fn source_template_shell_zero_literal_arena(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let digits = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    if digits.is_empty() {
        return false;
    }

    if let Some((base, value)) = digits.split_once('#') {
        return base.parse::<u32>().is_ok_and(|base| {
            (2..=64).contains(&base) && !value.is_empty() && value.chars().all(|ch| ch == '0')
        });
    }

    let digits = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
        .unwrap_or(digits);
    !digits.is_empty() && digits.chars().all(|ch| ch == '0')
}

fn dirname_source_template_part_arena(
    commands: StmtSeqView<'_>,
    store: &AstStore,
    source: &str,
) -> Option<TemplatePart> {
    let mut stmts = commands.stmts();
    let stmt = stmts.next()?;
    if stmts.next().is_some() {
        return None;
    }
    let command = stmt.command().simple()?;
    if stmt.negated() || !stmt.redirects().is_empty() || !command.assignments().is_empty() {
        return None;
    }
    let args = command.args().collect::<Vec<_>>();
    if args.len() != 1 {
        return None;
    }
    if static_word_text_arena(command.name(), source).as_deref() != Some("dirname") {
        return None;
    }
    current_source_file_word_arena(args[0], store, source).then_some(TemplatePart::SourceDir)
}

fn current_source_file_word_arena(
    word: shuck_ast::WordView<'_>,
    store: &AstStore,
    source: &str,
) -> bool {
    matches!(
        word.parts(),
        [part] if is_current_source_part_arena(&part.kind, store, source)
    )
}

fn is_current_source_part_arena(part: &WordPartArena, store: &AstStore, source: &str) -> bool {
    match part {
        WordPartArena::Variable(name) => source_template_is_bash_source_var_arena(name),
        WordPartArena::Parameter(parameter) => {
            source_template_parameter_is_current_source_file_arena(parameter, source)
        }
        WordPartArena::ArrayAccess(reference) => {
            source_template_is_bash_source_index_ref_arena(reference, source)
        }
        WordPartArena::DoubleQuoted { parts, .. } => {
            matches!(
                store.word_parts(*parts),
                [part] if is_current_source_part_arena(&part.kind, store, source)
            )
        }
        _ => false,
    }
}

fn normalize_recorded_zsh_effect_command(words: &[&Word], source: &str) -> Option<(String, usize)> {
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        let text = static_word_text(word, source)?;
        if is_recorded_assignment_word(&text) {
            index += 1;
            continue;
        }

        match static_command_wrapper_target_index(words.len(), index, text.as_ref(), |word_index| {
            static_word_text(words[word_index], source)
        }) {
            StaticCommandWrapperTarget::NotWrapper => return Some((text.into_owned(), index)),
            StaticCommandWrapperTarget::Wrapper {
                target_index: Some(target_index),
            } => {
                index = target_index;
                continue;
            }
            StaticCommandWrapperTarget::Wrapper { target_index: None } => return None,
        }
    }

    None
}

fn normalize_recorded_zsh_effect_command_arena(
    words: &[Option<String>],
) -> Option<(String, usize)> {
    let mut index = 0usize;

    while let Some(text) = words.get(index).and_then(|text| text.as_deref()) {
        if is_recorded_assignment_word(text) {
            index += 1;
            continue;
        }

        match static_command_wrapper_target_index(words.len(), index, text, |word_index| {
            words[word_index].as_deref().map(Cow::Borrowed)
        }) {
            StaticCommandWrapperTarget::NotWrapper => return Some((text.to_owned(), index)),
            StaticCommandWrapperTarget::Wrapper {
                target_index: Some(target_index),
            } => {
                index = target_index;
                continue;
            }
            StaticCommandWrapperTarget::Wrapper { target_index: None } => return None,
        }
    }

    None
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

        match text.as_ref() {
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

fn parse_emulate_effects_static(args: &[Option<String>]) -> Vec<RecordedZshCommandEffect> {
    let mut local = false;
    let mut mode = None;
    let mut updates = Vec::new();
    let mut index = 0usize;

    while let Some(Some(text)) = args.get(index) {
        match text.as_str() {
            "--" => break,
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if let Some(Some(option)) = args.get(index + 1)
                    && let Some(update) = parse_recorded_zsh_option_update(option, enable)
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
                if flag == 'L' {
                    local = true;
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

fn parse_setopt_updates_static(
    args: &[Option<String>],
    enable: bool,
) -> Vec<RecordedZshOptionUpdate> {
    args.iter()
        .filter_map(|text| text.as_deref())
        .filter(|text| *text != "--")
        .filter_map(|text| parse_recorded_zsh_option_update(text, enable))
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

        match text.as_ref() {
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

fn parse_set_builtin_option_updates_static(
    args: &[Option<String>],
) -> Vec<RecordedZshOptionUpdate> {
    let mut updates = Vec::new();
    let mut index = 0usize;

    while let Some(Some(text)) = args.get(index) {
        match text.as_str() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if let Some(Some(name)) = args.get(index + 1)
                    && let Some(update) = parse_recorded_zsh_option_update(name, enable)
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

fn classify_dynamic_source_word_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
) -> SourceRefKind {
    let mut variable = None;
    let mut tail = String::new();

    for part in word.parts() {
        match &part.kind {
            WordPartArena::Literal(text) => tail.push_str(text.as_str(source, part.span)),
            WordPartArena::Variable(name) if variable.is_none() && tail.is_empty() => {
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

fn classify_source_ref_diagnostic_class_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
    kind: &SourceRefKind,
) -> SourceRefDiagnosticClass {
    match kind {
        SourceRefKind::Literal(path)
            if literal_uses_current_user_home_tilde_arena(word, source, path) =>
        {
            SourceRefDiagnosticClass::DynamicPath
        }
        SourceRefKind::Dynamic if dynamic_root_with_slash_tail_arena(word, source) => {
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

fn literal_uses_current_user_home_tilde_arena(
    word: shuck_ast::WordView<'_>,
    source: &str,
    path: &str,
) -> bool {
    if !path.starts_with("~/") {
        return false;
    }

    let Some((first, tail)) = word.parts().split_first() else {
        return false;
    };

    match &first.kind {
        WordPartArena::Literal(_) => {
            let text = first.span.slice(source);
            text.starts_with("~/")
                || (text == "~"
                    && static_parts_text_arena(tail, source)
                        .is_some_and(|tail| tail.starts_with('/')))
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

fn dynamic_root_with_slash_tail_arena(word: shuck_ast::WordView<'_>, source: &str) -> bool {
    let Some((root, tail)) = word.parts().split_first() else {
        return false;
    };

    match &root.kind {
        WordPartArena::DoubleQuoted { parts, .. } => {
            let Some((inner_root, inner_tail)) = word.store().word_parts(*parts).split_first()
            else {
                return false;
            };

            root_word_part_is_dynamic_root_arena(&inner_root.kind)
                && static_tail_text_starts_with_slash_arena(inner_tail, tail, source)
        }
        _ => {
            root_word_part_is_dynamic_root_arena(&root.kind)
                && static_tail_text_starts_with_slash_arena(tail, &[], source)
        }
    }
}

fn root_word_part_is_dynamic_root(part: &WordPart) -> bool {
    matches!(
        part,
        WordPart::Variable(_)
            | WordPart::ArrayAccess(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
    )
}

fn root_word_part_is_dynamic_root_arena(part: &WordPartArena) -> bool {
    matches!(
        part,
        WordPartArena::Variable(_)
            | WordPartArena::ArrayAccess(_)
            | WordPartArena::Parameter(_)
            | WordPartArena::CommandSubstitution { .. }
    )
}

fn static_parts_text(parts: &[WordPartNode], source: &str) -> Option<String> {
    try_static_word_parts_text(parts, source).map(|text| text.into_owned())
}

fn static_parts_text_arena(parts: &[WordPartArenaNode], source: &str) -> Option<String> {
    // This helper is only used for tails already paired with a `WordView`; callers that
    // need nested double-quoted tails should use `static_tail_text_starts_with_slash_arena`.
    parts
        .iter()
        .map(|part| match &part.kind {
            WordPartArena::Literal(text) => Some(text.as_str(source, part.span)),
            WordPartArena::SingleQuoted { value, .. } => Some(value.slice(source)),
            WordPartArena::DoubleQuoted { .. } => None,
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.concat())
}

fn static_tail_text_starts_with_slash(
    parts: &[WordPartNode],
    trailing: &[WordPartNode],
    source: &str,
) -> bool {
    let Some(prefix) = try_static_word_parts_text(parts, source) else {
        return false;
    };
    if !prefix.is_empty() {
        return prefix.starts_with('/');
    }

    try_static_word_parts_text(trailing, source).is_some_and(|text| text.starts_with('/'))
}

fn static_tail_text_starts_with_slash_arena(
    parts: &[WordPartArenaNode],
    trailing: &[WordPartArenaNode],
    source: &str,
) -> bool {
    let Some(prefix) = try_static_tail_parts_text_arena(parts, source) else {
        return false;
    };
    if !prefix.is_empty() {
        return prefix.starts_with('/');
    }

    try_static_tail_parts_text_arena(trailing, source).is_some_and(|text| text.starts_with('/'))
}

fn try_static_tail_parts_text_arena<'a>(
    parts: &[WordPartArenaNode],
    source: &'a str,
) -> Option<Cow<'a, str>> {
    if parts.is_empty() {
        return Some(Cow::Borrowed(""));
    }
    let mut text = String::new();
    for part in parts {
        match &part.kind {
            WordPartArena::Literal(literal) => text.push_str(literal.as_str(source, part.span)),
            WordPartArena::SingleQuoted { value, .. } => text.push_str(value.slice(source)),
            WordPartArena::DoubleQuoted { .. } => return None,
            _ => return None,
        }
    }
    Some(Cow::Owned(text))
}

fn unset_flags_are_valid(flags: &str) -> bool {
    !flags.is_empty() && flags.chars().all(|flag| matches!(flag, 'f' | 'v' | 'n'))
}

fn parse_source_directives(
    source: &str,
    indexer: &Indexer,
) -> BTreeMap<usize, SourceDirectiveOverride> {
    let mut directives = BTreeMap::new();
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

fn arithmetic_lvalue_span(target: &ArithmeticLvalue, span: Span) -> Span {
    match target {
        ArithmeticLvalue::Variable(name) => arithmetic_name_span(span, name),
        ArithmeticLvalue::Indexed { index, .. } => {
            Span::from_positions(span.start, index.span.end.advanced_by("]"))
        }
    }
}

fn arithmetic_lvalue_span_arena(target: &ArithmeticLvalueArena, span: Span) -> Span {
    match target {
        ArithmeticLvalueArena::Variable(name) => arithmetic_name_span(span, name),
        ArithmeticLvalueArena::Indexed { index, .. } => {
            Span::from_positions(span.start, index.span.end.advanced_by("]"))
        }
    }
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

fn depth_from_static_text(text: Option<Cow<'_, str>>) -> usize {
    text.as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|depth| *depth > 0)
        .unwrap_or(1)
}

fn subscript_selector_arena(subscript: &SubscriptNode) -> Option<shuck_ast::SubscriptSelector> {
    match subscript.kind {
        shuck_ast::SubscriptKind::Ordinary => None,
        shuck_ast::SubscriptKind::Selector(selector) => Some(selector),
    }
}

fn subscript_span_arena(subscript: &SubscriptNode) -> Span {
    subscript.text.span()
}

fn subscript_syntax_source_text_arena(subscript: &SubscriptNode) -> &SourceText {
    subscript.raw.as_ref().unwrap_or(&subscript.text)
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

fn binding_origin_for_assignment(assignment: &Assignment, source: &str) -> BindingOrigin {
    let value = if assignment.target.subscript.is_some() {
        AssignmentValueOrigin::ArrayOrCompound
    } else {
        match &assignment.value {
            AssignmentValue::Scalar(word) => assignment_value_origin_for_word(word),
            AssignmentValue::Compound(_) => AssignmentValueOrigin::ArrayOrCompound,
        }
    };

    BindingOrigin::Assignment {
        definition_span: assignment_target_span(assignment, source),
        value,
    }
}

fn binding_origin_for_assignment_arena(
    assignment: &AssignmentNode,
    store: &AstStore,
    source: &str,
) -> BindingOrigin {
    let value = if assignment.target.subscript.is_some() {
        AssignmentValueOrigin::ArrayOrCompound
    } else {
        match &assignment.value {
            AssignmentValueNode::Scalar(word) => {
                assignment_value_origin_for_word_arena(store.word(*word))
            }
            AssignmentValueNode::Compound(_) => AssignmentValueOrigin::ArrayOrCompound,
        }
    };

    BindingOrigin::Assignment {
        definition_span: assignment_target_span_arena(assignment, source),
        value,
    }
}

fn assignment_target_span(assignment: &Assignment, source: &str) -> Span {
    let Some(subscript) = assignment.target.subscript.as_deref() else {
        return assignment.target.name_span;
    };

    let subscript_end = subscript.syntax_source_text().span().end;
    if source
        .get(subscript_end.offset..)
        .is_some_and(|rest| rest.starts_with(']'))
    {
        return Span::from_positions(
            assignment.target.name_span.start,
            subscript_end.advanced_by("]"),
        );
    }

    assignment.target.name_span
}

fn assignment_target_span_arena(assignment: &AssignmentNode, source: &str) -> Span {
    let Some(subscript) = assignment.target.subscript.as_ref() else {
        return assignment.target.name_span;
    };

    let subscript_end = subscript_syntax_source_text_arena(subscript).span().end;
    if source
        .get(subscript_end.offset..)
        .is_some_and(|rest| rest.starts_with(']'))
    {
        return Span::from_positions(
            assignment.target.name_span.start,
            subscript_end.advanced_by("]"),
        );
    }

    assignment.target.name_span
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

fn loop_binding_origin_for_static_texts<'a>(
    words: impl IntoIterator<Item = Option<Cow<'a, str>>>,
) -> LoopValueOrigin {
    if words.into_iter().all(|word| word.is_some()) {
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

fn assignment_value_origin_for_word_arena(word: shuck_ast::WordView<'_>) -> AssignmentValueOrigin {
    if !word.brace_syntax().is_empty() {
        return AssignmentValueOrigin::MixedDynamic;
    }
    if word_is_static_binding_literal_arena(word) {
        return AssignmentValueOrigin::StaticLiteral;
    }

    let mut scan = AssignmentWordOriginScan::default();
    for part in word.parts() {
        scan_assignment_word_part_arena(&part.kind, word.store(), &mut scan);
    }

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

fn word_is_static_binding_literal_arena(word: shuck_ast::WordView<'_>) -> bool {
    word.brace_syntax().is_empty()
        && word
            .parts()
            .iter()
            .all(|part| binding_literal_part_is_static_arena(&part.kind, word.store()))
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

fn binding_literal_part_is_static_arena(part: &WordPartArena, store: &AstStore) -> bool {
    match part {
        WordPartArena::Literal(_) | WordPartArena::SingleQuoted { .. } => true,
        WordPartArena::DoubleQuoted { parts, .. } => store
            .word_parts(*parts)
            .iter()
            .all(|part| binding_literal_part_is_static_arena(&part.kind, store)),
        WordPartArena::ZshQualifiedGlob(_)
        | WordPartArena::Variable(_)
        | WordPartArena::CommandSubstitution { .. }
        | WordPartArena::ArithmeticExpansion { .. }
        | WordPartArena::Parameter(_)
        | WordPartArena::ParameterExpansion { .. }
        | WordPartArena::Length(_)
        | WordPartArena::ArrayAccess(_)
        | WordPartArena::ArrayLength(_)
        | WordPartArena::ArrayIndices(_)
        | WordPartArena::Substring { .. }
        | WordPartArena::ArraySlice { .. }
        | WordPartArena::IndirectExpansion { .. }
        | WordPartArena::PrefixMatch { .. }
        | WordPartArena::ProcessSubstitution { .. }
        | WordPartArena::Transformation { .. } => false,
    }
}

fn scan_assignment_word_parts(parts: &[WordPartNode], scan: &mut AssignmentWordOriginScan) {
    for part in parts {
        scan_assignment_word_part(&part.kind, scan);
    }
}

fn scan_assignment_word_part_arena(
    part: &WordPartArena,
    store: &AstStore,
    scan: &mut AssignmentWordOriginScan,
) {
    match part {
        WordPartArena::Literal(_)
        | WordPartArena::SingleQuoted { .. }
        | WordPartArena::Variable(_)
        | WordPartArena::ArithmeticExpansion { .. } => {}
        WordPartArena::DoubleQuoted { parts, .. } => {
            for part in store.word_parts(*parts) {
                scan_assignment_word_part_arena(&part.kind, store, scan);
            }
        }
        WordPartArena::ParameterExpansion { reference, .. } => {
            if reference.subscript.is_some() {
                scan.array_or_compound = true;
            } else {
                scan.parameter_operator = true;
            }
        }
        WordPartArena::Transformation { .. } => scan.transformation = true,
        WordPartArena::IndirectExpansion { .. } | WordPartArena::ArrayIndices(_) => {
            scan.indirect_expansion = true;
        }
        WordPartArena::CommandSubstitution { .. } | WordPartArena::ProcessSubstitution { .. } => {
            scan.command_or_process_substitution = true;
        }
        WordPartArena::ArrayAccess(_)
        | WordPartArena::ArrayLength(_)
        | WordPartArena::Substring { .. }
        | WordPartArena::ArraySlice { .. } => scan.array_or_compound = true,
        WordPartArena::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntaxNode::Bourne(BourneParameterExpansionNode::Access {
                ..
            }) => {}
            ParameterExpansionSyntaxNode::Bourne(
                BourneParameterExpansionNode::Transformation { .. },
            ) => scan.transformation = true,
            ParameterExpansionSyntaxNode::Bourne(BourneParameterExpansionNode::Indirect {
                ..
            }) => scan.indirect_expansion = true,
            ParameterExpansionSyntaxNode::Bourne(BourneParameterExpansionNode::Operation {
                ..
            }) => scan.parameter_operator = true,
            ParameterExpansionSyntaxNode::Bourne(
                BourneParameterExpansionNode::Length { .. }
                | BourneParameterExpansionNode::Indices { .. }
                | BourneParameterExpansionNode::PrefixMatch { .. }
                | BourneParameterExpansionNode::Slice { .. },
            )
            | ParameterExpansionSyntaxNode::Zsh(_) => scan.mixed_dynamic = true,
        },
        WordPartArena::ZshQualifiedGlob(_)
        | WordPartArena::Length(_)
        | WordPartArena::PrefixMatch { .. } => scan.mixed_dynamic = true,
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

fn case_arm_matches_anything_arena(patterns: &[PatternNode], store: &AstStore) -> bool {
    patterns
        .iter()
        .any(|pattern| pattern_matches_anything_arena(pattern, store))
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

fn pattern_matches_anything_arena(pattern: &PatternNode, store: &AstStore) -> bool {
    let parts = store.pattern_parts(pattern.parts);
    !parts.is_empty()
        && parts
            .iter()
            .all(|part| pattern_part_can_match_empty_arena(&part.kind, store))
        && parts
            .iter()
            .any(|part| pattern_part_matches_anything_arena(&part.kind, store))
}

fn pattern_can_match_empty(pattern: &Pattern) -> bool {
    pattern
        .parts
        .iter()
        .all(|part| pattern_part_can_match_empty(&part.kind))
}

fn pattern_can_match_empty_arena(pattern: &PatternNode, store: &AstStore) -> bool {
    store
        .pattern_parts(pattern.parts)
        .iter()
        .all(|part| pattern_part_can_match_empty_arena(&part.kind, store))
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

fn pattern_part_matches_anything_arena(part: &PatternPartArena, store: &AstStore) -> bool {
    match part {
        PatternPartArena::AnyString => true,
        PatternPartArena::Group { kind, patterns } => {
            pattern_group_matches_anything_arena(*kind, store.patterns(*patterns), store)
        }
        PatternPartArena::Literal(_)
        | PatternPartArena::AnyChar
        | PatternPartArena::CharClass(_)
        | PatternPartArena::Word(_) => false,
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

fn pattern_part_can_match_empty_arena(part: &PatternPartArena, store: &AstStore) -> bool {
    match part {
        PatternPartArena::AnyString => true,
        PatternPartArena::Group { kind, patterns } => {
            pattern_group_can_match_empty_arena(*kind, store.patterns(*patterns), store)
        }
        PatternPartArena::Literal(_)
        | PatternPartArena::AnyChar
        | PatternPartArena::CharClass(_)
        | PatternPartArena::Word(_) => false,
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

fn pattern_group_matches_anything_arena(
    kind: PatternGroupKind,
    patterns: &[PatternNode],
    store: &AstStore,
) -> bool {
    match kind {
        PatternGroupKind::ZeroOrOne
        | PatternGroupKind::ZeroOrMore
        | PatternGroupKind::OneOrMore
        | PatternGroupKind::ExactlyOne => patterns
            .iter()
            .any(|pattern| pattern_matches_anything_arena(pattern, store)),
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

fn pattern_group_can_match_empty_arena(
    kind: PatternGroupKind,
    patterns: &[PatternNode],
    store: &AstStore,
) -> bool {
    match kind {
        PatternGroupKind::ZeroOrOne | PatternGroupKind::ZeroOrMore => true,
        PatternGroupKind::OneOrMore | PatternGroupKind::ExactlyOne => patterns
            .iter()
            .any(|pattern| pattern_can_match_empty_arena(pattern, store)),
        PatternGroupKind::NoneOf => false,
    }
}

fn word_is_semantically_inert(word: &Word) -> bool {
    word.parts
        .iter()
        .all(|part| word_part_is_semantically_inert(&part.kind))
}

fn word_is_semantically_inert_arena(word: shuck_ast::WordView<'_>, store: &AstStore) -> bool {
    word.parts()
        .iter()
        .all(|part| word_part_is_semantically_inert_arena(&part.kind, store))
}

fn heredoc_body_is_semantically_inert(body: &HeredocBody, source: &str) -> bool {
    body.parts
        .iter()
        .all(|part| heredoc_body_part_is_semantically_inert(&part.kind, part.span, source))
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

fn word_part_is_semantically_inert_arena(part: &WordPartArena, store: &AstStore) -> bool {
    match part {
        WordPartArena::Literal(_) | WordPartArena::SingleQuoted { .. } => true,
        WordPartArena::ZshQualifiedGlob(glob) => {
            store
                .zsh_glob_segments(glob.segments)
                .iter()
                .all(|segment| match segment {
                    shuck_ast::ZshGlobSegmentNode::Pattern(pattern) => {
                        pattern_is_semantically_inert_arena(pattern, store)
                    }
                    shuck_ast::ZshGlobSegmentNode::InlineControl(_) => true,
                })
        }
        WordPartArena::DoubleQuoted { parts, .. } => store
            .word_parts(*parts)
            .iter()
            .all(|part| word_part_is_semantically_inert_arena(&part.kind, store)),
        WordPartArena::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_none(),
        WordPartArena::Variable(_)
        | WordPartArena::CommandSubstitution { .. }
        | WordPartArena::Parameter(_)
        | WordPartArena::ParameterExpansion { .. }
        | WordPartArena::Length(_)
        | WordPartArena::ArrayAccess(_)
        | WordPartArena::ArrayLength(_)
        | WordPartArena::ArrayIndices(_)
        | WordPartArena::Substring { .. }
        | WordPartArena::ArraySlice { .. }
        | WordPartArena::IndirectExpansion { .. }
        | WordPartArena::PrefixMatch { .. }
        | WordPartArena::ProcessSubstitution { .. }
        | WordPartArena::Transformation { .. } => false,
    }
}

fn heredoc_body_part_is_semantically_inert(
    part: &HeredocBodyPart,
    span: Span,
    source: &str,
) -> bool {
    match part {
        HeredocBodyPart::Literal(text) => {
            !text.is_source_backed()
                || !escaped_braced_literal_may_contain_reference(text.syntax_str(source, span))
        }
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

fn pattern_is_semantically_inert_arena(pattern: &PatternNode, store: &AstStore) -> bool {
    store
        .pattern_parts(pattern.parts)
        .iter()
        .all(|part| pattern_part_is_semantically_inert_arena(&part.kind, store))
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

fn pattern_part_is_semantically_inert_arena(part: &PatternPartArena, store: &AstStore) -> bool {
    match part {
        PatternPartArena::Literal(_)
        | PatternPartArena::AnyString
        | PatternPartArena::AnyChar
        | PatternPartArena::CharClass(_) => true,
        PatternPartArena::Group { patterns, .. } => store
            .patterns(*patterns)
            .iter()
            .all(|pattern| pattern_is_semantically_inert_arena(pattern, store)),
        PatternPartArena::Word(word) => word_is_semantically_inert_arena(store.word(*word), store),
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

fn collect_pipeline_segments_arena<'a>(
    seq: StmtSeqView<'a>,
    out: &mut SmallVec<[StmtView<'a>; 4]>,
) {
    for stmt in seq.stmts() {
        match stmt.command().binary() {
            Some(command) if matches!(command.op(), BinaryOp::Pipe | BinaryOp::PipeAll) => {
                collect_pipeline_segments_arena(command.left(), out);
                collect_pipeline_segments_arena(command.right(), out);
            }
            _ => out.push(stmt),
        }
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

fn collect_logical_segments_arena<'a>(
    seq: StmtSeqView<'a>,
    commands: &mut SmallVec<[StmtView<'a>; 4]>,
    operators: &mut SmallVec<[RecordedListOperator; 4]>,
) {
    for stmt in seq.stmts() {
        match stmt.command().binary() {
            Some(command) if matches!(command.op(), BinaryOp::And | BinaryOp::Or) => {
                collect_logical_segments_arena(command.left(), commands, operators);
                operators.push(recorded_list_operator(command.op()));
                collect_logical_segments_arena(command.right(), commands, operators);
            }
            _ => commands.push(stmt),
        }
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

fn source_line_start_offsets(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (offset, ch) in source.char_indices() {
        if ch == '\n' {
            starts.push(offset + ch.len_utf8());
        }
    }
    starts
}

fn source_position_at_offset(
    source: &str,
    line_start_offsets: &[usize],
    offset: usize,
) -> Option<Position> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }

    let line_index = line_start_offsets
        .partition_point(|line_start| *line_start <= offset)
        .checked_sub(1)?;
    let line_start = *line_start_offsets.get(line_index)?;
    let column = source.get(line_start..offset)?.chars().count() + 1;
    Some(Position {
        line: line_index + 1,
        column,
        offset,
    })
}

fn reference_kind_uses_braced_parameter_syntax(kind: ReferenceKind) -> bool {
    matches!(
        kind,
        ReferenceKind::Expansion
            | ReferenceKind::ParameterExpansion
            | ReferenceKind::Length
            | ReferenceKind::ArrayAccess
            | ReferenceKind::IndirectExpansion
            | ReferenceKind::RequiredRead
    )
}

fn unbraced_parameter_reference_matches(text: &str, name: &str) -> bool {
    let Some(rest) = text.strip_prefix('$') else {
        return false;
    };
    if rest.starts_with('{') || !rest.starts_with(name) {
        return false;
    }

    rest.get(name.len()..)
        .and_then(|suffix| suffix.chars().next())
        .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
}

fn unbraced_parameter_start_matches(source: &str, start_offset: usize, name: &str) -> bool {
    let Some(candidate) = source.get(start_offset..) else {
        return false;
    };

    unbraced_parameter_reference_matches(candidate, name)
}

fn braced_parameter_start_matches(source: &str, start_offset: usize, name: &str) -> bool {
    let Some(after_name) = start_offset
        .checked_add("${".len())
        .and_then(|offset| offset.checked_add(name.len()))
    else {
        return false;
    };
    if after_name > source.len() || !source.is_char_boundary(after_name) {
        return false;
    }

    source
        .get(after_name..)
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
}

fn braced_parameter_end_offset(
    source: &str,
    start_offset: usize,
    search_end: usize,
) -> Option<usize> {
    if start_offset >= search_end
        || search_end > source.len()
        || !source.is_char_boundary(start_offset)
        || !source.is_char_boundary(search_end)
        || source
            .as_bytes()
            .get(start_offset..start_offset + "${".len())?
            != b"${"
    {
        return None;
    }

    let mut depth = 1usize;
    let mut offset = start_offset + "${".len();
    while offset < search_end {
        let ch = source.get(offset..search_end)?.chars().next()?;
        let next_offset = offset + ch.len_utf8();
        if ch == '\\' {
            offset = source
                .get(next_offset..search_end)
                .and_then(|suffix| suffix.chars().next())
                .map(|escaped| next_offset + escaped.len_utf8())
                .unwrap_or(next_offset);
            continue;
        }
        if ch == '$' && source.as_bytes().get(next_offset) == Some(&b'{') {
            depth += 1;
            offset = next_offset + '{'.len_utf8();
            continue;
        }
        if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(next_offset);
            }
        }
        offset = next_offset;
    }

    None
}

fn source_line(source: &str, target_line: usize) -> Option<(usize, &str)> {
    if target_line == 0 {
        return None;
    }

    let mut line_start = 0;
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let line_number = index + 1;
        if line_number == target_line {
            let line = line.strip_suffix('\n').unwrap_or(line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            return Some((line_start, line));
        }
        line_start += line.len();
    }

    if target_line == source.split_inclusive('\n').count() + 1 && line_start == source.len() {
        return Some((line_start, ""));
    }

    None
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
    fn source_position_lookup_uses_precomputed_line_starts() {
        let source = "alpha\nb\u{e9}ta\n";
        let line_starts = source_line_start_offsets(source);

        assert_eq!(
            source_position_at_offset(source, &line_starts, 0),
            Some(Position {
                line: 1,
                column: 1,
                offset: 0
            })
        );
        let beta_offset = source.find('b').expect("expected second line");
        assert_eq!(
            source_position_at_offset(source, &line_starts, beta_offset),
            Some(Position {
                line: 2,
                column: 1,
                offset: beta_offset
            })
        );
        let after_e_acute = beta_offset + "b\u{e9}".len();
        assert_eq!(
            source_position_at_offset(source, &line_starts, after_e_acute),
            Some(Position {
                line: 2,
                column: 3,
                offset: after_e_acute
            })
        );
        assert_eq!(
            source_position_at_offset(source, &line_starts, source.len()),
            Some(Position {
                line: 3,
                column: 1,
                offset: source.len()
            })
        );
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
