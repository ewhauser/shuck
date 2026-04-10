use shuck_ast::{
    Assignment, BuiltinCommand, Command, DeclClause, DeclOperand, Redirect, SimpleCommand, Span,
    Word, WordPart,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapperKind {
    Command,
    Builtin,
    Exec,
    Busybox,
    FindExec,
    FindExecDir,
    SudoFamily,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationKind {
    Export,
    Local,
    Declare,
    Typeset,
    Other(String),
}

impl DeclarationKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Export => "export",
            Self::Local => "local",
            Self::Declare => "declare",
            Self::Typeset => "typeset",
            Self::Other(name) => name.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NormalizedDeclaration<'a> {
    pub kind: DeclarationKind,
    pub readonly_flag: bool,
    pub span: Span,
    pub head_span: Span,
    pub redirects: &'a [Redirect],
    pub assignments: &'a [Assignment],
    pub operands: &'a [DeclOperand],
    pub assignment_operands: Vec<&'a Assignment>,
}

#[derive(Debug, Clone)]
pub struct NormalizedCommand<'a> {
    pub literal_name: Option<String>,
    pub effective_name: Option<String>,
    pub wrappers: Vec<WrapperKind>,
    pub body_span: Span,
    pub body_word_span: Option<Span>,
    pub body_words: Vec<&'a Word>,
    pub declaration: Option<NormalizedDeclaration<'a>>,
}

impl<'a> NormalizedCommand<'a> {
    pub fn effective_or_literal_name(&self) -> Option<&str> {
        self.effective_name
            .as_deref()
            .or(self.literal_name.as_deref())
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.effective_name.as_deref() == Some(name)
    }

    pub fn has_wrapper(&self, wrapper: WrapperKind) -> bool {
        self.wrappers.contains(&wrapper)
    }

    pub fn body_name_word(&self) -> Option<&'a Word> {
        self.body_words.first().copied()
    }

    pub fn body_word_span(&self) -> Option<Span> {
        self.body_word_span
    }

    pub fn body_args(&self) -> &[&'a Word] {
        self.body_words.split_first().map_or(&[], |(_, rest)| rest)
    }
}

pub(crate) fn normalize_command<'a>(command: &'a Command, source: &str) -> NormalizedCommand<'a> {
    match command {
        Command::Simple(command) => normalize_simple_command(command, source),
        Command::Decl(command) => normalize_decl_command(command, source),
        Command::Builtin(command) => {
            let name = builtin_name(command).to_owned();
            NormalizedCommand {
                literal_name: Some(name.clone()),
                effective_name: Some(name),
                wrappers: Vec::new(),
                body_span: builtin_span(command),
                body_word_span: None,
                body_words: Vec::new(),
                declaration: None,
            }
        }
        Command::Binary(command) => empty_normalized_command(command.span),
        Command::Compound(command) => empty_normalized_command(compound_span(command)),
        Command::Function(command) => empty_normalized_command(command.span),
        Command::AnonymousFunction(command) => empty_normalized_command(command.span),
    }
}

fn normalize_simple_command<'a>(command: &'a SimpleCommand, source: &str) -> NormalizedCommand<'a> {
    let words = std::iter::once(&command.name)
        .chain(command.args.iter())
        .collect::<Vec<_>>();
    let literal_name = static_word_text(&command.name, source);
    let mut normalized = NormalizedCommand {
        literal_name: literal_name.clone(),
        effective_name: literal_name.clone(),
        wrappers: Vec::new(),
        body_span: command.name.span,
        body_word_span: Some(command.name.span),
        body_words: literal_name
            .as_ref()
            .map_or_else(Vec::new, |_| words.clone()),
        declaration: None,
    };
    let mut current_index = 0usize;

    while let Some(current_name) = normalized.effective_name.clone() {
        let Some(resolution) =
            resolve_command_resolution(&words, current_index, current_name.as_str(), source)
        else {
            break;
        };

        match resolution {
            CommandResolution::Alias {
                effective_name,
                body_index,
            } => {
                normalized.effective_name = Some(effective_name);
                normalized.body_span = words[body_index].span;
                normalized.body_word_span = Some(words[body_index].span);
                normalized.body_words = words[body_index..].to_vec();
                break;
            }
            CommandResolution::Wrapper { kind, target_index } => {
                normalized.wrappers.push(kind);

                let Some(target_index) = target_index else {
                    normalized.effective_name = None;
                    normalized.body_word_span = None;
                    normalized.body_words.clear();
                    break;
                };

                normalized.body_span = words[target_index].span;
                normalized.body_word_span = Some(words[target_index].span);
                normalized.body_words = words[target_index..].to_vec();
                normalized.effective_name = static_word_text(words[target_index], source);
                current_index = target_index;

                if normalized.effective_name.is_none() {
                    normalized.body_words.clear();
                    break;
                }
            }
        }
    }

    normalized
}

