//! Editor-facing semantic query shapes.

use std::collections::BTreeSet;
use std::ops::Deref;

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span, TextSize};
use shuck_indexer::{Indexer, RegionKind};

use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, BindingOrigin, DeclarationOperand,
    Reference, ReferenceId, ReferenceKind, ScopeId, ScopeKind, SemanticModel, SpanKey,
};

/// LSP-agnostic query object for editor features.
pub struct EditorQuery<'model> {
    model: &'model SemanticModel,
}

/// One semantic symbol suitable for editor presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSymbol {
    /// User-visible symbol name.
    pub name: Name,
    /// Editor-facing symbol category.
    pub kind: EditorSymbolKind,
    /// Span that introduces the symbol.
    pub definition_span: Span,
    /// Span to select when navigating to the symbol.
    pub selection_span: Span,
    /// Semantic scope that owns the symbol.
    pub scope: ScopeId,
    /// Underlying semantic binding, when the symbol has one.
    pub binding: Option<BindingId>,
}

/// Hierarchical document-symbol item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorDocumentSymbol {
    /// Reusable semantic symbol payload.
    pub symbol: EditorSymbol,
    /// Span covering the item in the source document.
    pub range: Span,
    /// Child symbols nested inside this symbol.
    pub children: Vec<EditorDocumentSymbol>,
}

/// Semantic hover information for one source symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorHover {
    /// Reusable semantic symbol payload.
    pub symbol: EditorSymbol,
    /// Span that should be highlighted by the hover response.
    pub target_span: Span,
    /// Attributes attached to the underlying binding, if any.
    pub attributes: BindingAttributes,
    /// Whether the symbol came from an imported contract or sourced file.
    pub imported: bool,
    /// Whether the symbol is provided by the active shell runtime.
    pub runtime: bool,
    /// Number of file-local call sites resolved to this function, when applicable.
    pub function_call_count: Option<usize>,
}

/// A function call token that editor features can resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorFunctionCallTarget {
    /// Callee name as written at the call site.
    pub name: Name,
    /// Span of the callee token.
    pub name_span: Span,
    /// Resolved function binding when the call is statically proven.
    pub binding: Option<BindingId>,
}

/// A runtime-provided shell name target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRuntimeNameTarget {
    /// Runtime name.
    pub name: Name,
    /// Span of the runtime-name token.
    pub span: Span,
}

/// Semantic target under an editor cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorSymbolTarget {
    /// A binding definition or write site.
    Binding(BindingId),
    /// A reference to a binding or runtime name.
    Reference(ReferenceId),
    /// A command-position function call.
    FunctionCall(EditorFunctionCallTarget),
    /// A shell runtime name with no source binding.
    RuntimeName(EditorRuntimeNameTarget),
}

/// Read/write classification for an editor occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOccurrenceKind {
    /// A read-like occurrence.
    Read,
    /// A definition, assignment, declaration, or other write-like occurrence.
    Write,
}

/// One source occurrence of an editor symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorOccurrence {
    /// Source span of the occurrence.
    pub span: Span,
    /// Read/write classification.
    pub kind: EditorOccurrenceKind,
}

/// Completion category for editor presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCompletionKind {
    /// A shell variable or parameter.
    Variable,
    /// A shell function.
    Function,
    /// A shell builtin command modeled by Shuck.
    Builtin,
    /// A runtime-provided shell name.
    RuntimeName,
    /// A shell keyword or reserved word.
    Keyword,
}

/// One semantic completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCompletion {
    /// Text shown and inserted for the completion.
    pub name: Name,
    /// Completion category.
    pub kind: EditorCompletionKind,
    /// Optional source definition span for symbol completions.
    pub definition_span: Option<Span>,
}

/// Completion candidates plus the source span they replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCompletions {
    /// Span replaced by each completion item.
    pub replacement_span: Span,
    /// Candidate items.
    pub items: Vec<EditorCompletion>,
}

/// Editor completion feature switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorCompletionOptions {
    /// Include shell runtime-provided variables in parameter completions.
    pub include_runtime_names: bool,
    /// Include shell keywords in command-position completions.
    pub include_keywords: bool,
}

impl Default for EditorCompletionOptions {
    fn default() -> Self {
        Self {
            include_runtime_names: true,
            include_keywords: true,
        }
    }
}

/// Proven same-file rename group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameSet {
    /// Original symbol name.
    pub name: Name,
    /// Symbol kind being renamed.
    pub kind: EditorSymbolKind,
    /// Range that `prepareRename` should offer to the editor.
    pub editable_span: Span,
    /// Source spans to edit.
    pub spans: Vec<Span>,
}

/// Reason a cursor target cannot be renamed safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameUnavailable {
    /// No renameable target exists at the cursor.
    NoTarget,
    /// The target name is dynamic or otherwise not a plain static token.
    DynamicName,
    /// The target is an indirect reference.
    IndirectReference,
    /// The target is a nameref.
    Nameref,
    /// The target was imported from outside the current document.
    ImportedBinding,
    /// The target cannot be resolved to one proven symbol set.
    AmbiguousResolution,
    /// The original target name is not a shell identifier/function token Shuck can rename.
    InvalidIdentifier,
    /// The rename would need edits outside the current indexed document.
    CrossFileUnindexed,
}

impl Deref for EditorDocumentSymbol {
    type Target = EditorSymbol;

    fn deref(&self) -> &Self::Target {
        &self.symbol
    }
}

/// Coarse editor-facing symbol kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSymbolKind {
    /// A shell function definition.
    Function,
    /// A scalar-like shell variable.
    Variable,
    /// An array-like shell variable.
    Array,
    /// An associative-array-like shell variable.
    AssociativeArray,
    /// A declaration operand such as `local name` or `declare name`.
    Declaration,
    /// A runtime-provided name.
    RuntimeName,
}

