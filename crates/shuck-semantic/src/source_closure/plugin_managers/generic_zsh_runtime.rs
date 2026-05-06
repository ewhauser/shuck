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

impl ZshPluginManager for GenericZshRuntimeManager {
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
    let roots = deferred_zsh_entrypoint_roots(context.facts, context.scope);
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
fn deferred_zsh_entrypoint_roots(facts: &AstFacts, scope: ScopeId) -> Vec<Name> {
    let mut roots = Vec::new();
    for call in facts.calls.iter().filter(|call| call.scope == scope) {
        match call.name.as_str() {
            "add-zsh-hook" | "add-zle-hook-widget" => {
                if add_zsh_hook_removes_callback(call) {
                    continue;
                }
                if let Some(function) = zsh_hook_callback_name(call) {
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

// `add-zsh-hook -d/-D hook func` removes callbacks instead of registering them,
// so deletion calls must not become deferred entrypoints.
fn add_zsh_hook_removes_callback(call: &CallInfo) -> bool {
    call.args.iter().flatten().any(|arg| {
        let Some(flags) = arg.strip_prefix('-') else {
            return false;
        };
        flags.chars().any(|flag| matches!(flag, 'd' | 'D'))
    })
}

// `add-zsh-hook hook function` and `add-zle-hook-widget hook function` both use
// the second non-option operand as the callback name.
fn zsh_hook_callback_name(call: &CallInfo) -> Option<Name> {
    let operands = static_non_option_args(call);
    let function = operands.get(1)?;
    valid_static_function_name(function).then(|| Name::from(function.as_str()))
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
    for (function_scope, bindings) in analysis.function_bindings_by_scope() {
        if semantic.scope(function_scope).parent != Some(scope) {
            continue;
        }
        for binding_id in bindings {
            let binding = semantic.binding(*binding_id);
            scopes.insert(binding.name.clone(), function_scope);
        }
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
            for target in generated_template_targets_for_name(self.source, name) {
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
        for generated in generated_eval_calls_for_scope(self.semantic, self.source, scope, args) {
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
    source: &str,
    scope: ScopeId,
    args: &[Option<String>],
) -> Vec<Name> {
    let scope = &semantic.scopes()[scope.index()];
    let body =
        &source[scope.span.start.offset.min(source.len())..scope.span.end.offset.min(source.len())];
    let aliases = positional_aliases(body);
    let mut calls = Vec::new();
    for template in eval_template_bodies(body) {
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
fn eval_template_bodies(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut templates = Vec::new();
    let mut index = 0;
    while let Some(eval_offset) = body[index..].find("eval") {
        let eval_start = index + eval_offset;
        let eval_end = eval_start + "eval".len();
        if eval_start > 0 && is_function_name_byte(bytes[eval_start - 1])
            || bytes
                .get(eval_end)
                .is_some_and(|byte| is_function_name_byte(*byte))
        {
            index = eval_end;
            continue;
        }

        let mut cursor = eval_end;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        let Some(&quote) = bytes
            .get(cursor)
            .filter(|quote| matches!(quote, b'\'' | b'"'))
        else {
            index = eval_end;
            continue;
        };
        let content_start = cursor + 1;
        let Some(content_end) = find_quoted_template_end(bytes, content_start, quote) else {
            index = content_start;
            continue;
        };
        templates.push(&body[content_start..content_end]);
        index = content_end + 1;
    }
    templates
}

fn find_quoted_template_end(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == b'\\' && quote == b'"' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return Some(index);
        }
        index += 1;
    }
    None
}

// Given a generated wrapper name, inspect eval templates in the source to find
// the static function that wrapper delegates to. This is a source-shape bridge,
// not a general eval interpreter: it only rewrites shared-variable function-name
// prefixes that appear in the same template.
fn generated_template_targets_for_name(source: &str, name: &Name) -> Vec<Name> {
    let expansions = prefixed_variable_expansions_with_offsets(source);
    let mut targets = Vec::new();
    for (index, definition) in expansions.iter().enumerate() {
        let Some(fragment) = name.as_str().strip_prefix(&definition.prefix) else {
            continue;
        };
        if !valid_generated_function_fragment(fragment)
            || !looks_like_generated_function_definition(source, definition.end)
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
    targets.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    targets.dedup();
    targets
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
            && let Some(position) = value.strip_prefix('$').and_then(|value| value.parse().ok())
        {
            aliases.insert(name.to_owned(), position);
        }
    }
    aliases
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