fn normalize_decl_command<'a>(command: &'a DeclClause, source: &str) -> NormalizedCommand<'a> {
    let raw_kind = command.variant.as_ref().to_owned();
    let assignment_operands = command
        .operands
        .iter()
        .filter_map(|operand| match operand {
            DeclOperand::Assignment(assignment) => Some(assignment),
            DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Dynamic(_) => None,
        })
        .collect::<Vec<_>>();

    NormalizedCommand {
        literal_name: Some(raw_kind.clone()),
        effective_name: Some(raw_kind.clone()),
        wrappers: Vec::new(),
        body_span: command.variant_span,
        body_word_span: None,
        body_words: Vec::new(),
        declaration: Some(NormalizedDeclaration {
            kind: declaration_kind(raw_kind),
            readonly_flag: declaration_has_readonly_flag(command, source),
            span: command.span,
            head_span: declaration_head_span(command),
            redirects: &[],
            assignments: &command.assignments,
            operands: &command.operands,
            assignment_operands,
        }),
    }
}

fn declaration_kind(raw_kind: String) -> DeclarationKind {
    match raw_kind.as_str() {
        "export" => DeclarationKind::Export,
        "local" => DeclarationKind::Local,
        "declare" => DeclarationKind::Declare,
        "typeset" => DeclarationKind::Typeset,
        _ => DeclarationKind::Other(raw_kind),
    }
}

fn declaration_has_readonly_flag(command: &DeclClause, source: &str) -> bool {
    matches!(command.variant.as_ref(), "local" | "declare" | "typeset")
        && command.operands.iter().any(|operand| {
            let DeclOperand::Flag(word) = operand else {
                return false;
            };

            static_word_text(word, source)
                .is_some_and(|text| text.starts_with('-') && text.contains('r'))
        })
}

fn declaration_head_span(command: &DeclClause) -> Span {
    let end = command
        .operands
        .last()
        .map_or(command.variant_span.end, declaration_operand_head_end);
    Span::from_positions(command.variant_span.start, end)
}

fn declaration_operand_head_end(operand: &DeclOperand) -> shuck_ast::Position {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span.end,
        DeclOperand::Name(name) => name.span.end,
        DeclOperand::Assignment(assignment) => assignment.target.name_span.end,
    }
}

fn empty_normalized_command<'a>(span: Span) -> NormalizedCommand<'a> {
    NormalizedCommand {
        literal_name: None,
        effective_name: None,
        wrappers: Vec::new(),
        body_span: span,
        body_word_span: None,
        body_words: Vec::new(),
        declaration: None,
    }
}

fn compound_span(command: &shuck_ast::CompoundCommand) -> Span {
    match command {
        shuck_ast::CompoundCommand::If(command) => command.span,
        shuck_ast::CompoundCommand::For(command) => command.span,
        shuck_ast::CompoundCommand::Repeat(command) => command.span,
        shuck_ast::CompoundCommand::Foreach(command) => command.span,
        shuck_ast::CompoundCommand::ArithmeticFor(command) => command.span,
        shuck_ast::CompoundCommand::While(command) => command.span,
        shuck_ast::CompoundCommand::Until(command) => command.span,
        shuck_ast::CompoundCommand::Case(command) => command.span,
        shuck_ast::CompoundCommand::Select(command) => command.span,
        shuck_ast::CompoundCommand::Subshell(commands)
        | shuck_ast::CompoundCommand::BraceGroup(commands) => commands.span,
        shuck_ast::CompoundCommand::Arithmetic(command) => command.span,
        shuck_ast::CompoundCommand::Time(command) => command.span,
        shuck_ast::CompoundCommand::Conditional(command) => command.span,
        shuck_ast::CompoundCommand::Coproc(command) => command.span,
        shuck_ast::CompoundCommand::Always(command) => command.span,
    }
}

