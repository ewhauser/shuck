use super::*;

impl<'a, 'idx, 'observer> SemanticModelBuilder<'a, 'idx, 'observer> {
    /// Classifies a `source`/`.` operand, returning its syntactic kind and
    /// whether a directive asked to follow (not just import) the target.
    pub(super) fn classify_source_ref(&self, line: usize, word: &Word) -> (SourceRefKind, bool) {
        if let Some(directive) = self.source_directive_for_line(line) {
            return directive;
        }

        if let Some(text) = static_word_text(word, self.source) {
            return (SourceRefKind::Literal(text.as_ref().into()), false);
        }

        (classify_dynamic_source_word(word, self.source), false)
    }

    pub(super) fn source_directive_for_line(&self, line: usize) -> Option<(SourceRefKind, bool)> {
        if let Some(directive) = self.source_directives.get(&line) {
            return Some((directive.kind.clone(), directive.follow));
        }

        if let Some(previous) = line.checked_sub(1)
            && let Some(directive) = self.source_directives.get(&previous)
            && directive.own_line
        {
            return Some((directive.kind.clone(), directive.follow));
        }

        let directive = self
            .source_directives
            .range(..line)
            .rev()
            .find(|(_, directive)| directive.own_line)
            .map(|(_, directive)| directive)?;

        match directive.kind {
            SourceRefKind::DirectiveDevNull => Some((SourceRefKind::DirectiveDevNull, false)),
            _ => None,
        }
    }
}
