use shuck_ast::{File, Position, Span};
use shuck_parser::parser::{ParseDiagnostic, ParseResult};
use std::collections::VecDeque;

use crate::Locator;

use crate::rules::correctness::c_prototype_fragment::CPrototypeFragment;
use crate::rules::correctness::dangling_else::DanglingElse;
use crate::rules::correctness::if_bracket_glued::IfBracketGlued;
use crate::rules::correctness::if_missing_then::IfMissingThen;
use crate::rules::correctness::loop_without_end::LoopWithoutEnd;
use crate::rules::correctness::missing_done_in_for_loop::MissingDoneInForLoop;
use crate::rules::correctness::missing_fi::MissingFi;
use crate::rules::correctness::stray_closing_keyword::StrayClosingKeyword;
use crate::rules::correctness::unterminated_if::UnterminatedIf;
use crate::rules::correctness::until_missing_do::UntilMissingDo;
use crate::rules::portability::function_params_in_sh::FunctionParamsInSh;
use crate::rules::portability::targets_non_zsh_shell;
use crate::rules::portability::zsh_always_block::ZshAlwaysBlock;
use crate::rules::portability::zsh_brace_if::ZshBraceIf;
use crate::rules::style::linebreak_before_and::LinebreakBeforeAnd;
use crate::{
    Diagnostic, Edit, Fix, FixAvailability, LinterSemanticArtifacts, Rule, RuleSet, ShellDialect,
    Violation,
};

pub struct ExtglobCase;
pub struct ExtglobInCasePattern;

impl Violation for ExtglobCase {
    fn rule() -> Rule {
        Rule::ExtglobCase
    }

    fn message(&self) -> String {
        "grouped case patterns are not portable to POSIX sh".to_owned()
    }
}

impl Violation for ExtglobInCasePattern {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::ExtglobInCasePattern
    }

    fn message(&self) -> String {
        "extended glob alternation in a case pattern is not portable to POSIX sh".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("expand the case pattern alternatives".to_owned())
    }
}

pub(crate) fn collect_parse_rule_diagnostics(
    file: &File,
    locator: Locator<'_>,
    parse_result: Option<&ParseResult>,
    semantic: &LinterSemanticArtifacts<'_>,
    enabled_rules: &RuleSet,
    shell: ShellDialect,
) -> Vec<Diagnostic> {
    let source = locator.source();
    let mut diagnostics = Vec::new();
    let parse_diagnostics = parse_result
        .map(|result| result.diagnostics.as_slice())
        .unwrap_or(&[]);
    if parse_diagnostics.is_empty()
        && !targets_non_zsh_shell(shell)
        && !is_x037_shell(shell)
        && !is_x048_shell(shell)
    {
        return diagnostics;
    }
    let missing_done_loop_kind = (enabled_rules.contains(crate::Rule::LoopWithoutEnd)
        || enabled_rules.contains(crate::Rule::MissingDoneInForLoop))
    .then(|| missing_done_loop_kind(file, source, parse_diagnostics, semantic))
    .flatten();

    if enabled_rules.contains(crate::Rule::MissingFi)
        && parse_diagnostics
            .iter()
            .any(|diagnostic| is_missing_fi_error(&diagnostic.message))
    {
        diagnostics
            .push(Diagnostic::new(MissingFi, eof_point(file)).with_fix(missing_fi_fix(source)));
    }
    if enabled_rules.contains(crate::Rule::UnterminatedIf)
        && is_c034_shell(shell)
        && let Some(span) = unterminated_if_span(locator, source, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(UnterminatedIf, span));
    }
    if enabled_rules.contains(crate::Rule::IfMissingThen)
        && let Some(span) = if_missing_then_span(locator, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(IfMissingThen, span));
    }
    if enabled_rules.contains(crate::Rule::LoopWithoutEnd)
        && missing_done_loop_kind == Some(MissingDoneLoopKind::NonFor)
    {
        diagnostics.push(
            Diagnostic::new(LoopWithoutEnd, eof_point(file)).with_fix(missing_done_fix(source)),
        );
    }
    if enabled_rules.contains(crate::Rule::MissingDoneInForLoop)
        && missing_done_loop_kind == Some(MissingDoneLoopKind::For)
    {
        diagnostics.push(
            Diagnostic::new(MissingDoneInForLoop, eof_point(file))
                .with_fix(missing_done_fix(source)),
        );
    }
    if enabled_rules.contains(crate::Rule::DanglingElse)
        && let Some(span) = dangling_else_span(parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(DanglingElse, span));
    }
    if enabled_rules.contains(crate::Rule::UntilMissingDo)
        && let Some(span) = until_missing_do_span(locator, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(UntilMissingDo, span));
    }
    if enabled_rules.contains(crate::Rule::IfBracketGlued)
        && let Some(span) = if_bracket_glued_span(locator, parse_diagnostics)
    {
        diagnostics.push(
            Diagnostic::new(IfBracketGlued, span)
                .with_fix(Fix::safe_edit(Edit::insertion(span.start.offset + 2, " "))),
        );
    }
    if enabled_rules.contains(crate::Rule::LinebreakBeforeAnd) {
        for span in linebreak_before_and_spans(locator, parse_diagnostics) {
            let diagnostic = Diagnostic::new(LinebreakBeforeAnd, span);
            diagnostics.push(match linebreak_before_and_fix(locator, span) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            });
        }
    }
    if enabled_rules.contains(crate::Rule::StrayClosingKeyword) && is_c016_shell(shell) {
        for span in stray_closing_keyword_spans(locator, parse_diagnostics) {
            diagnostics.push(Diagnostic::new(StrayClosingKeyword, span));
        }
    }
    if enabled_rules.contains(crate::Rule::FunctionParamsInSh)
        && matches!(
            shell,
            ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
        )
    {
        for diagnostic in parse_diagnostics {
            if let Some(span) = function_parameter_syntax_span(locator, diagnostic) {
                diagnostics.push(Diagnostic::new(FunctionParamsInSh, span));
            }
        }
    }

    if enabled_rules.contains(crate::Rule::CPrototypeFragment) {
        for diagnostic in parse_diagnostics {
            let Some(span) = c_prototype_fragment_span(diagnostic, locator) else {
                continue;
            };
            diagnostics.push(
                Diagnostic::new(CPrototypeFragment, span)
                    .with_fix(Fix::safe_edit(Edit::insertion(span.start.offset + 1, " "))),
            );
        }
    }

    if enabled_rules.contains(crate::Rule::ZshBraceIf) && targets_non_zsh_shell(shell) {
        for span in parse_result
            .map(|result| result.syntax_facts.zsh_brace_if_spans.as_slice())
            .unwrap_or(&[])
        {
            diagnostics.push(Diagnostic::new(ZshBraceIf, *span));
        }
    }
    if enabled_rules.contains(crate::Rule::ZshAlwaysBlock) && targets_non_zsh_shell(shell) {
        for span in parse_result
            .map(|result| result.syntax_facts.zsh_always_spans.as_slice())
            .unwrap_or(&[])
        {
            diagnostics.push(Diagnostic::new(ZshAlwaysBlock, *span));
        }
    }
    if enabled_rules.contains(crate::Rule::ExtglobCase) && is_x037_shell(shell) {
        for part in parse_result
            .map(|result| result.syntax_facts.zsh_case_group_parts.as_slice())
            .unwrap_or(&[])
        {
            if part.pattern_part_index == 0 {
                diagnostics.push(Diagnostic::new(ExtglobCase, part.span));
            }
        }
    }
    if enabled_rules.contains(crate::Rule::ExtglobInCasePattern) && is_x048_shell(shell) {
        for part in parse_result
            .map(|result| result.syntax_facts.zsh_case_group_parts.as_slice())
            .unwrap_or(&[])
        {
            if part.pattern_part_index > 0 {
                let diagnostic = Diagnostic::new(ExtglobInCasePattern, part.span);
                diagnostics.push(match extglob_in_case_pattern_fix(source, part.span) {
                    Some(fix) => diagnostic.with_fix(fix),
                    None => diagnostic,
                });
            }
        }
    }

    diagnostics
}

fn extglob_in_case_pattern_fix(source: &str, part_span: Span) -> Option<Fix> {
    let group = part_span.slice(source);
    group.strip_prefix('(')?.strip_suffix(')')?;
    let (pattern_span, pattern_text) = simple_case_pattern_surface(source, part_span)?;
    let alternatives = expand_case_pattern_surface_alternatives(pattern_text)?;
    let replacement = alternatives.join("|");

    Some(Fix::safe_edit(Edit::replacement(replacement, pattern_span)))
}

fn simple_case_pattern_surface(source: &str, part_span: Span) -> Option<(Span, &str)> {
    let line_start = source[..part_span.start.offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let pattern_start = simple_case_pattern_start(source, line_start, part_span.start.offset)?;
    let prefix = &source[pattern_start..part_span.start.offset];
    if prefix.is_empty() || prefix.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return None;
    }

    let pattern_end = simple_case_pattern_end(source, part_span.end.offset)?;
    let pattern_text = &source[pattern_start..pattern_end];
    if pattern_text.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return None;
    }

    let start = Position {
        line: part_span.start.line,
        column: part_span
            .start
            .column
            .saturating_sub(part_span.start.offset - pattern_start),
        offset: pattern_start,
    };
    let end = part_span
        .end
        .advanced_by(&source[part_span.end.offset..pattern_end]);

    Some((Span::from_positions(start, end), pattern_text))
}

fn simple_case_pattern_start(source: &str, line_start: usize, group_start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = group_start;
    let mut depth = 0usize;
    let mut escaped = false;
    let mut in_bracket = false;

    while index > line_start {
        index -= 1;
        let byte = bytes[index];
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if in_bracket {
            in_bracket = byte != b'[';
            continue;
        }
        match byte {
            b']' => in_bracket = true,
            b')' => depth += 1,
            b'(' => {
                depth = depth.saturating_sub(1);
            }
            b'|' if depth == 0 => return Some(index + 1),
            _ => {}
        }
    }

    let leading_blanks = source[line_start..group_start]
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    Some(line_start + leading_blanks)
}

fn simple_case_pattern_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut escaped = false;
    let mut in_bracket = false;
    let mut depth = 0usize;

    while let Some(byte) = bytes.get(index).copied() {
        if byte == b'\n' {
            return None;
        }
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if in_bracket {
            in_bracket = byte != b']';
            index += 1;
            continue;
        }
        match byte {
            b'[' => in_bracket = true,
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            b'|' if depth == 0 => return Some(index),
            b')' => return Some(index),
            _ => {}
        }
        index += 1;
    }

    None
}

