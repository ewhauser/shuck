use shuck_ast::{
    Command, CompoundCommand, File, IfSyntax, PatternGroupKind, PatternPart, Position, Span,
};
use shuck_parser::parser::{ParseDiagnostic, Parser, ShellDialect as ParseShellDialect};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::correctness::c_prototype_fragment::CPrototypeFragment;
use crate::rules::correctness::loop_without_end::LoopWithoutEnd;
use crate::rules::correctness::missing_fi::MissingFi;
use crate::rules::portability::targets_non_zsh_shell;
use crate::rules::portability::zsh_always_block::ZshAlwaysBlock;
use crate::rules::portability::zsh_brace_if::ZshBraceIf;
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
    parse_diagnostics: &[ParseDiagnostic],
    enabled_rules: &RuleSet,
    shell: ShellDialect,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if enabled_rules.contains(crate::Rule::MissingFi)
        && parse_diagnostics
            .iter()
            .any(|diagnostic| is_missing_fi_error(&diagnostic.message))
    {
        diagnostics.push(Diagnostic::new(MissingFi, eof_point(file)));
    }
    if enabled_rules.contains(crate::Rule::LoopWithoutEnd)
        && has_loop_without_end_error(file, source, parse_diagnostics)
    {
        diagnostics.push(Diagnostic::new(LoopWithoutEnd, eof_point(file)));
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
        for span in zsh_brace_if_spans(source) {
            diagnostics.push(Diagnostic::new(ZshBraceIf, span));
        }
    }
    if enabled_rules.contains(crate::Rule::ZshAlwaysBlock) && targets_non_zsh_shell(shell) {
        for span in zsh_always_block_spans(source) {
            diagnostics.push(Diagnostic::new(ZshAlwaysBlock, span));
        }
    }
    if enabled_rules.contains(crate::Rule::ExtglobCase) && is_x037_shell(shell) {
        for span in zsh_case_leading_group_spans(source) {
            diagnostics.push(Diagnostic::new(ExtglobCase, span));
        }
    }
    if enabled_rules.contains(crate::Rule::ExtglobInCasePattern) && is_x048_shell(shell) {
        for span in zsh_case_embedded_group_spans(source) {
            diagnostics.push(Diagnostic::new(ExtglobInCasePattern, span));
        }
    }

    diagnostics
}

fn is_missing_fi_error(message: &str) -> bool {
    message.starts_with("expected 'fi'")
}

fn is_loop_without_end_error(message: &str) -> bool {
    message.starts_with("expected 'done'")
}

