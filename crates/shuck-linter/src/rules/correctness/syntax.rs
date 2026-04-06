use shuck_ast::{Assignment, Command, SimpleCommand, TextSize, Word, WordPart};
use shuck_indexer::{Indexer, RegionKind};

pub fn simple_command_name(command: &SimpleCommand, source: &str) -> Option<String> {
    static_word_text(&command.name, source)
}

pub fn effective_command_name(command: &Command, source: &str) -> Option<String> {
    let Command::Simple(command) = command else {
        return None;
    };

    let name = simple_command_name(command, source)?;
    let effective = match name.as_str() {
        "command" => command_wrapper_target(command, source),
        "exec" => exec_wrapper_target(command, source),
        "busybox" => first_static_arg(command, source),
        "find" => find_exec_target(command, source),
        "git" => git_subcommand_name(command, source),
        "mumps" => mumps_subcommand_name(command, source),
        _ => None,
    };

    Some(effective.unwrap_or(name))
}

pub fn assignment_target_name(assignment: &Assignment) -> &str {
    assignment.name.as_str()
}

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
        Command::Simple(command) => simple_command_name(command, source).as_deref() == Some(name),
        _ => false,
    }
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

pub fn word_has_expansion(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| !matches!(part, WordPart::Literal(_)))
}

pub fn word_is_double_quoted(indexer: &Indexer, word: &Word) -> bool {
    let span = word.part_span(0).unwrap_or(word.span);
    indexer
        .region_index()
        .region_at(TextSize::new(span.start.offset as u32))
        == Some(RegionKind::DoubleQuoted)
}

fn first_static_arg(command: &SimpleCommand, source: &str) -> Option<String> {
    command
        .args
        .first()
        .and_then(|arg| static_word_text(arg, source))
}

fn command_wrapper_target(command: &SimpleCommand, source: &str) -> Option<String> {
    let mut index = 0usize;

    while index < command.args.len() {
        let arg = static_word_text(&command.args[index], source)?;
        match arg.as_str() {
            "--" => {
                return command
                    .args
                    .get(index + 1)
                    .and_then(|arg| static_word_text(arg, source));
            }
            "-p" => index += 1,
            "-v" | "-V" => return None,
            _ if arg.starts_with('-') => return None,
            _ => return Some(arg),
        }
    }

    None
}

fn exec_wrapper_target(command: &SimpleCommand, source: &str) -> Option<String> {
    let mut index = 0usize;

    while index < command.args.len() {
        let arg = static_word_text(&command.args[index], source)?;
        match arg.as_str() {
            "--" => {
                return command
                    .args
                    .get(index + 1)
                    .and_then(|arg| static_word_text(arg, source));
            }
            "-c" | "-l" => index += 1,
            "-a" => {
                static_word_text(command.args.get(index + 1)?, source)?;
                index += 2;
            }
            _ if arg.starts_with('-') => return None,
            _ => return Some(arg),
        }
    }

    None
}

fn find_exec_target(command: &SimpleCommand, source: &str) -> Option<String> {
    for (index, arg) in command.args.iter().enumerate() {
        let arg = static_word_text(arg, source)?;
        if matches!(arg.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            return command
                .args
                .get(index + 1)
                .and_then(|arg| static_word_text(arg, source));
        }
    }

    None
}

fn git_subcommand_name(command: &SimpleCommand, source: &str) -> Option<String> {
    (command
        .args
        .first()
        .and_then(|arg| static_word_text(arg, source))
        .as_deref()
        == Some("filter-branch"))
    .then(|| "git filter-branch".to_owned())
}

fn mumps_subcommand_name(command: &SimpleCommand, source: &str) -> Option<String> {
    let run_flag = command
        .args
        .first()
        .and_then(|arg| static_word_text(arg, source))?;
    let entrypoint = command
        .args
        .get(1)
        .and_then(|arg| static_word_text(arg, source))?;
    if run_flag == "-run" && matches!(entrypoint.as_str(), "%XCMD" | "LOOP%XCMD") {
        Some(format!("mumps -run {entrypoint}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, DeclOperand};
    use shuck_parser::parser::Parser;

    use super::{assignment_target_name, effective_command_name, simple_command_name};

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
    fn effective_command_name_unwraps_known_wrappers() {
        let cases = [
            ("command jq '$__loc__'\n", Some("jq")),
            ("exec jq '$__loc__'\n", Some("jq")),
            ("exec -c -a foo jq '$__loc__'\n", Some("jq")),
            ("busybox awk '{print $1}'\n", Some("awk")),
            ("find . -exec awk '{print $1}' {} \\;\n", Some("awk")),
            (
                "git filter-branch 'test $GIT_COMMIT'\n",
                Some("git filter-branch"),
            ),
            (
                "mumps -run %XCMD 'W $O(^GLOBAL(5))'\n",
                Some("mumps -run %XCMD"),
            ),
            ("printf '%s\\n' hello\n", Some("printf")),
        ];

        for (source, expected) in cases {
            let command = parse_first_command(source);
            assert_eq!(
                effective_command_name(&command, source).as_deref(),
                expected
            );
        }
    }

    #[test]
    fn effective_command_name_falls_back_when_wrapper_target_is_not_static() {
        let cases = [
            ("command \"$tool\" '$__loc__'\n", Some("command")),
            ("exec \"$tool\" '$__loc__'\n", Some("exec")),
            ("find . -exec \"$tool\" {} \\;\n", Some("find")),
            ("git \"$subcommand\" 'test $GIT_COMMIT'\n", Some("git")),
        ];

        for (source, expected) in cases {
            let command = parse_first_command(source);
            assert_eq!(
                effective_command_name(&command, source).as_deref(),
                expected
            );
        }
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
