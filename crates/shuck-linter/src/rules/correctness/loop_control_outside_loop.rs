use shuck_ast::{BuiltinCommand, Command};

use crate::{Checker, Rule, Violation};

pub struct LoopControlOutsideLoop {
    pub keyword: &'static str,
}

impl Violation for LoopControlOutsideLoop {
    fn rule() -> Rule {
        Rule::LoopControlOutsideLoop
    }

    fn message(&self) -> String {
        format!("`{}` is only valid inside a loop", self.keyword)
    }
}

pub fn loop_control_outside_loop(checker: &mut Checker) {
    let violations = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Builtin(BuiltinCommand::Break(command)) => Some((command.span, "break")),
            Command::Builtin(BuiltinCommand::Continue(command)) => Some((command.span, "continue")),
            _ => None,
        })
        .filter(|(span, _)| {
            checker
                .semantic()
                .flow_context_at(span)
                .map(|context| context.loop_depth == 0)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    for (span, keyword) in violations {
        checker.report(LoopControlOutsideLoop { keyword }, span);
    }
}
