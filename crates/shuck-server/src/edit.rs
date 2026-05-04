#![allow(dead_code)]

mod range;
mod text_document;

use lsp_types::{PositionEncodingKind, Url};
use shuck_ast::TextRange;

pub(crate) use range::RangeExt;
pub(crate) use text_document::DocumentVersion;
pub(crate) use text_document::LanguageId;
pub use text_document::TextDocument;

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PositionEncoding {
    #[default]
    UTF16,
    UTF32,
    UTF8,
}

#[derive(Clone, Debug)]
pub enum DocumentKey {
    Text(Url),
}

impl DocumentKey {
    pub(crate) fn into_url(self) -> Url {
        match self {
            Self::Text(url) => url,
        }
    }
}

impl std::fmt::Display for DocumentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(url) => url.fmt(f),
        }
    }
}

impl From<PositionEncoding> for PositionEncodingKind {
    fn from(value: PositionEncoding) -> Self {
        match value {
            PositionEncoding::UTF8 => PositionEncodingKind::UTF8,
            PositionEncoding::UTF16 => PositionEncodingKind::UTF16,
            PositionEncoding::UTF32 => PositionEncodingKind::UTF32,
        }
    }
}

impl TryFrom<&PositionEncodingKind> for PositionEncoding {
    type Error = ();

    fn try_from(value: &PositionEncodingKind) -> Result<Self, Self::Error> {
        Ok(if value == &PositionEncodingKind::UTF8 {
            Self::UTF8
        } else if value == &PositionEncodingKind::UTF16 {
            Self::UTF16
        } else if value == &PositionEncodingKind::UTF32 {
            Self::UTF32
        } else {
            return Err(());
        })
    }
}

fn clamp_offset_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut clamped = offset.min(text.len());
    while clamped > 0 && !text.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

fn offset_to_position(
    text: &str,
    index: &shuck_indexer::LineIndex,
    offset: usize,
    encoding: PositionEncoding,
) -> lsp_types::Position {
    let offset = clamp_offset_to_char_boundary(text, offset);
    let line = index.line_number(shuck_ast::TextSize::new(offset as u32));
    let line_start = index.line_start(line).map(usize::from).unwrap_or_default();
    let prefix = &text[line_start..offset];
    let character = match encoding {
        PositionEncoding::UTF8 => prefix.len(),
        PositionEncoding::UTF16 => prefix.encode_utf16().count(),
        PositionEncoding::UTF32 => prefix.chars().count(),
    };

    lsp_types::Position {
        line: u32::try_from(line.saturating_sub(1)).unwrap_or(u32::MAX),
        character: u32::try_from(character).unwrap_or(u32::MAX),
    }
}

fn position_to_offset(
    text: &str,
    index: &shuck_indexer::LineIndex,
    position: lsp_types::Position,
    encoding: PositionEncoding,
) -> usize {
    let line = usize::try_from(position.line).unwrap_or(usize::MAX) + 1;
    let line = line.min(index.line_count());
    let line_start = index
        .line_start(line)
        .map(usize::from)
        .unwrap_or(text.len());
    let line_end = index
        .line_range(line, text)
        .map(|range| usize::from(range.end()))
        .unwrap_or(text.len());
    let line_text = &text[line_start..line_end];
    let target = usize::try_from(position.character).unwrap_or(usize::MAX);

    let relative = match encoding {
        PositionEncoding::UTF8 => target.min(line_text.len()),
        PositionEncoding::UTF16 => {
            let mut units = 0usize;
            let mut offset = line_text.len();
            for (idx, ch) in line_text.char_indices() {
                if units >= target {
                    offset = idx;
                    break;
                }
                units += ch.len_utf16();
            }
            if units < target {
                line_text.len()
            } else {
                offset
            }
        }
        PositionEncoding::UTF32 => {
            let mut chars = 0usize;
            let mut offset = line_text.len();
            for (idx, _) in line_text.char_indices() {
                if chars >= target {
                    offset = idx;
                    break;
                }
                chars += 1;
            }
            if chars < target {
                line_text.len()
            } else {
                offset
            }
        }
    };

    clamp_offset_to_char_boundary(line_text, relative) + line_start
}

pub(crate) fn to_text_range(
    range: &lsp_types::Range,
    text: &str,
    index: &shuck_indexer::LineIndex,
    encoding: PositionEncoding,
) -> TextRange {
    let start = position_to_offset(text, index, range.start, encoding);
    let end = position_to_offset(text, index, range.end, encoding);
    TextRange::new(
        shuck_ast::TextSize::new(start.min(end) as u32),
        shuck_ast::TextSize::new(end.max(start) as u32),
    )
}

pub(crate) fn to_lsp_range(
    range: TextRange,
    text: &str,
    index: &shuck_indexer::LineIndex,
    encoding: PositionEncoding,
) -> lsp_types::Range {
    lsp_types::Range {
        start: offset_to_position(text, index, usize::from(range.start()), encoding),
        end: offset_to_position(text, index, usize::from(range.end()), encoding),
    }
}

pub(crate) fn single_replacement_edit(
    text: &str,
    replacement: &str,
    index: &shuck_indexer::LineIndex,
    encoding: PositionEncoding,
) -> Option<lsp_types::TextEdit> {
    if text == replacement {
        return None;
    }

    let prefix_len = common_prefix_len(text, replacement);
    let suffix_len = common_suffix_len(&text[prefix_len..], &replacement[prefix_len..]);
    let original_end = text.len().saturating_sub(suffix_len);
    let replacement_end = replacement.len().saturating_sub(suffix_len);

    Some(lsp_types::TextEdit {
        range: to_lsp_range(
            TextRange::new(
                shuck_ast::TextSize::new(prefix_len as u32),
                shuck_ast::TextSize::new(original_end as u32),
            ),
            text,
            index,
            encoding,
        ),
        new_text: replacement[prefix_len..replacement_end].to_owned(),
    })
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum()
}

fn common_suffix_len(left: &str, right: &str) -> usize {
    left.chars()
        .rev()
        .zip(right.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_replacement_edit_keeps_shared_prefix_and_suffix() {
        let source = "echo old value\n";
        let replacement = "echo new value\n";
        let index = shuck_indexer::LineIndex::new(source);

        let edit = single_replacement_edit(source, replacement, &index, PositionEncoding::UTF16)
            .expect("edit should be present");

        assert_eq!(
            edit.range,
            lsp_types::Range {
                start: lsp_types::Position::new(0, 5),
                end: lsp_types::Position::new(0, 8),
            }
        );
        assert_eq!(edit.new_text, "new");
    }

    #[test]
    fn single_replacement_edit_returns_none_for_identical_text() {
        let source = "echo hi\n";
        let index = shuck_indexer::LineIndex::new(source);

        assert!(
            single_replacement_edit(source, source, &index, PositionEncoding::UTF16).is_none()
        );
    }
}
