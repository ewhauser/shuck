use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{ArenaFileCommandKind, Name, Span};
use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, DeclarationBuiltin, DeclarationOperand,
    Reference, ReferenceId, ScopeId,
};

use crate::{Checker, LinterFacts, Rule, Violation, WordQuote, facts::CommandId};

pub struct QuotedBashSource;

impl Violation for QuotedBashSource {
    fn rule() -> Rule {
        Rule::QuotedBashSource
    }

    fn message(&self) -> String {
        "array references should choose an explicit element or selector".to_owned()
    }
}

pub fn quoted_bash_source(checker: &mut Checker) {
    let semantic = checker.semantic();
    let candidate_spans = checker
        .facts()
        .plain_unindexed_reference_spans()
        .iter()
        .copied()
        .map(span_key)
        .collect::<FxHashSet<_>>();
    let candidate_references = semantic
        .references()
        .iter()
        .filter(|reference| candidate_spans.contains(&span_key(reference.span)))
        .collect::<Vec<_>>();
    let mut context =
        QuotedBashSourceContext::new(checker.facts(), semantic, &candidate_references);
    let spans = candidate_references
        .into_iter()
        .filter(|reference| context.reference_is_array_like(reference))
        .map(|reference| reference.span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedBashSource);
}

struct QuotedBashSourceContext<'a, 'src> {
    facts: &'a LinterFacts<'src>,
    semantic: &'a shuck_semantic::SemanticModel,
    local_declarations: LocalDeclarationIndex,
    innermost_command_ids_by_offset: FxHashMap<usize, Option<CommandId>>,
    simple_command_ancestors_by_offset: FxHashMap<usize, Vec<SimpleCommandAncestor>>,
    same_command_writers_by_name: FxHashMap<Name, Vec<BindingId>>,
    presence_test_ends_by_name_binding: FxHashMap<Name, FxHashMap<Option<BindingId>, Vec<usize>>>,
    resolved_binding_ids: FxHashMap<ReferenceId, Option<BindingId>>,
    binding_inherits_indexed_array_type_cache: FxHashMap<BindingId, bool>,
    binding_has_prior_local_barrier_cache: FxHashMap<BindingId, bool>,
    binding_is_append_declaration_cache: FxHashMap<BindingId, bool>,
    binding_reset_by_name_only_before_cache: FxHashMap<(BindingId, usize), bool>,
}

impl<'a, 'src> QuotedBashSourceContext<'a, 'src> {
    fn new(
        facts: &'a LinterFacts<'src>,
        semantic: &'a shuck_semantic::SemanticModel,
        candidate_references: &[&Reference],
    ) -> Self {
        let mut command_query_offsets = candidate_references
            .iter()
            .map(|reference| reference.span.start.offset)
            .collect::<Vec<_>>();
        command_query_offsets.extend(
            semantic
                .bindings()
                .iter()
                .filter(|binding| {
                    matches!(
                        binding.kind,
                        BindingKind::ArrayAssignment
                            | BindingKind::MapfileTarget
                            | BindingKind::ReadTarget
                    )
                })
                .map(|binding| binding.span.start.offset),
        );

        Self {
            facts,
            semantic,
            local_declarations: LocalDeclarationIndex::build(semantic),
            innermost_command_ids_by_offset: build_innermost_command_ids_by_offset(
                facts.commands(),
                command_query_offsets,
            ),
            simple_command_ancestors_by_offset: FxHashMap::default(),
            same_command_writers_by_name: FxHashMap::default(),
            presence_test_ends_by_name_binding: FxHashMap::default(),
            resolved_binding_ids: FxHashMap::default(),
            binding_inherits_indexed_array_type_cache: FxHashMap::default(),
            binding_has_prior_local_barrier_cache: FxHashMap::default(),
            binding_is_append_declaration_cache: FxHashMap::default(),
            binding_reset_by_name_only_before_cache: FxHashMap::default(),
        }
    }

