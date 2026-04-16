use shuck_ast::{Command, CompoundCommand, File, Position, Span};
use shuck_parser::parser::{ParseDiagnostic, ParseResult};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::correctness::c_prototype_fragment::CPrototypeFragment;
use crate::rules::correctness::dangling_else::DanglingElse;
use crate::rules::correctness::if_bracket_glued::IfBracketGlued;
use crate::rules::correctness::if_missing_then::IfMissingThen;
use crate::rules::correctness::loop_without_end::LoopWithoutEnd;
use crate::rules::correctness::missing_done_in_for_loop::MissingDoneInForLoop;
use crate::rules::correctness::missing_fi::MissingFi;
use crate::rules::correctness::until_missing_do::UntilMissingDo;
use crate::rules::portability::function_params_in_sh::FunctionParamsInSh;
use crate::rules::portability::targets_non_zsh_shell;
use crate::rules::portability::zsh_always_block::ZshAlwaysBlock;
use crate::rules::portability::zsh_brace_if::ZshBraceIf;
use crate::rules::style::linebreak_before_and::LinebreakBeforeAnd;
use crate::{Diagnostic, Rule, RuleSet, ShellDialect, Violation};

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
    fn rule() -> Rule {
        Rule::ExtglobInCasePattern
    }

    fn message(&self) -> String {
        "extended glob alternation in a case pattern is not portable to POSIX sh".to_owned()
    }
}

pub(crate) fn collect_parse_rule_diagnostics(
    file: &File,
    source: &str,
    parse_result: Option<&ParseResult>,
    enabled_rules: &RuleSet,
    shell: ShellDialect,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let parse_diagnostics = parse_result
        .map(|result| result.diagnostics.as_slice())
        .unwrap_or(&[]);
    let missing_done_loop_kind = (enabled_rules.contains(crate::Rule::LoopWithoutEnd)
        || enabled_rules.contains(crate::Rule::MissingDoneInForLoop))
    .then(|| missing_done_loop_kind(file, source, parse_diagnostics))
    .flatten();

    if enabled_rules.contains(crate::Rule::MissingFi)
        && parse_diagnostics
            .iter()
            .any(|diagnostic| is_missing_fi_error(&diagnostic.message))
    {
        diagnostics.push(Diagnostic::new(MissingFi, eof_point(file)));
    }
    if enabled_rules.contains(crate::Rule::IfMissingThen)
        && let Some(span) = if_missing_then_span(source, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(IfMissingThen, span));
    }
    if enabled_rules.contains(crate::Rule::LoopWithoutEnd)
        && missing_done_loop_kind == Some(MissingDoneLoopKind::NonFor)
    {
        diagnostics.push(Diagnostic::new(LoopWithoutEnd, eof_point(file)));
    }
    if enabled_rules.contains(crate::Rule::MissingDoneInForLoop)
        && missing_done_loop_kind == Some(MissingDoneLoopKind::For)
    {
        diagnostics.push(Diagnostic::new(MissingDoneInForLoop, eof_point(file)));
    }
    if enabled_rules.contains(crate::Rule::DanglingElse)
        && let Some(span) = dangling_else_span(parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(DanglingElse, span));
    }
    if enabled_rules.contains(crate::Rule::UntilMissingDo)
        && let Some(span) = until_missing_do_span(source, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(UntilMissingDo, span));
    }
    if enabled_rules.contains(crate::Rule::IfBracketGlued)
        && let Some(span) = if_bracket_glued_span(source, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(IfBracketGlued, span));
    }
    if enabled_rules.contains(crate::Rule::LinebreakBeforeAnd) {
        for span in linebreak_before_and_spans(source, parse_diagnostics) {
            diagnostics.push(Diagnostic::new(LinebreakBeforeAnd, span));
        }
    }
    if enabled_rules.contains(crate::Rule::FunctionParamsInSh)
        && matches!(
            shell,
            ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
        )
    {
        for diagnostic in parse_diagnostics {
            if let Some(span) = function_parameter_syntax_span(source, diagnostic) {
                diagnostics.push(Diagnostic::new(FunctionParamsInSh, span));
            }
        }
    }

    if enabled_rules.contains(crate::Rule::CPrototypeFragment) {
        for diagnostic in parse_diagnostics {
            let Some(span) = c_prototype_fragment_span(diagnostic, source) else {
                continue;
            };
            diagnostics.push(Diagnostic::new(CPrototypeFragment, span));
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
                diagnostics.push(Diagnostic::new(ExtglobInCasePattern, part.span));
            }
        }
    }

    diagnostics
}

