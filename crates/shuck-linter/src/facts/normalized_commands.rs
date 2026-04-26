use std::borrow::Cow;

use shuck_ast::{
    ArenaFileCommandKind, AssignmentNode, AstStore, BuiltinCommandNodeKind, CommandView,
    DeclCommandView, DeclOperandNode, Span, StaticCommandWrapperTarget, WordId,
};

pub use shuck_ast::{DeclarationKind, NormalizedCommand, NormalizedDeclaration, WrapperKind};
pub(crate) use shuck_ast::{normalize_command, normalize_command_words};

#[derive(Debug, Clone)]
pub struct ArenaNormalizedDeclaration<'a> {
    pub kind: DeclarationKind,
    pub readonly_flag: bool,
    pub span: Span,
    pub head_span: Span,
    pub assignments: &'a [AssignmentNode],
    pub operands: &'a [DeclOperandNode],
    pub assignment_operands: Vec<&'a AssignmentNode>,
}

#[derive(Debug, Clone)]
pub struct ArenaNormalizedCommand<'a> {
    pub literal_name: Option<Cow<'a, str>>,
    pub effective_name: Option<Cow<'a, str>>,
    pub wrappers: Vec<WrapperKind>,
    pub body_span: Span,
    pub body_word_span: Option<Span>,
    pub body_words: Vec<WordId>,
    pub declaration: Option<ArenaNormalizedDeclaration<'a>>,
}

impl ArenaNormalizedCommand<'_> {
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

    pub fn body_name_word_id(&self) -> Option<WordId> {
        self.body_words.first().copied()
    }

    pub fn body_args(&self) -> &[WordId] {
        self.body_words.split_first().map_or(&[], |(_, rest)| rest)
    }
}

pub(crate) fn normalize_arena_command<'a>(
    command: CommandView<'a>,
    source: &'a str,
) -> ArenaNormalizedCommand<'a> {
    match command.kind() {
        ArenaFileCommandKind::Simple => {
            let command_span = command.span();
            let simple = command.simple().expect("simple command view");
            let words = std::iter::once(simple.name_id())
                .chain(simple.arg_ids().iter().copied())
                .collect::<Vec<_>>();
            normalize_arena_command_words(simple.name().store(), &words, command_span, source)
                .expect("simple commands always have a name")
        }
        ArenaFileCommandKind::Decl => {
            let command_span = command.span();
            let command = command.decl().expect("decl command view");
            ArenaNormalizedCommand {
                literal_name: Some(Cow::Borrowed(command.variant().as_str())),
                effective_name: Some(Cow::Borrowed(command.variant().as_str())),
                wrappers: Vec::new(),
                body_span: command.variant_span(),
                body_word_span: None,
                body_words: Vec::new(),
                declaration: Some(normalize_arena_declaration(command, command_span, source)),
            }
        }
        ArenaFileCommandKind::Builtin => {
            let command_span = command.span();
            let builtin = command.builtin().expect("builtin command view");
            let name = match builtin.kind() {
                BuiltinCommandNodeKind::Break => "break",
                BuiltinCommandNodeKind::Continue => "continue",
                BuiltinCommandNodeKind::Return => "return",
                BuiltinCommandNodeKind::Exit => "exit",
            };
            ArenaNormalizedCommand {
                literal_name: Some(Cow::Borrowed(name)),
                effective_name: Some(Cow::Borrowed(name)),
                wrappers: Vec::new(),
                body_span: builtin.primary().map_or(command_span, |word| {
                    Span::from_positions(command_span.start, word.span().end)
                }),
                body_word_span: None,
                body_words: Vec::new(),
                declaration: None,
            }
        }
        ArenaFileCommandKind::Binary
        | ArenaFileCommandKind::Compound
        | ArenaFileCommandKind::Function
        | ArenaFileCommandKind::AnonymousFunction => ArenaNormalizedCommand {
            literal_name: None,
            effective_name: None,
            wrappers: Vec::new(),
            body_span: command.span(),
            body_word_span: None,
            body_words: Vec::new(),
            declaration: None,
        },
    }
}