    fn reference_is_array_like(&mut self, reference: &Reference) -> bool {
        if self.semantic.is_guarded_parameter_reference(reference.id)
            || self.reference_has_prior_presence_test(reference)
            || self.reference_reads_into_same_name_array_writer(reference)
        {
            return false;
        }
        if let Some(binding) = self.semantic.resolved_binding(reference.id)
            && self.semantic.binding_visible_at(binding.id, reference.span)
            && !binding_is_array_like(binding)
            && !self.binding_inherits_indexed_array_type(binding)
            && (binding_resets_indexed_array_type(binding)
                || self.binding_has_prior_local_barrier(binding))
        {
            return false;
        }

        if is_bash_runtime_array_name(reference.name.as_str()) {
            return true;
        }

        let mut binding_ids = Vec::new();
        let mut seen = FxHashSet::default();
        if let Some(binding) = self.semantic.resolved_binding(reference.id)
            && !binding_is_array_like(binding)
            && seen.insert(binding.id)
        {
            binding_ids.push(binding.id);
        }
        for binding_id in candidate_binding_ids_for_reference(self.semantic, reference) {
            if seen.insert(binding_id) {
                binding_ids.push(binding_id);
            }
        }

        binding_ids.into_iter().any(|binding_id| {
            let binding = self.semantic.binding(binding_id);
            !self.binding_reset_by_name_only_declaration_before(binding, reference.span)
                && (binding_is_array_like(binding)
                    || self.binding_inherits_indexed_array_type(binding))
        })
    }

    fn binding_inherits_indexed_array_type(&mut self, binding: &Binding) -> bool {
        if let Some(cached) = self
            .binding_inherits_indexed_array_type_cache
            .get(&binding.id)
            .copied()
        {
            return cached;
        }

        let inherited = if binding_resets_indexed_array_type(binding) {
            false
        } else {
            let initialized_scalar_declaration =
                matches!(binding.kind, BindingKind::Declaration(_))
                    && binding
                        .attributes
                        .contains(BindingAttributes::DECLARATION_INITIALIZED)
                    && !binding
                        .attributes
                        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
            let append_declaration = self.binding_is_append_declaration(binding);
            let prior_local_barrier = self.binding_has_prior_local_barrier(binding);
            let prior_bindings = self
                .semantic
                .bindings_for(&binding.name)
                .iter()
                .copied()
                .filter(|candidate_id| {
                    let candidate = self.semantic.binding(*candidate_id);
                    let same_scope_candidate_allowed =
                        !initialized_scalar_declaration || append_declaration;
                    candidate.span.start.offset < binding.span.start.offset
                        && ((candidate.scope != binding.scope && !prior_local_barrier)
                            || same_scope_candidate_allowed)
                        && !self
                            .binding_reset_by_name_only_declaration_before(candidate, binding.span)
                })
                .collect::<Vec<_>>();

            let mut inherited = false;
            for candidate_id in prior_bindings.into_iter().rev() {
                let candidate = self.semantic.binding(candidate_id);
                if binding_resets_indexed_array_type(candidate) {
                    inherited = false;
                    break;
                }
                if binding_is_sticky_indexed_array(candidate) {
                    inherited = true;
                    break;
                }
            }
            inherited
        };

        self.binding_inherits_indexed_array_type_cache
            .insert(binding.id, inherited);
        inherited
    }

    fn reference_reads_into_same_name_array_writer(&mut self, reference: &Reference) -> bool {
        let candidate_bindings = self
            .same_command_candidate_writer_bindings(&reference.name)
            .to_vec();
        candidate_bindings.into_iter().any(|binding_id| {
            let binding = self.semantic.binding(binding_id);
            binding.span.start.offset <= reference.span.start.offset
                && self
                    .same_simple_command_is_assignment_only(binding.span, reference.span)
                    .is_some_and(|assignment_only| {
                        binding_suppresses_same_command_array_read(binding, assignment_only)
                    })
        })
    }

    fn reference_has_prior_presence_test(&mut self, reference: &Reference) -> bool {
        if loop_header_word_quote(self.facts, reference.span)
            .is_some_and(|quote| quote != WordQuote::Unquoted)
        {
            return false;
        }

        let reference_binding = self.resolved_binding_id(reference.id);
        self.presence_test_ends_by_binding(&reference.name)
            .get(&reference_binding)
            .is_some_and(|ends| ends.partition_point(|end| *end < reference.span.start.offset) > 0)
    }

