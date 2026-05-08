//! Semantic-owned array-use classification and supporting indexes.
//!
//! This module centralizes the meaning-level question "what kind of value does this
//! reference behave like at this use site?" so the linter facts layer can stay focused on
//! syntactic candidate discovery and rule-specific suppressions.
//!
//! There are two main consumers:
//! - plain unindexed reference classification via `SemanticModel::reference_array_use_kind`
//! - zsh unquoted fanout checks via `SemanticValueFlow`, which reuse the same indexes and
//!   binding-shape predicates defined here

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::*;

/// Lazy semantic indexes shared by array-use classification and zsh fanout checks.
///
/// The cached data here falls into two buckets:
/// - declaration-history indexes used to answer "did a later local scalar barrier or
///   name-only declaration break inherited array shape?"
/// - name-based fast paths used by zsh fanout checks to avoid paying for full value-flow
///   on obviously scalar or obviously array-valued names
#[derive(Debug)]
pub(crate) struct ArrayUseIndex {
    name_only_local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    initialized_scalar_local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    binding_inherits_indexed_array_type: Vec<bool>,
    binding_has_prior_local_barrier: Vec<bool>,
    array_like_capable_names: FxHashSet<Name>,
    single_array_like_bindings: FxHashMap<Name, BindingId>,
    array_like_bindings_by_name: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    uniformly_array_like_names: FxHashSet<Name>,
}

impl ArrayUseIndex {
    /// Builds all semantic-owned array-use indexes once from the completed model.
    fn build(model: &SemanticModel) -> Self {
        let mut local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut name_only_local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut initialized_scalar_local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut append_local_declaration_spans = FxHashSet::default();

        for declaration in model.declarations() {
            if !matches!(declaration.builtin, DeclarationBuiltin::Local) {
                continue;
            }

            let scope = model.scope_at(declaration.span.start.offset);
            let declaration_has_array_flag = declaration.operands.iter().any(|operand| {
                matches!(
                    operand,
                    DeclarationOperand::Flag {
                        flag: 'a' | 'A',
                        ..
                    }
                )
            });
            for operand in &declaration.operands {
                match operand {
                    DeclarationOperand::Name { name, .. } => {
                        local_declarations_by_scope_name
                            .entry((scope, name.clone()))
                            .or_default()
                            .push(declaration.span);
                        name_only_local_declarations_by_scope_name
                            .entry((scope, name.clone()))
                            .or_default()
                            .push(declaration.span);
                    }
                    DeclarationOperand::Assignment {
                        name,
                        name_span,
                        append,
                        ..
                    } => {
                        local_declarations_by_scope_name
                            .entry((scope, name.clone()))
                            .or_default()
                            .push(declaration.span);
                        if !*append && !declaration_has_array_flag {
                            initialized_scalar_local_declarations_by_scope_name
                                .entry((scope, name.clone()))
                                .or_default()
                                .push(declaration.span);
                        }
                        if *append {
                            append_local_declaration_spans.insert((
                                scope,
                                name.clone(),
                                name_span.start.offset,
                                name_span.end.offset,
                            ));
                        }
                    }
                    DeclarationOperand::Flag { .. } | DeclarationOperand::DynamicWord { .. } => {}
                }
            }
        }

        let binding_has_prior_local_barrier = model
            .bindings()
            .iter()
            .map(|binding| {
                local_declarations_by_scope_name
                    .get(&(binding.scope, binding.name.clone()))
                    .is_some_and(|spans| {
                        spans
                            .iter()
                            .any(|span| span.end.offset < binding.span.start.offset)
                    })
            })
            .collect::<Vec<_>>();
        let binding_is_append_declaration = model
            .bindings()
            .iter()
            .map(|binding| {
                append_local_declaration_spans.contains(&(
                    binding.scope,
                    binding.name.clone(),
                    binding.span.start.offset,
                    binding.span.end.offset,
                ))
            })
            .collect::<Vec<_>>();

        let mut binding_inherits_indexed_array_type = vec![false; model.bindings().len()];
        for binding in model.bindings() {
            let inherited = if binding_resets_indexed_array_type(binding) {
                false
            } else {
                let initialized_scalar_declaration =
                    binding_is_initialized_scalar_declaration(binding);
                let append_declaration = binding_is_append_declaration[binding.id.index()];
                let prior_local_barrier = binding_has_prior_local_barrier[binding.id.index()];
                let same_scope_candidate_allowed = !initialized_scalar_declaration
                    || append_declaration
                    || model.shell_profile().dialect != ShellDialect::Zsh;

                let mut inherited = false;
                for candidate_id in model.bindings_for(&binding.name).iter().copied().rev() {
                    let candidate = model.binding(candidate_id);
                    if candidate.span.start.offset >= binding.span.start.offset {
                        continue;
                    }
                    if !same_scope_candidate_allowed
                        && (candidate.scope == binding.scope || prior_local_barrier)
                    {
                        continue;
                    }
                    if binding_reset_by_name_only_declaration_before(
                        &name_only_local_declarations_by_scope_name,
                        candidate,
                        binding.span,
                    ) {
                        continue;
                    }
                    if binding_resets_indexed_array_type(candidate) {
                        inherited = false;
                        break;
                    }
                    if binding_has_sticky_indexed_array_type(candidate) {
                        inherited = true;
                        break;
                    }
                }
                inherited
            };
            binding_inherits_indexed_array_type[binding.id.index()] = inherited;
        }

        let mut array_like_capable_names = FxHashSet::default();
        let mut single_array_like_bindings = FxHashMap::<Name, Option<BindingId>>::default();
        let mut array_like_bindings_by_name =
            FxHashMap::<Name, SmallVec<[BindingId; 2]>>::default();
        let mut uniformly_array_like_names = FxHashMap::<Name, bool>::default();

        for binding in model.bindings() {
            if !binding_can_supply_parameter_value(binding) {
                continue;
            }

            let array_like = binding_has_array_value_shape(binding);
            if array_like {
                array_like_capable_names.insert(binding.name.clone());
                array_like_bindings_by_name
                    .entry(binding.name.clone())
                    .or_default()
                    .push(binding.id);
            }

            match single_array_like_bindings.entry(binding.name.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(array_like.then_some(binding.id));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.insert(None);
                }
            }

            match uniformly_array_like_names.entry(binding.name.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(array_like);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if !array_like {
                        *entry.get_mut() = false;
                    }
                }
            }
        }

        Self {
            name_only_local_declarations_by_scope_name,
            initialized_scalar_local_declarations_by_scope_name,
            binding_inherits_indexed_array_type,
            binding_has_prior_local_barrier,
            array_like_capable_names,
            single_array_like_bindings: single_array_like_bindings
                .into_iter()
                .filter_map(|(name, binding_id)| binding_id.map(|binding_id| (name, binding_id)))
                .collect(),
            array_like_bindings_by_name,
            uniformly_array_like_names: uniformly_array_like_names
                .into_iter()
                .filter_map(|(name, uniform)| uniform.then_some(name))
                .collect(),
        }
    }
}

