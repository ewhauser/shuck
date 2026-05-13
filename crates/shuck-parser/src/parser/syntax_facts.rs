use super::*;

impl<'a> Parser<'a> {
    pub(super) fn maybe_record_comment(&mut self, token: &LexedToken<'_>) {
        if token.kind == TokenKind::Comment && !token.flags.is_synthetic() {
            self.comments.push(Comment {
                range: token.span.to_range(),
            });
        }
    }

    pub(super) fn record_zsh_brace_if_span(&mut self, span: Span) {
        if !self.syntax_facts.zsh_brace_if_spans.contains(&span) {
            self.syntax_facts.zsh_brace_if_spans.push(span);
        }
    }

    pub(super) fn record_zsh_always_span(&mut self, span: Span) {
        if !self.syntax_facts.zsh_always_spans.contains(&span) {
            self.syntax_facts.zsh_always_spans.push(span);
        }
    }

    pub(super) fn record_zsh_case_group_part(&mut self, pattern_part_index: usize, span: Span) {
        if !self
            .syntax_facts
            .zsh_case_group_parts
            .iter()
            .any(|fact| fact.pattern_part_index == pattern_part_index && fact.span == span)
        {
            self.syntax_facts
                .zsh_case_group_parts
                .push(ZshCaseGroupPart {
                    pattern_part_index,
                    span,
                });
        }
    }
}