fn expand_case_pattern_surface_alternatives(text: &str) -> Option<Vec<String>> {
    let mut expanded = vec![String::new()];
    let mut index = 0usize;

    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with('\\') {
            let escaped = rest.chars().take(2).collect::<String>();
            append_to_all(&mut expanded, &escaped);
            index += escaped.len();
            continue;
        }
        if rest.starts_with('(')
            && let Some(close) = matching_group_close(text, index)
            && let Some(group_alternatives) = split_case_group_alternatives(&text[index + 1..close])
        {
            let part_alternatives = group_alternatives
                .iter()
                .map(|alternative| expand_case_pattern_surface_alternatives(alternative))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            expanded = combine_alternatives(&expanded, &part_alternatives);
            index = close + 1;
            continue;
        }

        let ch = rest.chars().next()?;
        append_to_all(&mut expanded, &rest[..ch.len_utf8()]);
        index += ch.len_utf8();
    }

    Some(expanded)
}

fn append_to_all(alternatives: &mut [String], suffix: &str) {
    for alternative in alternatives {
        alternative.push_str(suffix);
    }
}

fn combine_alternatives(prefixes: &[String], suffixes: &[String]) -> Vec<String> {
    let mut combined = Vec::with_capacity(prefixes.len() * suffixes.len());
    for prefix in prefixes {
        for suffix in suffixes {
            let mut alternative = String::with_capacity(prefix.len() + suffix.len());
            alternative.push_str(prefix);
            alternative.push_str(suffix);
            combined.push(alternative);
        }
    }
    combined
}

fn matching_group_close(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut escaped = false;
    let mut in_bracket = false;
    for (relative, ch) in text[open..].char_indices() {
        let index = open + relative;
        if escaped {
            escaped = false;
            continue;
        }
        if in_bracket {
            in_bracket = ch != ']';
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => in_bracket = true,
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_case_group_alternatives(text: &str) -> Option<Vec<&str>> {
    let mut alternatives = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut escaped = false;
    let mut in_bracket = false;

    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_bracket {
            in_bracket = ch != ']';
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => in_bracket = true,
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            '|' if depth == 0 => {
                alternatives.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    alternatives.push(&text[start..]);

    (alternatives.len() > 1
        && alternatives
            .iter()
            .all(|alternative| !alternative.is_empty()))
    .then_some(alternatives)
}

fn is_missing_fi_error(message: &str) -> bool {
    message.starts_with("expected 'fi'")
}

fn unterminated_if_span(
    locator: Locator<'_>,
    source: &str,
    parse_diagnostics: &[ParseDiagnostic],
) -> Option<Span> {
    if !parse_diagnostics
        .iter()
        .any(|diagnostic| is_missing_fi_error(&diagnostic.message))
    {
        return None;
    }

    let (_line, _column, offset) = unterminated_if_position_from_source(source)?;
    let start = locator.position_at_offset(offset)?;
    Some(Span::from_positions(start, start))
}

#[derive(Debug, Clone, Copy)]
struct IfFrame {
    line: usize,
    column: usize,
    offset: usize,
    has_then: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellMultilineState {
    Code,
    SingleQuote,
    DoubleQuote,
    Backtick,
    DollarExpansion(DollarExpansionState),
}

impl ShellMultilineState {
    fn is_code(self) -> bool {
        self == Self::Code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DollarExpansionState {
    open: u8,
    close: u8,
    depth: usize,
}

fn unterminated_if_position_from_source(source: &str) -> Option<(usize, usize, usize)> {
    let mut stack = Vec::new();
    let mut continued_line = String::new();
    let mut continued_line_offsets = Vec::new();
    let mut logical_start_line = None;
    let mut logical_start_offset = None;
    let mut pending_heredocs = VecDeque::<PendingHeredoc>::new();
    let mut lexical_state = ShellMultilineState::Code;
    let mut source_offset = 0usize;

    for (index, raw_line) in source.split_inclusive('\n').enumerate() {
        let line_number = index + 1;
        let line_start_offset = source_offset;
        source_offset += raw_line.len();
        let line = raw_line
            .strip_suffix('\n')
            .unwrap_or(raw_line)
            .strip_suffix('\r')
            .unwrap_or_else(|| raw_line.strip_suffix('\n').unwrap_or(raw_line));
        if let Some(pending) = pending_heredocs.front() {
            if line_matches_heredoc_delimiter(line, pending) {
                pending_heredocs.pop_front();
            }
            continue;
        }

        let trimmed_end = line.trim_end();
        let has_continuation = raw_line.ends_with('\n') && line_ends_with_escaped_newline(line);

        if has_continuation || !continued_line.is_empty() {
            if !continued_line.is_empty() {
                push_logical_line_segment(
                    &mut continued_line,
                    &mut continued_line_offsets,
                    trimmed_end,
                    line_start_offset,
                );
            } else {
                logical_start_line = Some(line_number);
                logical_start_offset = Some(line_start_offset);
                push_logical_line_segment(
                    &mut continued_line,
                    &mut continued_line_offsets,
                    trimmed_end,
                    line_start_offset,
                );
            }

            if has_continuation {
                continued_line.pop();
                continued_line_offsets.pop();
                continue;
            }

            let logical_line = continued_line.trim_end();
            scan_if_logical_line(
                &mut stack,
                &mut pending_heredocs,
                &mut lexical_state,
                logical_line,
                logical_start_line.unwrap_or(line_number),
                logical_start_offset.unwrap_or(line_start_offset),
                Some(&continued_line_offsets[..logical_line.len()]),
            );
            continued_line.clear();
            continued_line_offsets.clear();
            logical_start_line = None;
            logical_start_offset = None;
        } else {
            scan_if_logical_line(
                &mut stack,
                &mut pending_heredocs,
                &mut lexical_state,
                trimmed_end,
                line_number,
                line_start_offset,
                None,
            );
        }
    }

    if !continued_line.is_empty() {
        let logical_line = continued_line.trim_end();
        scan_if_logical_line(
            &mut stack,
            &mut pending_heredocs,
            &mut lexical_state,
            logical_line,
            logical_start_line.unwrap_or_else(|| source.lines().count().max(1)),
            logical_start_offset.unwrap_or(source.len().saturating_sub(logical_line.len())),
            Some(&continued_line_offsets[..logical_line.len()]),
        );
    }

    let frame = stack.iter().rev().find(|frame| frame.has_then).copied()?;
    Some((frame.line, frame.column, frame.offset))
}

fn push_logical_line_segment(
    logical_line: &mut String,
    logical_line_offsets: &mut Vec<usize>,
    segment: &str,
    segment_start_offset: usize,
) {
    logical_line.push_str(segment);
    logical_line_offsets.extend((0..segment.len()).map(|offset| segment_start_offset + offset));
}

fn scan_if_logical_line(
    stack: &mut Vec<IfFrame>,
    pending_heredocs: &mut VecDeque<PendingHeredoc>,
    lexical_state: &mut ShellMultilineState,
    logical_line: &str,
    line_number: usize,
    line_start_offset: usize,
    source_offsets: Option<&[usize]>,
) {
    let starts_in_code = lexical_state.is_code();
    if starts_in_code && !line_has_multiline_sensitive_start(logical_line) {
        update_if_stack(
            stack,
            logical_line,
            logical_line,
            line_number,
            line_start_offset,
            source_offsets,
        );
        pending_heredocs.extend(heredoc_delimiters_in_line(logical_line));
        return;
    }

    let masked_line = mask_non_code_text(lexical_state, logical_line);
    update_if_stack(
        stack,
        &masked_line,
        logical_line,
        line_number,
        line_start_offset,
        source_offsets,
    );
    pending_heredocs.extend(heredoc_delimiters_in_scanned_code_line(
        &masked_line,
        logical_line,
    ));
}

fn line_has_multiline_sensitive_start(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        matches!(*byte, b'\'' | b'"' | b'`')
            || (*byte == b'$' && matches!(bytes.get(index + 1), Some(b'{' | b'(' | b'[')))
    })
}

fn mask_non_code_text(state: &mut ShellMultilineState, line: &str) -> String {
    let bytes = line.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0usize;
    let mut comment_can_start = true;

    while index < bytes.len() {
        match *state {
            ShellMultilineState::Code => match bytes[index] {
                b'#' if comment_can_start => break,
                b'\'' => {
                    let (end, closed) = single_quote_end(bytes, index + 1);
                    mask_span(&mut masked, index, end);
                    *state = if closed {
                        ShellMultilineState::Code
                    } else {
                        ShellMultilineState::SingleQuote
                    };
                    comment_can_start = false;
                    index = end;
                }
                b'"' => {
                    let (end, closed) = double_quote_end(bytes, index + 1);
                    mask_span(&mut masked, index, end);
                    *state = if closed {
                        ShellMultilineState::Code
                    } else {
                        ShellMultilineState::DoubleQuote
                    };
                    comment_can_start = false;
                    index = end;
                }
                b'`' => {
                    masked[index] = b'(';
                    *state = ShellMultilineState::Backtick;
                    comment_can_start = true;
                    index += 1;
                }
                b'$' => {
                    if starts_command_substitution(bytes, index) {
                        index += 2;
                    } else if let Some((expansion, content_start)) =
                        dollar_expansion_start(bytes, index)
                    {
                        let (end, next_state) = mask_dollar_expansion(
                            &mut masked,
                            bytes,
                            index,
                            content_start,
                            expansion,
                        );
                        *state = next_state
                            .map(ShellMultilineState::DollarExpansion)
                            .unwrap_or(ShellMultilineState::Code);
                        index = end;
                    } else {
                        index += 1;
                    }
                    comment_can_start = false;
                }
                b'\\' => {
                    index = (index + 2).min(bytes.len());
                    comment_can_start = false;
                }
                byte if byte.is_ascii_whitespace() || is_shell_word_separator(byte as char) => {
                    comment_can_start = true;
                    index += 1;
                }
                _ => {
                    comment_can_start = false;
                    index += 1;
                }
            },
            ShellMultilineState::SingleQuote => {
                let (end, closed) = single_quote_end(bytes, index);
                mask_span(&mut masked, index, end);
                *state = if closed {
                    ShellMultilineState::Code
                } else {
                    ShellMultilineState::SingleQuote
                };
                comment_can_start = false;
                index = end;
            }
            ShellMultilineState::DoubleQuote => {
                let (end, closed) = double_quote_end(bytes, index);
                mask_span(&mut masked, index, end);
                *state = if closed {
                    ShellMultilineState::Code
                } else {
                    ShellMultilineState::DoubleQuote
                };
                comment_can_start = false;
                index = end;
            }
            ShellMultilineState::Backtick => match bytes[index] {
                b'`' => {
                    masked[index] = b')';
                    *state = ShellMultilineState::Code;
                    comment_can_start = false;
                    index += 1;
                }
                b'\\' => {
                    index = (index + 2).min(bytes.len());
                    comment_can_start = false;
                }
                byte if byte.is_ascii_whitespace() || is_shell_word_separator(byte as char) => {
                    comment_can_start = true;
                    index += 1;
                }
                _ => {
                    comment_can_start = false;
                    index += 1;
                }
            },
            ShellMultilineState::DollarExpansion(expansion) => {
                let (end, next_state) =
                    mask_dollar_expansion(&mut masked, bytes, index, index, expansion);
                *state = next_state
                    .map(ShellMultilineState::DollarExpansion)
                    .unwrap_or(ShellMultilineState::Code);
                comment_can_start = false;
                index = end;
            }
        }
    }

    // SAFETY: every byte copied from `line` is either left unchanged from valid UTF-8
    // or replaced with ASCII `x`, so the masked buffer remains valid UTF-8.
    unsafe { String::from_utf8_unchecked(masked) }
}

fn mask_span(masked: &mut [u8], start: usize, end: usize) {
    for byte in &mut masked[start..end] {
        *byte = b'x';
    }
}

fn dollar_expansion_start(bytes: &[u8], index: usize) -> Option<(DollarExpansionState, usize)> {
    let next = bytes.get(index + 1).copied()?;
    let (open, close) = match next {
        b'{' => (b'{', b'}'),
        b'(' if bytes.get(index + 2) == Some(&b'(') => (b'(', b')'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    Some((
        DollarExpansionState {
            open,
            close,
            depth: 1,
        },
        index + 2,
    ))
}

fn starts_command_substitution(bytes: &[u8], index: usize) -> bool {
    bytes.get(index + 1) == Some(&b'(') && bytes.get(index + 2) != Some(&b'(')
}

fn mask_dollar_expansion(
    masked: &mut [u8],
    bytes: &[u8],
    start: usize,
    mut index: usize,
    mut expansion: DollarExpansionState,
) -> (usize, Option<DollarExpansionState>) {
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted_text(bytes, index + 1),
            b'"' => index = skip_double_quoted_text(bytes, index + 1),
            b'`' => index = skip_backtick_text(bytes, index + 1),
            b'\\' => index = (index + 2).min(bytes.len()),
            byte if byte == expansion.open => {
                expansion.depth += 1;
                index += 1;
            }
            byte if byte == expansion.close => {
                expansion.depth -= 1;
                index += 1;
                if expansion.depth == 0 {
                    mask_span(masked, start, index);
                    return (index, None);
                }
            }
            _ => index += 1,
        }
    }

    mask_span(masked, start, bytes.len());
    (bytes.len(), Some(expansion))
}

fn single_quote_end(bytes: &[u8], mut index: usize) -> (usize, bool) {
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return (index + 1, true);
        }
        index += 1;
    }
    (bytes.len(), false)
}

fn double_quote_end(bytes: &[u8], mut index: usize) -> (usize, bool) {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            b'"' => return (index + 1, true),
            _ => index += 1,
        }
    }
    (bytes.len(), false)
}

fn line_ends_with_escaped_newline(line: &str) -> bool {
    line.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn line_matches_heredoc_delimiter(line: &str, pending: &PendingHeredoc) -> bool {
    let candidate = if pending.strip_tabs {
        line.trim_start_matches('\t')
    } else {
        line
    };
    candidate == pending.delimiter
}

fn heredoc_delimiters_in_line(line: &str) -> Vec<PendingHeredoc> {
    heredoc_delimiters_in_scanned_code_line(line, line)
}

fn heredoc_delimiters_in_scanned_code_line(
    scan_line: &str,
    delimiter_line: &str,
) -> Vec<PendingHeredoc> {
    let scan_line = shell_code_before_comment(scan_line);
    let bytes = scan_line.as_bytes();
    let mut delimiters = Vec::new();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted_text(bytes, index + 1),
            b'"' => index = skip_double_quoted_text(bytes, index + 1),
            b'`' => index = skip_backtick_text(bytes, index + 1),
            b'\\' => index = (index + 2).min(bytes.len()),
            b'$' => index = skip_dollar_expansion_text(bytes, index),
            b'<' if bytes[index + 1] == b'<' => {
                let mut cursor = index + 2;
                let strip_tabs = bytes.get(cursor) == Some(&b'-');
                if strip_tabs {
                    cursor += 1;
                }
                while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_whitespace()) {
                    cursor += 1;
                }

                if let Some((delimiter, end)) = parse_heredoc_delimiter(delimiter_line, cursor) {
                    delimiters.push(PendingHeredoc {
                        delimiter,
                        strip_tabs,
                    });
                    index = end;
                } else {
                    index = cursor;
                }
            }
            _ => index += 1,
        }
    }

    delimiters
}