#[derive(Debug, Clone)]
struct PendingDocumentSymbol {
    symbol: EditorDocumentSymbol,
    parent: Option<usize>,
    sort_offset: usize,
}

type DocumentSymbolRangesByBinding = Vec<Option<Span>>;
type DeclarationOperandRanges = FxHashMap<SpanKey, Span>;
pub(crate) type EditorHoverTargets = Vec<EditorHoverTarget>;

#[derive(Debug, Clone)]
pub(crate) struct EditorHoverTarget {
    span: Span,
    kind: EditorHoverTargetKind,
}

#[derive(Debug, Clone)]
enum EditorHoverTargetKind {
    Binding(BindingId),
    Reference(ReferenceId),
    FunctionCall { name: Name, name_span: Span },
}

impl SemanticModel {
    /// Returns an editor-facing query object over this semantic model.
    pub fn editor_query(&self) -> EditorQuery<'_> {
        EditorQuery { model: self }
    }
}

impl<'model> EditorQuery<'model> {
    /// Creates an editor query over `model`.
    pub fn new(model: &'model SemanticModel) -> Self {
        Self { model }
    }

    /// Builds a hierarchical document-symbol tree for the analyzed document.
    pub fn document_symbols(&self) -> Vec<EditorDocumentSymbol> {
        let analysis = self.model.analysis();
        let mut pending = Vec::new();
        let mut function_symbols_by_scope = FxHashMap::default();
        let document_symbol_ranges_by_binding = self
            .model
            .editor_document_symbol_ranges_by_binding
            .get_or_init(|| document_symbol_ranges_by_binding(self.model));

        for binding in self.model.function_definition_bindings() {
            let Some(symbol) = document_symbol_for_function(binding) else {
                continue;
            };
            let index = pending.len();
            let function_scope = analysis.function_scope_for_binding(binding.id);
            pending.push(PendingDocumentSymbol {
                sort_offset: binding.span.start.offset,
                symbol,
                parent: None,
            });
            if let Some(function_scope) = function_scope {
                function_symbols_by_scope
                    .entry(function_scope)
                    .or_insert(index);
            }
        }

        let function_count = pending.len();
        for symbol in pending.iter_mut().take(function_count) {
            let scope = symbol.symbol.symbol.scope;
            symbol.parent =
                enclosing_function_symbol(self.model, scope, &function_symbols_by_scope);
        }

        for binding in self.model.bindings() {
            if matches!(binding.kind, BindingKind::FunctionDefinition) {
                continue;
            }
            let Some(parent) =
                document_symbol_parent_for_binding(self.model, binding, &function_symbols_by_scope)
            else {
                continue;
            };
            let Some(symbol) =
                document_symbol_for_binding(binding, document_symbol_ranges_by_binding)
            else {
                continue;
            };
            pending.push(PendingDocumentSymbol {
                sort_offset: binding.span.start.offset,
                symbol,
                parent,
            });
        }

        build_document_symbol_tree(pending)
    }

    /// Returns semantic hover information for the symbol at `offset`.
    pub fn hover_at_offset(&self, offset: usize) -> Option<EditorHover> {
        find_editor_target(self.model, offset)
            .and_then(|target| hover_for_target(self.model, target))
    }

    /// Returns the semantic target under `offset`.
    pub fn target_at_offset(&self, offset: usize) -> Option<EditorSymbolTarget> {
        find_editor_target(self.model, offset)
            .and_then(|target| target_to_editor_symbol(self.model, target))
    }

    /// Returns source spans that define the symbol under `offset`.
    pub fn definition_spans_at_offset(&self, offset: usize) -> Vec<Span> {
        let Some(target) = self.target_at_offset(offset) else {
            return Vec::new();
        };
        self.definition_spans_for_target(&target)
    }

    /// Returns source spans that define `target`.
    pub fn definition_spans_for_target(&self, target: &EditorSymbolTarget) -> Vec<Span> {
        let mut spans = match target {
            EditorSymbolTarget::Binding(binding_id) => {
                definition_spans_for_binding_family(self.model, *binding_id)
            }
            EditorSymbolTarget::Reference(reference_id) => self
                .model
                .resolved_binding(*reference_id)
                .map(|binding| definition_spans_for_binding_family(self.model, binding.id))
                .unwrap_or_default(),
            EditorSymbolTarget::FunctionCall(call) => call
                .binding
                .map(|binding_id| vec![binding_definition_span(self.model.binding(binding_id))])
                .unwrap_or_default(),
            EditorSymbolTarget::RuntimeName(_) => Vec::new(),
        };
        sort_dedup_spans(&mut spans);
        spans
    }

    /// Returns read/write occurrences for the symbol under `offset`.
    pub fn occurrences_at_offset(
        &self,
        offset: usize,
        include_declaration: bool,
    ) -> Vec<EditorOccurrence> {
        let Some(target) = self.target_at_offset(offset) else {
            return Vec::new();
        };
        self.occurrences_for_target(&target, include_declaration)
    }

    /// Returns read/write occurrences for `target`.
    pub fn occurrences_for_target(
        &self,
        target: &EditorSymbolTarget,
        include_declaration: bool,
    ) -> Vec<EditorOccurrence> {
        match target {
            EditorSymbolTarget::Binding(binding_id) => {
                occurrences_for_binding_family(self.model, *binding_id, include_declaration)
            }
            EditorSymbolTarget::Reference(reference_id) => {
                let Some(binding) = self.model.resolved_binding(*reference_id) else {
                    return Vec::new();
                };
                occurrences_for_binding_family(self.model, binding.id, include_declaration)
            }
            EditorSymbolTarget::FunctionCall(call) => call
                .binding
                .map(|binding_id| {
                    occurrences_for_function(self.model, binding_id, include_declaration)
                })
                .unwrap_or_default(),
            EditorSymbolTarget::RuntimeName(_) => Vec::new(),
        }
    }

