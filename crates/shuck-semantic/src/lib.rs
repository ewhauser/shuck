#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! Semantic analysis for shell scripts parsed by Shuck.
//!
//! The semantic model tracks scopes, bindings, references, control flow, and selected dataflow
//! facts so higher-level crates can reason about shell behavior without re-traversing the AST.
mod analysis;
mod binding;
mod builder;
mod call_graph;
mod cfg;
mod contract;
mod dataflow;
mod declaration;
mod dense_bit_set;
mod function_call_reachability;
mod function_resolution;
mod glob;
mod nonpersistent;
mod reachability;
mod reference;
mod runtime;
mod scope;
mod source_closure;
mod source_ref;
mod uninitialized;
mod unused;
mod value_flow;
mod zsh_options;

/// Binding types and provenance metadata discovered during semantic analysis.
pub use binding::{
    AssignmentValueOrigin, Binding, BindingAttributes, BindingId, BindingKind, BindingOrigin,
    BuiltinBindingTargetKind, LoopValueOrigin,
};
/// Call-graph structures derived from the analyzed script.
pub use call_graph::{
    CallGraph, CallSite, OverwrittenFunction, UnreachedFunction, UnreachedFunctionReason,
};
/// Control-flow graph types and flow-context annotations.
pub use cfg::{
    BasicBlock, BlockId, BuiltinCommandKind, CommandConditionRole, CommandId, CommandKind,
    CompoundCommandKind, ControlFlowGraph, EdgeKind, FlowContext, StatementSequenceCommand,
    UnreachableCauseKind,
};
/// Contract and build-option types used when constructing semantic models.
pub use contract::{
    ContractCertainty, FileContract, FileEntryBindingInitialization, FileEntryContractCollector,
    FunctionContract, ProvidedBinding, ProvidedBindingKind, SemanticBuildOptions,
};
/// Dataflow results surfaced by the semantic analysis layer.
pub use dataflow::{
    DeadCode, ReachingDefinitions, UninitializedCertainty, UninitializedReference,
    UnusedAssignment, UnusedReason,
};
/// Declaration records discovered while building the semantic model.
pub use declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
/// Direct function-call reachability query types.
pub use function_call_reachability::{
    DirectFunctionCallReachability, DirectFunctionCallWindow, FunctionCallCandidate,
    FunctionCallPersistence,
};
/// Option-sensitive globbing and expansion behavior queries.
pub use glob::{
    BraceCharacterClassBehavior, FieldSplittingBehavior, FileExpansionOrderBehavior,
    GlobDotBehavior, GlobFailureBehavior, GlobPatternBehavior, PathnameExpansionBehavior,
    PatternOperatorBehavior,
};
/// Nonpersistent assignment effects, such as assignments made in subshells and read later outside.
pub use nonpersistent::{
    NonpersistentAssignmentAnalysis, NonpersistentAssignmentAnalysisContext,
    NonpersistentAssignmentAnalysisOptions, NonpersistentAssignmentCommandContext,
    NonpersistentAssignmentEffect, NonpersistentAssignmentExtraRead, NonpersistentLaterUseKind,
};
/// Reference types and identifiers tracked by the semantic model.
pub use reference::{Reference, ReferenceId, ReferenceKind};
/// Scope types and identifiers tracked by the semantic model.
pub use scope::{FunctionScopeKind, Scope, ScopeId, ScopeKind};
/// Shell parser option types reused by the semantic analysis layer.
pub use shuck_parser::{OptionValue, ShellDialect, ShellProfile, ZshEmulationMode, ZshOptionState};
/// Source-reference records and resolution state.
pub use source_ref::{SourceRef, SourceRefDiagnosticClass, SourceRefKind, SourceRefResolution};
/// Value-flow query object built over semantic bindings, call sites, CFG, and dataflow.
pub use value_flow::SemanticValueFlow;

/// How an unindexed array reference behaves at a source offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayReferencePolicy {
    /// The shell requires an explicit element or selector for array references.
    RequiresExplicitSelector,
    /// Native zsh treats an unindexed array reference as a scalar first-element read.
    NativeZshScalar,
    /// Runtime option state may select either policy.
    Ambiguous,
}

/// How indexed array subscripts are interpreted at a source offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptIndexBehavior {
    /// Subscript `1` names the first array element.
    OneBased,
    /// Subscript `0` names the first array element.
    ZeroBased,
    /// Subscript `1` names the first element, but `0` is accepted as an alias.
    OneBasedWithZeroAlias,
    /// Runtime option state may select more than one indexing policy.
    Ambiguous,
}

/// How arithmetic number literals are interpreted at a source offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticLiteralBehavior {
    /// Decimal literals are used unless an explicit shell arithmetic base is present.
    DecimalUnlessExplicitBase,
    /// Leading zeroes denote octal literals.
    LeadingZeroOctal,
    /// Both C-style base prefixes and leading-zero octal literals are active.
    CStyleAndLeadingZeroOctal,
    /// Runtime option state may select more than one literal policy.
    Ambiguous,
}

/// Option-sensitive shell behavior visible at a source offset.
#[derive(Debug)]
pub struct ShellBehaviorAt<'model> {
    shell: ShellDialect,
    zsh_options: Option<&'model ZshOptionState>,
    runtime_options: Option<ZshOptionState>,
}

impl ShellBehaviorAt<'_> {
    fn effective_zsh_options(&self) -> Option<&ZshOptionState> {
        self.runtime_options.as_ref().or(self.zsh_options)
    }

    /// Returns the array-reference policy implied by the shell and runtime option state.
    pub fn array_reference_policy(&self) -> ArrayReferencePolicy {
        if self.shell != ShellDialect::Zsh {
            return ArrayReferencePolicy::RequiresExplicitSelector;
        }

        match self
            .effective_zsh_options()
            .map(|options| options.ksh_arrays)
        {
            Some(OptionValue::Off) => ArrayReferencePolicy::NativeZshScalar,
            Some(OptionValue::Unknown) => ArrayReferencePolicy::Ambiguous,
            Some(OptionValue::On) | None => ArrayReferencePolicy::RequiresExplicitSelector,
        }
    }

    /// Returns how array subscript indexes are interpreted.
    pub fn subscript_indexing(&self) -> SubscriptIndexBehavior {
        if self.shell != ShellDialect::Zsh {
            return SubscriptIndexBehavior::ZeroBased;
        }

        let Some(options) = self.effective_zsh_options() else {
            return SubscriptIndexBehavior::OneBased;
        };

        match (options.ksh_arrays, options.ksh_zero_subscript) {
            (OptionValue::On, _) => SubscriptIndexBehavior::ZeroBased,
            (OptionValue::Unknown, _) | (OptionValue::Off, OptionValue::Unknown) => {
                SubscriptIndexBehavior::Ambiguous
            }
            (OptionValue::Off, OptionValue::On) => SubscriptIndexBehavior::OneBasedWithZeroAlias,
            (OptionValue::Off, OptionValue::Off) => SubscriptIndexBehavior::OneBased,
        }
    }

    /// Returns how arithmetic numeric literals are interpreted.
    pub fn arithmetic_literals(&self) -> ArithmeticLiteralBehavior {
        if self.shell != ShellDialect::Zsh {
            return ArithmeticLiteralBehavior::CStyleAndLeadingZeroOctal;
        }

        let Some(options) = self.effective_zsh_options() else {
            return ArithmeticLiteralBehavior::DecimalUnlessExplicitBase;
        };

        match options.octal_zeroes {
            OptionValue::Off => ArithmeticLiteralBehavior::DecimalUnlessExplicitBase,
            OptionValue::On => ArithmeticLiteralBehavior::LeadingZeroOctal,
            OptionValue::Unknown => ArithmeticLiteralBehavior::Ambiguous,
        }
    }
}

/// A function scope reached through a top-level `case "$1"` style CLI dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseCliDispatch {
    function_scope: ScopeId,
    dispatcher_span: Span,
}

impl CaseCliDispatch {
    fn new(function_scope: ScopeId, dispatcher_span: Span) -> Self {
        Self {
            function_scope,
            dispatcher_span,
        }
    }

    /// The function body scope selected by the dispatcher.
    pub fn function_scope(self) -> ScopeId {
        self.function_scope
    }

    /// The span of the dynamic positional command used for dispatch.
    pub fn dispatcher_span(self) -> Span {
        self.dispatcher_span
    }
}

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, ConditionalExpr, File, Name, Span, Stmt};
use shuck_indexer::Indexer;
use smallvec::{Array, SmallVec};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::builder::SemanticModelBuilder;
use crate::call_graph::build_call_graph;
use crate::cfg::{
    RecordedCommandKind, RecordedListOperator, RecordedPipelineOperatorKind, RecordedProgram,
};
#[cfg(test)]
use crate::dataflow::DataflowResult;
use crate::dataflow::{DataflowContext, ExactVariableDataflow};
use crate::function_resolution::FunctionBindingLookup;
use crate::runtime::RuntimePrelude;
use crate::scope::{ancestor_scopes, enclosing_function_scope};
use crate::source_closure::ImportedBindingContractSite;
use crate::zsh_options::ZshOptionAnalysis;

const MAX_FUNCTIONS_FOR_TERMINATION_REACHABILITY: usize = 200;
const MAX_TERMINATION_REACHABILITY_WORK: usize = 20_000;

struct AssocCallerSeenNames {
    inline: SmallVec<[Name; 8]>,
    hashed: Option<FxHashSet<Name>>,
}

impl AssocCallerSeenNames {
    const HASH_THRESHOLD: usize = 32;

    fn new() -> Self {
        Self {
            inline: SmallVec::new(),
            hashed: None,
        }
    }

    fn insert(&mut self, name: &Name) -> bool {
        if let Some(hashed) = &mut self.hashed {
            return hashed.insert(name.clone());
        }

        if self.inline.iter().any(|seen_name| seen_name == name) {
            return false;
        }

        if self.inline.len() < Self::HASH_THRESHOLD {
            self.inline.push(name.clone());
            return true;
        }

        let mut hashed =
            FxHashSet::with_capacity_and_hasher(Self::HASH_THRESHOLD * 2, Default::default());
        hashed.extend(self.inline.drain(..));
        let inserted = hashed.insert(name.clone());
        self.hashed = Some(hashed);
        inserted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SpanKey {
    start: usize,
    end: usize,
}

impl SpanKey {
    pub(crate) fn new(span: Span) -> Self {
        Self {
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

#[derive(Debug)]
struct CommandTopology {
    syntax_backed_ids: Vec<CommandId>,
    structural_ids: Vec<CommandId>,
    contexts: Vec<Option<SemanticCommandContext>>,
    ids_by_syntax_span: FxHashMap<SpanKey, SmallVec<[CommandId; 1]>>,
    parent_ids: Vec<Option<CommandId>>,
    child_ids: Vec<Vec<CommandId>>,
    syntax_backed_parent_ids: Vec<Option<CommandId>>,
    syntax_backed_child_ids: Vec<Vec<CommandId>>,
    offset_order: Vec<CommandId>,
    containing_offset_entries: Vec<CommandContainingOffsetEntry>,
}

/// Syntax-backed command metadata derived during semantic traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCommandContext {
    id: CommandId,
    span: Span,
    syntax_span: Span,
    kind: CommandKind,
    scope: ScopeId,
    flow: FlowContext,
    structural: bool,
    nested_word_command: bool,
    nested_word_command_depth: usize,
    in_if_condition: bool,
    in_elif_condition: bool,
    condition_role: Option<CommandConditionRole>,
}

impl SemanticCommandContext {
    /// Semantic command identifier.
    pub fn id(&self) -> CommandId {
        self.id
    }

    /// Statement span including redirects.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Span of the syntactic command node.
    pub fn syntax_span(&self) -> Span {
        self.syntax_span
    }

    /// Syntax command kind.
    pub fn kind(&self) -> CommandKind {
        self.kind
    }

    /// Scope active at the command.
    pub fn scope(&self) -> ScopeId {
        self.scope
    }

    /// Flow context active at the command.
    pub fn flow(&self) -> FlowContext {
        self.flow
    }

    /// Whether this command is part of the structural command stream.
    pub fn is_structural(&self) -> bool {
        self.structural
    }

    /// Whether this command came from a command-like expansion in a word.
    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    /// Number of command-like word-expansion regions enclosing this command.
    pub fn nested_word_command_depth(&self) -> usize {
        self.nested_word_command_depth
    }

    /// Whether this command is inside an `if` or `elif` condition list.
    pub fn is_in_if_condition(&self) -> bool {
        self.in_if_condition
    }

    /// Whether this command is inside an `elif` condition list.
    pub fn is_in_elif_condition(&self) -> bool {
        self.in_elif_condition
    }

    /// Condition-list role inherited from surrounding shell syntax, if any.
    pub fn condition_role(&self) -> Option<CommandConditionRole> {
        self.condition_role
    }
}

#[derive(Debug, Clone, Copy)]
struct CommandContainingOffsetEntry {
    start_offset: usize,
    end_offset: usize,
    id: CommandId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CommandContainingOffsetEventKind {
    End,
    Start,
}

#[derive(Debug, Clone, Copy)]
struct CommandContainingOffsetEvent {
    offset: usize,
    end_offset: usize,
    id: CommandId,
    kind: CommandContainingOffsetEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceDirectiveOverride {
    pub(crate) kind: SourceRefKind,
    pub(crate) own_line: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndirectTargetHint {
    Exact {
        name: Name,
        array_like: bool,
    },
    Pattern {
        prefix: compact_str::CompactString,
        suffix: compact_str::CompactString,
        array_like: bool,
    },
}

/// Synthetic read introduced by semantic modeling for later analysis passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticRead {
    pub(crate) scope: ScopeId,
    pub(crate) span: Span,
    pub(crate) name: Name,
}

/// Summary of positional-parameter reads reachable from a function scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FunctionPositionalReferenceSummary {
    required_arg_count: usize,
    uses_unprotected_positional_parameters: bool,
}

impl FunctionPositionalReferenceSummary {
    /// Returns the highest positional argument index that is definitely required.
    pub fn required_arg_count(self) -> usize {
        self.required_arg_count
    }

    /// Returns whether the function reads positional parameters without a guarding default.
    pub fn uses_unprotected_positional_parameters(self) -> bool {
        self.uses_unprotected_positional_parameters
    }

    fn record_required_arg_count(&mut self, index: usize) {
        self.required_arg_count = self.required_arg_count.max(index);
        self.uses_unprotected_positional_parameters = true;
    }

    fn record_use(&mut self) {
        self.uses_unprotected_positional_parameters = true;
    }
}

/// Behavior flags for unused-assignment analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnusedAssignmentAnalysisOptions {
    /// Whether a resolved scalar indirect-expansion target like `${!name}` counts as a use
    /// of the target. ShellCheck-compatible analysis leaves this disabled. Array-like
    /// targets such as `name=arr[@]; ${!name}` stay live in both modes.
    pub treat_indirect_expansion_targets_as_used: bool,
    /// Whether assignments in statically unreachable blocks should still be eligible
    /// for unused-assignment reporting.
    pub report_unreachable_assignments: bool,
}

/// Behavior flags for unreached-function analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnreachedFunctionAnalysisOptions {
    /// Whether nested function definitions should be reported when their enclosing function scope
    /// is not reached by a direct call chain.
    pub report_unreached_nested_definitions: bool,
}

impl SyntheticRead {
    /// Returns the scope where the synthetic read should be considered visible.
    pub fn scope(&self) -> ScopeId {
        self.scope
    }

    /// Returns the span that higher layers should attribute to the synthetic read.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Returns the runtime name read by this synthetic entry.
    pub fn name(&self) -> &Name {
        &self.name
    }
}

#[doc(hidden)]
pub trait TraversalObserver<'a> {
    fn enter_command(&mut self, _command: &Command, _scope: ScopeId, _flow: FlowContext) {}

    fn exit_command(&mut self, _command: &Command, _scope: ScopeId) {}

    fn conditional_expression(
        &mut self,
        _command_span: Span,
        _expression: &'a ConditionalExpr,
        _parent_in_same_logical_group: bool,
    ) {
    }

    fn recorded_command(
        &mut self,
        _id: CommandId,
        _stmt: &'a Stmt,
        _scope: ScopeId,
        _flow: FlowContext,
    ) {
    }

    fn recorded_statement_sequence_command(
        &mut self,
        _body_span: Span,
        _stmt_span: Span,
        _id: CommandId,
    ) {
    }

    fn record_binding(&mut self, _binding: &Binding) {}

    fn record_reference(&mut self, _reference: &Reference, _resolved: Option<&Binding>) {}
}

#[doc(hidden)]
pub struct NoopTraversalObserver;

impl<'a> TraversalObserver<'a> for NoopTraversalObserver {}

#[doc(hidden)]
pub trait SourcePathResolver {
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf>;
}

impl<F> SourcePathResolver for F
where
    F: Fn(&Path, &str) -> Vec<PathBuf> + Send + Sync,
{
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        self(source_path, candidate)
    }
}

fn dedup_synthetic_reads(reads: Vec<SyntheticRead>) -> Vec<SyntheticRead> {
    let mut seen = FxHashSet::default();
    let mut deduped = Vec::new();
    for read in reads {
        if seen.insert((read.scope, read.span.start.offset, read.name.clone())) {
            deduped.push(read);
        }
    }
    deduped
}

fn assignment_like_binding(kind: BindingKind) -> bool {
    matches!(
        kind,
        BindingKind::Assignment
            | BindingKind::AppendAssignment
            | BindingKind::ArrayAssignment
            | BindingKind::ArithmeticAssignment
    )
}

fn binding_blocks_same_scope_assoc_lookup(binding: &Binding) -> bool {
    binding.attributes.contains(BindingAttributes::LOCAL) || !assignment_like_binding(binding.kind)
}

fn previous_visible_binding_id_from_slice(
    all_bindings: &[Binding],
    bindings: &[BindingId],
    offset: usize,
    ignored_binding_span: Option<Span>,
) -> Option<BindingId> {
    let candidate_count = bindings
        .partition_point(|binding_id| all_bindings[binding_id.index()].span.start.offset <= offset);

    bindings[..candidate_count]
        .iter()
        .rev()
        .copied()
        .find(|binding_id| ignored_binding_span != Some(all_bindings[binding_id.index()].span))
}

trait BindingIdCollection {
    fn as_slice(&self) -> &[BindingId];
    fn insert_binding_id(&mut self, index: usize, id: BindingId);
}

impl BindingIdCollection for Vec<BindingId> {
    fn as_slice(&self) -> &[BindingId] {
        self
    }

