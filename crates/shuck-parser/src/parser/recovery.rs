use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryBoundary {
    CommandBoundary,
}

impl<'a> Parser<'a> {
    pub(super) fn is_recovery_separator(boundary: RecoveryBoundary, kind: TokenKind) -> bool {
        matches!(
            (boundary, kind),
            (_, TokenKind::Newline)
                | (_, TokenKind::Semicolon)
                | (_, TokenKind::Background)
                | (_, TokenKind::BackgroundPipe)
                | (_, TokenKind::BackgroundBang)
                | (_, TokenKind::And)
                | (_, TokenKind::Or)
                | (_, TokenKind::Pipe)
                | (_, TokenKind::DoubleSemicolon)
                | (_, TokenKind::SemiAmp)
                | (_, TokenKind::SemiPipe)
                | (_, TokenKind::DoubleSemiAmp)
        )
    }

    pub(super) fn recover_to_command_boundary(&mut self, failed_offset: usize) -> bool {
        let mut advanced = false;

        while let Some(kind) = self.current_token_kind {
            if Self::is_recovery_separator(RecoveryBoundary::CommandBoundary, kind) {
                while let Some(kind) = self.current_token_kind {
                    if !Self::is_recovery_separator(RecoveryBoundary::CommandBoundary, kind) {
                        break;
                    }
                    self.advance();
                    advanced = true;
                }
                break;
            }

            let before_offset = self.current_span.start.offset;
            self.advance();
            advanced = true;

            if self.current_token.is_none() {
                break;
            }

            if self.current_span.start.offset > failed_offset
                && before_offset != self.current_span.start.offset
            {
                continue;
            }
        }

        advanced
    }

    pub(super) fn parse_impl(&mut self) -> ParseResult {
        let file_span =
            Span::from_positions(Position::new(), Position::new().advanced_by(self.input));
        let mut stmts = Vec::new();
        let mut diagnostics = Vec::new();
        let mut terminal_error = None;

        while self.current_token.is_some() {
            let checkpoint = self.current_span.start.offset;

            if let Err(error) = self.tick() {
                diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                terminal_error.get_or_insert(error);
                break;
            }
            if let Err(error) = self.skip_newlines() {
                diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                terminal_error.get_or_insert(error);
                break;
            }
            if let Err(error) = self.check_error_token() {
                diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                let recovered = self.recover_to_command_boundary(checkpoint);
                if recovered
                    || (self.current_token.is_some()
                        && self.current_span.start.offset < self.input.len())
                {
                    terminal_error.get_or_insert(error);
                }
                if !recovered && terminal_error.is_some() {
                    break;
                }
                continue;
            }
            if self.current_token.is_none() {
                break;
            }

            let command_start = self.current_span.start.offset;
            match self.parse_command_list_required() {
                Ok(command_stmts) => {
                    self.apply_stmt_list_effects(&command_stmts);
                    stmts.extend(command_stmts);
                }
                Err(error) => {
                    diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                    let recovered = self.recover_to_command_boundary(command_start);
                    if recovered
                        || (self.current_token.is_some()
                            && self.current_span.start.offset < self.input.len())
                    {
                        terminal_error.get_or_insert(error);
                    }
                    if !recovered && terminal_error.is_some() {
                        break;
                    }
                }
            }
        }

        let mut file = File {
            body: Self::stmt_seq_with_span(file_span, stmts),
            span: file_span,
        };
        self.attach_comments_to_file(&mut file);

        let status = if terminal_error.is_some() {
            ParseStatus::Fatal
        } else if diagnostics.is_empty() {
            ParseStatus::Clean
        } else {
            ParseStatus::Recovered
        };

        ParseResult {
            file,
            diagnostics,
            status,
            terminal_error,
            syntax_facts: std::mem::take(&mut self.syntax_facts),
        }
    }
}
