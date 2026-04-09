use shuck_ast::{File, Position, Span};
use shuck_parser::parser::ParseDiagnostic;

use crate::rules::correctness::c_prototype_fragment::CPrototypeFragment;
use crate::rules::correctness::missing_fi::MissingFi;
use crate::{Diagnostic, RuleSet};

pub(crate) fn collect_parse_rule_diagnostics(
    file: &File,
    source: &str,
    parse_diagnostics: &[ParseDiagnostic],
    enabled_rules: &RuleSet,
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
    let point = Position { line, column, offset };
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
    use crate::{LinterSettings, Rule};

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
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::CPrototypeFragment);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 3);
    }
}
