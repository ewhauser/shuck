use super::super::*;

impl<'a> Parser<'a> {
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
            self.max_depth.saturating_sub(self.current_depth),
            self.fuel,
        )
        .map(Some)
        .map_err(|error| match error {
            Error::Parse { message, .. } => self.error(format!("{context}: {message}")),
        })
    }

    pub(in crate::parser) fn parse_source_text_as_arithmetic(
        &self,
        text: &SourceText,
    ) -> Result<ArithmeticExprNode> {
        arithmetic::parse_expression(
            text.slice(self.input),
            text.span(),
            self.dialect,
            self.max_depth.saturating_sub(self.current_depth),
            self.fuel,
        )
    }

    pub(in crate::parser) fn maybe_parse_source_text_as_arithmetic(
        &self,
        text: &SourceText,
    ) -> Option<ArithmeticExprNode> {
        if !text.is_source_backed() {
            return None;
        }
        self.parse_source_text_as_arithmetic(text).ok()
    }
}
