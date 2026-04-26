use shuck_ast::{AstStore, WordPartArena, WordPartArenaNode};

use crate::facts::words::FactWordRef;
use crate::{Edit, Fix};

pub(crate) fn leading_static_word_prefix_fix_in_source(
    word: FactWordRef<'_>,
    source: &str,
    prefix_len: usize,
) -> Option<Fix> {
    let mut remaining = prefix_len;
    let mut edits = Vec::new();
    collect_leading_static_word_prefix_edits(
        word.parts(),
        word.view().store(),
        source,
        &mut remaining,
        &mut edits,
    )?;
    (remaining == 0).then(|| Fix::unsafe_edits(edits))
}

fn collect_leading_static_word_prefix_edits(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    remaining: &mut usize,
    edits: &mut Vec<Edit>,
) -> Option<()> {
    for part in parts {
        if *remaining == 0 {
            break;
        }
        collect_leading_static_word_prefix_edits_from_part(part, store, source, remaining, edits)?;
    }

    Some(())
}

fn collect_leading_static_word_prefix_edits_from_part(
    part: &WordPartArenaNode,
    store: &AstStore,
    source: &str,
    remaining: &mut usize,
    edits: &mut Vec<Edit>,
) -> Option<()> {
    let semantic = static_word_part_string(part, store, source)?;
    if semantic.is_empty() {
        return Some(());
    }

    if semantic.len() <= *remaining {
        edits.push(Edit::deletion(part.span));
        *remaining -= semantic.len();
        return Some(());
    }

    match &part.kind {
        WordPartArena::Literal(text) => collect_leading_static_word_prefix_edit_from_segment(
            text.syntax_str(source, part.span),
            semantic.as_str(),
            part.span.start.offset,
            remaining,
            edits,
        ),
        WordPartArena::SingleQuoted { value, .. } => {
            let content_span = value.span();
            collect_leading_static_word_prefix_edit_from_segment(
                content_span.slice(source),
                semantic.as_str(),
                content_span.start.offset,
                remaining,
                edits,
            )
        }
        WordPartArena::DoubleQuoted { parts, .. } => collect_leading_static_word_prefix_edits(
            store.word_parts(*parts),
            store,
            source,
            remaining,
            edits,
        ),
        _ => None,
    }
}

fn collect_leading_static_word_prefix_edit_from_segment(
    raw: &str,
    semantic: &str,
    start_offset: usize,
    remaining: &mut usize,
    edits: &mut Vec<Edit>,
) -> Option<()> {
    if *remaining == 0 || semantic.is_empty() {
        return Some(());
    }

    let consumed = semantic.len().min(*remaining);
    let raw_prefix = raw.get(..consumed)?;
    let semantic_prefix = semantic.get(..consumed)?;
    if raw_prefix != semantic_prefix {
        return None;
    }

    edits.push(Edit::deletion_at(start_offset, start_offset + consumed));
    *remaining -= consumed;
    Some(())
}

fn static_word_part_string(
    part: &WordPartArenaNode,
    store: &AstStore,
    source: &str,
) -> Option<String> {
    match &part.kind {
        WordPartArena::Literal(text) => Some(text.as_str(source, part.span).to_owned()),
        WordPartArena::SingleQuoted { value, .. } => Some(value.slice(source).to_owned()),
        WordPartArena::DoubleQuoted { parts, .. } => {
            let mut text = String::new();
            for nested in store.word_parts(*parts) {
                text.push_str(&static_word_part_string(nested, store, source)?);
            }
            Some(text)
        }
        _ => None,
    }
}