    fn insert_binding_id(&mut self, index: usize, id: BindingId) {
        self.insert(index, id);
    }
}

impl<A> BindingIdCollection for SmallVec<A>
where
    A: Array<Item = BindingId>,
{
    fn as_slice(&self) -> &[BindingId] {
        self
    }

    fn insert_binding_id(&mut self, index: usize, id: BindingId) {
        self.insert(index, id);
    }
}

fn insert_binding_id_sorted(
    bindings: &mut impl BindingIdCollection,
    all_bindings: &[Binding],
    id: BindingId,
) {
    let target = &all_bindings[id.index()];
    let insertion = bindings.as_slice().partition_point(|candidate_id| {
        let candidate = &all_bindings[candidate_id.index()];
        candidate.span.start.offset < target.span.start.offset
            || (candidate.span.start.offset == target.span.start.offset
                && candidate.span.end.offset < target.span.end.offset)
            || (candidate.span.start.offset == target.span.start.offset
                && candidate.span.end.offset == target.span.end.offset
                && candidate.id.index() < target.id.index())
    });
    bindings.insert_binding_id(insertion, id);
}

#[derive(Debug)]
struct AssocLookupBindingIndex {
    blocking_bindings_by_scope: Vec<FxHashMap<Name, Box<[BindingId]>>>,
}

#[derive(Debug)]
struct ScopeProvidedBindingIndex {
    provided_bindings_by_scope: Vec<Box<[ProvidedBinding]>>,
    definite_provider_scopes_by_name: FxHashMap<Name, Box<[ScopeId]>>,
}

#[derive(Debug)]
struct ScopeLookup {
    children: Vec<Box<[ScopeId]>>,
}

impl ScopeLookup {
    fn new(scopes: &[Scope]) -> Self {
        let mut children = vec![Vec::new(); scopes.len()];

        for scope in scopes {
            if let Some(parent) = scope.parent {
                children[parent.index()].push(scope.id);
            }
        }

        for scope_ids in &mut children {
            scope_ids.sort_by_key(|scope_id| {
                let span = scopes[scope_id.index()].span;
                (span.start.offset, span.end.offset)
            });
        }

        Self {
            children: children.into_iter().map(Vec::into_boxed_slice).collect(),
        }
    }

    fn scope_at(&self, scopes: &[Scope], offset: usize) -> Option<ScopeId> {
        let root = scopes.first()?;
        if !contains_offset(root.span, offset) {
            return None;
        }

        let mut scope = root.id;
        while let Some(child) = self.child_scope_at(scopes, scope, offset) {
            scope = child;
        }

        Some(scope)
    }

    fn child_scope_at(&self, scopes: &[Scope], parent: ScopeId, offset: usize) -> Option<ScopeId> {
        let children = self.children.get(parent.index())?;
        let cutoff = children
            .partition_point(|scope_id| scopes[scope_id.index()].span.start.offset <= offset);
        let mut best: Option<ScopeId> = None;
        let mut index = cutoff;

        while index > 0 {
            index -= 1;
            let scope_id = children[index];
            let span = scopes[scope_id.index()].span;
            if span.end.offset < offset {
                break;
            }
            if contains_offset(span, offset) {
                match best {
                    Some(current)
                        if scope_span_width(scopes[current.index()].span)
                            <= scope_span_width(span) => {}
                    _ => best = Some(scope_id),
                }
            }
        }

        best
    }
}

/// Semantic model constructed from a parsed shell file and source text.
#[derive(Debug)]
pub struct SemanticModel {
    shell_profile: ShellProfile,
    scopes: Vec<Scope>,
    scope_lookup: ScopeLookup,
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
    call_graph: CallGraph,
    source_refs: Vec<SourceRef>,
    runtime: RuntimePrelude,
    declarations: Vec<Declaration>,
    indirect_target_hints: FxHashMap<BindingId, IndirectTargetHint>,
    indirect_targets_by_binding: FxHashMap<BindingId, Vec<BindingId>>,
    indirect_targets_by_reference: FxHashMap<ReferenceId, Vec<BindingId>>,
    array_like_indirect_expansion_refs: FxHashSet<ReferenceId>,
    synthetic_reads: Vec<SyntheticRead>,
    entry_bindings: Vec<BindingId>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_program: RecordedProgram,
    command_bindings: FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    cleared_variables: FxHashMap<(ScopeId, Name), SmallVec<[usize; 2]>>,
    import_origins_by_binding: FxHashMap<BindingId, Vec<PathBuf>>,
    heuristic_unused_assignments: Vec<BindingId>,
    zsh_option_analysis: Option<ZshOptionAnalysis>,
    zsh_runtime_ambiguous_entry_mask: OnceLock<zsh_options::ZshOptionMask>,
    zsh_runtime_by_function: OnceLock<FxHashMap<ScopeId, OnceLock<Option<ZshOptionAnalysis>>>>,
    assoc_lookup_binding_index: OnceLock<AssocLookupBindingIndex>,
    command_topology: OnceLock<CommandTopology>,
    references_sorted_by_start: OnceLock<Vec<ReferenceId>>,
    bindings_sorted_by_start: OnceLock<Vec<BindingId>>,
    bindings_by_definition_span: OnceLock<FxHashMap<SpanKey, BindingId>>,
    guarded_or_defaulting_reference_offsets_by_name: OnceLock<FxHashMap<Name, Box<[usize]>>>,
    declarations_by_command_span: OnceLock<FxHashMap<SpanKey, usize>>,
    unconditional_function_bindings: OnceLock<FxHashSet<BindingId>>,
    function_bindings_by_scope: OnceLock<FxHashMap<ScopeId, SmallVec<[BindingId; 2]>>>,
    visible_function_call_bindings: OnceLock<FxHashMap<SpanKey, BindingId>>,
    function_definition_binding_ids: OnceLock<Vec<BindingId>>,
}

/// Lazy analysis view over a `SemanticModel`.
#[derive(Debug)]
pub struct SemanticAnalysis<'model> {
    model: &'model SemanticModel,
    cfg: OnceLock<ControlFlowGraph>,
    exact_variable_dataflow: OnceLock<ExactVariableDataflow>,
    #[cfg(test)]
    dataflow: OnceLock<DataflowResult>,
    unused_assignments: OnceLock<Vec<BindingId>>,
    unused_assignments_shellcheck_compat: OnceLock<Vec<BindingId>>,
    uninitialized_references: OnceLock<Vec<UninitializedReference>>,
    uninitialized_reference_certainties: OnceLock<FxHashMap<SpanKey, UninitializedCertainty>>,
    dead_code: OnceLock<Vec<DeadCode>>,
    unreachable_blocks: OnceLock<FxHashSet<BlockId>>,
    binding_block_index: OnceLock<Vec<Vec<BlockId>>>,
    overwritten_functions: OnceLock<Vec<OverwrittenFunction>>,
    unreached_functions: OnceLock<Vec<UnreachedFunction>>,
    unreached_functions_shellcheck_compat: OnceLock<Vec<UnreachedFunction>>,
    scope_provided_binding_index: OnceLock<ScopeProvidedBindingIndex>,
}

/// A flattened logical list command recorded by semantic analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticListCommand {
    /// Span of the complete logical list command.
    pub span: Span,
    /// Commands in execution order, including the first command and each following list item.
    pub segments: Box<[SemanticListSegment]>,
}

/// A flattened logical list segment recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticListSegment {
    /// Span of the segment command.
    pub command_span: Span,
    /// Operator that precedes this segment, or `None` for the first segment.
    pub operator_before: Option<SemanticListOperator>,
}

/// A logical list operator recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticListOperator {
    /// Logical operator kind.
    pub kind: SemanticListOperatorKind,
    /// Span of the operator token.
    pub span: Span,
}

/// Logical list operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticListOperatorKind {
    /// `&&`
    And,
    /// `||`
    Or,
}

#[derive(Debug, Clone, Copy)]
struct RecordedListOperatorWithSpan {
    operator: RecordedListOperator,
    span: Span,
}

/// A flattened pipeline command recorded by semantic analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticPipelineCommand {
    /// Span of the complete pipeline command.
    pub span: Span,
    /// Commands in execution order, including the first command and each following pipeline segment.
    pub segments: Box<[SemanticPipelineSegment]>,
}

/// A flattened pipeline segment recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticPipelineSegment {
    /// Span of the segment command.
    pub command_span: Span,
    /// Operator that precedes this segment, or `None` for the first segment.
    pub operator_before: Option<SemanticPipelineOperator>,
}

/// A pipeline operator recorded by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticPipelineOperator {
    /// Pipeline operator kind.
    pub kind: SemanticPipelineOperatorKind,
    /// Span of the operator token.
    pub span: Span,
}

/// Pipeline operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticPipelineOperatorKind {
    /// `|`
    Pipe,
    /// `|&`
    PipeAll,
}

impl SemanticModel {
    /// Builds a semantic model for `file` using the default build options.
    pub fn build(file: &File, source: &str, indexer: &Indexer) -> Self {
        Self::build_with_options(file, source, indexer, SemanticBuildOptions::default())
    }

    /// Builds a semantic model for `file` using explicit semantic build options.
    pub fn build_with_options(
        file: &File,
        source: &str,
        indexer: &Indexer,
        options: SemanticBuildOptions<'_>,
    ) -> Self {
        let mut observer = NoopTraversalObserver;
        build_with_observer_with_options(file, source, indexer, &mut observer, options)
    }