enum CommandResolution {
    Alias {
        effective_name: String,
        body_index: usize,
    },
    Wrapper {
        kind: WrapperKind,
        target_index: Option<usize>,
    },
}

fn resolve_command_resolution(
    words: &[&Word],
    current_index: usize,
    current_name: &str,
    source: &str,
) -> Option<CommandResolution> {
    match current_name {
        "command" => Some(CommandResolution::Wrapper {
            kind: WrapperKind::Command,
            target_index: command_wrapper_target_index(words, current_index, source),
        }),
        "builtin" => Some(CommandResolution::Wrapper {
            kind: WrapperKind::Builtin,
            target_index: words.get(current_index + 1).map(|_| current_index + 1),
        }),
        "exec" => Some(CommandResolution::Wrapper {
            kind: WrapperKind::Exec,
            target_index: exec_wrapper_target_index(words, current_index, source),
        }),
        "busybox" => Some(CommandResolution::Wrapper {
            kind: WrapperKind::Busybox,
            target_index: words.get(current_index + 1).map(|_| current_index + 1),
        }),
        "find" => {
            find_exec_target_index(words, current_index, source).map(|(kind, target_index)| {
                CommandResolution::Wrapper {
                    kind,
                    target_index: Some(target_index),
                }
            })
        }
        "sudo" | "doas" | "run0" => Some(CommandResolution::Wrapper {
            kind: WrapperKind::SudoFamily,
            target_index: sudo_family_target_index(words, current_index, source),
        }),
        "git" => git_filter_branch_resolution(words, current_index, source),
        "mumps" => mumps_run_resolution(words, current_index, source),
        _ => None,
    }
}

fn command_wrapper_target_index(
    words: &[&Word],
    current_index: usize,
    source: &str,
) -> Option<usize> {
    let mut index = current_index + 1;

    while index < words.len() {
        let Some(arg) = static_word_text(words[index], source) else {
            return Some(index);
        };

        match arg.as_str() {
            "--" => return words.get(index + 1).map(|_| index + 1),
            "-p" => index += 1,
            "-v" | "-V" => return None,
            _ if arg.starts_with('-') => return None,
            _ => return Some(index),
        }
    }

    None
}

fn exec_wrapper_target_index(words: &[&Word], current_index: usize, source: &str) -> Option<usize> {
    let mut index = current_index + 1;

    while index < words.len() {
        let Some(arg) = static_word_text(words[index], source) else {
            return Some(index);
        };

        match arg.as_str() {
            "--" => return words.get(index + 1).map(|_| index + 1),
            "-c" | "-l" => index += 1,
            "-a" => {
                static_word_text(words.get(index + 1)?, source)?;
                index += 2;
            }
            _ if arg.starts_with('-') => return None,
            _ => return Some(index),
        }
    }

    None
}

fn find_exec_target_index(
    words: &[&Word],
    current_index: usize,
    source: &str,
) -> Option<(WrapperKind, usize)> {
    for index in current_index + 1..words.len() {
        let Some(arg) = static_word_text(words[index], source) else {
            continue;
        };
        match arg.as_str() {
            "-exec" | "-ok" => {
                return words
                    .get(index + 1)
                    .map(|_| (WrapperKind::FindExec, index + 1));
            }
            "-execdir" => {
                return words
                    .get(index + 1)
                    .map(|_| (WrapperKind::FindExecDir, index + 1));
            }
            "-okdir" => {
                return words
                    .get(index + 1)
                    .map(|_| (WrapperKind::FindExec, index + 1));
            }
            _ => {}
        }
    }

    None
}

