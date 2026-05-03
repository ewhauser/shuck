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
    if let Some(shell) = infer_source_declared_shell(source) {
        return Some(shell);
    }

    let shell = ShellDialect::infer(source, path);

    match language_id_preference(query.language_id()) {
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

    #[test]
    fn generic_shell_language_id_allows_shebang_override() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("generic-shell-script"),
            "#!/bin/bash\nfoo=1\n",
            "shellscript",
            PositionEncoding::UTF16,
        );

        assert_eq!(
            infer_document_shell(
                snapshot.query(),
                snapshot.query().document().contents(),
                snapshot.query().file_path().as_deref(),
            ),
            Some(ShellDialect::Bash)
        );
    }

    #[test]
    fn shell_shebang_still_lints_on_custom_extension() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("script.custom"),
            "#!/bin/bash\nfoo=1\n",
            "",
            PositionEncoding::UTF16,
        );

        assert_eq!(
            infer_document_shell(
                snapshot.query(),
                snapshot.query().document().contents(),
                snapshot.query().file_path().as_deref(),
            ),
            Some(ShellDialect::Bash)
        );
        assert_eq!(generate_diagnostics(&snapshot).len(), 1);
    }

    #[test]
    fn source_declared_shell_overrides_concrete_language_id() {
        let snapshot = make_snapshot(
            &std::env::temp_dir().join("header-override.sh"),
            "# shellcheck shell=bash\nfoo=1\n",
            "sh",
            PositionEncoding::UTF16,
        );

        assert_eq!(
            infer_document_shell(
                snapshot.query(),
                snapshot.query().document().contents(),
                snapshot.query().file_path().as_deref(),
            ),
            Some(ShellDialect::Bash)
        );
    }
}