    fn from_build_output(built: builder::BuildOutput) -> Self {
        let mut reference_index = built.reference_index;
        for reference_ids in reference_index.values_mut() {
            reference_ids.sort_by_key(|reference_id| {
                built.references[reference_id.index()].span.start.offset
            });
        }
        let indirect_targets_by_binding =
            build_indirect_targets_by_binding(&built.bindings, &built.indirect_target_hints);
        let indirect_targets_by_reference = build_indirect_targets_by_reference(
            &built.references,
            &built.resolved,
            &built.indirect_expansion_refs,
            &indirect_targets_by_binding,
        );
        let array_like_indirect_expansion_refs = build_array_like_indirect_expansion_refs(
            &built.references,
            &built.resolved,
            &built.indirect_expansion_refs,
            &built.indirect_target_hints,
        );
        let zsh_dynamic_calls = zsh_options::DynamicCallAnalysisContext {
            references: &built.references,
            resolved: &built.resolved,
            indirect_target_hints: &built.indirect_target_hints,
            indirect_targets_by_binding: &indirect_targets_by_binding,
            command_references: &built.command_references,
        };
        let zsh_option_analysis = zsh_options::analyze(
            &built.shell_profile,
            &built.scopes,
            &built.bindings,
            &built.recorded_program,
            zsh_dynamic_calls,
        );
        let scope_lookup = ScopeLookup::new(&built.scopes);
        Self {
            shell_profile: built.shell_profile,
            scopes: built.scopes,
            scope_lookup,
            bindings: built.bindings,
            references: built.references,
            reference_index,
            predefined_runtime_refs: built.predefined_runtime_refs,
            guarded_parameter_refs: built.guarded_parameter_refs,
            parameter_guard_flow_refs: built.parameter_guard_flow_refs,
            defaulting_parameter_operand_refs: built.defaulting_parameter_operand_refs,
            self_referential_assignment_refs: built.self_referential_assignment_refs,
            binding_index: built.binding_index,
            resolved: built.resolved,
            unresolved: built.unresolved,
            functions: built.functions,
            call_sites: built.call_sites,
            call_graph: built.call_graph,
            source_refs: built.source_refs,
            runtime: built.runtime,
            declarations: built.declarations,
            indirect_target_hints: built.indirect_target_hints,
            indirect_targets_by_binding,
            indirect_targets_by_reference,
            array_like_indirect_expansion_refs,
            synthetic_reads: Vec::new(),
            entry_bindings: Vec::new(),
            flow_contexts: built.flow_contexts,
            recorded_program: built.recorded_program,
            command_bindings: built.command_bindings,
            command_references: built.command_references,
            cleared_variables: built.cleared_variables,
            import_origins_by_binding: FxHashMap::default(),
            heuristic_unused_assignments: built.heuristic_unused_assignments,
            zsh_option_analysis,
            zsh_runtime_ambiguous_entry_mask: OnceLock::new(),
            zsh_runtime_by_function: OnceLock::new(),
            assoc_lookup_binding_index: OnceLock::new(),
            command_topology: OnceLock::new(),
            references_sorted_by_start: OnceLock::new(),
            bindings_sorted_by_start: OnceLock::new(),
            bindings_by_definition_span: OnceLock::new(),
            guarded_or_defaulting_reference_offsets_by_name: OnceLock::new(),
            declarations_by_command_span: OnceLock::new(),
            unconditional_function_bindings: OnceLock::new(),
            function_bindings_by_scope: OnceLock::new(),
            visible_function_call_bindings: OnceLock::new(),
            function_definition_binding_ids: OnceLock::new(),
        }
    }

