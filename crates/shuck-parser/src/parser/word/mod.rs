use super::*;

mod arithmetic;
mod fallback;
mod heredocs;
mod parameters;
mod preserve;
mod raw_shell;
mod render;
mod substitutions;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn word_text_needs_parse(text: &str) -> bool {
        memchr3(b'$', b'`', b'\0', text.as_bytes()).is_some()
    }

    pub(in crate::parser) fn word_with_parts(&self, parts: Vec<WordPartNode>, span: Span) -> Word {
        let brace_syntax = self.brace_syntax_from_parts(&parts, span.start.offset);
        Word {
            parts,
            span,
            brace_syntax,
        }
    }

    pub(in crate::parser) fn word_with_part_buffer(
        &self,
        parts: WordPartBuffer,
        span: Span,
    ) -> Word {
        let brace_syntax = self.brace_syntax_from_parts(&parts, span.start.offset);
        let parts = if parts.spilled() {
            parts.into_vec()
        } else {
            let mut vec = Vec::with_capacity(parts.len());
            vec.extend(parts);
            vec
        };
        Word {
            parts,
            span,
            brace_syntax,
        }
    }

    pub(in crate::parser) fn word_with_single_part(&self, part: WordPartNode, span: Span) -> Word {
        let mut parts = WordPartBuffer::new();
        parts.push(part);
        self.word_with_part_buffer(parts, span)
    }

    pub(in crate::parser) fn push_word_part(
        parts: &mut WordPartBuffer,
        part: WordPart,
        start: Position,
        end: Position,
    ) {
        Self::push_word_part_node(
            parts,
            WordPartNode::new(part, Span::from_positions(start, end)),
        );
    }

    pub(in crate::parser) fn push_word_part_node(parts: &mut WordPartBuffer, part: WordPartNode) {
        parts.push(part);
    }

    pub(in crate::parser) fn flush_literal_part(
        &self,
        parts: &mut WordPartBuffer,
        current: &mut String,
        current_start: Position,
        end: Position,
        source_backed: bool,
    ) {
        if !current.is_empty() {
            Self::push_word_part(
                parts,
                WordPart::Literal(self.literal_text(
                    std::mem::take(current),
                    current_start,
                    end,
                    source_backed,
                )),
                current_start,
                end,
            );
        }
    }

    pub(in crate::parser) fn word_part_buffer_with_capacity(capacity: usize) -> WordPartBuffer {
        if capacity <= 2 {
            WordPartBuffer::new()
        } else {
            WordPartBuffer::with_capacity(capacity)
        }
    }
}
