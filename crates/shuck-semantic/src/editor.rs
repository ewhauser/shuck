//! Editor-facing semantic query shapes.

use std::ops::Deref;

use rustc_hash::FxHashMap;
use shuck_ast::{Name, Span};

use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, DeclarationOperand, ScopeId, ScopeKind,
    SemanticModel, SpanKey,
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
        | crate::BindingOrigin::ParameterDefaultAssignment { definition_span }
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
