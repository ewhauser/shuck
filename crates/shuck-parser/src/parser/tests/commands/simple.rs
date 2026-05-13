use super::*;

#[test]
fn test_parse_simple_command() {
    let input = "echo hello";
    let parser = Parser::new(input);
    let parsed = parser.parse().unwrap();
    assert_eq!(parsed.status, ParseStatus::Clean);
    assert!(parsed.diagnostics.is_empty());
    assert!(parsed.terminal_error.is_none());
    let script = parsed.file;

    assert_eq!(script.body.len(), 1);

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(cmd.args.len(), 1);
        assert_eq!(cmd.args[0].render(input), "hello");
    } else {
        panic!("expected simple command");
    }
}

#[test]
fn test_parse_break_as_typed_builtin() {
    let input = "break 2";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Break(command)) = &script.body[0].command else {
        panic!("expected break builtin");
    };

    assert_eq!(command.depth.as_ref().unwrap().render(input), "2");
    assert!(command.extra_args.is_empty());
}

#[test]
fn test_parse_continue_preserves_extra_args() {
    let input = "continue 1 extra";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Continue(command)) = &script.body[0].command else {
        panic!("expected continue builtin");
    };

    assert_eq!(command.depth.as_ref().unwrap().render(input), "1");
    assert_eq!(command.extra_args.len(), 1);
    assert_eq!(command.extra_args[0].render(input), "extra");
}

#[test]
fn test_parse_exit_as_typed_builtin() {
    let input = "exit 1";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    let AstCommand::Builtin(AstBuiltinCommand::Exit(command)) = &script.body[0].command else {
        panic!("expected exit builtin");
    };

    assert_eq!(command.code.as_ref().unwrap().render(input), "1");
    assert!(command.extra_args.is_empty());
}

#[test]
fn test_parse_multiple_args() {
    let input = "echo hello world";
    let parser = Parser::new(input);
    let script = parser.parse().unwrap().file;

    if let AstCommand::Simple(cmd) = &script.body[0].command {
        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(cmd.args.len(), 2);
        assert_eq!(cmd.args[0].render(input), "hello");
        assert_eq!(cmd.args[1].render(input), "world");
    } else {
        panic!("expected simple command");
    }
}

#[test]
fn test_simple_command_allows_nft_brace_literals_inside_and_block() {
    let input = "\
start_nftables() {
    [ \"$tun_statu\" = true ] && {
        nft add chain inet fw4 forward { type filter hook forward priority filter \\; } 2>/dev/null
        nft add chain inet fw4 input { type filter hook input priority filter \\; } 2>/dev/null
    }
}
";
    let parsed = Parser::new(input).parse().unwrap();

    assert_eq!(parsed.file.body.len(), 1);
    let function = expect_function(&parsed.file.body[0]);
    assert!(matches!(function.body.command, AstCommand::Compound(..)));
}

#[test]
fn test_parse_nft_literal_braces_inside_nested_and_groups() {
    let input = r#"
start_nftables() {
    [ "$redir_mod" = "Tproxy" ] && (modprobe nft_tproxy >/dev/null 2>&1 || lsmod 2>/dev/null | grep -q nft_tproxy) && {
        [ "$local_proxy" = true ] && {
            nft add chain inet shellcrash mark_out { type filter hook prerouting priority -100 \; }
        }
    }
    [ "$tun_statu" = true ] && {
        [ "$lan_proxy" = true ] && {
            nft list chain inet fw4 forward >/dev/null 2>&1 || nft add chain inet fw4 forward { type filter hook forward priority filter \; } 2>/dev/null
            nft list chain inet fw4 input >/dev/null 2>&1 || nft add chain inet fw4 input { type filter hook input priority filter \; } 2>/dev/null
        }
        [ "$local_proxy" = true ] && start_nft_route output output route -150
    }
}
"#;
    let parsed = Parser::with_dialect(input, ShellDialect::Bash).parse();
    assert_eq!(
        parsed.status,
        ParseStatus::Clean,
        "{}",
        parsed.strict_error()
    );
}

#[test]
fn test_brace_group_command_can_use_right_brace_as_literal_argument() {
    let source = "rbrace() { echo }; }; rbrace\n";
    let output = Parser::new(source).parse().unwrap();

    let function = expect_function(&output.file.body[0]);
    let (compound, redirects) = expect_compound(function.body.as_ref());
    let AstCompoundCommand::BraceGroup(body) = compound else {
        panic!("expected brace-group function body");
    };

    assert!(redirects.is_empty());
    assert_eq!(body.len(), 1);
    let command = expect_simple(&body[0]);
    assert_eq!(command.name.render(source), "echo");
    assert_eq!(command.args.len(), 1);
    assert_eq!(command.args[0].render(source), "}");
}

#[test]
fn test_parse_coproc_with_non_plain_name_candidate_defaults_to_coproc_name() {
    let source = "coproc 'roc' cat\n";
    let output = Parser::new(source).parse().unwrap().file;

    let (compound, _) = expect_compound(&output.body[0]);
    let AstCompoundCommand::Coproc(command) = compound else {
        panic!("expected coproc command");
    };

    assert_eq!(command.name.as_str(), "COPROC");
    let simple = expect_simple(command.body.as_ref());
    assert_eq!(simple.name.render_syntax(source), "'roc'");
    assert_eq!(simple.args[0].render(source), "cat");
}
