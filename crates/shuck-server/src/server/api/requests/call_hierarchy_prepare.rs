use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct CallHierarchyPrepare;

impl super::RequestHandler for CallHierarchyPrepare {
    type RequestType = req::CallHierarchyPrepare;
}

impl super::BackgroundDocumentRequestHandler for CallHierarchyPrepare {
    fn document_url(
        params: &types::CallHierarchyPrepareParams,
    ) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position_params.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::CallHierarchyPrepareParams,
    ) -> crate::server::Result<editor_features::CallHierarchyPrepareResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::CallHierarchyPrepareParams,
    ) -> crate::server::Result<editor_features::CallHierarchyPrepareResponse> {
        editor_features::prepare_call_hierarchy(snapshot, client, params)
    }
}
