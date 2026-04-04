//! Linter-oriented syntax wrapper over bashkit's parser.

mod dialects;
mod suppressions;

pub use bashkit::parser::Script;
pub use dialects::{
    Dialect, DialectProfile, Grammar, ParseMode, ParseOptions, ParseStrategy, ParseView,
};
pub use suppressions::{Suppression, SuppressionIndex, SuppressionKind};

use bashkit::Error as BashkitError;
use bashkit::parser::{
    Lexer, ParseDiagnostic as BashkitParseDiagnostic, Parser, Position, Span, Token,
};
use thiserror::Error;

/// Source position tracked through lexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl From<Position> for SourcePosition {
    fn from(position: Position) -> Self {
        Self {
            line: position.line,
            column: position.column,
            offset: position.offset,
        }
    }
}

/// Source span tracked through lexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: SourcePosition,
    pub end: SourcePosition,
}

impl From<Span> for SourceSpan {
    fn from(span: Span) -> Self {
        Self {
            start: span.start.into(),
            end: span.end.into(),
        }
    }
}

/// A shell comment with its original source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub text: String,
    pub span: SourceSpan,
}

/// A parse diagnostic surfaced from recovered parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub message: String,
    pub span: SourceSpan,
}

impl From<BashkitParseDiagnostic> for ParseDiagnostic {
    fn from(diagnostic: BashkitParseDiagnostic) -> Self {
        Self {
            message: diagnostic.message,
            span: diagnostic.span.into(),
        }
    }
}

/// Whether a comment occupies its own line or trails code on the same line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentPlacement {
    OwnLine,
    Trailing,
}

/// The directive namespace recognized from a shell comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectiveSource {
    Shuck,
    ShellCheck,
}

/// Supported suppression actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuppressionAction {
    Disable,
    Enable,
    DisableFile,
}

impl SuppressionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Enable => "enable",
            Self::DisableFile => "disable-file",
        }
    }
}

/// A parsed suppression directive from either shuck or shellcheck comment syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionDirective {
    pub source: DirectiveSource,
    pub action: SuppressionAction,
    pub codes: Vec<String>,
    pub placement: CommentPlacement,
    pub comment_text: String,
    pub span: SourceSpan,
}

/// Why a directive-shaped comment could not be used as a valid suppression directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MalformedDirectiveReason {
    MissingActionAssignment,
    UnsupportedAction,
    MissingCodes,
    InvalidCodes,
    TrailingShellCheckDirective,
}

/// A directive-like comment that was recognized but could not be parsed as a valid directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedDirective {
    pub source: DirectiveSource,
    pub reason: MalformedDirectiveReason,
    pub placement: CommentPlacement,
    pub comment_text: String,
    pub span: SourceSpan,
}

/// A directive parsed from a shell comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Suppression(SuppressionDirective),
    Malformed(MalformedDirective),
}

/// Parse result for the current minimal frontend.
#[derive(Debug, Clone)]
pub struct ParsedSyntax {
    pub view: ParseView,
    pub script: Script,
    pub comments: Vec<Comment>,
    pub directives: Vec<Directive>,
    pub diagnostics: Vec<ParseDiagnostic>,
    suppression_index: SuppressionIndex,
}

impl ParsedSyntax {
    pub fn dialect(&self) -> Dialect {
        self.view.dialect
    }

    pub fn grammar(&self) -> Grammar {
        self.view.grammar
    }

    pub fn is_permissive(&self) -> bool {
        self.view.is_permissive()
    }

    pub fn is_recovered(&self) -> bool {
        self.view.is_recovered()
    }

    pub fn suppression_index(&self) -> &SuppressionIndex {
        &self.suppression_index
    }

    pub fn suppression_for_line(&self, code: &str, line: usize) -> Option<Suppression> {
        self.suppression_index.match_line(code, line)
    }

