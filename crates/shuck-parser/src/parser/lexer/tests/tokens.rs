use super::*;

#[test]
fn test_simple_words() {
    let mut lexer = Lexer::new("echo hello world");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("world"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_operators() {
    let mut lexer = Lexer::new("a |& b | c && d || e; f &");

    assert_next_token(&mut lexer, TokenKind::Word, Some("a"));
    assert_next_token(&mut lexer, TokenKind::PipeBoth, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("b"));
    assert_next_token(&mut lexer, TokenKind::Pipe, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("c"));
    assert_next_token(&mut lexer, TokenKind::And, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("d"));
    assert_next_token(&mut lexer, TokenKind::Or, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("e"));
    assert_next_token(&mut lexer, TokenKind::Semicolon, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("f"));
    assert_next_token(&mut lexer, TokenKind::Background, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_double_left_bracket_requires_separator() {
    let mut lexer = Lexer::new("[[ foo ]]\n[[z]\n");

    assert_next_token(&mut lexer, TokenKind::DoubleLeftBracket, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("foo"));
    assert_next_token(&mut lexer, TokenKind::DoubleRightBracket, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("[[z]"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_redirects() {
    let mut lexer = Lexer::new("a > b >> c >>| d 2>>| e 2>| f < g << h <<< i &>> j <> k");

    assert_next_token(&mut lexer, TokenKind::Word, Some("a"));
    assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("b"));
    assert_next_token(&mut lexer, TokenKind::RedirectAppend, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("c"));
    assert_next_token(&mut lexer, TokenKind::RedirectAppend, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("d"));
    assert_next_token(&mut lexer, TokenKind::RedirectFdAppend, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("e"));
    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Clobber);
    assert_eq!(token.fd_value(), Some(2));
    assert_eq!(token_text(&token, lexer.input), None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("f"));
    assert_next_token(&mut lexer, TokenKind::RedirectIn, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("g"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("h"));
    assert_next_token(&mut lexer, TokenKind::HereString, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("i"));
    assert_next_token(&mut lexer, TokenKind::RedirectBothAppend, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("j"));
    assert_next_token(&mut lexer, TokenKind::RedirectReadWrite, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("k"));
}

#[test]
fn test_comment() {
    let mut lexer = Lexer::new("echo hello # this is a comment\necho world");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("world"));
}

#[test]
fn test_comment_token_with_span() {
    let mut lexer = Lexer::new("# lead\necho hi # tail");

    let comment = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(comment.kind, TokenKind::Comment);
    assert_eq!(token_text(&comment, lexer.input).as_deref(), Some(" lead"));
    assert_eq!(comment.span.start.line, 1);
    assert_eq!(comment.span.start.column, 1);
    assert_eq!(comment.span.end.line, 1);
    assert_eq!(comment.span.end.column, 7);

    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("hi"));

    let inline = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(inline.kind, TokenKind::Comment);
    assert_eq!(token_text(&inline, lexer.input).as_deref(), Some(" tail"));
    assert_eq!(inline.span.start.line, 2);
    assert_eq!(inline.span.start.column, 9);
}

#[test]
fn test_comment_token_preserves_hash_boundaries() {
    let mut lexer = Lexer::new("echo foo#bar ${x#y} '# nope' \"# nope\" # yep");

    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("foo#bar"));
    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("${x#y}"));
    assert_next_token_with_comments(&mut lexer, TokenKind::LiteralWord, Some("# nope"));
    assert_next_token_with_comments(&mut lexer, TokenKind::QuotedWord, Some("# nope"));
    assert_next_token_with_comments(&mut lexer, TokenKind::Comment, Some(" yep"));
    assert!(lexer.next_lexed_token_with_comments().is_none());
}

#[test]
fn test_variable_words() {
    let mut lexer = Lexer::new("echo $HOME $USER");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("$HOME"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("$USER"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_pipeline_tokens() {
    let mut lexer = Lexer::new("echo hello | cat");

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
    assert_next_token(&mut lexer, TokenKind::Pipe, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_comment_with_unicode() {
    // Comment containing multi-byte UTF-8 characters
    let source = "# café résumé\necho ok";
    let mut lexer = Lexer::new(source);

    let comment = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(comment.kind, TokenKind::Comment);
    assert_eq!(
        token_text(&comment, lexer.input).as_deref(),
        Some(" café résumé")
    );
    // Span should cover exactly the comment bytes (including #)
    let start = comment.span.start.offset;
    let end = comment.span.end.offset;
    assert_eq!(start, 0);
    assert_eq!(&source[start..end], "# café résumé");
    assert!(source.is_char_boundary(start));
    assert!(source.is_char_boundary(end));

    assert_next_token_with_comments(&mut lexer, TokenKind::Newline, None);
    assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("echo"));
}

#[test]
fn test_comment_with_cjk_characters() {
    // CJK characters are 3-byte UTF-8; offsets must land on char boundaries
    let source = "# 你好世界\necho ok";
    let mut lexer = Lexer::new(source);

    let comment = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(comment.kind, TokenKind::Comment);
    assert_eq!(
        token_text(&comment, lexer.input).as_deref(),
        Some(" 你好世界")
    );
    let start = comment.span.start.offset;
    let end = comment.span.end.offset;
    assert_eq!(&source[start..end], "# 你好世界");
    assert!(source.is_char_boundary(start));
    assert!(source.is_char_boundary(end));
}
