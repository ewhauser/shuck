use super::*;

pub(super) enum CompoundChild<'a> {
    Stmt(&'a Stmt),
    Sequence(&'a StmtSeq),
}

pub(super) fn for_each_compound_child(
    command: &CompoundCommand,
    mut visitor: impl FnMut(CompoundChild<'_>),
) {
    match command {
        CompoundCommand::If(command) => {
            visitor(CompoundChild::Sequence(&command.condition));
            visitor(CompoundChild::Sequence(&command.then_branch));
            for (condition, body) in &command.elif_branches {
                visitor(CompoundChild::Sequence(condition));
                visitor(CompoundChild::Sequence(body));
            }
            if let Some(body) = &command.else_branch {
                visitor(CompoundChild::Sequence(body));
            }
        }
        CompoundCommand::For(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::Repeat(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::Foreach(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::ArithmeticFor(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::While(command) => {
            visitor(CompoundChild::Sequence(&command.condition));
            visitor(CompoundChild::Sequence(&command.body));
        }
        CompoundCommand::Until(command) => {
            visitor(CompoundChild::Sequence(&command.condition));
            visitor(CompoundChild::Sequence(&command.body));
        }
        CompoundCommand::Case(command) => {
            for item in &command.cases {
                visitor(CompoundChild::Sequence(&item.body));
            }
        }
        CompoundCommand::Select(command) => visitor(CompoundChild::Sequence(&command.body)),
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            visitor(CompoundChild::Sequence(commands));
        }
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = command.command.as_deref() {
                visitor(CompoundChild::Stmt(command));
            }
        }
        CompoundCommand::Coproc(command) => visitor(CompoundChild::Stmt(&command.body)),
        CompoundCommand::Always(command) => {
            visitor(CompoundChild::Sequence(&command.body));
            visitor(CompoundChild::Sequence(&command.always_body));
        }
    }
}