impl SemanticModel {
    /// Returns the lazy array-use index shared across semantic array queries.
    pub(crate) fn array_use_index(&self) -> &ArrayUseIndex {
        self.array_use_index
            .get_or_init(|| ArrayUseIndex::build(self))
    }

    /// Returns whether the binding itself has array value shape at its definition site.
    ///
    /// This is the shared predicate used by facts and rules that need the direct shape of a
    /// binding, independent of inherited array history.
    #[doc(hidden)]
    pub fn binding_has_array_value_shape(&self, binding_id: BindingId) -> bool {
        binding_has_array_value_shape(self.binding(binding_id))
    }

    /// Returns whether this binding contributes sticky indexed-array type for later plain reads.
    ///
    /// Unlike `binding_has_array_value_shape`, this excludes uninitialized `local -a/-A`
    /// declarations because those reserve array intent without yet making a later plain read
    /// definitively array-valued.
    pub(crate) fn binding_has_sticky_indexed_array_type(&self, binding_id: BindingId) -> bool {
        binding_has_sticky_indexed_array_type(self.binding(binding_id))
    }

    /// Returns whether the binding can supply a runtime parameter value at all.
    ///
    /// This stays semantic-owned so value-flow, fanout checks, and linter consumers all agree on
    /// which bindings participate in value propagation.
    #[doc(hidden)]
    pub fn binding_can_supply_parameter_value(&self, binding_id: BindingId) -> bool {
        binding_can_supply_parameter_value(self.binding(binding_id))
    }

    /// Returns whether the binding inherits indexed-array type from earlier visible history.
    pub(crate) fn binding_inherits_indexed_array_type(&self, binding_id: BindingId) -> bool {
        self.array_use_index().binding_inherits_indexed_array_type[binding_id.index()]
    }

