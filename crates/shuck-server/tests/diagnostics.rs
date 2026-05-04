use std::path::Path;

use crossbeam::channel;
use lsp_types::{ClientCapabilities, NumberOrString, Url};
use shuck_server::{
    Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    generate_diagnostics,
};

fn make_session(encoding: PositionEncoding) -> Session {
    let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
    let (client_sender, _client_receiver) = channel::unbounded();
    let client = Client::new(main_loop_sender, client_sender);
    let workspaces = Workspaces::new(vec![Workspace::default(
        Url::from_file_path(std::env::temp_dir())
            .expect("temporary directory should convert to a file URL"),
    )]);
    let global = GlobalOptions::default().into_settings(client.clone());

    Session::new(
        &ClientCapabilities::default(),
        encoding,
        global,
        &workspaces,
        &client,
    )
    .expect("test session should initialize")
}

fn open_snapshot(
    path: &Path,
    source: &str,
    language_id: &str,
    encoding: PositionEncoding,
) -> shuck_server::DocumentSnapshot {
    let mut session = make_session(encoding);
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
fn shell_document_snapshot_reports_native_shuck_diagnostic() {
    let snapshot = open_snapshot(
        &std::env::temp_dir().join("integration-unused-assignment.sh"),
        "foo=1\n",
        "shellscript",
        PositionEncoding::UTF16,
    );

    let diagnostics = generate_diagnostics(&snapshot);
    assert_eq!(diagnostics.len(), 1);

    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.source.as_deref(), Some("shuck"));
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("C001".to_owned()))
    );
    assert!(!diagnostic.message.is_empty());

    let data = diagnostic
        .data
        .clone()
        .expect("diagnostic payload should be present");
    assert_eq!(data["code"], "C001");
    assert!(data["directive_edit"].is_object());
    assert_eq!(data["applicability"], "Unsafe");
    assert_eq!(data["edits"][0]["newText"], "_");
}

#[test]
fn non_shell_document_snapshot_reports_no_diagnostics() {
    let snapshot = open_snapshot(
        &std::env::temp_dir().join("README.md"),
        "# Heading\n",
        "markdown",
        PositionEncoding::UTF16,
    );

    assert!(generate_diagnostics(&snapshot).is_empty());
}
