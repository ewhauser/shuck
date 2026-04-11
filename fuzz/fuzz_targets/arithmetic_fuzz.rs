//! Fuzz target for arithmetic expansion parsing.
//!
//! Run with: `cd fuzz && cargo +nightly fuzz run arithmetic_fuzz`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        if input.len() > 512 {
            return;
        }

        let depth: i32 = input
            .bytes()
            .map(|byte| match byte {
                b'(' => 1,
                b')' => -1,
                _ => 0,
            })
            .scan(0i32, |acc, delta| {
                *acc += delta;
                Some(*acc)
            })
            .max()
            .unwrap_or(0);
        if depth > 20 {
            return;
        }

        let script = format!("echo $(({}))\n", input);
        let _ = shuck_parser::parser::Parser::new(&script).parse();
    }
});
