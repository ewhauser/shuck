//! Workspace variable facts for cross-file editor navigation.
//!
//! Shell files sourced at top level execute in the caller's variable
//! environment. This index projects source-backed, file-scope variable
//! definitions and references, then relates them through statically resolved
//! top-level `source`/`.` edges. Function-local and transient bindings stay
//! document-local so lexical shadows never leak into workspace results.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use shuck_ast::{Name, Span};
use shuck_semantic::{
    Binding, BindingAttributes, BindingKind, BindingOrigin, CallFactSourceEffect,
    EditorSymbolTarget, ReferenceKind, SemanticModel,
};

/// A variable target under the active editor cursor.
pub(crate) struct WorkspaceVariableTarget {
    name: Name,
    cutoff: usize,
    local_family: bool,
}

/// One path/span pair returned by a workspace variable query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceVariableOccurrence {
    pub(crate) path: PathBuf,
    pub(crate) span: Span,
}

#[derive(Clone, Debug)]
struct VariableDefinition {
    name: Name,
    definition_span: Span,
    occurrence_span: Span,
}

#[derive(Clone, Debug)]
struct VariableReference {
    name: Name,
    occurrence_span: Span,
    cutoff: usize,
    local_family: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct IncomingVariableSource {
    parent: PathBuf,
    cutoff: usize,
}

#[derive(Clone, Debug, Default)]
struct FileVariableFacts {
    definitions: Vec<VariableDefinition>,
    references: Vec<VariableReference>,
    source_effects: Vec<CallFactSourceEffect>,
}

impl FileVariableFacts {
    fn project(model: &SemanticModel, source_effects: &[CallFactSourceEffect]) -> Self {
        let definitions = model
            .bindings()
            .iter()
            .filter(|binding| persistent_file_variable_binding(model, binding))
            .map(|binding| VariableDefinition {
                name: binding.name.clone(),
                definition_span: binding_definition_span(binding),
                occurrence_span: binding_occurrence_span(binding),
            })
            .collect();
        let references = model
            .references()
            .iter()
            .filter_map(|reference| {
                if !source_backed_reference(reference.kind, reference.name_span)
                    || !valid_variable_name(reference.name.as_str())
                {
                    return None;
                }
                let local_family = match model.resolved_binding(reference.id) {
                    Some(binding) if persistent_file_variable_binding(model, binding) => true,
                    Some(_) => return None,
                    None if model.name_is_predefined_runtime(reference.name.as_str()) => {
                        return None;
                    }
                    None => false,
                };
                Some(VariableReference {
                    name: reference.name.clone(),
                    occurrence_span: reference.name_span,
                    cutoff: if model.enclosing_function_scope(reference.scope).is_some() {
                        usize::MAX
                    } else {
                        reference.name_span.start.offset()
                    },
                    local_family,
                })
            })
            .collect();

        Self {
            definitions,
            references,
            source_effects: source_effects.to_vec(),
        }
    }

    fn has_definition(&self, name: &Name) -> bool {
        self.definitions
            .iter()
            .any(|definition| definition.name == *name)
    }

    fn has_definition_before(&self, name: &Name, cutoff: usize) -> bool {
        self.definitions.iter().any(|definition| {
            definition.name == *name && definition.definition_span.start.offset() < cutoff
        })
    }

    fn resolved_top_level_source_edges(
        &self,
        cutoff: usize,
    ) -> impl Iterator<Item = (&Path, usize)> {
        self.source_effects.iter().filter_map(move |effect| {
            (effect.persistent
                && effect.enclosing_function.is_none()
                && effect.span.start.offset() < cutoff)
                .then_some(
                    effect
                        .path
                        .as_deref()
                        .map(|path| (path, effect.span.start.offset())),
                )
                .flatten()
        })
    }

    fn unconditional_top_level_source_paths(&self, cutoff: usize) -> impl Iterator<Item = &Path> {
        self.source_effects.iter().filter_map(move |effect| {
            (effect.persistent
                && !effect.conditional
                && effect.enclosing_function.is_none()
                && effect.span.start.offset() < cutoff)
                .then_some(effect.path.as_deref())
                .flatten()
        })
    }

