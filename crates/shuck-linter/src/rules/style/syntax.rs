use shuck_ast::{Command, static_word_text};

pub fn is_simple_command_named(command: &Command, source: &str, name: &str) -> bool {
    match command {
        Command::Simple(command) => {
            static_word_text(&command.name, source).as_deref() == Some(name)
        }
        _ => false,
    }
}