    /// Returns a lazy analysis view that computes CFG, dataflow, and reachability on demand.
    pub fn analysis(&self) -> SemanticAnalysis<'_> {
        SemanticAnalysis::new(self)
    }

    /// Returns the shell profile used when building this model.
    pub fn shell_profile(&self) -> &ShellProfile {
        &self.shell_profile
    }

    /// Returns the zsh option state visible at `offset`, when the model tracks zsh options.
    pub fn zsh_options_at(&self, offset: usize) -> Option<&ZshOptionState> {
        self.zsh_option_analysis
            .as_ref()
            .and_then(|analysis| analysis.options_at(&self.scopes, offset))
    }

    /// Returns option-sensitive shell behavior visible at `offset`.
    pub fn shell_behavior_at(&self, offset: usize) -> ShellBehaviorAt<'_> {
        ShellBehaviorAt {
            shell: self.shell_profile.dialect,
            zsh_options: self.zsh_options_at(offset),
            runtime_options: self.zsh_runtime_options_at(offset),
        }
    }

    fn zsh_runtime_ambiguous_entry_mask(&self) -> zsh_options::ZshOptionMask {
        if self.shell_profile.zsh_options().is_none() {
            return zsh_options::ZshOptionMask::default();
        }

        *self.zsh_runtime_ambiguous_entry_mask.get_or_init(|| {
            crate::zsh_options::runtime_ambiguous_entry_mask(&self.recorded_program)
        })
    }

    fn zsh_runtime_options_at(&self, offset: usize) -> Option<ZshOptionState> {
        self.shell_profile.zsh_options()?;
        let ordinary = self.zsh_options_at(offset)?.clone();
        let ambiguous_entry = self.zsh_runtime_ambiguous_entry_mask();
        if ambiguous_entry.is_empty() {
            return None;
        }

        let scope = self.scope_at(offset);
        let function_scope = self.enclosing_function_scope(scope)?;
        let ambient = self
            .zsh_runtime_analysis_for_function(function_scope)
            .and_then(|analysis| analysis.options_at(&self.scopes, offset));

        Some(ambient.map_or(ordinary.clone(), |options| ordinary.merge(options)))
    }

    fn zsh_runtime_by_function(&self) -> &FxHashMap<ScopeId, OnceLock<Option<ZshOptionAnalysis>>> {
        self.zsh_runtime_by_function.get_or_init(|| {
            self.recorded_program
                .function_bodies()
                .keys()
                .map(|&function_scope| (function_scope, OnceLock::new()))
                .collect()
        })
    }

    fn zsh_runtime_analysis_for_function(
        &self,
        function_scope: ScopeId,
    ) -> Option<&ZshOptionAnalysis> {
        self.zsh_runtime_by_function()
            .get(&function_scope)?
            .get_or_init(|| self.build_zsh_runtime_analysis_for_function(function_scope))
            .as_ref()
    }

    fn build_zsh_runtime_analysis_for_function(
        &self,
        function_scope: ScopeId,
    ) -> Option<ZshOptionAnalysis> {
        let function_entry_offset = self.scope(function_scope).span.start.offset;
        let mut function_entry = self.zsh_options_at(function_entry_offset)?.clone();
        for field in self.zsh_runtime_ambiguous_entry_mask().iter() {
            crate::zsh_options::set_public_option_field(
                &mut function_entry,
                field,
                OptionValue::Unknown,
            );
        }
        crate::zsh_options::function_runtime_analysis_with_entry(
            &self.scopes,
            &self.bindings,
            &self.recorded_program,
            crate::zsh_options::DynamicCallAnalysisContext {
                references: &self.references,
                resolved: &self.resolved,
                indirect_target_hints: &self.indirect_target_hints,
                indirect_targets_by_binding: &self.indirect_targets_by_binding,
                command_references: &self.command_references,
            },
            function_scope,
            function_entry,
        )
    }

    /// Returns all semantic scopes discovered in the file.
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Returns the scope identified by `id`.
    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.index()]
    }

    /// Returns all semantic bindings discovered in the file.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Yield every binding with `BindingKind::FunctionDefinition`.
    ///
    /// Backed by a lazily-built index so repeat calls avoid rescanning the
    /// full `bindings()` slice.
    pub fn function_definition_bindings(&self) -> impl ExactSizeIterator<Item = &Binding> + '_ {
        let ids = self.function_definition_binding_ids.get_or_init(|| {
            self.bindings
                .iter()
                .filter(|binding| matches!(binding.kind, BindingKind::FunctionDefinition))
                .map(|binding| binding.id)
                .collect()
        });
        ids.iter().map(|id| &self.bindings[id.index()])
    }

    /// Returns all semantic references discovered in the file.
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// Returns guarded or defaulting reference offsets grouped by variable name.
    ///
    /// Backed by a lazily-built summary so repeated undefined-variable suppression
    /// checks can reuse the same grouped offsets instead of rescanning `references()`.
    pub fn guarded_or_defaulting_reference_offsets_by_name(
        &self,
    ) -> &FxHashMap<Name, Box<[usize]>> {
        self.guarded_or_defaulting_reference_offsets_by_name
            .get_or_init(|| {
                build_guarded_or_defaulting_reference_offsets_by_name(
                    &self.references,
                    &self.guarded_parameter_refs,
                    &self.defaulting_parameter_operand_refs,
                )
            })
    }

    /// Summarize unguarded positional-parameter reads by enclosing function scope.
    ///
    /// Callers can pass transient-scope `set --` offsets to exclude reads that are
    /// masked by a nested local positional reset.
    pub fn function_positional_reference_summary(
        &self,
        local_reset_offsets_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
    ) -> FxHashMap<ScopeId, FunctionPositionalReferenceSummary> {
        let mut summaries = FxHashMap::<ScopeId, FunctionPositionalReferenceSummary>::default();

        for (name, reference_ids) in &self.reference_index {
            let Some(kind) = positional_parameter_reference_kind(name.as_str()) else {
                continue;
            };

            for reference_id in reference_ids {
                let reference = &self.references[reference_id.index()];
                if self.is_guarded_parameter_reference(reference.id)
                    || reference_has_local_positional_reset(
                        self,
                        reference.scope,
                        reference.span.start.offset,
                        local_reset_offsets_by_scope,
                    )
                {
                    continue;
                }

                let Some(function_scope) = self.enclosing_function_scope(reference.scope) else {
                    continue;
                };

                let entry = summaries.entry(function_scope).or_default();
                match kind {
                    PositionalParameterReferenceKind::Indexed(index) => {
                        entry.record_required_arg_count(index);
                    }
                    PositionalParameterReferenceKind::Special => entry.record_use(),
                }
            }
        }

        summaries
    }

    /// Yield every reference whose span is fully contained within `outer`.
    ///
    /// Backed by a lazily-built index sorted by reference start offset, so a
    /// per-span query costs `O(log n + matches)` rather than scanning every
    /// reference in the file.
    pub fn references_in_span(&self, outer: Span) -> ReferencesInSpan<'_> {
        let sorted = self
            .references_sorted_by_start
            .get_or_init(|| build_references_sorted_by_start(&self.references));
        let lower = sorted.partition_point(|id| {
            self.references[id.index()].span.start.offset < outer.start.offset
        });
        ReferencesInSpan {
            references: &self.references,
            ids: sorted[lower..].iter(),
            end: outer.end.offset,
        }
    }

    /// Yield direct references for `command_span` whose spans are fully contained within `outer`.
    ///
    /// References recorded for nested commands are excluded because semantic
    /// stores them against their own command spans instead of the enclosing
    /// command.
    pub fn references_in_command_span(
        &self,
        command_span: Span,
        outer: Span,
    ) -> CommandReferencesInSpan<'_> {
        let command_span = self
            .command_references
            .contains_key(&SpanKey::new(command_span))
            .then_some(command_span)
            .or_else(|| {
                self.command_by_span(command_span)
                    .map(|id| self.command_span(id))
            });
        let ids = command_span
            .filter(|span| contains_span(*span, outer))
            .and_then(|span| self.command_references.get(&SpanKey::new(span)))
            .map(SmallVec::as_slice)
            .unwrap_or(&[]);
        CommandReferencesInSpan {
            references: &self.references,
            ids: ids.iter(),
            outer,
        }
    }

    /// Yield every binding whose span is fully contained within `outer`.
    ///
    /// Backed by a lazily-built index sorted by binding start offset, so a
    /// per-span query costs `O(log n + matches)` rather than scanning every
    /// binding in the file.
    pub fn bindings_in_span(&self, outer: Span) -> BindingsInSpan<'_> {
        let sorted = self
            .bindings_sorted_by_start
            .get_or_init(|| build_bindings_sorted_by_start(&self.bindings));
        let lower = sorted
            .partition_point(|id| self.bindings[id.index()].span.start.offset < outer.start.offset);
        BindingsInSpan {
            bindings: &self.bindings,
            ids: sorted[lower..].iter(),
            end: outer.end.offset,
        }
    }

    /// Returns the binding identified by `id`.
    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.index()]
    }

    /// Returns the binding introduced at exactly `span`, when such a definition is indexed.
    pub fn binding_for_definition_span(&self, span: Span) -> Option<BindingId> {
        let index = self
            .bindings_by_definition_span
            .get_or_init(|| build_bindings_by_definition_span(&self.bindings));
        index.get(&SpanKey::new(span)).copied()
    }

    /// Returns the reference identified by `id`.
    pub fn reference(&self, id: ReferenceId) -> &Reference {
        &self.references[id.index()]
    }

    /// Returns the binding resolved for `id`, if reference resolution succeeded.
    pub fn resolved_binding(&self, id: ReferenceId) -> Option<&Binding> {
        self.resolved
            .get(&id)
            .map(|binding| &self.bindings[binding.index()])
    }

    /// Returns whether `id` names a predefined runtime array variable.
    pub fn reference_is_predefined_runtime_array(&self, id: ReferenceId) -> bool {
        self.predefined_runtime_refs.contains(&id)
            && self
                .references
                .get(id.index())
                .is_some_and(|reference| self.runtime.is_preinitialized_array(&reference.name))
    }

    /// Returns whether `name` is provided by the shell runtime for this model's dialect.
    pub fn name_is_predefined_runtime(&self, name: &str) -> bool {
        self.runtime.is_preinitialized(&Name::from(name))
    }

    /// Returns whether `name` is a well-known runtime-style name that typo suppression should
    /// ignore for this model's shell dialect.
    pub fn name_is_known_runtime(&self, name: &str) -> bool {
        self.runtime.is_known_runtime_name(&Name::from(name))
    }

    /// Returns whether `id` is guarded by parameter-expansion syntax that suppresses missing-name
    /// diagnostics.
    pub fn is_guarded_parameter_reference(&self, id: ReferenceId) -> bool {
        self.guarded_parameter_refs.contains(&id)
    }

    /// Returns whether `id` appears inside a defaulting parameter operand.
    pub fn is_defaulting_parameter_operand_reference(&self, id: ReferenceId) -> bool {
        self.defaulting_parameter_operand_refs.contains(&id)
    }

    /// Returns bindings that may be targeted by indirect reads through `id`.
    pub fn indirect_targets_for_binding(&self, id: BindingId) -> &[BindingId] {
        self.indirect_targets_by_binding
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns bindings that may be targeted by the indirect reference `id`.
    pub fn indirect_targets_for_reference(&self, id: ReferenceId) -> &[BindingId] {
        self.indirect_targets_by_reference
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns bindings recorded for `name`, ordered by definition offset.
    pub fn bindings_for(&self, name: &Name) -> &[BindingId] {
        self.binding_index
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the latest binding for `name` that is visible at `at`.
    pub fn visible_binding(&self, name: &Name, at: Span) -> Option<&Binding> {
        self.previous_visible_binding(name, at, None)
    }

    /// Return binding candidates for a reference using lexical visibility first,
    /// then prior same-name bindings outside the reference scope.
    pub fn visible_candidate_bindings_for_reference(
        &self,
        reference: &Reference,
    ) -> Vec<BindingId> {
        let all_bindings = self.bindings_for(&reference.name);
        let binding_ids = self
            .ancestor_scopes(reference.scope)
            .filter_map(|scope| {
                all_bindings.iter().copied().rev().find(|binding_id| {
                    let binding = self.binding(*binding_id);
                    binding.scope == scope && self.binding_visible_at(*binding_id, reference.span)
                })
            })
            .collect::<Vec<_>>();
        if !binding_ids.is_empty() {
            return binding_ids;
        }

        self.ancestor_scopes(reference.scope)
            .skip(1)
            .filter_map(|scope| {
                all_bindings.iter().copied().rev().find(|binding_id| {
                    let binding = self.binding(*binding_id);
                    binding.scope == scope && self.binding_visible_at(*binding_id, reference.span)
                })
            })
            .chain(all_bindings.iter().copied().filter(|binding_id| {
                let binding = self.binding(*binding_id);
                binding.scope != reference.scope
                    && binding.span.start.offset < reference.span.start.offset
            }))
            .collect::<FxHashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    }

    #[doc(hidden)]
    /// Returns whether `binding_id` is lexically visible at `at`.
    #[doc(hidden)]
    pub fn binding_visible_at(&self, binding_id: BindingId, at: Span) -> bool {
        let binding = self.binding(binding_id);
        binding.span.start.offset <= at.start.offset
            && self
                .ancestor_scopes(self.scope_at(at.start.offset))
                .any(|scope| scope == binding.scope)
    }

    /// Returns whether `binding_id` is cleared between its definition and `at`.
    #[doc(hidden)]
    pub fn binding_cleared_before(&self, binding_id: BindingId, at: Span) -> bool {
        let binding = self.binding(binding_id);
        self.cleared_variables
            .get(&(binding.scope, binding.name.clone()))
            .is_some_and(|cleared_offsets| {
                cleared_offsets.iter().any(|cleared_offset| {
                    *cleared_offset > binding.span.start.offset && *cleared_offset < at.start.offset
                })
            })
    }

    /// Returns whether `binding_id` and `reference_id` were recorded under the same command span.
    #[doc(hidden)]
    pub fn binding_and_reference_share_command(
        &self,
        binding_id: BindingId,
        reference_id: ReferenceId,
    ) -> bool {
        self.command_bindings.iter().any(|(command, bindings)| {
            bindings.contains(&binding_id)
                && self
                    .command_references
                    .get(command)
                    .is_some_and(|references| references.contains(&reference_id))
        })
    }

    /// Returns the previous visible binding for `name` at `at`, optionally ignoring one exact
    /// binding span.
    #[doc(hidden)]
    pub fn previous_visible_binding(
        &self,
        name: &Name,
        at: Span,
        ignored_binding_span: Option<Span>,
    ) -> Option<&Binding> {
        let scope = self.scope_at(at.start.offset);
        self.previous_visible_binding_id_in_scope_chain(
            name,
            scope,
            at.start.offset,
            ignored_binding_span,
        )
        .map(|binding_id| self.binding(binding_id))
    }

    /// Returns the binding visible for an associative-array lookup in `current_scope`.
    #[doc(hidden)]
    pub fn visible_binding_for_assoc_lookup(
        &self,
        name: &Name,
        current_scope: ScopeId,
        at: Span,
    ) -> Option<&Binding> {
        if let Some(binding_id) =
            self.previous_assoc_lookup_binding_id_in_scope(current_scope, name, at.start.offset)
        {
            return Some(self.binding(binding_id));
        }

        self.ancestor_scopes(current_scope)
            .skip(1)
            .find_map(|scope| {
                self.previous_visible_binding_id_in_scope(scope, name, at.start.offset, None)
            })
            .map(|binding_id| self.binding(binding_id))
    }

    /// Returns a visible binding for a contextual lookup, including named callers.
    #[doc(hidden)]
    pub fn visible_binding_for_lookup(
        &self,
        name: &Name,
        current_scope: ScopeId,
        at: Span,
    ) -> Option<&Binding> {
        if let Some(binding_id) = self.previous_visible_binding_id_in_scope_chain(
            name,
            current_scope,
            at.start.offset,
            None,
        ) {
            return Some(self.binding(binding_id));
        }

        self.visible_binding_from_named_callers(name, current_scope)
    }

    /// Returns a visible binding that decides contextual associative lookups.
    #[doc(hidden)]
    pub fn visible_assoc_lookup_binding_for_lookup(
        &self,
        name: &Name,
        current_scope: ScopeId,
        at: Span,
    ) -> Option<&Binding> {
        if let Some(binding) = self.visible_binding_for_assoc_lookup(name, current_scope, at) {
            return Some(binding);
        }

        self.visible_assoc_lookup_binding_from_named_callers(name, current_scope)
    }

    /// Returns whether an associative binding is visible for a contextual array lookup.
    pub fn assoc_binding_visible_for_lookup(
        &self,
        name: &Name,
        current_scope: ScopeId,
        at: Span,
    ) -> bool {
        if let Some(visible) = self.assoc_binding_visible_in_scope(name, current_scope, at) {
            return visible;
        }

        self.assoc_binding_visible_from_named_callers(name, current_scope)
    }

    fn assoc_binding_visible_in_scope(
        &self,
        name: &Name,
        current_scope: ScopeId,
        at: Span,
    ) -> Option<bool> {
        self.visible_binding_for_assoc_lookup(name, current_scope, at)
            .map(|binding| binding.attributes.contains(BindingAttributes::ASSOC))
    }

    fn assoc_binding_visible_from_named_callers(
        &self,
        name: &Name,
        current_scope: ScopeId,
    ) -> bool {
        let Some(function_names) = self.named_function_scope_names(current_scope) else {
            return false;
        };

        let mut seen = AssocCallerSeenNames::new();
        let mut worklist = SmallVec::<[Name; 4]>::new();
        worklist.extend(function_names.iter().cloned());

        while let Some(function_name) = worklist.pop() {
            if !seen.insert(&function_name) {
                continue;
            }

            for call_site in self.call_sites_for(&function_name) {
                if let Some(binding) = self.visible_binding_for_assoc_lookup(
                    name,
                    call_site.scope,
                    call_site.name_span,
                ) {
                    if binding.attributes.contains(BindingAttributes::ASSOC) {
                        return true;
                    }
                    continue;
                }

                if let Some(caller_names) = self.named_function_scope_names(call_site.scope) {
                    worklist.extend(caller_names.iter().cloned());
                }
            }
        }

        false
    }

    fn visible_assoc_lookup_binding_from_named_callers(
        &self,
        name: &Name,
        current_scope: ScopeId,
    ) -> Option<&Binding> {
        let function_names = self.named_function_scope_names(current_scope)?;

        let mut seen = AssocCallerSeenNames::new();
        let mut worklist = SmallVec::<[Name; 4]>::new();
        worklist.extend(function_names.iter().cloned());

        while let Some(function_name) = worklist.pop() {
            if !seen.insert(&function_name) {
                continue;
            }

            for call_site in self.call_sites_for(&function_name) {
                if let Some(binding) = self.visible_binding_for_assoc_lookup(
                    name,
                    call_site.scope,
                    call_site.name_span,
                ) {
                    return Some(binding);
                }

                if let Some(caller_names) = self.named_function_scope_names(call_site.scope) {
                    worklist.extend(caller_names.iter().cloned());
                }
            }
        }

        None
    }

    fn visible_binding_from_named_callers(
        &self,
        name: &Name,
        current_scope: ScopeId,
    ) -> Option<&Binding> {
        let function_names = self.named_function_scope_names(current_scope)?;

        let mut seen = AssocCallerSeenNames::new();
        let mut worklist = SmallVec::<[Name; 4]>::new();
        worklist.extend(function_names.iter().cloned());

        while let Some(function_name) = worklist.pop() {
            if !seen.insert(&function_name) {
                continue;
            }

            for call_site in self.call_sites_for(&function_name) {
                if let Some(binding_id) = self.previous_visible_binding_id_in_scope_chain(
                    name,
                    call_site.scope,
                    call_site.name_span.start.offset,
                    None,
                ) {
                    return Some(self.binding(binding_id));
                }

                if let Some(caller_names) = self.named_function_scope_names(call_site.scope) {
                    worklist.extend(caller_names.iter().cloned());
                }
            }
        }

        None
    }

    fn named_function_scope_names(&self, scope: ScopeId) -> Option<&[Name]> {
        self.ancestor_scopes(scope)
            .find_map(|scope_id| match &self.scope(scope_id).kind {
                ScopeKind::Function(FunctionScopeKind::Named(names)) => Some(names.as_slice()),
                _ => None,
            })
    }

    /// Returns whether any binding for `name` exists in the model.
    pub fn defined_anywhere(&self, name: &Name) -> bool {
        self.binding_index.contains_key(name)
    }

    /// Returns whether `name` is defined inside at least one function scope.
    pub fn defined_in_any_function(&self, name: &Name) -> bool {
        self.binding_index.get(name).is_some_and(|bindings| {
            bindings.iter().any(|binding| {
                matches!(
                    self.scopes[self.bindings[binding.index()].scope.index()].kind,
                    ScopeKind::Function(_)
                )
            })
        })
    }

    /// Returns whether runtime behavior can consume `binding_id` even without a direct read in
    /// the current file.
    pub fn is_runtime_consumed_binding(&self, binding_id: BindingId) -> bool {
        self.bindings
            .get(binding_id.index())
            .is_some_and(|binding| self.runtime.is_always_used_binding(&binding.name))
    }

    /// Returns whether `name` has a required-read reference in `scope` before `offset`.
    pub fn required_before(&self, name: &Name, scope: ScopeId, offset: usize) -> bool {
        self.references.iter().any(|reference| {
            reference.scope == scope
                && &reference.name == name
                && matches!(reference.kind, ReferenceKind::RequiredRead)
                && reference.span.start.offset < offset
        })
    }

    /// Returns whether an ancestor scope outside `scope` defines `name`.
    pub fn maybe_defined_outside(&self, name: &Name, scope: ScopeId) -> bool {
        self.ancestor_scopes(scope)
            .skip(1)
            .any(|scope| self.scopes[scope.index()].bindings.contains_key(name))
    }

    /// Returns references that did not resolve to any binding.
    pub fn unresolved_references(&self) -> &[ReferenceId] {
        &self.unresolved
    }

    /// Returns the innermost semantic scope containing `offset`.
    pub fn scope_at(&self, offset: usize) -> ScopeId {
        self.scope_lookup
            .scope_at(&self.scopes, offset)
            .unwrap_or(ScopeId(0))
    }

    /// Returns the kind metadata for `scope`.
    pub fn scope_kind(&self, scope: ScopeId) -> &ScopeKind {
        &self.scopes[scope.index()].kind
    }

    fn scope_is_transient(&self, scope: ScopeId) -> bool {
        matches!(
            self.scope_kind(scope),
            ScopeKind::Subshell | ScopeKind::CommandSubstitution | ScopeKind::Pipeline
        )
    }

    /// Iterates `scope` and then each lexical ancestor scope outward.
    pub fn ancestor_scopes(&self, scope: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
        ancestor_scopes(&self.scopes, scope)
    }

    /// Returns whether `scope` is equal to `ancestor_scope` or nested within it.
    pub fn scope_is_in_scope_or_descendant(&self, scope: ScopeId, ancestor_scope: ScopeId) -> bool {
        self.ancestor_scopes(scope)
            .any(|scope| scope == ancestor_scope)
    }

    /// Returns whether `scope` is strictly nested within `ancestor_scope`.
    pub fn scope_is_descendant_of(&self, scope: ScopeId, ancestor_scope: ScopeId) -> bool {
        scope != ancestor_scope && self.scope_is_in_scope_or_descendant(scope, ancestor_scope)
    }

    /// Returns the nearest enclosing function scope for `scope`, if one exists.
    pub fn enclosing_function_scope(&self, scope: ScopeId) -> Option<ScopeId> {
        enclosing_function_scope(&self.scopes, scope)
    }

    /// Iterates transient ancestor scopes between `scope` and its enclosing function boundary.
    #[doc(hidden)]
    pub fn transient_ancestor_scopes_within_function(
        &self,
        scope: ScopeId,
    ) -> impl Iterator<Item = ScopeId> + '_ {
        self.ancestor_scopes(scope)
            .take_while(|scope_id| !matches!(self.scope_kind(*scope_id), ScopeKind::Function(_)))
            .filter(|scope_id| self.scope_is_transient(*scope_id))
    }

    /// Returns the innermost transient ancestor scope before crossing a function boundary.
    #[doc(hidden)]
    pub fn innermost_transient_scope_within_function(&self, scope: ScopeId) -> Option<ScopeId> {
        self.transient_ancestor_scopes_within_function(scope).next()
    }

    /// Returns the enclosing function scope only when no transient boundary intervenes.
    #[doc(hidden)]
    pub fn enclosing_function_scope_without_transient_boundary(
        &self,
        scope: ScopeId,
    ) -> Option<ScopeId> {
        if self
            .transient_ancestor_scopes_within_function(scope)
            .next()
            .is_some()
        {
            None
        } else {
            self.enclosing_function_scope(scope)
        }
    }

    fn previous_visible_binding_id_in_scope_chain(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
        ignored_binding_span: Option<Span>,
    ) -> Option<BindingId> {
        self.ancestor_scopes(scope).find_map(|scope_id| {
            self.previous_visible_binding_id_in_scope(scope_id, name, offset, ignored_binding_span)
        })
    }

    fn previous_visible_binding_id_in_scope(
        &self,
        scope: ScopeId,
        name: &Name,
        offset: usize,
        ignored_binding_span: Option<Span>,
    ) -> Option<BindingId> {
        let bindings = self.scopes[scope.index()].bindings.get(name)?;
        previous_visible_binding_id_from_slice(
            &self.bindings,
            bindings,
            offset,
            ignored_binding_span,
        )
    }

    fn previous_assoc_lookup_binding_id_in_scope(
        &self,
        scope: ScopeId,
        name: &Name,
        offset: usize,
    ) -> Option<BindingId> {
        let bindings = self
            .assoc_lookup_binding_index()
            .blocking_bindings_by_scope
            .get(scope.index())
            .and_then(|bindings_by_name| bindings_by_name.get(name))?;
        previous_visible_binding_id_from_slice(&self.bindings, bindings, offset, None)
    }

    fn assoc_lookup_binding_index(&self) -> &AssocLookupBindingIndex {
        self.assoc_lookup_binding_index.get_or_init(|| {
            let blocking_bindings_by_scope = self
                .scopes
                .iter()
                .map(|scope| {
                    let mut bindings_by_name = FxHashMap::default();
                    for (name, bindings) in &scope.bindings {
                        let filtered = bindings
                            .iter()
                            .copied()
                            .filter(|binding_id| {
                                binding_blocks_same_scope_assoc_lookup(
                                    &self.bindings[binding_id.index()],
                                )
                            })
                            .collect::<Vec<_>>();
                        if !filtered.is_empty() {
                            bindings_by_name.insert(name.clone(), filtered.into_boxed_slice());
                        }
                    }
                    bindings_by_name
                })
                .collect();

            AssocLookupBindingIndex {
                blocking_bindings_by_scope,
            }
        })
    }

    /// Returns the most specific recorded flow context associated with `span`.
    pub fn flow_context_at(&self, span: &Span) -> Option<&FlowContext> {
        self.flow_contexts
            .iter()
            .rfind(|(candidate, _)| candidate == span)
            .map(|(_, context)| context)
            .or_else(|| {
                self.flow_contexts
                    .iter()
                    .enumerate()
                    .filter(|(_, (candidate, _))| {
                        contains_span(*candidate, *span) || contains_span(*span, *candidate)
                    })
                    .min_by_key(|(index, (candidate, _))| {
                        (
                            candidate.end.offset.saturating_sub(candidate.start.offset),
                            std::cmp::Reverse(*index),
                        )
                    })
                    .map(|(_, (_, context))| context)
            })
    }

    fn add_imported_binding(
        &mut self,
        provided: &ProvidedBinding,
        scope: ScopeId,
        span: Span,
        command_span: Option<Span>,
        origin_paths: Vec<PathBuf>,
        file_entry_contract: bool,
    ) -> BindingId {
        let mut attributes = BindingAttributes::empty();
        if provided.certainty == ContractCertainty::Possible {
            attributes |= BindingAttributes::IMPORTED_POSSIBLE;
        }
        if provided.kind == ProvidedBindingKind::Function {
            attributes |= BindingAttributes::IMPORTED_FUNCTION;
        }
        if file_entry_contract {
            attributes |= BindingAttributes::IMPORTED_FILE_ENTRY;
            if provided.file_entry_initialization == FileEntryBindingInitialization::Initialized {
                attributes |= BindingAttributes::IMPORTED_FILE_ENTRY_INITIALIZED;
            }
        }

        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            id,
            name: provided.name.clone(),
            kind: BindingKind::Imported,
            origin: BindingOrigin::Imported {
                definition_span: span,
            },
            scope,
            span,
            references: Vec::new(),
            attributes,
        });
        insert_binding_id_sorted(
            self.binding_index.entry(provided.name.clone()).or_default(),
            &self.bindings,
            id,
        );
        insert_binding_id_sorted(
            self.scopes[scope.index()]
                .bindings
                .entry(provided.name.clone())
                .or_default(),
            &self.bindings,
            id,
        );
        if provided.kind == ProvidedBindingKind::Function {
            insert_binding_id_sorted(
                self.functions.entry(provided.name.clone()).or_default(),
                &self.bindings,
                id,
            );
        }
        if let Some(command_span) = command_span {
            self.command_bindings
                .entry(SpanKey::new(command_span))
                .or_default()
                .push(id);
        }
        if !origin_paths.is_empty() {
            self.import_origins_by_binding.insert(id, origin_paths);
        }
        self.bindings_by_definition_span.take();
        id
    }

    pub(crate) fn apply_file_entry_contract(&mut self, contract: FileContract, file: &File) {
        if contract.required_reads.is_empty()
            && contract.provided_bindings.is_empty()
            && contract.provided_functions.is_empty()
            && !contract.externally_consumed_bindings
        {
            return;
        }

        if contract.externally_consumed_bindings {
            self.mark_file_entry_consumed_bindings();
        }

        let mut synthetic_reads = self.synthetic_reads.clone();
        for name in contract.required_reads {
            synthetic_reads.push(SyntheticRead {
                scope: ScopeId(0),
                span: file.span,
                name,
            });
        }

        let entry_span = Span::from_positions(file.span.start, file.span.start);
        let mut entry_bindings = self.entry_bindings.clone();
        let function_origin_paths = contract
            .provided_functions
            .iter()
            .map(|function| (function.name.clone(), function.origin_paths.clone()))
            .collect::<FxHashMap<_, _>>();
        let mut provided_bindings = contract.provided_bindings;
        for function in contract.provided_functions {
            if !provided_bindings.iter().any(|binding| {
                binding.kind == ProvidedBindingKind::Function && binding.name == function.name
            }) {
                provided_bindings.push(ProvidedBinding::new(
                    function.name,
                    ProvidedBindingKind::Function,
                    ContractCertainty::Definite,
                ));
            }
        }
        for binding in &provided_bindings {
            let origin_paths = function_origin_paths
                .get(&binding.name)
                .cloned()
                .unwrap_or_default();
            let id = self.add_imported_binding(
                binding,
                ScopeId(0),
                entry_span,
                None,
                origin_paths,
                true,
            );
            entry_bindings.push(id);
        }

        self.set_synthetic_reads(dedup_synthetic_reads(synthetic_reads));
        self.set_entry_bindings(entry_bindings);
        self.invalidate_function_binding_lookup();
        self.resolve_unresolved_references();
        self.rebuild_call_graph();
    }

    fn mark_file_entry_consumed_bindings(&mut self) {
        for binding in &mut self.bindings {
            if file_entry_contract_can_consume_binding(binding) {
                binding.attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
            }
        }
        self.heuristic_unused_assignments.retain(|binding_id| {
            !self.bindings[binding_id.index()]
                .attributes
                .contains(BindingAttributes::EXTERNALLY_CONSUMED)
        });
    }

    pub(crate) fn apply_source_contracts(
        &mut self,
        synthetic_reads: Vec<SyntheticRead>,
        imported_bindings: Vec<ImportedBindingContractSite>,
        source_ref_resolutions: Vec<SourceRefResolution>,
        source_ref_explicitness: Vec<bool>,
        source_ref_diagnostic_classes: Vec<SourceRefDiagnosticClass>,
    ) {
        if synthetic_reads.is_empty()
            && imported_bindings.is_empty()
            && source_ref_resolutions.is_empty()
            && source_ref_explicitness.is_empty()
            && source_ref_diagnostic_classes.is_empty()
        {
            return;
        }

        let mut merged_reads = self.synthetic_reads.clone();
        merged_reads.extend(synthetic_reads);
        self.set_synthetic_reads(dedup_synthetic_reads(merged_reads));

        if !source_ref_resolutions.is_empty() {
            debug_assert_eq!(source_ref_resolutions.len(), self.source_refs.len());
            for (source_ref, resolution) in self.source_refs.iter_mut().zip(source_ref_resolutions)
            {
                source_ref.resolution = resolution;
            }
        }
        if !source_ref_explicitness.is_empty() {
            debug_assert_eq!(source_ref_explicitness.len(), self.source_refs.len());
            for (source_ref, explicitly_provided) in
                self.source_refs.iter_mut().zip(source_ref_explicitness)
            {
                source_ref.explicitly_provided = explicitly_provided;
            }
        }
        if !source_ref_diagnostic_classes.is_empty() {
            debug_assert_eq!(source_ref_diagnostic_classes.len(), self.source_refs.len());
            for (source_ref, diagnostic_class) in self
                .source_refs
                .iter_mut()
                .zip(source_ref_diagnostic_classes)
            {
                source_ref.diagnostic_class = diagnostic_class;
            }
        }

        for site in imported_bindings {
            self.add_imported_binding(
                &site.binding,
                site.scope,
                site.span,
                Some(site.span),
                site.origin_paths,
                false,
            );
        }
        self.invalidate_function_binding_lookup();
        self.resolve_unresolved_references();
        self.rebuild_call_graph();
    }

    fn invalidate_function_binding_lookup(&mut self) {
        self.unconditional_function_bindings.take();
        self.function_bindings_by_scope.take();
        self.visible_function_call_bindings.take();
        self.function_definition_binding_ids.take();
    }

    fn rebuild_call_graph(&mut self) {
        self.call_graph = build_call_graph(
            &self.scopes,
            &self.bindings,
            &self.functions,
            &self.call_sites,
        );
    }

    fn resolve_unresolved_references(&mut self) {
        let unresolved = std::mem::take(&mut self.unresolved);
        for reference_id in unresolved {
            let reference = &self.references[reference_id.index()];
            let resolved =
                self.resolve_binding_at(&reference.name, reference.scope, reference.span);
            if let Some(binding_id) = resolved {
                self.resolved.insert(reference_id, binding_id);
                self.bindings[binding_id.index()]
                    .references
                    .push(reference_id);
            } else {
                self.unresolved.push(reference_id);
            }
        }
    }

    fn resolve_binding_at(&self, name: &Name, scope: ScopeId, span: Span) -> Option<BindingId> {
        for scope in self.ancestor_scopes(scope) {
            let Some(bindings) = self.scopes[scope.index()].bindings.get(name) else {
                continue;
            };

            for binding in bindings.iter().rev().copied() {
                if self.bindings[binding.index()].span.start.offset <= span.start.offset {
                    return Some(binding);
                }
            }
        }
        None
    }

    /// Returns same-name function-definition bindings for `name`, ordered by definition offset.
    pub fn function_definitions(&self, name: &Name) -> &[BindingId] {
        self.functions
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns recorded call sites for `name`.
    pub fn call_sites_for(&self, name: &Name) -> &[CallSite] {
        self.call_sites
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the current call-graph summary for the model.
    pub fn call_graph(&self) -> &CallGraph {
        &self.call_graph
    }

    /// Returns declaration commands recorded in the file.
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    /// Returns the declaration recorded for the command with syntax span `span`.
    pub fn declaration_for_command_span(&self, span: Span) -> Option<&Declaration> {
        let index = self
            .declarations_by_command_span
            .get_or_init(|| build_declarations_by_command_span(&self.declarations));
        index
            .get(&SpanKey::new(span))
            .map(|declaration_index| &self.declarations[*declaration_index])
    }

    /// Returns the function-definition binding recorded for command span `span`, if any.
    pub fn function_definition_binding_for_command_span(&self, span: Span) -> Option<BindingId> {
        self.command_bindings
            .get(&SpanKey::new(span))
            .and_then(|bindings| {
                bindings.iter().copied().find(|binding_id| {
                    matches!(
                        self.bindings[binding_id.index()].kind,
                        BindingKind::FunctionDefinition
                    )
                })
            })
    }

    /// Returns source-like file references discovered in the script.
    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    /// Returns synthetic reads introduced by contracts or semantic modeling.
    pub fn synthetic_reads(&self) -> &[SyntheticRead] {
        &self.synthetic_reads
    }

    /// Returns origin paths that contributed the imported binding `id`.
    pub fn import_origins_for_binding(&self, id: BindingId) -> &[PathBuf] {
        self.import_origins_by_binding
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns flattened statement-sequence commands recorded during traversal.
    pub fn statement_sequence_commands(&self) -> &[StatementSequenceCommand] {
        self.recorded_program.statement_sequence_commands()
    }

    /// Returns the number of recorded semantic commands.
    pub fn command_count(&self) -> usize {
        self.recorded_program.commands().len()
    }

    /// Returns command ids for every syntax-backed recorded command.
    pub fn commands(&self) -> &[CommandId] {
        &self.command_topology().syntax_backed_ids
    }

    /// Returns command ids for the structural command stream.
    pub fn structural_commands(&self) -> &[CommandId] {
        &self.command_topology().structural_ids
    }

    /// Returns the recorded semantic context for `id`.
    pub fn command_context(&self, id: CommandId) -> Option<&SemanticCommandContext> {
        self.command_topology()
            .contexts
            .get(id.index())
            .and_then(Option::as_ref)
    }

    /// Iterates recorded command contexts in command-id order.
    pub fn command_contexts(&self) -> impl Iterator<Item = &SemanticCommandContext> {
        self.command_topology()
            .contexts
            .iter()
            .filter_map(Option::as_ref)
    }

    /// Iterates only structural command contexts.
    pub fn structural_command_contexts(&self) -> impl Iterator<Item = &SemanticCommandContext> {
        self.command_contexts()
            .filter(|context| context.is_structural())
    }

    /// Returns the surrounding condition-list role for `id`, if one applies.
    pub fn command_condition_role(&self, id: CommandId) -> Option<CommandConditionRole> {
        self.command_context(id)
            .and_then(SemanticCommandContext::condition_role)
    }

    /// Returns whether `id` came from a command-like expansion inside a word.
    pub fn command_is_nested_word_command(&self, id: CommandId) -> bool {
        self.command_context(id)
            .is_some_and(SemanticCommandContext::is_nested_word_command)
    }

    /// Returns the recorded statement span for `id`.
    pub fn command_span(&self, id: CommandId) -> Span {
        self.recorded_program.command(id).span
    }

    /// Returns the underlying syntax-node span for `id`.
    pub fn command_syntax_span(&self, id: CommandId) -> Span {
        self.recorded_program.command(id).syntax_span
    }

    /// Returns the syntax-backed command kind for `id`.
    pub fn command_kind(&self, id: CommandId) -> CommandKind {
        self.command_syntax_kind(id)
            .expect("semantic command syntax kind is recorded")
    }

    /// Returns the first syntax-backed command recorded for exact syntax span `span`.
    pub fn command_by_span(&self, span: Span) -> Option<CommandId> {
        self.command_topology()
            .ids_by_syntax_span
            .get(&SpanKey::new(span))
            .and_then(|ids| {
                ids.iter()
                    .copied()
                    .find(|id| self.command_syntax_kind(*id).is_some())
            })
    }

    /// Returns the command recorded for exact syntax span `span` and syntax kind `kind`.
    pub fn command_by_span_and_kind(&self, span: Span, kind: CommandKind) -> Option<CommandId> {
        self.command_topology()
            .ids_by_syntax_span
            .get(&SpanKey::new(span))
            .and_then(|ids| {
                ids.iter()
                    .copied()
                    .find(|id| self.command_syntax_kind(*id) == Some(kind))
            })
    }

    /// Returns the structural parent command of `id`, if one exists.
    pub fn command_parent_id(&self, id: CommandId) -> Option<CommandId> {
        self.command_topology().parent_ids[id.index()]
    }

    /// Returns structural child commands nested under `id`.
    pub fn command_children(&self, id: CommandId) -> &[CommandId] {
        &self.command_topology().child_ids[id.index()]
    }

    /// Returns the syntax-backed parent command of `id`, if one exists.
    pub fn syntax_backed_command_parent_id(&self, id: CommandId) -> Option<CommandId> {
        self.command_topology().syntax_backed_parent_ids[id.index()]
    }

    /// Returns syntax-backed child commands nested directly under `id`.
    pub fn syntax_backed_command_children(&self, id: CommandId) -> &[CommandId] {
        &self.command_topology().syntax_backed_child_ids[id.index()]
    }

    /// Returns the innermost syntax-backed command whose syntax span contains `offset`.
    pub fn innermost_command_id_at(&self, offset: usize) -> Option<CommandId> {
        let topology = self.command_topology();
        let mut innermost = None;
        for id in topology.offset_order.iter().copied() {
            let span = self.command_syntax_span(id);
            if span.start.offset > offset {
                break;
            }
            if offset <= span.end.offset && self.command_syntax_kind(id).is_some() {
                innermost = Some(id);
            }
        }
        innermost
    }

    /// Returns the innermost command known to contain `offset` using the topology index.
    pub fn innermost_command_id_containing_offset(&self, offset: usize) -> Option<CommandId> {
        let topology = self.command_topology();
        let upper_bound = topology
            .containing_offset_entries
            .partition_point(|entry| entry.start_offset <= offset);
        let entry = topology
            .containing_offset_entries
            .get(upper_bound.checked_sub(1)?)?;
        (offset <= entry.end_offset).then_some(entry.id)
    }

    /// Returns logical list commands flattened by the semantic traversal.
    pub fn list_commands(&self) -> Vec<SemanticListCommand> {
        self.recorded_program
            .commands()
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let RecordedCommandKind::List { first, rest } = command.kind else {
                    return None;
                };
                let command_id = CommandId(index as u32);
                if self.command_parent_id(command_id).is_some_and(|parent| {
                    matches!(
                        self.recorded_program.command(parent).kind,
                        RecordedCommandKind::List { .. }
                    )
                }) {
                    return None;
                }

                let mut segments = Vec::new();
                self.flatten_list_segment(first, None, &mut segments);
                for item in self.recorded_program.list_items(rest) {
                    self.flatten_list_segment(
                        item.command,
                        Some(RecordedListOperatorWithSpan {
                            operator: item.operator,
                            span: item.operator_span,
                        }),
                        &mut segments,
                    );
                }

                Some(SemanticListCommand {
                    span: command.span,
                    segments: segments.into_boxed_slice(),
                })
            })
            .collect()
    }

    fn flatten_list_segment(
        &self,
        command: CommandId,
        operator_before: Option<RecordedListOperatorWithSpan>,
        out: &mut Vec<SemanticListSegment>,
    ) {
        if let RecordedCommandKind::List { first, rest } =
            self.recorded_program.command(command).kind
        {
            self.flatten_list_segment(first, operator_before, out);
            for item in self.recorded_program.list_items(rest) {
                self.flatten_list_segment(
                    item.command,
                    Some(RecordedListOperatorWithSpan {
                        operator: item.operator,
                        span: item.operator_span,
                    }),
                    out,
                );
            }
            return;
        }

        out.push(SemanticListSegment {
            command_span: self.recorded_program.command(command).span,
            operator_before: operator_before.map(|operator| SemanticListOperator {
                kind: match operator.operator {
                    RecordedListOperator::And => SemanticListOperatorKind::And,
                    RecordedListOperator::Or => SemanticListOperatorKind::Or,
                },
                span: operator.span,
            }),
        });
    }

    /// Returns pipeline commands flattened by the semantic traversal.
    pub fn pipeline_commands(&self) -> Vec<SemanticPipelineCommand> {
        self.recorded_program
            .commands()
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let RecordedCommandKind::Pipeline { segments } = command.kind else {
                    return None;
                };
                let command_id = CommandId(index as u32);
                if self.command_parent_id(command_id).is_some_and(|parent| {
                    matches!(
                        self.recorded_program.command(parent).kind,
                        RecordedCommandKind::Pipeline { .. }
                    )
                }) {
                    return None;
                }

                let mut flattened = Vec::new();
                for segment in self.recorded_program.pipeline_segments(segments) {
                    self.flatten_pipeline_segment(
                        segment.command,
                        segment.operator_before,
                        &mut flattened,
                    );
                }

                Some(SemanticPipelineCommand {
                    span: command.span,
                    segments: flattened.into_boxed_slice(),
                })
            })
            .collect()
    }

    fn flatten_pipeline_segment(
        &self,
        command: CommandId,
        operator_before: Option<crate::cfg::RecordedPipelineOperator>,
        out: &mut Vec<SemanticPipelineSegment>,
    ) {
        if let RecordedCommandKind::Pipeline { segments } =
            self.recorded_program.command(command).kind
        {
            for (index, segment) in self
                .recorded_program
                .pipeline_segments(segments)
                .iter()
                .enumerate()
            {
                let operator = if index == 0 {
                    operator_before
                } else {
                    segment.operator_before
                };
                self.flatten_pipeline_segment(segment.command, operator, out);
            }
            return;
        }

        out.push(SemanticPipelineSegment {
            command_span: self.recorded_program.command(command).span,
            operator_before: operator_before.map(|operator| SemanticPipelineOperator {
                kind: match operator.operator {
                    RecordedPipelineOperatorKind::Pipe => SemanticPipelineOperatorKind::Pipe,
                    RecordedPipelineOperatorKind::PipeAll => SemanticPipelineOperatorKind::PipeAll,
                },
                span: operator.span,
            }),
        });
    }

    pub(crate) fn recorded_program(&self) -> &RecordedProgram {
        &self.recorded_program
    }

    fn command_topology(&self) -> &CommandTopology {
        self.command_topology
            .get_or_init(|| build_command_topology(self))
    }

    fn command_syntax_kind(&self, id: CommandId) -> Option<CommandKind> {
        self.recorded_program.command(id).syntax_kind
    }

    pub(crate) fn set_synthetic_reads(&mut self, synthetic_reads: Vec<SyntheticRead>) {
        self.synthetic_reads = synthetic_reads;
    }

    fn set_entry_bindings(&mut self, entry_bindings: Vec<BindingId>) {
        self.entry_bindings = entry_bindings;
    }

    fn function_binding_lookup(&self) -> FunctionBindingLookup<'_> {
        FunctionBindingLookup {
            program: &self.recorded_program,
            scopes: &self.scopes,
            bindings: &self.bindings,
            call_sites: &self.call_sites,
            unconditional_function_bindings: self.unconditional_function_bindings(),
            function_bindings_by_scope: self.function_binding_scope_index(),
        }
    }

    fn unconditional_function_bindings(&self) -> &FxHashSet<BindingId> {
        self.unconditional_function_bindings.get_or_init(|| {
            function_resolution::collect_unconditional_function_bindings(
                &self.recorded_program,
                &self.command_bindings,
                &self.bindings,
            )
        })
    }

    pub(crate) fn function_binding_scope_index(
        &self,
    ) -> &FxHashMap<ScopeId, SmallVec<[BindingId; 2]>> {
        self.function_bindings_by_scope
            .get_or_init(|| function_resolution::function_bindings_by_scope(&self.recorded_program))
    }

    pub(crate) fn visible_function_call_bindings(&self) -> &FxHashMap<SpanKey, BindingId> {
        self.visible_function_call_bindings.get_or_init(|| {
            self.function_binding_lookup()
                .visible_function_call_bindings()
        })
    }

    fn dataflow_context<'a>(&'a self, cfg: &'a ControlFlowGraph) -> DataflowContext<'a> {
        DataflowContext {
            cfg,
            runtime: &self.runtime,
            scopes: &self.scopes,
            bindings: &self.bindings,
            references: &self.references,
            predefined_runtime_refs: &self.predefined_runtime_refs,
            guarded_parameter_refs: &self.guarded_parameter_refs,
            parameter_guard_flow_refs: &self.parameter_guard_flow_refs,
            self_referential_assignment_refs: &self.self_referential_assignment_refs,
            resolved: &self.resolved,
            call_sites: &self.call_sites,
            visible_function_call_bindings: self.visible_function_call_bindings(),
            function_body_scopes: &self.recorded_program.function_body_scopes,
            indirect_targets_by_reference: &self.indirect_targets_by_reference,
            array_like_indirect_expansion_refs: &self.array_like_indirect_expansion_refs,
            synthetic_reads: &self.synthetic_reads,
            entry_bindings: &self.entry_bindings,
        }
    }
}