fn parse_heredoc_delimiter(line: &str, start: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    if start >= bytes.len() {
        return None;
    }

    let mut delimiter = String::new();
    let mut index = start;
    while index < bytes.len() {
        let ch = line[index..].chars().next()?;
        if ch.is_ascii_whitespace() || is_shell_word_separator(ch) {
            break;
        }

        match ch {
            '\'' => {
                index += ch.len_utf8();
                while index < bytes.len() {
                    let quoted = line[index..].chars().next()?;
                    index += quoted.len_utf8();
                    if quoted == '\'' {
                        break;
                    }
                    delimiter.push(quoted);
                }
            }
            '"' => {
                index += ch.len_utf8();
                while index < bytes.len() {
                    let quoted = line[index..].chars().next()?;
                    index += quoted.len_utf8();
                    if quoted == '"' {
                        break;
                    }
                    if quoted == '\\'
                        && let Some(escaped) = line[index..].chars().next()
                    {
                        index += escaped.len_utf8();
                        delimiter.push(escaped);
                    } else {
                        delimiter.push(quoted);
                    }
                }
            }
            '\\' => {
                index += ch.len_utf8();
                if let Some(escaped) = line[index..].chars().next() {
                    index += escaped.len_utf8();
                    delimiter.push(escaped);
                }
            }
            _ => {
                index += ch.len_utf8();
                delimiter.push(ch);
            }
        }
    }

    (!delimiter.is_empty()).then_some((delimiter, index))
}

