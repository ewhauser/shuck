use super::*;

impl<'a> Parser<'a> {
    pub(super) fn current_static_heredoc_delimiter(&mut self) -> Option<(Word, String, bool)> {
        let word = self.current_word_ref()?.clone();
        let raw_text = word.span.slice(self.input);
        let quoted_parts = word.has_quoted_parts();

        if let Some((text, token_quoted)) = self.current_static_token_text() {
            let delimiter_text = if raw_text.contains('\\') {
                unescape_heredoc_delimiter_text(raw_text)
            } else {
                text
            };
            let quoted = quoted_parts || token_quoted || raw_text != delimiter_text;
            return Some((word, delimiter_text, quoted));
        }

        let text = self.literal_word_text(&word)?;
        let delimiter_text = if raw_text.contains('\\') {
            unescape_heredoc_delimiter_text(raw_text)
        } else {
            text
        };
        let quoted = quoted_parts || raw_text != delimiter_text;
        Some((word, delimiter_text, quoted))
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
        let raw_content = heredoc.content;
        let stripped_content = strip_tabs.then(|| Self::strip_heredoc_tabs(raw_content.clone()));

        let body = if quoted {
            HeredocBody::literal_with_span(stripped_content.unwrap_or(raw_content), content_span)
                .with_source_backed(!strip_tabs)
        } else {
            let mut body = self.decode_heredoc_body_text(&raw_content, content_span, true);
            if strip_tabs {
                self.strip_tab_indentation_from_heredoc_body(&mut body);
                body.source_backed = false;
            }
            body
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

    fn strip_tab_indentation_from_heredoc_body(&self, body: &mut HeredocBody) {
        let mut at_line_start = true;
        let mut parts = Vec::with_capacity(body.parts.len());

        for mut part in body.parts.drain(..) {
            match &mut part.kind {
                HeredocBodyPart::Literal(text) => {
                    let original = text.as_str(self.input, part.span);
                    let stripped = strip_heredoc_literal_indentation(original, &mut at_line_start);
                    if stripped.is_empty() {
                        continue;
                    }
                    *text = LiteralText::owned(stripped);
                }
                _ => {
                    update_line_start_state(part.span.slice(self.input), &mut at_line_start);
                }
            }
            parts.push(part);
        }

        body.parts = parts;
    }
    /// Parse redirections that follow a compound command (>, >>, 2>, etc.)
    pub(super) fn parse_heredoc_redirect(
        &mut self,
        strip_tabs: bool,
        redirects: &mut Vec<Redirect>,
        fd_var: Option<Name>,
        fd_var_span: Option<Span>,
    ) -> Result<()> {
        self.consume_heredoc_redirect(strip_tabs, redirects, fd_var, fd_var_span, true, true)?;
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

fn unescape_heredoc_delimiter_text(text: &str) -> String {
    let mut cooked = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                cooked.push(next);
            } else {
                cooked.push(ch);
            }
            continue;
        }

        cooked.push(ch);
    }

    cooked
}

fn strip_heredoc_literal_indentation(text: &str, at_line_start: &mut bool) -> String {
    let mut stripped = String::with_capacity(text.len());

    for ch in text.chars() {
        if *at_line_start && ch == '\t' {
            continue;
        }

        stripped.push(ch);
        *at_line_start = ch == '\n';
    }

    stripped
}

fn update_line_start_state(text: &str, at_line_start: &mut bool) {
    for ch in text.chars() {
        *at_line_start = ch == '\n';
    }
}
