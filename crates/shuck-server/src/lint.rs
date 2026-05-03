use lsp_types as types;

use crate::session::DocumentSnapshot;

pub(crate) fn generate_diagnostics(_snapshot: &DocumentSnapshot) -> Vec<types::Diagnostic> {
    Vec::new()
}
