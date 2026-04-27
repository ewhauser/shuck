use super::*;

pub(crate) fn word_starts_with_literal_dash(word: &Word, source: &str) -> bool {
    matches!(
        word.parts_with_spans().next(),
        Some((WordPart::Literal(text), span)) if text.as_str(source, span).starts_with('-')
    )
}

pub(super) fn word_starts_with_static_or_literal_dash(word: &Word, source: &str) -> bool {
    static_word_text(word, source).is_some_and(|text| text.starts_with('-'))
        || word_starts_with_literal_dash(word, source)
}