    fn has_ambiguous_top_level_source_before(&self, cutoff: usize) -> bool {
        self.source_effects.iter().any(|effect| {
            effect.persistent
                && effect.enclosing_function.is_none()
                && effect.span.start.offset() < cutoff
                && (effect.conditional || effect.path.is_none())
        })
    }
}

/// Compact variable projection over the same files as the workspace function index.
#[derive(Default)]
pub(crate) struct WorkspaceVariableIndex {
    files: BTreeMap<PathBuf, FileVariableFacts>,
    incoming: BTreeMap<PathBuf, BTreeSet<IncomingVariableSource>>,
}

impl WorkspaceVariableIndex {
    pub(crate) fn insert(
        &mut self,
        path: PathBuf,
        model: &SemanticModel,
        source_effects: &[CallFactSourceEffect],
    ) {
        self.insert_facts(path, FileVariableFacts::project(model, source_effects));
    }

    fn insert_facts(&mut self, path: PathBuf, facts: FileVariableFacts) {
        if let Some(previous) = self.files.remove(&path) {
            for (target, cutoff) in previous.resolved_top_level_source_edges(usize::MAX) {
                if let Some(sources) = self.incoming.get_mut(target) {
                    sources.remove(&IncomingVariableSource {
                        parent: path.clone(),
                        cutoff,
                    });
                }
            }
        }
        for (target, cutoff) in facts.resolved_top_level_source_edges(usize::MAX) {
            self.incoming
                .entry(target.to_path_buf())
                .or_default()
                .insert(IncomingVariableSource {
                    parent: path.clone(),
                    cutoff,
                });
        }
        self.files.insert(path, facts);
    }

