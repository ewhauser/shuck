use super::super::substitutions::hash_starts_comment;
use super::*;

#[test]
fn test_double_quoted_token_preserves_inner_quoted_command_substitution_pipeline() {
    let source = r#""$(echo "$line" | cut -d' ' -f2-)""#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::QuotedWord);
    assert_eq!(
        token.word_text(),
        Some(r#"$(echo "$line" | cut -d' ' -f2-)"#)
    );
}

#[test]
fn test_deep_command_substitution_preserves_simple_parameter_expansion() {
    let source = r#""$(echo "$(echo "$(echo "$(echo "${name}")")")")""#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::QuotedWord);
    assert_eq!(
        token.word_text(),
        Some(r#"$(echo "$(echo "$(echo "$(echo "${name}")")")")"#)
    );
}

#[test]
fn test_command_substitution_preserves_deep_parameter_operand_paren() {
    let source = r#""$(echo "${a:-${b:-${c:-${d:-${e:-x})}}}}")""#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::QuotedWord);
    assert_eq!(
        token.word_text(),
        Some(r#"$(echo "${a:-${b:-${c:-${d:-${e:-x})}}}}")"#)
    );
}

#[test]
fn test_scan_command_substitution_body_len_handles_separator_started_comment() {
    let source = "printf '%s' x;# comment with ) and ,\nprintf '%s' y\n)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf '%s' y"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_grouping_comment_after_left_paren() {
    let source = " (# comment with )\nprintf %s 1,2\n) )\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_parameter_expansion_with_right_paren() {
    let source = "printf %s ${x//foo/)},1)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("${x//foo/)},1"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_case_pattern_comment_after_right_paren() {
    let source = "case $kind in\na)# comment with esac )\nprintf %s 1,2 ;;\nesac\n)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_hash_starts_comment_ignores_zsh_inline_glob_controls_after_left_paren() {
    let source = "[[ \"$buf\" == (#b)(*) ]]";
    let index = source.find('#').expect("expected hash");

    assert!(!hash_starts_comment(source, index));
}

#[test]
fn test_hash_starts_comment_allows_grouped_comments_without_space_after_hash() {
    let source = "(#comment with )";
    let index = source.find('#').expect("expected hash");

    assert!(hash_starts_comment(source, index));
}

#[test]
fn test_hash_starts_comment_ignores_hash_inside_unclosed_double_parens() {
    let source = "(( #c < 256 ))";
    let index = source.find('#').expect("expected hash");

    assert!(!hash_starts_comment(source, index));
}

#[test]
fn test_hash_starts_comment_respects_quoted_double_parens() {
    let source = "printf '((' # comment";
    let index = source.find('#').expect("expected hash");

    assert!(hash_starts_comment(source, index));
}

#[test]
fn test_scan_command_substitution_body_len_handles_quoted_double_parens_before_comments() {
    let source = "printf '((' # comment with )\nprintf %s 1,2\n)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_grouped_comments_without_space_after_hash() {
    let source = " (#comment with )\nprintf %s 1,2\n) )\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_nested_case_pattern_right_paren() {
    let source = "(case $kind in\na) printf %s 1,2 ;;\nesac\n))\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with("))"));
}

#[test]
fn test_scan_command_substitution_body_len_ignores_plain_case_words_in_commands() {
    let source = "printf %s 1,2; echo case in)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("echo case in"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_ansi_c_quotes_with_escaped_single_quotes() {
    let source = "printf %s $'a\\'b'; printf %s 1,2)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("$'a\\'b'"));
    assert!(body.contains("printf %s 1,2"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_backticks_with_right_parens() {
    let source = "printf %s `echo foo)`; printf %s ok)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("`echo foo)`"));
    assert!(body.contains("printf %s ok"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_backticks_inside_parameter_expansions() {
    let source = "printf %s ${x/`echo }`/foo)},1)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("${x/`echo }`/foo)},1"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_process_substitutions_inside_parameter_expansions()
 {
    let source = "printf %s ${x/<(echo })/foo)},1)\"";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert!(body.contains("${x/<(echo })/foo)},1"));
    assert!(body.ends_with(')'));
}

#[test]
fn test_scan_command_substitution_body_len_handles_plain_case_words_at_eof() {
    let source = "printf %s 1,2; echo case in)";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_scan_command_substitution_body_len_handles_ansi_c_quotes_at_eof() {
    let source = "printf %s $'a\\'b'; printf %s 1,2)";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_scan_command_substitution_body_len_handles_backticks_with_right_parens_at_eof() {
    let source = "printf %s `echo foo)`; printf %s ok)";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_scan_command_substitution_body_len_handles_inner_quotes_in_pipeline_at_eof() {
    let source = "echo \"$line\" | cut -d' ' -f2-)";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_scan_command_substitution_body_len_handles_braced_params_in_pipeline_at_eof() {
    let source = "echo \"${@}\" | tr -d '[:space:]')";

    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    let body = &source[..consumed];

    assert_eq!(body, source);
}

#[test]
fn test_lexer_handles_quoted_right_paren_inside_command_substitution_nested_in_arithmetic() {
    let source = "echo \"$(echo \"$(( $(printf ')') + 1 ))\")\"";
    let mut lexer = Lexer::new(source);

    let first = lexer.next_lexed_token().expect("expected first token");
    assert!(first.kind.is_word_like(), "{:?}", first.kind);
    assert_eq!(first.word_string().as_deref(), Some("echo"));

    let second = lexer.next_lexed_token().expect("expected second token");
    assert!(second.kind.is_word_like(), "{:?}", second.kind);
    assert_eq!(
        second.word_string().as_deref(),
        Some("$(echo \"$(( $(printf ')') + 1 ))\")")
    );
}

#[test]
fn test_scan_command_substitution_body_len_handles_escaped_quotes_before_substitution_tail() {
    let source = "echo -n \"\\\"adp_$(echo $var | tr A-Z a-z)\\\": [\"";
    let start = source.find("$(").expect("expected command substitution") + 2;
    let consumed = scan_command_substitution_body_len(&source[start..]).expect("expected match");
    assert_eq!(&source[start..start + consumed], "echo $var | tr A-Z a-z)");
}

#[test]
fn test_scan_command_substitution_body_len_keeps_nested_command_names() {
    let source = "echo $(echo $(basename $filename .fuzz))";
    let start = source.find("$(").expect("expected command substitution") + 2;
    let consumed = scan_command_substitution_body_len(&source[start..]).expect("expected match");
    assert_eq!(
        &source[start..start + consumed],
        "echo $(basename $filename .fuzz))"
    );
}

#[test]
fn test_scan_command_substitution_body_len_keeps_quoted_nested_control_command() {
    let source = "\n       [[ \"$config_file\" == *\"$theme.cfg\" ]] && echo \"$(basename \"$config_file\")\"\n    )";
    let consumed = scan_command_substitution_body_len(source).expect("expected match");
    assert_eq!(consumed, source.len());
}

#[test]
fn test_unquoted_command_substitution_word_keeps_source_backing() {
    let source = "$(printf hi)";
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
fn test_quoted_prefix_with_command_substitution_continuation_keeps_source_backing() {
    let source = "\"foo\"$(printf hi)";
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);

    let word = token.word().unwrap();
    let continuation = word.segments().nth(1).unwrap();
    assert_eq!(continuation.kind(), LexedWordSegmentKind::Plain);
    assert_eq!(continuation.as_str(), "$(printf hi)");
    assert_eq!(continuation.span().unwrap().slice(source), "$(printf hi)");
}

#[test]
fn test_parameter_expansion_replacing_double_quote_stays_on_one_line() {
    let source = r#"out_line="${out_line//'"'/'\"'}"
"#;
    let mut lexer = Lexer::new(source);

    assert_next_token(
        &mut lexer,
        TokenKind::Word,
        Some(r#"out_line=${out_line//'"'/'"'}"#),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_parameter_expansion_replacing_double_quote_does_not_swallow_following_commands() {
    let source = r#"out_line="${out_line//'"'/'\"'}"
echo "Error: Missing python3!"
cat << 'EOF' > "${pywrapper}"
import os
EOF
"#;
    let mut lexer = Lexer::new(source);

    assert_next_token(
        &mut lexer,
        TokenKind::Word,
        Some(r#"out_line=${out_line//'"'/'"'}"#),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(
        &mut lexer,
        TokenKind::QuotedWord,
        Some("Error: Missing python3!"),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
    assert_next_token(&mut lexer, TokenKind::HereDoc, None);
    assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("EOF"));
    assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
    assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("${pywrapper}"));
}

#[test]
fn test_parameter_expansion_replacement_with_escaped_backslashes_stays_single_token() {
    let source = "crypt=${crypt//\\\\/\\\\\\\\}\n";
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), "crypt=${crypt//\\\\/\\\\\\\\}");
    assert!(token.source_slice(source).is_none());
    assert_eq!(
        token.word_string().as_deref(),
        Some("crypt=${crypt//\\/\\\\}")
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_midword_brace_expansion_with_command_substitution_stays_single_word() {
    let source = "echo -{$(echo a),b}-\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("-{$(echo a),b}-"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_midword_brace_expansion_with_arithmetic_substitution_stays_single_word() {
    let source = "echo -{$((1 + 2)),b}-\n";
    let mut lexer = Lexer::new(source);

    assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("-{$((1 + 2)),b}-"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_zsh_arithmetic_char_literal_inside_double_parens_is_not_comment() {
    let mut lexer = Lexer::new("(( #c < 256 / $1 * $1 )) && break\n");

    let mut saw_comment = false;
    while let Some(token) = lexer.next_lexed_token_with_comments() {
        if token.kind == TokenKind::Comment {
            saw_comment = true;
            break;
        }
    }

    assert!(
        !saw_comment,
        "zsh arithmetic char literals inside (( )) should not lex as comments"
    );
}

#[test]
fn test_double_quoted_parameter_replacement_with_embedded_quotes_stays_single_word() {
    let mut lexer = Lexer::new(
        "builtin printf '\\e]133;C;cmdline_url=%s\\a' \"${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}\"\n",
    );

    assert_next_token(&mut lexer, TokenKind::Word, Some("builtin"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("printf"));
    assert_next_token(
        &mut lexer,
        TokenKind::LiteralWord,
        Some("\\e]133;C;cmdline_url=%s\\a"),
    );
    assert_next_token(
        &mut lexer,
        TokenKind::QuotedWord,
        Some("${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}"),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
}

#[test]
fn test_anonymous_function_body_with_nested_replacement_word_keeps_closing_brace_token() {
    let mut lexer = Lexer::new(
        "() {\n  builtin printf '\\e]133;C;cmdline_url=%s\\a' \"${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}\"\n} \"$1\"\n",
    );

    assert_next_token(&mut lexer, TokenKind::LeftParen, None);
    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert_next_token(&mut lexer, TokenKind::LeftBrace, None);
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::Word, Some("builtin"));
    assert_next_token(&mut lexer, TokenKind::Word, Some("printf"));
    assert_next_token(
        &mut lexer,
        TokenKind::LiteralWord,
        Some("\\e]133;C;cmdline_url=%s\\a"),
    );
    assert_next_token(
        &mut lexer,
        TokenKind::QuotedWord,
        Some("${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}"),
    );
    assert_next_token(&mut lexer, TokenKind::Newline, None);
    assert_next_token(&mut lexer, TokenKind::RightBrace, None);
    assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("$1"));
    assert_next_token(&mut lexer, TokenKind::Newline, None);
}

#[test]
fn test_parameter_expansion_with_zsh_qualifier_stays_single_word() {
    let source = r#"$dir/${~pats}(N)"#;
    let mut lexer = Lexer::new(source);

    let token = lexer.next_lexed_token().unwrap();
    assert_eq!(token.kind, TokenKind::Word);
    assert_eq!(token.span.slice(source), source);
    assert!(lexer.next_lexed_token().is_none());
}

#[test]
fn test_command_substitution_word_does_not_absorb_function_parens() {
    let mut lexer = Lexer::new(r#"foo-$(echo hi)()"#);

    assert_next_token(&mut lexer, TokenKind::Word, Some("foo-$(echo hi)"));
    assert_next_token(&mut lexer, TokenKind::LeftParen, None);
    assert_next_token(&mut lexer, TokenKind::RightParen, None);
    assert!(lexer.next_lexed_token().is_none());
}
