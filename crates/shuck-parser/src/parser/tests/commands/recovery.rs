use super::*;

#[test]
fn test_unexpected_top_level_token_errors_in_strict_mode() {
    let parsed = Parser::new("echo ok\n)\necho later\n").parse();
    assert_eq!(parsed.status, ParseStatus::Fatal);
    assert!(parsed.terminal_error.is_some());
    let error = parsed.unwrap_err();

    let Error::Parse {
        message,
        line,
        column,
    } = error;
    assert_eq!(message, "expected command");
    assert_eq!(line, 2);
    assert_eq!(column, 1);
}

#[test]
fn test_parse_recovered_skips_invalid_command_and_continues() {
    let input = "echo one\ncat >\necho two\n";
    let recovered = Parser::new(input).parse();

    assert_eq!(recovered.status, ParseStatus::Fatal);
    assert_eq!(recovered.file.body.len(), 2);
    assert_eq!(recovered.diagnostics.len(), 1);
    assert_eq!(recovered.diagnostics[0].message, "expected word");
    assert_eq!(recovered.diagnostics[0].span.start.line, 2);

    let first = expect_simple(&recovered.file.body[0]);
    assert_eq!(first.name.render(input), "echo");
    assert_eq!(first.args[0].render(input), "one");

    let second = expect_simple(&recovered.file.body[1]);
    assert_eq!(second.name.render(input), "echo");
    assert_eq!(second.args[0].render(input), "two");
}

#[test]
fn test_parse_reports_eof_only_missing_fi_as_recovered() {
    let input = "if true; then\n  :\n";
    let parsed = Parser::new(input).parse();

    assert_eq!(parsed.status, ParseStatus::Recovered);
    assert!(parsed.terminal_error.is_none());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].message, "expected 'fi'");
}

#[test]
fn test_empty_if_then_rejected() {
    let parser = Parser::new("if true; then\nfi");
    assert!(
        parser.parse().is_err(),
        "empty then clause should be rejected"
    );
}

#[test]
fn test_empty_else_rejected() {
    let parser = Parser::new("if false; then echo yes; else\nfi");
    assert!(
        parser.parse().is_err(),
        "empty else clause should be rejected"
    );
}

#[test]
fn test_parse_zsh_array_slice_assignment_to_empty_array() {
    let input = "if true; then\n  tokens[1,e]=()\nelse\n  tokens[1,e]=()\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_split_indexed_assignment_to_empty_array() {
    let input = "tokens[1,e]=()\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}

#[test]
fn test_parse_zsh_if_with_empty_then_before_elif() {
    let input = "if false; then\nelif [[ $arg = $'\\x7d' ]]; then\n  print ok\nfi\n";
    Parser::with_dialect(input, ShellDialect::Zsh)
        .parse()
        .unwrap();
}