    /// Returns whether a prior local declaration in the same scope forms a scalar barrier.
    pub(crate) fn binding_has_prior_local_barrier(&self, binding_id: BindingId) -> bool {
        self.array_use_index().binding_has_prior_local_barrier[binding_id.index()]
    }

    /// Returns whether a later name-only local declaration hides this binding before `at`.
    pub(crate) fn binding_reset_by_name_only_declaration_before(
        &self,
        binding_id: BindingId,
        at: Span,
    ) -> bool {
        binding_reset_by_name_only_declaration_before(
            &self
                .array_use_index()
                .name_only_local_declarations_by_scope_name,
            self.binding(binding_id),
            at,
        )
    }

    pub(crate) fn name_has_array_value_candidates(&self, name: &Name) -> bool {
        self.array_use_index()
            .array_like_capable_names
            .contains(name)
    }

    pub(crate) fn single_array_value_binding_for_name(&self, name: &Name) -> Option<BindingId> {
        self.array_use_index()
            .single_array_like_bindings
            .get(name)
            .copied()
    }

    pub(crate) fn name_is_uniformly_array_valued(&self, name: &Name) -> bool {
        self.array_use_index()
            .uniformly_array_like_names
            .contains(name)
    }

    pub(crate) fn array_value_bindings_for_name(&self, name: &Name) -> &[BindingId] {
        self.array_use_index()
            .array_like_bindings_by_name
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns whether zsh has a prior initialized local scalar declaration that prevents a
    /// plain unindexed reference from inheriting outer array shape.
    ///
    /// This is the semantic part of the old linter-local "scalar local barrier" rule; the facts
    /// layer still decides whether a classified reference should be suppressed for policy reasons.
    pub(crate) fn reference_has_prior_zsh_scalar_local_barrier(
        &self,
        reference: &Reference,
    ) -> bool {
        if self.shell_profile().dialect != ShellDialect::Zsh {
            return false;
        }

        let latest_barrier = self
            .ancestor_scopes(self.scope_at(reference.span.start.offset))
            .flat_map(|scope| {
                self.array_use_index()
                    .initialized_scalar_local_declarations_by_scope_name
                    .get(&(scope, reference.name.clone()))
                    .into_iter()
                    .flatten()
                    .copied()
            })
            .filter(|span| span.end.offset < reference.span.start.offset)
            .max_by_key(|span| span.start.offset);

        latest_barrier.is_some_and(|barrier| {
            !self.zsh_array_binding_after_scalar_local_barrier(reference, barrier)
        })
    }

    fn zsh_array_binding_after_scalar_local_barrier(
        &self,
        reference: &Reference,
        barrier: Span,
    ) -> bool {
        self.bindings_for(&reference.name)
            .iter()
            .copied()
            .any(|binding_id| {
                let binding = self.binding(binding_id);
                binding.span.start.offset > barrier.start.offset
                    && binding.span.start.offset < reference.span.start.offset
                    && self.binding_visible_at(binding_id, reference.span)
                    && self.binding_has_sticky_indexed_array_type(binding_id)
            })
    }

    /// Classifies a plain unindexed reference by the array semantics visible at that use site.
    ///
    /// The returned policy is only produced when the use site is meaningfully array-like after
    /// considering runtime arrays, inherited array history, name-only declaration resets, and zsh
    /// scalar-local barriers. Fact-local suppressions such as presence tests or same-command
    /// reader/writer filtering intentionally remain outside this semantic query.
    #[doc(hidden)]
    pub fn reference_array_use_kind(
        &self,
        reference_id: ReferenceId,
    ) -> Option<ArrayReferencePolicy> {
        let reference = self.reference(reference_id);
        if is_bash_runtime_array_name(reference.name.as_str()) {
            return Some(ArrayReferencePolicy::RequiresExplicitSelector);
        }

        if self.reference_has_prior_zsh_scalar_local_barrier(reference) {
            return None;
        }

        if let Some(binding) = self.resolved_binding(reference.id)
            && self.binding_visible_at(binding.id, reference.span)
            && !self.binding_has_sticky_indexed_array_type(binding.id)
            && !self.binding_inherits_indexed_array_type(binding.id)
            && (binding_resets_indexed_array_type(binding)
                || self.binding_has_prior_local_barrier(binding.id)
                || (self.shell_profile().dialect == ShellDialect::Zsh
                    && binding_is_initialized_scalar_declaration(binding)))
        {
            return None;
        }

        let mut binding_ids = Vec::new();
        let mut seen = FxHashSet::default();
        if let Some(binding) = self.resolved_binding(reference.id)
            && !self.binding_has_sticky_indexed_array_type(binding.id)
            && seen.insert(binding.id)
        {
            binding_ids.push(binding.id);
        }
        for binding_id in self.visible_candidate_bindings_for_reference(reference) {
            if seen.insert(binding_id) {
                binding_ids.push(binding_id);
            }
        }

        let array_like = binding_ids.into_iter().any(|binding_id| {
            !self.binding_reset_by_name_only_declaration_before(binding_id, reference.span)
                && (self.binding_has_sticky_indexed_array_type(binding_id)
                    || self.binding_inherits_indexed_array_type(binding_id))
        });
        if !array_like {
            return None;
        }

        Some(
            self.shell_behavior_at(reference.span.start.offset)
                .array_reference_policy(),
        )
    }
}

fn binding_reset_by_name_only_declaration_before(
    name_only_local_declarations_by_scope_name: &FxHashMap<(ScopeId, Name), Vec<Span>>,
    binding: &Binding,
    at: Span,
) -> bool {
    name_only_local_declarations_by_scope_name
        .get(&(binding.scope, binding.name.clone()))
        .is_some_and(|spans| {
            spans.iter().any(|span| {
                span.start.offset > binding.span.start.offset && span.end.offset < at.start.offset
            })
        })
}

/// Returns whether the binding directly carries array value shape at its own definition.
fn binding_has_array_value_shape(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
}

/// Returns whether this binding should keep indexed-array type sticky for later plain reads.
fn binding_has_sticky_indexed_array_type(binding: &Binding) -> bool {
    !binding_is_uninitialized_local_array_declaration(binding)
        && (binding.attributes.contains(BindingAttributes::ARRAY)
            || matches!(
                binding.kind,
                BindingKind::ArrayAssignment | BindingKind::MapfileTarget
            ))
}

/// Returns whether this binding participates in value flow for parameter reads.
fn binding_can_supply_parameter_value(binding: &Binding) -> bool {
    match binding.origin {
        BindingOrigin::FunctionDefinition { .. } => false,
        BindingOrigin::Declaration { .. } => {
            binding_is_name_only_declaration(binding)
                || binding.attributes.intersects(
                    BindingAttributes::DECLARATION_INITIALIZED | BindingAttributes::INTEGER,
                )
        }
        _ => true,
    }
}

/// Returns whether the binding is a name-only local declaration like `local foo`.
fn binding_is_name_only_declaration(binding: &Binding) -> bool {
    matches!(binding.origin, BindingOrigin::Declaration { .. })
        && binding.attributes.contains(BindingAttributes::LOCAL)
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

/// Returns whether this binding breaks inherited indexed-array type for later plain reads.
fn binding_resets_indexed_array_type(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::ArithmeticAssignment
            | BindingKind::GetoptsTarget
            | BindingKind::Imported
            | BindingKind::LoopVariable
            | BindingKind::PrintfTarget
    ) || (matches!(binding.kind, BindingKind::ReadTarget)
        && !binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::Declaration(_))
            && !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
            && !binding
                .attributes
                .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC))
}

