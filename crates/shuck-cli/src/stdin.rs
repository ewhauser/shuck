use std::io::{self, Read};

use anyhow::Result;

pub(crate) fn read_from_stdin() -> Result<String> {
    let mut buffer = String::new();
    io::stdin().lock().read_to_string(&mut buffer)?;
    Ok(buffer)
}
