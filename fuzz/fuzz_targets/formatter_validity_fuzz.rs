//! Fuzz target for formatter validity against strict parsing and lint counts.

#![no_main]

mod common;
mod formatter_validity_common;

use libfuzzer_sys::{Corpus, fuzz_target};
use shuck_formatter::{FormatError, format_source};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for case in formatter_validity_common::FORMAT_CASES {
        let path = case.path();
        let options = case.format_options();

        let formatted = match format_source(input, Some(path), &options) {
            Ok(result) => formatter_validity_common::format_result_to_string(result, input),
            Err(FormatError::Parse { .. }) => continue,
            Err(FormatError::Internal(message)) => {
                panic!("internal formatter error for {}: {message}", path.display())
            }
        };

        let original_diagnostics =
            formatter_validity_common::lint_source_strict(input, path, case.parse_dialect());
        let formatted_diagnostics =
            formatter_validity_common::lint_source_strict(&formatted, path, case.parse_dialect());
        formatter_validity_common::compare_lint_counts(
            &original_diagnostics,
            &formatted_diagnostics,
            path,
        );
    }

    Corpus::Keep
});
