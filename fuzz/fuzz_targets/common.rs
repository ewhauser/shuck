use libfuzzer_sys::Corpus;

const MAX_FUZZ_INPUT_BYTES: usize = 16 * 1024;
const MAX_FUZZ_NESTING: i32 = 64;

pub(crate) fn filtered_input(data: &[u8]) -> Result<&str, Corpus> {
    let input = std::str::from_utf8(data).map_err(|_| Corpus::Reject)?;
    if input.len() > MAX_FUZZ_INPUT_BYTES
        || max_nesting(input) > MAX_FUZZ_NESTING
        || contains_disallowed_controls(input)
    {
        return Err(Corpus::Reject);
    }
    Ok(input)
}

fn contains_disallowed_controls(input: &str) -> bool {
    input
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}

fn max_nesting(input: &str) -> i32 {
    input
        .bytes()
        .map(|byte| match byte {
            b'(' | b'{' | b'[' => 1,
            b')' | b'}' | b']' => -1,
            _ => 0,
        })
        .scan(0i32, |depth, delta| {
            *depth += delta;
            Some(*depth)
        })
        .max()
        .unwrap_or(0)
}
