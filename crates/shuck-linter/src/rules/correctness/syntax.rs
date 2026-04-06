use shuck_ast::{Assignment, SimpleCommand, TextSize, Word};
use shuck_indexer::{Indexer, RegionKind};

pub use crate::rules::common::word::static_word_text;

fn simple_command_name(command: &SimpleCommand, source: &str) -> Option<String> {
    static_word_text(&command.name, source)
}

pub fn assignment_target_name(assignment: &Assignment) -> &str {
    assignment.name.as_str()
}

pub fn simple_test_operands<'a>(command: &'a SimpleCommand, source: &str) -> Option<&'a [Word]> {
    let name = simple_command_name(command, source)?;
    match name.as_str() {
        "[" => {
            let (closing_bracket, operands) = command.args.split_last()?;
            (static_word_text(closing_bracket, source).as_deref() == Some("]")).then_some(operands)
        }
        "test" => Some(&command.args),
        _ => None,
    }
}

pub fn word_is_double_quoted(indexer: &Indexer, word: &Word) -> bool {
    let span = word.part_span(0).unwrap_or(word.span);
    indexer
        .region_index()
        .region_at(TextSize::new(span.start.offset as u32))
        == Some(RegionKind::DoubleQuoted)
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, DeclOperand};
    use shuck_parser::parser::Parser;

    use super::{assignment_target_name, simple_command_name};

    fn parse_first_command(source: &str) -> Command {
        let output = Parser::new(source).parse().unwrap();
        output.script.commands.into_iter().next().unwrap()
    }

    #[test]
    fn simple_command_name_returns_static_command_name() {
        let source = "printf '%s\\n' hello\n";
        let command = parse_first_command(source);
        let Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            simple_command_name(&command, source).as_deref(),
            Some("printf")
        );
    }

    #[test]
    fn simple_command_name_returns_none_for_dynamic_command_name() {
        let source = "\"$tool\" --help\n";
        let command = parse_first_command(source);
        let Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(simple_command_name(&command, source), None);
    }

    #[test]
    fn assignment_target_name_returns_assignment_name() {
        let source = "export PS1='$PWD'\n";
        let command = parse_first_command(source);
        let Command::Decl(command) = command else {
            panic!("expected declaration command");
        };
        let DeclOperand::Assignment(assignment) = &command.operands[0] else {
            panic!("expected declaration assignment");
        };

        assert_eq!(assignment_target_name(assignment), "PS1");
    }
}