    /// Returns syntax-aware completion candidates at `offset`.
    pub fn completions_at_offset(
        &self,
        source: &str,
        indexer: &Indexer,
        offset: usize,
        options: EditorCompletionOptions,
    ) -> Option<EditorCompletions> {
        completion_context(source, indexer, offset)
            .map(|context| completions_for_context(self.model, source, offset, context, options))
            .filter(|completions| !completions.items.is_empty())
    }

    /// Returns a conservative rename set for the target under `offset`.
    pub fn rename_set_at_offset(&self, offset: usize) -> Result<RenameSet, RenameUnavailable> {
        let target = self
            .target_at_offset(offset)
            .ok_or(RenameUnavailable::NoTarget)?;
        self.rename_set_for_target(&target)
    }

    /// Returns a conservative rename set for `target`.
    pub fn rename_set_for_target(
        &self,
        target: &EditorSymbolTarget,
    ) -> Result<RenameSet, RenameUnavailable> {
        rename_set_for_target(self.model, target)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct HoverTargetRank {
    priority: u8,
    width: usize,
}

fn span_contains_offset(span: Span, offset: usize) -> bool {
    span.start.offset <= offset && offset < span.end.offset
}

fn hover_target_rank(target: &EditorHoverTarget) -> HoverTargetRank {
    let priority = match target.kind {
        EditorHoverTargetKind::Binding(_) => 0,
        EditorHoverTargetKind::FunctionCall { .. } => 1,
        EditorHoverTargetKind::Reference(_) => 2,
    };
    HoverTargetRank {
        priority,
        width: target
            .span
            .end
            .offset
            .saturating_sub(target.span.start.offset),
    }
}

fn find_editor_target(model: &SemanticModel, offset: usize) -> Option<&EditorHoverTarget> {
    let targets = model
        .editor_hover_targets
        .get_or_init(|| build_editor_hover_targets(model));
    let upper = targets.partition_point(|target| target.span.start.offset <= offset);

    let mut best: Option<(&EditorHoverTarget, HoverTargetRank)> = None;
    for target in targets[..upper].iter().rev() {
        if target.span.end.offset <= offset {
            break;
        }
        if !span_contains_offset(target.span, offset) {
            continue;
        }
        let rank = hover_target_rank(target);
        if best.is_none_or(|(_, best_rank)| rank < best_rank) {
            best = Some((target, rank));
        }
    }

    best.map(|(target, _)| target)
}

fn target_to_editor_symbol(
    model: &SemanticModel,
    target: &EditorHoverTarget,
) -> Option<EditorSymbolTarget> {
    match &target.kind {
        EditorHoverTargetKind::Binding(binding_id) => {
            Some(EditorSymbolTarget::Binding(*binding_id))
        }
        EditorHoverTargetKind::Reference(reference_id) => {
            let reference = model.reference(*reference_id);
            if model.resolved_binding(*reference_id).is_some() {
                return Some(EditorSymbolTarget::Reference(*reference_id));
            }
            (model.predefined_runtime_refs.contains(reference_id)
                || model.name_is_predefined_runtime(reference.name.as_str()))
            .then(|| {
                EditorSymbolTarget::RuntimeName(EditorRuntimeNameTarget {
                    name: reference.name.clone(),
                    span: reference.name_span,
                })
            })
        }
        EditorHoverTargetKind::FunctionCall { name, name_span } => {
            let binding = model
                .analysis()
                .visible_function_binding_at_call(name, *name_span);
            Some(EditorSymbolTarget::FunctionCall(EditorFunctionCallTarget {
                name: name.clone(),
                name_span: *name_span,
                binding,
            }))
        }
    }
}

fn build_editor_hover_targets(model: &SemanticModel) -> EditorHoverTargets {
    let mut targets = Vec::new();
    for binding in model.bindings() {
        let span = binding_hover_span(binding);
        if hover_symbol_kind_for_binding(binding).is_some() && !span.to_range().is_empty() {
            targets.push(EditorHoverTarget {
                span,
                kind: EditorHoverTargetKind::Binding(binding.id),
            });
        }
    }
    for reference in model.references() {
        if !reference.name_span.to_range().is_empty() {
            targets.push(EditorHoverTarget {
                span: reference.name_span,
                kind: EditorHoverTargetKind::Reference(reference.id),
            });
        }
    }
    for (name, sites) in &model.call_sites {
        for site in sites {
            if !site.name_span.to_range().is_empty() {
                targets.push(EditorHoverTarget {
                    span: site.name_span,
                    kind: EditorHoverTargetKind::FunctionCall {
                        name: name.clone(),
                        name_span: site.name_span,
                    },
                });
            }
        }
    }
    targets.sort_by_key(|target| (target.span.start.offset, target.span.end.offset));
    targets
}

fn hover_for_target(model: &SemanticModel, target: &EditorHoverTarget) -> Option<EditorHover> {
    match &target.kind {
        EditorHoverTargetKind::Binding(binding_id) => {
            hover_for_binding(model, model.binding(*binding_id), target.span)
        }
        EditorHoverTargetKind::Reference(reference_id) => {
            let reference = model.reference(*reference_id);
            if let Some(binding) = model.resolved_binding(*reference_id) {
                return hover_for_binding(model, binding, reference.name_span);
            }
            if model.predefined_runtime_refs.contains(reference_id)
                || model.name_is_predefined_runtime(reference.name.as_str())
            {
                return Some(runtime_hover(
                    model,
                    reference.name.clone(),
                    reference.name_span,
                ));
            }
            None
        }
        EditorHoverTargetKind::FunctionCall { name, name_span } => {
            let binding_id = model
                .analysis()
                .visible_function_binding_at_call(name, *name_span)?;
            hover_for_binding(model, model.binding(binding_id), *name_span)
        }
    }
}

fn definition_spans_for_binding_family(model: &SemanticModel, binding_id: BindingId) -> Vec<Span> {
    let binding = model.binding(binding_id);
    if matches!(binding.kind, BindingKind::FunctionDefinition) {
        return vec![binding_definition_span(binding)];
    }

    storage_family_bindings(model, binding)
        .into_iter()
        .map(|binding_id| binding_definition_span(model.binding(binding_id)))
        .collect()
}

fn occurrences_for_binding_family(
    model: &SemanticModel,
    binding_id: BindingId,
    include_declaration: bool,
) -> Vec<EditorOccurrence> {
    let binding = model.binding(binding_id);
    if matches!(binding.kind, BindingKind::FunctionDefinition) {
        return occurrences_for_function(model, binding_id, include_declaration);
    }

    let family = storage_family_bindings(model, binding);
    let family_set = family.iter().copied().collect::<FxHashSet<_>>();
    let mut occurrences = Vec::new();

    if include_declaration {
        occurrences.extend(family.iter().map(|binding_id| EditorOccurrence {
            span: rename_span_for_binding(model.binding(*binding_id)),
            kind: EditorOccurrenceKind::Write,
        }));
    }

    for reference in model.references() {
        let Some(resolved) = model.resolved.get(&reference.id) else {
            continue;
        };
        if !family_set.contains(resolved) || !source_backed_reference(reference) {
            continue;
        }
        let kind = if matches!(reference.kind, ReferenceKind::DeclarationName) {
            if !include_declaration {
                continue;
            }
            EditorOccurrenceKind::Write
        } else {
            EditorOccurrenceKind::Read
        };
        occurrences.push(EditorOccurrence {
            span: reference.name_span,
            kind,
        });
    }

    sort_dedup_occurrences(&mut occurrences);
    occurrences
}

fn occurrences_for_function(
    model: &SemanticModel,
    binding_id: BindingId,
    include_declaration: bool,
) -> Vec<EditorOccurrence> {
    let binding = model.binding(binding_id);
    if !matches!(binding.kind, BindingKind::FunctionDefinition) {
        return Vec::new();
    }

    let mut occurrences = Vec::new();
    if include_declaration {
        occurrences.push(EditorOccurrence {
            span: binding.span,
            kind: EditorOccurrenceKind::Write,
        });
    }
    occurrences.extend(
        model
            .analysis()
            .resolved_function_call_sites(&binding.name)
            .filter(|(_, candidate)| *candidate == binding_id)
            .map(|(site, _)| EditorOccurrence {
                span: site.name_span,
                kind: EditorOccurrenceKind::Read,
            }),
    );
    sort_dedup_occurrences(&mut occurrences);
    occurrences
}

fn storage_family_bindings(model: &SemanticModel, binding: &Binding) -> Vec<BindingId> {
    model
        .bindings_for(&binding.name)
        .iter()
        .copied()
        .filter(|candidate| {
            let candidate_binding = model.binding(*candidate);
            candidate_binding.scope == binding.scope
                && !matches!(candidate_binding.kind, BindingKind::FunctionDefinition)
                && editor_symbol_kind_for_binding(candidate_binding).is_some()
        })
        .collect()
}

fn rename_set_for_target(
    model: &SemanticModel,
    target: &EditorSymbolTarget,
) -> Result<RenameSet, RenameUnavailable> {
    match target {
        EditorSymbolTarget::Binding(binding_id) => rename_set_for_binding(model, *binding_id, None),
        EditorSymbolTarget::Reference(reference_id) => {
            let reference = model.reference(*reference_id);
            if matches!(reference.kind, ReferenceKind::IndirectExpansion) {
                return Err(RenameUnavailable::IndirectReference);
            }
            let binding_id = model
                .resolved
                .get(reference_id)
                .copied()
                .ok_or(RenameUnavailable::AmbiguousResolution)?;
            rename_set_for_binding(model, binding_id, Some(reference.name_span))
        }
        EditorSymbolTarget::FunctionCall(call) => {
            let binding_id = call.binding.ok_or(RenameUnavailable::AmbiguousResolution)?;
            rename_set_for_function(model, binding_id, call.name_span)
        }
        EditorSymbolTarget::RuntimeName(_) => Err(RenameUnavailable::ImportedBinding),
    }
}

fn rename_set_for_binding(
    model: &SemanticModel,
    binding_id: BindingId,
    editable_span: Option<Span>,
) -> Result<RenameSet, RenameUnavailable> {
    let binding = model.binding(binding_id);
    if matches!(binding.kind, BindingKind::FunctionDefinition) {
        return rename_set_for_function(model, binding_id, editable_span.unwrap_or(binding.span));
    }
    let kind = editor_symbol_kind_for_binding(binding).ok_or(RenameUnavailable::DynamicName)?;
    if !valid_variable_name(binding.name.as_str()) {
        return Err(RenameUnavailable::InvalidIdentifier);
    }

    let family = storage_family_bindings(model, binding);
    if family.is_empty() {
        return Err(RenameUnavailable::AmbiguousResolution);
    }
    for candidate in &family {
        let candidate = model.binding(*candidate);
        if binding_is_imported(candidate) {
            return Err(RenameUnavailable::ImportedBinding);
        }
        if matches!(candidate.kind, BindingKind::Nameref)
            || candidate.attributes.contains(BindingAttributes::NAMEREF)
        {
            return Err(RenameUnavailable::Nameref);
        }
        if matches!(candidate.kind, BindingKind::ParameterDefaultAssignment)
            || matches!(
                candidate.origin,
                BindingOrigin::ParameterDefaultAssignment { .. }
            )
        {
            return Err(RenameUnavailable::DynamicName);
        }
    }

    let mut spans = occurrences_for_binding_family(model, binding_id, true)
        .into_iter()
        .filter(|occurrence| {
            !model.references().iter().any(|reference| {
                reference.name_span == occurrence.span
                    && matches!(reference.kind, ReferenceKind::IndirectExpansion)
            })
        })
        .map(|occurrence| occurrence.span)
        .collect::<Vec<_>>();
    sort_dedup_spans(&mut spans);
    if spans.is_empty() {
        return Err(RenameUnavailable::AmbiguousResolution);
    }

    Ok(RenameSet {
        name: binding.name.clone(),
        kind,
        editable_span: editable_span.unwrap_or_else(|| rename_span_for_binding(binding)),
        spans,
    })
}

fn rename_set_for_function(
    model: &SemanticModel,
    binding_id: BindingId,
    editable_span: Span,
) -> Result<RenameSet, RenameUnavailable> {
    let binding = model.binding(binding_id);
    if !matches!(binding.kind, BindingKind::FunctionDefinition) {
        return Err(RenameUnavailable::AmbiguousResolution);
    }
    if binding_is_imported(binding) {
        return Err(RenameUnavailable::ImportedBinding);
    }
    if !valid_function_name(binding.name.as_str()) {
        return Err(RenameUnavailable::InvalidIdentifier);
    }
    let mut spans = occurrences_for_function(model, binding_id, true)
        .into_iter()
        .map(|occurrence| occurrence.span)
        .collect::<Vec<_>>();
    sort_dedup_spans(&mut spans);
    if spans.is_empty() {
        return Err(RenameUnavailable::AmbiguousResolution);
    }

    Ok(RenameSet {
        name: binding.name.clone(),
        kind: EditorSymbolKind::Function,
        editable_span,
        spans,
    })
}

fn rename_span_for_binding(binding: &Binding) -> Span {
    match binding.origin {
        BindingOrigin::ParameterDefaultAssignment { target_span, .. }
        | BindingOrigin::ArithmeticAssignment { target_span, .. } => target_span,
        _ => binding.span,
    }
}

fn source_backed_reference(reference: &Reference) -> bool {
    reference.name_span.start.offset < reference.name_span.end.offset
        && !matches!(
            reference.kind,
            ReferenceKind::ImplicitRead | ReferenceKind::RequiredRead
        )
}

fn sort_dedup_occurrences(occurrences: &mut Vec<EditorOccurrence>) {
    occurrences.sort_by_key(|occurrence| {
        (
            occurrence.span.start.offset,
            occurrence.span.end.offset,
            occurrence.kind == EditorOccurrenceKind::Read,
        )
    });
    occurrences.dedup_by_key(|occurrence| {
        (
            occurrence.span.start.offset,
            occurrence.span.end.offset,
            occurrence.kind,
        )
    });
}

fn sort_dedup_spans(spans: &mut Vec<Span>) {
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup_by_key(|span| (span.start.offset, span.end.offset));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionContext {
    Parameter { replacement_span: Span },
    Declaration { replacement_span: Span },
    Command { replacement_span: Span },
}

fn completion_context(source: &str, indexer: &Indexer, offset: usize) -> Option<CompletionContext> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }
    if completion_is_blocked_by_region(indexer, offset) {
        return None;
    }
    if let Some(replacement_span) = parameter_completion_span(source, offset) {
        return Some(CompletionContext::Parameter { replacement_span });
    }
    let word_span = current_word_span(source, offset);
    if declaration_operand_context(source, indexer, word_span.start.offset) {
        return Some(CompletionContext::Declaration {
            replacement_span: word_span,
        });
    }
    if command_position_context(source, word_span.start.offset) {
        return Some(CompletionContext::Command {
            replacement_span: word_span,
        });
    }
    None
}

fn completion_is_blocked_by_region(indexer: &Indexer, offset: usize) -> bool {
    let probe = TextSize::new(offset.saturating_sub(1) as u32);
    indexer.comment_index().is_comment(probe)
        || matches!(
            indexer.region_index().region_at(probe),
            Some(RegionKind::SingleQuoted | RegionKind::Heredoc)
        )
}

fn parameter_completion_span(source: &str, offset: usize) -> Option<Span> {
    let start = identifier_prefix_start(source, offset);
    let before = &source.as_bytes()[..start];
    if before.last() == Some(&b'$') {
        return Some(span_from_offsets(source, start, offset));
    }
    if before.last() == Some(&b'{') && before.get(before.len().saturating_sub(2)) == Some(&b'$') {
        return Some(span_from_offsets(source, start, offset));
    }
    if source[..offset].ends_with('$') || source[..offset].ends_with("${") {
        return Some(span_from_offsets(source, offset, offset));
    }
    None
}

fn current_word_span(source: &str, offset: usize) -> Span {
    let start = word_prefix_start(source, offset);
    span_from_offsets(source, start, offset)
}

fn identifier_prefix_start(source: &str, offset: usize) -> usize {
    let mut start = offset;
    while start > 0 {
        let Some((prev, ch)) = previous_char(source, start) else {
            break;
        };
        if !is_identifier_continue(ch) {
            break;
        }
        start = prev;
    }
    start
}

fn word_prefix_start(source: &str, offset: usize) -> usize {
    let mut start = offset;
    while start > 0 {
        let Some((prev, ch)) = previous_char(source, start) else {
            break;
        };
        if ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '{' | '}') {
            break;
        }
        start = prev;
    }
    start
}

