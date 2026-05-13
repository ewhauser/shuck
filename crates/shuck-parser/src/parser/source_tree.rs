use super::*;

impl<'a> Parser<'a> {
    pub(super) fn nested_stmt_seq_from_source(&mut self, source: &str, base: Position) -> StmtSeq {
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        let nested_profile = self
            .current_zsh_options()
            .cloned()
            .map(|options| ShellProfile::with_zsh_options(self.dialect, options))
            .unwrap_or_else(|| self.shell_profile.clone());
        let inner_parser =
            Parser::with_limits_and_profile(source, remaining_depth, self.fuel, nested_profile);
        let mut output = inner_parser.parse();
        if output.is_ok() {
            Self::materialize_stmt_seq_source_backing(&mut output.file.body, source);
            Self::rebase_file(&mut output.file, base);
            output.file.body
        } else {
            StmtSeq {
                leading_comments: Vec::new(),
                stmts: Vec::new(),
                trailing_comments: Vec::new(),
                span: Span::from_positions(base, base),
            }
        }
    }

    pub(super) fn nested_stmt_seq_from_current_input(
        &mut self,
        start: Position,
        end: Position,
    ) -> StmtSeq {
        if start.offset > end.offset || end.offset > self.input.len() {
            return StmtSeq {
                leading_comments: Vec::new(),
                stmts: Vec::new(),
                trailing_comments: Vec::new(),
                span: Span::from_positions(start, start),
            };
        }
        let source = &self.input[start.offset..end.offset];
        self.nested_stmt_seq_from_source(source, start)
    }

    pub(super) fn merge_optional_span(primary: Span, other: Span) -> Span {
        if other == Span::new() {
            primary
        } else {
            primary.merge(other)
        }
    }

    pub(super) fn redirect_span(operator_span: Span, target: &Word) -> Span {
        Self::merge_optional_span(operator_span, target.span)
    }

    pub(super) fn optional_span(start: Position, end: Position) -> Option<Span> {
        (start.offset < end.offset).then(|| Span::from_positions(start, end))
    }

    pub(super) fn split_nested_arithmetic_close(&mut self, context: &'static str) -> Result<Span> {
        let right_paren_start = self.current_span.start.advanced_by(")");
        self.advance();

        if self.at(TokenKind::RightParen) {
            let right_paren_span = Span::from_positions(right_paren_start, self.current_span.end);
            self.advance();
            Ok(right_paren_span)
        } else {
            Err(Error::parse(format!(
                "expected ')' after '))' in {context}"
            )))
        }
    }

    pub(super) fn split_double_semicolon(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by(";");
        (
            Span::from_positions(span.start, middle),
            Span::from_positions(middle, span.end),
        )
    }

    pub(super) fn split_double_left_paren(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by("(");
        (
            Span::from_positions(span.start, middle),
            Span::from_positions(middle, span.end),
        )
    }

    pub(super) fn split_double_right_paren(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by(")");
        (
            Span::from_positions(span.start, middle),
            Span::from_positions(middle, span.end),
        )
    }

    pub(super) fn record_arithmetic_for_separator(
        semicolon_span: Span,
        segment_start: &mut Position,
        init_span: &mut Option<Span>,
        first_semicolon_span: &mut Option<Span>,
        condition_span: &mut Option<Span>,
        second_semicolon_span: &mut Option<Span>,
    ) -> Result<()> {
        if first_semicolon_span.is_none() {
            *init_span = Self::optional_span(*segment_start, semicolon_span.start);
            *first_semicolon_span = Some(semicolon_span);
            *segment_start = semicolon_span.end;
            return Ok(());
        }

        if second_semicolon_span.is_none() {
            *condition_span = Self::optional_span(*segment_start, semicolon_span.start);
            *second_semicolon_span = Some(semicolon_span);
            *segment_start = semicolon_span.end;
            return Ok(());
        }

        Err(Error::parse(
            "unexpected ';' in arithmetic for header".to_string(),
        ))
    }

    pub(super) fn rebase_file(file: &mut File, base: Position) {
        file.span = file.span.rebased(base);
        Self::rebase_stmt_seq(&mut file.body, base);
    }

    pub(super) fn rebase_comments(comments: &mut [Comment], base: Position) {
        let base_offset = TextSize::new(base.offset as u32);
        for comment in comments {
            comment.range = comment.range.offset_by(base_offset);
        }
    }

    pub(super) fn rebase_stmt_seq(sequence: &mut StmtSeq, base: Position) {
        sequence.span = sequence.span.rebased(base);
        Self::rebase_comments(&mut sequence.leading_comments, base);
        for stmt in &mut sequence.stmts {
            Self::rebase_stmt(stmt, base);
        }
        Self::rebase_comments(&mut sequence.trailing_comments, base);
    }

    pub(super) fn rebase_stmt(stmt: &mut Stmt, base: Position) {
        stmt.span = stmt.span.rebased(base);
        Self::rebase_comments(&mut stmt.leading_comments, base);
        stmt.terminator_span = stmt.terminator_span.map(|span| span.rebased(base));
        if let Some(comment) = &mut stmt.inline_comment {
            let base_offset = TextSize::new(base.offset as u32);
            comment.range = comment.range.offset_by(base_offset);
        }
        Self::rebase_redirects(&mut stmt.redirects, base);
        Self::rebase_ast_command(&mut stmt.command, base);
    }

