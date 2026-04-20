//! Fuzz target for the shuck lexer.
//!
//! Run with: `cd fuzz && cargo +nightly fuzz run lexer_fuzz`

#![no_main]

mod common;

use libfuzzer_sys::{Corpus, fuzz_target};

fuzz_target!(|data: &[u8]| -> Corpus {
    let input = match common::filtered_input(data) {
        Ok(input) => input,
        Err(reject) => return reject,
    };

    let mut lexer = shuck_parser::parser::Lexer::new(input);
    while lexer.next_lexed_token().is_some() {}

    Corpus::Keep
});
