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

            let before_offset = self.current_span.start.offset();
            self.advance();
            advanced = true;

            if self.current_token.is_none() {
                break;
            }

            if self.current_span.start.offset() > failed_offset
                && before_offset != self.current_span.start.offset()
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
        if let Some(error) = self.oversized_input.take() {
            let Error::Parse { message, .. } = error.clone();
            diagnostics.push(ParseDiagnostic {
                message,
                span: file_span,
            });
            terminal_error = Some(error);
        }
        let base_depth = self.current_depth;
        if !self.nested {
            self.budget.anchor_stack();
        }

        while self.current_token.is_some() {
            let checkpoint = self.current_span.start.offset();
            // A failed command may unwind without popping every level it pushed.
            self.current_depth = base_depth;

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
                        && self.current_span.start.offset() < self.input.len())
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

            let command_start = self.current_span.start.offset();
            let command_start_span = self.current_span;
            match self.parse_command_list_required() {
                Ok(command_stmts) => {
                    self.apply_stmt_list_effects(&command_stmts);
                    stmts.extend(command_stmts);
                    // A nested parse may have hit a resource limit and fallen
                    // back to a partial result without failing this command.
                    if !self.nested
                        && let Some(error) = self.budget.take_limit_error()
                    {
                        let Error::Parse { message, .. } = error.clone();
                        let end = if self.current_span.start.offset()
                            >= command_start_span.start.offset()
                        {
                            self.current_span.start
                        } else {
                            command_start_span.end
                        };
                        diagnostics.push(ParseDiagnostic {
                            message,
                            span: Span::from_positions(command_start_span.start, end),
                        });
                        terminal_error.get_or_insert(error);
                    }
                }
                Err(error) => {
                    if !self.nested {
                        self.budget.take_limit_error();
                    }
                    diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                    let recovered = self.recover_to_command_boundary(command_start);
                    if recovered
                        || (self.current_token.is_some()
                            && self.current_span.start.offset() < self.input.len())
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
