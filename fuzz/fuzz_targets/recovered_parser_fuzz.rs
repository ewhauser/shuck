//! Fuzz target for recovered parsing across supported dialects.

#![no_main]

mod common;

use libfuzzer_sys::{Corpus, fuzz_target};
use shuck_ast::Span;

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for dialect in common::PARSER_DIALECTS {
        let (recovered, _indexer) = common::recovered_parse_and_index(input, dialect);
        validate_recovered_parse(input, &recovered);
    }

    Corpus::Keep
});

fn validate_recovered_parse(source: &str, recovered: &shuck_parser::parser::RecoveredParse) {
    common::assert_span_valid(recovered.file.span, source);
    assert_eq!(
        recovered.file.span.end.offset,
        source.len(),
        "recovered parse should cover the full source"
    );

    for diagnostic in &recovered.diagnostics {
        common::assert_span_valid(diagnostic.span, source);
        assert!(
            !diagnostic.message.trim().is_empty(),
            "recovered parse diagnostics must have a message"
        );
    }

    validate_non_overlapping_root_span(recovered.file.span, source);
}

fn validate_non_overlapping_root_span(span: Span, source: &str) {
    common::assert_span_valid(span, source);
    assert_eq!(span.start.offset, 0, "root span should start at offset 0");
}
