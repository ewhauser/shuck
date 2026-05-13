use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_if(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'if'
        self.skip_newlines()?;

        // Parse condition
        let condition_start = self.current_span.start;
        let allow_brace_syntax = self.zsh_brace_if_enabled();
        let condition = self.parse_if_condition_until_body_start(allow_brace_syntax)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::stmt_seq_with_span(condition_span, condition);

        let (mut syntax, then_branch, brace_style) = if allow_brace_syntax
            && self.at(TokenKind::LeftBrace)
        {
            let (then_branch, left_brace_span, right_brace_span) = self
                .parse_brace_enclosed_stmt_seq(
                    "syntax error: empty then clause",
                    BraceBodyContext::IfClause,
                )?;
            self.record_zsh_brace_if_span(left_brace_span);
            (
                IfSyntax::Brace {
                    left_brace_span,
                    right_brace_span,
                },
                then_branch,
                true,
            )
        } else if let Some((then_branch, left_brace_span, right_brace_span)) = allow_brace_syntax
            .then(|| self.try_parse_compact_zsh_brace_body(BraceBodyContext::IfClause))
            .transpose()?
            .flatten()
        {
            self.record_zsh_brace_if_span(left_brace_span);
            (
                IfSyntax::Brace {
                    left_brace_span,
                    right_brace_span,
                },
                then_branch,
                true,
            )
        } else {
            let then_span = self.current_span;
            self.expect_keyword(Keyword::Then)?;
            self.skip_newlines()?;

            let then_start = self.current_span.start;
            let then_branch = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;
            let then_branch_span = Span::from_positions(then_start, self.current_span.start);

            let then_branch = if then_branch.is_empty() {
                if self.dialect == ShellDialect::Zsh && self.is_keyword(Keyword::Elif) {
                    Self::stmt_seq_with_span(then_branch_span, Vec::new())
                } else {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty then clause"));
                }
            } else {
                Self::stmt_seq_with_span(then_branch_span, then_branch)
            };

            (
                IfSyntax::ThenFi {
                    then_span,
                    fi_span: Span::new(),
                },
                then_branch,
                false,
            )
        };

        // Parse elif branches
        let mut elif_branches = Vec::new();
        while self.is_keyword(Keyword::Elif) {
            self.advance(); // consume 'elif'
            self.skip_newlines()?;

            let elif_condition_start = self.current_span.start;
            let elif_condition = self.parse_if_condition_until_body_start(brace_style)?;
            let elif_condition_span =
                Span::from_positions(elif_condition_start, self.current_span.start);
            let elif_condition = Self::stmt_seq_with_span(elif_condition_span, elif_condition);

            let elif_body = if brace_style {
                if self.at(TokenKind::LeftBrace) {
                    self.parse_brace_enclosed_stmt_seq(
                        "syntax error: empty elif clause",
                        BraceBodyContext::IfClause,
                    )?
                    .0
                } else if let Some((body, _, _)) =
                    self.try_parse_compact_zsh_brace_body(BraceBodyContext::IfClause)?
                {
                    body
                } else {
                    self.pop_depth();
                    return Err(self.error("expected '{' to start elif clause"));
                }
            } else {
                self.expect_keyword(Keyword::Then)?;
                let elif_body_region_start = self.current_span.start;
                self.skip_newlines()?;

                let elif_body_start = self.current_span.start;
                let elif_body = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;
                let elif_body_span = Span::from_positions(elif_body_start, self.current_span.start);

                if elif_body.is_empty() {
                    if self.dialect == ShellDialect::Zsh
                        && self.has_recorded_comment_between(
                            elif_body_region_start.offset,
                            self.current_span.start.offset,
                        )
                    {
                        Self::stmt_seq_with_span(
                            Span::from_positions(elif_body_region_start, self.current_span.start),
                            Vec::new(),
                        )
                    } else {
                        self.pop_depth();
                        return Err(self.error("syntax error: empty elif clause"));
                    }
                } else {
                    Self::stmt_seq_with_span(elif_body_span, elif_body)
                }
            };

            elif_branches.push((elif_condition, elif_body));
        }

        // Parse else branch
        let else_branch = if self.is_keyword(Keyword::Else) {
            self.advance(); // consume 'else'
            let else_region_start = self.current_span.start;
            self.skip_newlines()?;
            if brace_style {
                if self.at(TokenKind::LeftBrace) {
                    Some(
                        self.parse_brace_enclosed_stmt_seq(
                            "syntax error: empty else clause",
                            BraceBodyContext::IfClause,
                        )?
                        .0,
                    )
                } else if let Some((body, _, _)) =
                    self.try_parse_compact_zsh_brace_body(BraceBodyContext::IfClause)?
                {
                    Some(body)
                } else {
                    self.pop_depth();
                    return Err(self.error("expected '{' to start else clause"));
                }
            } else {
                let else_start = self.current_span.start;
                let branch = self.parse_compound_list(Keyword::Fi)?;
                let else_span = Span::from_positions(else_start, self.current_span.start);

                if branch.is_empty() {
                    if self.dialect == ShellDialect::Zsh
                        && self.has_recorded_comment_between(
                            else_region_start.offset,
                            self.current_span.start.offset,
                        )
                    {
                        Some(Self::stmt_seq_with_span(
                            Span::from_positions(else_region_start, self.current_span.start),
                            Vec::new(),
                        ))
                    } else {
                        self.pop_depth();
                        return Err(self.error("syntax error: empty else clause"));
                    }
                } else {
                    Some(Self::stmt_seq_with_span(else_span, branch))
                }
            }
        } else {
            None
        };

        if !brace_style {
            self.expect_keyword(Keyword::Fi)?;
            if let IfSyntax::ThenFi { then_span, .. } = syntax {
                syntax = IfSyntax::ThenFi {
                    then_span,
                    fi_span: self.current_span,
                };
            }
        }

        self.pop_depth();
        Ok(CompoundCommand::If(IfCommand {
            condition,
            then_branch,
            elif_branches,
            else_branch,
            syntax,
            span: start_span.merge(self.current_span),
        }))
    }
}
