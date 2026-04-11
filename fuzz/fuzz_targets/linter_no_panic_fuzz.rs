//! Fuzz target for linter robustness on recovered parses.

#![no_main]

mod common;

use libfuzzer_sys::{Corpus, fuzz_target};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for case in common::FORMAT_CASES {
        let path = case.path();

        let with_path = common::lint_source_with_recovery(input, Some(path), case.parse_dialect());
        let without_path = common::lint_source_with_recovery(input, None, case.parse_dialect());

        for diagnostic in with_path.iter().chain(without_path.iter()) {
            assert!(
                !diagnostic.message.trim().is_empty(),
                "linter emitted an empty diagnostic message for {}",
                path.display()
            );
            common::assert_span_valid(diagnostic.span, input);
        }
    }

    Corpus::Keep
});
