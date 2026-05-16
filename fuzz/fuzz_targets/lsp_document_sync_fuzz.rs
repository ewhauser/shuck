//! Fuzz target for LSP document synchronization edits.

#![no_main]

mod common;
mod lsp_common;

use libfuzzer_sys::{Corpus, fuzz_target};
use lsp_types::TextDocumentContentChangeEvent;

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for encoding in lsp_common::LSP_ENCODINGS {
        let mut source = input.to_owned();
        let mut version = 1;
        for chunk in data.chunks(6).take(10) {
            let text = lsp_common::replacement_from_bytes(input, chunk);
            let range = (chunk.first().copied().unwrap_or_default() % 4 != 0)
                .then(|| lsp_common::range_from_bytes(&source, chunk, encoding));
            let range_length = chunk
                .get(5)
                .copied()
                .map(u32::from)
                .filter(|_| chunk.first().copied().unwrap_or_default() & 1 == 0);
            let state = shuck_server::fuzzing::apply_text_document_changes(
                &source,
                version,
                vec![TextDocumentContentChangeEvent {
                    range,
                    range_length,
                    text,
                }],
                version + 1,
                encoding,
            );

            assert_eq!(state.version, version + 1);
            source = state.contents;
            version += 1;
        }
    }

    Corpus::Keep
});
