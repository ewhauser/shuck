use super::*;

#[test]
fn test_zsh_inline_glob_control_after_left_paren_is_not_comment() {
    let mut lexer = Lexer::new("if [[ \"$buf\" == (#b)(*)(${~pat})* ]]; then\n");

    let mut saw_comment = false;
    while let Some(token) = lexer.next_lexed_token_with_comments() {
        if token.kind == TokenKind::Comment {
            saw_comment = true;
            break;
        }
    }

    assert!(
        !saw_comment,
        "zsh inline glob controls inside [[ ]] should not lex as comments"
    );
}

#[test]
fn test_assoc_compound_assignment() {
    // declare -A m=([foo]="bar" [baz]="qux") should keep the compound
    // assignment as a single Word token
    let mut lexer = Lexer::new(r#"m=([foo]="bar" [baz]="qux")"#);
    assert_next_token(
        &mut lexer,
        TokenKind::Word,
        Some(r#"m=([foo]="bar" [baz]="qux")"#),
    );
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_assoc_compound_assignment_after_escaped_literal_keeps_compound_word() {
    let source = r#"foo\_bar=([foo]="bar" [baz]="qux")"#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), source);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_extglob_after_escaped_literal_keeps_suffix_group() {
    let source = r#"foo\_bar@(baz|qux)"#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), source);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_zsh_alternative_glob_after_dot_keeps_suffix_group() {
    let source = "file.(txt|doc|pdf)";
    let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
    let mut lexer = Lexer::with_profile(source, &profile);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), source);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_zsh_path_glob_modifier_keeps_suffix_group() {
    let source = "/path/file(:h)";
    let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
    let mut lexer = Lexer::with_profile(source, &profile);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), source);
    assert!(lexer.next_lexed_token().is_none());

    let mut default_lexer = Lexer::new(source);
    let token = default_lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), "/path/file");
}

#[test]
fn test_case_arm_with_zsh_semipipe_terminator_lexes_as_single_token() {
    let input = concat!(
        "case $2 in\n",
        "  cygwin*) bin='cygwin32/bin' ;|\n",
        "esac\n",
    );

    let mut lexer = Lexer::new(input);
    let tokens = std::iter::from_fn(|| lexer.next_lexed_token())
        .map(|token| (token.kind, token_text(&token, input)))
        .collect::<Vec<_>>();

    assert!(tokens.contains(&(TokenKind::SemiPipe, None)));
    assert!(!tokens.contains(&(TokenKind::Semicolon, None)));
    assert!(!tokens.contains(&(TokenKind::Pipe, None)));
}

#[test]
fn test_zsh_midfile_unsetopt_interactive_comments_keeps_hash_as_word() {
    let source = "unsetopt interactive_comments\n#literal\n";
    let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
    let mut lexer = Lexer::with_profile(source, &profile);

    assert_next_token(&mut lexer, TokenKind::Word, Some("unsetopt"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("interactive_comments"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("#literal"));
}

#[test]
fn test_zsh_midfile_setopt_ignore_braces_lexes_braces_as_words() {
    let source = "setopt ignore_braces\n{ echo }\n";
    let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
    let mut lexer = Lexer::with_profile(source, &profile);

    assert_next_token(&mut lexer, TokenKind::Word, Some("setopt"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("ignore_braces"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("{"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("}"));
}

#[test]
fn test_zsh_midfile_setopt_brace_ccl_keeps_adjacent_brace_expansions_in_one_word() {
    let source = "setopt brace_ccl\n{ab}{0-2}\n";
    let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
    let mut lexer = Lexer::with_profile(source, &profile);

    assert_next_token(&mut lexer, TokenKind::Word, Some("setopt"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("brace_ccl"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("{ab}{0-2}"));
}
