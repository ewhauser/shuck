fn simple_command_name(command: &shuck_ast::SimpleCommand, source: &str) -> Option<String> {
    shuck_ast::static_word_text(&command.name, source).map(|text| text.into_owned())
}

pub fn assignment_target_name(assignment: &shuck_ast::Assignment) -> &str {
    assignment.target.name.as_str()
}

pub fn simple_test_operands<'a>(
    command: &'a shuck_ast::SimpleCommand,
    source: &str,
) -> Option<&'a [shuck_ast::Word]> {
    let name = simple_command_name(command, source)?;
    match name.as_str() {
        "[" => {
            let (closing_bracket, operands) = command.args.split_last()?;
            (shuck_ast::static_word_text(closing_bracket, source).as_deref() == Some("]"))
                .then_some(operands)
        }
        "test" => Some(&command.args),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use super::{assignment_target_name, simple_command_name};

    fn parse_first_command(source: &str) -> shuck_ast::Command {
        let output = Parser::new(source).parse().unwrap();
        output.file.body.stmts.into_iter().next().unwrap().command
    }

    #[test]
    fn simple_command_name_returns_static_command_name() {
        let source = "printf '%s\\n' hello\n";
        let command = parse_first_command(source);
        let shuck_ast::Command::Simple(command) = command else {
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
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(simple_command_name(&command, source), None);
    }

    #[test]
    fn assignment_target_name_returns_assignment_name() {
        let source = "export PS1='$PWD'\n";
        let command = parse_first_command(source);
        let shuck_ast::Command::Decl(command) = command else {
            panic!("expected declaration command");
        };
        let shuck_ast::DeclOperand::Assignment(assignment) = &command.operands[0] else {
            panic!("expected declaration assignment");
        };

        assert_eq!(assignment_target_name(assignment), "PS1");
    }
}