    pub fn suppression_for_line_with_aliases<'a, I>(
        &self,
        line: usize,
        codes: I,
    ) -> Option<Suppression>
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.suppression_index.match_line_with_aliases(line, codes)
    }

    pub fn suppresses_whole_file(&self, code: &str) -> bool {
        self.suppression_index.suppresses_whole_file(code)
    }

    pub fn suppresses_whole_file_with_aliases<'a, I>(&self, codes: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.suppression_index
            .suppresses_whole_file_with_aliases(codes)
    }
}

/// Error surface for the linter-oriented wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("{dialect} does not support {mode} parsing yet")]
    UnsupportedParse { dialect: Dialect, mode: ParseMode },
    #[error("parse error at line {line}, column {column}: {message}")]
    Parse {
        message: String,
        line: usize,
        column: usize,
    },
    #[error("{message}")]
    Backend { message: String },
}

impl ParseError {
    fn from_bashkit(error: BashkitError) -> Self {
        let BashkitError::Parse {
            message,
            line,
            column,
        } = error;
        Self::Parse {
            message,
            line,
            column,
        }
    }
}

/// Collect comments while preserving bashkit's definition of when `#` starts a comment.
pub fn collect_comments(input: &str) -> Vec<Comment> {
    let mut comments = collect_comments_with_base(input, Position::new());
    collect_nested_substitution_comments(input, Position::new(), &mut comments);
    comments.sort_by(|left, right| {
        left.span
            .start
            .offset
            .cmp(&right.span.start.offset)
            .then(left.span.end.offset.cmp(&right.span.end.offset))
    });
    comments
}

/// Collect recognized directives and malformed directive-shaped comments.
pub fn collect_directives(input: &str) -> Vec<Directive> {
    let comments = collect_comments(input);
    collect_directives_from_comments(input, &comments)
}

/// Parse a shell source string with the currently supported frontend settings.
pub fn parse(input: &str, options: ParseOptions) -> Result<ParsedSyntax, ParseError> {
    let view = DialectProfile::for_dialect(options.dialect)
        .parse_view(options.mode)
        .map_err(|request| ParseError::UnsupportedParse {
            dialect: request.dialect,
            mode: request.mode,
        })?;
    let comments = collect_comments(input);
    let directives = collect_directives_from_comments(input, &comments);
    let (script, diagnostics) = match (view.grammar, view.is_recovered()) {
        (Grammar::Bash, false) => {
            let script = Parser::new(input)
                .parse()
                .map_err(ParseError::from_bashkit)?;
            (script, Vec::new())
        }
        (Grammar::Bash, true) => {
            let recovered = Parser::new(input).parse_recovered();
            (
                recovered.script,
                recovered
                    .diagnostics
                    .into_iter()
                    .map(ParseDiagnostic::from)
                    .collect(),
            )
        }
    };
    let suppression_index = SuppressionIndex::from_parts(&script, &directives);

    Ok(ParsedSyntax {
        view,
        script,
        comments,
        directives,
        diagnostics,
        suppression_index,
    })
}

fn collect_directives_from_comments(input: &str, comments: &[Comment]) -> Vec<Directive> {
    comments
        .iter()
        .filter_map(|comment| parse_directive(input, comment))
        .collect()
}

fn collect_comments_with_base(input: &str, base: Position) -> Vec<Comment> {
    let mut lexer = Lexer::new(input);
    let mut comments = Vec::new();

    while let Some(token) = lexer.next_spanned_token_with_comments() {
        if let Token::Comment(text) = token.token {
            comments.push(Comment {
                text,
                span: token.span.rebased(base).into(),
            });
        }
    }

    comments
}

