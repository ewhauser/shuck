use super::*;

#[test]
fn test_mixed_word_keeps_segment_kinds() {
    let source = r#"foo"bar"'baz'"#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);

    let word = token.word().unwrap();
    let segments: Vec<_> = word
        .segments()
        .map(|segment| (segment.kind(), segment.as_str().to_string()))
        .collect();

    assert_eq!(
        segments,
        vec![
            (LexedWordSegmentKind::Plain, "foo".to_string()),
            (LexedWordSegmentKind::DoubleQuoted, "bar".to_string()),
            (LexedWordSegmentKind::SingleQuoted, "baz".to_string()),
        ]
    );
    assert_eq!(word.joined_text(), "foobarbaz");
    assert_eq!(
        word.segments()
            .next()
            .and_then(LexedWordSegment::span)
            .unwrap()
            .slice(source),
        "foo"
    );
}

#[test]
fn test_trim_pattern_with_literal_left_brace_does_not_swallow_following_tokens() {
    let source = "dns_servercow_info='ServerCow.de\nSite: ServerCow.de\n'\n\nf(){\n  if true; then\n    txtvalue_old=${response#*{\\\"name\\\":\\\"\"$_sub_domain\"\\\",\\\"ttl\\\":20,\\\"type\\\":\\\"TXT\\\",\\\"content\\\":\\\"}\n  fi\n}\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(
        &mut lexer,
        TokenKind::Word,
        Some("dns_servercow_info=ServerCow.de\nSite: ServerCow.de\n"),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("f"));
    assert_next_token(&mut lexer, TokenKind::LeftParen, None);
    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert_next_token(&mut lexer, TokenKind::LeftBrace, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("if"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("true"));
    assert_next_token(&mut lexer, TokenKind::Semicolon, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("then"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(
        &mut lexer,
        TokenKind::Word,
        Some(
            "txtvalue_old=${response#*{\"name\":\"\"$_sub_domain\"\",\"ttl\":20,\"type\":\"TXT\",\"content\":\"}",
        ),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("fi"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::RightBrace, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_case_pattern_literal_left_brace_does_not_swallow_following_arms() {
    let source = "case \"$word\" in\n  {) : ;;\n  :) : ;;\nesac\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("case"));
    assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("$word"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("in"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("{"));
    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some(":"));
    assert_next_token(&mut lexer, TokenKind::DoubleSemicolon, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some(":"));
    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some(":"));
    assert_next_token(&mut lexer, TokenKind::DoubleSemicolon, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("esac"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_conditional_regex_literal_left_brace_keeps_closing_tokens() {
    let source = "if [[ $MOTD ]] && ! [[ $MOTD =~ ^{ ]]; then\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("if"));
    assert_next_token(&mut lexer, TokenKind::DoubleLeftBracket, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("$MOTD"));
    assert_next_token(&mut lexer, TokenKind::DoubleRightBracket, None);
    assert_next_token(&mut lexer, TokenKind::And, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("!"));
    assert_next_token(&mut lexer, TokenKind::DoubleLeftBracket, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("$MOTD"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("=~"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("^{"));
    assert_next_token(&mut lexer, TokenKind::DoubleRightBracket, None);
    assert_next_token(&mut lexer, TokenKind::Semicolon, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("then"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_indexed_array_not_collapsed() {
    // arr=("hello world") should NOT be collapsed — parser handles
    // quoted elements token-by-token via the LeftParen path
    let mut lexer = Lexer::new(r#"arr=("hello world")"#);
    assert_next_token(&mut lexer, TokenKind::Word, Some("arr="));
    assert_next_token(&mut lexer, TokenKind::LeftParen, None);
}

#[test]
fn test_dollar_word_does_not_absorb_function_parens() {
    let mut lexer = Lexer::new(r#"foo$x()"#);

    assert_next_token(&mut lexer, TokenKind::Word, Some("foo$x"));
    assert_next_token(&mut lexer, TokenKind::LeftParen, None);
    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_digit_at_eof_no_panic() {
    // A lone digit with no following redirect operator must not panic
    let mut lexer = Lexer::new("2");
    let token = lexer.next_lexed_token();
    assert!(token.is_some());
}

/// Issue #599: Nested ${...} inside unquoted ${...} must be a single token.
#[test]
fn test_nested_brace_expansion_single_token() {
    // ${arr[${#arr[@]} - 1]} should be ONE word token, not split at inner }
    let mut lexer = Lexer::new("${arr[${#arr[@]} - 1]}");
    assert_next_token(&mut lexer, TokenKind::Word, Some("${arr[${#arr[@]} - 1]}"));
    // No more tokens — everything was consumed
    assert!(lexer.next_lexed_token().is_none());
}

/// Simple ${var} still works after brace depth change.
#[test]
fn test_simple_brace_expansion_unchanged() {
    let mut lexer = Lexer::new("${foo}");
    assert_next_token(&mut lexer, TokenKind::Word, Some("${foo}"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_nvm_fixture_lexes_without_stalling() {
    let input = include_str!("../../../../../shuck-benchmark/resources/files/nvm.sh");
    let mut lexer = Lexer::new(input);
    let mut tokens = 0usize;

    while lexer.next_lexed_token().is_some() {
        tokens += 1;
        assert!(
            tokens < 100_000,
            "lexer should continue making progress on the nvm fixture"
        );
    }

    assert!(tokens > 0, "nvm fixture should produce at least one token");
}

#[test]
fn test_inline_if_with_array_append_stays_line_local() {
    let input = concat!(
        "if [[ -n $arr ]]; then pyout+=(\"${output}\")\n",
        "elif [[ -n $var ]]; then pyout+=\"${output}${ln:+\\n}\"; fi\n",
    );

    assert_non_newline_tokens_stay_on_one_line(input);
}
