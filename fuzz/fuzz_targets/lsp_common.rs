#![allow(dead_code)]

use lsp_types as types;
use serde_json::Value;
use shuck_server::{ClientOptions, PositionEncoding};

pub(crate) const LSP_ENCODINGS: [PositionEncoding; 3] = [
    PositionEncoding::UTF8,
    PositionEncoding::UTF16,
    PositionEncoding::UTF32,
];

pub(crate) fn encoding_from_byte(byte: u8) -> PositionEncoding {
    LSP_ENCODINGS[usize::from(byte) % LSP_ENCODINGS.len()]
}

pub(crate) fn language_id_from_byte(byte: u8) -> &'static str {
    match byte % 6 {
        0 => "shellscript",
        1 => "bash",
        2 => "sh",
        3 => "zsh",
        4 => "ksh",
        _ => "markdown",
    }
}

pub(crate) fn file_name_for_language(language_id: &str) -> &'static str {
    match language_id {
        "bash" => "fuzz.bash",
        "zsh" => "fuzz.zsh",
        "ksh" => "fuzz.ksh",
        "markdown" => "fuzz.md",
        _ => "fuzz.sh",
    }
}

pub(crate) fn position_from_bytes(
    source: &str,
    bytes: &[u8],
    encoding: PositionEncoding,
) -> types::Position {
    let first = bytes.first().copied().unwrap_or_default();
    let second = bytes.get(1).copied().unwrap_or_default();
    let lines = source_lines(source);
    let line_count = lines.len().max(1);
    let line = usize::from(first) % (line_count + 2);
    let line_text = lines.get(line.min(line_count - 1)).copied().unwrap_or("");
    let units = line_units(line_text, encoding).saturating_add(5).max(1);
    types::Position {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        character: u32::try_from(usize::from(second) % units).unwrap_or(u32::MAX),
    }
}

pub(crate) fn range_from_bytes(
    source: &str,
    bytes: &[u8],
    encoding: PositionEncoding,
) -> types::Range {
    let start = position_from_bytes(source, bytes, encoding);
    let end = position_from_bytes(source, bytes.get(2..).unwrap_or_default(), encoding);
    if bytes.get(4).copied().unwrap_or_default() & 1 == 0 {
        types::Range { start, end }
    } else {
        types::Range {
            start: end,
            end: start,
        }
    }
}

pub(crate) fn replacement_from_bytes(source: &str, bytes: &[u8]) -> String {
    let selector = bytes.first().copied().unwrap_or_default();
    let count = usize::from(bytes.get(1).copied().unwrap_or(8) % 32);
    match selector % 8 {
        0 => String::new(),
        1 => "\n".to_owned(),
        2 => "_".to_owned(),
        3 => "name=value\n".to_owned(),
        4 => "echo ${value}\n".to_owned(),
        5 => format!("unicode_{}\n", "\u{1f642}"),
        6 => source.chars().take(count).collect(),
        _ => {
            let mut chars = source.chars().rev().take(count).collect::<Vec<_>>();
            chars.reverse();
            chars.into_iter().collect()
        }
    }
}

pub(crate) fn new_name_from_byte(byte: u8) -> &'static str {
    match byte % 8 {
        0 => "renamed",
        1 => "_",
        2 => "name_1",
        3 => "foo=bar",
        4 => "",
        5 => "with space",
        6 => "new-function",
        _ => "z",
    }
}

pub(crate) fn workspace_query_from_byte(byte: u8) -> &'static str {
    match byte % 6 {
        0 => "",
        1 => "f",
        2 => "name",
        3 => "build",
        4 => "_",
        _ => "zz",
    }
}

pub(crate) fn client_options_from_byte(byte: u8) -> ClientOptions {
    ClientOptions {
        fix_all: Some(true),
        unsafe_fixes: Some(byte & 1 == 0),
        show_syntax_errors: Some(byte & 2 == 0),
        ..ClientOptions::default()
    }
}

pub(crate) fn capabilities_from_byte(byte: u8, encoding: PositionEncoding) -> types::ClientCapabilities {
    let encoding_name = match encoding {
        PositionEncoding::UTF8 => "utf-8",
        PositionEncoding::UTF16 => "utf-16",
        PositionEncoding::UTF32 => "utf-32",
    };
    serde_json::from_value(serde_json::json!({
        "general": {
            "positionEncodings": [encoding_name]
        },
        "textDocument": {
            "diagnostic": {
                "dynamicRegistration": false,
                "relatedDocumentSupport": false
            },
            "codeAction": {
                "dataSupport": byte & 1 == 0,
                "resolveSupport": { "properties": ["edit"] }
            },
            "documentSymbol": {
                "hierarchicalDocumentSymbolSupport": byte & 2 == 0
            },
            "hover": {
                "contentFormat": ["markdown", "plaintext"]
            }
        },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": {
                "documentChanges": byte & 4 == 0
            },
            "workspaceFolders": true,
            "configuration": false
        }
    }))
    .unwrap_or_default()
}

pub(crate) fn validate_lsp_ranges_in_values(
    source: &str,
    encoding: PositionEncoding,
    values: &[Value],
) {
    for value in values {
        validate_lsp_ranges_in_value(source, encoding, value);
    }
}

pub(crate) fn validate_lsp_ranges_in_value(source: &str, encoding: PositionEncoding, value: &Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                validate_lsp_ranges_in_value(source, encoding, value);
            }
        }
        Value::Object(object) => {
            if let Some(range) = object.get("range")
                && let Ok(range) = serde_json::from_value::<types::Range>(range.clone())
            {
                assert_valid_range(source, encoding, range);
            }
            for value in object.values() {
                validate_lsp_ranges_in_value(source, encoding, value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

pub(crate) fn assert_valid_range(
    source: &str,
    encoding: PositionEncoding,
    range: types::Range,
) {
    assert!(
        position_leq(range.start, range.end),
        "LSP range is not ordered: {:?}",
        range
    );
    assert_valid_position(source, encoding, range.start);
    assert_valid_position(source, encoding, range.end);
}

fn assert_valid_position(source: &str, encoding: PositionEncoding, position: types::Position) {
    let lines = source_lines(source);
    let line = usize::try_from(position.line).unwrap_or(usize::MAX);
    assert!(
        line < lines.len(),
        "LSP position line is out of bounds: {:?} for {} lines",
        position,
        lines.len()
    );
    let max_units = line_units(lines[line], encoding);
    let character = usize::try_from(position.character).unwrap_or(usize::MAX);
    assert!(
        character <= max_units,
        "LSP position character is out of bounds: {:?} for {} units",
        position,
        max_units
    );
}

fn position_leq(left: types::Position, right: types::Position) -> bool {
    (left.line, left.character) <= (right.line, right.character)
}

fn source_lines(source: &str) -> Vec<&str> {
    let mut lines = source.split('\n').collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("");
    }
    lines
}

fn line_units(line: &str, encoding: PositionEncoding) -> usize {
    match encoding {
        PositionEncoding::UTF8 => line.len(),
        PositionEncoding::UTF16 => line.encode_utf16().count(),
        PositionEncoding::UTF32 => line.chars().count(),
    }
}
