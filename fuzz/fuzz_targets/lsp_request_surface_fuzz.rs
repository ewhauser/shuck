//! Fuzz target for direct LSP request handler surfaces.

#![no_main]

mod common;
mod lsp_common;

use libfuzzer_sys::{Corpus, fuzz_target};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    let encoding = lsp_common::encoding_from_byte(data.first().copied().unwrap_or_default());
    let language_id = lsp_common::language_id_from_byte(data.get(1).copied().unwrap_or_default());
    let file_name = lsp_common::file_name_for_language(language_id);
    let position = lsp_common::position_from_bytes(input, data.get(2..).unwrap_or_default(), encoding);
    let range = lsp_common::range_from_bytes(input, data.get(4..).unwrap_or_default(), encoding);
    let capabilities =
        lsp_common::capabilities_from_byte(data.get(6).copied().unwrap_or_default(), encoding);
    let client_options = lsp_common::client_options_from_byte(data.get(7).copied().unwrap_or_default());
    let new_name = lsp_common::new_name_from_byte(data.get(8).copied().unwrap_or_default());
    let workspace_query =
        lsp_common::workspace_query_from_byte(data.get(9).copied().unwrap_or_default());

    let outputs = shuck_server::fuzzing::exercise_request_surface(
        shuck_server::fuzzing::RequestSurfaceInput {
            source: input,
            language_id,
            file_name,
            encoding,
            capabilities,
            client_options,
            position,
            range,
            new_name,
            workspace_query,
        },
    )
    .expect("LSP request surface harness should run");

    lsp_common::validate_lsp_ranges_in_values(input, encoding, &outputs);

    Corpus::Keep
});