fn skip_single_quoted_text(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_double_quoted_text(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_backtick_text(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            b'`' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_dollar_expansion_text(bytes: &[u8], index: usize) -> usize {
    let Some(next) = bytes.get(index + 1).copied() else {
        return bytes.len();
    };

    match next {
        b'{' => skip_balanced_text(bytes, index + 2, b'{', b'}'),
        b'(' => skip_balanced_text(bytes, index + 2, b'(', b')'),
        b'[' => skip_balanced_text(bytes, index + 2, b'[', b']'),
        _ => index + 1,
    }
}

fn skip_balanced_text(bytes: &[u8], mut index: usize, open: u8, close: u8) -> usize {
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted_text(bytes, index + 1),
            b'"' => index = skip_double_quoted_text(bytes, index + 1),
            b'`' => index = skip_backtick_text(bytes, index + 1),
            b'\\' => index = (index + 2).min(bytes.len()),
            b'$' => index = skip_dollar_expansion_text(bytes, index),
            byte if byte == open => {
                depth += 1;
                index += 1;
            }
            byte if byte == close => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    bytes.len()
}

fn update_if_stack(
    stack: &mut Vec<IfFrame>,
    scan_line: &str,
    original_line: &str,
    line_number: usize,
    line_start_offset: usize,
    source_offsets: Option<&[usize]>,
) {
    for (word, start_index) in shell_like_command_word_positions(scan_line) {
        match word {
            "if" => stack.push(IfFrame {
                line: line_number,
                column: original_line[..start_index].chars().count() + 1,
                offset: source_offsets
                    .and_then(|offsets| offsets.get(start_index))
                    .copied()
                    .unwrap_or(line_start_offset + start_index),
                has_then: false,
            }),
            "then" => {
                if let Some(frame) = stack.last_mut() {
                    frame.has_then = true;
                }
            }
            "fi" => {
                stack.pop();
            }
            _ => {}
        }
    }
}

fn shell_like_command_word_positions(line: &str) -> impl Iterator<Item = (&str, usize)> {
    let line = shell_code_before_comment(line);
    ShellLikeCommandWordPositions {
        line,
        index: 0,
        at_command_start: true,
        after_time_prefix: false,
    }
}

fn missing_fi_fix(source: &str) -> Fix {
    let content = if source.is_empty() || source.ends_with('\n') {
        "fi\n"
    } else {
        "\nfi\n"
    };

    Fix::unsafe_edit(Edit::insertion(source.len(), content))
}

fn missing_done_fix(source: &str) -> Fix {
    let content = if source.is_empty() || source.ends_with('\n') {
        "done\n"
    } else {
        "\ndone\n"
    };

    Fix::unsafe_edit(Edit::insertion(source.len(), content))
}

fn is_missing_then_error(message: &str) -> bool {
    message.starts_with("expected 'then'")
}

fn if_missing_then_span(
    locator: Locator<'_>,
    parse_diagnostics: &[ParseDiagnostic],
) -> Option<Span> {
    parse_diagnostics.iter().find_map(|diagnostic| {
        if !is_missing_then_error(&diagnostic.message)
            && !is_expected_command_error(&diagnostic.message)
        {
            return None;
        }

        let mut line = diagnostic.span.start.line;
        let mut saw_then = false;
        while line > 0 {
            let text = line_text_at(locator, line)?;
            let trimmed = text.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                line = line.saturating_sub(1);
                continue;
            }

            if trimmed == "fi" || trimmed == "else" {
                line = line.saturating_sub(1);
                continue;
            }
            if line_contains_shell_word(trimmed, "then") {
                saw_then = true;
                line = line.saturating_sub(1);
                continue;
            }
            if trimmed == "if" || trimmed.starts_with("if ") || trimmed.starts_with("if\t") {
                if saw_then {
                    return None;
                }
                let offset = line_start_offset(locator, line)?;
                let start = locator.position_at_offset(offset)?;
                return Some(Span::from_positions(start, start));
            }
            if trimmed == "elif" || trimmed.starts_with("elif ") || trimmed.starts_with("elif\t") {
                return None;
            }

            line = line.saturating_sub(1);
        }

        None
    })
}

fn line_contains_shell_word(line: &str, word: &str) -> bool {
    let bytes = line.as_bytes();
    let mut token = String::new();
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut double_quote_escape = false;
    let mut in_backticks = false;
    let mut parameter_expansion_depth = 0usize;
    let mut command_substitution_depth = 0usize;

    while index < bytes.len() {
        let byte = bytes[index];

        if in_single_quotes {
            token.push('_');
            if byte == b'\'' {
                in_single_quotes = false;
            }
            index += 1;
            continue;
        }

        if in_double_quotes {
            token.push('_');
            if double_quote_escape {
                double_quote_escape = false;
            } else if byte == b'\\' {
                double_quote_escape = true;
            } else if byte == b'"' {
                in_double_quotes = false;
            }
            index += 1;
            continue;
        }

        if in_backticks {
            token.push('_');
            if byte == b'\\' && index + 1 < bytes.len() {
                token.push('_');
                index += 2;
                continue;
            }
            if byte == b'`' {
                in_backticks = false;
            }
            index += 1;
            continue;
        }

        if parameter_expansion_depth > 0 {
            token.push('_');
            if byte == b'$' && bytes.get(index + 1) == Some(&b'{') {
                token.push('_');
                parameter_expansion_depth += 1;
                index += 2;
                continue;
            }
            if byte == b'}' {
                parameter_expansion_depth -= 1;
            }
            index += 1;
            continue;
        }

        if command_substitution_depth > 0 {
            token.push('_');
            if byte == b'$' && bytes.get(index + 1) == Some(&b'(') {
                token.push('_');
                command_substitution_depth += 1;
                index += 2;
                continue;
            }
            if byte == b')' {
                command_substitution_depth -= 1;
            }
            index += 1;
            continue;
        }

        match byte {
            b'\'' => {
                token.push('_');
                in_single_quotes = true;
            }
            b'"' => {
                token.push('_');
                in_double_quotes = true;
            }
            b'`' => {
                token.push('_');
                in_backticks = true;
            }
            b'\\' => {
                token.push('_');
                if index + 1 < bytes.len() {
                    token.push('_');
                    index += 1;
                }
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                token.push('_');
                token.push('_');
                parameter_expansion_depth = 1;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                token.push('_');
                token.push('_');
                command_substitution_depth = 1;
                index += 1;
            }
            b'#' if token.is_empty() => break,
            byte if byte.is_ascii_whitespace() || is_shell_word_separator(byte as char) => {
                if token == word {
                    return true;
                }
                token.clear();
            }
            _ => token.push(char::from(byte)),
        }

        index += 1;
    }

    token == word
}

fn is_shell_word_separator(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '{' | '}' | '<' | '>')
}

fn is_loop_without_end_error(message: &str) -> bool {
    message.starts_with("expected 'done'")
}

fn is_dangling_else_error(message: &str) -> bool {
    message.starts_with("syntax error: empty else clause")
}

fn is_expected_command_error(message: &str) -> bool {
    message.starts_with("expected command")
}

fn stray_closing_keyword_spans(
    locator: Locator<'_>,
    parse_diagnostics: &[ParseDiagnostic],
) -> Vec<Span> {
    let source = locator.source();
    parse_diagnostics
        .iter()
        .filter_map(|diagnostic| {
            if !is_expected_command_error(&diagnostic.message) {
                return None;
            }

            let keyword = diagnostic.span.slice(source);
            if !matches!(
                keyword,
                "then" | "else" | "elif" | "fi" | "do" | "done" | "esac"
            ) {
                return None;
            }

            if is_more_specific_parse_diagnostic(locator, diagnostic) {
                return None;
            }

            Some(diagnostic.span)
        })
        .collect()
}

fn is_more_specific_parse_diagnostic(locator: Locator<'_>, diagnostic: &ParseDiagnostic) -> bool {
    let diagnostic = std::slice::from_ref(diagnostic);
    let keyword = diagnostic[0].span.slice(locator.source());
    (matches!(keyword, "fi" | "else" | "elif")
        && if_missing_then_span(locator, diagnostic).is_some())
        || until_missing_do_span(locator, diagnostic).is_some()
}

fn is_function_parameter_syntax_error(message: &str) -> bool {
    message.starts_with("expected ')' in function definition")
}

fn function_parameter_syntax_span(
    locator: Locator<'_>,
    diagnostic: &ParseDiagnostic,
) -> Option<Span> {
    if !is_function_parameter_syntax_error(&diagnostic.message) {
        return None;
    }

    let line = line_text_at(locator, diagnostic.span.start.line)?;
    let search_end = line
        .char_indices()
        .nth(diagnostic.span.start.column.saturating_sub(1))
        .map_or(line.len(), |(index, ch)| index + ch.len_utf8());
    let paren_index = line
        .get(..search_end)
        .and_then(|prefix| prefix.rfind('('))
        .or_else(|| {
            line.get(search_end..)
                .and_then(|suffix| suffix.find('(').map(|relative| search_end + relative))
        })?;
    let line_start = line_start_offset(locator, diagnostic.span.start.line)?;
    let start_offset = line_start + paren_index;
    let start = locator.position_at_offset(start_offset)?;
    let end = locator.position_at_offset(start_offset + 1)?;
    Some(Span::from_positions(start, end))
}

fn linebreak_before_and_spans(
    locator: Locator<'_>,
    parse_diagnostics: &[ParseDiagnostic],
) -> Vec<Span> {
    parse_diagnostics
        .iter()
        .filter_map(|diagnostic| {
            if !is_expected_command_error(&diagnostic.message) {
                return None;
            }

            leading_control_operator_span(locator, diagnostic.span.start.line)
        })
        .collect()
}

fn leading_control_operator_span(locator: Locator<'_>, line_number: usize) -> Option<Span> {
    let line = line_text_at(locator, line_number)?;
    let text = line.split_once('#').map_or(line, |(before, _)| before);
    let leading_bytes = text
        .bytes()
        .take_while(|byte| matches!(*byte, b' ' | b'\t' | b'\r'))
        .count();
    let trimmed = &text[leading_bytes..];

    let operator = if let Some(rest) = trimmed.strip_prefix("&&") {
        if rest
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_ascii_whitespace())
        {
            return None;
        }
        "&&"
    } else if let Some(rest) = trimmed.strip_prefix("||") {
        if rest
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_ascii_whitespace())
        {
            return None;
        }
        "||"
    } else if let Some(rest) = trimmed.strip_prefix('|') {
        if rest
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_ascii_whitespace())
        {
            return None;
        }
        "|"
    } else {
        return None;
    };

    let line_start = line_start_offset(locator, line_number)?;
    let start_offset = line_start + leading_bytes;
    let start = locator.position_at_offset(start_offset)?;
    let end = locator.position_at_offset(start_offset + operator.len())?;
    Some(Span::from_positions(start, end))
}

fn linebreak_before_and_fix(locator: Locator<'_>, span: Span) -> Option<Fix> {
    let source = locator.source();
    let line_start = line_start_offset(locator, span.start.line)?;
    let insert_offset = line_start.checked_sub(1)?;
    if source.as_bytes().get(insert_offset) != Some(&b'\n') {
        return None;
    }
    if previous_line_has_shell_comment(source, insert_offset) {
        return None;
    }

    let operator = span.slice(source);
    let rest = source.get(span.end.offset..)?;
    let trailing_ws_len = rest
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .map(char::len_utf8)
        .sum::<usize>();
    Some(Fix::safe_edits([
        Edit::insertion(insert_offset, format!(" {operator}")),
        Edit::deletion_at(span.start.offset, span.end.offset + trailing_ws_len),
    ]))
}

fn previous_line_has_shell_comment(source: &str, line_end: usize) -> bool {
    let line_start = source[..line_end]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    line_has_shell_comment(&source[line_start..line_end])
}

fn line_has_shell_comment(line: &str) -> bool {
    shell_code_before_comment(line).len() < line.len()
}

fn dangling_else_span(parse_diagnostics: &[ParseDiagnostic]) -> Option<Span> {
    parse_diagnostics
        .iter()
        .find(|diagnostic| is_dangling_else_error(&diagnostic.message))
        .map(|diagnostic| diagnostic.span)
}

fn until_missing_do_span(
    locator: Locator<'_>,
    parse_diagnostics: &[ParseDiagnostic],
) -> Option<Span> {
    let mut pending_until_flags: Option<Vec<bool>> = None;
    parse_diagnostics
        .iter()
        .find(|diagnostic| {
            is_expected_command_error(&diagnostic.message)
                && is_done_line(locator, diagnostic.span.start.line)
                && {
                    let flags = pending_until_flags
                        .get_or_insert_with(|| compute_pending_until_flags(locator.source()));
                    flags
                        .get(diagnostic.span.start.line.saturating_sub(1))
                        .copied()
                        .unwrap_or(false)
                }
        })
        .map(|diagnostic| diagnostic.span)
}

fn compute_pending_until_flags(source: &str) -> Vec<bool> {
    let mut depth: usize = 0;
    let mut flags: Vec<bool> = Vec::new();
    flags.push(false);
    for line in source.lines() {
        let text = line.split_once('#').map_or(line, |(before, _)| before);
        if line_has_command_leading_word(text, "until") {
            depth += 1;
        }
        if depth > 0
            && (line_has_command_leading_word(text, "do")
                || line_has_command_leading_word(text, "done"))
        {
            depth -= 1;
        }
        flags.push(depth > 0);
    }
    flags
}

fn if_bracket_glued_span(
    locator: Locator<'_>,
    parse_diagnostics: &[ParseDiagnostic],
) -> Option<Span> {
    parse_diagnostics.iter().find_map(|diagnostic| {
        if !is_expected_command_error(&diagnostic.message) {
            return None;
        }
        if_bracket_glued_span_on_line(locator, diagnostic.span.start.line)
    })
}

