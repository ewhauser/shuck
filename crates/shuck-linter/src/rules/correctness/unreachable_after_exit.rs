use crate::{Checker, CommandFact, CommandFacts, ListFact, Rule, Violation, WrapperKind};
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command as AstCommand, Name, Span};
use shuck_semantic::{
    BindingId, BindingOrigin, ScopeId, ScopeKind, SemanticAnalysis, UnreachableCauseKind,
};

pub struct UnreachableAfterExit;

impl Violation for UnreachableAfterExit {
    fn rule() -> Rule {
        Rule::UnreachableAfterExit
    }

    fn message(&self) -> String {
        "code is unreachable".to_owned()
    }
}

pub fn unreachable_after_exit(checker: &mut Checker) {
    let source = checker.source();
    let short_circuit_lists = checker.facts().lists();
    let commands = checker.facts().commands();
    let semantic_analysis = checker.semantic_analysis();
    let unreached_function_spans = unreached_function_definition_spans(checker);
    let unreachable_spans = outermost_unreachable_spans(
        semantic_analysis
            .dead_code()
            .iter()
            .filter(|dead_code| dead_code.cause_kind != UnreachableCauseKind::LoopControl)
            .flat_map(|dead_code| dead_code.unreachable.iter().copied())
            .filter(|span| {
                !span_matches_short_circuit_skip(
                    *span,
                    short_circuit_lists,
                    commands,
                    semantic_analysis,
                ) && !span_is_inside_unreached_function(*span, &unreached_function_spans)
            })
            .collect::<Vec<_>>(),
    );

    for span in unreachable_spans
        .into_iter()
        .map(|span| trim_trailing_terminator(span, source))
    {
        checker.report(UnreachableAfterExit, span);
    }
}

fn unreached_function_definition_spans(checker: &Checker<'_>) -> Vec<Span> {
    let mut spans = Vec::new();

    if file_has_file_scope_exit(checker) {
        spans.extend(
            checker
                .semantic_analysis()
                .unreached_functions()
                .iter()
                .filter_map(|unreached| function_definition_span(checker, unreached.binding)),
        );
    }

    if file_has_top_level_exit_command(checker) {
        spans.extend(statically_unreached_function_definition_spans(checker));
    }

    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

fn statically_unreached_function_definition_spans(checker: &Checker<'_>) -> Vec<Span> {
    let scope_bindings = function_bindings_by_scope(checker);
    let reachable = statically_reachable_function_bindings(checker, &scope_bindings);

    checker
        .facts()
        .function_headers()
        .iter()
        .filter_map(|header| {
            let binding = header.binding_id()?;
            if !function_definition_is_in_file_scope(checker, binding) {
                return None;
            }
            if reachable.contains(&binding) {
                return None;
            }
            function_definition_span(checker, binding)
        })
        .collect()
}

fn statically_reachable_function_bindings(
    checker: &Checker<'_>,
    scope_bindings: &FxHashMap<ScopeId, BindingId>,
) -> FxHashSet<BindingId> {
    let mut calls_by_caller = FxHashMap::<Option<BindingId>, Vec<StaticFunctionCall>>::default();

    for command in checker.facts().commands() {
        let Some(name) = command.effective_or_literal_name() else {
            continue;
        };
        let Some(name_span) = command.body_word_span() else {
            continue;
        };
        let name = Name::from(name);
        let Some(callee) = checker
            .semantic_analysis()
            .visible_function_binding_at_call(&name, name_span)
        else {
            continue;
        };
        let caller = enclosing_function_binding(checker, scope_bindings, name_span);
        calls_by_caller
            .entry(caller)
            .or_default()
            .push(StaticFunctionCall { callee, name_span });
    }

    let mut reachable = FxHashSet::default();
    let mut latest_entry_offsets = FxHashMap::<BindingId, usize>::default();
    let mut worklist = calls_by_caller
        .remove(&None)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|call| {
            static_call_can_resolve_at_entry(
                checker,
                call.callee,
                call.name_span,
                call.name_span.start.offset,
            )
            .then_some((call.callee, call.name_span.start.offset))
        })
        .collect::<Vec<_>>();

    while let Some((binding, entry_offset)) = worklist.pop() {
        if latest_entry_offsets
            .get(&binding)
            .is_some_and(|latest_entry| *latest_entry >= entry_offset)
        {
            continue;
        }
        latest_entry_offsets.insert(binding, entry_offset);
        reachable.insert(binding);

        if let Some(callees) = calls_by_caller.get(&Some(binding)) {
            worklist.extend(callees.iter().filter_map(|call| {
                static_call_can_resolve_at_entry(checker, call.callee, call.name_span, entry_offset)
                    .then_some((call.callee, entry_offset))
            }));
        }
    }

    reachable
}

