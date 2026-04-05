mod checker;
mod diagnostic;
mod registry;
mod rule_selector;
mod rule_set;
pub mod rules;
mod settings;
mod suppression;
mod violation;

pub use checker::Checker;
pub use diagnostic::{Diagnostic, Severity};
pub use registry::{Category, Rule, code_to_rule};
pub use rule_selector::{RuleSelector, SelectorParseError};
pub use rule_set::RuleSet;
pub use settings::LinterSettings;
pub use suppression::{
    ShellCheckCodeMap, SuppressionAction, SuppressionDirective, SuppressionIndex,
    SuppressionSource, first_statement_line, parse_directives,
};
pub use violation::Violation;

use shuck_ast::{Script, TextSize};
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;

pub fn lint_file(
    script: &Script,
    source: &str,
    semantic: &SemanticModel,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> Vec<Diagnostic> {
    let checker = Checker::new(script, source, semantic, indexer, &settings.rules);
    let mut diagnostics = checker.check();

    for diagnostic in &mut diagnostics {
        if let Some(&severity) = settings.severity_overrides.get(&diagnostic.rule) {
            diagnostic.severity = severity;
        }
    }

    if let Some(suppression_index) = suppression_index {
        filter_suppressed_diagnostics(&mut diagnostics, indexer, suppression_index);
    }

    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.start.offset, diagnostic.span.end.offset));
    diagnostics
}

fn filter_suppressed_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    indexer: &Indexer,
    suppression_index: &SuppressionIndex,
) {
    diagnostics.retain(|diagnostic| {
        let line = indexer
            .line_index()
            .line_number(TextSize::new(diagnostic.span.start.offset as u32));
        let Ok(line) = u32::try_from(line) else {
            return true;
        };

        !suppression_index.is_suppressed(diagnostic.rule, line)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_ast::Command;
    use shuck_parser::parser::Parser;

    fn lint(source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.script, source, &indexer);
        lint_file(&output.script, source, &semantic, &indexer, settings, None)
    }

    #[test]
    fn default_settings_run_without_emitting_noop_diagnostics() {
        let diagnostics = lint("#!/bin/bash\necho ok\n", &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn empty_rule_set_is_a_noop() {
        let diagnostics = lint(
            "#!/bin/bash\necho ok\n",
            &LinterSettings {
                rules: RuleSet::EMPTY,
                ..LinterSettings::default()
            },
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn post_hoc_filtering_removes_only_suppressed_diagnostics() {
        let source = "\
echo ok
# shellcheck disable=SC2086
echo $foo
echo $bar
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        let suppressions = SuppressionIndex::new(
            &directives,
            &output.script,
            first_statement_line(&output.script).unwrap_or(u32::MAX),
        );

        let echo_foo = match &output.script.commands[1] {
            Command::Simple(command) => command.span,
            other => panic!("expected simple command, got {other:?}"),
        };
        let echo_bar = match &output.script.commands[2] {
            Command::Simple(command) => command.span,
            other => panic!("expected simple command, got {other:?}"),
        };

        let mut diagnostics = vec![
            Diagnostic {
                rule: Rule::UnquotedExpansion,
                message: "first".to_owned(),
                severity: Rule::UnquotedExpansion.default_severity(),
                span: echo_foo,
            },
            Diagnostic {
                rule: Rule::UnquotedExpansion,
                message: "second".to_owned(),
                severity: Rule::UnquotedExpansion.default_severity(),
                span: echo_bar,
            },
        ];

        filter_suppressed_diagnostics(&mut diagnostics, &indexer, &suppressions);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "second");
    }
}
