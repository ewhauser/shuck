//! Zsh deferred plugin entrypoint modeling.
//!
//! This module summarizes reads that happen after a plugin file is sourced via
//! zsh lifecycle APIs such as hooks and ZLE widgets. It is intentionally a
//! bounded symbolic model: it follows static callbacks and narrow generated
//! wrapper patterns, but it does not execute shell code or interpret arbitrary
//! `eval` strings.

use super::super::*;
use super::*;
use crate::ReferenceKind;

pub(super) struct GenericZshRuntimeManager;

impl ZshDeferredRuntimeManager for GenericZshRuntimeManager {
    fn collect_deferred_required_reads(
        &self,
        context: &DeferredPluginRuntimeContext<'_>,
    ) -> Vec<Name> {
        collect_generic_deferred_required_reads(context)
    }
}

// Zsh plugins often install work that runs after the file is sourced, e.g.
// through `add-zsh-hook precmd callback` or `zle -N widget callback`. For source
// closure purposes those callbacks are part of the plugin contract: a user file
// that sets a variable before loading the plugin expects the callback to observe
// that value later, even though the callback body is not called syntactically at
// source time.
fn collect_generic_deferred_required_reads(
    context: &DeferredPluginRuntimeContext<'_>,
) -> Vec<Name> {
    let registration_scopes = source_time_registration_scopes(
        context.semantic,
        context.analysis,
        context.facts,
        context.scope,
    );
    let roots = deferred_zsh_entrypoint_roots(context.facts, &registration_scopes);
    if roots.is_empty() {
        return Vec::new();
    }

    let function_scopes =
        function_scopes_by_name(context.semantic, context.analysis, context.scope);
    let mut visitor = DeferredFunctionReadVisitor {
        semantic: context.semantic,
        analysis: context.analysis,
        facts: context.facts,
        source: context.source,
        synthetic_reads: context.synthetic_reads,
        function_scopes: &function_scopes,
        visited: FxHashSet::default(),
        visited_generated: FxHashSet::default(),
        reads: FxHashSet::default(),
    };

    for root in roots {
        visitor.visit_name(&root, &[]);
    }

    let mut reads = visitor.reads.into_iter().collect::<Vec<_>>();
    reads.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    reads
}

// Return statically registered zsh lifecycle callbacks in this scope. This is
// intentionally zsh-framework-agnostic: the roots come from zsh's hook/widget
// APIs, not from plugin names or path conventions.
fn deferred_zsh_entrypoint_roots(
    facts: &AstFacts,
    registration_scopes: &FxHashSet<ScopeId>,
) -> Vec<Name> {
    let mut roots = Vec::new();
    for call in facts
        .calls
        .iter()
        .filter(|call| registration_scopes.contains(&call.scope))
    {
        match call.name.as_str() {
            "add-zsh-hook" | "add-zle-hook-widget" => {
                if zsh_hook_call_does_not_register_callbacks(call) {
                    continue;
                }
                for function in zsh_hook_callback_names(call) {
                    roots.push(function);
                }
            }
            "zle" => {
                if let Some(function) = zle_widget_callback_name(call) {
                    roots.push(function);
                }
            }
            _ => {}
        }
    }
    roots.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    roots.dedup();
    roots
}

// Hook registrations can happen directly at file scope or inside setup
// functions that run while the plugin is sourced. Follow static source-time
// function calls using the semantic function resolver, and collect the
// non-function scopes where hook registration commands may execute.
fn source_time_registration_scopes(
    semantic: &SemanticModel,
    analysis: &crate::SemanticAnalysis<'_>,
    facts: &AstFacts,
    root: ScopeId,
) -> FxHashSet<ScopeId> {
    let mut registration_scopes = FxHashSet::default();
    let mut visited_roots = FxHashSet::default();
    let mut pending = vec![root];

    while let Some(scope) = pending.pop() {
        if !visited_roots.insert(scope) {
            continue;
        }
        let member_scopes = scope_members_excluding_functions(semantic.scopes(), scope);
        registration_scopes.extend(member_scopes.iter().copied());
        for call in facts
            .calls
            .iter()
            .filter(|call| member_scopes.contains(&call.scope))
        {
            let Some(binding) = analysis.visible_function_binding_defined_before(
                &call.name,
                call.scope,
                call.span.start.offset,
            ) else {
                continue;
            };
            if let Some(callee_scope) = analysis.function_scope_for_binding(binding) {
                pending.push(callee_scope);
            }
        }
    }

    registration_scopes
}

