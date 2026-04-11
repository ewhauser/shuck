//! Fuzz target for the shuck lexer.
//!
//! Run with: `cd fuzz && cargo +nightly fuzz run lexer_fuzz`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        if input.len() > 1_000_000 {
            return;
        }

        let mut lexer = shuck_parser::parser::Lexer::new(input);
        while lexer.next_lexed_token().is_some() {}
    }
});
