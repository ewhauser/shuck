use std::path::Path;

use lsp_types as types;
use serde::{Deserialize, Serialize};
use shuck_indexer::LineIndex;
use shuck_linter::{
    Diagnostic as ShuckDiagnostic, Edit as ShuckEdit, Fix, LinterSettings, Severity,
    ShellCheckCodeMap, ShellDialect,
};
use shuck_parser::parser::Parser;

use crate::edit::LanguageId;
use crate::session::{DocumentQuery, DocumentSnapshot};
use crate::{DIAGNOSTIC_NAME, PositionEncoding};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticApplicability {
    Safe,
    Unsafe,
}

impl From<shuck_linter::Applicability> for DiagnosticApplicability {
    fn from(value: shuck_linter::Applicability) -> Self {
        match value {
            shuck_linter::Applicability::Safe => Self::Safe,
            shuck_linter::Applicability::Unsafe => Self::Unsafe,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct AssociatedDiagnosticData {
    title: String,
    code: String,
    edits: Vec<types::TextEdit>,
    directive_edit: Option<types::TextEdit>,
    applicability: DiagnosticApplicability,
}

pub fn generate_diagnostics(snapshot: &DocumentSnapshot) -> Vec<types::Diagnostic> {
    let query = snapshot.query();
    let source = query.document().contents();
    let path = query.file_path();
    let Some(shell) = infer_document_shell(query, source, path.as_deref()) else {
        return Vec::new();
    };

    let parse_result = Parser::with_dialect(source, shell.parser_dialect()).parse();
    let indexer = shuck_indexer::Indexer::new(source, &parse_result);
    let settings = LinterSettings::default().with_shell(shell);
    let diagnostics = shuck_linter::lint_file(
        &parse_result,
        source,
        &indexer,
        &settings,
        &ShellCheckCodeMap::default(),
        path.as_deref(),
    );

    diagnostics
        .into_iter()
        .map(|diagnostic| {
            to_lsp_diagnostic(
                diagnostic,
                source,
                query.document().index(),
                snapshot.encoding(),
            )
        })
        .collect()
}

fn to_lsp_diagnostic(
    diagnostic: ShuckDiagnostic,
    source: &str,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> types::Diagnostic {
    let code = diagnostic.code().to_owned();
    let data = associated_diagnostic_data(&diagnostic, source, line_index, encoding);

    types::Diagnostic {
        range: crate::edit::to_lsp_range(diagnostic.span.to_range(), source, line_index, encoding),
        severity: Some(diagnostic_severity(diagnostic.severity)),
        code: Some(types::NumberOrString::String(code)),
        code_description: None,
        source: Some(DIAGNOSTIC_NAME.into()),
        message: diagnostic.message,
        related_information: None,
        tags: None,
        data,
    }
}

fn associated_diagnostic_data(
    diagnostic: &ShuckDiagnostic,
    source: &str,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Option<serde_json::Value> {
    let edits = diagnostic
        .fix
        .as_ref()
        .into_iter()
        .flat_map(Fix::edits)
        .map(|edit| to_lsp_text_edit(edit, source, line_index, encoding))
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

    match serde_json::to_value(AssociatedDiagnosticData {
        title,
        code: diagnostic.code().to_owned(),
        edits,
        directive_edit: None,
        applicability,
    }) {
        Ok(data) => Some(data),
        Err(error) => {
            tracing::error!("failed to serialize associated diagnostic data: {error}");
            None
        }
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
    query: &DocumentQuery,
    source: &str,
    path: Option<&Path>,
) -> Option<ShellDialect> {
    if let Some(shell) = shell_from_language_id(query.language_id()) {
        return Some(shell);
    }

    let shell = ShellDialect::infer(source, path);
    (shell != ShellDialect::Unknown && path_supports_shell_inference(path)).then_some(shell)
}

fn shell_from_language_id(language_id: Option<LanguageId>) -> Option<ShellDialect> {
    match language_id {
        Some(LanguageId::Bash) => Some(ShellDialect::Bash),
        Some(LanguageId::Sh) => Some(ShellDialect::Sh),
        Some(LanguageId::Zsh) => Some(ShellDialect::Zsh),
        Some(LanguageId::Ksh) => Some(ShellDialect::Ksh),
        Some(LanguageId::Other) | None => None,
    }
}

fn path_supports_shell_inference(path: Option<&Path>) -> bool {
    path.and_then(|path| path.extension().and_then(|ext| ext.to_str()))
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "sh" | "bash" | "dash" | "ksh" | "mksh" | "zsh"
            )
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{ClientCapabilities, Url};

    use super::*;
    use crate::{Client, GlobalOptions, Session, TextDocument, Workspace, Workspaces};

    fn make_snapshot(
        path: &Path,
        source: &str,
        language_id: &str,
        encoding: PositionEncoding,
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
        assert_eq!(data.directive_edit, None);
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
        );

        assert!(generate_diagnostics(&snapshot).is_empty());
    }

    #[test]
    fn uses_utf16_ranges_for_diagnostics_and_fix_edits() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("utf16-range.sh"),
            "printf 'é'; foo=1\n",
            "shellscript",
            PositionEncoding::UTF16,
        );

        let diagnostics = generate_diagnostics(&snapshot);
        assert_eq!(diagnostics.len(), 1);

        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.range.start.line, 0);
        assert_eq!(diagnostic.range.start.character, 12);
        assert_eq!(diagnostic.range.end.character, 15);

        let data: AssociatedDiagnosticData = serde_json::from_value(
            diagnostic
                .data
                .clone()
                .expect("diagnostic payload should be serialized"),
        )
        .expect("diagnostic payload should deserialize");
        assert_eq!(data.edits[0].range.start.character, 12);
        assert_eq!(data.edits[0].range.end.character, 15);
    }

    #[test]
    fn infers_shell_from_shebang_when_path_has_no_extension() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("extensionless-script"),
            "#!/bin/sh\nfoo=1\n",
            "",
            PositionEncoding::UTF16,
        );

        let diagnostics = generate_diagnostics(&snapshot);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code,
            Some(types::NumberOrString::String("C001".to_owned()))
        );
    }
}