fn previous_char(source: &str, offset: usize) -> Option<(usize, char)> {
    source[..offset].char_indices().next_back()
}

fn declaration_operand_context(source: &str, indexer: &Indexer, word_start: usize) -> bool {
    let line = indexer
        .line_index()
        .line_number(TextSize::new(word_start as u32));
    let Some(line_range) = indexer.line_index().line_range(line, source) else {
        return false;
    };
    let line_start = usize::from(line_range.start());
    let before = &source[line_start..word_start];
    let command_start = before
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            matches!(ch, ';' | '|' | '&' | '(' | '{').then_some(index + ch.len_utf8())
        })
        .unwrap_or(0);
    let mut words = before[command_start..].split_whitespace();
    let first = match words.next() {
        Some("then" | "do" | "else") => words.next(),
        first => first,
    };
    let Some(first) = first else {
        return false;
    };
    matches!(
        first,
        "declare" | "export" | "local" | "readonly" | "typeset"
    )
}

fn command_position_context(source: &str, word_start: usize) -> bool {
    let raw_before = &source[..word_start];
    if raw_before.ends_with('\n') {
        return true;
    }
    let before = raw_before.trim_end();
    if before.is_empty() {
        return true;
    }
    if before.ends_with('\n')
        || before.ends_with(';')
        || before.ends_with('|')
        || before.ends_with('&')
        || before.ends_with('(')
        || before.ends_with('{')
    {
        return true;
    }
    let previous = before
        .rsplit(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | '{'))
        .find(|word| !word.is_empty());
    matches!(previous, Some("then" | "do" | "else" | "elif"))
}

