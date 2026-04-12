use rustc_hash::FxHashMap;
use shuck_ast::{BuiltinCommand, Command, Span};
use shuck_semantic::{ScopeId, ScopeKind};

use crate::facts::CommandFact;
use crate::{Checker, Rule, Violation, word_is_standalone_status_capture};

pub struct RedundantReturnStatus;

impl Violation for RedundantReturnStatus {
    fn rule() -> Rule {
        Rule::RedundantReturnStatus
    }

    fn message(&self) -> String {
        "function already propagates the last command status".to_owned()
    }
}

pub fn redundant_return_status(checker: &mut Checker) {
    let mut spans = Vec::new();
    let mut last_structural_by_function: FxHashMap<ScopeId, &CommandFact<'_>> =
        FxHashMap::default();

    let mut structural_commands = checker.facts().structural_commands().collect::<Vec<_>>();
    structural_commands.sort_by_key(|fact| fact.span().start.offset);

    for fact in structural_commands {
        let Some(function_scope) = enclosing_function_scope(checker, fact.span()) else {
            continue;
        };

        let previous = last_structural_by_function.insert(function_scope, fact);

        let Command::Builtin(BuiltinCommand::Return(command)) = fact.command() else {
            continue;
        };
        let Some(code) = command.code.as_ref() else {
            continue;
        };
        if !word_is_standalone_status_capture(code) {
            continue;
        }

        let Some(previous) = previous else {
            continue;
        };
        if !is_plain_command(previous.command()) {
            continue;
        }

        spans.push(code.span);
    }

    checker.report_all_dedup(spans, || RedundantReturnStatus);
}

fn enclosing_function_scope(checker: &Checker<'_>, span: Span) -> Option<ScopeId> {
    let scope = checker.semantic().scope_at(span.start.offset);
    checker.semantic().ancestor_scopes(scope).find(|scope| {
        matches!(
            checker.semantic().scope_kind(*scope),
            ScopeKind::Function(_)
        )
    })
}

fn is_plain_command(command: &Command) -> bool {
    matches!(
        command,
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_)
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_returning_the_previous_status_inside_functions() {
        let source = "\
#!/bin/sh
f() {
  false
  return $?
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::RedundantReturnStatus);
    }

    #[test]
    fn ignores_returns_outside_functions_and_with_explicit_statuses() {
        let source = "\
#!/bin/sh
return $?
f() {
  return 1
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
