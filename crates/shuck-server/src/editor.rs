use std::path::Path;

use shuck_indexer::Indexer;
use shuck_linter::ShellDialect;
use shuck_parser::{
    ShellProfile,
    parser::{ParseResult, Parser},
};
use shuck_semantic::{SemanticBuildOptions, SemanticModel};

pub(crate) struct ParsedEditorDocument {
    parse_result: ParseResult,
    shell_profile: ShellProfile,
    pub(crate) indexer: Indexer,
}

pub(crate) fn analyze_editor_document(
    source: &str,
    path: Option<&Path>,
    shell: ShellDialect,
) -> SemanticModel {
    let parsed = parse_editor_document(source, shell);
    semantic_for_parsed_document(&parsed, source, path)
}

pub(crate) fn parse_editor_document(source: &str, shell: ShellDialect) -> ParsedEditorDocument {
    let shell_profile = shell.shell_profile();
    let parse_result = Parser::with_profile(source, shell_profile.clone()).parse();
    let indexer = Indexer::new(source, &parse_result);
    ParsedEditorDocument {
        parse_result,
        shell_profile,
        indexer,
    }
}

pub(crate) fn semantic_for_parsed_document(
    parsed: &ParsedEditorDocument,
    source: &str,
    path: Option<&Path>,
) -> SemanticModel {
    SemanticModel::build_with_options(
        &parsed.parse_result.file,
        source,
        &parsed.indexer,
        SemanticBuildOptions {
            source_path: path,
            shell_profile: Some(parsed.shell_profile.clone()),
            resolve_source_closure: false,
            ..SemanticBuildOptions::default()
        },
    )
}
