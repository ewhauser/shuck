use super::*;

impl<'a, 'idx, 'observer> SemanticModelBuilder<'a, 'idx, 'observer> {
    /// Classifies a `source`/`.` operand, returning its syntactic kind and the
    /// explicit directive (if any) that annotated the site.
    pub(super) fn classify_source_ref(
        &self,
        line: usize,
        word: &Word,
    ) -> (SourceRefKind, Option<SourceDirectiveInfo>) {
        if let Some((kind, directive)) = self.source_directive_for_line(line) {
            return (kind, Some(directive));
        }

        if let Some(text) = static_word_text(word, self.source) {
            return (SourceRefKind::Literal(text.as_ref().into()), None);
        }

        (classify_dynamic_source_word(word, self.source), None)
    }

    pub(super) fn source_directive_for_line(
        &self,
        line: usize,
    ) -> Option<(SourceRefKind, SourceDirectiveInfo)> {
        if let Some(directive) = self.source_directives.get(&line) {
            return Some((directive.kind.clone(), directive.directive));
        }

        if let Some(previous) = line.checked_sub(1)
            && let Some(directive) = self.source_directives.get(&previous)
            && directive.own_line
        {
            return Some((directive.kind.clone(), directive.directive));
        }

        let directive = self
            .source_directives
            .range(..line)
            .rev()
            .find(|(_, directive)| directive.own_line)
            .map(|(_, directive)| directive)?;

        match directive.kind {
            SourceRefKind::DirectiveDevNull => {
                Some((SourceRefKind::DirectiveDevNull, directive.directive))
            }
            _ => None,
        }
    }
}