fn collect_nested_substitution_comments(input: &str, base: Position, comments: &mut Vec<Comment>) {
    let mut index = 0;
    let mut cursor = base;

    while index < input.len() {
        let Some(ch) = input[index..].chars().next() else {
            break;
        };

        if ch == '\'' {
            let next = skip_single_quoted(input, index);
            cursor = cursor.advanced_by(&input[index..next]);
            index = next;
            continue;
        }

        if ch == '$'
            && let Some((next_index, next_char)) = next_char_at(input, index + ch.len_utf8())
            && next_char == '('
            && !matches!(
                next_char_at(input, next_index + next_char.len_utf8()),
                Some((_, '('))
            )
            && let Some(close_index) =
                find_matching_substitution_end(input, next_index + next_char.len_utf8())
        {
            let inner_start = next_index + next_char.len_utf8();
            let inner_base = cursor.advanced_by("$(");
            let inner_source = &input[inner_start..close_index];
            comments.extend(collect_comments_with_base(inner_source, inner_base));
            collect_nested_substitution_comments(inner_source, inner_base, comments);
            cursor = cursor.advanced_by(&input[index..close_index + 1]);
            index = close_index + 1;
            continue;
        }

        if matches!(ch, '<' | '>')
            && let Some((next_index, next_char)) = next_char_at(input, index + ch.len_utf8())
            && next_char == '('
            && let Some(close_index) =
                find_matching_substitution_end(input, next_index + next_char.len_utf8())
        {
            let inner_start = next_index + next_char.len_utf8();
            let inner_base = cursor.advanced_by(&input[index..inner_start]);
            let inner_source = &input[inner_start..close_index];
            comments.extend(collect_comments_with_base(inner_source, inner_base));
            collect_nested_substitution_comments(inner_source, inner_base, comments);
            cursor = cursor.advanced_by(&input[index..close_index + 1]);
            index = close_index + 1;
            continue;
        }

        cursor.advance(ch);
        index += ch.len_utf8();
    }
}

fn next_char_at(input: &str, index: usize) -> Option<(usize, char)> {
    if index >= input.len() {
        None
    } else {
        input[index..].chars().next().map(|ch| (index, ch))
    }
}

fn skip_single_quoted(input: &str, start: usize) -> usize {
    let mut index = start + '\''.len_utf8();
    while let Some((next_index, ch)) = next_char_at(input, index) {
        index = next_index + ch.len_utf8();
        if ch == '\'' {
            break;
        }
    }
    index
}

fn skip_comment(input: &str, start: usize) -> usize {
    let mut index = start;
    while let Some((next_index, ch)) = next_char_at(input, index) {
        index = next_index + ch.len_utf8();
        if ch == '\n' {
            break;
        }
    }
    index
}

fn find_matching_substitution_end(input: &str, mut index: usize) -> Option<usize> {
    let mut depth = 1usize;

    while let Some((next_index, ch)) = next_char_at(input, index) {
        match ch {
            '\'' => {
                index = skip_single_quoted(input, next_index);
            }
            '"' => {
                index = next_index + ch.len_utf8();
                while let Some((quoted_index, quoted_char)) = next_char_at(input, index) {
                    if quoted_char == '\\' {
                        index = quoted_index + quoted_char.len_utf8();
                        if let Some((escaped_index, escaped_char)) = next_char_at(input, index) {
                            index = escaped_index + escaped_char.len_utf8();
                        }
                        continue;
                    }
                    if quoted_char == '"' {
                        index = quoted_index + quoted_char.len_utf8();
                        break;
                    }
                    index = quoted_index + quoted_char.len_utf8();
                }
            }
            '\\' => {
                index = next_index + ch.len_utf8();
                if let Some((escaped_index, escaped_char)) = next_char_at(input, index) {
                    index = escaped_index + escaped_char.len_utf8();
                }
            }
            '#' => {
                index = skip_comment(input, next_index);
            }
            '(' => {
                depth += 1;
                index = next_index + ch.len_utf8();
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(next_index);
                }
                index = next_index + ch.len_utf8();
            }
            _ => {
                index = next_index + ch.len_utf8();
            }
        }
    }

    None
}

fn parse_directive(input: &str, comment: &Comment) -> Option<Directive> {
    let text = comment.text.trim();
    if text.is_empty() {
        return None;
    }

    let lower = text.to_ascii_lowercase();
    if lower.starts_with("shuck:") {
        return Some(parse_shuck_directive(input, comment, text));
    }

    if lower.starts_with("shellcheck") && lower.contains("disable=") {
        return Some(parse_shellcheck_disable_directive(input, comment, text));
    }

    None
}

