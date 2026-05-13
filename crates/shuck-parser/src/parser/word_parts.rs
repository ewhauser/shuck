use super::*;

impl<'a> Parser<'a> {
    pub(super) fn word_text_needs_parse(text: &str) -> bool {
        memchr3(b'$', b'`', b'\0', text.as_bytes()).is_some()
    }

    pub(super) fn word_with_parts(&self, parts: Vec<WordPartNode>, span: Span) -> Word {
        let brace_syntax = self.brace_syntax_from_parts(&parts, span.start.offset);
        Word {
            parts,
            span,
            brace_syntax,
        }
    }

    pub(super) fn word_with_part_buffer(&self, parts: WordPartBuffer, span: Span) -> Word {
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

    pub(super) fn word_with_single_part(&self, part: WordPartNode, span: Span) -> Word {
        let mut parts = WordPartBuffer::new();
        parts.push(part);
        self.word_with_part_buffer(parts, span)
    }

    pub(super) fn heredoc_body_with_parts(
        &self,
        parts: Vec<HeredocBodyPartNode>,
        span: Span,
        mode: HeredocBodyMode,
        source_backed: bool,
    ) -> HeredocBody {
        HeredocBody {
            mode,
            source_backed,
            parts,
            span,
        }
    }

    pub(super) fn heredoc_body_part_from_word_part_node(
        &self,
        part: WordPartNode,
        source_backed: bool,
    ) -> HeredocBodyPartNode {
        let span = part.span;
        let kind = match part.kind {
            WordPart::Literal(text) => HeredocBodyPart::Literal(text),
            WordPart::Variable(name) => HeredocBodyPart::Variable(name),
            WordPart::CommandSubstitution { body, syntax } => {
                HeredocBodyPart::CommandSubstitution { body, syntax }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                syntax,
            } => HeredocBodyPart::ArithmeticExpansion {
                expression,
                expression_ast: expression_ast.map(|ast| *ast),
                expression_word_ast: *expression_word_ast,
                syntax,
            },
            WordPart::Parameter(parameter) => HeredocBodyPart::Parameter(parameter),
            part @ (WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }) => {
                match self.parameter_word_part_from_legacy(
                    part,
                    span.start,
                    span.end,
                    source_backed,
                ) {
                    WordPart::Parameter(parameter) => HeredocBodyPart::Parameter(parameter),
                    other => self.literal_heredoc_body_part_from_word_part(other, span),
                }
            }
            other => self.literal_heredoc_body_part_from_word_part(other, span),
        };

        HeredocBodyPartNode::new(kind, span)
    }

    pub(super) fn literal_heredoc_body_part_from_word_part(
        &self,
        part: WordPart,
        span: Span,
    ) -> HeredocBodyPart {
        if span.end.offset <= self.input.len() {
            return HeredocBodyPart::Literal(LiteralText::source());
        }

        let mut syntax = String::new();
        self.push_word_part_syntax(&mut syntax, &part, span);
        HeredocBodyPart::Literal(LiteralText::owned(syntax))
    }
}
