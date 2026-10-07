use anyhow::Error;
use lsp_types as types;
use shuck_ast::TextRange;
use shuck_formatter::FormattedSource;

use crate::edit::RangeExt;
use crate::session::{Client, DocumentSnapshot};

pub(crate) type FormatResponse = Option<Vec<types::TextEdit>>;

pub(crate) fn format_document(
    snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentFormattingParams,
) -> crate::server::Result<FormatResponse> {
    let query = snapshot.query();
    let file_path = query.file_path();
    if snapshot
        .shuck_settings()
        .is_format_excluded(file_path.as_deref())
    {
        return Ok(None);
    }
    if let Some(error) = snapshot.shuck_settings().format_dialect_error() {
        return Err(Error::msg(error.to_owned()).into());
    }
    let source = query.document().contents();
    let formatted = shuck_formatter::format_source(
        source,
        file_path.as_deref(),
        snapshot.shuck_settings().formatter(),
    )
    .map_err(Error::new)?;

    Ok(Some(match formatted {
        FormattedSource::Unchanged => Vec::new(),
        FormattedSource::Formatted(code) => crate::edit::single_replacement_edit(
            source,
            &code,
            query.document().index(),
            snapshot.encoding(),
        )
        .into_iter()
        .collect(),
    }))
}

pub(crate) fn format_range(
    snapshot: DocumentSnapshot,
    _client: &Client,
    params: types::DocumentRangeFormattingParams,
) -> crate::server::Result<FormatResponse> {
    let file_path = snapshot.query().file_path();
    if snapshot
        .shuck_settings()
        .is_format_excluded(file_path.as_deref())
    {
        return Ok(None);
    }
    if let Some(error) = snapshot.shuck_settings().format_dialect_error() {
        return Err(Error::msg(error.to_owned()).into());
    }
    let Some(analysis) = snapshot.analysis() else {
        return Ok(None);
    };
    let source = analysis.source();
    let requested = params
        .range
        .to_text_range(source, analysis.line_index(), snapshot.encoding());
    let Some(formatted_range) = shuck_formatter::format_range(
        source,
        requested,
        file_path.as_deref(),
        snapshot.shuck_settings().formatter(),
    )
    .map_err(Error::new)?
    else {
        return Ok(None);
    };

    Ok(Some(format_response_for_range(
        source,
        formatted_range.range,
        formatted_range.formatted,
        analysis.line_index(),
        snapshot.encoding(),
    )))
}

fn format_response_for_range(
    source: &str,
    range: TextRange,
    formatted: FormattedSource,
    line_index: &shuck_indexer::LineIndex,
    encoding: crate::PositionEncoding,
) -> Vec<types::TextEdit> {
    match formatted {
        FormattedSource::Unchanged => Vec::new(),
        FormattedSource::Formatted(code) => crate::edit::single_replacement_edit_in_range(
            source, range, &code, line_index, encoding,
        )
        .into_iter()
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{ClientCapabilities, PositionEncodingKind, Url};

    use super::*;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

    #[test]
    fn document_formatting_uses_shuck_formatter() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-format-document-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("if true; then\necho ok\nfi\n".to_owned(), 1)
                .with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_document(
            snapshot,
            &client,
            types::DocumentFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("document formatting should succeed")
        .expect("document formatting should return an edit list");

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, types::Position::new(1, 0));
        assert_eq!(edits[0].range.end, types::Position::new(1, 0));
        assert_eq!(edits[0].new_text, "\t");
    }

    #[test]
    fn document_formatting_uses_shared_per_file_shell_config() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(
            tempdir.path().join("shuck.toml"),
            "[per-file-shell]\n'dot_z*' = 'zsh'\n",
        )
        .expect("config should be written");
        let source = "(($+commands[vivid]))\n";
        let file_path = tempdir.path().join("dot_zshenv");
        std::fs::write(&file_path, source).expect("source should be written");

        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri =
            Url::from_file_path(tempdir.path()).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(file_path).expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_document(
            snapshot,
            &client,
            types::DocumentFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("document formatting should succeed")
        .expect("document formatting should return an edit list");

        assert!(edits.is_empty());
    }

    #[test]
    fn formatting_returns_none_for_config_excluded_document() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(
            tempdir.path().join("shuck.toml"),
            "[format]\nexclude = ['**/*p10k.zsh']\n",
        )
        .expect("config should be written");

        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri =
            Url::from_file_path(tempdir.path()).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(tempdir.path().join(".p10k.zsh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("if true; then\nprint ok\nfi\n".to_owned(), 1)
                .with_language_id("shellscript"),
        );

        let document_response = format_document(
            session
                .take_snapshot(uri.clone())
                .expect("test document should produce a snapshot"),
            &client,
            types::DocumentFormattingParams {
                text_document: types::TextDocumentIdentifier { uri: uri.clone() },
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("document formatting should succeed");
        assert!(document_response.is_none());

        let range_response = format_range(
            session
                .take_snapshot(uri.clone())
                .expect("test document should produce a snapshot"),
            &client,
            types::DocumentRangeFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                range: types::Range::new(types::Position::new(0, 0), types::Position::new(2, 2)),
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("range formatting should succeed");
        assert!(range_response.is_none());
    }

    #[test]
    fn range_formatting_returns_empty_edits_for_already_formatted_buffer() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-format-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities {
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![PositionEncodingKind::UTF16]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("echo hi\n".to_owned(), 1).with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_range(
            snapshot,
            &client,
            types::DocumentRangeFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                range: types::Range::new(types::Position::new(0, 0), types::Position::new(0, 7)),
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("range formatting should succeed")
        .expect("range formatting should return an edit list");

        assert!(edits.is_empty());
    }

    #[test]
    fn range_formatting_returns_none_for_ranges_without_one_complete_statement() {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-format-tests-partial");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("echo one\necho two\n".to_owned(), 1).with_language_id("shellscript"),
        );
        let snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");

        let edits = format_range(
            snapshot,
            &client,
            types::DocumentRangeFormattingParams {
                text_document: types::TextDocumentIdentifier { uri },
                range: types::Range::new(types::Position::new(0, 2), types::Position::new(1, 2)),
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("range formatting should succeed");

        assert!(edits.is_none());
    }

    #[test]
    fn range_formatting_edit_helper_does_not_escape_statement_range() {
        let source = "echo one\necho two\n";
        let index = shuck_indexer::LineIndex::new(source);
        let edits = format_response_for_range(
            source,
            TextRange::new(shuck_ast::TextSize::new(9), shuck_ast::TextSize::new(18)),
            FormattedSource::Formatted("printf two\n".to_owned()),
            &index,
            PositionEncoding::UTF16,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, types::Position::new(1, 0));
        assert_eq!(edits[0].range.end, types::Position::new(1, 4));
        assert_eq!(edits[0].new_text, "printf");
    }
}