    fn presence_test_ends_by_binding(
        &mut self,
        name: &Name,
    ) -> &FxHashMap<Option<BindingId>, Vec<usize>> {
        if !self.presence_test_ends_by_name_binding.contains_key(name) {
            let mut by_binding = FxHashMap::<Option<BindingId>, Vec<usize>>::default();

            for test in self.facts.presence_test_references(name) {
                let binding_id = self.resolved_binding_id(test.reference_id());
                by_binding
                    .entry(binding_id)
                    .or_default()
                    .push(test.command_span().end.offset);
            }

            for test in self.facts.presence_test_names(name) {
                let binding_id =
                    resolve_binding_visible_at(self.semantic, name, test.tested_span());
                by_binding
                    .entry(binding_id)
                    .or_default()
                    .push(test.command_span().end.offset);
            }

            for ends in by_binding.values_mut() {
                ends.sort_unstable();
                ends.dedup();
            }

            self.presence_test_ends_by_name_binding
                .insert(name.clone(), by_binding);
        }

        self.presence_test_ends_by_name_binding
            .get(name)
            .expect("presence-test bindings should be cached")
    }

    fn resolved_binding_id(&mut self, reference_id: ReferenceId) -> Option<BindingId> {
        *self
            .resolved_binding_ids
            .entry(reference_id)
            .or_insert_with(|| {
                self.semantic
                    .resolved_binding(reference_id)
                    .map(|binding| binding.id)
            })
    }

    fn same_command_candidate_writer_bindings(&mut self, name: &Name) -> &[BindingId] {
        self.same_command_writers_by_name
            .entry(name.clone())
            .or_insert_with(|| {
                let mut bindings = self
                    .semantic
                    .bindings_for(name)
                    .iter()
                    .copied()
                    .filter(|binding_id| {
                        let binding = self.semantic.binding(*binding_id);
                        matches!(
                            binding.kind,
                            BindingKind::ArrayAssignment
                                | BindingKind::MapfileTarget
                                | BindingKind::ReadTarget
                        )
                    })
                    .collect::<Vec<_>>();
                bindings.sort_unstable_by_key(|binding_id| {
                    self.semantic.binding(*binding_id).span.start.offset
                });
                bindings
            })
    }

    fn simple_command_ancestors(&mut self, offset: usize) -> &[SimpleCommandAncestor] {
        self.simple_command_ancestors_by_offset
            .entry(offset)
            .or_insert_with(|| {
                let mut ancestors = Vec::new();
                let mut current = self
                    .innermost_command_ids_by_offset
                    .get(&offset)
                    .copied()
                    .flatten();
                while let Some(command_id) = current {
                    let command = self.facts.command(command_id);
                    if command.command_kind() == ArenaFileCommandKind::Simple {
                        ancestors.push(SimpleCommandAncestor {
                            id: command_id,
                            assignment_only: command.literal_name() == Some(""),
                        });
                    }
                    current = self.facts.command_parent_id(command_id);
                }
                ancestors
            })
    }

    fn same_simple_command_is_assignment_only(
        &mut self,
        binding_span: Span,
        reference_span: Span,
    ) -> Option<bool> {
        let binding_ancestors = self
            .simple_command_ancestors(binding_span.start.offset)
            .to_vec();
        let reference_ancestors = self
            .simple_command_ancestors(reference_span.start.offset)
            .to_vec();

        for reference_ancestor in reference_ancestors {
            if let Some(binding_ancestor) = binding_ancestors
                .iter()
                .find(|binding_ancestor| binding_ancestor.id == reference_ancestor.id)
            {
                return Some(binding_ancestor.assignment_only);
            }
        }

        None
    }

