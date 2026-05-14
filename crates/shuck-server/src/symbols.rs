use lsp_types as types;
use shuck_parser::parser::Parser;
use shuck_semantic::{EditorDocumentSymbol, EditorSymbolKind, SemanticBuildOptions, SemanticModel};

use crate::session::{Client, DocumentSnapshot};

pub(crate) type DocumentSymbolResponse = Option<types::DocumentSymbolResponse>;

pub(crate) fn document_symbols(
    snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentSymbolParams,
) -> crate::server::Result<DocumentSymbolResponse> {
    let Some(shell) = crate::lint::document_shell(&snapshot) else {
        return Ok(None);
    };

    let query = snapshot.query();
    let source = query.document().contents();
    let shell_profile = shell.shell_profile();
    let parse_result = Parser::with_profile(source, shell_profile.clone()).parse();
    let indexer = shuck_indexer::Indexer::new(source, &parse_result);
    let semantic = SemanticModel::build_with_options(
        &parse_result.file,
        source,
        &indexer,
        SemanticBuildOptions {
            source_path: query.file_path().as_deref(),
            shell_profile: Some(shell_profile),
            resolve_source_closure: false,
            ..SemanticBuildOptions::default()
        },
    );
    let editor_symbols = semantic.editor_query().document_symbols();

    if snapshot
        .resolved_client_capabilities()
        .hierarchical_document_symbols
    {
        let symbols = editor_symbols
            .iter()
            .map(|symbol| to_lsp_document_symbol(symbol, &snapshot))
            .collect();
        Ok(Some(types::DocumentSymbolResponse::Nested(symbols)))
    } else {
        let symbols = editor_symbols
            .iter()
            .flat_map(|symbol| to_lsp_symbol_information(symbol, &snapshot, None))
            .collect();
        Ok(Some(types::DocumentSymbolResponse::Flat(symbols)))
    }
}

#[allow(deprecated)]
fn to_lsp_document_symbol(
    symbol: &EditorDocumentSymbol,
    snapshot: &DocumentSnapshot,
) -> types::DocumentSymbol {
    let source = snapshot.query().document().contents();
    let line_index = snapshot.query().document().index();
    let children = (!symbol.children.is_empty()).then(|| {
        symbol
            .children
            .iter()
            .map(|child| to_lsp_document_symbol(child, snapshot))
            .collect()
    });

    types::DocumentSymbol {
        name: symbol.name.to_string(),
        detail: None,
        kind: to_lsp_symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        range: crate::edit::to_lsp_range(
            symbol.range.to_range(),
            source,
            line_index,
            snapshot.encoding(),
        ),
        selection_range: crate::edit::to_lsp_range(
            symbol.selection_span.to_range(),
            source,
            line_index,
            snapshot.encoding(),
        ),
        children,
    }
}

#[allow(deprecated)]
fn to_lsp_symbol_information(
    symbol: &EditorDocumentSymbol,
    snapshot: &DocumentSnapshot,
    container_name: Option<&str>,
) -> Vec<types::SymbolInformation> {
    let source = snapshot.query().document().contents();
    let line_index = snapshot.query().document().index();
    let mut symbols = vec![types::SymbolInformation {
        name: symbol.name.to_string(),
        kind: to_lsp_symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        location: types::Location::new(
            snapshot.query().file_url().clone(),
            crate::edit::to_lsp_range(
                symbol.selection_span.to_range(),
                source,
                line_index,
                snapshot.encoding(),
            ),
        ),
        container_name: container_name.map(str::to_owned),
    }];

    symbols.extend(
        symbol.children.iter().flat_map(|child| {
            to_lsp_symbol_information(child, snapshot, Some(symbol.name.as_str()))
        }),
    );
    symbols
}