// `add-zsh-hook -d/-D hook func` removes callbacks and `-L` lists callbacks
// instead of registering them, so these calls must not become deferred roots.
fn zsh_hook_call_does_not_register_callbacks(call: &CallInfo) -> bool {
    call.args.iter().flatten().any(|arg| {
        let Some(flags) = arg.strip_prefix('-') else {
            return false;
        };
        flags.chars().any(|flag| matches!(flag, 'd' | 'D' | 'L'))
    })
}

// `add-zsh-hook hook function ...` and `add-zle-hook-widget hook function ...`
// both use operands after the hook name as callback names.
fn zsh_hook_callback_names(call: &CallInfo) -> Vec<Name> {
    let operands = static_non_option_args(call);
    operands
        .into_iter()
        .skip(1)
        .filter(|function| valid_static_function_name(function))
        .map(|function| Name::from(function.as_str()))
        .collect()
}

// For `zle -N`, a single operand means widget and function share a name; two
// operands mean `widget function`. Dynamic widget registrations are left alone.
fn zle_widget_callback_name(call: &CallInfo) -> Option<Name> {
    let args = static_args(call)?;
    let registration_index = args.iter().position(|arg| *arg == "-N")?;
    let operands = args[registration_index + 1..]
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect::<Vec<_>>();
    let function = match operands.as_slice() {
        [widget] => *widget,
        [_, function, ..] => *function,
        _ => return None,
    };
    valid_static_function_name(function).then(|| Name::from(function.as_str()))
}

fn static_non_option_args(call: &CallInfo) -> Vec<&String> {
    call.args
        .iter()
        .flatten()
        .filter(|arg| !arg.starts_with('-'))
        .collect()
}

fn static_args(call: &CallInfo) -> Option<Vec<&String>> {
    call.args.iter().map(Option::as_ref).collect()
}

fn valid_static_function_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
}

fn function_scopes_by_name(
    semantic: &SemanticModel,
    analysis: &crate::SemanticAnalysis<'_>,
    scope: ScopeId,
) -> FxHashMap<Name, ScopeId> {
    let mut scopes = FxHashMap::default();
    let mut definitions = semantic
        .function_definition_bindings()
        .filter_map(|binding| {
            analysis
                .function_scope_for_binding(binding.id)
                .map(|function_scope| (binding, function_scope))
        })
        .collect::<Vec<_>>();
    definitions.sort_by_key(|(binding, _)| binding.span.start.offset);

    for (binding, function_scope) in definitions {
        if semantic.scope(function_scope).parent != Some(scope) {
            continue;
        }
        scopes.insert(binding.name.clone(), function_scope);
    }
    scopes
}

struct DeferredFunctionReadVisitor<'a> {
    semantic: &'a SemanticModel,
    analysis: &'a crate::SemanticAnalysis<'a>,
    facts: &'a AstFacts,
    source: &'a str,
    synthetic_reads: &'a [SyntheticRead],
    function_scopes: &'a FxHashMap<Name, ScopeId>,
    visited: FxHashSet<(ScopeId, Vec<Option<String>>)>,
    visited_generated: FxHashSet<Name>,
    reads: FxHashSet<Name>,
}