fn file_entry_contract_can_consume_binding(binding: &Binding) -> bool {
    if binding.attributes.contains(BindingAttributes::LOCAL) {
        return false;
    }

    matches!(
        binding.kind,
        BindingKind::Assignment
            | BindingKind::ArrayAssignment
            | BindingKind::AppendAssignment
            | BindingKind::ParameterDefaultAssignment
            | BindingKind::LoopVariable
            | BindingKind::ReadTarget
            | BindingKind::MapfileTarget
            | BindingKind::PrintfTarget
            | BindingKind::GetoptsTarget
            | BindingKind::ArithmeticAssignment
            | BindingKind::Declaration(_)
    )
}

fn build_command_topology(model: &SemanticModel) -> CommandTopology {
    let program = model.recorded_program();
    let command_count = program.commands().len();
    let ids = (0..command_count)
        .map(|index| CommandId(index as u32))
        .collect::<Vec<_>>();
    let mut ids_by_syntax_span = FxHashMap::<SpanKey, SmallVec<[CommandId; 1]>>::default();
    let mut parent_ids = vec![None; command_count];
    let mut child_ids = vec![Vec::new(); command_count];
    let mut nested_region_command_ids = FxHashSet::default();
    let mut nested_region_root_command_ids = FxHashSet::default();

    for id in ids.iter().copied() {
        let command = program.command(id);
        ids_by_syntax_span
            .entry(SpanKey::new(command.syntax_span))
            .or_default()
            .push(id);
        record_command_children(
            program,
            id,
            &mut parent_ids,
            &mut child_ids,
            &mut nested_region_command_ids,
            &mut nested_region_root_command_ids,
        );
    }
    let structural_parent_ids = parent_ids.clone();
    let structural_child_ids = child_ids.clone();

    attach_function_body_commands(model, &ids, &mut parent_ids, &mut child_ids);
    attach_containing_command_parents(model, &ids, &mut parent_ids, &mut child_ids);
    let nested_region_depths = build_nested_region_depths(
        command_count,
        &parent_ids,
        &child_ids,
        &nested_region_root_command_ids,
    );

    let mut structural_ids = ids
        .iter()
        .copied()
        .filter(|id| !nested_region_command_ids.contains(id))
        .collect::<Vec<_>>();
    structural_ids
        .sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));

    let mut offset_order = ids.clone();
    offset_order
        .sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));

    let syntax_backed_ids = ids
        .into_iter()
        .filter(|id| program.command(*id).syntax_kind.is_some())
        .collect::<Vec<_>>();
    let syntax_backed_parent_ids =
        build_syntax_backed_command_parent_ids(model, &syntax_backed_ids, &parent_ids);
    let syntax_backed_child_ids =
        build_command_child_ids(command_count, &syntax_backed_ids, &syntax_backed_parent_ids);
    let containing_offset_entries =
        build_command_containing_offset_entries(model, &syntax_backed_ids);
    let condition_contexts = build_command_condition_contexts(
        program,
        command_count,
        &structural_parent_ids,
        &structural_child_ids,
    );
    let contexts = build_command_contexts(
        model,
        &syntax_backed_ids,
        &structural_ids,
        &nested_region_command_ids,
        &nested_region_depths,
        &condition_contexts,
    );

    CommandTopology {
        syntax_backed_ids,
        structural_ids,
        contexts,
        ids_by_syntax_span,
        parent_ids,
        child_ids,
        syntax_backed_parent_ids,
        syntax_backed_child_ids,
        offset_order,
        containing_offset_entries,
    }
}