    fn binding_reset_by_name_only_declaration_before(
        &mut self,
        binding: &Binding,
        at: Span,
    ) -> bool {
        *self
            .binding_reset_by_name_only_before_cache
            .entry((binding.id, at.start.offset))
            .or_insert_with(|| {
                self.local_declarations
                    .name_only_local_declarations_for(binding.scope, &binding.name)
                    .iter()
                    .any(|span| {
                        span.start.offset > binding.span.start.offset
                            && span.end.offset < at.start.offset
                    })
            })
    }

    fn binding_has_prior_local_barrier(&mut self, binding: &Binding) -> bool {
        *self
            .binding_has_prior_local_barrier_cache
            .entry(binding.id)
            .or_insert_with(|| {
                self.local_declarations
                    .local_declarations_for(binding.scope, &binding.name)
                    .iter()
                    .any(|span| span.end.offset < binding.span.start.offset)
            })
    }

    fn binding_is_append_declaration(&mut self, binding: &Binding) -> bool {
        *self
            .binding_is_append_declaration_cache
            .entry(binding.id)
            .or_insert_with(|| {
                self.local_declarations.is_local_append_declaration(
                    binding.scope,
                    &binding.name,
                    binding.span,
                )
            })
    }
}

#[derive(Clone, Copy)]
struct SimpleCommandAncestor {
    id: CommandId,
    assignment_only: bool,
}

struct LocalDeclarationIndex {
    local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    name_only_local_declarations_by_scope_name: FxHashMap<(ScopeId, Name), Vec<Span>>,
    append_local_declaration_spans: FxHashSet<(ScopeId, Name, usize, usize)>,
}

impl LocalDeclarationIndex {
    fn build(semantic: &shuck_semantic::SemanticModel) -> Self {
        let mut local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut name_only_local_declarations_by_scope_name =
            FxHashMap::<(ScopeId, Name), Vec<Span>>::default();
        let mut append_local_declaration_spans = FxHashSet::default();

        for declaration in semantic.declarations() {
            if !matches!(declaration.builtin, DeclarationBuiltin::Local) {
                continue;
            }

            let scope = semantic.scope_at(declaration.span.start.offset);
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

        Self {
            local_declarations_by_scope_name,
            name_only_local_declarations_by_scope_name,
            append_local_declaration_spans,
        }
    }

    fn local_declarations_for(&self, scope: ScopeId, name: &Name) -> &[Span] {
        self.local_declarations_by_scope_name
            .get(&(scope, name.clone()))
            .map_or(&[], Vec::as_slice)
    }

    fn name_only_local_declarations_for(&self, scope: ScopeId, name: &Name) -> &[Span] {
        self.name_only_local_declarations_by_scope_name
            .get(&(scope, name.clone()))
            .map_or(&[], Vec::as_slice)
    }

    fn is_local_append_declaration(&self, scope: ScopeId, name: &Name, span: Span) -> bool {
        self.append_local_declaration_spans.contains(&(
            scope,
            name.clone(),
            span.start.offset,
            span.end.offset,
        ))
    }
}

#[derive(Clone, Copy)]
struct OpenCommand {
    end_offset: usize,
    id: CommandId,
}

fn build_innermost_command_ids_by_offset(
    commands: crate::facts::CommandFacts<'_, '_>,
    mut offsets: Vec<usize>,
) -> FxHashMap<usize, Option<CommandId>> {
    if offsets.is_empty() {
        return FxHashMap::default();
    }

    offsets.sort_unstable();
    offsets.dedup();

    let mut command_spans = commands
        .iter()
        .map(|command| (command.span(), command.id()))
        .collect::<Vec<_>>();
    if command_spans
        .windows(2)
        .any(|window| compare_command_offset_entries(window[0], window[1]).is_gt())
    {
        command_spans.sort_unstable_by(|left, right| compare_command_offset_entries(*left, *right));
    }

    let mut command_ids_by_offset = FxHashMap::default();
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some((span, id)) = command_spans.get(next_command).copied() {
            if span.start.offset > offset {
                break;
            }

            pop_finished_commands(&mut active_commands, span.start.offset);
            active_commands.push(OpenCommand {
                end_offset: span.end.offset,
                id,
            });
            next_command += 1;
        }

        pop_finished_commands(&mut active_commands, offset);
        command_ids_by_offset.insert(offset, active_commands.last().map(|command| command.id));
    }

