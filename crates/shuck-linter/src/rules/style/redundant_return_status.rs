use crate::{Checker, Rule, Violation, word_is_standalone_status_capture};
use shuck_ast::{
    BuiltinCommand, Command, CompoundCommand, FunctionDef, IfCommand, Span, Stmt, StmtSeq,
    StmtTerminator,
};

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
    for header in checker.facts().function_headers() {
        collect_terminal_redundant_return_spans(header.function(), &mut spans);
    }

    checker.report_all_dedup(spans, || RedundantReturnStatus);
}

fn collect_terminal_redundant_return_spans(function: &FunctionDef, spans: &mut Vec<Span>) {
    collect_terminal_redundant_return_spans_in_stmt(&function.body, spans);
}

fn collect_terminal_redundant_return_spans_in_stmt(stmt: &Stmt, spans: &mut Vec<Span>) {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            collect_terminal_redundant_return_spans_in_seq(commands, spans);
        }
        Command::Compound(CompoundCommand::If(command)) => {
            collect_terminal_redundant_return_spans_in_if(command, spans);
        }
        _ => {}
    }
}

fn collect_terminal_redundant_return_spans_in_if(command: &IfCommand, spans: &mut Vec<Span>) {
    collect_terminal_redundant_return_spans_in_seq(&command.then_branch, spans);
    for (_, branch) in &command.elif_branches {
        collect_terminal_redundant_return_spans_in_seq(branch, spans);
    }
    if let Some(branch) = &command.else_branch {
        collect_terminal_redundant_return_spans_in_seq(branch, spans);
    }
}

fn collect_terminal_redundant_return_spans_in_seq(commands: &StmtSeq, spans: &mut Vec<Span>) {
    if let Some(span) = terminal_redundant_return_span(commands) {
        spans.push(span);
    }

    let Some(last) = commands.last() else {
        return;
    };
    collect_terminal_redundant_return_spans_in_stmt(last, spans);
}

fn terminal_redundant_return_span(commands: &StmtSeq) -> Option<Span> {
    let [.., previous, last] = commands.as_slice() else {
        return None;
    };
    if !stmt_is_plain_command(previous) {
        return None;
    }

    let Command::Builtin(BuiltinCommand::Return(command)) = &last.command else {
        return None;
    };
    let Some(code) = command.code.as_ref() else {
        return None;
    };
    word_is_standalone_status_capture(code).then_some(code.span)
}

fn stmt_is_plain_command(stmt: &Stmt) -> bool {
    if stmt.negated || matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
        return false;
    }

    matches!(
        stmt.command,
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

    #[test]
    fn ignores_non_terminal_returns_inside_function_branches() {
        let source = "\
#!/bin/sh
f() {
  if cond; then
    false
    return $?
  fi
  echo done
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_terminal_returns_inside_final_if_branches() {
        let source = "\
#!/bin/sh
f() {
  if cond; then
    false
    return $?
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantReturnStatus),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$?");
    }
}
