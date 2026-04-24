use shuck_ast::Span;
use shuck_semantic::{ScopeKind, SourceRef, SourceRefKind};

use crate::{Checker, CommandFact, Rule, ShellDialect, Violation};

use super::source_common::source_anchor_span_for_command_fact;

pub struct SourceInsideFunctionInSh;

impl Violation for SourceInsideFunctionInSh {
    fn rule() -> Rule {
        Rule::SourceInsideFunctionInSh
    }

    fn message(&self) -> String {
        "`source` inside a function is not portable in `sh` scripts".to_owned()
    }
}

pub fn source_inside_function_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .semantic()
        .source_refs()
        .iter()
        .filter(|source_ref| {
            matches!(
                source_ref.kind,
                SourceRefKind::Directive(_) | SourceRefKind::DirectiveDevNull
            )
        })
        .filter_map(|source_ref| source_command_for_ref(checker, source_ref))
        .filter(|command| inside_function(checker, command.span()))
        .map(|command| source_anchor_span_for_command_fact(command, checker.source()))
        .collect::<Vec<_>>();
    checker.report_all(spans, || SourceInsideFunctionInSh);
}

fn source_command_for_ref<'a>(
    checker: &'a Checker<'_>,
    source_ref: &SourceRef,
) -> Option<&'a CommandFact<'a>> {
    checker
        .facts()
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("source") && same_start(fact.span(), source_ref.span))
}

fn same_start(left: Span, right: Span) -> bool {
    left.start.offset == right.start.offset
}

fn inside_function(checker: &Checker<'_>, span: Span) -> bool {
    let scope = checker.semantic().scope_at(span.start.offset);
    checker
        .semantic()
        .ancestor_scopes(scope)
        .any(|scope| matches!(checker.semantic().scope_kind(scope), ScopeKind::Function(_)))
}
