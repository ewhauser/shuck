use shuck_ast::{Command, CompoundCommand, File, IfSyntax, Position, Span};
use shuck_parser::parser::{ParseDiagnostic, Parser, ShellDialect as ParseShellDialect};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::correctness::c_prototype_fragment::CPrototypeFragment;
use crate::rules::correctness::missing_fi::MissingFi;
use crate::rules::portability::zsh_always_block::ZshAlwaysBlock;
use crate::rules::portability::zsh_brace_if::ZshBraceIf;
use crate::{Diagnostic, RuleSet, ShellDialect};

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

    if enabled_rules.contains(crate::Rule::CPrototypeFragment) {
        for diagnostic in parse_diagnostics {
            let Some(span) = c_prototype_fragment_span(diagnostic, source) else {
                continue;
            };
            diagnostics.push(Diagnostic::new(CPrototypeFragment, span));
        }
    }

    if enabled_rules.contains(crate::Rule::ZshBraceIf) && targets_x038_shell(shell) {
        for span in zsh_brace_if_spans(source) {
            diagnostics.push(Diagnostic::new(ZshBraceIf, span));
        }
    }
    if enabled_rules.contains(crate::Rule::ZshAlwaysBlock) && targets_x038_shell(shell) {
        for span in zsh_always_block_spans(source) {
            diagnostics.push(Diagnostic::new(ZshAlwaysBlock, span));
        }
    }

    diagnostics
}

fn is_missing_fi_error(message: &str) -> bool {
    message.starts_with("expected 'fi'")
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

fn targets_x038_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}

fn zsh_brace_if_spans(source: &str) -> Vec<Span> {
    let Ok(parsed) = Parser::with_dialect(source, ParseShellDialect::Zsh).parse() else {
        return Vec::new();
    };

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
    let Ok(parsed) = Parser::with_dialect(source, ParseShellDialect::Zsh).parse() else {
        return Vec::new();
    };

    query::iter_commands(&parsed.file.body, CommandWalkOptions::default())
        .filter_map(|visit| {
            let Command::Compound(CompoundCommand::Always(command)) = visit.command else {
                return None;
            };
            always_keyword_span(source, command.body.span.end.offset, command.always_body.span.start.offset)
                .or(Some(visit.stmt.span))
        })
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
}
