use super::*;

#[test]
fn test_scan_command_substitution_body_len_handles_tabstripped_heredoc() {
    let source = "\n\t\t\tcat <<-EOF | tr '\\n' ' '\n\t\t\t\t{\"query\":\"field, direction\"}\n\t\t\tEOF\n\t\t)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("field, direction"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_piped_heredoc_delimiter_without_space() {
    let source = "\ncat <<EOF|tr '\\n' ' '\n{\"query\":\"field, direction\"}\nEOF\n)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("field, direction"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_ignores_arithmetic_shift_for_heredoc_detection() {
    let source = "((x<<2))\nprintf %s 1,2\n)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_tabstripped_heredoc_at_eof() {
    let source = "\n\t\t\tcat <<-EOF | tr '\\n' ' '\n\t\t\t\t{\"query\":\"field, direction\"}\n\t\t\tEOF\n\t\t)";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_scan_command_substitution_body_len_handles_piped_heredoc_at_eof() {
    let source = "cat <<EOF|tr '\\n' ' '\n{\"query\":\"field, direction\"}\nEOF\n)";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_read_heredoc() {
    // Simulate state after reading "cat <<EOF" - positioned at newline before content
    let mut lexer = Lexer::new("\nhello\nworld\nEOF");
    let content = lexer.read_heredoc("EOF", false);
    assert_eq!(content.content, "hello\nworld\n");
}

#[test]
fn test_read_heredoc_single_line() {
    let mut lexer = Lexer::new("\ntest\nEOF");
    let content = lexer.read_heredoc("EOF", false);
    assert_eq!(content.content, "test\n");
}

#[test]
fn test_read_heredoc_full_scenario() {
    // Full scenario: "cat <<EOF\nhello\nworld\nEOF"
    let mut lexer = Lexer::new("cat <<EOF\nhello\nworld\nEOF");

    // Parser would read these tokens
    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    // Now read heredoc content
    let content = lexer.read_heredoc("EOF", false);
    assert_eq!(content.content, "hello\nworld\n");
}

#[test]
fn test_read_heredoc_with_redirect() {
    // Rest-of-line (> file.txt) is re-injected into the lexer buffer
    let mut lexer = Lexer::new("cat <<EOF > file.txt\nhello\nEOF");
    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));
    let content = lexer.read_heredoc("EOF", false);
    assert_eq!(content.content, "hello\n");
    // The redirect tokens are now available from the lexer
    assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("file.txt"));
}

