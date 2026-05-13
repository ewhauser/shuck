use super::*;

fn token_text(token: &LexedToken<'_>, source: &str) -> Option<String> {
    match token.kind {
        kind if kind.is_word_like() => token.word_string(),
        TokenKind::Comment => token
            .span
            .slice(source)
            .strip_prefix('#')
            .map(str::to_string),
        TokenKind::Error => token
            .error_kind()
            .map(LexerErrorKind::message)
            .map(str::to_string),
        _ => None,
    }
}

fn assert_next_token(lexer: &mut Lexer<'_>, expected_kind: TokenKind, expected_text: Option<&str>) {
    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, expected_kind);
    assert_eq!(token_text(&token, lexer.input).as_deref(), expected_text);
}

fn assert_next_token_with_comments(
    lexer: &mut Lexer<'_>,
    expected_kind: TokenKind,
    expected_text: Option<&str>,
) {
    let token = lexer.next_lexed_token_with_comments().unwrap();
    assert_eq!(token.kind, expected_kind);
    assert_eq!(token_text(&token, lexer.input).as_deref(), expected_text);
}

fn assert_non_newline_tokens_stay_on_one_line(input: &str) {
    let mut lexer = Lexer::new(input);

    while let Some(token) = lexer.next_lexed_token() {
        if token.kind == TokenKind::Newline {
            continue;
        }

        assert_eq!(
            token.span.start.line, token.span.end.line,
            "token should stay on one line: {:?}",
            token
        );
    }
}

mod heredoc;
mod quotes;
mod substitutions;
mod tokens;
mod words;
mod zsh;
