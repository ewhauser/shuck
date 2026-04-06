use shuck_ast::{BuiltinCommand, Command, Word};

pub use crate::rules::common::word::static_word_text;

pub fn is_simple_command_named(command: &Command, source: &str, name: &str) -> bool {
    match command {
        Command::Simple(command) => {
            static_word_text(&command.name, source).as_deref() == Some(name)
        }
        _ => false,
    }
}

pub fn visit_argument_words(command: &Command, mut visitor: impl FnMut(&Word)) {
    match command {
        Command::Simple(command) => {
            for word in &command.args {
                visitor(word);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
        },
        _ => {}
    }
}
