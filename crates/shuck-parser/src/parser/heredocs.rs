use super::*;

impl<'a> Parser<'a> {
    pub(super) fn current_static_heredoc_delimiter(&mut self) -> Option<(Word, String, bool)> {
        let word = self.current_word()?;
        let raw_text = word.span.slice(self.input);
        let quoted_parts = word.has_quoted_parts();

        if let Some((text, token_quoted)) = self.current_static_token_text() {
            let quoted = quoted_parts || token_quoted || raw_text != text;
            return Some((word, text, quoted));
        }

        let text = self.literal_word_text(&word)?;
        let quoted = quoted_parts || raw_text != text;
        Some((word, text, quoted))
    }

    pub(super) fn strip_heredoc_tabs(content: String) -> String {
        let had_trailing_newline = content.ends_with('\n');
        let mut stripped: String = content
            .lines()
            .map(|line: &str| line.trim_start_matches('\t'))
            .collect::<Vec<_>>()
            .join("\n");
        if had_trailing_newline {
            stripped.push('\n');
        }
        stripped
    }

    pub(super) fn consume_heredoc_redirect(
        &mut self,
        strip_tabs: bool,
        redirects: &mut Vec<Redirect>,
        fd_var: Option<Name>,
        fd_var_span: Option<Span>,
        strict: bool,
        collect_trailing_redirects: bool,
    ) -> Result<bool> {
        let operator_span = self.current_span;
        self.advance();
        let Some((raw_delimiter, delimiter_text, quoted)) = self.current_static_heredoc_delimiter()
        else {
            if strict {
                return Err(Error::parse(
                    "expected static heredoc delimiter".to_string(),
                ));
            }
            return Ok(false);
        };

        let delimiter_span = raw_delimiter.span;
        let delimiter = HeredocDelimiter {
            raw: raw_delimiter,
            cooked: delimiter_text.clone(),
            span: delimiter_span,
            quoted,
            expands_body: !quoted,
            strip_tabs,
        };

        let heredoc = self.lexer.read_heredoc(&delimiter_text, strip_tabs);
        let content_span = heredoc.content_span;
        let content = if strip_tabs {
            Self::strip_heredoc_tabs(heredoc.content)
        } else {
            heredoc.content
        };

        let body = if quoted {
            Word::quoted_literal_with_span(content, content_span)
        } else {
            self.decode_heredoc_body_text(&content, content_span, !strip_tabs)
        };

        redirects.push(Redirect {
            fd: None,
            fd_var,
            fd_var_span,
            kind: if strip_tabs {
                RedirectKind::HereDocStrip
            } else {
                RedirectKind::HereDoc
            },
            span: operator_span.merge(delimiter.span),
            target: RedirectTarget::Heredoc(Heredoc { delimiter, body }),
        });

        // Advance so re-injected rest-of-line tokens are picked up.
        self.advance();

        if collect_trailing_redirects {
            self.collect_trailing_redirects(redirects)?;
        }

        Ok(true)
    }

    /// Parse redirections that follow a compound command (>, >>, 2>, etc.)
    pub(super) fn parse_heredoc_redirect(
        &mut self,
        strip_tabs: bool,
        redirects: &mut Vec<Redirect>,
    ) -> Result<()> {
        self.consume_heredoc_redirect(strip_tabs, redirects, None, None, true, true)?;
        Ok(())
    }

    /// Consume redirect tokens that follow a heredoc on the same line.
    pub(super) fn collect_trailing_redirects(
        &mut self,
        redirects: &mut Vec<Redirect>,
    ) -> Result<()> {
        while self.consume_non_heredoc_redirect(redirects, None, None, false)? {}
        Ok(())
    }
}
