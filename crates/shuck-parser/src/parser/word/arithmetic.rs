use super::super::*;

impl<'a> Parser<'a> {
    /// Limits for an arithmetic expression parsed at the current depth.
    fn arithmetic_limits(&self) -> NestedLimits {
        NestedLimits {
            base_depth: self.current_depth,
            max_depth: self.max_depth,
            budget: Arc::clone(&self.budget),
        }
    }

    pub(in crate::parser) fn simple_subscript_arithmetic_ast(
        &self,
        text: &SourceText,
    ) -> Option<ArithmeticExprNode> {
        if !text.is_source_backed() {
            return None;
        }

        let raw = text.slice(self.input);
        if raw.is_empty() || raw.trim() != raw {
            return None;
        }

        let span = text.span();
        if raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Some(ArithmeticExprNode::new(
                ArithmeticExpr::Number(SourceText::source(span)),
                span,
            ));
        }

        if Self::is_valid_identifier(raw) {
            return Some(ArithmeticExprNode::new(
                ArithmeticExpr::Variable(Name::from(raw)),
                span,
            ));
        }

        None
    }

    pub(in crate::parser) fn parse_explicit_arithmetic_span(
        &self,
        span: Option<Span>,
        context: &'static str,
    ) -> Result<Option<ArithmeticExprNode>> {
        let Some(span) = span else {
            return Ok(None);
        };
        if span.slice(self.input).trim().is_empty() {
            return Ok(None);
        }
        arithmetic::parse_expression(
            span.slice(self.input),
            span,
            self.dialect,
            self.arithmetic_limits(),
            None,
        )
        .map(Some)
        .map_err(|error| match error {
            Error::Parse { message, .. } => self.error(format!("{context}: {message}")),
        })
    }

    pub(in crate::parser) fn parse_source_text_as_arithmetic(
        &self,
        text: &SourceText,
        word: Option<&Word>,
    ) -> Result<ArithmeticExprNode> {
        // `word` must have been parsed from this same source-backed text so its
        // part spans line up with the arithmetic lexer's token spans.
        let word = word.filter(|_| text.is_source_backed());
        let lookup = |span: Span| {
            let word = word?;
            let part = word
                .parts
                .iter()
                .find(|part| part.span == span && Self::is_reusable_arithmetic_word_part(part))?;
            Some(self.word_with_single_part(part.clone(), span))
        };
        arithmetic::parse_expression(
            text.slice(self.input),
            text.span(),
            self.dialect,
            self.arithmetic_limits(),
            word.is_some()
                .then_some(&lookup as arithmetic::KnownWordLookup<'_>),
        )
    }

    /// Expansion parts parse the same on their own as inside a larger word.
    /// Literal and quoted parts can merge with neighbouring text, so those are
    /// always reparsed.
    fn is_reusable_arithmetic_word_part(part: &WordPartNode) -> bool {
        !matches!(
            part.kind,
            WordPart::Literal(_)
                | WordPart::SingleQuoted { .. }
                | WordPart::DoubleQuoted { .. }
                | WordPart::ZshQualifiedGlob(_)
        )
    }

    pub(in crate::parser) fn maybe_parse_source_text_as_arithmetic(
        &self,
        text: &SourceText,
        word: Option<&Word>,
    ) -> Option<ArithmeticExprNode> {
        if !text.is_source_backed() {
            return None;
        }
        self.parse_source_text_as_arithmetic(text, word).ok()
    }
}