impl DeferredFunctionReadVisitor<'_> {
    // Follow a deferred callback or generated wrapper name when it resolves to a
    // local static function. If the name was generated by an eval template, map
    // the generated wrapper back to the static function it delegates to.
    fn visit_name(&mut self, name: &Name, args: &[Option<String>]) {
        let Some(&scope) = self.function_scopes.get(name) else {
            if !self.visited_generated.insert(name.clone()) {
                return;
            }
            for target in generated_template_targets_for_name(self.facts, name) {
                self.visit_name(&target, &[]);
            }
            return;
        };
        self.visit_scope(scope, args);
    }

    // Summarize the callback body plus any local static calls reachable from it.
    // This is deliberately symbolic and bounded: we follow function names and
    // literal arguments that Shuck already extracted, but we do not run shell
    // control flow or expand arbitrary strings.
    fn visit_scope(&mut self, scope: ScopeId, args: &[Option<String>]) {
        let key = (scope, args.to_vec());
        if !self.visited.insert(key) {
            return;
        }

        let contract = summarize_scope_body_contract(
            self.semantic,
            self.analysis,
            scope,
            self.synthetic_reads,
            false,
        );
        self.reads.extend(contract.required_reads);

        let member_scopes = scope_members_excluding_functions(self.semantic.scopes(), scope);
        self.reads
            .extend(file_scope_reads_in_scopes(self.semantic, &member_scopes));
        for call in self
            .facts
            .calls
            .iter()
            .filter(|call| member_scopes.contains(&call.scope))
        {
            self.visit_name(&call.name, &call.args);
        }
        for generated in
            generated_eval_calls_for_scope(self.semantic, self.facts, self.source, scope, args)
        {
            self.visit_name(&generated, &[]);
        }
    }
}

// Deferred callbacks can read a plugin's own file-scope default first, e.g.
// `ZSH_AUTOSUGGEST_STRATEGY=${...}` followed by a later callback read. That
// should still be exposed as a plugin contract because caller assignments are
// intended to override those defaults before the callback fires.
fn file_scope_reads_in_scopes(
    semantic: &SemanticModel,
    member_scopes: &FxHashSet<ScopeId>,
) -> Vec<Name> {
    let mut reads = FxHashSet::default();
    for reference in semantic.references() {
        if matches!(reference.kind, ReferenceKind::DeclarationName) {
            continue;
        }
        if !member_scopes.contains(&reference.scope) {
            continue;
        }
        let Some(binding) = semantic.resolved_binding(reference.id) else {
            continue;
        };
        if binding.scope == ScopeId(0) {
            reads.insert(reference.name.clone());
        }
    }
    reads.into_iter().collect()
}

// Recognize calls produced by simple zsh eval templates when a function copies a
// literal positional argument into a local variable and then embeds that variable
// in a function-name prefix. This covers patterns like:
// `local action=$2; eval '_plugin_widget_$action() { _plugin_$action "$@" }'`
// when the caller passed a static second argument such as `modify`.
fn generated_eval_calls_for_scope(
    semantic: &SemanticModel,
    facts: &AstFacts,
    source: &str,
    scope: ScopeId,
    args: &[Option<String>],
) -> Vec<Name> {
    let scope = &semantic.scopes()[scope.index()];
    let body =
        &source[scope.span.start.offset.min(source.len())..scope.span.end.offset.min(source.len())];
    let aliases = positional_aliases(body);
    let member_scopes = scope_members_excluding_functions(semantic.scopes(), scope.id);
    let mut calls = Vec::new();
    for template in eval_template_bodies(facts, &member_scopes) {
        for (prefix, variable) in prefixed_variable_expansions(template) {
            let Some(position) = aliases.get(&variable) else {
                continue;
            };
            let Some(Some(fragment)) = args.get(position.saturating_sub(1)) else {
                continue;
            };
            if valid_generated_function_fragment(fragment) {
                let generated = format!("{prefix}{fragment}");
                calls.push(Name::from(generated.as_str()));
            }
        }
    }
    calls.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    calls.dedup();
    calls
}

// Return static quoted arguments passed directly to `eval`. Generated callback
// inference is limited to these templates so ordinary text like
// `print "_plugin_$action"` does not become a synthetic call edge.
fn eval_template_bodies<'a>(
    facts: &'a AstFacts,
    member_scopes: &FxHashSet<ScopeId>,
) -> Vec<&'a str> {
    facts
        .calls
        .iter()
        .filter(|call| member_scopes.contains(&call.scope) && call.name.as_str() == "eval")
        .flat_map(|call| call.args.iter().filter_map(Option::as_deref))
        .collect()
}

