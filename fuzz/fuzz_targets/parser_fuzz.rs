//! Fuzz target for the shuck parser.
//!
//! Run with: `cd fuzz && cargo +nightly fuzz run parser_fuzz`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        if input.len() > 1_000_000 {
            return;
        }

        let parser = shuck_parser::parser::Parser::new(input);
        let _ = parser.parse();
    }
});
