use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn classify_source_ref(&self, line: usize, word: &Word) -> SourceRefKind {
        if let Some(directive) = self.source_directive_for_line(line) {
            return directive;
        }

        if let Some(text) = static_word_text(word, self.source) {
            return SourceRefKind::Literal(text.as_ref().into());
        }

        classify_dynamic_source_word(word, self.source)
    }

    pub(super) fn source_directive_for_line(&self, line: usize) -> Option<SourceRefKind> {
        if let Some(directive) = self.source_directives.get(&line) {
            return Some(directive.kind.clone());
        }

        if let Some(previous) = line.checked_sub(1)
            && let Some(directive) = self.source_directives.get(&previous)
            && directive.own_line
        {
            return Some(directive.kind.clone());
        }

        let directive = self
            .source_directives
            .range(..line)
            .rev()
            .find(|(_, directive)| directive.own_line)
            .map(|(_, directive)| directive)?;

        match directive.kind {
            SourceRefKind::DirectiveDevNull => Some(SourceRefKind::DirectiveDevNull),
            _ => None,
        }
    }
}