// Given a generated wrapper name, inspect eval templates in the source to find
// the static function that wrapper delegates to. This is a source-shape bridge,
// not a general eval interpreter: it only rewrites shared-variable function-name
// prefixes that appear in the same template.
fn generated_template_targets_for_name(facts: &AstFacts, name: &Name) -> Vec<Name> {
    let mut targets = Vec::new();
    for template in eval_template_bodies_for_all_scopes(facts) {
        let expansions = prefixed_variable_expansions_with_offsets(template);
        for (index, definition) in expansions.iter().enumerate() {
            let Some(fragment) = name.as_str().strip_prefix(&definition.prefix) else {
                continue;
            };
            if !valid_generated_function_fragment(fragment)
                || !looks_like_generated_function_definition(template, definition.end)
            {
                continue;
            }

            for target in expansions
                .iter()
                .skip(index + 1)
                .filter(|target| target.variable == definition.variable)
            {
                if target.prefix == definition.prefix {
                    continue;
                }
                let generated = format!("{}{}", target.prefix, fragment);
                if valid_static_function_name(&generated) {
                    targets.push(Name::from(generated.as_str()));
                }
            }
        }
    }
    targets.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    targets.dedup();
    targets
}

fn eval_template_bodies_for_all_scopes(facts: &AstFacts) -> Vec<&str> {
    facts
        .calls
        .iter()
        .filter(|call| call.name.as_str() == "eval")
        .flat_map(|call| call.args.iter().filter_map(Option::as_deref))
        .collect()
}

// A generated function definition has `()` immediately after the variable-backed
// name, modulo whitespace. Other prefixed expansions are treated as ordinary
// dynamic text and ignored.
fn looks_like_generated_function_definition(source: &str, variable_end: usize) -> bool {
    source
        .get(variable_end..)
        .is_some_and(|tail| tail.trim_start().starts_with("()"))
}

// Capture straightforward local aliases from positional parameters, e.g.
// `local action=$2`. The value is the 1-based positional index.
fn positional_aliases(body: &str) -> FxHashMap<String, usize> {
    let mut aliases = FxHashMap::default();
    for token in body.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '\n' | '\r')) {
        let Some((name, value)) = token.split_once('=') else {
            continue;
        };
        if valid_variable_name(name)
            && let Some(position) = positional_alias_value(value)
        {
            aliases.insert(name.to_owned(), position);
        }
    }
    aliases
}

fn positional_alias_value(value: &str) -> Option<usize> {
    value
        .strip_prefix('$')
        .or_else(|| {
            value
                .strip_prefix("\"$")
                .and_then(|value| value.strip_suffix('"'))
        })
        .and_then(|value| value.parse().ok())
}

// A function-name prefix ending in `$variable`, plus the byte offset where the
// variable name ended. The offset lets callers distinguish generated function
// definitions from ordinary references to generated names.
struct PrefixedVariableExpansion {
    prefix: String,
    variable: String,
    end: usize,
}

// Return `_prefix_$variable`-style expansions that can participate in generated
// function names. The recognizer is intentionally narrow to avoid treating
// arbitrary shell words as statically known calls.
fn prefixed_variable_expansions(body: &str) -> Vec<(String, String)> {
    prefixed_variable_expansions_with_offsets(body)
        .into_iter()
        .map(|expansion| (expansion.prefix, expansion.variable))
        .collect()
}

fn prefixed_variable_expansions_with_offsets(body: &str) -> Vec<PrefixedVariableExpansion> {
    let bytes = body.as_bytes();
    let mut expansions = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }

        let prefix_start = (0..index)
            .rev()
            .find(|&candidate| !is_function_name_byte(bytes[candidate]))
            .map_or(0, |candidate| candidate + 1);
        let prefix = &body[prefix_start..index];
        let mut variable_end = index + 1;
        while variable_end < bytes.len() && is_variable_name_byte(bytes[variable_end]) {
            variable_end += 1;
        }
        let variable = &body[index + 1..variable_end];
        if !prefix.is_empty()
            && prefix.starts_with('_')
            && prefix.ends_with('_')
            && valid_variable_name(variable)
        {
            expansions.push(PrefixedVariableExpansion {
                prefix: prefix.to_owned(),
                variable: variable.to_owned(),
                end: variable_end,
            });
        }
        index = variable_end.max(index + 1);
    }
    expansions
}

fn is_function_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_')
}

fn is_variable_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn valid_generated_function_fragment(fragment: &str) -> bool {
    !fragment.is_empty()
        && fragment
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
