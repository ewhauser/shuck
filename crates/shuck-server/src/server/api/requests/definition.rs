use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct Definition;

impl super::RequestHandler for Definition {
    type RequestType = req::GotoDefinition;
}

impl super::BackgroundDocumentRequestHandler for Definition {
    fn document_url(params: &types::GotoDefinitionParams) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position_params.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::GotoDefinitionParams,
    ) -> crate::server::Result<editor_features::DefinitionResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::GotoDefinitionParams,
    ) -> crate::server::Result<editor_features::DefinitionResponse> {
        editor_features::definition(snapshot, client, params)
    }
}
