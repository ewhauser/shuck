use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticAssignOp, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticLvalue, ArithmeticUnaryOp, ArrayElem, ArrayExpr, ArrayKind, Assignment,
    AssignmentValue, BinaryCommand, BinaryOp, BourneParameterExpansion, BuiltinCommand, Command,
    CompoundCommand, ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, DeclOperand, File,
    FunctionDef, HeredocBody, HeredocBodyPart, HeredocBodyPartNode, LiteralText, Name,
    NormalizedCommand, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern,
    PatternGroupKind, PatternPart, PatternPartNode, Position, SourceText, Span,
    StaticCommandWrapperTarget, Stmt, StmtSeq, Subscript, SubscriptInterpretation, VarRef, Word,
    WordPart, WordPartNode, WrapperKind, ZshExpansionOperation, ZshExpansionTarget, ZshGlobSegment,
    normalize_command_words, static_command_name_text, static_command_wrapper_target_index,
    static_word_text, try_static_word_parts_text,
};
use shuck_indexer::Indexer;
use shuck_parser::{ShellProfile, ZshEmulationMode, parser::Parser};
use smallvec::SmallVec;

use crate::binding::{
    AssignmentValueOrigin, Binding, BindingAttributes, BindingKind, BindingOrigin,
    BuiltinBindingTargetKind, LoopValueOrigin,
};
use crate::call_graph::{CallGraph, CallSite, build_call_graph};
use crate::cfg::{
    FlowContext, IsolatedRegion, RecordedCaseArm, RecordedCommand, RecordedCommandId,
    RecordedCommandInfo, RecordedCommandKind, RecordedCommandRange, RecordedElifBranch,
    RecordedListItem, RecordedListOperator, RecordedPipelineSegment, RecordedProgram,
    RecordedZshCommandEffect, RecordedZshOptionUpdate,
};
use crate::declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
use crate::reference::{Reference, ReferenceKind};
use crate::runtime::RuntimePrelude;
use crate::scope::ancestor_scopes;
use crate::source_closure::source_path_template;
use crate::source_ref::{
    SourceRef, SourceRefDiagnosticClass, SourceRefKind, SourceRefResolution,
    default_diagnostic_class,
};
use crate::{
    BindingId, FileEntryContractCollector, FunctionScopeKind, IndirectTargetHint, ReferenceId,
    Scope, ScopeId, ScopeKind, SourceDirectiveOverride, SpanKey, TraversalObserver,
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

pub(crate) struct SemanticModelBuilder<'a, 'observer> {
    source: &'a str,
    file_entry_contract_collector: Option<&'observer mut dyn FileEntryContractCollector>,
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
    deferred_functions: Vec<DeferredFunction<'a>>,
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

#[derive(Debug, Clone, Copy)]
struct DeferredFunction<'a> {
    function: &'a FunctionDef,
    scope: ScopeId,
    flow: FlowState,
}

mod arithmetic;
mod bindings;
mod declarations;
mod inert;
mod references;
mod source_refs;
mod special_builtins;
mod traversal;
mod words;
mod zsh_effects;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(crate) fn build(
        file: &'a File,
        source: &'a str,
        indexer: &'a Indexer,
        observer: &'observer mut dyn TraversalObserver,
        file_entry_contract_collector: Option<&'observer mut dyn FileEntryContractCollector>,
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
            file_entry_contract_collector,
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
        let file_commands = builder.visit_stmt_seq(&file.body, FlowState::default());
        builder.recorded_program.set_file_commands(file_commands);
        builder.mark_scope_completed(ScopeId(0));
        builder.drain_deferred_functions();

        let call_graph = build_call_graph(
            &builder.scopes,
            &builder.bindings,
            &builder.functions,
            &builder.call_sites,
        );
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
                operand_span: assignment.span,
                target_span: assignment_target_span(assignment, source),
                name_span: assignment.target.name_span,
                value_span: assignment_value_span(assignment),
                append: assignment.append,
                value_origin: assignment_value_origin(&assignment.value),
                has_command_substitution: assignment_value_has_command_substitution(
                    &assignment.value,
                ),
                has_command_or_process_substitution:
                    assignment_value_has_command_or_process_substitution(&assignment.value),
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

fn getopts_target(args: &[&Word], source: &str) -> Option<(Name, Span)> {
    args.get(1).and_then(|word| named_target_word(word, source))
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

fn span_is_escaped_parameter_template_name(word_span: Span, span: Span, source: &str) -> bool {
    if span.start.offset < word_span.start.offset || span.start.offset >= word_span.end.offset {
        return false;
    }

    let text = word_span.slice(source);
    let relative_offset = span.start.offset - word_span.start.offset;
    let mut index = 0usize;

    while index < text.len() {
        if text[index..].starts_with("\\${") {
            let dollar_offset = index + '\\'.len_utf8();
            if offset_is_backslash_escaped(word_span.start.offset + dollar_offset, source)
                && let Some(end_offset) = escaped_parameter_template_end(text, dollar_offset)
            {
                let body_start = dollar_offset + "${".len();
                let body_end = end_offset.saturating_sub('}'.len_utf8());
                if relative_offset == body_start
                    && relative_offset < body_end
                    && text[relative_offset..]
                        .chars()
                        .next()
                        .is_some_and(is_name_start_character)
                {
                    return true;
                }
                index = end_offset;
                continue;
            }
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();
    }

    false
}

fn escaped_parameter_template_end(text: &str, dollar_offset: usize) -> Option<usize> {
    if dollar_offset >= text.len() || !text[dollar_offset..].starts_with("${") {
        return None;
    }

    let bytes = text.as_bytes();
    let mut index = dollar_offset + "${".len();
    let mut depth = 1usize;
    let mut quote_state = EscapedTemplateQuote::None;

    while index < bytes.len() {
        let byte = bytes[index];
        match quote_state {
            EscapedTemplateQuote::Single => {
                if byte == b'\'' {
                    quote_state = EscapedTemplateQuote::None;
                }
                index += 1;
                continue;
            }
            EscapedTemplateQuote::Double => {
                if byte == b'\\' {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                    continue;
                }
                if byte == b'"' {
                    quote_state = EscapedTemplateQuote::None;
                }
                index += 1;
                continue;
            }
            EscapedTemplateQuote::None => {}
        }

        match byte {
            b'\\' => {
                index += usize::from(index + 1 < bytes.len()) + 1;
            }
            b'\'' => {
                quote_state = EscapedTemplateQuote::Single;
                index += 1;
            }
            b'"' => {
                quote_state = EscapedTemplateQuote::Double;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                depth += 1;
                index += "${".len();
            }
            b'}' => {
                depth -= 1;
                index += '}'.len_utf8();
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = advance_text_char(text, index),
        }
    }

    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EscapedTemplateQuote {
    None,
    Single,
    Double,
}

fn offset_is_backslash_escaped(offset: usize, source: &str) -> bool {
    if offset == 0 {
        return false;
    }

    let bytes = source.as_bytes();
    let mut index = offset;
    let mut backslash_count = 0usize;
    while index > 0 && bytes[index - 1] == b'\\' {
        backslash_count += 1;
        index -= 1;
    }

    backslash_count % 2 == 1
}

fn advance_text_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index + 1, |ch| index + ch.len_utf8())
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

fn strip_parenthesized_conditional(expression: &ConditionalExpr) -> &ConditionalExpr {
    let mut current = expression;
    while let ConditionalExpr::Parenthesized(paren) = current {
        current = &paren.expr;
    }
    current
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

fn prompt_assignment_reference_names(word: &Word, source: &str) -> Vec<(Name, Span)> {
    let Some(text) = static_word_text(word, source) else {
        return Vec::new();
    };
    scan_prompt_parameter_reference_names(text.as_ref(), word.span)
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

fn named_target_word(word: &Word, source: &str) -> Option<(Name, Span)> {
    let text = static_word_text(word, source)?;
    is_name(&text).then_some((Name::from(text.as_ref()), word.span))
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
    has_command_substitution: bool,
    has_command_or_process_substitution: bool,
}

fn parse_simple_declaration_assignment(
    words: &[&Word],
    source: &str,
    explicit_array_kind: Option<ArrayKind>,
) -> Option<SimpleDeclarationAssignment> {
    let assignment = Parser::parse_assignment_word_group(
        source,
        words,
        explicit_array_kind,
        SubscriptInterpretation::Contextual,
    )?;
    let target_span = assignment_target_span(&assignment, source);
    let value_span = assignment_value_span(&assignment);
    let array_like = assignment.target.subscript.is_some()
        || matches!(assignment.value, AssignmentValue::Compound(_));
    let value_origin = assignment_value_origin(&assignment.value);
    let has_command_substitution = assignment_value_has_command_substitution(&assignment.value);
    let has_command_or_process_substitution =
        assignment_value_has_command_or_process_substitution(&assignment.value);

    Some(SimpleDeclarationAssignment {
        name: assignment.target.name,
        name_span: assignment.target.name_span,
        target_span,
        value_span,
        append: assignment.append,
        array_like,
        value_origin,
        has_command_substitution,
        has_command_or_process_substitution,
    })
}

fn contiguous_word_groups<'a>(words: &'a [&'a Word]) -> Vec<&'a [&'a Word]> {
    let mut groups = Vec::new();
    let mut start = 0usize;

    while start < words.len() {
        let mut end = start + 1;
        while let Some(next) = words.get(end).copied() {
            if words[end - 1].span.end.offset != next.span.start.offset {
                break;
            }
            end += 1;
        }
        groups.push(&words[start..end]);
        start = end;
    }

    groups
}

fn word_group_span(words: &[&Word]) -> Span {
    let first = words.first().expect("word groups are non-empty");
    let last = words.last().expect("word groups are non-empty");
    Span::from_positions(first.span.start, last.span.end)
}

fn declaration_explicit_array_kind(flags: &FxHashSet<char>) -> Option<ArrayKind> {
    if flags.contains(&'A') {
        Some(ArrayKind::Associative)
    } else if flags.contains(&'a') {
        Some(ArrayKind::Indexed)
    } else {
        None
    }
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

fn read_option_attached_target_span(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
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
    matches!(
        part,
        WordPart::Variable(_)
            | WordPart::ArrayAccess(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
    )
}

fn static_parts_text(parts: &[WordPartNode], source: &str) -> Option<String> {
    try_static_word_parts_text(parts, source).map(|text| text.into_owned())
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

fn assignment_value_origin_for_assignment(
    assignment: &Assignment,
    _source: &str,
) -> AssignmentValueOrigin {
    if assignment.target.subscript.is_some() {
        AssignmentValueOrigin::ArrayOrCompound
    } else {
        match &assignment.value {
            AssignmentValue::Scalar(word) => assignment_value_origin_for_word(word),
            AssignmentValue::Compound(_) => AssignmentValueOrigin::ArrayOrCompound,
        }
    }
}

fn assignment_value_origin(value: &AssignmentValue) -> AssignmentValueOrigin {
    match value {
        AssignmentValue::Scalar(word) => assignment_value_origin_for_word(word),
        AssignmentValue::Compound(_) => AssignmentValueOrigin::ArrayOrCompound,
    }
}

fn assignment_value_has_command_or_process_substitution(value: &AssignmentValue) -> bool {
    let AssignmentValue::Scalar(word) = value else {
        return false;
    };
    let mut scan = AssignmentWordOriginScan::default();
    scan_assignment_word_parts(&word.parts, &mut scan);
    scan.command_substitution || scan.process_substitution
}

fn assignment_value_has_command_substitution(value: &AssignmentValue) -> bool {
    let AssignmentValue::Scalar(word) = value else {
        return false;
    };
    let mut scan = AssignmentWordOriginScan::default();
    scan_assignment_word_parts(&word.parts, &mut scan);
    scan.command_substitution
}

fn binding_origin_for_assignment(assignment: &Assignment, source: &str) -> BindingOrigin {
    BindingOrigin::Assignment {
        definition_span: assignment_target_span(assignment, source),
        value: assignment_value_origin_for_assignment(assignment, source),
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
    command_substitution: bool,
    process_substitution: bool,
    array_or_compound: bool,
    mixed_dynamic: bool,
}

impl AssignmentWordOriginScan {
    fn category_count(&self) -> usize {
        [
            self.parameter_operator,
            self.transformation,
            self.indirect_expansion,
            self.command_substitution || self.process_substitution,
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
        } else if self.command_substitution || self.process_substitution {
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
        WordPart::CommandSubstitution { .. } => scan.command_substitution = true,
        WordPart::ProcessSubstitution { .. } => scan.process_substitution = true,
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

fn source_line_start_offsets(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (offset, ch) in source.char_indices() {
        if ch == '\n' {
            starts.push(offset + ch.len_utf8());
        }
    }
    starts
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
