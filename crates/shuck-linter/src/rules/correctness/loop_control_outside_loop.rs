use shuck_ast::{BuiltinCommand, Command};

use crate::rules::common::query::{self, CommandWalkOptions};
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
    let mut violations = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            let context = visit.context;
            if context.loop_depth > 0 {
                return;
            }

            match command {
                Command::Builtin(BuiltinCommand::Break(command)) => {
                    violations.push((command.span, "break"));
                }
                Command::Builtin(BuiltinCommand::Continue(command)) => {
                    violations.push((command.span, "continue"));
                }
                _ => {}
            }
        },
    );

    for (span, keyword) in violations {
        checker.report(LoopControlOutsideLoop { keyword }, span);
    }
}