fn completions_for_context(
    model: &SemanticModel,
    source: &str,
    offset: usize,
    context: CompletionContext,
    options: EditorCompletionOptions,
) -> EditorCompletions {
    let (replacement_span, mut items) = match context {
        CompletionContext::Parameter { replacement_span } => (
            replacement_span,
            variable_completions(model, offset, options.include_runtime_names),
        ),
        CompletionContext::Declaration { replacement_span } => (
            replacement_span,
            variable_completions(model, offset, options.include_runtime_names),
        ),
        CompletionContext::Command { replacement_span } => {
            let mut items = function_completions(model, offset);
            items.extend(builtin_completions(model));
            if options.include_keywords {
                items.extend(keyword_completions());
            }
            (replacement_span, items)
        }
    };
    filter_and_sort_completions(&mut items, replacement_span.slice(source));
    EditorCompletions {
        replacement_span,
        items,
    }
}

fn variable_completions(
    model: &SemanticModel,
    offset: usize,
    include_runtime_names: bool,
) -> Vec<EditorCompletion> {
    let at = Span::at(position_with_offset(offset));
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();

    for binding in model.bindings() {
        if matches!(binding.kind, BindingKind::FunctionDefinition)
            || editor_symbol_kind_for_binding(binding).is_none()
            || !model.binding_visible_at(binding.id, at)
            || !seen.insert(binding.name.to_string())
        {
            continue;
        }
        items.push(EditorCompletion {
            name: binding.name.clone(),
            kind: EditorCompletionKind::Variable,
            definition_span: Some(binding_definition_span(binding)),
        });
    }

    if include_runtime_names {
        for name in model.runtime.preinitialized_names() {
            if seen.insert(name.to_owned()) {
                items.push(EditorCompletion {
                    name: Name::from(name),
                    kind: EditorCompletionKind::RuntimeName,
                    definition_span: None,
                });
            }
        }
    }

    items
}