pub(crate) fn normalize_arena_command_words<'a>(
    store: &'a AstStore,
    words: &[WordId],
    _command_span: Span,
    source: &'a str,
) -> Option<ArenaNormalizedCommand<'a>> {
    let first_id = words.first().copied()?;
    let first_word = store.word(first_id);
    let literal_name = shuck_ast::static_command_name_text_arena(first_word, source);
    let mut effective_name = literal_name.clone();
    let mut wrappers = Vec::new();
    let mut body_span = first_word.span();
    let mut body_word_span = Some(first_word.span());
    let mut body_start = literal_name.as_ref().map(|_| 0usize);
    let mut current_index = 0usize;

    while let Some(current_name) = effective_name.as_deref() {
        let Some(resolution) =
            resolve_arena_command_resolution(store, words, current_index, current_name, source)
        else {
            break;
        };

        match resolution {
            ArenaCommandResolution::Alias {
                effective_name: resolved_name,
                body_index,
            } => {
                let body = store.word(words[body_index]);
                body_span = body.span();
                body_word_span = Some(body.span());
                body_start = Some(body_index);
                effective_name = Some(Cow::Owned(resolved_name));
                break;
            }
            ArenaCommandResolution::Wrapper { kind, target_index } => {
                wrappers.push(kind);

                let Some(target_index) = target_index else {
                    effective_name = None;
                    body_word_span = None;
                    body_start = None;
                    break;
                };

                let target = store.word(words[target_index]);
                body_span = target.span();
                body_word_span = Some(target.span());
                effective_name = shuck_ast::static_command_name_text_arena(target, source);
                body_start = effective_name.as_ref().map(|_| target_index);
                current_index = target_index;

                if effective_name.is_none() {
                    break;
                }
            }
        }
    }

    Some(ArenaNormalizedCommand {
        literal_name,
        effective_name,
        wrappers,
        body_span,
        body_word_span,
        body_words: body_start.map_or_else(Vec::new, |start| words[start..].to_vec()),
        declaration: None,
    })
}

fn normalize_arena_declaration<'a>(
    command: DeclCommandView<'a>,
    command_span: Span,
    source: &str,
) -> ArenaNormalizedDeclaration<'a> {
    let raw_kind = command.variant().as_str();
    let assignment_operands = command
        .operands()
        .iter()
        .filter_map(|operand| match operand {
            DeclOperandNode::Assignment(assignment) => Some(assignment),
            DeclOperandNode::Flag(_) | DeclOperandNode::Name(_) | DeclOperandNode::Dynamic(_) => {
                None
            }
        })
        .collect::<Vec<_>>();

    ArenaNormalizedDeclaration {
        kind: declaration_kind(raw_kind),
        readonly_flag: arena_declaration_has_readonly_flag(command, source),
        span: command_span,
        head_span: arena_declaration_head_span(command),
        assignments: command.assignments(),
        operands: command.operands(),
        assignment_operands,
    }
}

fn declaration_kind(raw_kind: &str) -> DeclarationKind {
    match raw_kind {
        "export" => DeclarationKind::Export,
        "local" => DeclarationKind::Local,
        "declare" => DeclarationKind::Declare,
        "typeset" => DeclarationKind::Typeset,
        _ => DeclarationKind::Other(raw_kind.to_owned()),
    }
}

fn arena_declaration_has_readonly_flag(command: DeclCommandView<'_>, source: &str) -> bool {
    matches!(command.variant().as_str(), "local" | "declare" | "typeset")
        && command.operands().iter().any(|operand| {
            let DeclOperandNode::Flag(word) = operand else {
                return false;
            };

            shuck_ast::static_word_text_arena(command.store().word(*word), source)
                .is_some_and(|text| text.starts_with('-') && text.contains('r'))
        })
}

