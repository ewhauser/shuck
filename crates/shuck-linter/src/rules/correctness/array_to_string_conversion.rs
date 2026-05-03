use std::collections::{HashMap, HashSet};

use shuck_ast::Name;
use shuck_semantic::{
    Binding, BindingAttributes, BindingKind, Declaration, DeclarationBuiltin, DeclarationOperand,
    ScopeId,
};

use crate::{Checker, ComparableNameUseKind, Rule, ShellDialect, Violation, WrapperKind};

use super::variable_reference_common::has_visible_function_name_binding;

pub struct ArrayToStringConversion;

impl Violation for ArrayToStringConversion {
    fn rule() -> Rule {
        Rule::ArrayToStringConversion
    }

    fn message(&self) -> String {
        "a variable name switches from array-like use to a plain scalar assignment".to_owned()
    }
}

pub fn array_to_string_conversion(checker: &mut Checker) {
    let semantic = checker.semantic();
    let mut array_history = HashMap::new();
    let mut bindings = semantic.bindings().iter().collect::<Vec<_>>();
    bindings.sort_by_key(|binding| (binding.span.start.offset, binding.span.end.offset));
    let builtin_history = builtin_array_history(checker);
    let history_events = array_history_events(checker, &bindings, &builtin_history.events);
    let mut next_history_event = 0usize;
    let append_declaration_assignments = append_declaration_assignment_name_spans(checker);

    let spans = bindings
        .into_iter()
        .filter_map(|binding| {
            while let Some(event) = history_events.get(next_history_event) {
                if event.offset > binding.span.start.offset {
                    break;
                }
                push_array_history(&mut array_history, event.name.clone(), event.state);
                next_history_event += 1;
            }

            let name = binding.name.clone();
            let saw_array_history = visible_array_history(&array_history, &name, checker, binding)
                .unwrap_or_else(|| binding_uses_builtin_array_history(checker, binding));

            if declaration_resets_array_history(binding) {
                push_array_history(
                    &mut array_history,
                    name,
                    ArrayHistoryState::from_binding(checker, binding, false, None),
                );
                return None;
            }
            if !binding_can_trigger_array_to_string_conversion(
                binding,
                &append_declaration_assignments,
            ) {
                if binding_establishes_array_history(binding, &builtin_history) {
                    let prior = latest_array_history(&array_history, &name);
                    push_array_history(
                        &mut array_history,
                        name,
                        ArrayHistoryState::from_binding(checker, binding, true, prior),
                    );
                } else if binding_resets_array_history(binding, &builtin_history) {
                    push_array_history(
                        &mut array_history,
                        name,
                        ArrayHistoryState::from_binding(checker, binding, false, None),
                    );
                }
                return None;
            }
            if binding_is_array_like(binding) {
                if binding_establishes_array_history(binding, &builtin_history) {
                    let prior = latest_array_history(&array_history, &name);
                    push_array_history(
                        &mut array_history,
                        name,
                        ArrayHistoryState::from_binding(checker, binding, true, prior),
                    );
                }
                return None;
            }

            checker.facts().binding_value(binding.id)?.scalar_word()?;

            if binding_establishes_array_history(binding, &builtin_history) {
                let prior = latest_array_history(&array_history, &name);
                push_array_history(
                    &mut array_history,
                    name.clone(),
                    ArrayHistoryState::from_binding(checker, binding, true, prior),
                );
            } else if binding_establishes_local_scalar_history(binding) {
                push_array_history(
                    &mut array_history,
                    name.clone(),
                    ArrayHistoryState::from_binding(checker, binding, false, None),
                );
            }

            saw_array_history.then_some(binding.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayToStringConversion);
}

fn push_array_history(
    array_history: &mut HashMap<Name, Vec<ArrayHistoryState>>,
    name: Name,
    state: ArrayHistoryState,
) {
    array_history.entry(name).or_default().push(state);
}

fn latest_array_history(
    array_history: &HashMap<Name, Vec<ArrayHistoryState>>,
    name: &Name,
) -> Option<ArrayHistoryState> {
    array_history
        .get(name)
        .and_then(|history| history.last())
        .copied()
}

fn visible_array_history(
    array_history: &HashMap<Name, Vec<ArrayHistoryState>>,
    name: &Name,
    checker: &Checker<'_>,
    binding: &Binding,
) -> Option<bool> {
    array_history.get(name).and_then(|history| {
        history
            .iter()
            .rev()
            .find(|state| state.visible_to_binding(checker, binding))
            .map(|state| state.array_like)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArrayHistoryEvent {
    offset: usize,
    name: Name,
    state: ArrayHistoryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArrayHistoryState {
    array_like: bool,
    local: bool,
    function_scope: Option<ScopeId>,
}

impl ArrayHistoryState {
    fn from_binding(
        checker: &Checker<'_>,
        binding: &Binding,
        array_like: bool,
        prior: Option<Self>,
    ) -> Self {
        let function_scope = checker.semantic().enclosing_function_scope(binding.scope);
        let inherits_local_history =
            prior.is_some_and(|state| state.local && state.function_scope == function_scope);
        Self {
            array_like,
            local: binding.attributes.contains(BindingAttributes::LOCAL)
                || binding.kind == BindingKind::Declaration(DeclarationBuiltin::Local)
                || inherits_local_history,
            function_scope,
        }
    }

    fn from_offset(checker: &Checker<'_>, offset: usize, array_like: bool, local: bool) -> Self {
        let scope = checker.semantic().scope_at(offset);
        Self {
            array_like,
            local,
            function_scope: checker.semantic().enclosing_function_scope(scope),
        }
    }

    fn visible_to_binding(self, checker: &Checker<'_>, binding: &Binding) -> bool {
        !self.local
            || self.function_scope == checker.semantic().enclosing_function_scope(binding.scope)
    }
}

fn array_history_events(
    checker: &Checker<'_>,
    bindings: &[&Binding],
    builtin_events: &[ArrayHistoryEvent],
) -> Vec<ArrayHistoryEvent> {
    let mut events = builtin_events.to_vec();
    events.extend(presence_test_reset_events(checker, bindings));
    events.extend(name_only_declaration_history_events(checker));
    events.sort_by_key(|event| (event.offset, !event.state.array_like));
    events
}

#[derive(Debug, Default)]
struct BuiltinArrayHistory {
    events: Vec<ArrayHistoryEvent>,
    target_spans: HashSet<(usize, usize)>,
}

impl BuiltinArrayHistory {
    fn contains_target(&self, binding: &Binding) -> bool {
        self.target_spans
            .contains(&(binding.span.start.offset, binding.span.end.offset))
    }
}

fn builtin_array_history(checker: &Checker<'_>) -> BuiltinArrayHistory {
    let mut history = BuiltinArrayHistory::default();
    let mut seen_commands = HashSet::new();

    for binding in checker.semantic().bindings() {
        if !matches!(
            binding.kind,
            BindingKind::ReadTarget | BindingKind::MapfileTarget
        ) {
            continue;
        }
        let Some(command) = binding_command(checker, binding) else {
            continue;
        };
        if !seen_commands.insert(command.id()) {
            continue;
        }
        collect_command_array_history(checker, command, &mut history);
    }

    let events = &mut history.events;
    events.sort_by_key(|event| event.offset);
    history
}

fn collect_command_array_history(
    checker: &Checker<'_>,
    command: crate::facts::commands::CommandFactRef<'_, '_>,
    history: &mut BuiltinArrayHistory,
) {
    if matches!(checker.shell(), ShellDialect::Bash) && command.effective_name_is("read") {
        let Some(read) = command.options().read() else {
            return;
        };
        if command_is_shadowed_function(checker, command) {
            return;
        }
        for target in read.array_target_name_uses() {
            if !matches!(target.kind(), ComparableNameUseKind::Literal) {
                continue;
            }
            history
                .target_spans
                .insert((target.span().start.offset, target.span().end.offset));
            history.events.push(ArrayHistoryEvent {
                offset: target.span().start.offset,
                name: Name::from(target.key().as_str()),
                state: ArrayHistoryState::from_offset(
                    checker,
                    target.span().start.offset,
                    true,
                    false,
                ),
            });
        }
        return;
    }

    if matches!(checker.shell(), ShellDialect::Bash)
        && (command.effective_name_is("mapfile") || command.effective_name_is("readarray"))
    {
        let Some(mapfile) = command.options().mapfile() else {
            return;
        };
        if command_is_shadowed_function(checker, command) {
            return;
        }
        for target in mapfile.target_name_uses() {
            if !matches!(target.kind(), ComparableNameUseKind::Literal) {
                continue;
            }
            history
                .target_spans
                .insert((target.span().start.offset, target.span().end.offset));
            history.events.push(ArrayHistoryEvent {
                offset: target.span().start.offset,
                name: Name::from(target.key().as_str()),
                state: ArrayHistoryState::from_offset(
                    checker,
                    target.span().start.offset,
                    true,
                    false,
                ),
            });
        }
    }
}

fn presence_test_reset_events(
    checker: &Checker<'_>,
    bindings: &[&Binding],
) -> Vec<ArrayHistoryEvent> {
    let mut names = HashSet::<Name>::new();
    for binding in bindings {
        names.insert(binding.name.clone());
    }

    let mut seen = HashSet::<(usize, Name)>::new();
    let mut events = Vec::new();
    for name in names {
        for fact in checker.facts().presence_test_references(&name) {
            push_reset_event(
                checker,
                &mut events,
                &mut seen,
                fact.command_span().start.offset,
                &name,
            );
        }
        for fact in checker.facts().presence_test_names(&name) {
            push_reset_event(
                checker,
                &mut events,
                &mut seen,
                fact.command_span().start.offset,
                &name,
            );
        }
    }
    events
}

fn name_only_declaration_history_events(checker: &Checker<'_>) -> Vec<ArrayHistoryEvent> {
    let mut events = Vec::new();

    for declaration in checker.semantic().declarations() {
        let flags = declaration_flag_state(declaration);
        for operand in &declaration.operands {
            let DeclarationOperand::Name { name, span } = operand else {
                continue;
            };
            if name_only_declaration_resets_array_history(declaration.builtin, flags) {
                events.push(ArrayHistoryEvent {
                    offset: span.start.offset,
                    name: name.clone(),
                    state: ArrayHistoryState::from_offset(
                        checker,
                        span.start.offset,
                        false,
                        declaration.builtin == DeclarationBuiltin::Local,
                    ),
                });
            }
        }
    }

    events
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DeclarationFlagState {
    indexed_array: bool,
    associative_array: bool,
    query: bool,
}

impl DeclarationFlagState {
    fn array_enabled(self) -> bool {
        self.indexed_array || self.associative_array
    }
}

fn declaration_flag_state(declaration: &Declaration) -> DeclarationFlagState {
    let mut state = DeclarationFlagState::default();

    for operand in &declaration.operands {
        let DeclarationOperand::Flag { flags, .. } = operand else {
            continue;
        };

        let Some((enabled, flags)) = flags
            .strip_prefix('-')
            .map(|flags| (true, flags))
            .or_else(|| flags.strip_prefix('+').map(|flags| (false, flags)))
        else {
            continue;
        };

        for flag in flags.chars() {
            match flag {
                'a' => state.indexed_array = enabled,
                'A' => state.associative_array = enabled,
                'p' => state.query = enabled,
                _ => {}
            }
        }
    }

    state
}

fn push_reset_event(
    checker: &Checker<'_>,
    events: &mut Vec<ArrayHistoryEvent>,
    seen: &mut HashSet<(usize, Name)>,
    offset: usize,
    name: &Name,
) {
    if seen.insert((offset, name.clone())) {
        events.push(ArrayHistoryEvent {
            offset,
            name: name.clone(),
            state: ArrayHistoryState::from_offset(checker, offset, false, false),
        });
    }
}

fn name_only_declaration_resets_array_history(
    builtin: DeclarationBuiltin,
    flags: DeclarationFlagState,
) -> bool {
    match builtin {
        DeclarationBuiltin::Local => true,
        DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset if flags.query => false,
        DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset => !flags.array_enabled(),
        DeclarationBuiltin::Export | DeclarationBuiltin::Readonly => false,
    }
}

fn append_declaration_assignment_name_spans(checker: &Checker<'_>) -> HashSet<(usize, usize)> {
    checker
        .semantic()
        .declarations()
        .iter()
        .flat_map(|declaration| declaration.operands.iter())
        .filter_map(|operand| match operand {
            DeclarationOperand::Assignment {
                name_span, append, ..
            } if *append => Some((name_span.start.offset, name_span.end.offset)),
            _ => None,
        })
        .collect()
}

fn binding_can_trigger_array_to_string_conversion(
    binding: &Binding,
    append_declaration_assignments: &HashSet<(usize, usize)>,
) -> bool {
    if append_declaration_assignments
        .contains(&(binding.span.start.offset, binding.span.end.offset))
    {
        return false;
    }

    matches!(
        binding.kind,
        BindingKind::Assignment
            | BindingKind::ParameterDefaultAssignment
            | BindingKind::Declaration(_)
    )
}

fn binding_establishes_array_history(
    binding: &Binding,
    builtin_history: &BuiltinArrayHistory,
) -> bool {
    match binding.kind {
        BindingKind::Imported => false,
        BindingKind::ReadTarget | BindingKind::MapfileTarget => {
            builtin_history.contains_target(binding)
        }
        BindingKind::Declaration(DeclarationBuiltin::Local)
            if !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED) =>
        {
            false
        }
        _ => binding_is_array_like(binding),
    }
}

fn binding_establishes_local_scalar_history(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Declaration(DeclarationBuiltin::Local)
    ) && binding
        .attributes
        .contains(BindingAttributes::DECLARATION_INITIALIZED)
        && !binding_is_array_like(binding)
}

fn binding_resets_array_history(binding: &Binding, builtin_history: &BuiltinArrayHistory) -> bool {
    match binding.kind {
        BindingKind::LoopVariable
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::ReadTarget | BindingKind::MapfileTarget => {
            !builtin_history.contains_target(binding)
        }
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::Declaration(_)
        | BindingKind::FunctionDefinition
        | BindingKind::Nameref
        | BindingKind::Imported => false,
    }
}

fn declaration_resets_array_history(binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::Declaration(DeclarationBuiltin::Local) => !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED),
        BindingKind::Declaration(DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset) => {
            !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
                && !binding_is_array_like(binding)
        }
        _ => false,
    }
}

fn binding_uses_builtin_array_history(checker: &Checker<'_>, binding: &Binding) -> bool {
    matches!(checker.shell(), ShellDialect::Bash) && matches!(binding.name.as_str(), "MAPFILE")
}

fn binding_command<'checker, 'ast>(
    checker: &'checker Checker<'ast>,
    binding: &Binding,
) -> Option<crate::facts::commands::CommandFactRef<'checker, 'ast>> {
    checker
        .facts()
        .innermost_command_at_binding_offset(binding.span.start.offset)
        .or_else(|| {
            checker
                .facts()
                .commands()
                .iter()
                .rev()
                .find(|command| contains_span(command.span(), binding.span))
        })
}

fn command_is_shadowed_function(
    checker: &Checker<'_>,
    command: crate::facts::commands::CommandFactRef<'_, '_>,
) -> bool {
    let Some(name_span) = command.body_word_span() else {
        return false;
    };
    if command_wrapper_is_shadowed_function(checker, command, name_span) {
        return true;
    }
    if command_forces_builtin_resolution(command) {
        return false;
    }

    let Some(command_name) = command.effective_or_literal_name() else {
        return false;
    };
    command_name_has_visible_function_binding(checker, command_name, name_span)
}

fn command_name_has_visible_function_binding(
    checker: &Checker<'_>,
    name: &str,
    at: shuck_ast::Span,
) -> bool {
    let name = Name::from(name);
    has_visible_function_name_binding(checker, &name, at)
}

fn command_forces_builtin_resolution(
    command: crate::facts::commands::CommandFactRef<'_, '_>,
) -> bool {
    let mut saw_forcing_wrapper = false;

    for wrapper in command.wrappers() {
        match wrapper {
            WrapperKind::Command | WrapperKind::Builtin => saw_forcing_wrapper = true,
            _ => return false,
        }
    }

    saw_forcing_wrapper
}

fn command_wrapper_is_shadowed_function(
    checker: &Checker<'_>,
    command: crate::facts::commands::CommandFactRef<'_, '_>,
    at: shuck_ast::Span,
) -> bool {
    let mut lookup_bypasses_functions = false;

    for wrapper in command.wrappers() {
        let wrapper_name = match wrapper {
            WrapperKind::Command => "command",
            WrapperKind::Builtin => "builtin",
            _ => return false,
        };

        if !lookup_bypasses_functions
            && command_name_has_visible_function_binding(checker, wrapper_name, at)
        {
            return true;
        }

        lookup_bypasses_functions = true;
    }

    false
}

fn contains_span(outer: shuck_ast::Span, inner: shuck_ast::Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn binding_is_array_like(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || binding.kind == BindingKind::ArrayAssignment
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_scalar_reassignments_after_prior_array_bindings() {
        let source = "\
#!/bin/bash
exts=(txt pdf doc)
exts=\"${exts[*]}\"
items=(one two)
items=\"${items[0]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts", "items"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_assignments_without_prior_array_like_binding() {
        let source = "\
#!/bin/bash
name=base
name=\"${name}-suffix\"
other=\"${unknown:-fallback}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_shadowed_local_scalars_after_prior_array_bindings() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  local exts=base
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_scalar_declarations_after_prior_array_declarations() {
        let source = "\
#!/bin/bash
f() {
  declare -a cmd
  cmd=\"curl\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["cmd"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_shadowed_local_scalars_after_sibling_function_local_arrays() {
        let source = "\
#!/bin/zsh
f() {
  local -a cmd
  cmd=(curl -I)
}
g() {
  local cmd=cp
}
h() {
  local -a ___opt
  ___opt=(-a -b)
}
i() {
  local ___opt=\"$1\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_global_scalar_after_hidden_function_local_array_history() {
        let source = "\
#!/bin/zsh
arr=(global)
f() {
  local -a arr
  arr=(local)
}
arr=scalar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["arr"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_shadowed_array_history_after_initialized_local_scalar() {
        let source = "\
#!/bin/zsh
f() {
  local cmd=cp
  cmd=(curl -I)
}
g() {
  local cmd=mv
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_same_function_local_scalar_after_initialized_local_array() {
        let source = "\
#!/bin/zsh
f() {
  local -a p
  p=(one two)
  local p=$'\\n'
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["p"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn combined_declaration_array_flags_keep_array_history() {
        let source = "\
#!/bin/bash
declare -a indexed=(one)
declare -ga indexed
indexed=value
declare -A assoc=([key]=value)
declare -gA assoc
assoc=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["indexed", "assoc"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn declaration_array_flag_removals_keep_the_other_array_kind() {
        let source = "\
#!/bin/bash
declare -a indexed=(one)
declare -a +A indexed
indexed=value
declare -A assoc=([key]=value)
declare -A +a assoc
assoc=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["indexed", "assoc"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn declare_and_typeset_query_only_forms_keep_array_history() {
        let source = "\
#!/bin/bash
declare -a declared=(one)
declare -p declared >/dev/null
declared=value
typeset -a typed=(one)
typeset -p typed >/dev/null
typed=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["declared", "typed"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn local_query_only_forms_clear_array_history() {
        let source = "\
#!/bin/bash
f() {
  local -a local_arr=(one)
  local -p local_arr >/dev/null
  local_arr=value
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_assignments_after_bare_local_resets() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  local exts
  exts=base
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_bare_local_array_declarations_without_initializers() {
        let source = "\
#!/bin/bash
f() {
  local -a cmd
  local cmd=\"curl\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_same_scope_bare_local_resets() {
        let source = "\
#!/bin/bash
f() {
  local res=(
    320x240
    640x480
  )
  local res
  res=$(choose_resolution)
  res=\"${res[0]}\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn presence_tests_clear_array_history() {
        let source = "\
#!/bin/bash
declare -A rate ipv4 ipv6
name=guest
if [ -n \"${rate[$name]}\" ]; then
  rate=${rate[$name]}
elif [ -n \"${rate[::default]}\" ]; then
  rate=${rate[::default]}
else
  rate=0
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn presence_tests_inside_functions_clear_array_history() {
        let source = "\
#!/bin/bash
if ((${BASH_VERSINFO[0]} > 3)); then
  declare -A registered_shims
  remove_stale_shims() {
    if [[ ! ${registered_shims[\"${shim##*/}\"]} ]]; then
      :
    fi
  }
else
  registered_shims=\" \"
  register_shim() {
    registered_shims=\"${registered_shims}${1} \"
  }
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_scalar_reassignments_after_read_array_targets() {
        let source = "\
#!/bin/bash
f() {
  read -r -a resolution <<< \"1 2 3\"
  resolution=\"${resolution[0]} x ${resolution[1]} @ ${resolution[2]} fps\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["resolution"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_scalar_reassignments_after_attached_read_array_targets() {
        let source = "\
#!/bin/bash
f() {
  read -aresolution <<< \"1 2 3\"
  resolution=\"${resolution[0]} x ${resolution[1]} @ ${resolution[2]} fps\"
  read -ar <<< \"4 5 6\"
  r=\"${r[0]} x ${r[1]} @ ${r[2]} fps\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["resolution", "r"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn scalar_binding_targets_clear_array_history() {
        let source = "\
#!/bin/bash
f() {
  local option
  read -ra option <<< \"$option\"
  for option in \"${params[@]}\"; do
    :
  done
}
g() {
  local option=\"$1\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn plain_read_targets_clear_array_history() {
        let source = "\
#!/bin/bash
arr=()
read arr
arr=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_sibling_and_global_scalars_after_function_local_array_use() {
        let source = "\
#!/bin/bash
f() {
  local fuzzer=$1
  if [[ $fuzzer == *\"@\"* ]]; then
    fuzzer=(${fuzzer//@/ }[0])
  fi
}
g() {
  local fuzzer=$1
}
fuzzer=$1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_scalar_assignments_outside_bash() {
        let source = "\
#!/bin/sh
mapfile entries
entries=value
MAPFILE=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_targets_from_shadowing_functions() {
        let source = "\
#!/bin/bash
mapfile() {
  :
}
mapfile entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_callback_names() {
        let source = "\
#!/bin/bash
mapfile -C cb -c 1 lines
cb=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_mapfile_targets_after_callback_options() {
        let source = "\
#!/bin/bash
mapfile -C cb -c 1 lines
lines=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["lines"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_read_array_history_through_command_wrapper() {
        let source = "\
#!/bin/bash
read() {
  :
}
command read -a entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["entries"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_mapfile_history_through_builtin_wrapper() {
        let source = "\
#!/bin/bash
mapfile() {
  :
}
builtin mapfile lines
lines=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["lines"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_dynamic_command_targets_as_array_history() {
        let source = "\
#!/bin/bash
dest=entries
command read -a \"$dest\"
builtin mapfile \"$dest\"
read -a \"$dest\"
mapfile \"$dest\"
dest=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_quoted_wrapper_targets_as_array_history() {
        let source = "\
#!/bin/bash
read() {
  :
}
mapfile() {
  :
}
command read -a \"entries\"
builtin mapfile 'lines'
entries=value
lines=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["entries", "lines"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_wrapper_targets_when_wrapper_commands_are_shadowed() {
        let source = "\
#!/bin/bash
command() {
  :
}
builtin() {
  :
}
command read -a entries
builtin mapfile lines
entries=value
lines=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_read_scalar_assignments_outside_bash() {
        let source = "\
#!/bin/sh
read -a entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_read_targets_from_shadowing_functions() {
        let source = "\
#!/bin/bash
read() {
  :
}
read -a entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_targets_from_imported_shadowing_functions() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/bash
source ./helper.sh
mapfile entries
entries=value
";

        fs::write(&main, source).unwrap();
        fs::write(&helper, "mapfile() { :; }\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_targets_when_function_shadowing_is_followed_by_variable_rebindings() {
        let source = "\
#!/bin/bash
read() {
  :
}
mapfile() {
  :
}
read=value
mapfile=value
read -a entries
mapfile lines
entries=value
lines=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_string_appends_after_scalar_reassignments() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
exts=\"${exts[*]}\"
exts+=\" ${exts^^}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_declaration_appends_as_scalar_conversion_triggers() {
        let source = "\
#!/bin/bash
f() {
  local logs=() running=()
  logs+=\"cmd\"
  if [[ $i != \"$max\" ]]; then
    local logs+=\"& \"
  else
    local logs+=\"; \"
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn keeps_array_history_after_declaration_appends() {
        let source = "\
#!/bin/bash
f() {
  local logs=()
  local logs+=\"& \"
  logs=\"done\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["logs"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_later_assignments_after_bare_declare_resets() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  declare exts
}
g() {
  local exts=base
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_array_style_references_without_prior_array_bindings() {
        let source = "\
#!/bin/bash
echo \"${exts[@]}\"
exts=base
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