#[derive(Clone, Copy)]
struct StaticFunctionCall {
    callee: BindingId,
    name_span: Span,
}

fn static_call_can_resolve_at_entry(
    checker: &Checker<'_>,
    callee: BindingId,
    name_span: Span,
    entry_offset: usize,
) -> bool {
    let callee_binding = checker.semantic().binding(callee);
    name_span.start.offset >= callee_binding.span.start.offset
        || (matches!(
            checker.semantic().scope(callee_binding.scope).kind,
            ScopeKind::File
        ) && entry_offset >= callee_binding.span.start.offset)
}

fn function_bindings_by_scope(checker: &Checker<'_>) -> FxHashMap<ScopeId, BindingId> {
    checker
        .facts()
        .function_headers()
        .iter()
        .filter_map(|header| Some((header.function_scope()?, header.binding_id()?)))
        .collect()
}

fn enclosing_function_binding(
    checker: &Checker<'_>,
    scope_bindings: &FxHashMap<ScopeId, BindingId>,
    span: Span,
) -> Option<BindingId> {
    let scope = checker.semantic().scope_at(span.start.offset);
    checker
        .semantic()
        .ancestor_scopes(scope)
        .find_map(|scope| scope_bindings.get(&scope).copied())
}

fn function_definition_span(
    checker: &Checker<'_>,
    binding: shuck_semantic::BindingId,
) -> Option<Span> {
    let binding = checker.semantic().binding(binding);
    match binding.origin {
        BindingOrigin::FunctionDefinition { definition_span } => Some(definition_span),
        BindingOrigin::Assignment { .. }
        | BindingOrigin::LoopVariable { .. }
        | BindingOrigin::ParameterDefaultAssignment { .. }
        | BindingOrigin::Imported { .. }
        | BindingOrigin::BuiltinTarget { .. }
        | BindingOrigin::ArithmeticAssignment { .. }
        | BindingOrigin::Declaration { .. }
        | BindingOrigin::Nameref { .. } => None,
    }
}

fn function_definition_is_in_file_scope(checker: &Checker<'_>, binding: BindingId) -> bool {
    let binding = checker.semantic().binding(binding);
    matches!(
        checker.semantic().scope(binding.scope).kind,
        ScopeKind::File
    )
}

fn file_has_file_scope_exit(checker: &Checker<'_>) -> bool {
    checker.facts().commands().iter().any(|command| {
        command.effective_or_literal_name() == Some("exit")
            && checker
                .semantic()
                .flow_context_at(&command.stmt().span)
                .is_some_and(|context| !context.in_function && !context.in_subshell)
    })
}

fn file_has_top_level_exit_command(checker: &Checker<'_>) -> bool {
    checker.facts().commands().iter().any(|command| {
        command.effective_or_literal_name() == Some("exit")
            && checker.facts().command_parent_id(command.id()).is_none()
            && checker
                .semantic()
                .flow_context_at(&command.stmt().span)
                .is_some_and(|context| !context.in_function && !context.in_subshell)
    })
}