fn sudo_family_target_index(words: &[&Word], current_index: usize, source: &str) -> Option<usize> {
    let mut index = current_index + 1;

    while index < words.len() {
        let Some(arg) = static_word_text(words[index], source) else {
            return Some(index);
        };

        if arg == "--" {
            return words.get(index + 1).map(|_| index + 1);
        }

        if !arg.starts_with('-') || arg == "-" {
            return Some(index);
        }

        if arg.len() == 2 && matches!(arg.as_str(), "-l" | "-v" | "-V") {
            return None;
        }

        if sudo_option_takes_value(arg.as_str()) {
            if arg.len() == 2 {
                words.get(index + 1)?;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if arg.starts_with("--") && !matches!(arg.as_str(), "--preserve-env" | "--login") {
            return None;
        }

        index += 1;
    }

    None
}

fn sudo_option_takes_value(arg: &str) -> bool {
    matches!(
        arg.chars().nth(1),
        Some('u' | 'g' | 'h' | 'p' | 'C' | 'D' | 'R' | 'T' | 'r' | 't')
    )
}

fn git_filter_branch_resolution(
    words: &[&Word],
    current_index: usize,
    source: &str,
) -> Option<CommandResolution> {
    (words
        .get(current_index + 1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("filter-branch"))
    .then(|| CommandResolution::Alias {
        effective_name: "git filter-branch".to_owned(),
        body_index: current_index + 1,
    })
}

fn mumps_run_resolution(
    words: &[&Word],
    current_index: usize,
    source: &str,
) -> Option<CommandResolution> {
    let run_flag = words
        .get(current_index + 1)
        .and_then(|word| static_word_text(word, source))?;
    let entrypoint = words
        .get(current_index + 2)
        .and_then(|word| static_word_text(word, source))?;

    if run_flag == "-run" && matches!(entrypoint.as_str(), "%XCMD" | "LOOP%XCMD") {
        Some(CommandResolution::Alias {
            effective_name: format!("mumps -run {entrypoint}"),
            body_index: current_index + 2,
        })
    } else {
        None
    }
}

fn builtin_name(command: &BuiltinCommand) -> &'static str {
    match command {
        BuiltinCommand::Break(_) => "break",
        BuiltinCommand::Continue(_) => "continue",
        BuiltinCommand::Return(_) => "return",
        BuiltinCommand::Exit(_) => "exit",
    }
}

fn builtin_span(command: &BuiltinCommand) -> Span {
    match command {
        BuiltinCommand::Break(command) => command.span,
        BuiltinCommand::Continue(command) => command.span,
        BuiltinCommand::Return(command) => command.span,
        BuiltinCommand::Exit(command) => command.span,
    }
}

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use shuck_ast::Command;
    use shuck_parser::parser::Parser;

    use super::{DeclarationKind, WrapperKind, normalize_command};

    fn parse_first_command(source: &str) -> Command {
        let output = Parser::new(source).parse().unwrap();
        output.file.body.stmts.into_iter().next().unwrap().command
    }

    #[test]
    fn normalize_command_peels_wrappers_and_aliases() {
        let cases = [
            (
                "command printf '%s\\n' hi\n",
                Some("command"),
                Some("printf"),
                vec![WrapperKind::Command],
                ("printf", Some("'%s\\n'")),
            ),
            (
                "builtin read line\n",
                Some("builtin"),
                Some("read"),
                vec![WrapperKind::Builtin],
                ("read", Some("line")),
            ),
            (
                "exec printf '%s\\n' hi\n",
                Some("exec"),
                Some("printf"),
                vec![WrapperKind::Exec],
                ("printf", Some("'%s\\n'")),
            ),
            (
                "busybox awk '{print $1}'\n",
                Some("busybox"),
                Some("awk"),
                vec![WrapperKind::Busybox],
                ("awk", Some("'{print $1}'")),
            ),
            (
                "find . -exec awk '{print $1}' {} \\;\n",
                Some("find"),
                Some("awk"),
                vec![WrapperKind::FindExec],
                ("awk", Some("'{print $1}'")),
            ),
            (
                "find . -execdir sh -c 'echo {}' \\;\n",
                Some("find"),
                Some("sh"),
                vec![WrapperKind::FindExecDir],
                ("sh", Some("-c")),
            ),
            (
                "sudo -u root tee /tmp/out >/dev/null\n",
                Some("sudo"),
                Some("tee"),
                vec![WrapperKind::SudoFamily],
                ("tee", Some("/tmp/out")),
            ),
            (
                "sudo -- tee /tmp/out >/dev/null\n",
                Some("sudo"),
                Some("tee"),
                vec![WrapperKind::SudoFamily],
                ("tee", Some("/tmp/out")),
            ),
            (
                "git filter-branch 'test $GIT_COMMIT'\n",
                Some("git"),
                Some("git filter-branch"),
                Vec::new(),
                ("filter-branch", Some("'test $GIT_COMMIT'")),
            ),
            (
                "mumps -run %XCMD 'W $O(^GLOBAL(5))'\n",
                Some("mumps"),
                Some("mumps -run %XCMD"),
                Vec::new(),
                ("%XCMD", Some("'W $O(^GLOBAL(5))'")),
            ),
        ];

        for (source, literal_name, effective_name, wrappers, (body_name, first_arg)) in cases {
            let command = parse_first_command(source);
            let normalized = normalize_command(&command, source);

            assert_eq!(normalized.literal_name.as_deref(), literal_name);
            assert_eq!(normalized.effective_name.as_deref(), effective_name);
            assert_eq!(normalized.wrappers, wrappers);
            assert_eq!(
                normalized
                    .body_name_word()
                    .map(|word| word.span.slice(source)),
                Some(body_name)
            );
            assert_eq!(
                normalized
                    .body_args()
                    .first()
                    .map(|word| word.span.slice(source)),
                first_arg
            );
        }
    }

    #[test]
    fn normalize_command_keeps_unresolved_wrappers_but_drops_effective_name() {
        let cases = [
            ("command \"$tool\" --help\n", WrapperKind::Command),
            ("exec \"$tool\" --help\n", WrapperKind::Exec),
            ("sudo \"$tool\" > out.txt\n", WrapperKind::SudoFamily),
        ];

        for (source, wrapper) in cases {
            let command = parse_first_command(source);
            let normalized = normalize_command(&command, source);

            assert!(normalized.literal_name.is_some(), "{source}");
            assert_eq!(normalized.effective_name, None, "{source}");
            assert_eq!(normalized.wrappers, vec![wrapper], "{source}");
            assert!(normalized.body_words.is_empty(), "{source}");
        }
    }

    #[test]
    fn normalize_command_collects_declaration_kinds_and_assignments() {
        let cases = [
            (
                "export foo=$(date)\n",
                DeclarationKind::Export,
                vec!["foo"],
                false,
            ),
            (
                "local foo=$(date)\n",
                DeclarationKind::Local,
                vec!["foo"],
                false,
            ),
            (
                "declare -r foo=$(date) bar\n",
                DeclarationKind::Declare,
                vec!["foo"],
                true,
            ),
            (
                "typeset foo=$(date) bar=$(pwd)\n",
                DeclarationKind::Typeset,
                vec!["foo", "bar"],
                false,
            ),
            (
                "readonly foo=$(date)\n",
                DeclarationKind::Other("readonly".to_owned()),
                vec!["foo"],
                false,
            ),
            (
                "local -r foo=$(date)\n",
                DeclarationKind::Local,
                vec!["foo"],
                true,
            ),
        ];

        for (source, kind, assignment_names, readonly) in cases {
            let command = parse_first_command(source);
            let normalized = normalize_command(&command, source);
            let declaration = normalized.declaration.expect("expected declaration");

            assert_eq!(declaration.kind, kind);
            assert_eq!(declaration.readonly_flag, readonly);
            assert_eq!(
                declaration
                    .assignment_operands
                    .iter()
                    .map(|assignment| assignment.target.name.as_str())
                    .collect::<Vec<_>>(),
                assignment_names
            );
        }
    }

    #[test]
    fn normalize_command_tracks_declaration_head_span_without_final_assignment_values() {
        let cases = [
            ("local name=portable\n", "local name"),
            (
                "local plugins_path plugin_path ext_cmd_path ext_cmds plugin\n",
                "local plugins_path plugin_path ext_cmd_path ext_cmds plugin",
            ),
            ("local -r foo=$(date) bar\n", "local -r foo=$(date) bar"),
        ];

        for (source, expected) in cases {
            let command = parse_first_command(source);
            let normalized = normalize_command(&command, source);
            let declaration = normalized.declaration.expect("expected declaration");
            assert_eq!(declaration.head_span.slice(source), expected);
        }
    }

    #[test]
    fn normalize_command_preserves_dynamic_declaration_operands() {
        let source = "declare \"$name=$value\"\n";
        let command = parse_first_command(source);
        let normalized = normalize_command(&command, source);
        let declaration = normalized.declaration.expect("expected declaration");

        assert_eq!(declaration.kind, DeclarationKind::Declare);
        assert!(declaration.assignment_operands.is_empty());
        assert_eq!(declaration.operands.len(), 1);
    }
}
