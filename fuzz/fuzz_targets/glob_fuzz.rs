//! Fuzz target for glob and pattern parsing.
//!
//! Run with: `cd fuzz && cargo +nightly fuzz run glob_fuzz`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        if input.len() > 512 {
            return;
        }

        let nesting: i32 = input
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
        if nesting > 15 {
            return;
        }

        let case_script = format!(
            "case \"test.txt\" in {}) echo match;; *) echo no;; esac\n",
            input
        );
        let _ = shuck_parser::parser::Parser::new(&case_script).parse();

        let conditional_script =
            format!("if [[ \"hello.world\" == {} ]]; then echo y; fi\n", input);
        let _ = shuck_parser::parser::Parser::new(&conditional_script).parse();
    }
});