fn build_command_contexts(
    model: &SemanticModel,
    syntax_backed_ids: &[CommandId],
    structural_ids: &[CommandId],
    nested_region_command_ids: &FxHashSet<CommandId>,
    nested_region_depths: &[usize],
    condition_contexts: &[CommandConditionContext],
) -> Vec<Option<SemanticCommandContext>> {
    let program = model.recorded_program();
    let structural_ids = structural_ids.iter().copied().collect::<FxHashSet<_>>();
    let mut contexts = vec![None; program.commands().len()];
    for id in syntax_backed_ids.iter().copied() {
        let command = program.command(id);
        let Some(kind) = command.syntax_kind else {
            continue;
        };
        let Some(scope) = command.scope else {
            continue;
        };
        let Some(flow) = command.flow_context else {
            continue;
        };
        contexts[id.index()] = Some(SemanticCommandContext {
            id,
            span: command.span,
            syntax_span: command.syntax_span,
            kind,
            scope,
            flow,
            structural: structural_ids.contains(&id),
            nested_word_command: nested_region_command_ids.contains(&id),
            nested_word_command_depth: nested_region_depths[id.index()],
            in_if_condition: condition_contexts
                .get(id.index())
                .is_some_and(|context| context.in_if_condition),
            in_elif_condition: condition_contexts
                .get(id.index())
                .is_some_and(|context| context.in_elif_condition),
            condition_role: condition_contexts
                .get(id.index())
                .and_then(|context| context.role),
        });
    }
    contexts
}

#[derive(Debug, Clone, Copy, Default)]
struct CommandConditionContext {
    role: Option<CommandConditionRole>,
    in_if_condition: bool,
    in_elif_condition: bool,
}

fn build_command_condition_contexts(
    program: &RecordedProgram,
    command_count: usize,
    parent_ids: &[Option<CommandId>],
    child_ids: &[Vec<CommandId>],
) -> Vec<CommandConditionContext> {
    let mut starts = vec![SmallVec::<[ConditionAssignment; 1]>::new(); command_count];
    let mut contexts = vec![CommandConditionContext::default(); command_count];
    for id in (0..command_count).map(|index| CommandId(index as u32)) {
        record_command_condition_starts(program, id, &mut starts);
    }

    let mut visited = vec![false; command_count];
    for id in (0..command_count).map(|index| CommandId(index as u32)) {
        if parent_ids[id.index()].is_none() {
            propagate_command_condition_contexts(
                id,
                CommandConditionContext::default(),
                &starts,
                child_ids,
                &mut contexts,
                &mut visited,
            );
        }
    }
    for id in (0..command_count).map(|index| CommandId(index as u32)) {
        if !visited[id.index()] {
            propagate_command_condition_contexts(
                id,
                CommandConditionContext::default(),
                &starts,
                child_ids,
                &mut contexts,
                &mut visited,
            );
        }
    }
    contexts
}