fn has_loop_without_end_error(
    file: &File,
    source: &str,
    parse_diagnostics: &[ParseDiagnostic],
) -> bool {
    if !parse_diagnostics
        .iter()
        .any(|diagnostic| is_loop_without_end_error(&diagnostic.message))
    {
        return false;
    }

    !missing_done_belongs_to_for_loop(file, source)
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

    for line in source.lines() {
        let text = line.split_once('#').map_or(line, |(before, _)| before);
        let words = shell_like_words(text);
        if words.is_empty() {
            continue;
        }

        let has_do = words.iter().any(|word| *word == "do");
        if has_do {
            if words.iter().any(|word| *word == "for") {
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

fn zsh_brace_if_spans(source: &str) -> Vec<Span> {
    let parsed = Parser::with_dialect(source, ParseShellDialect::Zsh).parse_recovered();

    query::iter_commands(&parsed.file.body, CommandWalkOptions::default())
        .filter_map(|visit| {
            let Command::Compound(CompoundCommand::If(command)) = visit.command else {
                return None;
            };
            let IfSyntax::Brace {
                left_brace_span, ..
            } = command.syntax
            else {
                return None;
            };
            Some(left_brace_span)
        })
        .collect()
}

fn zsh_always_block_spans(source: &str) -> Vec<Span> {
    let parsed = Parser::with_dialect(source, ParseShellDialect::Zsh).parse_recovered();

    query::iter_commands(&parsed.file.body, CommandWalkOptions::default())
        .filter_map(|visit| {
            let Command::Compound(CompoundCommand::Always(command)) = visit.command else {
                return None;
            };
            always_keyword_span(
                source,
                command.body.span.end.offset,
                command.always_body.span.start.offset,
            )
            .or(Some(visit.stmt.span))
        })
        .collect()
}

fn zsh_case_group_spans(source: &str) -> Vec<(usize, Span)> {
    let parsed = Parser::with_dialect(source, ParseShellDialect::Zsh).parse_recovered();

    query::iter_commands(&parsed.file.body, CommandWalkOptions::default())
        .flat_map(|visit| {
            let Command::Compound(CompoundCommand::Case(command)) = visit.command else {
                return Vec::new();
            };

            command
                .cases
                .iter()
                .flat_map(|case| {
                    case.patterns.iter().flat_map(|pattern| {
                        pattern
                            .parts
                            .iter()
                            .enumerate()
                            .filter_map(|(index, part)| match &part.kind {
                                PatternPart::Group {
                                    kind: PatternGroupKind::ExactlyOne,
                                    ..
                                } if part.span.slice(source).starts_with('(') => {
                                    Some((index, part.span))
                                }
                                PatternPart::Word(_)
                                | PatternPart::Literal(_)
                                | PatternPart::AnyString
                                | PatternPart::AnyChar
                                | PatternPart::CharClass(_)
                                | PatternPart::Group { .. } => None,
                            })
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn zsh_case_leading_group_spans(source: &str) -> Vec<Span> {
    zsh_case_group_spans(source)
        .into_iter()
        .filter_map(|(index, span)| (index == 0).then_some(span))
        .collect()
}

fn zsh_case_embedded_group_spans(source: &str) -> Vec<Span> {
    zsh_case_group_spans(source)
        .into_iter()
        .filter_map(|(index, span)| (index > 0).then_some(span))
        .collect()
}

fn always_keyword_span(source: &str, search_start: usize, search_end: usize) -> Option<Span> {
    let search_start = search_start.min(source.len());
    let search_end = search_end.min(source.len());
    if search_start >= search_end {
        return None;
    }

    let text = &source[search_start..search_end];
    let relative = text.find("always")?;
    let start_offset = search_start + relative;
    let end_offset = start_offset + "always".len();

    let start = position_at_offset(source, start_offset)?;
    let end = position_at_offset(source, end_offset)?;
    Some(Span::from_positions(start, end))
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

    use super::collect_parse_rule_diagnostics;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn maps_missing_fi_parse_error_to_c035_at_end_of_file() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::MissingFi);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::MissingFi);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn ignores_missing_fi_parse_error_when_rule_is_not_enabled() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::UnusedAssignment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_loop_without_end_parse_error_to_c141_at_end_of_file() {
        let source = "#!/bin/sh\nwhile true; do\n  :\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::UnusedAssignment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_for_loop_missing_done_for_c141() {
        let source = "#!/bin/sh\nfor x in a; do\n  :\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::LoopWithoutEnd);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_c_prototype_fragment_parse_recovery_to_c042() {
        let source = "#!/bin/sh\nX &NextItem ();\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::CPrototypeFragment);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_brace_if_when_target_shell_is_zsh() {
        let source = "#!/bin/zsh\nif [[ -n \"$x\" ]] {\n  :\n}\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Zsh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_zsh_brace_if_recovery_even_with_later_parse_errors() {
        let source = "#!/bin/sh\nif [[ -n \"$x\" ]] {\n  :\n}\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshBraceIf);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ExtglobCase);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ExtglobCase);

        for shell in [ShellDialect::Bash, ShellDialect::Ksh] {
            let diagnostics = collect_parse_rule_diagnostics(
                &recovered.file,
                source,
                &recovered.diagnostics,
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
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ExtglobInCasePattern);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInCasePattern);
        assert_eq!(diagnostics[0].span.slice(source), "(a|b)");
    }

    #[test]
    fn ignores_non_always_brace_groups_for_x039() {
        let source = "#!/bin/sh\n{ :; }\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_always_block_when_target_shell_is_zsh() {
        let source = "#!/bin/zsh\n{ :; } always { :; }\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Zsh,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn maps_zsh_always_block_even_with_later_parse_errors() {
        let source = "#!/bin/sh\n{ :; } always { :; }\nif true; then\n  :\n";
        let recovered = Parser::new(source).parse_recovered();
        let settings = LinterSettings::for_rule(Rule::ZshAlwaysBlock);
        let diagnostics = collect_parse_rule_diagnostics(
            &recovered.file,
            source,
            &recovered.diagnostics,
            &settings.rules,
            ShellDialect::Sh,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshAlwaysBlock);
        assert_eq!(diagnostics[0].span.slice(source), "always");
    }
}
