mod checker;
pub mod context;
mod diagnostic;
mod facts;
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
pub use context::{
    ContextRegion, ContextRegionKind, FileContext, FileContextTag, classify_file_context,
};
pub use diagnostic::{Diagnostic, Severity};
pub use facts::{
    CommandFact, CommandOptionFacts, ConditionalBareWordFact, ConditionalBinaryFact,
    ConditionalFact, ConditionalNodeFact, ConditionalOperandFact, ConditionalOperatorFamily,
    ConditionalUnaryFact, ExitCommandFacts, FactSpan, FindCommandFacts, ForHeaderFact, LinterFacts,
    ListFact, ListOperatorFact, LoopHeaderWordFact, PipelineFact, PipelineSegmentFact,
    PrintfCommandFacts, ReadCommandFacts, SelectHeaderFact, SimpleTestFact,
    SimpleTestOperatorFamily, SimpleTestShape, SimpleTestSyntax, SudoFamilyCommandFacts,
    SudoFamilyInvoker, UnsetCommandFacts, XargsCommandFacts,
};
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

use shuck_ast::{File, TextSize};
use shuck_indexer::Indexer;
use shuck_semantic::{
    SemanticModel, SourcePathResolver, TraversalObserver, build_with_observer,
    build_with_observer_at_path_with_resolver,
};
use std::path::Path;

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
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> AnalysisResult {
    analyze_file_at_path(file, source, indexer, settings, suppression_index, None)
}

pub fn analyze_file_at_path(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
) -> AnalysisResult {
    analyze_file_at_path_with_resolver(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        None,
    )
}