fn arena_declaration_head_span(command: DeclCommandView<'_>) -> Span {
    let end = command
        .operands()
        .last()
        .map_or(command.variant_span().end, |operand| {
            arena_declaration_operand_head_end(command.store(), operand)
        });
    Span::from_positions(command.variant_span().start, end)
}

fn arena_declaration_operand_head_end(
    store: &AstStore,
    operand: &DeclOperandNode,
) -> shuck_ast::Position {
    match operand {
        DeclOperandNode::Flag(word) | DeclOperandNode::Dynamic(word) => {
            store.word(*word).span().end
        }
        DeclOperandNode::Name(name) => name.span.end,
        DeclOperandNode::Assignment(assignment) => assignment.target.name_span.end,
    }
}

enum ArenaCommandResolution {
    Alias {
        effective_name: String,
        body_index: usize,
    },
    Wrapper {
        kind: WrapperKind,
        target_index: Option<usize>,
    },
}

fn resolve_arena_command_resolution(
    store: &AstStore,
    words: &[WordId],
    current_index: usize,
    current_name: &str,
    source: &str,
) -> Option<ArenaCommandResolution> {
    match current_name {
        "noglob" | "command" | "builtin" | "exec" => Some(resolve_shared_arena_wrapper(
            store,
            words,
            current_index,
            current_name,
            source,
        )),
        "busybox" => Some(ArenaCommandResolution::Wrapper {
            kind: WrapperKind::Busybox,
            target_index: words.get(current_index + 1).map(|_| current_index + 1),
        }),
        "find" => find_exec_arena_target_index(store, words, current_index, source).map(
            |(kind, target_index)| ArenaCommandResolution::Wrapper {
                kind,
                target_index: Some(target_index),
            },
        ),
        "sudo" | "doas" | "run0" => Some(ArenaCommandResolution::Wrapper {
            kind: WrapperKind::SudoFamily,
            target_index: sudo_family_arena_target_index(store, words, current_index, source),
        }),
        "git" => git_filter_branch_arena_resolution(store, words, current_index, source),
        "mumps" => mumps_run_arena_resolution(store, words, current_index, source),
        _ => None,
    }
}

fn resolve_shared_arena_wrapper(
    store: &AstStore,
    words: &[WordId],
    current_index: usize,
    current_name: &str,
    source: &str,
) -> ArenaCommandResolution {
    let kind = match current_name {
        "noglob" => WrapperKind::Noglob,
        "command" => WrapperKind::Command,
        "builtin" => WrapperKind::Builtin,
        "exec" => WrapperKind::Exec,
        _ => unreachable!("caller only passes shared wrappers"),
    };
    let StaticCommandWrapperTarget::Wrapper { target_index } =
        shuck_ast::static_command_wrapper_target_index(
            words.len(),
            current_index,
            current_name,
            |index| shuck_ast::static_word_text_arena(store.word(words[index]), source),
        )
    else {
        unreachable!("shared wrapper name should resolve to a wrapper target");
    };

    ArenaCommandResolution::Wrapper { kind, target_index }
}

fn find_exec_arena_target_index(
    store: &AstStore,
    words: &[WordId],
    current_index: usize,
    source: &str,
) -> Option<(WrapperKind, usize)> {
    let mut fallback = None;

    for index in current_index + 1..words.len() {
        let Some(arg) = shuck_ast::static_word_text_arena(store.word(words[index]), source) else {
            continue;
        };
        match arg.as_ref() {
            "-exec" | "-ok" => {
                if fallback.is_none() {
                    fallback = words
                        .get(index + 1)
                        .map(|_| (WrapperKind::FindExec, index + 1));
                }
            }
            "-execdir" => {
                return words
                    .get(index + 1)
                    .map(|_| (WrapperKind::FindExecDir, index + 1));
            }
            "-okdir" => {
                if fallback.is_none() {
                    fallback = words
                        .get(index + 1)
                        .map(|_| (WrapperKind::FindExec, index + 1));
                }
            }
            _ => {}
        }
    }

    fallback
}