fn to_lsp_symbol_kind(kind: EditorSymbolKind) -> types::SymbolKind {
    match kind {
        EditorSymbolKind::Function => types::SymbolKind::FUNCTION,
        EditorSymbolKind::Array | EditorSymbolKind::AssociativeArray => types::SymbolKind::ARRAY,
        EditorSymbolKind::Variable
        | EditorSymbolKind::Declaration
        | EditorSymbolKind::RuntimeName => types::SymbolKind::VARIABLE,
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, DocumentSymbolClientCapabilities, DocumentSymbolParams,
        DocumentSymbolResponse, PartialResultParams, PositionEncodingKind,
        TextDocumentClientCapabilities, TextDocumentIdentifier, Url, WorkDoneProgressParams,
    };

    use super::*;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

    fn position_encoding_kind(encoding: PositionEncoding) -> PositionEncodingKind {
        match encoding {
            PositionEncoding::UTF8 => PositionEncodingKind::UTF8,
            PositionEncoding::UTF16 => PositionEncodingKind::UTF16,
            PositionEncoding::UTF32 => PositionEncodingKind::UTF32,
        }
    }

    fn make_snapshot(
        source: &str,
        encoding: PositionEncoding,
        hierarchical_document_symbols: bool,
    ) -> (DocumentSnapshot, Client, Url) {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-symbol-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities {
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![position_encoding_kind(encoding)]),
                    ..Default::default()
                }),
                text_document: Some(TextDocumentClientCapabilities {
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        hierarchical_document_symbol_support: Some(hierarchical_document_symbols),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            encoding,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id("shellscript"),
        );

        (
            session
                .take_snapshot(uri.clone())
                .expect("test document should produce a snapshot"),
            client,
            uri,
        )
    }

    #[test]
    fn document_symbols_return_nested_lsp_symbols() {
        let source = "\
#!/bin/bash
VERSION=1
build() {
  local artifact
}
";
        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF16, true);
        let response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");

        let DocumentSymbolResponse::Nested(symbols) = response else {
            panic!("expected nested document symbols");
        };
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "VERSION");
        assert_eq!(symbols[0].kind, types::SymbolKind::VARIABLE);
        assert_eq!(symbols[0].selection_range.start.line, 1);
        assert_eq!(symbols[0].selection_range.start.character, 0);
        assert_eq!(symbols[0].selection_range.end.character, 7);

        assert_eq!(symbols[1].name, "build");
        assert_eq!(symbols[1].kind, types::SymbolKind::FUNCTION);
        assert_eq!(symbols[1].selection_range.start.line, 2);
        assert_eq!(symbols[1].selection_range.start.character, 0);
        assert_eq!(symbols[1].selection_range.end.character, 5);

        let children = symbols[1]
            .children
            .as_ref()
            .expect("function symbol should have children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "artifact");
        assert_eq!(children[0].kind, types::SymbolKind::VARIABLE);
        assert_eq!(children[0].selection_range.start.line, 3);
        assert_eq!(children[0].selection_range.start.character, 8);
    }

    #[test]
    fn document_symbols_fall_back_to_flat_response_without_hierarchical_client_support() {
        let source = "\
#!/bin/bash
VERSION=1
build() {
  local artifact
}
";
        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF16, false);
        let response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");

        let DocumentSymbolResponse::Flat(symbols) = response else {
            panic!("expected flat document symbols");
        };
        assert_eq!(
            symbols
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            ["VERSION", "build", "artifact"]
        );
        assert_eq!(symbols[0].container_name, None);
        assert_eq!(symbols[2].container_name.as_deref(), Some("build"));
    }

    #[test]
    fn document_symbol_ranges_use_negotiated_position_encoding() {
        let source = "build() { echo \"é\"; local cafe; }\n";

        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF16, true);
        let utf16_response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");
        let DocumentSymbolResponse::Nested(utf16_symbols) = utf16_response else {
            panic!("expected nested document symbols");
        };
        let utf16_child = &utf16_symbols[0]
            .children
            .as_ref()
            .expect("function should have children")[0];
        assert_eq!(utf16_child.name, "cafe");
        assert_eq!(utf16_child.selection_range.start.character, 26);
        assert_eq!(utf16_child.selection_range.end.character, 30);

        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF8, true);
        let utf8_response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");
        let DocumentSymbolResponse::Nested(utf8_symbols) = utf8_response else {
            panic!("expected nested document symbols");
        };
        let utf8_child = &utf8_symbols[0]
            .children
            .as_ref()
            .expect("function should have children")[0];
        assert_eq!(utf8_child.name, "cafe");
        assert_eq!(utf8_child.selection_range.start.character, 27);
        assert_eq!(utf8_child.selection_range.end.character, 31);
    }
}