fn function_completions(model: &SemanticModel, offset: usize) -> Vec<EditorCompletion> {
    let at = Span::at(position_with_offset(offset));
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for binding in model.function_definition_bindings() {
        if !model.binding_visible_at(binding.id, at) || !seen.insert(binding.name.to_string()) {
            continue;
        }
        items.push(EditorCompletion {
            name: binding.name.clone(),
            kind: EditorCompletionKind::Function,
            definition_span: Some(binding_definition_span(binding)),
        });
    }
    items
}

fn builtin_completions(model: &SemanticModel) -> Vec<EditorCompletion> {
    model
        .runtime
        .known_builtin_names()
        .into_iter()
        .map(|builtin| EditorCompletion {
            name: Name::from(builtin),
            kind: EditorCompletionKind::Builtin,
            definition_span: None,
        })
        .collect()
}

fn keyword_completions() -> Vec<EditorCompletion> {
    [
        "case", "coproc", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if",
        "in", "select", "then", "time", "until", "while",
    ]
    .into_iter()
    .map(|keyword| EditorCompletion {
        name: Name::from(keyword),
        kind: EditorCompletionKind::Keyword,
        definition_span: None,
    })
    .collect()
}

fn filter_and_sort_completions(items: &mut Vec<EditorCompletion>, prefix: &str) {
    if !prefix.is_empty() {
        items.retain(|item| item.name.as_str().starts_with(prefix));
    }
    items.sort_by_key(|item| {
        let rank = match item.kind {
            EditorCompletionKind::Variable => 0,
            EditorCompletionKind::Function => 1,
            EditorCompletionKind::Builtin => 2,
            EditorCompletionKind::RuntimeName => 3,
            EditorCompletionKind::Keyword => 4,
        };
        (rank, item.name.to_string())
    });
    items.dedup_by_key(|item| item.name.to_string());
}

fn span_from_offsets(source: &str, start: usize, end: usize) -> Span {
    Span::from_positions(
        position_at_offset(source, start),
        position_at_offset(source, end),
    )
}

