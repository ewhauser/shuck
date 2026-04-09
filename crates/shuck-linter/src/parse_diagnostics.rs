use shuck_ast::{File, Span};
use shuck_parser::parser::ParseDiagnostic;

use crate::rules::correctness::missing_fi::MissingFi;
use crate::{Diagnostic, RuleSet};

pub(crate) fn collect_parse_rule_diagnostics(
    file: &File,
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

    diagnostics
}

fn is_missing_fi_error(message: &str) -> bool {
    message.starts_with("expected 'fi'")
}

fn eof_point(file: &File) -> Span {
    Span::from_positions(file.span.end, file.span.end)
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
            &recovered.diagnostics,
            &settings.rules,
        );

        assert!(diagnostics.is_empty());
    }
}
