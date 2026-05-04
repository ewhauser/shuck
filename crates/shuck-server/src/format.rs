use anyhow::Error;
use lsp_types as types;
use shuck_formatter::FormattedSource;

use crate::session::{Client, DocumentSnapshot};

pub(crate) type FormatResponse = Option<Vec<types::TextEdit>>;

pub(crate) fn format_document(
    snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentFormattingParams,
) -> crate::server::Result<FormatResponse> {
    let query = snapshot.query();
    let source = query.document().contents();
    let formatted = shuck_formatter::format_source(
        source,
        query.file_path().as_deref(),
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
    client: &Client,
    _params: types::DocumentRangeFormattingParams,
) -> crate::server::Result<FormatResponse> {
    format_document(
        snapshot,
        client,
        types::DocumentFormattingParams {
            text_document: types::TextDocumentIdentifier {
                uri: lsp_types::Url::parse("file:///dev/null").expect("static URI should parse"),
            },
            options: types::FormattingOptions {
                tab_size: 8,
                insert_spaces: false,
                ..types::FormattingOptions::default()
            },
            work_done_progress_params: types::WorkDoneProgressParams::default(),
        },
    )
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{ClientCapabilities, PositionEncodingKind, Url};

    use super::*;
    use crate::{Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces};

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
                range: types::Range::new(
                    types::Position::new(0, 0),
                    types::Position::new(0, 7),
                ),
                options: types::FormattingOptions::default(),
                work_done_progress_params: types::WorkDoneProgressParams::default(),
            },
        )
        .expect("range formatting should succeed")
        .expect("range formatting should return an edit list");

        assert!(edits.is_empty());
    }
}