fn sudo_family_arena_target_index(
    store: &AstStore,
    words: &[WordId],
    current_index: usize,
    source: &str,
) -> Option<usize> {
    let mut index = current_index + 1;

    while index < words.len() {
        let Some(arg) = shuck_ast::static_word_text_arena(store.word(words[index]), source) else {
            return Some(index);
        };

        if arg == "--" {
            return words.get(index + 1).map(|_| index + 1);
        }

        if !arg.starts_with('-') || arg == "-" {
            return Some(index);
        }

        if arg.len() == 2 && matches!(arg.as_ref(), "-l" | "-v" | "-V") {
            return None;
        }

        if sudo_option_takes_value(arg.as_ref()) {
            if arg.len() == 2 {
                words.get(index + 1)?;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if arg.starts_with("--") && !matches!(arg.as_ref(), "--preserve-env" | "--login") {
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

fn git_filter_branch_arena_resolution(
    store: &AstStore,
    words: &[WordId],
    current_index: usize,
    source: &str,
) -> Option<ArenaCommandResolution> {
    (words
        .get(current_index + 1)
        .and_then(|word| shuck_ast::static_word_text_arena(store.word(*word), source))
        .as_deref()
        == Some("filter-branch"))
    .then(|| ArenaCommandResolution::Alias {
        effective_name: "git filter-branch".to_owned(),
        body_index: current_index + 1,
    })
}

fn mumps_run_arena_resolution(
    store: &AstStore,
    words: &[WordId],
    current_index: usize,
    source: &str,
) -> Option<ArenaCommandResolution> {
    let run_flag = words
        .get(current_index + 1)
        .and_then(|word| shuck_ast::static_word_text_arena(store.word(*word), source))?;
    let entrypoint = words
        .get(current_index + 2)
        .and_then(|word| shuck_ast::static_word_text_arena(store.word(*word), source))?;

    if run_flag == "-run" && matches!(entrypoint.as_ref(), "%XCMD" | "LOOP%XCMD") {
        Some(ArenaCommandResolution::Alias {
            effective_name: format!("mumps -run {entrypoint}"),
            body_index: current_index + 2,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{ArenaFile, Command};
    use shuck_parser::parser::Parser;

    use super::{DeclarationKind, WrapperKind, normalize_arena_command, normalize_command};

    fn parse_first_command(source: &str) -> Command {
        let output = Parser::new(source).parse().unwrap();
        output.file.body.stmts.into_iter().next().unwrap().command
    }

    fn parse_first_arena_command(source: &str) -> ArenaFile {
        Parser::new(source).parse().unwrap().arena_file
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
                "command -pp printf '%s\\n' hi\n",
                Some("command"),
                Some("printf"),
                vec![WrapperKind::Command],
                ("printf", Some("'%s\\n'")),
            ),
            (
                "noglob rm *.txt\n",
                Some("noglob"),
                Some("rm"),
                vec![WrapperKind::Noglob],
                ("rm", Some("*.txt")),
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
                "exec -cl printf '%s\\n' hi\n",
                Some("exec"),
                Some("printf"),
                vec![WrapperKind::Exec],
                ("printf", Some("'%s\\n'")),
            ),
            (
                "exec -lc printf '%s\\n' hi\n",
                Some("exec"),
                Some("printf"),
                vec![WrapperKind::Exec],
                ("printf", Some("'%s\\n'")),
            ),
            (
                "exec -la shuck printf '%s\\n' hi\n",
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
    fn normalize_arena_command_peels_wrappers_and_aliases() {
        let cases = [
            (
                "command printf '%s\\n' hi\n",
                Some("command"),
                Some("printf"),
                vec![WrapperKind::Command],
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
                "find . -execdir sh -c 'echo {}' \\;\n",
                Some("find"),
                Some("sh"),
                vec![WrapperKind::FindExecDir],
                ("sh", Some("-c")),
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
            let arena = parse_first_arena_command(source);
            let command = arena.view().body().stmts().next().unwrap().command();
            let normalized = normalize_arena_command(command, source);

            assert_eq!(normalized.literal_name.as_deref(), literal_name);
            assert_eq!(normalized.effective_name.as_deref(), effective_name);
            assert_eq!(normalized.wrappers, wrappers);
            assert_eq!(
                normalized
                    .body_name_word_id()
                    .map(|id| arena.store.word(id).span().slice(source)),
                Some(body_name)
            );
            assert_eq!(
                normalized
                    .body_args()
                    .first()
                    .map(|id| arena.store.word(*id).span().slice(source)),
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
    fn normalize_command_tracks_quoted_static_command_names() {
        let source = "\"printf\" '%s\\n' hi\n";
        let command = parse_first_command(source);
        let normalized = normalize_command(&command, source);

        assert_eq!(normalized.literal_name.as_deref(), Some("printf"));
        assert_eq!(normalized.effective_name.as_deref(), Some("printf"));
        assert!(normalized.wrappers.is_empty());
        assert_eq!(normalized.body_words.len(), 3);
        assert_eq!(
            normalized
                .body_name_word()
                .map(|word| word.span.slice(source)),
            Some("\"printf\"")
        );
    }

    #[test]
    fn normalize_command_preserves_wrapper_markers_for_quoted_targets() {
        let source = "command \"printf\" '%s\\n' hi\n";
        let command = parse_first_command(source);
        let normalized = normalize_command(&command, source);

        assert_eq!(normalized.literal_name.as_deref(), Some("command"));
        assert_eq!(normalized.effective_name.as_deref(), Some("printf"));
        assert_eq!(normalized.wrappers, vec![WrapperKind::Command]);
        assert_eq!(
            normalized
                .body_name_word()
                .map(|word| word.span.slice(source)),
            Some("\"printf\"")
        );
    }

    #[test]
    fn normalize_command_does_not_peel_unsupported_wrapper_options() {
        for (source, wrapper) in [
            ("command -x printf '%s\\n' hi\n", WrapperKind::Command),
            ("builtin -x read var\n", WrapperKind::Builtin),
            ("exec -x printf '%s\\n' hi\n", WrapperKind::Exec),
        ] {
            let command = parse_first_command(source);
            let normalized = normalize_command(&command, source);

            assert!(normalized.literal_name.is_some(), "{source}");
            assert_eq!(normalized.effective_name.as_deref(), None, "{source}");
            assert_eq!(normalized.wrappers, vec![wrapper], "{source}");
            assert!(normalized.body_words.is_empty(), "{source}");
        }
    }

    #[test]
    fn normalize_command_canonicalizes_escaped_static_names() {
        for (source, expected) in [
            ("\\cd /tmp\n", "cd"),
            ("\\grep foo input.txt\n", "grep"),
            ("\\. /dev/stdin yes\n", "."),
        ] {
            let command = parse_first_command(source);
            let normalized = normalize_command(&command, source);

            assert_eq!(
                normalized.literal_name.as_deref(),
                Some(expected),
                "{source}"
            );
            assert_eq!(
                normalized.effective_name.as_deref(),
                Some(expected),
                "{source}"
            );
            assert_eq!(
                normalized
                    .body_name_word()
                    .map(|word| word.span.slice(source)),
                Some(source.lines().next().unwrap().split(' ').next().unwrap()),
                "{source}"
            );
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

    #[test]
    fn normalize_command_prefers_later_find_execdir_targets() {
        let source = "find . -exec echo {} \\; -execdir sh -c 'mv {}' \\;\n";
        let command = parse_first_command(source);
        let normalized = normalize_command(&command, source);

        assert_eq!(normalized.effective_name.as_deref(), Some("sh"));
        assert_eq!(normalized.wrappers, vec![WrapperKind::FindExecDir]);
        assert_eq!(
            normalized
                .body_args()
                .first()
                .map(|word| word.span.slice(source)),
            Some("-c")
        );
    }
}
