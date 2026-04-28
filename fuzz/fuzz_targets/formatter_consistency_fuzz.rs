//! Fuzz target for formatter consistency and idempotency.

#![no_main]

mod common;
mod formatter_consistency_common;

use libfuzzer_sys::{Corpus, fuzz_target};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    for case in formatter_consistency_common::FORMAT_CASES {
        formatter_consistency_common::compare_formatting_invariants(input, case);
    }

    Corpus::Keep
});