fn parse_shuck_directive(input: &str, comment: &Comment, text: &str) -> Directive {
    let placement = comment_placement(input, comment);
    let body = text["shuck:".len()..].trim();
    let Some((action, raw_codes)) = body.split_once('=') else {
        return Directive::Malformed(MalformedDirective {
            source: DirectiveSource::Shuck,
            reason: MalformedDirectiveReason::MissingActionAssignment,
            placement,
            comment_text: comment.text.clone(),
            span: comment.span,
        });
    };

    let action = action.trim().to_ascii_lowercase();
    let Some(action) = parse_shuck_action(&action) else {
        return Directive::Malformed(MalformedDirective {
            source: DirectiveSource::Shuck,
            reason: MalformedDirectiveReason::UnsupportedAction,
            placement,
            comment_text: comment.text.clone(),
            span: comment.span,
        });
    };

    let trimmed_codes = trim_directive_reason(raw_codes);
    let codes = normalize_codes(trimmed_codes.split(','), canonicalize_shuck_code);
    let reason = if trimmed_codes.trim().is_empty() {
        MalformedDirectiveReason::MissingCodes
    } else if codes.is_empty() {
        MalformedDirectiveReason::InvalidCodes
    } else {
        return Directive::Suppression(SuppressionDirective {
            source: DirectiveSource::Shuck,
            action,
            codes,
            placement,
            comment_text: comment.text.clone(),
            span: comment.span,
        });
    };

    Directive::Malformed(MalformedDirective {
        source: DirectiveSource::Shuck,
        reason,
        placement,
        comment_text: comment.text.clone(),
        span: comment.span,
    })
}

fn parse_shellcheck_disable_directive(input: &str, comment: &Comment, text: &str) -> Directive {
    let placement = comment_placement(input, comment);
    if placement == CommentPlacement::Trailing {
        return Directive::Malformed(MalformedDirective {
            source: DirectiveSource::ShellCheck,
            reason: MalformedDirectiveReason::TrailingShellCheckDirective,
            placement,
            comment_text: comment.text.clone(),
            span: comment.span,
        });
    }

    let body = text["shellcheck".len()..].trim();
    let groups = shellcheck_disable_code_groups(body);
    let raw_codes: Vec<&str> = groups
        .iter()
        .flat_map(|group| trim_directive_reason(group).split(','))
        .collect();
    let codes = normalize_codes(raw_codes, canonicalize_shellcheck_code);

    let reason = if groups.is_empty() {
        MalformedDirectiveReason::MissingCodes
    } else if codes.is_empty() {
        MalformedDirectiveReason::InvalidCodes
    } else {
        return Directive::Suppression(SuppressionDirective {
            source: DirectiveSource::ShellCheck,
            action: SuppressionAction::Disable,
            codes,
            placement,
            comment_text: comment.text.clone(),
            span: comment.span,
        });
    };

    Directive::Malformed(MalformedDirective {
        source: DirectiveSource::ShellCheck,
        reason,
        placement,
        comment_text: comment.text.clone(),
        span: comment.span,
    })
}

fn parse_shuck_action(action: &str) -> Option<SuppressionAction> {
    match action {
        "disable" => Some(SuppressionAction::Disable),
        "enable" => Some(SuppressionAction::Enable),
        "disable-file" => Some(SuppressionAction::DisableFile),
        _ => None,
    }
}

fn comment_placement(input: &str, comment: &Comment) -> CommentPlacement {
    let offset = comment.span.start.offset;
    let line_start = input[..offset].rfind('\n').map_or(0, |index| index + 1);
    if input[line_start..offset].trim().is_empty() {
        CommentPlacement::OwnLine
    } else {
        CommentPlacement::Trailing
    }
}

fn trim_directive_reason(raw: &str) -> &str {
    raw.split_once('#').map_or(raw, |(head, _)| head).trim()
}

