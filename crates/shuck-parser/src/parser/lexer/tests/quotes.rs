use super::*;

#[test]
fn test_single_quoted_string() {
    let mut lexer = Lexer::new("echo 'hello world'");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    // Single-quoted strings return LiteralWord (no variable expansion)
    assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("hello world"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_double_quoted_string() {
    let mut lexer = Lexer::new("echo \"hello world\"");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("hello world"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_brace_expansion_token_ignores_quoted_closers() {
    let mut lexer = Lexer::new("echo {\"}\",a}\n");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some(r#"{"}",a}"#));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_brace_expansion_token_preserves_single_quoted_backslash_member_boundary() {
    let mut lexer = Lexer::new("echo {'a\\',b} next\n");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some(r#"{'a\',b}"#));
    assert_next_token(&mut lexer, TokenKind::Word, Some("next"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_double_quoted_expansion_token_keeps_source_backing() {
    let source = r#""$bar""#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::QuotedWord);
    assert_eq!(token.word_text(), Some("$bar"));

    let word = token.word().unwrap();
    let segment = word.single_segment().unwrap();
    assert_eq!(segment.kind(), LexedWordSegmentKind::DoubleQuoted);
    assert_eq!(segment.span().unwrap().slice(source), "$bar");
}

#[test]
fn test_double_quoted_token_preserves_braced_param_pipeline_substitution() {
    let source = r#""$(echo "${@}" | tr -d '[:space:]')""#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::QuotedWord);
    assert_eq!(
        token.word_text(),
        Some(r#"$(echo "${@}" | tr -d '[:space:]')"#)
    );
}

#[test]
fn test_single_quoted_prefix_keeps_plain_continuation_segment() {
    let source = "'foo'bar";
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::LiteralWord);

    let word = token.word().unwrap();
    let segments: Vec<_> = word
        .segments()
        .map(|segment| (segment.kind(), segment.as_str().to_string()))
        .collect();

    assert_eq!(
        segments,
        vec![
            (LexedWordSegmentKind::SingleQuoted, "foo".to_string()),
            (LexedWordSegmentKind::Plain, "bar".to_string()),
        ]
    );
    assert_eq!(word.joined_text(), "foobar");
    assert_eq!(
        word.segments()
            .nth(1)
            .and_then(LexedWordSegment::span)
            .unwrap()
            .slice(source),
        "bar"
    );
}

#[test]
fn test_unquoted_nested_param_expansion_word_keeps_source_backing() {
    let source = "${arr[$RANDOM % ${#arr[@]}]}";
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);

    let word = token.word().unwrap();
    let segment = word.single_segment().unwrap();
    assert_eq!(segment.kind(), LexedWordSegmentKind::Plain);
    assert_eq!(segment.as_str(), source);
    assert_eq!(segment.span().unwrap().slice(source), source);
}

#[test]
fn test_double_quoted_nested_param_expansion_keeps_source_backing() {
    let source = r#""${arr[$RANDOM % ${#arr[@]}]}""#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::QuotedWord);

    let word = token.word().unwrap();
    let segment = word.single_segment().unwrap();
    assert_eq!(segment.kind(), LexedWordSegmentKind::DoubleQuoted);
    assert_eq!(segment.as_str(), "${arr[$RANDOM % ${#arr[@]}]}");
    assert_eq!(
        segment.span().unwrap().slice(source),
        "${arr[$RANDOM % ${#arr[@]}]}"
    );
}

#[test]
fn test_ansi_c_control_escape_can_consume_quote() {
    let mut lexer = Lexer::new("echo $'\\c''");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("\x07"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_array_element_with_quoted_prefix_zsh_glob_qualifier_stays_one_word() {
    let source = r#"plugins=( "$plugin_dir"/*(:t) )"#;
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("plugins="));
    assert_next_token(&mut lexer, TokenKind::LeftParen, None);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), r#""$plugin_dir"/*(:t)"#);

    let word = token.word().unwrap();
    let segments: Vec<_> = word
        .segments()
        .map(|segment| (segment.kind(), segment.as_str().to_string()))
        .collect();
    assert_eq!(
        segments,
        vec![
            (
                LexedWordSegmentKind::DoubleQuoted,
                "$plugin_dir".to_string()
            ),
            (LexedWordSegmentKind::Plain, "/*".to_string()),
            (LexedWordSegmentKind::Plain, "(:t)".to_string()),
        ]
    );

    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_array_element_with_quoted_variable_zsh_qualifier_stays_one_word() {
    let source = r#"__GREP_ALIAS_CACHES=( "$__GREP_CACHE_FILE"(Nm-1) )"#;
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("__GREP_ALIAS_CACHES="));
    assert_next_token(&mut lexer, TokenKind::LeftParen, None);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), r#""$__GREP_CACHE_FILE"(Nm-1)"#);

    let word = token.word().unwrap();
    let segments: Vec<_> = word
        .segments()
        .map(|segment| (segment.kind(), segment.as_str().to_string()))
        .collect();
    assert_eq!(
        segments,
        vec![
            (
                LexedWordSegmentKind::DoubleQuoted,
                "$__GREP_CACHE_FILE".to_string()
            ),
            (LexedWordSegmentKind::Plain, "(Nm-1)".to_string()),
        ]
    );

    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_case_arm_with_quoted_space_substitution_stays_line_local() {
    let input = concat!(
        "case \"${_input_type:-}\" in\n",
        "  html) _hashtag_pattern=\"<a\\ href=\\\"${_hashtag_replacement_url//' '/%20}\\\">\\#\\\\2<\\/a>\" ;;\n",
        "  org)  _hashtag_pattern=\"[[${_hashtag_replacement_url//' '/%20}][\\#\\\\2]]\" ;;\n",
        "esac\n",
    );

    assert_non_newline_tokens_stay_on_one_line(input);

    let mut lexer = Lexer::new(input);
    let tokens = std::iter::from_fn(|| lexer.next_lexed_token())
        .map(|token| (token.kind, token_text(&token, input)))
        .collect::<Vec<_>>();
    assert!(tokens.contains(&(TokenKind::DoubleSemicolon, None)));
    assert!(tokens.contains(&(TokenKind::Word, Some("esac".to_string()))));
}

#[test]
fn test_zsh_midfile_setopt_rc_quotes_merges_adjacent_single_quotes() {
    let source = "setopt rc_quotes\nprint 'a''b'\n";
    let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
    let mut lexer = Lexer::with_profile(source, &profile);

    assert_next_token(&mut lexer, TokenKind::Word, Some("setopt"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("rc_quotes"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("print"));
    assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("a'b"));
}
