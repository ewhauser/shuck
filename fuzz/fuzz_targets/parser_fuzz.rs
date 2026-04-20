//! Fuzz target for the shuck parser.
//!
//! Run with: `cd fuzz && cargo +nightly fuzz run parser_fuzz`

#![no_main]

mod common;

use libfuzzer_sys::{Corpus, fuzz_target};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    let parser = shuck_parser::parser::Parser::new(input);
    let _ = parser.parse();

    Corpus::Keep
});