fn shellcheck_disable_code_groups(body: &str) -> Vec<&str> {
    let mut lower = body.to_ascii_lowercase();
    let mut tail = body;
    let mut groups = Vec::new();

    loop {
        let Some(index) = lower.find("disable=") else {
            break;
        };

        tail = &tail[index + "disable=".len()..];
        lower = lower[index + "disable=".len()..].to_string();

        if let Some(next) = lower.find(" disable=") {
            groups.push(tail[..next].trim());
            tail = &tail[next + 1..];
            lower = lower[next + 1..].to_string();
        } else {
            groups.push(tail.trim());
            break;
        }
    }

    groups
}

fn normalize_codes<'a, I, F>(codes: I, canonicalize: F) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
    F: Fn(&str) -> Option<String>,
{
    let mut out = Vec::new();

    for code in codes {
        let Some(code) = canonicalize(code) else {
            continue;
        };
        if out.iter().any(|existing| existing == &code) {
            continue;
        }
        out.push(code);
    }

    out.sort();
    out
}

fn canonicalize_shuck_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();

    if let Some(digits) = code.strip_prefix("SH-") {
        return canonicalize_shuck_digits(digits);
    }
    if let Some(digits) = code.strip_prefix("SH") {
        return canonicalize_shuck_digits(digits);
    }

    None
}

fn canonicalize_shuck_digits(digits: &str) -> Option<String> {
    if digits.len() == 3 && digits.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("SH-{digits}"))
    } else {
        None
    }
}