fn span_is_inside_unreached_function(span: Span, function_spans: &[Span]) -> bool {
    function_spans.iter().any(|function| {
        function.start.offset < span.start.offset && span.end.offset <= function.end.offset
    })
}

fn span_matches_short_circuit_skip(
    span: Span,
    short_circuit_lists: &[ListFact<'_>],
    commands: CommandFacts<'_, '_>,
    semantic_analysis: &SemanticAnalysis<'_>,
) -> bool {
    short_circuit_lists.iter().any(|list| {
        if span == list.span() {
            return true;
        }

        if list.segments().len() < 3 || !span_contained_by(span, list.span()) {
            return false;
        }

        if !list_starts_with_condition(list, commands, semantic_analysis) {
            return false;
        }

        list.segments()
            .iter()
            .enumerate()
            .any(|(index, segment)| index > 0 && span.start == segment.span().start)
    })
}

fn list_starts_with_condition(
    list: &ListFact<'_>,
    commands: CommandFacts<'_, '_>,
    semantic_analysis: &SemanticAnalysis<'_>,
) -> bool {
    let Some(first_segment) = list.segments().first() else {
        return false;
    };
    let Some(command) = commands
        .iter()
        .find(|command| command.id() == first_segment.command_id())
    else {
        return false;
    };

    let starts_like_condition = command.simple_test().is_some()
        || command.conditional().is_some()
        || matches!(
            command.effective_or_literal_name(),
            Some("[" | "test" | "true" | "false")
        );

    starts_like_condition && !command_name_resolves_to_function(&command, semantic_analysis)
}

fn command_name_resolves_to_function(
    command: &CommandFact<'_>,
    semantic_analysis: &SemanticAnalysis<'_>,
) -> bool {
    if wrapper_name_resolves_to_function(command, semantic_analysis) {
        return true;
    }

    if command.has_wrapper(WrapperKind::Command) || command.has_wrapper(WrapperKind::Builtin) {
        return false;
    }

    let Some(name) = command.effective_or_literal_name() else {
        return false;
    };
    let Some(name_span) = command.body_word_span() else {
        return false;
    };
    let name = Name::from(name);

    semantic_analysis
        .visible_function_binding_at_call(&name, name_span)
        .is_some()
}

fn wrapper_name_resolves_to_function(
    command: &CommandFact<'_>,
    semantic_analysis: &SemanticAnalysis<'_>,
) -> bool {
    if command.wrappers().is_empty() {
        return false;
    }
    let Some(name) = command.literal_name() else {
        return false;
    };
    let AstCommand::Simple(simple) = command.command() else {
        return false;
    };

    semantic_analysis
        .visible_function_binding_at_call(&Name::from(name), simple.name.span)
        .is_some()
}

fn outermost_unreachable_spans(mut spans: Vec<shuck_ast::Span>) -> Vec<shuck_ast::Span> {
    spans.sort_by(|left, right| {
        left.start
            .offset
            .cmp(&right.start.offset)
            .then_with(|| right.end.offset.cmp(&left.end.offset))
    });

    let mut outermost = Vec::new();
    for span in spans {
        if outermost
            .iter()
            .any(|outer| span_contained_by(span, *outer))
        {
            continue;
        }
        if outermost.contains(&span) {
            continue;
        }
        outermost.push(span);
    }
    outermost
}

fn span_contained_by(inner: shuck_ast::Span, outer: shuck_ast::Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn trim_trailing_terminator(span: Span, source: &str) -> Span {
    let trimmed = span.slice(source).trim_end_matches(char::is_whitespace);
    let trimmed = trimmed
        .strip_suffix("&&")
        .or_else(|| trimmed.strip_suffix("||"))
        .unwrap_or(trimmed);
    let trimmed = trimmed
        .trim_end_matches(char::is_whitespace)
        .trim_end_matches(';')
        .trim_end_matches(char::is_whitespace);
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}