    /// Returns `None` when source effects are ambiguous or the query is cancelled.
    pub(crate) fn definitions(
        &self,
        from_path: &Path,
        target: &WorkspaceVariableTarget,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<WorkspaceVariableOccurrence>> {
        let paths = self.definition_paths(
            from_path,
            &target.name,
            target.cutoff,
            target.local_family,
            is_cancelled,
        )?;
        self.definition_occurrences(&paths, &target.name, is_cancelled)
    }

    /// Returns `None` when the complete reference family cannot be proven or queried.
    pub(crate) fn references(
        &self,
        from_path: &Path,
        target: &WorkspaceVariableTarget,
        include_declaration: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<WorkspaceVariableOccurrence>> {
        let definitions = self.definitions(from_path, target, is_cancelled)?;
        let seed_paths = definitions
            .iter()
            .map(|definition| definition.path.clone())
            .collect::<BTreeSet<_>>();
        if seed_paths.is_empty() {
            return Some(Vec::new());
        }

        let family_paths = self.environment_paths_from(&seed_paths, is_cancelled)?;

        let mut occurrences = Vec::new();
        let mut inherited_cache = BTreeMap::new();
        for path in &family_paths {
            if is_cancelled() {
                return None;
            }
            let Some(facts) = self.files.get(path) else {
                continue;
            };
            if facts.has_ambiguous_top_level_source_before(usize::MAX) {
                return None;
            }
            if include_declaration {
                for definition in &facts.definitions {
                    if is_cancelled() {
                        return None;
                    }
                    if definition.name == target.name {
                        occurrences.push(WorkspaceVariableOccurrence {
                            path: path.clone(),
                            span: definition.occurrence_span,
                        });
                    }
                }
            }
            for reference in &facts.references {
                if is_cancelled() {
                    return None;
                }
                if reference.name != target.name {
                    continue;
                }
                let definition_paths = self.definition_paths_with_cache(
                    path,
                    &reference.name,
                    reference.cutoff,
                    reference.local_family,
                    &mut inherited_cache,
                    is_cancelled,
                )?;
                if definition_paths
                    .iter()
                    .any(|definition_path| seed_paths.contains(definition_path))
                {
                    occurrences.push(WorkspaceVariableOccurrence {
                        path: path.clone(),
                        span: reference.occurrence_span,
                    });
                }
            }
        }
        if is_cancelled() {
            return None;
        }
        sort_dedup_occurrences(&mut occurrences);
        Some(occurrences)
    }

    fn definition_paths(
        &self,
        from_path: &Path,
        name: &Name,
        cutoff: usize,
        local_family: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<BTreeSet<PathBuf>> {
        let mut inherited_cache = BTreeMap::new();
        self.definition_paths_with_cache(
            from_path,
            name,
            cutoff,
            local_family,
            &mut inherited_cache,
            is_cancelled,
        )
    }

    fn definition_paths_with_cache(
        &self,
        from_path: &Path,
        name: &Name,
        cutoff: usize,
        local_family: bool,
        inherited_cache: &mut BTreeMap<PathBuf, BTreeSet<PathBuf>>,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<BTreeSet<PathBuf>> {
        let mut paths = self.descendants_before(
            from_path,
            cutoff,
            local_family || cutoff == usize::MAX,
            is_cancelled,
        )?;

        if !self.paths_have_definition(&paths, name, is_cancelled)? {
            paths =
                self.inherited_definition_paths(from_path, name, inherited_cache, is_cancelled)?;
        }
        let mut definitions = BTreeSet::new();
        for path in paths {
            if is_cancelled() {
                return None;
            }
            if self
                .files
                .get(&path)
                .is_some_and(|facts| facts.has_definition(name))
            {
                definitions.insert(path);
            }
        }
        Some(definitions)
    }

    fn inherited_definition_paths(
        &self,
        path: &Path,
        name: &Name,
        cache: &mut BTreeMap<PathBuf, BTreeSet<PathBuf>>,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<BTreeSet<PathBuf>> {
        if let Some(cached) = cache.get(path) {
            return Some(cached.clone());
        }

        let mut inherited = BTreeSet::new();
        let mut visited = BTreeSet::from([path.to_path_buf()]);
        let mut pending = vec![path.to_path_buf()];
        while let Some(current) = pending.pop() {
            if is_cancelled() {
                return None;
            }
            let Some(sources) = self.incoming.get(&current) else {
                continue;
            };
            for source in sources {
                if is_cancelled() {
                    return None;
                }
                let candidates =
                    self.descendants_before(&source.parent, source.cutoff, true, is_cancelled)?;
                let mut definitions = BTreeSet::new();
                for candidate in candidates {
                    if is_cancelled() {
                        return None;
                    }
                    if self.files.get(&candidate).is_some_and(|facts| {
                        if candidate == source.parent {
                            facts.has_definition_before(name, source.cutoff)
                        } else {
                            facts.has_definition(name)
                        }
                    }) {
                        definitions.insert(candidate);
                    }
                }
                if definitions.is_empty() && visited.insert(source.parent.clone()) {
                    pending.push(source.parent.clone());
                }
                inherited.extend(definitions);
            }
        }
        cache.insert(path.to_path_buf(), inherited.clone());
        Some(inherited)
    }

    fn descendants_before(
        &self,
        root: &Path,
        cutoff: usize,
        include_root: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<BTreeSet<PathBuf>> {
        let mut paths = BTreeSet::new();
        let mut active = BTreeSet::new();
        self.collect_descendants(
            root,
            cutoff,
            include_root,
            &mut active,
            &mut paths,
            is_cancelled,
        )?;
        Some(paths)
    }

    fn collect_descendants(
        &self,
        path: &Path,
        cutoff: usize,
        include_path: bool,
        active: &mut BTreeSet<PathBuf>,
        paths: &mut BTreeSet<PathBuf>,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        if is_cancelled() {
            return None;
        }
        if !active.insert(path.to_path_buf()) {
            return Some(());
        }
        if include_path {
            paths.insert(path.to_path_buf());
        }
        let Some(facts) = self.files.get(path) else {
            active.remove(path);
            return Some(());
        };
        if facts.has_ambiguous_top_level_source_before(cutoff) {
            active.remove(path);
            return None;
        }
        let targets = facts
            .unconditional_top_level_source_paths(cutoff)
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        for target in targets {
            if is_cancelled() {
                active.remove(path);
                return None;
            }
            paths.insert(target.clone());
            self.collect_descendants(&target, usize::MAX, true, active, paths, is_cancelled)?;
        }
        active.remove(path);
        Some(())
    }

    fn environment_paths_from(
        &self,
        paths: &BTreeSet<PathBuf>,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<BTreeSet<PathBuf>> {
        let mut ancestors = paths.clone();
        let mut pending = paths.iter().cloned().collect::<Vec<_>>();
        while let Some(target) = pending.pop() {
            if is_cancelled() {
                return None;
            }
            let Some(sources) = self.incoming.get(&target) else {
                continue;
            };
            for source in sources {
                if is_cancelled() {
                    return None;
                }
                if ancestors.insert(source.parent.clone()) {
                    pending.push(source.parent.clone());
                }
            }
        }

        let mut environment = ancestors.clone();
        let mut pending = ancestors.into_iter().collect::<Vec<_>>();
        while let Some(parent) = pending.pop() {
            if is_cancelled() {
                return None;
            }
            let Some(facts) = self.files.get(&parent) else {
                continue;
            };
            for target in facts
                .resolved_top_level_source_edges(usize::MAX)
                .map(|(path, _)| path)
            {
                if is_cancelled() {
                    return None;
                }
                if environment.insert(target.to_path_buf()) {
                    pending.push(target.to_path_buf());
                }
            }
        }
        Some(environment)
    }

    fn paths_have_definition(
        &self,
        paths: &BTreeSet<PathBuf>,
        name: &Name,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<bool> {
        for path in paths {
            if is_cancelled() {
                return None;
            }
            if self
                .files
                .get(path)
                .is_some_and(|facts| facts.has_definition(name))
            {
                return Some(true);
            }
        }
        Some(false)
    }

    fn definition_occurrences(
        &self,
        paths: &BTreeSet<PathBuf>,
        name: &Name,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<WorkspaceVariableOccurrence>> {
        let mut occurrences = Vec::new();
        for path in paths {
            if is_cancelled() {
                return None;
            }
            let Some(facts) = self.files.get(path) else {
                continue;
            };
            for definition in &facts.definitions {
                if is_cancelled() {
                    return None;
                }
                if definition.name == *name {
                    occurrences.push(WorkspaceVariableOccurrence {
                        path: path.clone(),
                        span: definition.definition_span,
                    });
                }
            }
        }
        sort_dedup_occurrences(&mut occurrences);
        Some(occurrences)
    }
}

pub(crate) fn variable_target(
    model: &SemanticModel,
    target: &EditorSymbolTarget,
) -> Option<WorkspaceVariableTarget> {
    match target {
        EditorSymbolTarget::Binding(binding_id) => {
            let binding = model.binding(*binding_id);
            persistent_file_variable_binding(model, binding).then(|| WorkspaceVariableTarget {
                name: binding.name.clone(),
                cutoff: usize::MAX,
                local_family: true,
            })
        }
        EditorSymbolTarget::Reference(reference_id) => {
            let reference = model.reference(*reference_id);
            if !source_backed_reference(reference.kind, reference.name_span)
                || !valid_variable_name(reference.name.as_str())
            {
                return None;
            }
            let local_family = match model.resolved_binding(*reference_id) {
                Some(binding) if persistent_file_variable_binding(model, binding) => true,
                Some(_) => return None,
                None if model.name_is_predefined_runtime(reference.name.as_str()) => return None,
                None => false,
            };
            Some(WorkspaceVariableTarget {
                name: reference.name.clone(),
                cutoff: if model.enclosing_function_scope(reference.scope).is_some() {
                    usize::MAX
                } else {
                    reference.name_span.start.offset()
                },
                local_family,
            })
        }
        EditorSymbolTarget::FunctionCall(_) | EditorSymbolTarget::RuntimeName(_) => None,
    }
}

fn persistent_file_variable_binding(model: &SemanticModel, binding: &Binding) -> bool {
    !matches!(
        binding.kind,
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref
    ) && !binding.attributes.contains(BindingAttributes::NAMEREF)
        && model.enclosing_function_scope(binding.scope).is_none()
        && model
            .innermost_transient_scope_within_function(binding.scope)
            .is_none()
        && valid_variable_name(binding.name.as_str())
}

fn binding_definition_span(binding: &Binding) -> Span {
    match binding.origin {
        BindingOrigin::Assignment {
            definition_span, ..
        }
        | BindingOrigin::LoopVariable {
            definition_span, ..
        }
        | BindingOrigin::ParameterDefaultAssignment {
            definition_span, ..
        }
        | BindingOrigin::Imported { definition_span }
        | BindingOrigin::FunctionDefinition { definition_span }
        | BindingOrigin::BuiltinTarget {
            definition_span, ..
        }
        | BindingOrigin::ArithmeticAssignment {
            definition_span, ..
        }
        | BindingOrigin::Declaration { definition_span }
        | BindingOrigin::Nameref { definition_span } => definition_span,
    }
}

fn binding_occurrence_span(binding: &Binding) -> Span {
    match binding.origin {
        BindingOrigin::ParameterDefaultAssignment { target_span, .. }
        | BindingOrigin::ArithmeticAssignment { target_span, .. } => target_span,
        _ => binding.span,
    }
}

fn source_backed_reference(kind: ReferenceKind, span: Span) -> bool {
    span.start.offset() < span.end.offset()
        && !matches!(
            kind,
            ReferenceKind::DeclarationName
                | ReferenceKind::ImplicitRead
                | ReferenceKind::RequiredRead
        )
}

fn valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn sort_dedup_occurrences(occurrences: &mut Vec<WorkspaceVariableOccurrence>) {
    occurrences.sort_by(|left, right| {
        left.path.cmp(&right.path).then_with(|| {
            (left.span.start.offset(), left.span.end.offset())
                .cmp(&(right.span.start.offset(), right.span.end.offset()))
        })
    });
    occurrences.dedup();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use shuck_ast::Position;

    use super::*;

    fn span(offset: usize) -> Span {
        Span::from_positions(
            Position::at(1, offset + 1, offset),
            Position::at(1, offset + 2, offset + 1),
        )
    }

    fn definition(name: &str, offset: usize) -> VariableDefinition {
        VariableDefinition {
            name: Name::from(name),
            definition_span: span(offset),
            occurrence_span: span(offset),
        }
    }

    fn reference(name: &str, offset: usize) -> VariableReference {
        VariableReference {
            name: Name::from(name),
            occurrence_span: span(offset),
            cutoff: offset,
            local_family: false,
        }
    }

    fn source_effect(path: Option<&str>, offset: usize, conditional: bool) -> CallFactSourceEffect {
        CallFactSourceEffect {
            path: path.map(PathBuf::from),
            span: span(offset),
            conditional,
            enclosing_function: None,
            persistent: true,
        }
    }

    fn target(name: &str, cutoff: usize, local_family: bool) -> WorkspaceVariableTarget {
        WorkspaceVariableTarget {
            name: Name::from(name),
            cutoff,
            local_family,
        }
    }

    #[test]
    fn conditional_and_dynamic_sources_make_definition_queries_ambiguous() {
        let never_cancelled = || false;
        let cases = [
            vec![source_effect(Some("vars.sh"), 10, true)],
            vec![
                source_effect(Some("vars.sh"), 5, false),
                source_effect(None, 10, false),
            ],
        ];
        for source_effects in cases {
            let mut index = WorkspaceVariableIndex::default();
            index.insert_facts(
                PathBuf::from("vars.sh"),
                FileVariableFacts {
                    definitions: vec![definition("SHARED", 1)],
                    ..FileVariableFacts::default()
                },
            );
            index.insert_facts(
                PathBuf::from("main.sh"),
                FileVariableFacts {
                    references: vec![reference("SHARED", 20)],
                    source_effects,
                    ..FileVariableFacts::default()
                },
            );

            assert!(
                index
                    .definitions(
                        Path::new("main.sh"),
                        &target("SHARED", 20, false),
                        &never_cancelled,
                    )
                    .is_none()
            );
            assert!(
                index
                    .references(
                        Path::new("vars.sh"),
                        &target("SHARED", usize::MAX, true),
                        false,
                        &never_cancelled,
                    )
                    .is_none()
            );
        }
    }

    #[test]
    fn cyclic_source_graphs_do_not_poison_inherited_definition_caches() {
        let never_cancelled = || false;
        let mut index = WorkspaceVariableIndex::default();
        index.insert_facts(
            PathBuf::from("entry.sh"),
            FileVariableFacts {
                definitions: vec![definition("SHARED", 1)],
                source_effects: vec![source_effect(Some("a.sh"), 10, false)],
                ..FileVariableFacts::default()
            },
        );
        index.insert_facts(
            PathBuf::from("a.sh"),
            FileVariableFacts {
                references: vec![reference("SHARED", 20)],
                source_effects: vec![source_effect(Some("b.sh"), 10, false)],
                ..FileVariableFacts::default()
            },
        );
        index.insert_facts(
            PathBuf::from("b.sh"),
            FileVariableFacts {
                references: vec![reference("SHARED", 20)],
                source_effects: vec![source_effect(Some("a.sh"), 10, false)],
                ..FileVariableFacts::default()
            },
        );

        let references = index
            .references(
                Path::new("entry.sh"),
                &target("SHARED", usize::MAX, true),
                false,
                &never_cancelled,
            )
            .expect("the deterministic cycle should resolve");
        let paths = references
            .into_iter()
            .map(|reference| reference.path)
            .collect::<BTreeSet<_>>();

        assert_eq!(
            paths,
            BTreeSet::from([PathBuf::from("a.sh"), PathBuf::from("b.sh")])
        );
    }

    #[test]
    fn graph_traversal_observes_cancellation() {
        let mut index = WorkspaceVariableIndex::default();
        for file in 0..64 {
            let next = (file < 63)
                .then(|| source_effect(Some(&format!("{next}.sh", next = file + 1)), 1, false));
            index.insert_facts(
                PathBuf::from(format!("{file}.sh")),
                FileVariableFacts {
                    definitions: (file == 63)
                        .then(|| definition("SHARED", 2))
                        .into_iter()
                        .collect(),
                    source_effects: next.into_iter().collect(),
                    ..FileVariableFacts::default()
                },
            );
        }

        let polls = Cell::new(0);
        let result = index.definitions(
            Path::new("0.sh"),
            &target("SHARED", usize::MAX, true),
            &|| {
                let next = polls.get() + 1;
                polls.set(next);
                next > 10
            },
        );

        assert!(result.is_none());
        assert!(polls.get() > 10);
    }
}
