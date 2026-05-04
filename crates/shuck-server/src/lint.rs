use std::path::Path;

use lsp_types as types;
use serde::{Deserialize, Serialize};
use shuck_indexer::LineIndex;
use shuck_linter::{
    Applicability, Diagnostic as ShuckDiagnostic, Edit as ShuckEdit, Fix, Severity,
    ShellCheckCodeMap, ShellDialect,
};
use shuck_parser::parser::Parser;

use crate::edit::{LanguageId, RangeExt};
use crate::session::DocumentSnapshot;
use crate::{DIAGNOSTIC_NAME, PositionEncoding};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticApplicability {
    Safe,
    Unsafe,
}

impl From<Applicability> for DiagnosticApplicability {
    fn from(value: Applicability) -> Self {
        match value {
            Applicability::Safe => Self::Safe,
            Applicability::Unsafe => Self::Unsafe,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssociatedDiagnosticData {
    pub(crate) title: String,
    pub(crate) code: String,
    pub(crate) edits: Vec<types::TextEdit>,
    pub(crate) directive_edit: Option<types::TextEdit>,
    pub(crate) applicability: DiagnosticApplicability,
}

pub(crate) struct RawDocumentDiagnostics {
    pub(crate) shell_diagnostics: Vec<ShuckDiagnostic>,
    pub(crate) parse_error: Option<ParseErrorDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseErrorDiagnostic {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) message: String,
}

pub fn generate_diagnostics(snapshot: &DocumentSnapshot) -> Vec<types::Diagnostic> {
    let query = snapshot.query();
    let source = query.document().contents();
    let path = query.file_path();
    let Some(shell) = infer_document_shell(snapshot, source, path.as_deref()) else {
        return Vec::new();
    };

    let raw = collect_raw_diagnostics(snapshot, shell, source, path.as_deref());
    let mut diagnostics = raw
        .shell_diagnostics
        .into_iter()
        .map(|diagnostic| {
            to_lsp_diagnostic(
                snapshot,
                &diagnostic,
                source,
                query.document().index(),
                path.as_deref(),
            )
        })
        .collect::<Vec<_>>();

    if snapshot.client_settings().show_syntax_errors() && let Some(parse_error) = raw.parse_error {
        diagnostics.insert(
            0,
            parse_error_to_lsp(snapshot, source, query.document().index(), parse_error),
        );
    }

    diagnostics
}

pub(crate) fn collect_raw_diagnostics_for_snapshot(
    snapshot: &DocumentSnapshot,
) -> Option<RawDocumentDiagnostics> {
    let query = snapshot.query();
    let source = query.document().contents();
    let path = query.file_path();
    document_shell(snapshot)
        .map(|shell| collect_raw_diagnostics(snapshot, shell, source, path.as_deref()))
}

pub(crate) fn document_shell(snapshot: &DocumentSnapshot) -> Option<ShellDialect> {
    let query = snapshot.query();
    let source = query.document().contents();
    let path = query.file_path();
    infer_document_shell(snapshot, source, path.as_deref())
}

pub(crate) fn fix_all_document_edits(
    snapshot: &DocumentSnapshot,
    applicability: Applicability,
) -> Vec<types::TextEdit> {
    let Some(raw) = collect_raw_diagnostics_for_snapshot(snapshot) else {
        return Vec::new();
    };

    let source = snapshot.query().document().contents();
    let applied = shuck_linter::apply_fixes(source, &raw.shell_diagnostics, applicability);
    if applied.fixes_applied == 0 || applied.code == source {
        return Vec::new();
    }

    crate::edit::single_replacement_edit(
        source,
        &applied.code,
        snapshot.query().document().index(),
        snapshot.encoding(),
    )
    .into_iter()
    .collect()
}

pub(crate) fn directive_edit_for_line(
    snapshot: &DocumentSnapshot,
    line: usize,
) -> Option<types::TextEdit> {
    let query = snapshot.query();
    let source = query.document().contents();
    let path = query.file_path();
    let edit = shuck_linter::build_ignore_edit_for_line(
        source,
        snapshot.shuck_settings().linter(),
        line,
        None,
        path.as_deref(),
    )?;
    Some(to_lsp_text_edit(
        &edit,
        source,
        query.document().index(),
        snapshot.encoding(),
    ))
}

pub(crate) fn associated_diagnostic_data(
    _snapshot: &DocumentSnapshot,
    diagnostic: &types::Diagnostic,
) -> Option<AssociatedDiagnosticData> {
    diagnostic
        .data
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn collect_raw_diagnostics(
    snapshot: &DocumentSnapshot,
    shell: ShellDialect,
    source: &str,
    path: Option<&Path>,
) -> RawDocumentDiagnostics {
    let parse_result = Parser::with_profile(source, shell.shell_profile()).parse();
    let indexer = shuck_indexer::Indexer::new(source, &parse_result);
    let shellcheck_map = ShellCheckCodeMap::default();
    let shell_diagnostics = shuck_linter::lint_file(
        &parse_result,
        source,
        &indexer,
        snapshot.shuck_settings().linter(),
        &shellcheck_map,
        path,
    );
    let parse_error = parse_result.is_err().then(|| {
        let shuck_parser::Error::Parse {
            message,
            line,
            column,
        } = parse_result.strict_error();
        ParseErrorDiagnostic {
            line,
            column,
            message: message.clone(),
        }
    });

    RawDocumentDiagnostics {
        shell_diagnostics,
        parse_error,
    }
}

fn to_lsp_diagnostic(
    snapshot: &DocumentSnapshot,
    diagnostic: &ShuckDiagnostic,
    source: &str,
    line_index: &LineIndex,
    path: Option<&Path>,
) -> types::Diagnostic {
    let code = diagnostic.code().to_owned();
    let data = associated_diagnostic_data_for_shuck(snapshot, diagnostic, source, line_index, path);

    types::Diagnostic {
        range: crate::edit::to_lsp_range(
            diagnostic.span.to_range(),
            source,
            line_index,
            snapshot.encoding(),
        ),
        severity: Some(diagnostic_severity(diagnostic.severity)),
        code: Some(types::NumberOrString::String(code)),
        code_description: None,
        source: Some(DIAGNOSTIC_NAME.into()),
        message: diagnostic.message.clone(),
        related_information: None,
        tags: None,
        data,
    }
}

fn associated_diagnostic_data_for_shuck(
    snapshot: &DocumentSnapshot,
    diagnostic: &ShuckDiagnostic,
    source: &str,
    line_index: &LineIndex,
    path: Option<&Path>,
) -> Option<serde_json::Value> {
    let edits = diagnostic
        .fix
        .as_ref()
        .into_iter()
        .flat_map(Fix::edits)
        .map(|edit| to_lsp_text_edit(edit, source, line_index, snapshot.encoding()))
        .collect();
    let applicability = diagnostic
        .fix
        .as_ref()
        .map_or(DiagnosticApplicability::Safe, |fix| {
            fix.applicability().into()
        });
    let title = diagnostic
        .fix_title
        .clone()
        .unwrap_or_else(|| diagnostic.message.clone());
    let directive_edit = shuck_linter::build_ignore_edit_for_line(
        source,
        snapshot.shuck_settings().linter(),
        diagnostic.span.start.line,
        None,
        path,
    )
    .map(|edit| to_lsp_text_edit(&edit, source, line_index, snapshot.encoding()));

    match serde_json::to_value(AssociatedDiagnosticData {
        title,
        code: diagnostic.code().to_owned(),
        edits,
        directive_edit,
        applicability,
    }) {
        Ok(data) => Some(data),
        Err(error) => {
            tracing::error!("failed to serialize associated diagnostic data: {error}");
            None
        }
    }
}

fn parse_error_to_lsp(
    snapshot: &DocumentSnapshot,
    source: &str,
    line_index: &LineIndex,
    parse_error: ParseErrorDiagnostic,
) -> types::Diagnostic {
    let line = parse_error.line.saturating_sub(1) as u32;
    let character = parse_error.column.saturating_sub(1) as u32;
    let start = types::Position::new(line, character);
    let end = types::Position::new(line, character);
    let range = types::Range { start, end };
    let adjusted_range = range.to_text_range(source, line_index, snapshot.encoding());

    types::Diagnostic {
        range: crate::edit::to_lsp_range(adjusted_range, source, line_index, snapshot.encoding()),
        severity: Some(types::DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some(DIAGNOSTIC_NAME.into()),
        message: format!("parse error {}", parse_error.message),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn to_lsp_text_edit(
    edit: &ShuckEdit,
    source: &str,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> types::TextEdit {
    types::TextEdit {
        range: crate::edit::to_lsp_range(edit.range(), source, line_index, encoding),
        new_text: edit.content().to_owned(),
    }
}

fn diagnostic_severity(severity: Severity) -> types::DiagnosticSeverity {
    match severity {
        Severity::Hint => types::DiagnosticSeverity::HINT,
        Severity::Warning => types::DiagnosticSeverity::WARNING,
        Severity::Error => types::DiagnosticSeverity::ERROR,
    }
}

fn infer_document_shell(
    snapshot: &DocumentSnapshot,
    query_source: &str,
    path: Option<&Path>,
) -> Option<ShellDialect> {
    if snapshot.shuck_settings().linter().shell != ShellDialect::Unknown {
        return Some(snapshot.shuck_settings().linter().shell);
    }

    if let Some(shell) = infer_source_declared_shell(query_source) {
        return Some(shell);
    }

    let shell = ShellDialect::infer(query_source, path);

    match language_id_preference(snapshot.query().language_id()) {
        LanguageIdPreference::Concrete(shell) => Some(shell),
        LanguageIdPreference::GenericShell => Some(match shell {
            ShellDialect::Unknown => ShellDialect::Sh,
            shell => shell,
        }),
        LanguageIdPreference::Unknown => (shell != ShellDialect::Unknown).then_some(shell),
    }
}

fn infer_source_declared_shell(source: &str) -> Option<ShellDialect> {
    infer_shellcheck_header(source).or_else(|| infer_shebang_shell(source))
}

fn infer_shebang_shell(source: &str) -> Option<ShellDialect> {
    let interpreter = shuck_parser::shebang::interpreter_name(source.lines().next()?)?;
    let shell = ShellDialect::from_name(interpreter);
    (shell != ShellDialect::Unknown).then_some(shell)
}

fn infer_shellcheck_header(source: &str) -> Option<ShellDialect> {
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }

        let Some(comment) = trimmed.strip_prefix('#') else {
            break;
        };
        let body = comment.trim_start().to_ascii_lowercase();
        let Some(shell_name) = body.strip_prefix("shellcheck shell=") else {
            continue;
        };

        let shell =
            ShellDialect::from_name(shell_name.split_whitespace().next().unwrap_or_default());
        if shell != ShellDialect::Unknown {
            return Some(shell);
        }
    }

    None
}

enum LanguageIdPreference {
    Concrete(ShellDialect),
    GenericShell,
    Unknown,
}

fn language_id_preference(language_id: Option<LanguageId>) -> LanguageIdPreference {
    match language_id {
        Some(LanguageId::Bash) => LanguageIdPreference::Concrete(ShellDialect::Bash),
        Some(LanguageId::Sh) => LanguageIdPreference::Concrete(ShellDialect::Sh),
        Some(LanguageId::Zsh) => LanguageIdPreference::Concrete(ShellDialect::Zsh),
        Some(LanguageId::Ksh) => LanguageIdPreference::Concrete(ShellDialect::Ksh),
        Some(LanguageId::ShellScript) => LanguageIdPreference::GenericShell,
        Some(LanguageId::Other) | None => LanguageIdPreference::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{ClientCapabilities, Url};

    use super::*;
    use crate::{
        Client, ClientOptions, GlobalOptions, Session, TextDocument, Workspace, Workspaces,
    };

    fn make_snapshot(
        path: &Path,
        source: &str,
        language_id: &str,
        encoding: PositionEncoding,
        settings: ClientOptions,
    ) -> DocumentSnapshot {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspaces = Workspaces::new(vec![Workspace::default(
            Url::from_file_path(std::env::temp_dir())
                .expect("temporary directory should convert to a file URL"),
        )]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            encoding,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(path).expect("test path should convert to a file URL");
        session.update_client_options(settings);
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id(language_id),
        );

        session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot")
    }

    #[test]
    fn reports_native_shuck_diagnostic_with_fix_data() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("unused-assignment.sh"),
            "foo=1\n",
            "shellscript",
            PositionEncoding::UTF16,
            ClientOptions::default(),
        );

        let diagnostics = generate_diagnostics(&snapshot);
        assert_eq!(diagnostics.len(), 1);

        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.source.as_deref(), Some(DIAGNOSTIC_NAME));
        assert_eq!(
            diagnostic.code,
            Some(types::NumberOrString::String("C001".to_owned()))
        );

        let data: AssociatedDiagnosticData = serde_json::from_value(
            diagnostic
                .data
                .clone()
                .expect("diagnostic payload should be serialized"),
        )
        .expect("diagnostic payload should deserialize");
        assert_eq!(data.title, "rename the unused assignment target to `_`");
        assert_eq!(data.code, "C001");
        assert!(data.directive_edit.is_some());
        assert_eq!(data.applicability, DiagnosticApplicability::Unsafe);
        assert_eq!(data.edits.len(), 1);
        assert_eq!(data.edits[0].new_text, "_");
        assert_eq!(data.edits[0].range.start.line, 0);
        assert_eq!(data.edits[0].range.start.character, 0);
        assert_eq!(data.edits[0].range.end.character, 3);
    }

    #[test]
    fn skips_non_shell_documents() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("README.md"),
            "# Heading\n",
            "markdown",
            PositionEncoding::UTF16,
            ClientOptions::default(),
        );

        assert!(generate_diagnostics(&snapshot).is_empty());
    }

    #[test]
    fn surfaces_parse_errors_when_requested() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("parse-error.sh"),
            "if true\n",
            "shellscript",
            PositionEncoding::UTF16,
            ClientOptions {
                show_syntax_errors: Some(true),
                ..ClientOptions::default()
            },
        );

        let diagnostics = generate_diagnostics(&snapshot);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.message.contains("parse error")));
    }

    #[test]
    fn uses_utf16_ranges_for_diagnostics_and_fix_edits() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("emoji.sh"),
            "printf '🙂'\nfoo=1\n",
            "shellscript",
            PositionEncoding::UTF16,
            ClientOptions::default(),
        );

        let diagnostics = generate_diagnostics(&snapshot);
        let data: AssociatedDiagnosticData = serde_json::from_value(
            diagnostics[0]
                .data
                .clone()
                .expect("diagnostic payload should serialize"),
        )
        .expect("diagnostic payload should deserialize");
        assert_eq!(data.edits[0].range.start.line, 1);
        assert_eq!(data.edits[0].range.start.character, 0);
    }
}