    pub(super) fn rebase_ast_command(command: &mut AstCommand, base: Position) {
        match command {
            AstCommand::Simple(simple) => {
                simple.span = simple.span.rebased(base);
                Self::rebase_word(&mut simple.name, base);
                Self::rebase_words(&mut simple.args, base);
                Self::rebase_assignments(&mut simple.assignments, base);
            }
            AstCommand::Builtin(builtin) => match builtin {
                AstBuiltinCommand::Break(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(depth) = &mut command.depth {
                        Self::rebase_word(depth, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
                AstBuiltinCommand::Continue(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(depth) = &mut command.depth {
                        Self::rebase_word(depth, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
                AstBuiltinCommand::Return(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(code) = &mut command.code {
                        Self::rebase_word(code, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
                AstBuiltinCommand::Exit(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(code) = &mut command.code {
                        Self::rebase_word(code, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
            },
            AstCommand::Decl(decl) => {
                decl.span = decl.span.rebased(base);
                decl.variant_span = decl.variant_span.rebased(base);
                Self::rebase_assignments(&mut decl.assignments, base);
                for operand in &mut decl.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            Self::rebase_word(word, base);
                        }
                        DeclOperand::Name(name) => Self::rebase_var_ref(name, base),
                        DeclOperand::Assignment(assignment) => {
                            Self::rebase_assignments(std::slice::from_mut(assignment), base);
                        }
                    }
                }
            }
            AstCommand::Binary(binary) => {
                binary.span = binary.span.rebased(base);
                binary.op_span = binary.op_span.rebased(base);
                Self::rebase_stmt(binary.left.as_mut(), base);
                Self::rebase_stmt(binary.right.as_mut(), base);
            }
            AstCommand::Compound(compound) => Self::rebase_compound(compound, base),
            AstCommand::Function(function) => {
                function.span = function.span.rebased(base);
                if let Some(span) = &mut function.header.function_keyword_span {
                    *span = span.rebased(base);
                }
                if let Some(span) = &mut function.header.trailing_parens_span {
                    *span = span.rebased(base);
                }
                for entry in &mut function.header.entries {
                    Self::rebase_word(&mut entry.word, base);
                }
                Self::rebase_stmt(function.body.as_mut(), base);
            }
            AstCommand::AnonymousFunction(function) => {
                function.span = function.span.rebased(base);
                function.surface = match function.surface {
                    AnonymousFunctionSurface::FunctionKeyword {
                        function_keyword_span,
                    } => AnonymousFunctionSurface::FunctionKeyword {
                        function_keyword_span: function_keyword_span.rebased(base),
                    },
                    AnonymousFunctionSurface::Parens { parens_span } => {
                        AnonymousFunctionSurface::Parens {
                            parens_span: parens_span.rebased(base),
                        }
                    }
                };
                Self::rebase_stmt(function.body.as_mut(), base);
                Self::rebase_words(&mut function.args, base);
            }
        }
    }

    pub(super) fn rebase_subscript(subscript: &mut Subscript, base: Position) {
        subscript.text.rebased(base);
        if let Some(raw) = &mut subscript.raw {
            raw.rebased(base);
        }
        if let Some(word) = &mut subscript.word_ast {
            Self::rebase_word(word, base);
        }
        if let Some(expr) = &mut subscript.arithmetic_ast {
            Self::rebase_arithmetic_expr(expr, base);
        }
    }

    pub(super) fn rebase_var_ref(reference: &mut VarRef, base: Position) {
        reference.span = reference.span.rebased(base);
        reference.name_span = reference.name_span.rebased(base);
        if let Some(subscript) = &mut reference.subscript {
            Self::rebase_subscript(subscript, base);
        }
    }

    pub(super) fn rebase_array_expr(array: &mut ArrayExpr, base: Position) {
        array.span = array.span.rebased(base);
        for element in &mut array.elements {
            match element {
                ArrayElem::Sequential(word) => Self::rebase_word(word, base),
                ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                    Self::rebase_subscript(key, base);
                    Self::rebase_word(value, base);
                }
            }
        }
    }

    pub(super) fn rebase_compound(compound: &mut CompoundCommand, base: Position) {
        match compound {
            CompoundCommand::If(command) => {
                command.span = command.span.rebased(base);
                command.syntax = match command.syntax {
                    IfSyntax::ThenFi { then_span, fi_span } => IfSyntax::ThenFi {
                        then_span: then_span.rebased(base),
                        fi_span: fi_span.rebased(base),
                    },
                    IfSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    } => IfSyntax::Brace {
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.condition, base);
                Self::rebase_stmt_seq(&mut command.then_branch, base);
                for (condition, body) in &mut command.elif_branches {
                    Self::rebase_stmt_seq(condition, base);
                    Self::rebase_stmt_seq(body, base);
                }
                if let Some(else_branch) = &mut command.else_branch {
                    Self::rebase_stmt_seq(else_branch, base);
                }
            }
            CompoundCommand::For(command) => {
                command.span = command.span.rebased(base);
                for target in &mut command.targets {
                    target.span = target.span.rebased(base);
                }
                if let Some(words) = &mut command.words {
                    Self::rebase_words(words, base);
                }
                command.syntax = match command.syntax {
                    ForSyntax::InDoDone {
                        in_span,
                        do_span,
                        done_span,
                    } => ForSyntax::InDoDone {
                        in_span: in_span.map(|span| span.rebased(base)),
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                    ForSyntax::InDirect { in_span } => ForSyntax::InDirect {
                        in_span: in_span.map(|span| span.rebased(base)),
                    },
                    ForSyntax::InBrace {
                        in_span,
                        left_brace_span,
                        right_brace_span,
                    } => ForSyntax::InBrace {
                        in_span: in_span.map(|span| span.rebased(base)),
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                    ForSyntax::ParenDoDone {
                        left_paren_span,
                        right_paren_span,
                        do_span,
                        done_span,
                    } => ForSyntax::ParenDoDone {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                    ForSyntax::ParenDirect {
                        left_paren_span,
                        right_paren_span,
                    } => ForSyntax::ParenDirect {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                    },
                    ForSyntax::ParenBrace {
                        left_paren_span,
                        right_paren_span,
                        left_brace_span,
                        right_brace_span,
                    } => ForSyntax::ParenBrace {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Repeat(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_word(&mut command.count, base);
                command.syntax = match command.syntax {
                    RepeatSyntax::DoDone { do_span, done_span } => RepeatSyntax::DoDone {
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                    RepeatSyntax::Direct => RepeatSyntax::Direct,
                    RepeatSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    } => RepeatSyntax::Brace {
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Foreach(command) => {
                command.span = command.span.rebased(base);
                command.variable_span = command.variable_span.rebased(base);
                Self::rebase_words(&mut command.words, base);
                command.syntax = match command.syntax {
                    ForeachSyntax::ParenBrace {
                        left_paren_span,
                        right_paren_span,
                        left_brace_span,
                        right_brace_span,
                    } => ForeachSyntax::ParenBrace {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                    ForeachSyntax::InDoDone {
                        in_span,
                        do_span,
                        done_span,
                    } => ForeachSyntax::InDoDone {
                        in_span: in_span.rebased(base),
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::ArithmeticFor(command) => {
                command.span = command.span.rebased(base);
                command.left_paren_span = command.left_paren_span.rebased(base);
                command.init_span = command.init_span.map(|span| span.rebased(base));
                if let Some(expr) = &mut command.init_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                command.first_semicolon_span = command.first_semicolon_span.rebased(base);
                command.condition_span = command.condition_span.map(|span| span.rebased(base));
                if let Some(expr) = &mut command.condition_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                command.second_semicolon_span = command.second_semicolon_span.rebased(base);
                command.step_span = command.step_span.map(|span| span.rebased(base));
                if let Some(expr) = &mut command.step_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                command.right_paren_span = command.right_paren_span.rebased(base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::While(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_stmt_seq(&mut command.condition, base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Until(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_stmt_seq(&mut command.condition, base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Case(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_word(&mut command.word, base);
                for case in &mut command.cases {
                    Self::rebase_patterns(&mut case.patterns, base);
                    Self::rebase_stmt_seq(&mut case.body, base);
                }
            }
            CompoundCommand::Select(command) => {
                command.span = command.span.rebased(base);
                command.variable_span = command.variable_span.rebased(base);
                Self::rebase_words(&mut command.words, base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                Self::rebase_stmt_seq(commands, base);
            }
            CompoundCommand::Arithmetic(command) => {
                command.span = command.span.rebased(base);
                command.left_paren_span = command.left_paren_span.rebased(base);
                command.expr_span = command.expr_span.map(|span| span.rebased(base));
                if let Some(expr) = &mut command.expr_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                command.right_paren_span = command.right_paren_span.rebased(base);
            }
            CompoundCommand::Time(command) => {
                command.span = command.span.rebased(base);
                if let Some(inner) = &mut command.command {
                    Self::rebase_stmt(inner.as_mut(), base);
                }
            }
            CompoundCommand::Conditional(command) => {
                command.span = command.span.rebased(base);
                command.left_bracket_span = command.left_bracket_span.rebased(base);
                command.right_bracket_span = command.right_bracket_span.rebased(base);
                Self::rebase_conditional_expr(&mut command.expression, base);
            }
            CompoundCommand::Coproc(command) => {
                command.span = command.span.rebased(base);
                command.name_span = command.name_span.map(|span| span.rebased(base));
                Self::rebase_stmt(command.body.as_mut(), base);
            }
            CompoundCommand::Always(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_stmt_seq(&mut command.body, base);
                Self::rebase_stmt_seq(&mut command.always_body, base);
            }
        }
    }

    pub(super) fn materialize_stmt_seq_source_backing(sequence: &mut StmtSeq, source: &str) {
        for stmt in &mut sequence.stmts {
            Self::materialize_stmt_source_backing(stmt, source);
        }
    }

    pub(super) fn materialize_stmt_source_backing(stmt: &mut Stmt, source: &str) {
        Self::materialize_ast_command_source_backing(&mut stmt.command, source);
    }

    pub(super) fn materialize_ast_command_source_backing(command: &mut AstCommand, source: &str) {
        match command {
            AstCommand::Simple(simple) => {
                Self::materialize_word_source_backing(&mut simple.name, source);
            }
            AstCommand::Builtin(_) | AstCommand::Decl(_) => {}
            AstCommand::Binary(binary) => {
                Self::materialize_stmt_source_backing(binary.left.as_mut(), source);
                Self::materialize_stmt_source_backing(binary.right.as_mut(), source);
            }
            AstCommand::Compound(compound) => {
                Self::materialize_compound_source_backing(compound, source);
            }
            AstCommand::Function(function) => {
                Self::materialize_stmt_source_backing(function.body.as_mut(), source);
            }
            AstCommand::AnonymousFunction(function) => {
                Self::materialize_stmt_source_backing(function.body.as_mut(), source);
            }
        }
    }

    pub(super) fn materialize_compound_source_backing(
        compound: &mut CompoundCommand,
        source: &str,
    ) {
        match compound {
            CompoundCommand::If(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.condition, source);
                Self::materialize_stmt_seq_source_backing(&mut command.then_branch, source);
                for (condition, body) in &mut command.elif_branches {
                    Self::materialize_stmt_seq_source_backing(condition, source);
                    Self::materialize_stmt_seq_source_backing(body, source);
                }
                if let Some(else_branch) = &mut command.else_branch {
                    Self::materialize_stmt_seq_source_backing(else_branch, source);
                }
            }
            CompoundCommand::For(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::Repeat(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::Foreach(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::ArithmeticFor(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::While(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.condition, source);
                Self::materialize_stmt_seq_source_backing(&mut command.body, source);
            }
            CompoundCommand::Until(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.condition, source);
                Self::materialize_stmt_seq_source_backing(&mut command.body, source);
            }
            CompoundCommand::Case(command) => {
                for case in &mut command.cases {
                    Self::materialize_stmt_seq_source_backing(&mut case.body, source);
                }
            }
            CompoundCommand::Select(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                Self::materialize_stmt_seq_source_backing(commands, source);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(inner) = &mut command.command {
                    Self::materialize_stmt_source_backing(inner.as_mut(), source);
                }
            }
            CompoundCommand::Conditional(_) => {}
            CompoundCommand::Coproc(command) => {
                Self::materialize_stmt_source_backing(command.body.as_mut(), source);
            }
            CompoundCommand::Always(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source);
                Self::materialize_stmt_seq_source_backing(&mut command.always_body, source);
            }
        }
    }

    pub(super) fn rebase_words(words: &mut [Word], base: Position) {
        for word in words {
            Self::rebase_word(word, base);
        }
    }

    pub(super) fn rebase_patterns(patterns: &mut [Pattern], base: Position) {
        for pattern in patterns {
            Self::rebase_pattern(pattern, base);
        }
    }

    pub(super) fn materialize_literal_text_source_backing(
        text: &mut LiteralText,
        span: Span,
        source: &str,
    ) {
        match text {
            LiteralText::Source => {
                *text = LiteralText::owned(span.slice(source).to_string());
            }
            LiteralText::CookedSource(cooked) => {
                *text = LiteralText::owned(cooked.to_string());
            }
            LiteralText::Owned(_) => {}
        }
    }

    pub(super) fn materialize_source_text_source_backing(text: &mut SourceText, source: &str) {
        if text.is_source_backed() {
            let span = text.span();
            let cooked = text.slice(source).to_string();
            *text = SourceText::cooked(span, cooked);
        }
    }

    pub(super) fn materialize_word_source_backing(word: &mut Word, source: &str) {
        for part in &mut word.parts {
            Self::materialize_word_part_source_backing(part, source);
        }
    }

    pub(super) fn materialize_pattern_source_backing(pattern: &mut Pattern, source: &str) {
        for part in &mut pattern.parts {
            Self::materialize_pattern_part_source_backing(part, source);
        }
    }

    pub(super) fn materialize_pattern_part_source_backing(
        part: &mut PatternPartNode,
        source: &str,
    ) {
        match &mut part.kind {
            PatternPart::Literal(text) => {
                Self::materialize_literal_text_source_backing(text, part.span, source);
            }
            PatternPart::CharClass(text) => {
                Self::materialize_source_text_source_backing(text, source);
            }
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    Self::materialize_pattern_source_backing(pattern, source);
                }
            }
            PatternPart::Word(word) => Self::materialize_word_source_backing(word, source),
            PatternPart::AnyString | PatternPart::AnyChar => {}
        }
    }

    pub(super) fn materialize_word_part_source_backing(part: &mut WordPartNode, source: &str) {
        match &mut part.kind {
            WordPart::Literal(text) => {
                Self::materialize_literal_text_source_backing(text, part.span, source);
            }
            WordPart::ZshQualifiedGlob(glob) => {
                Self::materialize_zsh_qualified_glob_source_backing(glob, source);
            }
            WordPart::SingleQuoted { value, .. } => {
                Self::materialize_source_text_source_backing(value, source);
            }
            WordPart::DoubleQuoted { parts, .. } => {
                for part in parts {
                    Self::materialize_word_part_source_backing(part, source);
                }
            }
            WordPart::Parameter(parameter) => {
                Self::materialize_source_text_source_backing(&mut parameter.raw_body, source);
                Self::materialize_parameter_expansion_syntax_source_backing(
                    &mut parameter.syntax,
                    source,
                );
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::materialize_var_ref_source_backing(reference, source);
                Self::materialize_parameter_operator_source_backing(operator, source);
                if let Some(operand) = operand {
                    Self::materialize_source_text_source_backing(operand, source);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::materialize_word_source_backing(word_ast, source);
                }
            }
            WordPart::ArrayAccess(reference)
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => {
                Self::materialize_var_ref_source_backing(reference, source);
            }
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            } => {
                Self::materialize_var_ref_source_backing(reference, source);
                Self::materialize_source_text_source_backing(offset, source);
                Self::materialize_word_source_backing(offset_word_ast, source);
                if let Some(expr) = offset_ast {
                    Self::materialize_arithmetic_expr_source_backing(expr, source);
                }
                if let Some(length) = length {
                    Self::materialize_source_text_source_backing(length, source);
                }
                if let Some(word_ast) = length_word_ast {
                    Self::materialize_word_source_backing(word_ast, source);
                }
                if let Some(expr) = length_ast {
                    Self::materialize_arithmetic_expr_source_backing(expr, source);
                }
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::materialize_var_ref_source_backing(reference, source);
                if let Some(operator) = operator {
                    Self::materialize_parameter_operator_source_backing(operator, source);
                }
                if let Some(operand) = operand {
                    Self::materialize_source_text_source_backing(operand, source);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::materialize_word_source_backing(word_ast, source);
                }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                Self::materialize_source_text_source_backing(expression, source);
                Self::materialize_word_source_backing(expression_word_ast, source);
                if let Some(expr) = expression_ast {
                    Self::materialize_arithmetic_expr_source_backing(expr, source);
                }
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. } => {}
        }
    }

    pub(super) fn materialize_var_ref_source_backing(reference: &mut VarRef, source: &str) {
        if let Some(subscript) = &mut reference.subscript {
            Self::materialize_subscript_source_backing(subscript, source);
        }
    }

    pub(super) fn materialize_subscript_source_backing(subscript: &mut Subscript, source: &str) {
        Self::materialize_source_text_source_backing(&mut subscript.text, source);
        if let Some(raw) = &mut subscript.raw {
            Self::materialize_source_text_source_backing(raw, source);
        }
        if let Some(word_ast) = &mut subscript.word_ast {
            Self::materialize_word_source_backing(word_ast, source);
        }
        if let Some(expr) = &mut subscript.arithmetic_ast {
            Self::materialize_arithmetic_expr_source_backing(expr, source);
        }
    }

    pub(super) fn materialize_zsh_qualified_glob_source_backing(
        glob: &mut ZshQualifiedGlob,
        source: &str,
    ) {
        for segment in &mut glob.segments {
            match segment {
                ZshGlobSegment::Pattern(pattern) => {
                    Self::materialize_pattern_source_backing(pattern, source);
                }
                ZshGlobSegment::InlineControl(_) => {}
            }
        }
        if let Some(qualifiers) = &mut glob.qualifiers {
            for fragment in &mut qualifiers.fragments {
                match fragment {
                    ZshGlobQualifier::LetterSequence { text, .. } => {
                        Self::materialize_source_text_source_backing(text, source);
                    }
                    ZshGlobQualifier::NumericArgument { start, end, .. } => {
                        Self::materialize_source_text_source_backing(start, source);
                        if let Some(end) = end {
                            Self::materialize_source_text_source_backing(end, source);
                        }
                    }
                    ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => {}
                }
            }
        }
    }

    pub(super) fn materialize_parameter_expansion_syntax_source_backing(
        syntax: &mut ParameterExpansionSyntax,
        source: &str,
    ) {
        match syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Transformation { reference, .. } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                }
                BourneParameterExpansion::Indirect {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                    if let Some(operator) = operator {
                        Self::materialize_parameter_operator_source_backing(operator, source);
                    }
                    if let Some(operand) = operand {
                        Self::materialize_source_text_source_backing(operand, source);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::materialize_word_source_backing(word_ast, source);
                    }
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset,
                    offset_ast,
                    offset_word_ast,
                    length,
                    length_ast,
                    length_word_ast,
                } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                    Self::materialize_source_text_source_backing(offset, source);
                    Self::materialize_word_source_backing(offset_word_ast, source);
                    if let Some(expr) = offset_ast {
                        Self::materialize_arithmetic_expr_source_backing(expr, source);
                    }
                    if let Some(length) = length {
                        Self::materialize_source_text_source_backing(length, source);
                    }
                    if let Some(word_ast) = length_word_ast {
                        Self::materialize_word_source_backing(word_ast, source);
                    }
                    if let Some(expr) = length_ast {
                        Self::materialize_arithmetic_expr_source_backing(expr, source);
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                    Self::materialize_parameter_operator_source_backing(operator, source);
                    if let Some(operand) = operand {
                        Self::materialize_source_text_source_backing(operand, source);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::materialize_word_source_backing(word_ast, source);
                    }
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &mut syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        Self::materialize_var_ref_source_backing(reference, source);
                    }
                    ZshExpansionTarget::Word(word) => {
                        Self::materialize_word_source_backing(word, source);
                    }
                    ZshExpansionTarget::Nested(parameter) => {
                        Self::materialize_source_text_source_backing(
                            &mut parameter.raw_body,
                            source,
                        );
                        Self::materialize_parameter_expansion_syntax_source_backing(
                            &mut parameter.syntax,
                            source,
                        );
                    }
                    ZshExpansionTarget::Empty => {}
                }
                for modifier in &mut syntax.modifiers {
                    if let Some(argument) = &mut modifier.argument {
                        Self::materialize_source_text_source_backing(argument, source);
                    }
                    if let Some(argument_word_ast) = &mut modifier.argument_word_ast {
                        Self::materialize_word_source_backing(argument_word_ast, source);
                    }
                }
                if let Some(operation) = &mut syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::Defaulting {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::TrimOperation {
                            operand,
                            operand_word_ast,
                            ..
                        } => {
                            Self::materialize_source_text_source_backing(operand, source);
                            Self::materialize_word_source_backing(operand_word_ast, source);
                        }
                        ZshExpansionOperation::Unknown { text, word_ast } => {
                            Self::materialize_source_text_source_backing(text, source);
                            Self::materialize_word_source_backing(word_ast, source);
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            pattern_word_ast,
                            replacement,
                            replacement_word_ast,
                            ..
                        } => {
                            Self::materialize_source_text_source_backing(pattern, source);
                            Self::materialize_word_source_backing(pattern_word_ast, source);
                            if let Some(replacement) = replacement {
                                Self::materialize_source_text_source_backing(replacement, source);
                            }
                            if let Some(replacement_word_ast) = replacement_word_ast {
                                Self::materialize_word_source_backing(replacement_word_ast, source);
                            }
                        }
                        ZshExpansionOperation::Slice {
                            offset,
                            offset_word_ast,
                            length,
                            length_word_ast,
                        } => {
                            Self::materialize_source_text_source_backing(offset, source);
                            Self::materialize_word_source_backing(offset_word_ast, source);
                            if let Some(length) = length {
                                Self::materialize_source_text_source_backing(length, source);
                            }
                            if let Some(length_word_ast) = length_word_ast {
                                Self::materialize_word_source_backing(length_word_ast, source);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn materialize_parameter_operator_source_backing(
        operator: &mut ParameterOp,
        source: &str,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                Self::materialize_pattern_source_backing(pattern, source);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                replacement_word_ast,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                replacement_word_ast,
            } => {
                Self::materialize_pattern_source_backing(pattern, source);
                Self::materialize_source_text_source_backing(replacement, source);
                Self::materialize_word_source_backing(replacement_word_ast, source);
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    pub(super) fn materialize_arithmetic_expr_source_backing(
        expr: &mut ArithmeticExprNode,
        source: &str,
    ) {
        match &mut expr.kind {
            ArithmeticExpr::Number(text) => {
                Self::materialize_source_text_source_backing(text, source);
            }
            ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => {
                Self::materialize_arithmetic_expr_source_backing(index, source);
            }
            ArithmeticExpr::ShellWord(word) => {
                Self::materialize_word_source_backing(word, source);
            }
            ArithmeticExpr::Parenthesized { expression } => {
                Self::materialize_arithmetic_expr_source_backing(expression, source);
            }
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                Self::materialize_arithmetic_expr_source_backing(expr, source);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                Self::materialize_arithmetic_expr_source_backing(left, source);
                Self::materialize_arithmetic_expr_source_backing(right, source);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::materialize_arithmetic_expr_source_backing(condition, source);
                Self::materialize_arithmetic_expr_source_backing(then_expr, source);
                Self::materialize_arithmetic_expr_source_backing(else_expr, source);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                Self::materialize_arithmetic_lvalue_source_backing(target, source);
                Self::materialize_arithmetic_expr_source_backing(value, source);
            }
        }
    }

    pub(super) fn materialize_arithmetic_lvalue_source_backing(
        target: &mut ArithmeticLvalue,
        source: &str,
    ) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => {
                Self::materialize_arithmetic_expr_source_backing(index, source);
            }
        }
    }

    pub(super) fn rebase_word(word: &mut Word, base: Position) {
        word.span = word.span.rebased(base);
        for brace in &mut word.brace_syntax {
            brace.span = brace.span.rebased(base);
        }
        Self::rebase_word_parts(&mut word.parts, base);
    }

    pub(super) fn rebase_heredoc_body(body: &mut HeredocBody, base: Position) {
        body.span = body.span.rebased(base);
        for part in &mut body.parts {
            Self::rebase_heredoc_body_part(part, base);
        }
    }

    pub(super) fn rebase_pattern(pattern: &mut Pattern, base: Position) {
        pattern.span = pattern.span.rebased(base);
        Self::rebase_pattern_parts(&mut pattern.parts, base);
    }

    pub(super) fn rebase_word_parts(parts: &mut [WordPartNode], base: Position) {
        for part in parts {
            Self::rebase_word_part(part, base);
        }
    }

    pub(super) fn rebase_pattern_parts(parts: &mut [PatternPartNode], base: Position) {
        for part in parts {
            part.span = part.span.rebased(base);
            match &mut part.kind {
                PatternPart::CharClass(text) => text.rebased(base),
                PatternPart::Group { patterns, .. } => Self::rebase_patterns(patterns, base),
                PatternPart::Word(word) => Self::rebase_word(word, base),
                PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => {}
            }
        }
    }

    pub(super) fn rebase_heredoc_body_part(part: &mut HeredocBodyPartNode, base: Position) {
        part.span = part.span.rebased(base);
        match &mut part.kind {
            HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
            HeredocBodyPart::CommandSubstitution { body, .. } => Self::rebase_stmt_seq(body, base),
            HeredocBodyPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                expression.rebased(base);
                Self::rebase_word(expression_word_ast, base);
                if let Some(expr) = expression_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            HeredocBodyPart::Parameter(parameter) => {
                parameter.span = parameter.span.rebased(base);
                parameter.raw_body.rebased(base);
                Self::rebase_parameter_expansion_syntax(&mut parameter.syntax, base);
            }
        }
    }

    pub(super) fn rebase_word_part(part: &mut WordPartNode, base: Position) {
        part.span = part.span.rebased(base);
        match &mut part.kind {
            WordPart::ZshQualifiedGlob(glob) => Self::rebase_zsh_qualified_glob(glob, base),
            WordPart::SingleQuoted { value, .. } => value.rebased(base),
            WordPart::DoubleQuoted { parts, .. } => Self::rebase_word_parts(parts, base),
            WordPart::Parameter(parameter) => {
                parameter.span = parameter.span.rebased(base);
                parameter.raw_body.rebased(base);
                Self::rebase_parameter_expansion_syntax(&mut parameter.syntax, base);
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
                match operator.as_mut() {
                    ParameterOp::RemovePrefixShort { pattern }
                    | ParameterOp::RemovePrefixLong { pattern }
                    | ParameterOp::RemoveSuffixShort { pattern }
                    | ParameterOp::RemoveSuffixLong { pattern } => {
                        Self::rebase_pattern(pattern, base);
                    }
                    ParameterOp::ReplaceFirst {
                        pattern,
                        replacement,
                        ..
                    }
                    | ParameterOp::ReplaceAll {
                        pattern,
                        replacement,
                        ..
                    } => {
                        Self::rebase_pattern(pattern, base);
                        replacement.rebased(base);
                    }
                    ParameterOp::UseDefault
                    | ParameterOp::AssignDefault
                    | ParameterOp::UseReplacement
                    | ParameterOp::Error
                    | ParameterOp::UpperFirst
                    | ParameterOp::UpperAll
                    | ParameterOp::LowerFirst
                    | ParameterOp::LowerAll => {}
                }
                if let Some(operand) = operand {
                    operand.rebased(base);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::rebase_word(word_ast, base);
                }
            }
            WordPart::ArrayAccess(reference)
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => Self::rebase_var_ref(reference, base),
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
                offset.rebased(base);
                Self::rebase_word(offset_word_ast, base);
                if let Some(expr) = offset_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                if let Some(length) = length {
                    length.rebased(base);
                }
                if let Some(word_ast) = length_word_ast {
                    Self::rebase_word(word_ast, base);
                }
                if let Some(expr) = length_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
                if let Some(operator) = operator {
                    Self::rebase_parameter_operator(operator, base);
                }
                if let Some(operand) = operand {
                    operand.rebased(base);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::rebase_word(word_ast, base);
                }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                expression.rebased(base);
                Self::rebase_word(expression_word_ast, base);
                if let Some(expr) = expression_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => Self::rebase_stmt_seq(body, base),
            WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {}
        }
    }

    pub(super) fn rebase_zsh_qualified_glob(glob: &mut ZshQualifiedGlob, base: Position) {
        glob.span = glob.span.rebased(base);
        for segment in &mut glob.segments {
            Self::rebase_zsh_glob_segment(segment, base);
        }
        if let Some(qualifiers) = &mut glob.qualifiers {
            Self::rebase_zsh_glob_qualifier_group(qualifiers, base);
        }
    }

    pub(super) fn rebase_zsh_glob_segment(segment: &mut ZshGlobSegment, base: Position) {
        match segment {
            ZshGlobSegment::Pattern(pattern) => Self::rebase_pattern(pattern, base),
            ZshGlobSegment::InlineControl(control) => {
                Self::rebase_zsh_inline_glob_control(control, base)
            }
        }
    }

    pub(super) fn rebase_zsh_inline_glob_control(
        control: &mut ZshInlineGlobControl,
        base: Position,
    ) {
        match control {
            ZshInlineGlobControl::CaseInsensitive { span }
            | ZshInlineGlobControl::Backreferences { span }
            | ZshInlineGlobControl::StartAnchor { span }
            | ZshInlineGlobControl::EndAnchor { span } => {
                *span = span.rebased(base);
            }
        }
    }

    pub(super) fn rebase_zsh_glob_qualifier_group(
        group: &mut ZshGlobQualifierGroup,
        base: Position,
    ) {
        group.span = group.span.rebased(base);
        for fragment in &mut group.fragments {
            match fragment {
                ZshGlobQualifier::Negation { span } | ZshGlobQualifier::Flag { span, .. } => {
                    *span = span.rebased(base);
                }
                ZshGlobQualifier::LetterSequence { text, span } => {
                    *span = span.rebased(base);
                    text.rebased(base);
                }
                ZshGlobQualifier::NumericArgument { span, start, end } => {
                    *span = span.rebased(base);
                    start.rebased(base);
                    if let Some(end) = end {
                        end.rebased(base);
                    }
                }
            }
        }
    }

    pub(super) fn rebase_parameter_expansion_syntax(
        syntax: &mut ParameterExpansionSyntax,
        base: Position,
    ) {
        match syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Transformation { reference, .. } => {
                    Self::rebase_var_ref(reference, base);
                }
                BourneParameterExpansion::Indirect {
                    reference,
                    operand,
                    operator,
                    operand_word_ast,
                    ..
                } => {
                    Self::rebase_var_ref(reference, base);
                    if let Some(operator) = operator {
                        Self::rebase_parameter_operator(operator, base);
                    }
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::rebase_word(word_ast, base);
                    }
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset,
                    offset_ast,
                    offset_word_ast,
                    length,
                    length_ast,
                    length_word_ast,
                } => {
                    Self::rebase_var_ref(reference, base);
                    offset.rebased(base);
                    Self::rebase_word(offset_word_ast, base);
                    if let Some(expr) = offset_ast {
                        Self::rebase_arithmetic_expr(expr, base);
                    }
                    if let Some(length) = length {
                        length.rebased(base);
                    }
                    if let Some(word_ast) = length_word_ast {
                        Self::rebase_word(word_ast, base);
                    }
                    if let Some(expr) = length_ast {
                        Self::rebase_arithmetic_expr(expr, base);
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    Self::rebase_var_ref(reference, base);
                    Self::rebase_parameter_operator(operator, base);
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::rebase_word(word_ast, base);
                    }
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &mut syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        Self::rebase_var_ref(reference, base)
                    }
                    ZshExpansionTarget::Word(word) => Self::rebase_word(word, base),
                    ZshExpansionTarget::Nested(parameter) => {
                        parameter.span = parameter.span.rebased(base);
                        parameter.raw_body.rebased(base);
                        Self::rebase_parameter_expansion_syntax(&mut parameter.syntax, base);
                    }
                    ZshExpansionTarget::Empty => {}
                }
                for modifier in &mut syntax.modifiers {
                    modifier.span = modifier.span.rebased(base);
                    if let Some(argument) = &mut modifier.argument {
                        argument.rebased(base);
                    }
                    if let Some(argument_word_ast) = &mut modifier.argument_word_ast {
                        Self::rebase_word(argument_word_ast, base);
                    }
                }
                if let Some(length_prefix) = &mut syntax.length_prefix {
                    *length_prefix = length_prefix.rebased(base);
                }
                if let Some(operation) = &mut syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::Defaulting {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::TrimOperation {
                            operand,
                            operand_word_ast,
                            ..
                        } => {
                            operand.rebased(base);
                            Self::rebase_word(operand_word_ast, base);
                        }
                        ZshExpansionOperation::Unknown { text, word_ast } => {
                            text.rebased(base);
                            Self::rebase_word(word_ast, base);
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            pattern_word_ast,
                            replacement,
                            replacement_word_ast,
                            ..
                        } => {
                            pattern.rebased(base);
                            Self::rebase_word(pattern_word_ast, base);
                            if let Some(replacement) = replacement {
                                replacement.rebased(base);
                            }
                            if let Some(replacement_word_ast) = replacement_word_ast {
                                Self::rebase_word(replacement_word_ast, base);
                            }
                        }
                        ZshExpansionOperation::Slice {
                            offset,
                            offset_word_ast,
                            length,
                            length_word_ast,
                        } => {
                            offset.rebased(base);
                            Self::rebase_word(offset_word_ast, base);
                            if let Some(length) = length {
                                length.rebased(base);
                            }
                            if let Some(length_word_ast) = length_word_ast {
                                Self::rebase_word(length_word_ast, base);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn rebase_parameter_operator(operator: &mut ParameterOp, base: Position) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                Self::rebase_pattern(pattern, base);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                replacement_word_ast,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                replacement_word_ast,
            } => {
                Self::rebase_pattern(pattern, base);
                replacement.rebased(base);
                Self::rebase_word(replacement_word_ast, base);
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    pub(super) fn rebase_conditional_expr(expr: &mut ConditionalExpr, base: Position) {
        match expr {
            ConditionalExpr::Binary(binary) => {
                binary.op_span = binary.op_span.rebased(base);
                Self::rebase_conditional_expr(&mut binary.left, base);
                Self::rebase_conditional_expr(&mut binary.right, base);
            }
            ConditionalExpr::Unary(unary) => {
                unary.op_span = unary.op_span.rebased(base);
                Self::rebase_conditional_expr(&mut unary.expr, base);
            }
            ConditionalExpr::Parenthesized(paren) => {
                paren.left_paren_span = paren.left_paren_span.rebased(base);
                paren.right_paren_span = paren.right_paren_span.rebased(base);
                Self::rebase_conditional_expr(&mut paren.expr, base);
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                Self::rebase_word(word, base);
            }
            ConditionalExpr::Pattern(pattern) => Self::rebase_pattern(pattern, base),
            ConditionalExpr::VarRef(var_ref) => Self::rebase_var_ref(var_ref, base),
        }
    }

    pub(super) fn rebase_arithmetic_expr(expr: &mut ArithmeticExprNode, base: Position) {
        expr.span = expr.span.rebased(base);
        match &mut expr.kind {
            ArithmeticExpr::Number(text) => text.rebased(base),
            ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => Self::rebase_arithmetic_expr(index, base),
            ArithmeticExpr::ShellWord(word) => Self::rebase_word(word, base),
            ArithmeticExpr::Parenthesized { expression } => {
                Self::rebase_arithmetic_expr(expression, base)
            }
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                Self::rebase_arithmetic_expr(expr, base)
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                Self::rebase_arithmetic_expr(left, base);
                Self::rebase_arithmetic_expr(right, base);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::rebase_arithmetic_expr(condition, base);
                Self::rebase_arithmetic_expr(then_expr, base);
                Self::rebase_arithmetic_expr(else_expr, base);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                Self::rebase_arithmetic_lvalue(target, base);
                Self::rebase_arithmetic_expr(value, base);
            }
        }
    }

    pub(super) fn rebase_arithmetic_lvalue(target: &mut ArithmeticLvalue, base: Position) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => Self::rebase_arithmetic_expr(index, base),
        }
    }
}
