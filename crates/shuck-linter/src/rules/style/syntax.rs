pub fn is_simple_command_named(command: &shuck_ast::Command, source: &str, name: &str) -> bool {
    match command {
        shuck_ast::Command::Simple(command) => {
            shuck_ast::static_word_text(&command.name, source).as_deref() == Some(name)
        }
        _ => false,
    }
}