fn position_at_offset(source: &str, offset: usize) -> shuck_ast::Position {
    source.get(..offset).unwrap_or(source).chars().fold(
        shuck_ast::Position::new(),
        |mut position, ch| {
            position.advance(ch);
            position
        },
    )
}

fn position_with_offset(offset: usize) -> shuck_ast::Position {
    shuck_ast::Position {
        line: 1,
        column: offset + 1,
        offset,
    }
}

fn valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn valid_function_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '$' | '`'
                        | '\\'
                        | '"'
                        | '\''
                        | ';'
                        | '&'
                        | '|'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '['
                        | ']'
                        | '*'
                        | '?'
                        | '/'
                )
        })
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn hover_for_binding(
    model: &SemanticModel,
    binding: &Binding,
    target_span: Span,
) -> Option<EditorHover> {
    let kind = hover_symbol_kind_for_binding(binding)?;
    let definition_span = binding_definition_span(binding);
    Some(EditorHover {
        symbol: EditorSymbol {
            name: binding.name.clone(),
            kind,
            definition_span,
            selection_span: binding.span,
            scope: binding.scope,
            binding: Some(binding.id),
        },
        target_span,
        attributes: binding.attributes,
        imported: binding_is_imported(binding),
        runtime: false,
        function_call_count: (kind == EditorSymbolKind::Function)
            .then(|| function_call_count_for_binding(model, binding)),
    })
}

fn runtime_hover(model: &SemanticModel, name: Name, target_span: Span) -> EditorHover {
    let kind = if model.runtime.is_preinitialized_associative_array(&name) {
        EditorSymbolKind::AssociativeArray
    } else if model.runtime.is_preinitialized_array(&name) {
        EditorSymbolKind::Array
    } else {
        EditorSymbolKind::RuntimeName
    };
    EditorHover {
        symbol: EditorSymbol {
            name,
            kind,
            definition_span: target_span,
            selection_span: target_span,
            scope: model.scope_at(target_span.start.offset),
            binding: None,
        },
        target_span,
        attributes: BindingAttributes::empty(),
        imported: false,
        runtime: true,
        function_call_count: None,
    }
}

fn hover_symbol_kind_for_binding(binding: &Binding) -> Option<EditorSymbolKind> {
    if matches!(binding.kind, BindingKind::FunctionDefinition)
        || binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FUNCTION)
    {
        return Some(EditorSymbolKind::Function);
    }
    if matches!(
        binding.kind,
        BindingKind::Imported | BindingKind::ParameterDefaultAssignment
    ) {
        return Some(EditorSymbolKind::Variable);
    }
    editor_symbol_kind_for_binding(binding)
}

fn binding_is_imported(binding: &Binding) -> bool {
    matches!(binding.kind, BindingKind::Imported)
        || binding.attributes.intersects(
            BindingAttributes::IMPORTED_POSSIBLE
                | BindingAttributes::IMPORTED_FUNCTION
                | BindingAttributes::IMPORTED_FILE_ENTRY
                | BindingAttributes::IMPORTED_FILE_ENTRY_INITIALIZED,
        )
}

fn function_call_count_for_binding(model: &SemanticModel, binding: &Binding) -> usize {
    model
        .analysis()
        .resolved_function_call_sites(&binding.name)
        .filter(|(_, binding_id)| *binding_id == binding.id)
        .count()
}

fn binding_hover_span(binding: &Binding) -> Span {
    match binding.origin {
        crate::BindingOrigin::ParameterDefaultAssignment { target_span, .. } => target_span,
        _ => binding.span,
    }
}

fn document_symbol_parent_for_binding(
    model: &SemanticModel,
    binding: &Binding,
    function_symbols_by_scope: &FxHashMap<ScopeId, usize>,
) -> Option<Option<usize>> {
    if file_scope_binding_is_document_symbol(model, binding) {
        return Some(None);
    }

    if function_child_binding_is_document_symbol(binding) {
        return enclosing_function_symbol(model, binding.scope, function_symbols_by_scope)
            .map(Some);
    }

    None
}

fn file_scope_binding_is_document_symbol(model: &SemanticModel, binding: &Binding) -> bool {
    matches!(model.scope_kind(binding.scope), ScopeKind::File)
        && matches!(
            binding.kind,
            BindingKind::Assignment
                | BindingKind::AppendAssignment
                | BindingKind::ArrayAssignment
                | BindingKind::Declaration(_)
                | BindingKind::Nameref
        )
}

fn function_child_binding_is_document_symbol(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Declaration(_) | BindingKind::LoopVariable | BindingKind::Nameref
    )
}

fn document_symbol_for_function(binding: &Binding) -> Option<EditorDocumentSymbol> {
    let BindingKind::FunctionDefinition = binding.kind else {
        return None;
    };
    let definition_span = binding_definition_span(binding);
    Some(EditorDocumentSymbol {
        range: definition_span,
        symbol: EditorSymbol {
            name: binding.name.clone(),
            kind: EditorSymbolKind::Function,
            definition_span,
            selection_span: binding.span,
            scope: binding.scope,
            binding: Some(binding.id),
        },
        children: Vec::new(),
    })
}

fn document_symbol_for_binding(
    binding: &Binding,
    document_symbol_ranges_by_binding: &DocumentSymbolRangesByBinding,
) -> Option<EditorDocumentSymbol> {
    let definition_span = binding_definition_span(binding);
    Some(EditorDocumentSymbol {
        range: document_symbol_range_for_binding(binding, document_symbol_ranges_by_binding)
            .unwrap_or(definition_span),
        symbol: EditorSymbol {
            name: binding.name.clone(),
            kind: editor_symbol_kind_for_binding(binding)?,
            definition_span,
            selection_span: binding.span,
            scope: binding.scope,
            binding: Some(binding.id),
        },
        children: Vec::new(),
    })
}

