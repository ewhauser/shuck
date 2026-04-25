pub use shuck_ast::{DeclarationKind, NormalizedCommand, NormalizedDeclaration, WrapperKind};
pub(crate) use shuck_ast::{normalize_command, normalize_command_words};

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
            ("\\typeset result\n", "typeset"),
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