pub fn analyze_file_at_path_with_resolver(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> AnalysisResult {
    let mut observer = LintTraversalObserver::default();
    let mut semantic = if source_path.is_some() {
        build_with_observer_at_path_with_resolver(
            file,
            source,
            indexer,
            &mut observer,
            source_path,
            source_path_resolver,
        )
    } else {
        build_with_observer(file, source, indexer, &mut observer)
    };
    if settings.rules.contains(Rule::UnusedAssignment) {
        let _ = semantic.precompute_unused_assignments();
    }
    if settings.rules.contains(Rule::UndefinedVariable) {
        let _ = semantic.precompute_uninitialized_references();
    }
    if settings.rules.contains(Rule::UnreachableAfterExit) {
        let _ = semantic.precompute_dead_code();
    }
    let shell = if settings.shell == ShellDialect::Unknown {
        ShellDialect::infer(source, source_path)
    } else {
        settings.shell
    };
    let file_context = classify_file_context(source, source_path, shell);
    let checker = Checker::new(
        file,
        source,
        &semantic,
        indexer,
        &settings.rules,
        shell,
        &file_context,
    );
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
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> Vec<Diagnostic> {
    lint_file_at_path(file, source, indexer, settings, suppression_index, None)
}

pub fn lint_file_at_path(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
) -> Vec<Diagnostic> {
    lint_file_at_path_with_resolver(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        None,
    )
}

pub fn lint_file_at_path_with_resolver(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<Diagnostic> {
    analyze_file_at_path_with_resolver(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        source_path_resolver,
    )
    .diagnostics
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
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn lint(source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        lint_file(&output.file, source, &indexer, settings, None)
    }

    fn lint_path(path: &Path, settings: &LinterSettings) -> Vec<Diagnostic> {
        let source = fs::read_to_string(path).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        lint_file_at_path(&output.file, &source, &indexer, settings, None, Some(path))
    }

    fn lint_for_rule(source: &str, rule: Rule) -> Vec<Diagnostic> {
        lint(source, &LinterSettings::for_rule(rule))
    }

    fn lint_path_for_rule(path: &Path, rule: Rule) -> Vec<Diagnostic> {
        lint_path(path, &LinterSettings::for_rule(rule))
    }

    fn lint_named_source(path: &Path, source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        lint_file_at_path(&output.file, source, &indexer, settings, None, Some(path))
    }

    fn runtime_prelude_source(shebang: &str) -> String {
        format!(
            "{shebang}\nprintf '%s\\n' \"$IFS\" \"$USER\" \"$HOME\" \"$SHELL\" \"$PWD\" \"$TERM\" \"$LANG\" \"$SUDO_USER\" \"$DOAS_USER\"\nprintf '%s\\n' \"$LINENO\" \"$FUNCNAME\" \"${{BASH_SOURCE[0]}}\" \"${{BASH_LINENO[0]}}\" \"$RANDOM\" \"${{BASH_REMATCH[0]}}\" \"$READLINE_LINE\" \"$BASH_VERSION\" \"${{BASH_VERSINFO[0]}}\" \"$OSTYPE\" \"$HISTCONTROL\" \"$HISTSIZE\"\n"
        )
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
            &output.file,
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
    fn path_sensitive_context_classification_uses_the_supplied_path() {
        let shellspec_path = Path::new("/tmp/project/spec/clone_spec.sh");
        let source = "\
Describe 'clone'
Parameters
  \"test\"
End
";
        let diagnostics = lint_named_source(
            shellspec_path,
            source,
            &LinterSettings::for_rule(Rule::EmptyTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn shell_inference_uses_path_when_shebang_is_missing() {
        let source = "local value=ok\n";
        let diagnostics = lint_named_source(
            Path::new("/tmp/example.bash"),
            source,
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalTopLevel);
    }

    #[test]
    fn helper_library_context_uses_path_tokens() {
        let context = classify_file_context(
            "helper() { :; }\n",
            Some(Path::new("/tmp/repo/libexec/plugins/tool.func")),
            ShellDialect::Sh,
        );

        assert!(context.has_tag(FileContextTag::HelperLibrary));
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
            &output.file,
            first_statement_line(&output.file).unwrap_or(u32::MAX),
        );

        let echo_foo = match &output.file.body[1].command {
            Command::Simple(command) => command.span,
            other => panic!("expected simple command, got {other:?}"),
        };
        let echo_bar = match &output.file.body[2].command {
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
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("foo"));
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn unused_assignment_reports_read_target_name_span() {
        let source = "#!/bin/sh\nread -r foo\n";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

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
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "opt");
    }

    #[test]
    fn read_header_bindings_used_in_loop_body_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
printf '%s\n' 'service safe ok yes' | while read UNIT EXPOSURE PREDICATE HAPPY; do
  printf '%s %s %s %s\n' \"$UNIT\" \"$EXPOSURE\" \"$PREDICATE\" \"$HAPPY\"
done
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn command_prefix_environment_assignment_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
CFLAGS=\"$SLKCFLAGS\" make
DESTDIR=\"$pkgdir\" install
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn indirect_expansion_keeps_dynamic_target_arrays_live() {
        let diagnostics = lint(
            "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn array_append_used_by_later_expansion_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
arr=(--first)
arr+=(--second)
printf '%s\\n' \"${arr[@]}\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_append_assignment_is_not_flagged() {
        let diagnostics = lint_for_rule("#!/bin/bash\nfoo+=bar\n", Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn later_defined_helper_assignment_to_caller_local_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
main() {
  local status=''
  helper
  printf '%s\\n' \"$status\"
}
helper() {
  status=ok
}
main
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn later_defined_helper_array_append_to_caller_local_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
main() {
  local errors=()
  helper
  printf '%s\\n' \"${errors[@]}\"
}
helper() {
  errors+=(oops)
}
main
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn read_implicitly_consumes_ifs_but_still_flags_unrelated_local() {
        let source = "\
#!/bin/bash
f() {
  local IFS=$'\\n'
  local unused=1
  read -d '' -ra reply < <(printf 'alpha\\nbeta\\0')
  printf '%s\\n' \"${reply[@]}\"
}
f
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn global_ifs_assignment_is_not_flagged_but_unrelated_assignment_is() {
        let source = "\
#!/bin/bash
IFS=$'\\n\\t'
unused=1
echo ok
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn unrelated_array_assignment_is_still_flagged_with_indirect_expansion() {
        let source = "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused_args");
    }

    #[test]
    fn used_variable_produces_no_diagnostic() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\necho \"$foo\"\n",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_at_script_scope_is_flagged() {
        let diagnostics = lint(
            "#!/bin/bash\nlocal foo=bar\nprintf '%s\\n' \"$foo\"\n",
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalTopLevel);
    }

    #[test]
    fn local_at_script_scope_in_sh_is_not_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nlocal foo=bar\nprintf '%s\\n' \"$foo\"\n",
            &LinterSettings::for_rule(Rule::LocalTopLevel),
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
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn exported_variable_not_flagged() {
        let diagnostics = lint_for_rule("#!/bin/sh\nexport FOO=1\n", Rule::UnusedAssignment);
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
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn mutually_exclusive_unused_branch_assignments_report_one_diagnostic() {
        let source = "\
#!/bin/sh
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 5);
    }

    #[test]
    fn partially_used_branch_assignments_still_report_each_dead_arm() {
        let source = "\
#!/bin/sh
if a; then
  VAR=1
elif b; then
  VAR=2
else
  VAR=3
  echo \"$VAR\"
fi
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.line, 5);
    }

    #[test]
    fn case_branch_assignments_used_in_function_body_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
case \"$arch\" in
amd64 | x86_64)
  jq_arch=amd64
  core_arch=64
  ;;
arm64 | aarch64)
  jq_arch=arm64
  core_arch=arm64-v8a
  ;;
esac
download() {
  echo \"$jq_arch\"
  echo \"$core_arch\"
}
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_global_assignments_read_later_by_caller_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
pass_args() {
  local_install=1
  proxy=$1
}
main() {
  pass_args \"$@\"
  printf '%s %s\\n' \"$local_install\" \"$proxy\"
}
main \"$@\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn recursive_function_state_assignment_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
check_status() {
  if [[ $is_wget ]]; then
    printf '%s\\n' ok
  else
    is_wget=1
    check_status
  fi
}
check_status
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_function_global_assignment_is_still_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
f() {
  foo=1
}
f
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(
            diagnostics[0]
                .span
                .slice("#!/bin/sh\nf() {\n  foo=1\n}\nf\n"),
            "foo"
        );
    }

    #[test]
    fn name_only_local_declaration_read_is_reported_as_uninitialized() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
f() {
  local foo
  printf '%s\\n' \"$foo\"
}
f
",
            Rule::UndefinedVariable,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("foo"));
    }

    #[test]
    fn resolved_indirect_expansion_carrier_is_not_reported_as_uninitialized() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
f() {
  local foo
  printf '%s\\n' \"${!foo}\"
}
f
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn indirect_reads_do_not_report_missing_targets_for_indirect_or_nameref_access() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
name=missing
declare -n ref=missing
printf '%s %s\\n' \"${!name}\" \"$ref\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unresolved_indirect_expansion_carrier_is_still_reported() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
printf '%s\\n' \"${!foo}\"
",
            Rule::UndefinedVariable,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("foo"));
    }

    #[test]
    fn undefined_variable_reports_definite_and_possible_reads() {
        let source = "\
#!/bin/bash
echo \"$missing\"
if true; then
  maybe=1
fi
echo \"$maybe\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("missing"));
        assert!(
            diagnostics[0]
                .message
                .contains("referenced before assignment")
        );
        assert_eq!(diagnostics[1].rule, Rule::UndefinedVariable);
        assert!(diagnostics[1].message.contains("maybe"));
        assert!(
            diagnostics[1]
                .message
                .contains("may be referenced before assignment")
        );
    }

    #[test]
    fn undefined_variable_ignores_declaration_names_and_special_parameters() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
readonly declared
export exported
printf '%s %s %s\\n' \"$1\" \"$@\" \"$#\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn undefined_variable_ignores_bash_runtime_vars_in_bash_scripts() {
        let source = runtime_prelude_source("#!/bin/bash");
        let diagnostics = lint_for_rule(&source, Rule::UndefinedVariable);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn undefined_variable_still_reports_bash_runtime_vars_in_sh_scripts() {
        let source = runtime_prelude_source("#!/bin/sh");
        let diagnostics = lint_for_rule(&source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 12);
        for name in [
            "LINENO",
            "FUNCNAME",
            "BASH_SOURCE",
            "BASH_LINENO",
            "RANDOM",
            "BASH_REMATCH",
            "READLINE_LINE",
            "BASH_VERSION",
            "BASH_VERSINFO",
            "OSTYPE",
            "HISTCONTROL",
            "HISTSIZE",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(name)),
                "missing diagnostic for {name}"
            );
        }
    }

    #[test]
    fn unread_name_only_declarations_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo
  declare bar
  typeset baz
}
f
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
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
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("foo"));
    }

    #[test]
    fn name_only_export_consumes_existing_assignment() {
        let diagnostics = lint_for_rule("#!/bin/sh\nfoo=1\nexport foo\n", Rule::UnusedAssignment);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn name_only_readonly_consumes_existing_assignment() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\nreadonly foo\n",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn corpus_false_negative_moduleselfname_is_now_flagged() {
        let diagnostics = lint(
            "#!/bin/bash\nmoduleselfname=\"$(basename \"$(readlink -f \"${BASH_SOURCE[0]}\")\")\"\n",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
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
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn top_level_assignment_read_by_later_function_call_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
show() { echo \"$flag\"; }
flag=1
show
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn sourced_helper_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
. ./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn source_builtin_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./helper.bash
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_scalar_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_index_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[0]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_double_zero_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[00]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_spaced_zero_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[ 0 ]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_nonzero_suffix_source_does_not_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[1]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn bash_source_scalar_dirname_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_index_dirname_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"$(dirname \"${BASH_SOURCE[0]}\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn executed_helper_reads_keep_loop_variable_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
for queryip in 127.0.0.1; do
  helper.sh
done
",
        )
        .unwrap();
        fs::write(&helper, "printf '%s\\n' \"$queryip\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn executed_helper_without_read_still_flags_unused_assignment() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/sh
unused=1
helper.sh
";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "printf '%s\\n' ok\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn loader_function_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
load() { . \"$ROOT/$1\"; }
flag=1
load helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_reports_each_unreachable_command() {
        let source = "\
#!/bin/bash
if [ -f /etc/hosts ]; then
  echo found
  exit 0
else
  echo missing
  exit 1
fi
echo unreachable
printf '%s\\n' never
f() {
  return 0
  echo also_unreachable
}
f
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 5);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule == Rule::UnreachableAfterExit)
        );
        assert_eq!(
            diagnostics[0].span.slice(source).trim_end(),
            "echo unreachable"
        );
        assert_eq!(
            diagnostics[1].span.slice(source).trim_end(),
            "printf '%s\\n' never"
        );
        assert!(
            diagnostics[2]
                .span
                .slice(source)
                .trim_end()
                .starts_with("f() {")
        );
        assert_eq!(
            diagnostics[3].span.slice(source).trim_end(),
            "echo also_unreachable"
        );
        assert_eq!(diagnostics[4].span.slice(source).trim_end(), "f");
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
            &output.file,
            first_statement_line(&output.file).unwrap_or(u32::MAX),
        );
        let diagnostics = lint_file(
            &output.file,
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
            &output.file,
            first_statement_line(&output.file).unwrap_or(u32::MAX),
        );
        let diagnostics = lint_file(
            &output.file,
            source,
            &indexer,
            &LinterSettings::default(),
            Some(&suppressions),
        );
        assert!(diagnostics.is_empty());
    }
}