fn editor_symbol_kind_for_binding(binding: &Binding) -> Option<EditorSymbolKind> {
    if binding.attributes.contains(BindingAttributes::ASSOC) {
        return Some(EditorSymbolKind::AssociativeArray);
    }
    if binding.attributes.contains(BindingAttributes::ARRAY)
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment
                | BindingKind::MapfileTarget
                | BindingKind::ZparseoptsTarget
        )
    {
        return Some(EditorSymbolKind::Array);
    }

    match binding.kind {
        BindingKind::Declaration(_) | BindingKind::Nameref => Some(EditorSymbolKind::Declaration),
        BindingKind::Assignment
        | BindingKind::AppendAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => Some(EditorSymbolKind::Variable),
        BindingKind::ArrayAssignment
        | BindingKind::MapfileTarget
        | BindingKind::ZparseoptsTarget => Some(EditorSymbolKind::Array),
        BindingKind::FunctionDefinition
        | BindingKind::Imported
        | BindingKind::ParameterDefaultAssignment => None,
    }
}

fn document_symbol_range_for_binding(
    binding: &Binding,
    document_symbol_ranges_by_binding: &DocumentSymbolRangesByBinding,
) -> Option<Span> {
    if !matches!(
        binding.kind,
        BindingKind::Declaration(_) | BindingKind::Nameref
    ) {
        return None;
    }

    document_symbol_ranges_by_binding
        .get(binding.id.index())
        .copied()
        .flatten()
}

fn document_symbol_ranges_by_binding(model: &SemanticModel) -> DocumentSymbolRangesByBinding {
    let declaration_operand_ranges = declaration_operand_ranges_by_definition_span(model);
    let mut ranges = vec![None; model.bindings().len()];
    for binding in model.bindings() {
        ranges[binding.id.index()] =
            declaration_operand_range_for_binding(binding, &declaration_operand_ranges);
    }
    ranges
}

fn declaration_operand_range_for_binding(
    binding: &Binding,
    declaration_operand_ranges: &DeclarationOperandRanges,
) -> Option<Span> {
    if matches!(
        binding.kind,
        BindingKind::Declaration(_) | BindingKind::Nameref
    ) {
        if let Some(range) = declaration_operand_ranges.get(&SpanKey::new(binding.span)) {
            return Some(*range);
        }

        if let Some(range) =
            declaration_operand_ranges.get(&SpanKey::new(binding_definition_span(binding)))
        {
            return Some(*range);
        }
    }

    None
}

fn declaration_operand_ranges_by_definition_span(
    model: &SemanticModel,
) -> DeclarationOperandRanges {
    let mut ranges = DeclarationOperandRanges::default();
    for declaration in model.declarations() {
        for operand in &declaration.operands {
            match operand {
                DeclarationOperand::Name { span, .. } => {
                    ranges.insert(SpanKey::new(*span), *span);
                }
                DeclarationOperand::Assignment {
                    operand_span,
                    target_span,
                    name_span,
                    ..
                } => {
                    ranges.insert(SpanKey::new(*name_span), *operand_span);
                    ranges.insert(SpanKey::new(*target_span), *operand_span);
                }
                DeclarationOperand::Flag { .. } | DeclarationOperand::DynamicWord { .. } => {}
            }
        }
    }
    ranges
}

fn binding_definition_span(binding: &Binding) -> Span {
    match binding.origin {
        crate::BindingOrigin::Assignment {
            definition_span, ..
        }
        | crate::BindingOrigin::LoopVariable {
            definition_span, ..
        }
        | crate::BindingOrigin::ParameterDefaultAssignment {
            definition_span, ..
        }
        | crate::BindingOrigin::Imported { definition_span }
        | crate::BindingOrigin::FunctionDefinition { definition_span }
        | crate::BindingOrigin::BuiltinTarget {
            definition_span, ..
        }
        | crate::BindingOrigin::ArithmeticAssignment {
            definition_span, ..
        }
        | crate::BindingOrigin::Declaration { definition_span }
        | crate::BindingOrigin::Nameref { definition_span } => definition_span,
    }
}

fn enclosing_function_symbol(
    model: &SemanticModel,
    scope: ScopeId,
    function_symbols_by_scope: &FxHashMap<ScopeId, usize>,
) -> Option<usize> {
    model
        .ancestor_scopes(scope)
        .find_map(|scope| function_symbols_by_scope.get(&scope).copied())
}

fn build_document_symbol_tree(
    mut pending: Vec<PendingDocumentSymbol>,
) -> Vec<EditorDocumentSymbol> {
    let mut child_ids = vec![Vec::new(); pending.len()];
    let mut root_ids = Vec::new();

    for (index, symbol) in pending.iter().enumerate() {
        if let Some(parent) = symbol.parent {
            child_ids[parent].push(index);
        } else {
            root_ids.push(index);
        }
    }

    let sort_offsets = pending
        .iter()
        .map(|symbol| symbol.sort_offset)
        .collect::<Vec<_>>();
    root_ids.sort_by_key(|index| sort_offsets[*index]);
    for children in &mut child_ids {
        children.sort_by_key(|index| sort_offsets[*index]);
    }

    root_ids
        .into_iter()
        .map(|index| take_document_symbol(index, &mut pending, &child_ids))
        .collect()
}

fn take_document_symbol(
    index: usize,
    pending: &mut [PendingDocumentSymbol],
    child_ids: &[Vec<usize>],
) -> EditorDocumentSymbol {
    let mut symbol = pending[index].symbol.clone();
    symbol.children = child_ids[index]
        .iter()
        .copied()
        .map(|child| take_document_symbol(child, pending, child_ids))
        .collect();
    symbol
}