fn is_missing_fi_error(message: &str) -> bool {
    message.starts_with("expected 'fi'")
}

fn is_missing_then_error(message: &str) -> bool {
    message.starts_with("expected 'then'")
}

fn if_missing_then_span(source: &str, parse_diagnostics: &[ParseDiagnostic]) -> Option<Span> {
    parse_diagnostics.iter().find_map(|diagnostic| {
        if !is_missing_then_error(&diagnostic.message)
            && !is_expected_command_error(&diagnostic.message)
        {
            return None;
        }

        let mut line = diagnostic.span.start.line;
        let mut saw_then = false;
        while line > 0 {
            let text = line_text_at(source, line)?;
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
                let offset = line_start_offset(source, line)?;
                let start = position_at_offset(source, offset)?;
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

fn is_function_parameter_syntax_error(message: &str) -> bool {
    message.starts_with("expected ')' in function definition")
}

fn function_parameter_syntax_span(source: &str, diagnostic: &ParseDiagnostic) -> Option<Span> {
    if !is_function_parameter_syntax_error(&diagnostic.message) {
        return None;
    }

    let line = line_text_at(source, diagnostic.span.start.line)?;
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
    let line_start = line_start_offset(source, diagnostic.span.start.line)?;
    let start_offset = line_start + paren_index;
    let start = position_at_offset(source, start_offset)?;
    let end = position_at_offset(source, start_offset + 1)?;
    Some(Span::from_positions(start, end))
}

fn linebreak_before_and_spans(source: &str, parse_diagnostics: &[ParseDiagnostic]) -> Vec<Span> {
    parse_diagnostics
        .iter()
        .filter_map(|diagnostic| {
            if !is_expected_command_error(&diagnostic.message) {
                return None;
            }

            leading_control_operator_span(source, diagnostic.span.start.line)
        })
        .collect()
}

fn leading_control_operator_span(source: &str, line_number: usize) -> Option<Span> {
    let line = line_text_at(source, line_number)?;
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

    let line_start = line_start_offset(source, line_number)?;
    let start_offset = line_start + leading_bytes;
    let start = position_at_offset(source, start_offset)?;
    let end = position_at_offset(source, start_offset + operator.len())?;
    Some(Span::from_positions(start, end))
}

fn dangling_else_span(parse_diagnostics: &[ParseDiagnostic]) -> Option<Span> {
    parse_diagnostics
        .iter()
        .find(|diagnostic| is_dangling_else_error(&diagnostic.message))
        .map(|diagnostic| diagnostic.span)
}

fn until_missing_do_span(source: &str, parse_diagnostics: &[ParseDiagnostic]) -> Option<Span> {
    parse_diagnostics
        .iter()
        .find(|diagnostic| {
            is_expected_command_error(&diagnostic.message)
                && is_done_line(source, diagnostic.span.start.line)
                && has_pending_until_without_do_before_line(source, diagnostic.span.start.line)
        })
        .map(|diagnostic| diagnostic.span)
}

fn if_bracket_glued_span(source: &str, parse_diagnostics: &[ParseDiagnostic]) -> Option<Span> {
    parse_diagnostics.iter().find_map(|diagnostic| {
        if !is_expected_command_error(&diagnostic.message) {
            return None;
        }
        if_bracket_glued_span_on_line(source, diagnostic.span.start.line)
    })
}

fn if_bracket_glued_span_on_line(source: &str, line_number: usize) -> Option<Span> {
    let line = line_text_at(source, line_number)?;
    let line_offset = line_start_offset(source, line_number)?;
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
                let start = position_at_offset(source, start_offset)?;
                let end = position_at_offset(source, end_offset)?;
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

fn is_done_line(source: &str, line_number: usize) -> bool {
    line_text_at(source, line_number)
        .map(|line| {
            let text = line.split_once('#').map_or(line, |(before, _)| before);
            shell_like_words(text).contains(&"done")
        })
        .unwrap_or(false)
}

fn has_pending_until_without_do_before_line(source: &str, line_number: usize) -> bool {
    if line_number <= 1 {
        return false;
    }

    let mut pending_until_depth = 0usize;
    for (index, line) in source.lines().enumerate() {
        let current_line = index + 1;
        if current_line >= line_number {
            break;
        }

        let text = line.split_once('#').map_or(line, |(before, _)| before);
        if line_has_command_leading_word(text, "until") {
            pending_until_depth += 1;
        }
        if pending_until_depth > 0
            && (line_has_command_leading_word(text, "do")
                || line_has_command_leading_word(text, "done"))
        {
            pending_until_depth -= 1;
        }
    }

    pending_until_depth > 0
}

fn line_has_command_leading_word(line: &str, word: &str) -> bool {
    line.split([';', '|', '&'])
        .filter_map(|segment| shell_like_words(segment.trim_start()).into_iter().next())
        .any(|candidate| candidate == word)
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
) -> Option<MissingDoneLoopKind> {
    if !parse_diagnostics
        .iter()
        .any(|diagnostic| is_loop_without_end_error(&diagnostic.message))
    {
        return None;
    }

    Some(if missing_done_belongs_to_for_loop(file, source) {
        MissingDoneLoopKind::For
    } else {
        MissingDoneLoopKind::NonFor
    })
}

fn missing_done_belongs_to_for_loop(file: &File, source: &str) -> bool {
    let eof_offset = file.span.end.offset;
    let mut trailing_loop_kind = None;

    for visit in query::iter_commands(&file.body, CommandWalkOptions::default()) {
        if visit.stmt.span.end.offset != eof_offset {
            continue;
        }

        let is_for_loop = match visit.command {
            Command::Compound(CompoundCommand::For(_)) => true,
            Command::Compound(CompoundCommand::While(_) | CompoundCommand::Until(_)) => false,
            _ => continue,
        };

        let start_offset = visit.stmt.span.start.offset;
        if trailing_loop_kind
            .as_ref()
            .is_none_or(|(best_start, _)| start_offset >= *best_start)
        {
            trailing_loop_kind = Some((start_offset, is_for_loop));
        }
    }

    trailing_loop_kind
        .map(|(_, is_for_loop)| is_for_loop)
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
        let words = shell_like_words(&logical_line);
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
        let words = shell_like_words(continued_line.trim());
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

fn shell_like_words(line: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = None;

    for (index, ch) in line.char_indices() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if start.is_none() {
                start = Some(index);
            }
        } else if let Some(word_start) = start.take() {
            words.push(&line[word_start..index]);
        }
    }

    if let Some(word_start) = start {
        words.push(&line[word_start..]);
    }

    words
}

fn is_x037_shell(shell: ShellDialect) -> bool {
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

fn c_prototype_fragment_span(diagnostic: &ParseDiagnostic, source: &str) -> Option<Span> {
    if !diagnostic
        .message
        .starts_with("expected compound command for function body")
    {
        return None;
    }
    let line = diagnostic.span.start.line;
    let line_text = line_text_at(source, line)?;
    let column = find_attached_background_ampersand_column(line_text)?;
    let line_start_offset = line_start_offset(source, line)?;
    let offset = line_start_offset + (column - 1);
    let point = Position {
        line,
        column,
        offset,
    };
    Some(Span::from_positions(point, point))
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }
    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
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

fn line_text_at(source: &str, target_line: usize) -> Option<&str> {
    source
        .lines()
        .enumerate()
        .find_map(|(index, line)| (index + 1 == target_line).then_some(line))
}

fn line_start_offset(source: &str, target_line: usize) -> Option<usize> {
    let mut line = 1usize;
    let mut offset = 0usize;
    for raw_line in source.split_inclusive('\n') {
        if line == target_line {
            return Some(offset);
        }
        offset += raw_line.len();
        line += 1;
    }
    (line == target_line).then_some(offset)
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use super::{
        collect_parse_rule_diagnostics, if_bracket_glued_span_on_line, is_expected_command_error,
        line_contains_shell_word,
    };
    use crate::{LinterSettings, Rule, ShellDialect};

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
            assert!(
                if_bracket_glued_span_on_line(&source, 2).is_none(),
                "unexpected glued match for `{line}`"
            );
        }
    }

    #[test]
    fn ignores_quoted_if_bracket_prefix_text_on_line() {
        let source = "#!/bin/sh\necho \"if[ literal\"\n";

        assert!(if_bracket_glued_span_on_line(source, 2).is_none());
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
