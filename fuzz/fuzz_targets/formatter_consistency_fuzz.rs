//! Fuzz target for formatter consistency and idempotency.

#![no_main]

mod common;

use libfuzzer_sys::{Corpus, fuzz_target};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for case in common::FORMAT_CASES {
        common::compare_formatting_invariants(input, case);
    }

    Corpus::Keep
});