fn record_command_condition_starts(
    program: &RecordedProgram,
    id: CommandId,
    starts: &mut [SmallVec<[ConditionAssignment; 1]>],
) {
    match program.command(id).kind {
        RecordedCommandKind::If {
            condition,
            elif_branches,
            ..
        } => {
            record_condition_range_starts(
                program,
                condition,
                ConditionAssignment {
                    role: CommandConditionRole::If,
                    in_if_condition: true,
                    in_elif_condition: false,
                },
                starts,
            );
            for branch in program.elif_branches(elif_branches) {
                record_condition_range_starts(
                    program,
                    branch.condition,
                    ConditionAssignment {
                        role: CommandConditionRole::Elif,
                        in_if_condition: true,
                        in_elif_condition: true,
                    },
                    starts,
                );
            }
        }
        RecordedCommandKind::While { condition, .. } => {
            record_condition_range_starts(
                program,
                condition,
                ConditionAssignment {
                    role: CommandConditionRole::While,
                    in_if_condition: false,
                    in_elif_condition: false,
                },
                starts,
            );
        }
        RecordedCommandKind::Until { condition, .. } => {
            record_condition_range_starts(
                program,
                condition,
                ConditionAssignment {
                    role: CommandConditionRole::Until,
                    in_if_condition: false,
                    in_elif_condition: false,
                },
                starts,
            );
        }
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit
        | RecordedCommandKind::List { .. }
        | RecordedCommandKind::For { .. }
        | RecordedCommandKind::Select { .. }
        | RecordedCommandKind::ArithmeticFor { .. }
        | RecordedCommandKind::Case { .. }
        | RecordedCommandKind::BraceGroup { .. }
        | RecordedCommandKind::Subshell { .. }
        | RecordedCommandKind::Pipeline { .. } => {}
    }
}

#[derive(Debug, Clone, Copy)]
struct ConditionAssignment {
    role: CommandConditionRole,
    in_if_condition: bool,
    in_elif_condition: bool,
}

fn record_condition_range_starts(
    program: &RecordedProgram,
    range: crate::cfg::RecordedCommandRange,
    assignment: ConditionAssignment,
    starts: &mut [SmallVec<[ConditionAssignment; 1]>],
) {
    for command in program.commands_in(range).iter().copied() {
        starts[command.index()].push(assignment);
    }
}

fn propagate_command_condition_contexts(
    root: CommandId,
    inherited: CommandConditionContext,
    starts: &[SmallVec<[ConditionAssignment; 1]>],
    child_ids: &[Vec<CommandId>],
    contexts: &mut [CommandConditionContext],
    visited: &mut [bool],
) {
    let mut stack = vec![(root, inherited)];
    while let Some((id, mut context)) = stack.pop() {
        if visited[id.index()] {
            continue;
        }
        visited[id.index()] = true;

        for assignment in &starts[id.index()] {
            context.role = Some(assignment.role);
            context.in_if_condition |= assignment.in_if_condition;
            context.in_elif_condition |= assignment.in_elif_condition;
        }
        contexts[id.index()] = context;

        for child in child_ids[id.index()].iter().rev().copied() {
            stack.push((child, context));
        }
    }
}

fn build_syntax_backed_command_parent_ids(
    model: &SemanticModel,
    syntax_backed_ids: &[CommandId],
    parent_ids: &[Option<CommandId>],
) -> Vec<Option<CommandId>> {
    let mut syntax_backed_parent_ids = vec![None; parent_ids.len()];
    for id in syntax_backed_ids.iter().copied() {
        let mut current = parent_ids[id.index()];
        while let Some(parent) = current {
            if model.command_syntax_kind(parent).is_some() {
                syntax_backed_parent_ids[id.index()] = Some(parent);
                break;
            }
            current = parent_ids[parent.index()];
        }
    }
    syntax_backed_parent_ids
}

fn build_command_child_ids(
    command_count: usize,
    command_ids: &[CommandId],
    parent_ids: &[Option<CommandId>],
) -> Vec<Vec<CommandId>> {
    let mut child_ids = vec![Vec::new(); command_count];
    for child in command_ids.iter().copied() {
        if let Some(parent) = parent_ids[child.index()] {
            child_ids[parent.index()].push(child);
        }
    }
    child_ids
}

fn build_command_containing_offset_entries(
    model: &SemanticModel,
    syntax_backed_ids: &[CommandId],
) -> Vec<CommandContainingOffsetEntry> {
    let mut events = syntax_backed_ids
        .iter()
        .copied()
        .flat_map(|id| {
            let span = model.command_span(id);
            [
                CommandContainingOffsetEvent {
                    offset: span.start.offset,
                    end_offset: span.end.offset,
                    id,
                    kind: CommandContainingOffsetEventKind::Start,
                },
                CommandContainingOffsetEvent {
                    offset: span.end.offset.saturating_add(1),
                    end_offset: span.end.offset,
                    id,
                    kind: CommandContainingOffsetEventKind::End,
                },
            ]
        })
        .collect::<Vec<_>>();
    events.sort_unstable_by(|left, right| {
        left.offset
            .cmp(&right.offset)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| right.end_offset.cmp(&left.end_offset))
            .then_with(|| left.id.index().cmp(&right.id.index()))
    });

    let mut entries = Vec::new();
    let mut active = Vec::<CommandId>::new();
    let mut index = 0;
    while let Some(event) = events.get(index).copied() {
        let offset = event.offset;
        while events.get(index).is_some_and(|event| {
            event.offset == offset && event.kind == CommandContainingOffsetEventKind::End
        }) {
            let id = events[index].id;
            active.retain(|active_id| *active_id != id);
            index += 1;
        }
        while events.get(index).is_some_and(|event| {
            event.offset == offset && event.kind == CommandContainingOffsetEventKind::Start
        }) {
            active.push(events[index].id);
            index += 1;
        }

        let Some(next_offset) = events.get(index).map(|event| event.offset) else {
            break;
        };
        if offset < next_offset
            && let Some(id) = active.last().copied()
        {
            push_command_containing_offset_entry(&mut entries, offset, next_offset - 1, id);
        }
    }

    entries
}

fn push_command_containing_offset_entry(
    entries: &mut Vec<CommandContainingOffsetEntry>,
    start_offset: usize,
    end_offset: usize,
    id: CommandId,
) {
    if let Some(last) = entries.last_mut()
        && last.id == id
        && last.end_offset.saturating_add(1) == start_offset
    {
        last.end_offset = end_offset;
        return;
    }

    entries.push(CommandContainingOffsetEntry {
        start_offset,
        end_offset,
        id,
    });
}

fn attach_containing_command_parents(
    model: &SemanticModel,
    command_ids: &[CommandId],
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    let mut sorted = command_ids.to_vec();
    sorted.sort_unstable_by(|left, right| compare_command_ids_by_syntax_span(model, *left, *right));

    let mut stack = Vec::<CommandId>::new();
    for child in sorted {
        let child_span = model.command_syntax_span(child);
        while stack.last().is_some_and(|candidate| {
            !contains_command_span(model.command_syntax_span(*candidate), child_span)
        }) {
            stack.pop();
        }

        if parent_ids[child.index()].is_none()
            && let Some(parent) = stack.iter().rev().copied().find(|candidate| {
                *candidate != child
                    && contains_command_span(model.command_syntax_span(*candidate), child_span)
                    && !would_create_command_parent_cycle(*candidate, child, parent_ids)
            })
        {
            assign_command_parent(parent, child, parent_ids, child_ids);
        }
        stack.push(child);
    }
}

fn record_command_children(
    program: &RecordedProgram,
    parent: CommandId,
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
    nested_region_command_ids: &mut FxHashSet<CommandId>,
    nested_region_root_command_ids: &mut FxHashSet<CommandId>,
) {
    let command = program.command(parent);
    for region in program.nested_regions(command.nested_regions) {
        for child in commands_in_range_recursive(program, region.commands) {
            nested_region_command_ids.insert(child);
        }
        for child in program.commands_in(region.commands).iter().copied() {
            nested_region_root_command_ids.insert(child);
            assign_command_parent(parent, child, parent_ids, child_ids);
        }
    }

    match command.kind {
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit => {}
        RecordedCommandKind::List { first, rest } => {
            assign_command_parent(parent, first, parent_ids, child_ids);
            for item in program.list_items(rest) {
                assign_command_parent(parent, item.command, parent_ids, child_ids);
            }
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            assign_range_parent(program, parent, condition, parent_ids, child_ids);
            assign_range_parent(program, parent, then_branch, parent_ids, child_ids);
            for branch in program.elif_branches(elif_branches) {
                assign_range_parent(program, parent, branch.condition, parent_ids, child_ids);
                assign_range_parent(program, parent, branch.body, parent_ids, child_ids);
            }
            assign_range_parent(program, parent, else_branch, parent_ids, child_ids);
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            assign_range_parent(program, parent, condition, parent_ids, child_ids);
            assign_range_parent(program, parent, body, parent_ids, child_ids);
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body }
        | RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::Subshell { body } => {
            assign_range_parent(program, parent, body, parent_ids, child_ids);
        }
        RecordedCommandKind::Case { arms } => {
            for arm in program.case_arms(arms) {
                assign_range_parent(program, parent, arm.commands, parent_ids, child_ids);
            }
        }
        RecordedCommandKind::Pipeline { segments } => {
            for segment in program.pipeline_segments(segments) {
                assign_command_parent(parent, segment.command, parent_ids, child_ids);
            }
        }
    }
}

fn assign_range_parent(
    program: &RecordedProgram,
    parent: CommandId,
    range: crate::cfg::RecordedCommandRange,
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    for child in program.commands_in(range).iter().copied() {
        assign_command_parent(parent, child, parent_ids, child_ids);
    }
}

fn assign_command_parent(
    parent: CommandId,
    child: CommandId,
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    if parent != child
        && parent_ids[child.index()].is_none()
        && !would_create_command_parent_cycle(parent, child, parent_ids)
    {
        parent_ids[child.index()] = Some(parent);
        child_ids[parent.index()].push(child);
    }
}

fn would_create_command_parent_cycle(
    parent: CommandId,
    child: CommandId,
    parent_ids: &[Option<CommandId>],
) -> bool {
    let mut current = Some(parent);
    while let Some(id) = current {
        if id == child {
            return true;
        }
        current = parent_ids[id.index()];
    }
    false
}

fn build_nested_region_depths(
    command_count: usize,
    parent_ids: &[Option<CommandId>],
    child_ids: &[Vec<CommandId>],
    nested_region_root_command_ids: &FxHashSet<CommandId>,
) -> Vec<usize> {
    let mut depths = vec![0; command_count];
    let mut stack = Vec::new();
    for index in (0..command_count).rev() {
        let id = CommandId(index as u32);
        if parent_ids[id.index()].is_none() {
            stack.push((id, 0));
        }
    }
    while let Some((id, depth)) = stack.pop() {
        depths[id.index()] = depth;
        for child in child_ids[id.index()].iter().rev().copied() {
            let child_depth = if nested_region_root_command_ids.contains(&child) {
                depth + 1
            } else {
                depth
            };
            stack.push((child, child_depth));
        }
    }
    depths
}

fn commands_in_range_recursive(
    program: &RecordedProgram,
    range: crate::cfg::RecordedCommandRange,
) -> Vec<CommandId> {
    let mut commands = Vec::new();
    for command in program.commands_in(range).iter().copied() {
        commands.push(command);
        commands.extend(command_descendants(program, command));
    }
    commands
}

fn command_descendants(program: &RecordedProgram, command: CommandId) -> Vec<CommandId> {
    let mut descendants = Vec::new();
    let command = program.command(command);
    for region in program.nested_regions(command.nested_regions) {
        descendants.extend(commands_in_range_recursive(program, region.commands));
    }
    match command.kind {
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit => {}
        RecordedCommandKind::List { first, rest } => {
            descendants.push(first);
            descendants.extend(command_descendants(program, first));
            for item in program.list_items(rest) {
                descendants.push(item.command);
                descendants.extend(command_descendants(program, item.command));
            }
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            descendants.extend(commands_in_range_recursive(program, condition));
            descendants.extend(commands_in_range_recursive(program, then_branch));
            for branch in program.elif_branches(elif_branches) {
                descendants.extend(commands_in_range_recursive(program, branch.condition));
                descendants.extend(commands_in_range_recursive(program, branch.body));
            }
            descendants.extend(commands_in_range_recursive(program, else_branch));
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            descendants.extend(commands_in_range_recursive(program, condition));
            descendants.extend(commands_in_range_recursive(program, body));
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body }
        | RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::Subshell { body } => {
            descendants.extend(commands_in_range_recursive(program, body));
        }
        RecordedCommandKind::Case { arms } => {
            for arm in program.case_arms(arms) {
                descendants.extend(commands_in_range_recursive(program, arm.commands));
            }
        }
        RecordedCommandKind::Pipeline { segments } => {
            for segment in program.pipeline_segments(segments) {
                descendants.push(segment.command);
                descendants.extend(command_descendants(program, segment.command));
            }
        }
    }
    descendants
}

fn attach_function_body_commands(
    model: &SemanticModel,
    command_ids: &[CommandId],
    parent_ids: &mut [Option<CommandId>],
    child_ids: &mut [Vec<CommandId>],
) {
    for body in model.recorded_program.function_bodies().values().copied() {
        for child in model.recorded_program.commands_in(body).iter().copied() {
            if parent_ids[child.index()].is_some() {
                continue;
            }
            let child_span = model.command_syntax_span(child);
            let Some(parent) = command_ids
                .iter()
                .copied()
                .filter(|candidate| {
                    model.command_syntax_kind(*candidate) == Some(CommandKind::Function)
                        && contains_command_span(model.command_syntax_span(*candidate), child_span)
                })
                .min_by_key(|candidate| {
                    let span = model.command_syntax_span(*candidate);
                    (span.end.offset - span.start.offset, candidate.index())
                })
            else {
                continue;
            };
            assign_command_parent(parent, child, parent_ids, child_ids);
        }
    }
}

fn contains_command_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn compare_command_ids_by_syntax_span(
    model: &SemanticModel,
    left: CommandId,
    right: CommandId,
) -> std::cmp::Ordering {
    let left_span = model.command_syntax_span(left);
    let right_span = model.command_syntax_span(right);
    left_span
        .start
        .offset
        .cmp(&right_span.start.offset)
        .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
        .then_with(|| right.index().cmp(&left.index()))
}

#[doc(hidden)]
pub fn build_with_observer<'a>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    observer: &mut dyn TraversalObserver<'a>,
) -> SemanticModel {
    build_with_observer_with_options(
        file,
        source,
        indexer,
        observer,
        SemanticBuildOptions::default(),
    )
}

#[doc(hidden)]
pub fn build_with_observer_with_options<'a>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    observer: &mut dyn TraversalObserver<'a>,
    options: SemanticBuildOptions<'_>,
) -> SemanticModel {
    build_semantic_model(file, source, indexer, observer, options)
}