fn if_bracket_glued_span_on_line(locator: Locator<'_>, line_number: usize) -> Option<Span> {
    let line = line_text_at(locator, line_number)?;
    let line_offset = line_start_offset(locator, line_number)?;
    let bytes = line.as_bytes();
    if bytes.len() < 3 {
        return None;
    }

    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut double_quote_escape = false;
    let mut parameter_expansion_depth = 0usize;
    let mut index = 0usize;

    while index + 2 < bytes.len() {
        let byte = bytes[index];

        if in_single_quotes {
            if byte == b'\'' {
                in_single_quotes = false;
            }
            index += 1;
            continue;
        }

        if in_double_quotes {
            if double_quote_escape {
                double_quote_escape = false;
            } else if byte == b'\\' {
                double_quote_escape = true;
            } else if byte == b'"' {
                in_double_quotes = false;
            }
            index += 1;
            continue;
        }

        if parameter_expansion_depth > 0 {
            match byte {
                b'\\' => {
                    index += 2.min(bytes.len().saturating_sub(index));
                }
                b'$' if bytes.get(index + 1) == Some(&b'{') => {
                    parameter_expansion_depth += 1;
                    index += 2;
                }
                b'}' => {
                    parameter_expansion_depth -= 1;
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
            continue;
        }

        match byte {
            b'\'' => {
                in_single_quotes = true;
                index += 1;
                continue;
            }
            b'"' => {
                in_double_quotes = true;
                index += 1;
                continue;
            }
            b'\\' => {
                index += 2.min(bytes.len().saturating_sub(index));
                continue;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                parameter_expansion_depth = 1;
                index += 2;
                continue;
            }
            b'#' if shell_comment_starts_at(bytes, index) => break,
            b'i' if bytes[index + 1] == b'f' && bytes[index + 2] == b'[' => {
                if !shell_command_boundary_before(bytes, index) {
                    index += 1;
                    continue;
                }

                let start_offset = line_offset + index;
                let end_offset = start_offset + 3;
                let start = locator.position_at_offset(start_offset)?;
                let end = locator.position_at_offset(end_offset)?;
                return Some(Span::from_positions(start, end));
            }
            _ => {
                index += 1;
            }
        }
    }

    None
}

fn shell_command_boundary_before(bytes: &[u8], index: usize) -> bool {
    index == 0
        || matches!(
            bytes[index - 1],
            b' ' | b'\t' | b'\r' | b';' | b'|' | b'&' | b'(' | b')'
        )
}

fn shell_comment_starts_at(bytes: &[u8], index: usize) -> bool {
    index == 0
        || matches!(
            bytes[index - 1],
            b' ' | b'\t' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>'
        )
}

fn is_done_line(locator: Locator<'_>, line_number: usize) -> bool {
    line_text_at(locator, line_number)
        .map(|line| {
            let text = line.split_once('#').map_or(line, |(before, _)| before);
            shell_like_words(text).any(|word| word == "done")
        })
        .unwrap_or(false)
}

fn line_has_command_leading_word(line: &str, word: &str) -> bool {
    let word_bytes = word.as_bytes();
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut at_segment_start = true;
    while i < bytes.len() {
        let b = bytes[i];
        if matches!(b, b';' | b'|' | b'&') {
            at_segment_start = true;
            i += 1;
            continue;
        }
        if at_segment_start && (b.is_ascii_alphanumeric() || b == b'_') {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            at_segment_start = false;
            if &bytes[start..i] == word_bytes {
                return true;
            }
            continue;
        }
        i += 1;
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingDoneLoopKind {
    For,
    NonFor,
}

fn missing_done_loop_kind(
    file: &File,
    source: &str,
    parse_diagnostics: &[ParseDiagnostic],
    semantic: &LinterSemanticArtifacts<'_>,
) -> Option<MissingDoneLoopKind> {
    if !parse_diagnostics
        .iter()
        .any(|diagnostic| is_loop_without_end_error(&diagnostic.message))
    {
        return None;
    }

    Some(
        if missing_done_belongs_to_for_loop(file, source, semantic) {
            MissingDoneLoopKind::For
        } else {
            MissingDoneLoopKind::NonFor
        },
    )
}

fn missing_done_belongs_to_for_loop(
    file: &File,
    source: &str,
    semantic: &LinterSemanticArtifacts<'_>,
) -> bool {
    semantic
        .missing_done_trailing_loop_is_for(&file.body, file.span.end.offset)
        .or_else(|| missing_done_loop_kind_from_source(source))
        .unwrap_or(false)
}

fn missing_done_loop_kind_from_source(source: &str) -> Option<bool> {
    let mut loop_stack = Vec::new();
    let mut continued_line = String::new();

    for line in source.lines() {
        let text = line.split_once('#').map_or(line, |(before, _)| before);
        let trimmed_end = text.trim_end();
        if !continued_line.is_empty() {
            continued_line.push(' ');
            continued_line.push_str(trimmed_end.trim_start());
        } else {
            continued_line.push_str(trimmed_end);
        }

        if continued_line.ends_with('\\') {
            continued_line.pop();
            continue;
        }

        let logical_line = continued_line.trim().to_owned();
        let words: Vec<&str> = shell_like_words(&logical_line).collect();
        continued_line.clear();
        if words.is_empty() {
            continue;
        }

        let has_do = words.contains(&"do");
        if has_do {
            if words.contains(&"for") {
                loop_stack.push(true);
            } else if words.iter().any(|word| matches!(*word, "while" | "until")) {
                loop_stack.push(false);
            }
        }

        let done_count = words.iter().filter(|word| **word == "done").count();
        for _ in 0..done_count {
            if loop_stack.pop().is_none() {
                break;
            }
        }
    }

    if !continued_line.is_empty() {
        let words: Vec<&str> = shell_like_words(continued_line.trim()).collect();
        if !words.is_empty() {
            let has_do = words.contains(&"do");
            if has_do {
                if words.contains(&"for") {
                    loop_stack.push(true);
                } else if words.iter().any(|word| matches!(*word, "while" | "until")) {
                    loop_stack.push(false);
                }
            }

            let done_count = words.iter().filter(|word| **word == "done").count();
            for _ in 0..done_count {
                if loop_stack.pop().is_none() {
                    break;
                }
            }
        }
    }

    loop_stack.last().copied()
}

fn shell_like_words(line: &str) -> impl Iterator<Item = &str> {
    shell_like_word_positions(line).map(|(word, _)| word)
}

fn shell_like_word_positions(line: &str) -> ShellLikeWordPositions<'_> {
    let line = shell_code_before_comment(line);
    ShellLikeWordPositions {
        line,
        iter: line.char_indices(),
        start: None,
        finished: false,
    }
}

fn shell_code_before_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    let mut comment_can_start = true;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut double_quote_escape = false;

    while index < bytes.len() {
        let byte = bytes[index];

        if in_single_quotes {
            if byte == b'\'' {
                in_single_quotes = false;
            }
            index += 1;
            continue;
        }

        if in_double_quotes {
            if double_quote_escape {
                double_quote_escape = false;
            } else if byte == b'\\' {
                double_quote_escape = true;
            } else if byte == b'"' {
                in_double_quotes = false;
            }
            index += 1;
            continue;
        }

        match byte {
            b'#' if comment_can_start => return &line[..index],
            b'\'' => {
                in_single_quotes = true;
                comment_can_start = false;
            }
            b'"' => {
                in_double_quotes = true;
                comment_can_start = false;
            }
            b'\\' => {
                index += 1;
                comment_can_start = false;
            }
            b'$' => {
                index = skip_dollar_expansion_text(bytes, index);
                comment_can_start = false;
                continue;
            }
            byte if byte.is_ascii_whitespace() || is_shell_word_separator(byte as char) => {
                comment_can_start = true;
            }
            _ => comment_can_start = false,
        }

        index += 1;
    }

    line
}

struct ShellLikeCommandWordPositions<'a> {
    line: &'a str,
    index: usize,
    at_command_start: bool,
    after_time_prefix: bool,
}

impl<'a> Iterator for ShellLikeCommandWordPositions<'a> {
    type Item = (&'a str, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.line.as_bytes();

        while self.index < bytes.len() {
            match bytes[self.index] {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.index += 1;
                }
                b'(' if self.at_command_start && bytes.get(self.index + 1) == Some(&b'(') => {
                    self.index = skip_balanced_text(bytes, self.index + 1, b'(', b')');
                    self.at_command_start = false;
                    self.after_time_prefix = false;
                }
                b';' | b'|' | b'&' | b'(' | b')' | b'{' | b'}' => {
                    self.at_command_start = true;
                    self.after_time_prefix = false;
                    self.index += 1;
                }
                b'!' if self.at_command_start && bang_is_command_prefix(bytes, self.index) => {
                    self.index += 1;
                }
                b'\'' => {
                    self.index = skip_single_quoted_text(bytes, self.index + 1);
                    self.at_command_start = false;
                    self.after_time_prefix = false;
                }
                b'"' => {
                    self.index = skip_double_quoted_text(bytes, self.index + 1);
                    self.at_command_start = false;
                    self.after_time_prefix = false;
                }
                b'`' => {
                    self.index += 1;
                    self.at_command_start = true;
                    self.after_time_prefix = false;
                }
                b'$' => {
                    if starts_command_substitution(bytes, self.index) {
                        self.index += 2;
                        self.at_command_start = true;
                        self.after_time_prefix = false;
                    } else {
                        self.index = skip_dollar_expansion_text(bytes, self.index);
                        self.at_command_start = false;
                        self.after_time_prefix = false;
                    }
                }
                b'<' | b'>' if bytes.get(self.index + 1) == Some(&b'(') => {
                    self.index += 2;
                    self.at_command_start = true;
                    self.after_time_prefix = false;
                }
                b'\\' => {
                    self.index = (self.index + 2).min(bytes.len());
                    self.at_command_start = false;
                    self.after_time_prefix = false;
                }
                b'-' if self.at_command_start && self.after_time_prefix => {
                    let start = self.index;
                    while self.index < bytes.len()
                        && !bytes[self.index].is_ascii_whitespace()
                        && !is_shell_word_separator(bytes[self.index] as char)
                    {
                        self.index += 1;
                    }
                    if &self.line[start..self.index] != "-p" {
                        self.at_command_start = false;
                        self.after_time_prefix = false;
                    }
                }
                byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                    let start = self.index;
                    while self.index < bytes.len()
                        && (bytes[self.index].is_ascii_alphanumeric() || bytes[self.index] == b'_')
                    {
                        self.index += 1;
                    }
                    let is_command_word = self.at_command_start;
                    let is_assignment_word = bytes.get(self.index) == Some(&b'=');
                    let word = &self.line[start..self.index];
                    let is_case_pattern_label =
                        is_command_word && word_starts_case_pattern_label(bytes, self.index);
                    if is_case_pattern_label {
                        self.at_command_start = false;
                        self.after_time_prefix = false;
                        continue;
                    }
                    self.after_time_prefix =
                        is_command_word && !is_assignment_word && command_word_is_time_prefix(word);
                    self.at_command_start = is_command_word
                        && !is_assignment_word
                        && (command_word_opens_clause_context(word) || self.after_time_prefix);
                    if is_command_word && !is_assignment_word {
                        return Some((word, start));
                    }
                }
                _ => {
                    self.index += 1;
                    self.at_command_start = false;
                    self.after_time_prefix = false;
                }
            }
        }

        None
    }
}

fn bang_is_command_prefix(bytes: &[u8], index: usize) -> bool {
    bytes
        .get(index + 1)
        .is_none_or(|byte| byte.is_ascii_whitespace())
}