#[test]
fn test_read_heredoc_reinjects_line_continued_pipeline_tail() {
    let source = "cat <<EOF | grep hello \\\n  | sort \\\n  > out.txt\nhello\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "hello\n");

    assert_next_token(&mut lexer, TokenKind::Pipe, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("grep"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
    assert_next_token(&mut lexer, TokenKind::Pipe, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("sort"));
    assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("out.txt"));
}

#[test]
fn test_read_heredoc_does_not_continue_body_when_backslash_is_immediately_after_delimiter() {
    let source = "cat <<EOF \\\n1\n2\n3\nEOF\n| tac\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "1\n2\n3\n");
}

#[test]
fn test_read_heredoc_escaped_backslash_before_newline_does_not_continue_tail() {
    let source = "cat <<EOF foo\\\\\nbody\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "body\n");
}

#[test]
fn test_read_heredoc_comment_backslash_does_not_continue_tail() {
    let source = "cat <<EOF # note \\\nbody\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "body\n");
}

#[test]
fn test_read_heredoc_right_paren_comment_backslash_does_not_continue_tail() {
    let source = "( cat <<EOF )# note \\\nbody\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::LeftParen, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "body\n");

    assert_next_token(&mut lexer, TokenKind::RightParen, None);
}

#[test]
fn test_read_heredoc_blank_prefix_continues_into_operator_led_tail() {
    let source = "cat <<EOF \\\n| tac\n1\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "1\n");

    assert_next_token(&mut lexer, TokenKind::Pipe, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("tac"));
}

#[test]
fn test_read_heredoc_with_redirect_preserves_following_spans() {
    let source = "cat <<EOF > file.txt\nhello\nEOF\n# done\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "hello\n");

    let redirect = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(redirect.kind, TokenKind::RedirectOut);
    assert_eq!(redirect.span.slice(source), ">");

    let target = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(target.kind, TokenKind::Word);
    assert_eq!(
        token_text(&target, lexer.input).as_deref(),
        Some("file.txt")
    );
    assert_eq!(target.span.slice(source), "file.txt");

    let newline = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(newline.kind, TokenKind::Newline);
    assert_eq!(newline.span.slice(source), "\n");

    let comment = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(comment.kind, TokenKind::Comment);
    assert_eq!(token_text(&comment, lexer.input).as_deref(), Some(" done"));
    assert_eq!(comment.span.slice(source), "# done");
}

#[test]
fn test_heredoc_with_comments_inside() {
    // Comments inside heredoc body should NOT appear as comment tokens
    let source = "cat <<EOF\n# not a comment\nreal line\nEOF\n# real comment\n";
    let mut lexer = Lexer::new(source);

    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token_with_comments(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "# not a comment\nreal line\n");

    // After heredoc, replayed line termination should appear before
    // tokens from following source lines.
    assert_next_token_with_comments(&mut lexer, TokenKind::Newline, None);
    let comment = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(comment.kind, TokenKind::Comment);
    assert_eq!(
        token_text(&comment, lexer.input).as_deref(),
        Some(" real comment")
    );
}

#[test]
fn test_heredoc_with_hash_in_variable() {
    // ${var#pattern} inside heredoc should not produce comment tokens
    let source = "cat <<EOF\nval=${x#prefix}\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token_with_comments(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "val=${x#prefix}\n");
}

#[test]
fn test_heredoc_span_does_not_leak() {
    // Heredoc content span must be within source bounds and must not
    // overlap with content before or after.
    let source = "cat <<EOF\nhello\nworld\nEOF\necho after";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    let start = heredoc.content_span.start.offset;
    let end = heredoc.content_span.end.offset;
    assert!(
        end <= source.len(),
        "heredoc span end ({end}) exceeds source length ({})",
        source.len()
    );
    assert_eq!(&source[start..end], "hello\nworld\n");

    // Tokens after heredoc should still parse correctly
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("after"));
}

#[test]
fn test_quoted_heredoc_preserves_following_backtick_word_spans() {
    let source = "\
cat <<\\_ACEOF
Use these variables to override the choices made by `configure' or to help
it to find libraries and programs with nonstandard names/locations.
_ACEOF
ac_dir_suffix=/`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`
ac_top_builddir_sub=`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`
";
    let mut lexer = Lexer::new(source);

    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token_with_comments(&mut lexer, TokenKind::HereDoc, None);
    let delimiter = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(delimiter.kind, TokenKind::Word);
    assert_eq!(delimiter.span.slice(source), "\\_ACEOF");

    let heredoc = lexer.read_heredoc("_ACEOF", false);
    assert_eq!(
        heredoc.content,
        "Use these variables to override the choices made by `configure' or to help\nit to find libraries and programs with nonstandard names/locations.\n"
    );

    assert_next_token_with_comments(&mut lexer, TokenKind::Newline, None);

    let first = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(first.kind, TokenKind::Word);
    assert_eq!(
        first.span.slice(source),
        "ac_dir_suffix=/`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`"
    );
    let first_segments = first
        .word()
        .unwrap()
        .segments()
        .map(|segment| {
            (
                segment.kind(),
                segment.as_str().to_string(),
                segment.span().map(|span| span.slice(source).to_string()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        first_segments,
        vec![
            (
                LexedWordSegmentKind::Plain,
                "ac_dir_suffix=/".to_string(),
                Some("ac_dir_suffix=/".to_string()),
            ),
            (
                LexedWordSegmentKind::Plain,
                "`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`".to_string(),
                Some("`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`".to_string()),
            ),
        ]
    );

    assert_next_token_with_comments(&mut lexer, TokenKind::Newline, None);

    let second = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(second.kind, TokenKind::Word);
    assert_eq!(
        second.span.slice(source),
        "ac_top_builddir_sub=`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`"
    );
    let second_segments = second
        .word()
        .unwrap()
        .segments()
        .map(|segment| {
            (
                segment.kind(),
                segment.as_str().to_string(),
                segment.span().map(|span| span.slice(source).to_string()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        second_segments,
        vec![
            (
                LexedWordSegmentKind::Plain,
                "ac_top_builddir_sub=".to_string(),
                Some("ac_top_builddir_sub=".to_string()),
            ),
            (
                LexedWordSegmentKind::Plain,
                "`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`".to_string(),
                Some("`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`".to_string(),),
            ),
        ]
    );
}

#[test]
fn test_heredoc_with_unicode_content() {
    // Heredoc containing multi-byte characters; spans must be on char boundaries
    let source = "cat <<EOF\n# 你好\ncafé\nEOF\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

    let heredoc = lexer.read_heredoc("EOF", false);
    assert_eq!(heredoc.content, "# 你好\ncafé\n");
    let start = heredoc.content_span.start.offset;
    let end = heredoc.content_span.end.offset;
    assert!(
        source.is_char_boundary(start),
        "heredoc span start ({start}) not on char boundary"
    );
    assert!(
        source.is_char_boundary(end),
        "heredoc span end ({end}) not on char boundary"
    );
    assert_eq!(&source[start..end], "# 你好\ncafé\n");
}

#[test]
fn test_heredoc_in_arithmetic_fuzz_crash() {
    // Regression test: the fuzzer found that heredoc re-injection inside
    // arithmetic context can push self.offset past self.input.len(),
    // causing a panic in read_unquoted_segment's borrowed-slice path.
    let data: &[u8] = &[
        35, 33, 111, 98, 105, 110, 41, 41, 10, 40, 40, 32, 36, 111, 98, 105, 110, 41, 41, 10, 40,
        40, 32, 36, 53, 32, 43, 32, 49, 32, 6, 0, 0, 0, 0, 0, 0, 0, 41, 60, 60, 69, 41, 4, 33, 61,
        26, 40, 40, 32, 110, 119, 119, 49, 32, 119, 119, 109, 119, 119, 119, 119, 119, 119, 122,
        39, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 0, 0, 0, 0, 0, 41, 60, 60,
        69, 41, 4, 33, 61, 26, 40, 40, 32, 110, 119, 119, 49, 32, 119, 119, 109, 119, 119, 110,
        119, 119, 49, 32, 119, 119, 109, 119, 119, 119, 0, 14, 119, 122, 39, 122, 122, 122, 122,
        122, 122, 122, 47, 33, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 40, 122, 122, 122,
        122, 39, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 0, 53, 32,
        43, 32, 49, 32, 41, 41, 10, 40, 40, 32, 36, 53, 32, 43, 32, 49, 32, 6, 0, 0, 0, 0, 0, 0, 0,
        41, 60, 60, 69, 41, 4, 33, 61, 26, 40, 40, 32, 110, 119, 119, 49, 32, 119, 119, 109, 119,
        119, 119, 119, 119, 119, 122, 39, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122,
        122, 0, 0, 0, 0, 0, 41, 60, 60, 69, 41, 4, 33, 61, 26, 40, 40, 32, 110, 119, 119, 48, 32,
        119, 119, 109, 119, 119, 110, 119, 119, 49, 32, 119, 119, 109, 119, 119, 119, 0, 14, 119,
        122, 39, 122, 122, 122, 122, 122, 122, 122, 47, 33, 122, 122, 122, 122, 122, 122, 122, 122,
        122, 122, 40, 122, 122, 122, 122, 39, 122, 122, 122, 122, 122, 122, 122, 88, 88, 88, 88,
        122, 122, 40, 122, 122, 122, 122, 39, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122,
        122, 122, 122, 122, 0, 53, 32, 43, 32, 49, 32, 53, 41, 10, 40, 40, 32, 36, 53, 32, 43, 32,
        49, 32, 6, 0, 0, 0, 0, 0, 0, 0, 41, 60, 60, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 0,
        0, 0,
    ];
    let input = std::str::from_utf8(data).unwrap();
    let script = format!("echo $(({input}))\n");
    // Must not panic.
    let _ = crate::parser::Parser::new(&script).parse();
}
