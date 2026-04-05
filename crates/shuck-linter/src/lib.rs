mod checker;
mod diagnostic;
mod registry;
mod rule_selector;
mod rule_set;
pub mod rules;
mod settings;
mod violation;

pub use checker::Checker;
pub use diagnostic::{Diagnostic, Severity};
pub use registry::{Category, Rule, code_to_rule};
pub use rule_selector::{RuleSelector, SelectorParseError};
pub use rule_set::RuleSet;
pub use settings::LinterSettings;
pub use violation::Violation;

use shuck_ast::Script;
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;

pub fn lint_file(
    script: &Script,
    source: &str,
    semantic: &SemanticModel,
    indexer: &Indexer,
    settings: &LinterSettings,
) -> Vec<Diagnostic> {
    let checker = Checker::new(script, source, semantic, indexer, &settings.rules);
    let mut diagnostics = checker.check();

    for diagnostic in &mut diagnostics {
        if let Some(&severity) = settings.severity_overrides.get(&diagnostic.rule) {
            diagnostic.severity = severity;
        }
    }

    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.start.offset, diagnostic.span.end.offset));
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn lint(source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.script, source, &indexer);
        lint_file(&output.script, source, &semantic, &indexer, settings)
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
}