fn command_word_opens_clause_context(word: &str) -> bool {
    matches!(word, "then" | "do" | "else" | "elif")
}

fn command_word_is_time_prefix(word: &str) -> bool {
    word == "time"
}

fn word_starts_case_pattern_label(bytes: &[u8], mut index: usize) -> bool {
    loop {
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }

        let Some(byte) = bytes.get(index).copied() else {
            return false;
        };
        match byte {
            b')' => return true,
            b'|' => {
                if bytes.get(index + 1) == Some(&b'|') {
                    return false;
                }
                index += 1;
            }
            b';' | b'&' | b'\r' | b'\n' => return false,
            b'\'' => index = skip_single_quoted_text(bytes, index + 1),
            b'"' => index = skip_double_quoted_text(bytes, index + 1),
            b'`' => index = skip_backtick_text(bytes, index + 1),
            b'$' => index = skip_dollar_expansion_text(bytes, index),
            b'\\' => index = (index + 2).min(bytes.len()),
            other if is_shell_word_separator(other as char) && other != b'|' => return false,
            _ => index += 1,
        }
    }
}

struct ShellLikeWordPositions<'a> {
    line: &'a str,
    iter: std::str::CharIndices<'a>,
    start: Option<usize>,
    finished: bool,
}

impl<'a> Iterator for ShellLikeWordPositions<'a> {
    type Item = (&'a str, usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            match self.iter.next() {
                Some((index, ch)) => match ch {
                    '\'' | '"' | '`' => {
                        self.start = None;
                        self.skip_quoted(ch);
                    }
                    '$' => {
                        self.start = None;
                        self.skip_dollar_expansion();
                    }
                    '<' | '>' if self.next_char_is('(') => {
                        self.start = None;
                        self.iter.next();
                        self.skip_balanced('(', ')');
                    }
                    '\\' => {
                        self.start = None;
                        self.iter.next();
                    }
                    _ if ch.is_ascii_alphanumeric() || ch == '_' => {
                        if self.start.is_none() {
                            self.start = Some(index);
                        }
                    }
                    _ => {
                        if let Some(word_start) = self.start.take() {
                            return Some((&self.line[word_start..index], word_start));
                        }
                    }
                },
                None => {
                    self.finished = true;
                    return self
                        .start
                        .take()
                        .map(|word_start| (&self.line[word_start..], word_start));
                }
            }
        }
    }
}

impl ShellLikeWordPositions<'_> {
    fn next_char_is(&self, expected: char) -> bool {
        matches!(self.iter.clone().next(), Some((_, ch)) if ch == expected)
    }

    fn skip_quoted(&mut self, quote: char) {
        while let Some((_, ch)) = self.iter.next() {
            if quote != '\'' && ch == '\\' {
                self.iter.next();
                continue;
            }
            if ch == quote {
                break;
            }
        }
    }

    fn skip_dollar_expansion(&mut self) {
        let Some((_, ch)) = self.iter.clone().next() else {
            return;
        };

        match ch {
            '{' => {
                self.iter.next();
                self.skip_balanced('{', '}');
            }
            '(' => {
                self.iter.next();
                self.skip_balanced('(', ')');
            }
            '[' => {
                self.iter.next();
                self.skip_balanced('[', ']');
            }
            _ if is_shell_name_start(ch) => {
                self.iter.next();
                self.skip_shell_name_tail();
            }
            _ => {
                self.iter.next();
            }
        }
    }

    fn skip_balanced(&mut self, open: char, close: char) {
        let mut depth = 1usize;

        while let Some((_, ch)) = self.iter.next() {
            match ch {
                '\'' | '"' | '`' => self.skip_quoted(ch),
                '$' => self.skip_dollar_expansion(),
                '\\' => {
                    self.iter.next();
                }
                _ if ch == open => depth += 1,
                _ if ch == close => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_shell_name_tail(&mut self) {
        while let Some((_, ch)) = self.iter.clone().next() {
            if !is_shell_name_char(ch) {
                break;
            }
            self.iter.next();
        }
    }
}

fn is_shell_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_shell_name_char(ch: char) -> bool {
    is_shell_name_start(ch) || ch.is_ascii_digit()
}

fn is_x037_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}

fn is_c016_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}

fn is_c034_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}

fn is_x048_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}
fn eof_point(file: &File) -> Span {
    Span::from_positions(file.span.end, file.span.end)
}

fn c_prototype_fragment_span(diagnostic: &ParseDiagnostic, locator: Locator<'_>) -> Option<Span> {
    if !diagnostic
        .message
        .starts_with("expected compound command for function body")
    {
        return None;
    }
    let line = diagnostic.span.start.line;
    let line_text = line_text_at(locator, line)?;
    let column = find_attached_background_ampersand_column(line_text)?;
    let line_start_offset = line_start_offset(locator, line)?;
    let offset = line_start_offset + (column - 1);
    let point = Position {
        line,
        column,
        offset,
    };
    Some(Span::from_positions(point, point))
}

fn find_attached_background_ampersand_column(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.len() < 2 {
        return None;
    }

    for index in 0..bytes.len() - 1 {
        if bytes[index] != b'&' {
            continue;
        }
        let next = bytes[index + 1];
        if !(next == b'_' || next.is_ascii_alphanumeric()) {
            continue;
        }

        if index > 0 {
            let previous = bytes[index - 1];
            if previous == b'\\' || previous == b'&' || previous == b'|' {
                continue;
            }
            if !previous.is_ascii_whitespace() && !matches!(previous, b';' | b'(' | b')') {
                continue;
            }
        }

        return Some(index + 1);
    }

    None
}

fn line_text_at<'a>(locator: Locator<'a>, target_line: usize) -> Option<&'a str> {
    let source = locator.source();
    let range = locator.line_index().line_range(target_line, source)?;
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    if start == end && start >= source.len() {
        // The trailing empty line after a final '\n' is not yielded by
        // str::lines(); preserve those semantics for callers.
        return None;
    }
    let text = &source[start..end];
    Some(text.strip_suffix('\r').unwrap_or(text))
}

fn line_start_offset(locator: Locator<'_>, target_line: usize) -> Option<usize> {
    let line_index = locator.line_index();
    if let Some(start) = line_index.line_start(target_line) {
        return Some(usize::from(start));
    }
    let source = locator.source();
    // Tolerate diagnostics referencing the implicit empty line at EOF when the
    // source has no trailing newline, matching the previous helper's behavior.
    if target_line == line_index.line_count() + 1
        && source.as_bytes().last().is_some_and(|&b| b != b'\n')
    {
        return Some(source.len());
    }
    None
}

#[cfg(test)]
mod tests {
    use shuck_ast::File;
    use shuck_indexer::Indexer;
    use shuck_indexer::LineIndex;
    use shuck_parser::parser::{ParseResult, Parser};

    use super::{
        collect_parse_rule_diagnostics as collect_parse_rule_diagnostics_impl,
        if_bracket_glued_span_on_line, is_expected_command_error, line_contains_shell_word,
        line_has_command_leading_word, unterminated_if_position_from_source,
    };
    use crate::{
        Applicability, Diagnostic, LinterSemanticArtifacts, LinterSettings, Locator, Rule, RuleSet,
        ShellDialect, apply_fixes,
    };

    fn collect_parse_rule_diagnostics(
        file: &File,
        source: &str,
        parse_result: Option<&ParseResult>,
        enabled_rules: &RuleSet,
        shell: ShellDialect,
    ) -> Vec<Diagnostic> {
        let parse_result = parse_result.expect("parse diagnostics tests pass a parse result");
        let indexer = Indexer::new(source, parse_result);
        let semantic = LinterSemanticArtifacts::build(file, source, &indexer);
        let locator = Locator::new(source, indexer.line_index());
        collect_parse_rule_diagnostics_impl(
            file,
            locator,
            Some(parse_result),
            &semantic,
            enabled_rules,
            shell,
        )
    }

