use super::*;

impl<'a, 'idx, 'observer> SemanticModelBuilder<'a, 'idx, 'observer> {
    /// Classifies a `source`/`.` operand, returning its syntactic kind and the
    /// explicit directive (if any) that annotated the site.
    pub(super) fn classify_source_ref(
        &self,
        line: usize,
        word: &Word,
    ) -> (SourceRefKind, Option<SourceDirectiveInfo>, Option<Span>) {
        if let Some((kind, directive, path_range)) = self.source_directive_for_line(line) {
            let directive_path_span = self
                .source_positions_for_offsets(
                    usize::from(path_range.start()),
                    usize::from(path_range.end()),
                )
                .map(|(start, end)| Span::from_positions(start, end));
            return (kind, Some(directive), directive_path_span);
        }

        if let Some(text) = static_word_text(word, self.source) {
            return (SourceRefKind::Literal(text.as_ref().into()), None, None);
        }

        (classify_dynamic_source_word(word, self.source), None, None)
    }

    pub(super) fn source_directive_for_line(
        &self,
        line: usize,
    ) -> Option<(SourceRefKind, SourceDirectiveInfo, TextRange)> {
        if let Some(directive) = self.source_directives.get(&line) {
            return Some((
                directive.kind.clone(),
                directive.directive,
                directive.path_range,
            ));
        }

        if let Some(previous) = line.checked_sub(1)
            && let Some(directive) = self.source_directives.get(&previous)
            && directive.own_line
        {
            return Some((
                directive.kind.clone(),
                directive.directive,
                directive.path_range,
            ));
        }

        let directive = self
            .source_directives
            .range(..line)
            .rev()
            .find(|(_, directive)| directive.own_line)
            .map(|(_, directive)| directive)?;

        match directive.kind {
            SourceRefKind::DirectiveDevNull => Some((
                SourceRefKind::DirectiveDevNull,
                directive.directive,
                directive.path_range,
            )),
            _ => None,
        }
    }
}