    command_ids_by_offset
}

fn compare_command_offset_entries(
    (left_span, _left_id): (Span, CommandId),
    (right_span, _right_id): (Span, CommandId),
) -> std::cmp::Ordering {
    left_span
        .start
        .offset
        .cmp(&right_span.start.offset)
        .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
}

fn pop_finished_commands(active_commands: &mut Vec<OpenCommand>, offset: usize) {
    while active_commands
        .last()
        .is_some_and(|command| command.end_offset < offset)
    {
        active_commands.pop();
    }
}

fn span_is_within(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn span_key(span: Span) -> (usize, usize) {
    (span.start.offset, span.end.offset)
}

fn binding_is_array_like(binding: &Binding) -> bool {
    let declared_array = binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
    (declared_array && !is_uninitialized_local_array_declaration(binding))
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
}

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

fn binding_is_sticky_indexed_array(binding: &Binding) -> bool {
    !is_uninitialized_local_array_declaration(binding)
        && (binding.attributes.contains(BindingAttributes::ARRAY)
            || matches!(
                binding.kind,
                BindingKind::ArrayAssignment | BindingKind::MapfileTarget
            ))
}

fn is_uninitialized_local_array_declaration(binding: &Binding) -> bool {
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

fn loop_header_word_quote(facts: &LinterFacts<'_>, span: Span) -> Option<WordQuote> {
    facts
        .for_headers()
        .iter()
        .flat_map(|header| header.words().iter())
        .chain(
            facts
                .select_headers()
                .iter()
                .flat_map(|header| header.words().iter()),
        )
        .find(|word| span_is_within(word.span(), span))
        .map(|word| word.classification().quote)
}

fn binding_suppresses_same_command_array_read(binding: &Binding, assignment_only: bool) -> bool {
    matches!(binding.kind, BindingKind::MapfileTarget)
        || (matches!(binding.kind, BindingKind::ReadTarget)
            && binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::ArrayAssignment) && assignment_only)
}

fn resolve_binding_visible_at(
    semantic: &shuck_semantic::SemanticModel,
    name: &shuck_ast::Name,
    tested_span: Span,
) -> Option<BindingId> {
    semantic
        .bindings_for(name)
        .iter()
        .copied()
        .rev()
        .find(|binding_id| semantic.binding_visible_at(*binding_id, tested_span))
}

fn candidate_binding_ids_for_reference(
    semantic: &shuck_semantic::SemanticModel,
    reference: &Reference,
) -> Vec<BindingId> {
    let all_bindings = semantic.bindings_for(&reference.name);
    let binding_ids = semantic
        .ancestor_scopes(reference.scope)
        .filter_map(|scope| {
            all_bindings.iter().copied().rev().find(|binding_id| {
                let binding = semantic.binding(*binding_id);
                binding.scope == scope && semantic.binding_visible_at(*binding_id, reference.span)
            })
        })
        .collect::<Vec<_>>();
    if !binding_ids.is_empty() {
        return binding_ids;
    }

    semantic
        .ancestor_scopes(reference.scope)
        .skip(1)
        .filter_map(|scope| {
            all_bindings.iter().copied().rev().find(|binding_id| {
                let binding = semantic.binding(*binding_id);
                binding.scope == scope && semantic.binding_visible_at(*binding_id, reference.span)
            })
        })
        .chain(all_bindings.iter().copied().filter(|binding_id| {
            let binding = semantic.binding(*binding_id);
            binding.scope != reference.scope
                && binding.span.start.offset < reference.span.start.offset
        }))
        .collect::<FxHashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

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

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, lint_file_at_path};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn reports_plain_unindexed_array_references() {
        let source = "\
#!/bin/bash
arr=(one two)
declare -A map=([key]=value)
read -ra read_items
mapfile map_items
x=\"$BASH_SOURCE\"
y=\"${BASH_SOURCE}\"
printf '%s\\n' $arr \"${arr}\" pre${arr}post \"$map\" \"$read_items\" \"$map_items\"
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
if [[ \"$BASH_SOURCE\" == foo ]]; then :; fi
for item in \"$BASH_SOURCE\"; do
  :
done
cat <<EOF
$arr
${arr}
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$BASH_SOURCE",
                "${BASH_SOURCE}",
                "$arr",
                "${arr}",
                "${arr}",
                "$map",
                "$read_items",
                "$map_items",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$arr",
                "${arr}",
            ]
        );
    }

    #[test]
    fn ignores_scalar_indexed_selector_and_non_access_forms() {
        let source = "\
#!/bin/bash
name=scalar
MAPFILE=scalar
arr=(one two)
x=$BASH_SOURCE
y=${BASH_SOURCE}
z=\"${BASH_SOURCE[0]}\"
q=\"${BASH_SOURCE[@]}\"
r=\"${BASH_SOURCE[*]}\"
s=\"${BASH_SOURCE%/*}\"
t=\"${BASH_SOURCE:-fallback}\"
v=\"${BASH_SOURCE-}\"
u=\"\\$BASH_SOURCE\"
printf '%s\\n' \"$name\" \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${arr%one}\" \"${arr:-fallback}\"
only_declared() {
  local -a local_array
  printf '%s\\n' \"$local_array\"
}
for item in \"$@\"; do
  item=($item)
done
read -ra read_items <<<\"$read_items\"
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE", "${BASH_SOURCE}", "$MAPFILE"]
        );
    }

    #[test]
    fn ignores_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