    #[test]
    fn maps_missing_fi_parse_error_to_c035_at_end_of_file() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::MissingFi);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::MissingFi);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("append a closing `fi`")
        );
    }

    #[test]
    fn maps_missing_fi_parse_error_to_c034_at_opening_if() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_does_not_run_for_zsh_sources() {
        let source = "#!/usr/bin/env zsh\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Zsh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn c034_handles_hash_in_parameter_expansion_before_then() {
        let source = "#!/bin/sh\nif [ \"${x#foo}\" = \"$x\" ]; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_handles_length_parameter_expansion_before_then() {
        let source = "#!/bin/sh\nif [ ${#x} -gt 0 ]; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_closing_keywords_inside_comments() {
        let source = "#!/bin/sh\nif true; then\n  : # fi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_inside_quotes() {
        let source = "#!/bin/sh\nif true; then\n  printf '%s\\n' \"fi\" 'then'\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_inside_multiline_quotes() {
        let source = "#!/bin/sh\nif true; then\n  printf \"\nfi\n\"\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_scans_commands_after_closing_multiline_quotes() {
        let source = "#!/bin/sh\nprintf \"\n\" ; if true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 5);
    }

    #[test]
    fn c034_tracks_heredocs_after_closing_multiline_quotes() {
        let source = "\
#!/bin/sh
if true; then
printf \"
\" ; cat <<'EOF'
fi
EOF
  :
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_inside_heredoc_payloads() {
        let source = "\
#!/bin/sh
if true; then
  cat <<EOF
fi
EOF
  :
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_in_case_pattern_labels() {
        let source = "\
#!/bin/sh
if outer; then
  case \"$value\" in
    fi) echo no ;;
  esac
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_in_case_alternation_labels() {
        let source = "\
#!/bin/sh
if outer; then
  case \"$value\" in
    fi|foo) echo no ;;
  esac
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_inside_expansions() {
        let source = "\
#!/bin/sh
if true; then
  : ${value:-fi} $(printf fi) $fi $(( fi + 1 ))
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_control_keywords_inside_multiline_expansions() {
        let source = "\
#!/bin/sh
if true; then
  : $((
fi + 1
))
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_tracks_if_inside_command_substitution() {
        let source = "\
#!/bin/sh
x=$(if true; then
  :
)
";
        let if_offset = source
            .find("if true")
            .expect("fixture has a command substitution if");

        assert_eq!(
            unterminated_if_position_from_source(source),
            Some((2, 5, if_offset))
        );
    }

    #[test]
    fn c034_tracks_if_inside_backtick_command_substitution() {
        let source = "\
#!/bin/sh
x=`if true; then
  :
`
";
        let if_offset = source
            .find("if true")
            .expect("fixture has a backtick substitution if");

        assert_eq!(
            unterminated_if_position_from_source(source),
            Some((2, 4, if_offset))
        );
    }

    #[test]
    fn c034_tracks_if_inside_process_substitution() {
        let source = "\
#!/bin/bash
cat <(if true; then
  :
)
";
        let if_offset = source
            .find("if true")
            .expect("fixture has a process substitution if");

        assert_eq!(
            unterminated_if_position_from_source(source),
            Some((2, 7, if_offset))
        );
    }

    #[test]
    fn c034_ignores_control_keywords_inside_arithmetic_commands() {
        let source = "\
#!/bin/bash
if true; then
  (( fi += 1 ))
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Bash,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_handles_even_trailing_backslashes_before_closing_keyword_words() {
        let source = "\
#!/bin/sh
if true; then
  echo foo\\\\
  echo fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_ignores_assignment_words_named_like_control_keywords() {
        let source = "\
#!/bin/sh
if true; then
  fi=1
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_keeps_command_start_after_bang_prefixes() {
        let source = "\
#!/bin/sh
if true; then
  ! if inner; then
    :
  fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_keeps_command_start_after_clause_bang_prefixes() {
        let source = "\
#!/bin/sh
if outer; then ! if inner; then
  :
fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_keeps_command_start_after_time_prefixes() {
        let source = "\
#!/bin/bash
if outer; then
  time if inner; then
    :
  fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Bash,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_keeps_command_start_after_time_p_option() {
        let source = "\
#!/bin/bash
if outer; then
  time -p if inner; then
    :
  fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Bash,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_tracks_nested_if_after_then_on_same_line() {
        let source = "\
#!/bin/sh
if outer; then if inner; then :; fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn c034_tracks_if_after_brace_group_openers() {
        let source = "\
#!/bin/sh
f() { if true; then
  :
}
";
        let if_offset = source.find("if true").expect("fixture has an if token");

        assert_eq!(
            unterminated_if_position_from_source(source),
            Some((2, 7, if_offset))
        );
    }

    #[test]
    fn c034_uses_byte_offset_after_non_ascii_prefix_text() {
        let source = "#!/bin/sh\necho é; if true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnterminatedIf);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 9);
    }

    #[test]
    fn c034_preserves_tokens_split_by_escaped_newline() {
        let source = "#!/bin/sh\ni\\\nf true; then\n  :\n";
        let if_offset = source.find("i\\").expect("fixture has split if token");

        assert_eq!(
            unterminated_if_position_from_source(source),
            Some((2, 1, if_offset))
        );
    }

    #[test]
    fn c034_reports_the_unclosed_nested_if() {
        let source = "\
#!/bin/sh
if outer; then
  if inner; then
    :
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 3);
    }

    #[test]
    fn c034_reports_the_outer_if_when_inner_block_is_closed() {
        let source = "\
#!/bin/sh
if outer; then
  if inner; then
    :
  fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnterminatedIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn does_not_map_line_continued_heredoc_pipeline_to_c035() {
        let source = "\
#!/bin/sh
if [ ! -f ${sslcert} ] ; then
cat << EOF | openssl req -new -key ${sslkey} \\
         -x509 -days 365 -set_serial $RANDOM \\
         -out ${sslcert} 2>/dev/null
--
SomeState
SomeCity
SomeOrganization
SomeOrganizationalUnit
${FQDN}
root@${FQDN}
EOF
fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::MissingFi);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert!(
            !recovered
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.starts_with("expected 'fi'")),
            "unexpected parse diagnostics: {:?}",
            recovered.diagnostics
        );
    }

    #[test]
    fn maps_function_parameter_parse_error_to_x035() {
        let source = "#!/bin/sh\nfunction g(y) { :; }\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::FunctionParamsInSh);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FunctionParamsInSh);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn maps_function_parameter_parse_error_to_the_paren_near_reported_position() {
        let source = "#!/bin/sh\necho \"$(x)\"; function f(y) { :; }\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::FunctionParamsInSh);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FunctionParamsInSh);
        assert_eq!(diagnostics[0].span.slice(source), "(");
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 24);
    }

    #[test]
    fn ignores_missing_fi_parse_error_when_rule_is_not_enabled() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnusedAssignment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_loop_without_end_parse_error_to_c141_at_end_of_file() {
        let source = "#!/bin/sh\nwhile true; do\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LoopWithoutEnd);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Unsafe).code,
            "#!/bin/sh\nwhile true; do\n  :\ndone\n"
        );
    }

    #[test]
    fn ignores_loop_without_end_parse_error_when_rule_is_not_enabled() {
        let source = "#!/bin/sh\nwhile true; do\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnusedAssignment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_balanced_while_loop_for_c141() {
        let source = "#!/bin/sh\nwhile true; do\n  :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_balanced_until_loop_for_c141() {
        let source = "#!/bin/sh\nuntil false; do\n  :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_nested_for_loop_missing_done_for_c141() {
        let source = "#!/bin/sh\nwhile true; do\n  for x in a; do\n    :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_balanced_nested_loops_for_c141() {
        let source = "#!/bin/sh\nwhile true; do\n  until false; do\n    :\n  done\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_for_loop_missing_done_for_c141() {
        let source = "#!/bin/sh\nfor x in a; do\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_for_loop_missing_done_parse_error_to_c142() {
        let source = "#!/bin/sh\nfor x in a; do\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::MissingDoneInForLoop);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::MissingDoneInForLoop);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("append a closing `done`")
        );
    }

    #[test]
    fn line_has_command_leading_word_matches_first_segment_word() {
        assert!(line_has_command_leading_word("until true", "until"));
        assert!(line_has_command_leading_word("  until true", "until"));
        assert!(line_has_command_leading_word("foo; until true", "until"));
        assert!(line_has_command_leading_word("foo |  until true", "until"));
        assert!(line_has_command_leading_word("foo && until true", "until"));
        assert!(!line_has_command_leading_word("foo until", "until"));
        assert!(!line_has_command_leading_word("", "until"));
    }

    #[test]
    fn line_has_command_leading_word_skips_segment_leading_punctuation() {
        // Original semantics yield the first identifier-like run anywhere in
        // the segment (after `split` on `;|&`); leading parens etc. must not
        // hide the keyword that follows.
        assert!(line_has_command_leading_word("(until true)", "until"));
        assert!(line_has_command_leading_word("foo; (until true)", "until"));
        assert!(line_has_command_leading_word(") done", "done"));
    }

    #[test]
    fn line_has_command_leading_word_does_not_loop_on_non_id_bytes() {
        // Regression guard: lines with non-id, non-whitespace, non-separator
        // bytes (parentheses, redirects, quotes) must terminate.
        assert!(!line_has_command_leading_word("()()()()", "until"));
        assert!(!line_has_command_leading_word("<<<", "until"));
        assert!(!line_has_command_leading_word(r#"""""#, "until"));
    }

    #[test]
    fn line_contains_shell_word_ignores_expansion_internals() {
        assert!(line_contains_shell_word("if true; then :; fi", "then"));
        assert!(line_contains_shell_word(
            "if true; then # keep body valid",
            "then"
        ));
        assert!(!line_contains_shell_word("${then}", "then"));
        assert!(!line_contains_shell_word("$(then)", "then"));
        assert!(!line_contains_shell_word("`then`", "then"));
    }

    #[test]
    fn ignores_inline_then_before_later_expected_command_for_c064() {
        let source = "#!/bin/sh\nif true; then :; fi\n&&\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfMissingThen);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(
            recovered
                .diagnostics
                .iter()
                .any(|diagnostic| is_expected_command_error(&diagnostic.message))
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_missing_then_even_when_later_lines_expand_then_identifiers() {
        let source = "#!/bin/sh\nif true\n  echo ${then}\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfMissingThen);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::IfMissingThen);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn ignores_then_with_trailing_comment_before_later_expected_command_for_c064() {
        let source = "#!/bin/sh\nif true; then # keep body valid\n  :\nfi\n&&\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfMissingThen);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(
            recovered
                .diagnostics
                .iter()
                .any(|diagnostic| is_expected_command_error(&diagnostic.message))
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_for_loop_missing_done_with_line_continuation_and_heredoc_to_c142() {
        let source = "#!/bin/sh\nfor name in \\\n  alpha beta; do\n  cat <<EOF\n$name\nEOF\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::MissingDoneInForLoop);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::MissingDoneInForLoop);
    }

    #[test]
    fn ignores_for_loop_with_heredoc_and_trailing_done_for_c142() {
        let source = "#!/bin/sh\nfor name in \\\n  alpha beta; do\n  cat <<EOF\n$name\nEOF\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::MissingDoneInForLoop);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_while_loop_missing_done_for_c142() {
        let source = "#!/bin/sh\nwhile true; do\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::MissingDoneInForLoop);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_dangling_else_parse_error_to_c143() {
        let source = "#!/bin/sh\nif true; then echo yes; else fi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::DanglingElse);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DanglingElse);
    }

    #[test]
    fn maps_nested_empty_else_parse_error_to_c143() {
        let source = "#!/bin/sh\nif true; then\n  if false; then\n    :\n  else\n  fi\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::DanglingElse);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DanglingElse);
    }

    #[test]
    fn ignores_non_empty_nested_else_with_other_parse_recovery_noise_for_c143() {
        let source = "#!/bin/sh\nif true; then\n  :\nelse\n  if false; then\n    :\n  fi\nfi\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::DanglingElse);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_dangling_else_parse_error_when_rule_is_not_enabled() {
        let source = "#!/bin/sh\nif true; then echo yes; else fi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UnusedAssignment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_until_missing_do_parse_error_to_c146() {
        let source = "#!/bin/sh\nuntil :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UntilMissingDo);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UntilMissingDo);
    }

    #[test]
    fn maps_multiline_until_header_missing_do_parse_error_to_c146() {
        let source = "#!/bin/sh\nuntil\n  false\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UntilMissingDo);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UntilMissingDo);
    }

    #[test]
    fn ignores_non_until_expected_command_parse_errors_for_c146() {
        let source = "#!/bin/sh\nwhile :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UntilMissingDo);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_until_with_do_after_comments_and_blank_lines_for_c146() {
        let source = "#!/bin/sh\nuntil false\n  # keep checking\n\ndo\n  :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UntilMissingDo);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_plain_until_word_before_done_for_c146() {
        let source = "#!/bin/sh\necho until\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::UntilMissingDo);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_if_bracket_glued_parse_error_to_c157() {
        let source = "#!/bin/sh\nif[ \"${1:-}\" = ok ]; then\n  :\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::IfBracketGlued);
        assert_eq!(diagnostics[0].span.slice(source), "if[");
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            "#!/bin/sh\nif [ \"${1:-}\" = ok ]; then\n  :\nfi\n"
        );
    }

    #[test]
    fn maps_if_bracket_glued_after_case_arm_terminator_to_c157() {
        let source = "\
#!/bin/sh
case \"$1\" in
ok)if[ \"$2\" = yes ]; then
  :
fi
;;
esac
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::IfBracketGlued);
        assert_eq!(diagnostics[0].span.slice(source), "if[");
    }

    #[test]
    fn ignores_valid_if_bracket_spacing_variants_on_line() {
        for line in [
            "if [ \"$x\" = ok ]; then",
            "if  [ \"$x\" = ok ]; then",
            "if\t[ \"$x\" = ok ]; then",
        ] {
            let source = format!("#!/bin/sh\n{line}\n");
            let line_index = LineIndex::new(&source);
            let locator = Locator::new(&source, &line_index);
            assert!(
                if_bracket_glued_span_on_line(locator, 2).is_none(),
                "unexpected glued match for `{line}`"
            );
        }
    }

    #[test]
    fn ignores_quoted_if_bracket_prefix_text_on_line() {
        let source = "#!/bin/sh\necho \"if[ literal\"\n";
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);

        assert!(if_bracket_glued_span_on_line(locator, 2).is_none());
    }

    #[test]
    fn ignores_non_if_bracket_expected_command_parse_errors_for_c157() {
        let source = "#!/bin/sh\nuntil :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_missing_then_parse_error_for_c016() {
        let source = "#!/bin/sh\nif true\n  echo hi\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::StrayClosingKeyword);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_later_stray_closing_keyword_with_missing_then_recovery() {
        let source = "#!/bin/sh\nif true\n  echo hi\nfi\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::StrayClosingKeyword);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::StrayClosingKeyword);
        assert_eq!(diagnostics[0].span.slice(source), "done");
    }

    #[test]
    fn ignores_until_missing_do_parse_error_for_c016() {
        let source = "#!/bin/sh\nuntil :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::StrayClosingKeyword);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_valid_if_bracket_spacing_variants_for_c157() {
        let source = "\
#!/bin/sh
if [ \"$1\" = ok ]; then
  :
fi

if  [ \"$1\" = ok ]; then
  :
fi
";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_quoted_if_bracket_text_on_expected_command_lines_for_c157() {
        let source = "#!/bin/sh\ntrue\n&& printf '%s\\n' \"if[\"\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_parameter_expansion_text_containing_if_bracket_on_expected_command_lines_for_c157() {
        let source = "#!/bin/sh\ntrue\n&& echo ${x#if[}\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_spaced_parameter_expansion_patterns_containing_if_bracket_for_c157() {
        let source = "#!/bin/sh\ntrue\n&& echo ${x# if[}\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_if_bracket_text_inside_comments_after_redirection_operators_for_c157() {
        let source = "#!/bin/sh\ntrue\n&& ># if[\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::IfBracketGlued);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_linebreak_before_and_parse_error_to_s072() {
        let source = "#!/bin/bash\ntrue\n&& echo x\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LinebreakBeforeAnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Bash,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LinebreakBeforeAnd);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            "#!/bin/bash\ntrue &&\necho x\n"
        );
    }

    #[test]
    fn skips_linebreak_before_and_fix_after_trailing_comment_for_s072() {
        let source = "#!/bin/bash\ntrue # note\n&& echo x\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LinebreakBeforeAnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Bash,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LinebreakBeforeAnd);
        assert!(diagnostics[0].fix.is_none());
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            source
        );
    }

    #[test]
    fn ignores_non_and_expected_command_parse_errors_for_s072() {
        let source = "#!/bin/sh\nuntil :\ndone\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::LinebreakBeforeAnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_c_prototype_fragment_parse_recovery_to_c042() {
        let source = "#!/bin/sh\nX &NextItem ();\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::CPrototypeFragment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::CPrototypeFragment);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 3);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Safe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("insert a space after `&`")
        );
    }

    #[test]
    fn maps_zsh_brace_if_recovery_to_x038() {
        let source = "#!/bin/sh\nif [[ -n \"$x\" ]] {\n  :\n}\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshBraceIf);
        assert_eq!(diagnostics[0].span.slice(source), "{");
    }

    #[test]
    fn ignores_missing_then_without_zsh_brace_syntax() {
        let source = "#!/bin/sh\nif [[ -n \"$x\" ]]\n  :\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_condition_brace_groups_for_zsh_brace_if() {
        let source = "#!/bin/sh\nif true\n{ echo ok; }\nthen\n  :\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_later_then_after_brace_group_conditions_for_zsh_brace_if() {
        let source = "#!/bin/sh\nif true; { echo ok; }; echo more; then\n  :\nfi\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_brace_if_when_target_shell_is_zsh() {
        let source = "#!/bin/zsh\nif [[ -n \"$x\" ]] {\n  :\n}\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Zsh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_zsh_brace_if_recovery_even_with_later_parse_errors() {
        let source = "#!/bin/sh\nif [[ -n \"$x\" ]] {\n  :\n}\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshBraceIf);
        assert_eq!(diagnostics[0].span.slice(source), "{");
    }

    #[test]
    fn maps_zsh_brace_if_for_mksh_targets() {
        let source = "#!/bin/mksh\nif [[ -n \"$x\" ]] {\n  :\n}\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Mksh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshBraceIf);
        assert_eq!(diagnostics[0].span.slice(source), "{");
    }

    #[test]
    fn maps_zsh_always_block_to_x039() {
        let source = "#!/bin/sh\n{ :; } always { :; }\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshAlwaysBlock);
        assert_eq!(diagnostics[0].span.slice(source), "always");
    }

    #[test]
    fn maps_zsh_case_group_recovery_to_x037() {
        let source = concat!(
            "#!/bin/sh\n",
            "case \"$OSTYPE\" in\n",
            "  (darwin|freebsd)*) print ok ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ExtglobCase);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobCase);
        assert_eq!(diagnostics[0].span.slice(source), "(darwin|freebsd)");
    }

    #[test]
    fn maps_zsh_case_group_recovery_to_x037_for_bash_and_ksh_targets() {
        let source = concat!(
            "#!/bin/sh\n",
            "case \"$OSTYPE\" in\n",
            "  (darwin|freebsd)*) print ok ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ExtglobCase);

        for shell in [ShellDialect::Bash, ShellDialect::Ksh] {
            let diagnostics = collect_parse_rule_diagnostics(
                &recovered.file,
                source,
                Some(&recovered),
                &settings.rules,
                shell,
            );

            assert_eq!(
                diagnostics.len(),
                1,
                "expected one diagnostic for {shell:?}"
            );
            assert_eq!(diagnostics[0].rule, Rule::ExtglobCase);
            assert_eq!(diagnostics[0].span.slice(source), "(darwin|freebsd)");
        }
    }

    #[test]
    fn maps_embedded_zsh_case_group_recovery_to_x048() {
        let source = concat!(
            "#!/bin/sh\n",
            "case \"$x\" in\n",
            "  foo_(a|b)_*) echo match ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ExtglobInCasePattern);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInCasePattern);
        assert_eq!(diagnostics[0].span.slice(source), "(a|b)");
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Safe)
        );
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            concat!(
                "#!/bin/sh\n",
                "case \"$x\" in\n",
                "  foo_a_*|foo_b_*) echo match ;;\n",
                "esac\n",
            )
        );
    }

    #[test]
    fn expands_multiple_embedded_case_groups_with_one_fix() {
        let source = concat!(
            "#!/bin/sh\n",
            "case \"$x\" in\n",
            "  foo_(a|b)_(c|d)) echo match ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ExtglobInCasePattern);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_some())
        );
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            concat!(
                "#!/bin/sh\n",
                "case \"$x\" in\n",
                "  foo_a_c|foo_a_d|foo_b_c|foo_b_d) echo match ;;\n",
                "esac\n",
            )
        );
    }

    #[test]
    fn expands_embedded_case_group_after_bracket_class_pipe() {
        let source = concat!(
            "#!/bin/sh\n",
            "case \"$x\" in\n",
            "  foo_[a|b](c|d)) echo match ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ExtglobInCasePattern);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInCasePattern);
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            concat!(
                "#!/bin/sh\n",
                "case \"$x\" in\n",
                "  foo_[a|b]c|foo_[a|b]d) echo match ;;\n",
                "esac\n",
            )
        );
    }

    #[test]
    fn expands_case_group_alternatives_around_bracket_class_pipe() {
        let source = concat!(
            "#!/bin/sh\n",
            "case \"$x\" in\n",
            "  foo_([a|b]|c)) echo match ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ExtglobInCasePattern);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInCasePattern);
        assert_eq!(
            apply_fixes(source, &diagnostics, Applicability::Safe).code,
            concat!(
                "#!/bin/sh\n",
                "case \"$x\" in\n",
                "  foo_[a|b]|foo_c) echo match ;;\n",
                "esac\n",
            )
        );
    }

    #[test]
    fn collects_multiple_zsh_recovery_rules_together() {
        let source = concat!(
            "#!/bin/sh\n",
            "if [[ -n \"$x\" ]] {\n",
            "  :\n",
            "}\n",
            "{ :; } always { :; }\n",
            "case \"$x\" in\n",
            "  (a|b)*) echo lead ;;\n",
            "esac\n",
            "case \"$x\" in\n",
            "  foo_(c|d)_*) echo embedded ;;\n",
            "esac\n",
        );
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rules([
            Rule::ZshBraceIf,
            Rule::ZshAlwaysBlock,
            Rule::ExtglobCase,
            Rule::ExtglobInCasePattern,
        ]);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );
        let rules: std::collections::HashSet<_> = diagnostics.iter().map(|d| d.rule).collect();

        assert_eq!(rules.len(), 4);
        assert!(rules.contains(&Rule::ZshBraceIf));
        assert!(rules.contains(&Rule::ZshAlwaysBlock));
        assert!(rules.contains(&Rule::ExtglobCase));
        assert!(rules.contains(&Rule::ExtglobInCasePattern));
    }

    #[test]
    fn ignores_non_always_brace_groups_for_x039() {
        let source = "#!/bin/sh\n{ :; }\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_always_block_when_target_shell_is_zsh() {
        let source = "#!/bin/zsh\n{ :; } always { :; }\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Zsh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_zsh_always_block_even_with_later_parse_errors() {
        let source = "#!/bin/sh\n{ :; } always { :; }\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            Some(&recovered),
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshAlwaysBlock);
        assert_eq!(diagnostics[0].span.slice(source), "always");
    }
}
