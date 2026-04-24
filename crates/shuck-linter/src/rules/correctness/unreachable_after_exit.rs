use crate::{Checker, CommandFact, ListFact, Rule, Violation, WrapperKind};
use shuck_ast::{Command as AstCommand, Name, Span};
use shuck_semantic::{SemanticAnalysis, UnreachableCauseKind};

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
                )
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

fn span_matches_short_circuit_skip(
    span: Span,
    short_circuit_lists: &[ListFact<'_>],
    commands: &[CommandFact<'_>],
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
    commands: &[CommandFact<'_>],
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

    starts_like_condition && !command_name_resolves_to_function(command, semantic_analysis)
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
    let Some(wrapper) = command.wrappers().first() else {
        return false;
    };
    let name = match wrapper {
        WrapperKind::Command => "command",
        WrapperKind::Builtin => "builtin",
        _ => return false,
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