fn canonicalize_shellcheck_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();

    if let Some(digits) = code.strip_prefix("SC")
        && !digits.is_empty()
        && digits.chars().all(|ch| ch.is_ascii_digit())
    {
        return Some(format!("SC{digits}"));
    }

    if !code.is_empty() && code.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(format!("SC{code}"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_leading_and_inline_comments() {
        let comments = collect_comments("# lead\necho hi # tail\n");

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].text, " lead");
        assert_eq!(comments[0].span.start.line, 1);
        assert_eq!(comments[0].span.start.column, 1);
        assert_eq!(comments[1].text, " tail");
        assert_eq!(comments[1].span.start.line, 2);
        assert_eq!(comments[1].span.start.column, 9);
    }

    #[test]
    fn collects_only_real_shell_comments() {
        let comments = collect_comments("echo \"# nope\" foo#bar ${x#y} '# still nope'\n# yes\n");

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, " yes");
        assert_eq!(comments[0].span.start.line, 2);
        assert_eq!(comments[0].span.start.column, 1);
    }

    #[test]
    fn collects_comments_inside_command_substitutions() {
        let comments =
            collect_comments("out=$(\n  # shellcheck disable=SC2086\n  printf '%s\\n' $x\n)\n");

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, " shellcheck disable=SC2086");
        assert_eq!(comments[0].span.start.line, 2);
        assert_eq!(comments[0].span.start.column, 3);
    }

    #[test]
    fn parse_returns_script_and_comments() {
        let parsed = parse("echo hi # tail\n", ParseOptions::default()).unwrap();

        assert_eq!(parsed.view.dialect, Dialect::Bash);
        assert_eq!(parsed.view.mode, ParseMode::Strict);
        assert_eq!(parsed.view.strategy, ParseStrategy::Native);
        assert_eq!(parsed.grammar(), Grammar::Bash);
        assert_eq!(parsed.script.commands.len(), 1);
        assert_eq!(parsed.comments.len(), 1);
        assert_eq!(parsed.directives.len(), 0);
        assert!(parsed.diagnostics.is_empty());
        assert!(!parsed.is_permissive());
        assert!(!parsed.is_recovered());
        assert_eq!(parsed.comments[0].text, " tail");
    }

    #[test]
    fn parse_exposes_suppression_queries() {
        let parsed = parse(
            "#!/bin/sh\nprintf '%s\\n' \"$x\"\n# shellcheck disable=SC2086\nprintf '%s\\n' $x\n",
            ParseOptions::default(),
        )
        .unwrap();

        assert!(parsed.diagnostics.is_empty());
        assert!(
            parsed
                .suppression_for_line_with_aliases(4, ["SH-001", "SC2086"])
                .is_some()
        );
        assert!(!parsed.suppresses_whole_file_with_aliases(["SH-001", "SC2086"]));
    }

    #[test]
    fn parse_supports_strict_recovered_mode() {
        let parsed = parse(
            "echo one\ncat >\n# shellcheck disable=SC2086\necho $x\n",
            ParseOptions {
                mode: ParseMode::StrictRecovered,
                ..ParseOptions::default()
            },
        )
        .unwrap();

        assert_eq!(parsed.view.strategy, ParseStrategy::Native);
        assert_eq!(parsed.script.commands.len(), 2);
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].message, "expected word");
        assert_eq!(parsed.diagnostics[0].span.start.line, 2);
        assert_eq!(parsed.comments.len(), 1);
        assert!(parsed.is_recovered());
        assert!(
            parsed
                .suppression_for_line_with_aliases(4, ["SH-001", "SC2086"])
                .is_some()
        );
    }

    #[test]
    fn parse_supports_dash_permissive_mode() {
        let parsed = parse(
            "echo hi\n",
            ParseOptions {
                dialect: Dialect::Dash,
                mode: ParseMode::Permissive,
            },
        )
        .unwrap();

        assert_eq!(parsed.view.dialect, Dialect::Dash);
        assert_eq!(parsed.view.mode, ParseMode::Permissive);
        assert_eq!(parsed.view.strategy, ParseStrategy::Permissive);
        assert_eq!(parsed.grammar(), Grammar::Bash);
        assert!(parsed.is_permissive());
        assert!(!parsed.is_recovered());
        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.script.commands.len(), 1);
    }

    #[test]
    fn parse_supports_dash_permissive_recovered_mode() {
        let parsed = parse(
            "echo one\ncat >\necho two\n",
            ParseOptions {
                dialect: Dialect::Dash,
                mode: ParseMode::PermissiveRecovered,
            },
        )
        .unwrap();

        assert_eq!(parsed.view.dialect, Dialect::Dash);
        assert_eq!(parsed.view.mode, ParseMode::PermissiveRecovered);
        assert_eq!(parsed.view.strategy, ParseStrategy::Permissive);
        assert_eq!(parsed.grammar(), Grammar::Bash);
        assert!(parsed.is_permissive());
        assert!(parsed.is_recovered());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.script.commands.len(), 2);
    }

    #[test]
    fn collects_shuck_suppression_directives() {
        let directives = collect_directives(
            "# shuck: disable=sh001, SH-002, SH001 # reason\n# shuck: enable=SH003\n# shuck: disable-file=SH-004\n",
        );

        assert_eq!(directives.len(), 3);

        let Directive::Suppression(first) = &directives[0] else {
            panic!("expected first directive to parse");
        };
        assert_eq!(first.source, DirectiveSource::Shuck);
        assert_eq!(first.action, SuppressionAction::Disable);
        assert_eq!(
            first.codes,
            vec!["SH-001".to_string(), "SH-002".to_string()]
        );
        assert_eq!(first.placement, CommentPlacement::OwnLine);
        assert_eq!(first.span.start.line, 1);
        assert_eq!(first.span.start.column, 1);

        let Directive::Suppression(second) = &directives[1] else {
            panic!("expected second directive to parse");
        };
        assert_eq!(second.source, DirectiveSource::Shuck);
        assert_eq!(second.action, SuppressionAction::Enable);
        assert_eq!(second.codes, vec!["SH-003".to_string()]);
        assert_eq!(second.placement, CommentPlacement::OwnLine);
        assert_eq!(second.span.start.line, 2);

        let Directive::Suppression(third) = &directives[2] else {
            panic!("expected third directive to parse");
        };
        assert_eq!(third.source, DirectiveSource::Shuck);
        assert_eq!(third.action, SuppressionAction::DisableFile);
        assert_eq!(third.codes, vec!["SH-004".to_string()]);
        assert_eq!(third.placement, CommentPlacement::OwnLine);
        assert_eq!(third.span.start.line, 3);
    }

    #[test]
    fn collects_shellcheck_disable_aliases() {
        let directives =
            collect_directives("# shellcheck disable=SC2086,2034 disable=sc1090 # reason\n");

        assert_eq!(directives.len(), 1);
        let Directive::Suppression(directive) = &directives[0] else {
            panic!("expected shellcheck directive to parse");
        };
        assert_eq!(directive.source, DirectiveSource::ShellCheck);
        assert_eq!(directive.action, SuppressionAction::Disable);
        assert_eq!(
            directive.codes,
            vec![
                "SC1090".to_string(),
                "SC2034".to_string(),
                "SC2086".to_string()
            ]
        );
        assert_eq!(directive.placement, CommentPlacement::OwnLine);
        assert_eq!(directive.span.start.line, 1);
    }

    #[test]
    fn marks_malformed_directives() {
        let directives =
            collect_directives("# shuck: disable\n# shuck: bogus=SH-001\n# shuck: disable=oops\n");

        assert_eq!(directives.len(), 3);

        let Directive::Malformed(first) = &directives[0] else {
            panic!("expected malformed directive");
        };
        assert_eq!(first.source, DirectiveSource::Shuck);
        assert_eq!(
            first.reason,
            MalformedDirectiveReason::MissingActionAssignment
        );
        assert_eq!(first.placement, CommentPlacement::OwnLine);
        assert_eq!(first.span.start.line, 1);

        let Directive::Malformed(second) = &directives[1] else {
            panic!("expected malformed directive");
        };
        assert_eq!(second.source, DirectiveSource::Shuck);
        assert_eq!(second.reason, MalformedDirectiveReason::UnsupportedAction);
        assert_eq!(second.placement, CommentPlacement::OwnLine);
        assert_eq!(second.span.start.line, 2);

        let Directive::Malformed(third) = &directives[2] else {
            panic!("expected malformed directive");
        };
        assert_eq!(third.source, DirectiveSource::Shuck);
        assert_eq!(third.reason, MalformedDirectiveReason::InvalidCodes);
        assert_eq!(third.placement, CommentPlacement::OwnLine);
        assert_eq!(third.span.start.line, 3);
    }

    #[test]
    fn marks_trailing_shellcheck_disable_as_malformed() {
        let directives = collect_directives("echo hi # shellcheck disable=SC2086\n");

        assert_eq!(directives.len(), 1);
        let Directive::Malformed(directive) = &directives[0] else {
            panic!("expected malformed shellcheck directive");
        };
        assert_eq!(directive.source, DirectiveSource::ShellCheck);
        assert_eq!(
            directive.reason,
            MalformedDirectiveReason::TrailingShellCheckDirective
        );
        assert_eq!(directive.placement, CommentPlacement::Trailing);
        assert_eq!(directive.span.start.line, 1);
        assert_eq!(directive.span.start.column, 9);
    }

    #[test]
    fn ignores_non_disable_shellcheck_directives_for_now() {
        let directives = collect_directives("# shellcheck source=path/to/file.sh\n");

        assert!(directives.is_empty());
    }

    #[test]
    fn parse_rejects_unsupported_native_dialects() {
        let error = parse(
            "echo hi",
            ParseOptions {
                dialect: Dialect::Dash,
                ..ParseOptions::default()
            },
        )
        .unwrap_err();

        assert_eq!(
            error,
            ParseError::UnsupportedParse {
                dialect: Dialect::Dash,
                mode: ParseMode::Strict
            }
        );
    }

    #[test]
    fn parse_rejects_unsupported_parse_views() {
        let error = parse(
            "echo hi",
            ParseOptions {
                dialect: Dialect::Zsh,
                mode: ParseMode::Permissive,
                ..ParseOptions::default()
            },
        )
        .unwrap_err();

        assert_eq!(
            error,
            ParseError::UnsupportedParse {
                dialect: Dialect::Zsh,
                mode: ParseMode::Permissive
            }
        );
    }
}