#[doc(hidden)]
pub fn build_with_observer_at_path<'a>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    observer: &mut dyn TraversalObserver<'a>,
    source_path: Option<&Path>,
) -> SemanticModel {
    build_with_observer_at_path_with_resolver(file, source, indexer, observer, source_path, None)
}

#[doc(hidden)]
pub fn build_with_observer_at_path_with_resolver<'a>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    observer: &mut dyn TraversalObserver<'a>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> SemanticModel {
    build_semantic_model(
        file,
        source,
        indexer,
        observer,
        SemanticBuildOptions {
            source_path,
            source_path_resolver,
            file_entry_contract: None,
            file_entry_contract_collector: None,
            analyzed_paths: None,
            shell_profile: None,
            resolve_source_closure: true,
        },
    )
}

fn build_semantic_model<'a>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    observer: &mut dyn TraversalObserver<'a>,
    options: SemanticBuildOptions<'_>,
) -> SemanticModel {
    let SemanticBuildOptions {
        source_path,
        source_path_resolver,
        file_entry_contract,
        mut file_entry_contract_collector,
        analyzed_paths,
        shell_profile,
        resolve_source_closure,
    } = options;
    let mut model = build_semantic_model_base(
        file,
        source,
        indexer,
        observer,
        source_path,
        shell_profile.clone(),
        file_entry_contract_collector
            .as_mut()
            .map(|collector| &mut **collector as &mut dyn FileEntryContractCollector),
    );
    if let Some(contract) = file_entry_contract {
        model.apply_file_entry_contract(contract, file);
    }
    if let Some(contract) = file_entry_contract_collector
        .as_ref()
        .and_then(|collector| collector.finish())
    {
        model.apply_file_entry_contract(contract, file);
    }
    if let Some(source_path) = source_path {
        let (
            synthetic_reads,
            imported_bindings,
            source_ref_resolutions,
            source_ref_explicitness,
            source_ref_diagnostic_classes,
        ) = if resolve_source_closure {
            source_closure::collect_source_closure_contracts(
                &model,
                file,
                source,
                source_path,
                source_path_resolver,
                analyzed_paths,
            )
        } else {
            let (source_ref_resolutions, source_ref_explicitness, source_ref_diagnostic_classes) =
                source_closure::collect_source_ref_metadata(
                    &model,
                    source_path,
                    source_path_resolver,
                    analyzed_paths,
                );
            (
                Vec::new(),
                Vec::new(),
                source_ref_resolutions,
                source_ref_explicitness,
                source_ref_diagnostic_classes,
            )
        };
        model.apply_source_contracts(
            synthetic_reads,
            imported_bindings,
            source_ref_resolutions,
            source_ref_explicitness,
            source_ref_diagnostic_classes,
        );
    }
    model
}

pub(crate) fn build_semantic_model_base<'a, 'observer>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    observer: &'observer mut dyn TraversalObserver<'a>,
    source_path: Option<&Path>,
    shell_profile: Option<ShellProfile>,
    file_entry_contract_collector: Option<&'observer mut dyn FileEntryContractCollector>,
) -> SemanticModel {
    let shell_profile = shell_profile.unwrap_or_else(|| infer_shell_profile(source, source_path));
    let built = SemanticModelBuilder::build(
        file,
        source,
        indexer,
        observer,
        file_entry_contract_collector,
        bash_runtime_vars_enabled(source, source_path),
        shell_profile,
    );
    SemanticModel::from_build_output(built)
}

fn infer_shell_profile(source: &str, path: Option<&Path>) -> ShellProfile {
    let dialect = infer_parse_dialect_from_source(source, path);
    ShellProfile::native(dialect)
}

fn infer_parse_dialect_from_source(
    source: &str,
    path: Option<&Path>,
) -> shuck_parser::ShellDialect {
    if let Some(interpreter) = shebang_interpreter(source) {
        return parse_dialect_from_name(interpreter).unwrap_or(shuck_parser::ShellDialect::Bash);
    }

    infer_parse_dialect_from_path(path).unwrap_or(shuck_parser::ShellDialect::Bash)
}

pub(crate) fn infer_explicit_parse_dialect_from_source(
    source: &str,
    path: Option<&Path>,
) -> Option<shuck_parser::ShellDialect> {
    if let Some(interpreter) = shebang_interpreter(source)
        && let Some(dialect) = parse_dialect_from_name(interpreter)
    {
        return Some(dialect);
    }

    infer_parse_dialect_from_path(path)
}

fn shebang_interpreter(source: &str) -> Option<&str> {
    shuck_parser::shebang::interpreter_name(source.lines().next()?)
}

fn infer_parse_dialect_from_path(path: Option<&Path>) -> Option<shuck_parser::ShellDialect> {
    match path
        .and_then(|path| path.extension().and_then(|ext| ext.to_str()))
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("sh" | "dash" | "ksh") => Some(shuck_parser::ShellDialect::Posix),
        Some("mksh") => Some(shuck_parser::ShellDialect::Mksh),
        Some("bash") => Some(shuck_parser::ShellDialect::Bash),
        Some("zsh") => Some(shuck_parser::ShellDialect::Zsh),
        _ => None,
    }
}

fn parse_dialect_from_name(name: &str) -> Option<shuck_parser::ShellDialect> {
    match name.to_ascii_lowercase().as_str() {
        "sh" | "dash" | "ksh" | "posix" => Some(shuck_parser::ShellDialect::Posix),
        "mksh" => Some(shuck_parser::ShellDialect::Mksh),
        "bash" => Some(shuck_parser::ShellDialect::Bash),
        "zsh" => Some(shuck_parser::ShellDialect::Zsh),
        _ => None,
    }
}

fn bash_runtime_vars_enabled(source: &str, path: Option<&Path>) -> bool {
    infer_bash_from_shebang(source).unwrap_or_else(|| {
        path.and_then(|path| path.extension().and_then(|ext| ext.to_str()))
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bash"))
    })
}

fn infer_bash_from_shebang(source: &str) -> Option<bool> {
    shebang_interpreter(source).map(|interpreter| interpreter.eq_ignore_ascii_case("bash"))
}

fn contains_offset(span: Span, offset: usize) -> bool {
    span.start.offset <= offset && offset <= span.end.offset
}

fn build_references_sorted_by_start(references: &[Reference]) -> Vec<ReferenceId> {
    let mut ids: Vec<ReferenceId> = (0..references.len() as u32).map(ReferenceId).collect();
    ids.sort_by_key(|id| references[id.index()].span.start.offset);
    ids
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionalParameterReferenceKind {
    Indexed(usize),
    Special,
}

fn positional_parameter_reference_kind(name: &str) -> Option<PositionalParameterReferenceKind> {
    match name {
        "@" | "*" | "#" => Some(PositionalParameterReferenceKind::Special),
        "0" => None,
        _ if name.chars().all(|ch| ch.is_ascii_digit()) => name
            .parse::<usize>()
            .ok()
            .map(PositionalParameterReferenceKind::Indexed),
        _ => None,
    }
}

fn reference_has_local_positional_reset(
    semantic: &SemanticModel,
    scope: ScopeId,
    offset: usize,
    local_reset_offsets_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
) -> bool {
    semantic
        .transient_ancestor_scopes_within_function(scope)
        .any(|transient_scope| {
            local_reset_offsets_by_scope
                .get(&transient_scope)
                .is_some_and(|offsets| offsets.iter().any(|reset_offset| *reset_offset < offset))
        })
}

fn build_bindings_sorted_by_start(bindings: &[Binding]) -> Vec<BindingId> {
    let mut ids: Vec<BindingId> = (0..bindings.len() as u32).map(BindingId).collect();
    ids.sort_by_key(|id| bindings[id.index()].span.start.offset);
    ids
}

fn build_guarded_or_defaulting_reference_offsets_by_name(
    references: &[Reference],
    guarded_parameter_refs: &FxHashSet<ReferenceId>,
    defaulting_parameter_operand_refs: &FxHashSet<ReferenceId>,
) -> FxHashMap<Name, Box<[usize]>> {
    let mut offsets_by_name = FxHashMap::<Name, Vec<usize>>::default();

    for reference in references {
        if guarded_parameter_refs.contains(&reference.id)
            || defaulting_parameter_operand_refs.contains(&reference.id)
        {
            offsets_by_name
                .entry(reference.name.clone())
                .or_default()
                .push(reference.span.start.offset);
        }
    }

    offsets_by_name
        .into_iter()
        .map(|(name, mut offsets)| {
            offsets.sort_unstable();
            offsets.dedup();
            (name, offsets.into_boxed_slice())
        })
        .collect()
}

fn build_declarations_by_command_span(declarations: &[Declaration]) -> FxHashMap<SpanKey, usize> {
    let mut index = FxHashMap::with_capacity_and_hasher(declarations.len(), Default::default());
    for (declaration_index, declaration) in declarations.iter().enumerate() {
        index.insert(SpanKey::new(declaration.span), declaration_index);
    }
    index
}

fn build_bindings_by_definition_span(bindings: &[Binding]) -> FxHashMap<SpanKey, BindingId> {
    let mut index = FxHashMap::with_capacity_and_hasher(bindings.len(), Default::default());
    for binding in bindings {
        index.insert(SpanKey::new(binding.span), binding.id);
    }
    index
}

/// Iterator returned by [`SemanticModel::references_in_span`].
///
/// Walks the references sorted index forward from the first candidate and
/// stops as soon as a reference starts past the outer span's end.
#[derive(Debug, Clone)]
pub struct ReferencesInSpan<'a> {
    references: &'a [Reference],
    ids: std::slice::Iter<'a, ReferenceId>,
    end: usize,
}

impl<'a> Iterator for ReferencesInSpan<'a> {
    type Item = &'a Reference;

    fn next(&mut self) -> Option<&'a Reference> {
        loop {
            let id = self.ids.next()?;
            let reference = &self.references[id.index()];
            if reference.span.start.offset > self.end {
                return None;
            }
            if reference.span.end.offset <= self.end {
                return Some(reference);
            }
        }
    }
}

/// Iterator returned by [`SemanticModel::references_in_command_span`].
///
/// Walks the direct reference ids recorded for a single command and yields
/// only those fully contained within the requested subspan.
#[derive(Debug, Clone)]
pub struct CommandReferencesInSpan<'a> {
    references: &'a [Reference],
    ids: std::slice::Iter<'a, ReferenceId>,
    outer: Span,
}

impl<'a> Iterator for CommandReferencesInSpan<'a> {
    type Item = &'a Reference;

    fn next(&mut self) -> Option<&'a Reference> {
        loop {
            let id = self.ids.next()?;
            let reference = &self.references[id.index()];
            if contains_span(self.outer, reference.span) {
                return Some(reference);
            }
        }
    }
}

/// Iterator returned by [`SemanticModel::bindings_in_span`].
///
/// Walks the bindings sorted index forward from the first candidate and
/// stops as soon as a binding starts past the outer span's end.
#[derive(Debug, Clone)]
pub struct BindingsInSpan<'a> {
    bindings: &'a [Binding],
    ids: std::slice::Iter<'a, BindingId>,
    end: usize,
}

impl<'a> Iterator for BindingsInSpan<'a> {
    type Item = &'a Binding;

    fn next(&mut self) -> Option<&'a Binding> {
        loop {
            let id = self.ids.next()?;
            let binding = &self.bindings[id.index()];
            if binding.span.start.offset > self.end {
                return None;
            }
            if binding.span.end.offset <= self.end {
                return Some(binding);
            }
        }
    }
}

fn scope_span_width(span: Span) -> usize {
    span.end.offset.saturating_sub(span.start.offset)
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

#[cfg(test)]
fn linear_scope_at(scopes: &[Scope], offset: usize) -> ScopeId {
    scopes
        .iter()
        .filter(|scope| contains_offset(scope.span, offset))
        .min_by_key(|scope| scope_span_width(scope.span))
        .map(|scope| scope.id)
        .unwrap_or(ScopeId(0))
}

fn build_indirect_targets_by_binding(
    bindings: &[Binding],
    indirect_target_hints: &FxHashMap<BindingId, IndirectTargetHint>,
) -> FxHashMap<BindingId, Vec<BindingId>> {
    let mut targets_by_binding = FxHashMap::default();
    for (binding_id, hint) in indirect_target_hints {
        let targets: Vec<_> = bindings
            .iter()
            .filter(|binding| indirect_target_matches(hint, binding))
            .map(|binding| binding.id)
            .collect();
        if !targets.is_empty() {
            targets_by_binding.insert(*binding_id, targets);
        }
    }
    targets_by_binding
}

fn build_indirect_targets_by_reference(
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_expansion_refs: &FxHashSet<ReferenceId>,
    indirect_targets_by_binding: &FxHashMap<BindingId, Vec<BindingId>>,
) -> FxHashMap<ReferenceId, Vec<BindingId>> {
    let mut targets_by_reference = FxHashMap::default();
    for reference in references {
        if !indirect_expansion_refs.contains(&reference.id) {
            continue;
        }
        let Some(binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        if let Some(targets) = indirect_targets_by_binding.get(&binding_id) {
            targets_by_reference.insert(reference.id, targets.clone());
        }
    }
    targets_by_reference
}

fn build_array_like_indirect_expansion_refs(
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_expansion_refs: &FxHashSet<ReferenceId>,
    indirect_target_hints: &FxHashMap<BindingId, IndirectTargetHint>,
) -> FxHashSet<ReferenceId> {
    let mut array_like_refs = FxHashSet::default();
    for reference in references {
        if !indirect_expansion_refs.contains(&reference.id) {
            continue;
        }
        let Some(binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        let Some(hint) = indirect_target_hints.get(&binding_id) else {
            continue;
        };
        let array_like = match hint {
            IndirectTargetHint::Exact { array_like, .. }
            | IndirectTargetHint::Pattern { array_like, .. } => *array_like,
        };
        if array_like {
            array_like_refs.insert(reference.id);
        }
    }
    array_like_refs
}

fn indirect_target_matches(hint: &IndirectTargetHint, binding: &Binding) -> bool {
    match hint {
        IndirectTargetHint::Exact { name, array_like } => {
            binding.name == *name && (!array_like || binding::is_array_like_binding(binding))
        }
        IndirectTargetHint::Pattern {
            prefix,
            suffix,
            array_like,
        } => {
            let name = binding.name.as_str();
            name.starts_with(prefix.as_str())
                && name.ends_with(suffix.as_str())
                && (!array_like || binding::is_array_like_binding(binding))
        }
    }
}

#[cfg(test)]
mod tests;
