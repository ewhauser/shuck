use std::path::{Path, PathBuf};

use shuck_linter::{
    AnalysisRequest, Diagnostic, FileContract, LinterSettings, PluginFramework, PluginRequest,
    PluginRequestKind, PluginResolution, PluginResolver, Rule, Severity, SourcePathResolver,
};
use shuck_parser::parser::{ParseResult, ParseStatus, Parser};

struct EmptyPluginResolver;

impl PluginResolver for EmptyPluginResolver {
    fn resolve_plugin_request(
        &self,
        _source_path: &Path,
        _request: &PluginRequest,
    ) -> PluginResolution {
        PluginResolution::default()
    }
}

fn accepts_source_path_resolver(_resolver: &(dyn SourcePathResolver + Send + Sync)) {}

fn inspect_parse_result(result: &ParseResult) -> (&shuck_ast::File, usize, ParseStatus) {
    (&result.file, result.diagnostics.len(), result.status)
}

fn inspect_diagnostic(diagnostic: &Diagnostic) -> (&str, Rule, Severity) {
    (&diagnostic.message, diagnostic.rule, diagnostic.severity)
}

fn is_selected_rule(rule: Rule) -> bool {
    matches!(rule, Rule::UnusedAssignment | Rule::UndefinedVariable)
}

#[test]
fn downstream_construction_and_inspection_path_stays_ergonomic() {
    let source = "echo hello\n";
    let parsed = Parser::new(source).parse();
    let (_, diagnostic_count, status) = inspect_parse_result(&parsed);
    assert_eq!(status, ParseStatus::Clean);
    assert_eq!(diagnostic_count, 0);

    let mut settings = LinterSettings::for_rules([Rule::UnusedAssignment]);
    settings
        .severity_overrides
        .insert(Rule::UnusedAssignment, Severity::Hint);
    settings.report_environment_style_names = true;

    let source_path_resolver = |_: &Path, _: &str| Vec::<PathBuf>::new();
    accepts_source_path_resolver(&source_path_resolver);
    let plugin_resolver = EmptyPluginResolver;
    let analysis = AnalysisRequest::from_parse_result(&parsed, source, &settings)
        .with_source_path_resolver(&source_path_resolver)
        .with_plugin_resolver(&plugin_resolver)
        .analyze();
    let _semantic = &analysis.semantic;
    for diagnostic in &analysis.diagnostics {
        let _ = inspect_diagnostic(diagnostic);
    }

    assert!(is_selected_rule(Rule::UnusedAssignment));
    let _resolver_companion_types = (
        PluginFramework::Other("custom".to_owned()),
        PluginRequestKind::Plugin,
        FileContract::default(),
    );
}