/// Returns whether this binding is an initialized scalar declaration such as `local foo=value`.
fn binding_is_initialized_scalar_declaration(binding: &Binding) -> bool {
    matches!(binding.kind, BindingKind::Declaration(_))
        && binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
        && !binding
            .attributes
            .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
}

/// Returns whether this binding is only reserving an array-shaped local without initializing it.
fn binding_is_uninitialized_local_array_declaration(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Declaration(DeclarationBuiltin::Local)
    ) && binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

/// Returns whether the name is one of bash's predefined runtime arrays.
///
/// This stays as a name-based check rather than `reference_is_predefined_runtime_array` because
/// we intentionally preserve the linter's historical behavior for these names even outside bash
/// profile setup and even after local scalar rebinding.
fn is_bash_runtime_array_name(name: &str) -> bool {
    matches!(
        name,
        "BASH_ALIASES"
            | "BASH_ARGC"
            | "BASH_ARGV"
            | "BASH_CMDS"
            | "BASH_LINENO"
            | "BASH_REMATCH"
            | "BASH_SOURCE"
            | "BASH_VERSINFO"
            | "COMP_WORDS"
            | "COMPREPLY"
            | "COPROC"
            | "DIRSTACK"
            | "FUNCNAME"
            | "GROUPS"
            | "MAPFILE"
            | "PIPESTATUS"
    )
}
