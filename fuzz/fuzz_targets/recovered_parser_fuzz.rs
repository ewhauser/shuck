//! Fuzz target for recovered parsing across supported dialects.

#![no_main]

mod common;
mod recovered_common;

use libfuzzer_sys::{Corpus, fuzz_target};
use shuck_ast::Span;
use shuck_parser::parser::{ParseResult, ParseStatus};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for dialect in recovered_common::PARSER_DIALECTS {
        let (recovered, _indexer) = recovered_common::recovered_parse_and_index(input, dialect);
        validate_recovered_parse(input, &recovered);
    }

    Corpus::Keep
});

fn validate_recovered_parse(source: &str, parse_result: &ParseResult) {
    recovered_common::assert_span_valid(parse_result.file.span, source);
    if parse_result.status != ParseStatus::Fatal {
        assert_eq!(
            parse_result.file.span.end.offset,
            source.len(),
            "non-fatal parse should cover the full source"
        );
    }

    for diagnostic in &parse_result.diagnostics {
        recovered_common::assert_span_valid(diagnostic.span, source);
        assert!(
            !diagnostic.message.trim().is_empty(),
            "recovered parse diagnostics must have a message"
        );
    }

    for span in &parse_result.syntax_facts.zsh_brace_if_spans {
        recovered_common::assert_span_valid(*span, source);
    }

    for span in &parse_result.syntax_facts.zsh_always_spans {
        recovered_common::assert_span_valid(*span, source);
    }

    for part in &parse_result.syntax_facts.zsh_case_group_parts {
        recovered_common::assert_span_valid(part.span, source);
    }

    match parse_result.status {
        ParseStatus::Clean => {
            assert!(
                parse_result.diagnostics.is_empty(),
                "clean parses should not emit recovery diagnostics"
            );
            assert!(
                parse_result.terminal_error.is_none(),
                "clean parses should not carry a terminal error"
            );
        }
        ParseStatus::Recovered => {
            assert!(
                !parse_result.diagnostics.is_empty(),
                "recovered parses should include diagnostics"
            );
            assert!(
                parse_result.terminal_error.is_none(),
                "recovered parses should not carry a terminal error"
            );
        }
        ParseStatus::Fatal => {
            assert!(
                parse_result.terminal_error.is_some(),
                "fatal parses should carry a terminal error"
            );
        }
    }

    validate_non_overlapping_root_span(parse_result.file.span, source);
}

fn validate_non_overlapping_root_span(span: Span, source: &str) {
    recovered_common::assert_span_valid(span, source);
    assert_eq!(span.start.offset, 0, "root span should start at offset 0");
}