for item in $filelist; do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nested_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
tests=\"$(for item in $filelist; do
  :
done)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn string_binary_conditions_do_not_count_as_presence_guards() {
        let source = "\
#!/bin/bash
apt_pkgs=()
for pkg in \"$@\"; do
  pkg=(one two three)
  if [[ \"${pkg[0]}\" == one ]]; then
    :
  fi
  if hasPackage \"$pkg\"; then
    apt_pkgs+=(\"$pkg\")
  fi
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pkg", "$pkg"]
        );
    }

    #[test]
    fn unset_does_not_reset_array_type() {
        let source = "\
#!/bin/bash
cleared_array=(one two)
unset cleared_array
cleared_array=scalar
printf '%s\\n' \"$cleared_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$cleared_array"]
        );
    }

    #[test]
    fn target_rebindings_reset_inherited_array_type() {
        let source = "\
#!/bin/bash
loop_value=(one two)
for loop_value in one two; do
  printf '%s\\n' \"$loop_value\"
done
read_value=(one two)
read read_value <<<input
printf '%s\\n' \"$read_value\"
printf_value=(one two)
printf -v printf_value '%s' input
printf '%s\\n' \"$printf_value\"
local_reset() {
  local local_value=(one two)
  local local_value
  printf '%s\\n' \"$local_value\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unbound_runtime_arrays_without_bash_prelude() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$BASH_SOURCE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE"]
        );
    }

    #[test]
    fn reports_runtime_array_names_even_after_scalar_rebinding() {
        let source = "\
#!/bin/bash
MAPFILE=scalar
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$MAPFILE"]
        );
    }

    #[test]
    fn array_declarations_stay_sticky_through_plain_assignments() {
        let source = "\
#!/bin/bash
declare -a additional_packages
additional_packages=$1
split_string ${additional_packages}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${additional_packages}"]
        );
    }

    #[test]
    fn later_presence_guards_only_suppress_the_same_binding() {
        let source = "\
#!/bin/bash
foo=scalar
[ -n \"$foo\" ]
foo=(one two)
printf '%s\\n' \"$foo\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn variable_set_presence_guards_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
[[ -v arr ]]
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn variable_set_presence_guards_do_not_cross_rebindings() {
        let source = "\
#!/bin/bash
arr=scalar
[[ -v arr ]]
arr=(one two)
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn prior_presence_guards_in_sibling_case_arms_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
f() {
  local dir
  case \"$1\" in
    up) dir=(\"Up\");;
  esac
  case \"$2\" in
    hat)
      [[ -n \"$dir\" ]]
      ;;
    *)
      [[ \"$dir\" == \"Up\" || \"$dir\" == \"Left\" ]]
      ;;
  esac
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir"]
        );
    }

    #[test]
    fn attribute_only_declarations_keep_array_type() {
        let source = "\
#!/bin/bash
arr=(one two)
readonly arr
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn function_local_declare_arrays_still_warn() {
        let source = "\
#!/bin/bash
f() {
  declare -a items
  printf '%s\\n' \"$items\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$items"]
        );
    }

    #[test]
    fn nested_command_substitution_presence_tests_do_not_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
[ -n \"$(printf '%s' \"$arr\")\" ]
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr", "$arr"]
        );
    }

    #[test]
    fn presence_tests_inside_command_substitutions_suppress_later_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
out=$( [ -n \"$arr\" ]; printf x )
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn same_command_prefix_array_assignments_still_warn() {
        let source = "\
#!/bin/bash
arr=(old1 old2)
arr=(new1 new2) printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn read_option_values_do_not_become_array_targets() {
        let source = "\
#!/bin/bash
delimiter=:
read -d delimiter -a arr <<<\":\"
printf '%s\\n' \"$delimiter\"
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn mapfile_option_values_do_not_become_array_targets() {
        let source = "\
#!/bin/bash
callback=scalar
mapfile -C callback -c 1 lines < <(printf '%s\\n' value)
printf '%s\\n' \"$callback\"
printf '%s\\n' \"$lines\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$lines"]
        );
    }

    #[test]
    fn local_scalar_assignments_do_not_inherit_outer_array_bindings() {
        let source = "\
#!/bin/bash
declare -a ids
ids=()
set_to_liked() {
  local ids
  { local IFS=','; ids=\"$*\"; }
  if [ -z \"$ids\" ]; then
    return
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn printf_targets_after_local_declarations_do_not_inherit_outer_arrays() {
        let source = "\
#!/bin/bash
args=(\"$@\")
f() {
  local args
  printf -v args '%q ' \"$@\"
  printf '%s\\n' \"$args\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn local_append_declarations_keep_array_type() {
        let source = "\
#!/bin/bash
f() {
  local DOKKU_LOGS_CMD=()
  DOKKU_LOGS_CMD+=\"(cmd)\"
  local DOKKU_LOGS_CMD+=\"; \"
  bash -c \"($DOKKU_LOGS_CMD)\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$DOKKU_LOGS_CMD"]
        );
    }

    #[test]
    fn ignores_references_inside_own_array_assignment() {
        let source = "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_references_inside_same_name_array_readers() {
        let source = "\
#!/bin/bash
read -r -a key_value <<<\"$(printf '%s\\n' \"$key_value\")\"
mapfile -t ports_configured < <(printf '%s\\n' \"${ports_configured}\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_bindings_reset_inherited_array_type() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(. ./helper.sh; printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
",
        )
        .unwrap();
        fs::write(&helper, "TERMUX_PKG_VERSION=helper\n").unwrap();

        let source = fs::read_to_string(&main).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let diagnostics = lint_file_at_path(
            &output.file,
            &source,
            &indexer,
            &LinterSettings::for_rule(Rule::QuotedBashSource),
            None,
            Some(&main),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn follows_prior_visible_array_bindings() {
        let source = "\
#!/bin/bash
before_use() {
  printf '%s\\n' \"$future_array\"
}
future_array=(one two)
after_use() {
  printf '%s\\n' \"$future_array\"
}
former_array=(one two)
former_array=scalar
printf '%s\\n' \"$former_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$future_array", "$former_array"]
        );
    }

    #[test]
    fn follows_prior_array_bindings_by_source_order() {
        let source = "\
#!/bin/bash
first_function() {
  target=(one two)
}
second_function() {
  local target=$1
  printf '%s\\n' \"$target\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn reports_runtime_arrays_inside_assign_default_and_error_operands() {
        let source = "\
#!/bin/bash
: ${PROG:=$(basename ${BASH_SOURCE})}
local PATTERN=${2:?$FUNCNAME: a pattern is required}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${BASH_SOURCE}", "$FUNCNAME"]
        );
    }
}
