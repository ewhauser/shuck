mod checker;
mod diagnostic;
mod registry;
mod rule_selector;
mod rule_set;
pub mod rules;
mod settings;
mod shell;
mod suppression;
mod violation;

#[cfg(test)]
pub mod test;

pub use checker::Checker;
pub use diagnostic::{Diagnostic, Severity};
pub use registry::{Category, Rule, code_to_rule};
pub use rule_selector::{RuleSelector, SelectorParseError};
pub use rule_set::RuleSet;
pub use settings::LinterSettings;
pub use shell::ShellDialect;
pub use suppression::{
    ShellCheckCodeMap, SuppressionAction, SuppressionDirective, SuppressionIndex,
    SuppressionSource, first_statement_line, parse_directives,
};
pub use violation::Violation;

use shuck_ast::{Script, TextSize};
use shuck_indexer::Indexer;
use shuck_semantic::{SemanticModel, TraversalObserver, build_with_observer};

pub struct AnalysisResult {
    pub semantic: SemanticModel,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Default)]
struct LintTraversalObserver {
    diagnostics: Vec<Diagnostic>,
}

impl LintTraversalObserver {
    fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

impl TraversalObserver for LintTraversalObserver {}

pub fn analyze_file(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> AnalysisResult {
    let mut observer = LintTraversalObserver::default();
    let mut semantic = build_with_observer(script, source, indexer, &mut observer);
    if settings.rules.contains(Rule::UnusedAssignment) {
        let _ = semantic.dataflow();
    }
    let shell = if settings.shell == ShellDialect::Unknown {
        ShellDialect::infer(source, None)
    } else {
        settings.shell
    };
    let checker = Checker::new(script, source, &semantic, indexer, &settings.rules, shell);
    let mut diagnostics = observer.into_diagnostics();
    diagnostics.extend(checker.check());
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
    AnalysisResult {
        semantic,
        diagnostics,
    }
}

pub fn lint_file(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> Vec<Diagnostic> {
    analyze_file(script, source, indexer, settings, suppression_index).diagnostics
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
        lint_file(&output.script, source, &indexer, settings, None)
    }

    #[test]
    fn default_settings_run_without_emitting_noop_diagnostics() {
        let diagnostics = lint("#!/bin/bash\necho ok\n", &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn analyze_file_returns_semantic_model_and_diagnostics() {
        let source = "#!/bin/bash\nvalue=ok\necho \"$value\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let result = analyze_file(
            &output.script,
            source,
            &indexer,
            &LinterSettings::default(),
            None,
        );

        assert!(result.diagnostics.is_empty());
        assert!(!result.semantic.scopes().is_empty());
        assert!(!result.semantic.bindings().is_empty());
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

    #[test]
    fn unused_assignment_flags_unread_variable() {
        let source = "#!/bin/sh\nfoo=1\n";
        let diagnostics = lint(source, &LinterSettings::default());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("foo"));
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn unused_assignment_reports_read_target_name_span() {
        let source = "#!/bin/sh\nread -r foo\n";
        let diagnostics = lint(source, &LinterSettings::default());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn unused_assignment_reports_getopts_target_name_span() {
        let source = "\
#!/bin/sh
while getopts \"ab\" opt; do
  :
done
";
        let diagnostics = lint(source, &LinterSettings::default());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "opt");
    }

    #[test]
    fn used_variable_produces_no_diagnostic() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\necho \"$foo\"\n",
            &LinterSettings::default(),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_at_script_scope_is_flagged() {
        let diagnostics = lint(
            "#!/bin/bash\nlocal foo=bar\nprintf '%s\\n' \"$foo\"\n",
            &LinterSettings::default(),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalTopLevel);
    }

    #[test]
    fn local_at_script_scope_in_sh_is_not_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nlocal foo=bar\nprintf '%s\\n' \"$foo\"\n",
            &LinterSettings::default(),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_inside_function_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo=bar
  printf '%s\\n' \"$foo\"
}
f
",
            &LinterSettings::default(),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn exported_variable_not_flagged() {
        let diagnostics = lint("#!/bin/sh\nexport FOO=1\n", &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn branch_assignments_followed_by_a_read_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
${code_command} --version
",
            &LinterSettings::default(),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn name_only_local_declaration_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo
  printf '%s\\n' \"$foo\"
}
f
",
            &LinterSettings::default(),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn initialized_local_declaration_is_flagged_when_unused() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo=1
}
f
",
            &LinterSettings::default(),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("foo"));
    }

    #[test]
    fn name_only_export_consumes_existing_assignment() {
        let diagnostics = lint("#!/bin/sh\nfoo=1\nexport foo\n", &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn name_only_readonly_consumes_existing_assignment() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\nreadonly foo\n",
            &LinterSettings::default(),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn corpus_false_negative_moduleselfname_is_now_flagged() {
        let diagnostics = lint(
            "#!/bin/bash\nmoduleselfname=\"$(basename \"$(readlink -f \"${BASH_SOURCE[0]}\")\")\"\n",
            &LinterSettings::default(),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("moduleselfname"));
    }

    #[test]
    fn global_assignment_used_in_a_function_body_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
red='\\e[31m'
print_red() { printf '%s\\n' \"$red\"; }
print_red
",
            &LinterSettings::default(),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_respects_disabled_rule() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\n",
            &LinterSettings {
                rules: RuleSet::EMPTY,
                ..LinterSettings::default()
            },
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2034
foo=1
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
        let diagnostics = lint_file(
            &output.script,
            source,
            &indexer,
            &LinterSettings::default(),
            Some(&suppressions),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_top_level_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/bash
# shellcheck disable=SC2168
local foo=bar
printf '%s\\n' \"$foo\"
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
        let diagnostics = lint_file(
            &output.script,
            source,
            &indexer,
            &LinterSettings::default(),
            Some(&suppressions),
        );
        assert!(diagnostics.is_empty());
    }
}
