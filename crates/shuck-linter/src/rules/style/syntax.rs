use shuck_ast::{BuiltinCommand, Command, Word, WordPart};

pub fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}

pub fn is_simple_command_named(command: &Command, source: &str, name: &str) -> bool {
    match command {
        Command::Simple(command) => {
            static_word_text(&command.name, source).as_deref() == Some(name)
        }
        _ => false,
    }
}

pub fn word_contains_command_substitution(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| matches!(part, WordPart::CommandSubstitution(_)))
}

pub fn word_is_plain_command_substitution(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [WordPart::CommandSubstitution(_)])
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
