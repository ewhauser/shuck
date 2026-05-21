use super::super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn subscript_source_text(
        &self,
        raw: &str,
        span: Span,
    ) -> (SourceText, Option<SourceText>) {
        if raw.len() >= 2
            && ((raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\'')))
        {
            let raw_text = raw.to_string();
            let raw = if self.source_matches(span, raw) {
                SourceText::source(span)
            } else {
                SourceText::cooked(span, raw_text.clone())
            };
            let cooked = raw_text[1..raw_text.len() - 1].to_string();
            return (self.source_text(cooked, span.start, span.end), Some(raw));
        }

        let text = if self.source_matches(span, raw) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, raw.to_string())
        };
        (text, None)
    }

    pub(in crate::parser) fn subscript_from_source_text(
        &self,
        text: SourceText,
        raw: Option<SourceText>,
        interpretation: SubscriptInterpretation,
    ) -> Subscript {
        let kind = match text.slice(self.input).trim() {
            "@" => SubscriptKind::Selector(SubscriptSelector::At),
            "*" => SubscriptKind::Selector(SubscriptSelector::Star),
            _ => SubscriptKind::Ordinary,
        };
        let word_ast = if matches!(kind, SubscriptKind::Ordinary) {
            Some(self.parse_source_text_as_word(raw.as_ref().unwrap_or(&text)))
        } else {
            None
        };
        let arithmetic_ast = if matches!(kind, SubscriptKind::Ordinary) {
            self.simple_subscript_arithmetic_ast(&text)
                .or_else(|| self.maybe_parse_source_text_as_arithmetic(&text))
        } else {
            None
        };
        Subscript {
            text,
            raw,
            kind,
            interpretation,
            word_ast,
            arithmetic_ast,
        }
    }

    pub(in crate::parser) fn subscript_from_text(
        &self,
        raw: &str,
        span: Span,
        interpretation: SubscriptInterpretation,
    ) -> Subscript {
        let (text, raw) = self.subscript_source_text(raw, span);
        self.subscript_from_source_text(text, raw, interpretation)
    }

    pub(in crate::parser) fn var_ref(
        &self,
        name: impl Into<Name>,
        name_span: Span,
        subscript: Option<Subscript>,
        span: Span,
    ) -> VarRef {
        VarRef {
            name: name.into(),
            name_span,
            subscript: subscript.map(Box::new),
            span,
        }
    }

    pub(in crate::parser) fn parameter_var_ref(
        &self,
        part_start: Position,
        prefix: &str,
        name: &str,
        subscript: Option<Subscript>,
        part_end: Position,
    ) -> VarRef {
        let name_start = part_start.advanced_by(prefix);
        let name_span = Span::from_positions(name_start, name_start.advanced_by(name));
        self.var_ref(
            Name::from(name),
            name_span,
            subscript,
            Span::from_positions(part_start, part_end),
        )
    }
}
