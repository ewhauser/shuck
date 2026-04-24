use shuck_ast::{
    ArithmeticAssignOp, ArithmeticBinaryOp, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue,
    ArithmeticPostfixOp, ArithmeticUnaryOp, Name, Position, SourceText, Span, Word,
};

use crate::error::{Error, Result};

use super::{Parser, ShellDialect};

#[derive(Debug, Clone)]
enum TokenKind {
    End,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Question,
    Colon,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    DoubleStar,
    ShiftLeft,
    ShiftRight,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    AndAssign,
    XorAssign,
    OrAssign,
    Increment,
    Decrement,
    Bang,
    Tilde,
    Ident(Name),
    Number(SourceText),
    ShellWord(Word),
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    span: Span,
}

impl Token {
    fn end(span: Span) -> Self {
        Self {
            kind: TokenKind::End,
            span,
        }
    }
}

pub(super) fn parse_expression(
    input: &str,
    base: Span,
    dialect: ShellDialect,
    max_depth: usize,
    max_fuel: usize,
) -> Result<ArithmeticExprNode> {
    let mut parser = ArithmeticParser::new(input, base, dialect, max_depth, max_fuel);
    let expr = parser.parse_expression()?;
    if !matches!(parser.peek_token()?.kind, TokenKind::End) {
        let trailing_start = parser.peek_token()?.span.start;
        return Err(parser.error_at(trailing_start, "unexpected token in arithmetic expression"));
    }
    Ok(expr)
}

struct ArithmeticParser<'a> {
    input: &'a str,
    base: Span,
    dialect: ShellDialect,
    index: usize,
    peeked: Option<Token>,
    max_depth: usize,
    max_fuel: usize,
    fuel: usize,
}

impl<'a> ArithmeticParser<'a> {
    fn new(
        input: &'a str,
        base: Span,
        dialect: ShellDialect,
        max_depth: usize,
        max_fuel: usize,
    ) -> Self {
        Self {
            input,
            base,
            dialect,
            index: 0,
            peeked: None,
            max_depth,
            max_fuel,
            fuel: max_fuel.max(1),
        }
    }

    fn parse_expression(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_comma()
    }

    fn parse_comma(&mut self) -> Result<ArithmeticExprNode> {
        let mut expr = self.parse_assignment()?;
        loop {
            if !matches!(self.peek_token()?.kind, TokenKind::Comma) {
                break;
            }
            self.next_token()?;
            let right = self.parse_assignment()?;
            let span = expr.span.merge(right.span);
            expr = ArithmeticExprNode::new(
                ArithmeticExpr::Binary {
                    left: Box::new(expr),
                    op: ArithmeticBinaryOp::Comma,
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_assignment(&mut self) -> Result<ArithmeticExprNode> {
        let left = self.parse_conditional()?;
        let op = match &self.peek_token()?.kind {
            TokenKind::Assign => ArithmeticAssignOp::Assign,
            TokenKind::PlusAssign => ArithmeticAssignOp::AddAssign,
            TokenKind::MinusAssign => ArithmeticAssignOp::SubAssign,
            TokenKind::StarAssign => ArithmeticAssignOp::MulAssign,
            TokenKind::SlashAssign => ArithmeticAssignOp::DivAssign,
            TokenKind::PercentAssign => ArithmeticAssignOp::ModAssign,
            TokenKind::ShiftLeftAssign => ArithmeticAssignOp::ShiftLeftAssign,
            TokenKind::ShiftRightAssign => ArithmeticAssignOp::ShiftRightAssign,
            TokenKind::AndAssign => ArithmeticAssignOp::AndAssign,
            TokenKind::XorAssign => ArithmeticAssignOp::XorAssign,
            TokenKind::OrAssign => ArithmeticAssignOp::OrAssign,
            _ => return Ok(left),
        };
        self.next_token()?;
        let right = self.parse_assignment()?;
        let span = left.span.merge(right.span);
        let target = self.node_to_lvalue(left)?;
        Ok(ArithmeticExprNode::new(
            ArithmeticExpr::Assignment {
                target,
                op,
                value: Box::new(right),
            },
            span,
        ))
    }

    fn parse_conditional(&mut self) -> Result<ArithmeticExprNode> {
        let condition = self.parse_logical_or()?;
        if !matches!(self.peek_token()?.kind, TokenKind::Question) {
            return Ok(condition);
        }
        self.next_token()?;
        let then_expr = self.parse_comma()?;
        let colon = self.next_token()?;
        if !matches!(colon.kind, TokenKind::Colon) {
            return Err(self.error_at(colon.span.start, "expected ':' in arithmetic conditional"));
        }
        let else_expr = self.parse_conditional()?;
        let span = condition.span.merge(else_expr.span);
        Ok(ArithmeticExprNode::new(
            ArithmeticExpr::Conditional {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            span,
        ))
    }

    fn parse_logical_or(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_logical_and, |kind| match kind {
            TokenKind::LogicalOr => Some(ArithmeticBinaryOp::LogicalOr),
            _ => None,
        })
    }

    fn parse_logical_and(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_bitwise_or, |kind| match kind {
            TokenKind::LogicalAnd => Some(ArithmeticBinaryOp::LogicalAnd),
            _ => None,
        })
    }

    fn parse_bitwise_or(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_bitwise_xor, |kind| match kind {
            TokenKind::BitwiseOr => Some(ArithmeticBinaryOp::BitwiseOr),
            _ => None,
        })
    }

    fn parse_bitwise_xor(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_bitwise_and, |kind| match kind {
            TokenKind::BitwiseXor => Some(ArithmeticBinaryOp::BitwiseXor),
            _ => None,
        })
    }

    fn parse_bitwise_and(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_equality, |kind| match kind {
            TokenKind::BitwiseAnd => Some(ArithmeticBinaryOp::BitwiseAnd),
            _ => None,
        })
    }

    fn parse_equality(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_relational, |kind| match kind {
            TokenKind::Equal => Some(ArithmeticBinaryOp::Equal),
            TokenKind::NotEqual => Some(ArithmeticBinaryOp::NotEqual),
            _ => None,
        })
    }

    fn parse_relational(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_shift, |kind| match kind {
            TokenKind::LessThan => Some(ArithmeticBinaryOp::LessThan),
            TokenKind::LessThanOrEqual => Some(ArithmeticBinaryOp::LessThanOrEqual),
            TokenKind::GreaterThan => Some(ArithmeticBinaryOp::GreaterThan),
            TokenKind::GreaterThanOrEqual => Some(ArithmeticBinaryOp::GreaterThanOrEqual),
            _ => None,
        })
    }

    fn parse_shift(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_additive, |kind| match kind {
            TokenKind::ShiftLeft => Some(ArithmeticBinaryOp::ShiftLeft),
            TokenKind::ShiftRight => Some(ArithmeticBinaryOp::ShiftRight),
            _ => None,
        })
    }

    fn parse_additive(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_multiplicative, |kind| match kind {
            TokenKind::Plus => Some(ArithmeticBinaryOp::Add),
            TokenKind::Minus => Some(ArithmeticBinaryOp::Subtract),
            _ => None,
        })
    }

    fn parse_multiplicative(&mut self) -> Result<ArithmeticExprNode> {
        self.parse_left_associative(Self::parse_power, |kind| match kind {
            TokenKind::Star => Some(ArithmeticBinaryOp::Multiply),
            TokenKind::Slash => Some(ArithmeticBinaryOp::Divide),
            TokenKind::Percent => Some(ArithmeticBinaryOp::Modulo),
            _ => None,
        })
    }

    fn parse_power(&mut self) -> Result<ArithmeticExprNode> {
        let left = self.parse_unary()?;
        if !matches!(self.peek_token()?.kind, TokenKind::DoubleStar) {
            return Ok(left);
        }
        self.next_token()?;
        let right = self.parse_power()?;
        let span = left.span.merge(right.span);
        Ok(ArithmeticExprNode::new(
            ArithmeticExpr::Binary {
                left: Box::new(left),
                op: ArithmeticBinaryOp::Power,
                right: Box::new(right),
            },
            span,
        ))
    }

    fn parse_unary(&mut self) -> Result<ArithmeticExprNode> {
        let (op, start_span) = match self.peek_token()? {
            Token {
                kind: TokenKind::Increment,
                span,
            } => (ArithmeticUnaryOp::PreIncrement, *span),
            Token {
                kind: TokenKind::Decrement,
                span,
            } => (ArithmeticUnaryOp::PreDecrement, *span),
            Token {
                kind: TokenKind::Plus,
                span,
            } => (ArithmeticUnaryOp::Plus, *span),
            Token {
                kind: TokenKind::Minus,
                span,
            } => (ArithmeticUnaryOp::Minus, *span),
            Token {
                kind: TokenKind::Bang,
                span,
            } => (ArithmeticUnaryOp::LogicalNot, *span),
            Token {
                kind: TokenKind::Tilde,
                span,
            } => (ArithmeticUnaryOp::BitwiseNot, *span),
            _ => return self.parse_postfix(),
        };
        self.next_token()?;
        let expr = self.parse_unary()?;
        let span = start_span.merge(expr.span);
        self.ensure_lvalue_if_update(op, &expr)?;
        Ok(ArithmeticExprNode::new(
            ArithmeticExpr::Unary {
                op,
                expr: Box::new(expr),
            },
            span,
        ))
    }

    fn parse_postfix(&mut self) -> Result<ArithmeticExprNode> {
        let mut expr = self.parse_primary()?;
        loop {
            if matches!(self.peek_token()?.kind, TokenKind::LeftBracket) {
                self.next_token()?;
                let index = self.parse_comma()?;
                let closing = self.next_token()?;
                if !matches!(closing.kind, TokenKind::RightBracket) {
                    return Err(
                        self.error_at(closing.span.start, "expected ']' in arithmetic index")
                    );
                }
                let span = expr.span.merge(closing.span);
                expr = self.index_expr(expr, index, span)?;
                continue;
            }

            let (op, at, span) = match self.peek_token()? {
                Token {
                    kind: TokenKind::Increment,
                    span,
                } => (ArithmeticPostfixOp::Increment, span.start, *span),
                Token {
                    kind: TokenKind::Decrement,
                    span,
                } => (ArithmeticPostfixOp::Decrement, span.start, *span),
                _ => break,
            };

            self.next_token()?;
            self.ensure_postfix_target(&expr, at)?;
            let expr_span = expr.span;
            expr = ArithmeticExprNode::new(
                ArithmeticExpr::Postfix {
                    expr: Box::new(expr),
                    op,
                },
                expr_span.merge(span),
            );
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<ArithmeticExprNode> {
        let token = self.next_token()?;
        match token.kind {
            TokenKind::LeftParen => {
                let expr = self.parse_comma()?;
                let closing = self.next_token()?;
                if !matches!(closing.kind, TokenKind::RightParen) {
                    return Err(
                        self.error_at(closing.span.start, "expected ')' in arithmetic expression")
                    );
                }
                Ok(ArithmeticExprNode::new(
                    ArithmeticExpr::Parenthesized {
                        expression: Box::new(expr),
                    },
                    token.span.merge(closing.span),
                ))
            }
            TokenKind::Number(number) => Ok(ArithmeticExprNode::new(
                ArithmeticExpr::Number(number),
                token.span,
            )),
            TokenKind::Ident(name) => Ok(ArithmeticExprNode::new(
                ArithmeticExpr::Variable(name),
                token.span,
            )),
            TokenKind::ShellWord(word) => Ok(ArithmeticExprNode::new(
                ArithmeticExpr::ShellWord(Box::new(word)),
                token.span,
            )),
            TokenKind::End => {
                Err(self.error_at(token.span.start, "unexpected end of arithmetic expression"))
            }
            _ => Err(self.error_at(token.span.start, "expected arithmetic operand")),
        }
    }

    fn parse_left_associative(
        &mut self,
        subparser: fn(&mut Self) -> Result<ArithmeticExprNode>,
        op_of: fn(&TokenKind) -> Option<ArithmeticBinaryOp>,
    ) -> Result<ArithmeticExprNode> {
        let mut expr = subparser(self)?;
        while let Some(op) = op_of(&self.peek_token()?.kind) {
            self.next_token()?;
            let right = subparser(self)?;
            let span = expr.span.merge(right.span);
            expr = ArithmeticExprNode::new(
                ArithmeticExpr::Binary {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn index_expr(
        &self,
        expr: ArithmeticExprNode,
        index: ArithmeticExprNode,
        span: Span,
    ) -> Result<ArithmeticExprNode> {
        match expr.kind {
            ArithmeticExpr::Variable(name) => Ok(ArithmeticExprNode::new(
                ArithmeticExpr::Indexed {
                    name,
                    index: Box::new(index),
                },
                span,
            )),
            ArithmeticExpr::Indexed { .. } => Err(self.error_at(
                expr.span.start,
                "nested arithmetic indices are not supported",
            )),
            _ => Err(self.error_at(expr.span.start, "expected variable before arithmetic index")),
        }
    }

    fn node_to_lvalue(&self, expr: ArithmeticExprNode) -> Result<ArithmeticLvalue> {
        match expr.kind {
            ArithmeticExpr::Variable(name) => Ok(ArithmeticLvalue::Variable(name)),
            ArithmeticExpr::Indexed { name, index } => {
                Ok(ArithmeticLvalue::Indexed { name, index })
            }
            _ => Err(self.error_at(expr.span.start, "expected assignable arithmetic target")),
        }
    }

    fn ensure_postfix_target(&self, expr: &ArithmeticExprNode, at: Position) -> Result<()> {
        match expr.kind {
            ArithmeticExpr::Variable(_) | ArithmeticExpr::Indexed { .. } => Ok(()),
            _ => Err(self.error_at(
                at,
                "expected variable or indexed reference before update operator",
            )),
        }
    }

    fn ensure_lvalue_if_update(
        &self,
        op: ArithmeticUnaryOp,
        expr: &ArithmeticExprNode,
    ) -> Result<()> {
        if !matches!(
            op,
            ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
        ) {
            return Ok(());
        }
        self.ensure_postfix_target(expr, expr.span.start)
    }

    fn next_token(&mut self) -> Result<Token> {
        if let Some(token) = self.peeked.take() {
            return Ok(token);
        }
        self.lex_token()
    }

    fn peek_token(&mut self) -> Result<&Token> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lex_token()?);
        }
        self.peeked.as_ref().ok_or_else(|| {
            self.error_at(
                self.safe_position_at(self.index),
                "internal arithmetic parser failed to cache lookahead token",
            )
        })
    }

    fn lex_token(&mut self) -> Result<Token> {
        self.tick()?;
        self.skip_whitespace();
        if self.index >= self.input.len() {
            return Ok(Token::end(self.empty_span()));
        }

        let start = self.index;
        let ch = self.require_current_char()?;

        let token = if is_ident_start(ch) {
            self.lex_identifier_or_word(start)?
        } else if ch.is_ascii_digit() {
            self.lex_number(start)
        } else if ch == '#'
            && self.dialect == ShellDialect::Zsh
            && self
                .char_at(start + 1)
                .is_some_and(|next| !next.is_whitespace())
        {
            self.lex_zsh_char_literal(start)
        } else {
            match ch {
                '(' => self.simple_token(start, ch.len_utf8(), TokenKind::LeftParen),
                ')' => self.simple_token(start, ch.len_utf8(), TokenKind::RightParen),
                '[' => self.simple_token(start, ch.len_utf8(), TokenKind::LeftBracket),
                ']' => self.simple_token(start, ch.len_utf8(), TokenKind::RightBracket),
                '?' => self.simple_token(start, ch.len_utf8(), TokenKind::Question),
                ':' => self.simple_token(start, ch.len_utf8(), TokenKind::Colon),
                ',' => self.simple_token(start, ch.len_utf8(), TokenKind::Comma),
                '+' => {
                    if self.consume_str("++") {
                        self.token_for_range(start, self.index, TokenKind::Increment)
                    } else if self.consume_str("+=") {
                        self.token_for_range(start, self.index, TokenKind::PlusAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Plus)
                    }
                }
                '-' => {
                    if self.consume_str("--") {
                        self.token_for_range(start, self.index, TokenKind::Decrement)
                    } else if self.consume_str("-=") {
                        self.token_for_range(start, self.index, TokenKind::MinusAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Minus)
                    }
                }
                '*' => {
                    if self.consume_str("**") {
                        self.token_for_range(start, self.index, TokenKind::DoubleStar)
                    } else if self.consume_str("*=") {
                        self.token_for_range(start, self.index, TokenKind::StarAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Star)
                    }
                }
                '/' => {
                    if self.consume_str("/=") {
                        self.token_for_range(start, self.index, TokenKind::SlashAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Slash)
                    }
                }
                '%' => {
                    if self.consume_str("%=") {
                        self.token_for_range(start, self.index, TokenKind::PercentAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Percent)
                    }
                }
                '<' => {
                    if self.consume_str("<<=") {
                        self.token_for_range(start, self.index, TokenKind::ShiftLeftAssign)
                    } else if self.consume_str("<<") {
                        self.token_for_range(start, self.index, TokenKind::ShiftLeft)
                    } else if self.consume_str("<=") {
                        self.token_for_range(start, self.index, TokenKind::LessThanOrEqual)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::LessThan)
                    }
                }
                '>' => {
                    if self.consume_str(">>=") {
                        self.token_for_range(start, self.index, TokenKind::ShiftRightAssign)
                    } else if self.consume_str(">>") {
                        self.token_for_range(start, self.index, TokenKind::ShiftRight)
                    } else if self.consume_str(">=") {
                        self.token_for_range(start, self.index, TokenKind::GreaterThanOrEqual)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::GreaterThan)
                    }
                }
                '=' => {
                    if self.consume_str("==") {
                        self.token_for_range(start, self.index, TokenKind::Equal)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Assign)
                    }
                }
                '!' => {
                    if self.consume_str("!=") {
                        self.token_for_range(start, self.index, TokenKind::NotEqual)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::Bang)
                    }
                }
                '&' => {
                    if self.consume_str("&&") {
                        self.token_for_range(start, self.index, TokenKind::LogicalAnd)
                    } else if self.consume_str("&=") {
                        self.token_for_range(start, self.index, TokenKind::AndAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::BitwiseAnd)
                    }
                }
                '^' => {
                    if self.consume_str("^=") {
                        self.token_for_range(start, self.index, TokenKind::XorAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::BitwiseXor)
                    }
                }
                '|' => {
                    if self.consume_str("||") {
                        self.token_for_range(start, self.index, TokenKind::LogicalOr)
                    } else if self.consume_str("|=") {
                        self.token_for_range(start, self.index, TokenKind::OrAssign)
                    } else {
                        self.index += 1;
                        self.token_for_range(start, self.index, TokenKind::BitwiseOr)
                    }
                }
                '~' => self.simple_token(start, ch.len_utf8(), TokenKind::Tilde),
                '$' | '"' | '\'' | '`' | '\\' => self.lex_shell_word(start)?,
                _ if !ch.is_ascii() => self.lex_shell_word(start)?,
                _ => {
                    return Err(self.error_at(
                        self.position_at(start),
                        "unexpected character in arithmetic expression",
                    ));
                }
            }
        };

        Ok(token)
    }

    fn lex_identifier_or_word(&mut self, start: usize) -> Result<Token> {
        let mut end = start;
        while let Some(ch) = self.char_at(end) {
            if is_ident_continue(ch) {
                end += ch.len_utf8();
            } else {
                break;
            }
        }
        if matches!(self.char_at(end), Some('$' | '"' | '\'' | '`' | '\\'))
            || matches!(self.char_at(end), Some(ch) if !self.is_arithmetic_boundary(ch) && !ch.is_whitespace())
        {
            self.index = start;
            return self.lex_shell_word(start);
        }
        self.index = end;
        Ok(Token {
            kind: TokenKind::Ident(Name::from(&self.input[start..end])),
            span: self.span_for(start, end),
        })
    }

    fn lex_number(&mut self, start: usize) -> Token {
        let mut end = start;
        while let Some(ch) = self.char_at(end) {
            if ch.is_ascii_alphanumeric() || matches!(ch, '#' | '_') {
                end += ch.len_utf8();
            } else {
                break;
            }
        }
        self.index = end;
        Token {
            kind: TokenKind::Number(SourceText::source(self.span_for(start, end))),
            span: self.span_for(start, end),
        }
    }

    fn lex_zsh_char_literal(&mut self, start: usize) -> Token {
        let Some(next) = self.char_at(start + 1) else {
            unreachable!("zsh char literal requires a following character");
        };
        let end = start + 1 + next.len_utf8();
        self.index = end;
        Token {
            kind: TokenKind::Number(SourceText::source(self.span_for(start, end))),
            span: self.span_for(start, end),
        }
    }

    fn lex_shell_word(&mut self, start: usize) -> Result<Token> {
        let end = self.scan_shell_word_end(start)?;
        let raw = &self.input[start..end];
        let mut word = Parser::parse_word_string_with_limits_and_dialect(
            raw,
            self.max_depth,
            self.max_fuel,
            self.dialect,
        );
        Parser::rebase_word(&mut word, self.position_at(start));
        self.index = end;
        Ok(Token {
            kind: TokenKind::ShellWord(word),
            span: self.span_for(start, end),
        })
    }

    fn scan_shell_word_end(&self, start: usize) -> Result<usize> {
        let mut index = start;
        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            if index > start && (ch.is_whitespace() || self.is_arithmetic_boundary(ch)) {
                break;
            }
            index = match ch {
                '\'' => self.consume_single_quoted(index)?,
                '"' => self.consume_double_quoted(index)?,
                '`' => self.consume_backticks(index)?,
                '$' => self.consume_dollar(index)?,
                '\\' => self.consume_escape(index),
                _ => index + ch.len_utf8(),
            };
        }
        Ok(index)
    }

    fn consume_single_quoted(&self, start: usize) -> Result<usize> {
        let mut index = start + 1;
        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            index += ch.len_utf8();
            if ch == '\'' {
                return Ok(index);
            }
        }
        Err(self.error_at(
            self.position_at(start),
            "unterminated single-quoted string in arithmetic expression",
        ))
    }

    fn consume_double_quoted(&self, start: usize) -> Result<usize> {
        let mut index = start + 1;
        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            match ch {
                '"' => return Ok(index + 1),
                '\\' => index = self.consume_escape(index),
                '$' => index = self.consume_dollar(index)?,
                '`' => index = self.consume_backticks(index)?,
                _ => index += ch.len_utf8(),
            }
        }
        Err(self.error_at(
            self.position_at(start),
            "unterminated double-quoted string in arithmetic expression",
        ))
    }

    fn consume_backticks(&self, start: usize) -> Result<usize> {
        let mut index = start + 1;
        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            match ch {
                '`' => return Ok(index + 1),
                '\\' => index = self.consume_escape(index),
                _ => index += ch.len_utf8(),
            }
        }
        Err(self.error_at(
            self.position_at(start),
            "unterminated backtick substitution in arithmetic expression",
        ))
    }

    fn consume_dollar(&self, start: usize) -> Result<usize> {
        let Some(next) = self.char_at(start + 1) else {
            return Ok(start + 1);
        };
        let mut index = match next {
            '\'' | '"' => {
                if next == '\'' {
                    self.consume_single_quoted(start + 1)
                } else {
                    self.consume_double_quoted(start + 1)
                }
            }
            '{' => self.consume_braced(start),
            '(' => {
                if self.char_at(start + 2) == Some('(') {
                    self.consume_dollar_arithmetic(start)
                } else {
                    self.consume_command_substitution(start)
                }
            }
            '+' if self.dialect == ShellDialect::Zsh
                && self.char_at(start + 2).is_some_and(is_ident_start) =>
            {
                let mut index = start + 2;
                while let Some(ch) = self.char_at(index) {
                    if is_ident_continue(ch) {
                        index += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                Ok(index)
            }
            '#' if self.dialect == ShellDialect::Zsh => {
                let Some(next) = self.char_at(start + 2) else {
                    return Ok(start + 2);
                };
                let mut index = start + 2;

                if is_ident_start(next) {
                    index += next.len_utf8();
                    while let Some(ch) = self.char_at(index) {
                        if is_ident_continue(ch) {
                            index += ch.len_utf8();
                        } else {
                            break;
                        }
                    }
                } else if next.is_ascii_digit() || is_special_parameter(next) {
                    index += next.len_utf8();
                }

                Ok(index)
            }
            ch if is_special_parameter(ch) => Ok(start + 1 + ch.len_utf8()),
            ch if is_ident_start(ch) => {
                let mut index = start + 1 + ch.len_utf8();
                while let Some(ch) = self.char_at(index) {
                    if is_ident_continue(ch) {
                        index += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                Ok(index)
            }
            ch if ch.is_ascii_digit() => Ok(start + 1 + ch.len_utf8()),
            _ => Ok(start + 1),
        }?;

        if self.dialect == ShellDialect::Zsh {
            while self.char_at(index) == Some('[') {
                index = self.consume_zsh_subscript(index)?;
            }
        }

        Ok(index)
    }

    fn consume_zsh_subscript(&self, start: usize) -> Result<usize> {
        let mut index = start + 1;
        let mut depth = 1usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            if escaped {
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if in_single {
                index += ch.len_utf8();
                if ch == '\'' {
                    in_single = false;
                }
                continue;
            }
            if in_double {
                match ch {
                    '"' => {
                        in_double = false;
                        index += 1;
                    }
                    '\\' => index = self.consume_escape(index),
                    '$' => index = self.consume_dollar(index)?,
                    '`' => index = self.consume_backticks(index)?,
                    _ => index += ch.len_utf8(),
                }
                continue;
            }

            match ch {
                '\\' => {
                    escaped = true;
                    index += 1;
                }
                '\'' => {
                    in_single = true;
                    index += 1;
                }
                '"' => {
                    in_double = true;
                    index += 1;
                }
                '$' => index = self.consume_dollar(index)?,
                '`' => index = self.consume_backticks(index)?,
                '[' => {
                    depth += 1;
                    index += 1;
                }
                ']' => {
                    depth -= 1;
                    index += 1;
                    if depth == 0 {
                        return Ok(index);
                    }
                }
                _ => index += ch.len_utf8(),
            }
        }

        Err(self.error_at(
            self.position_at(start),
            "unterminated zsh subscript in arithmetic expression",
        ))
    }

    fn consume_braced(&self, start: usize) -> Result<usize> {
        let mut index = start + 2;
        let mut depth = 1usize;
        let mut in_single = false;
        let mut in_double = false;

        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            if in_single {
                index += ch.len_utf8();
                if ch == '\'' {
                    in_single = false;
                }
                continue;
            }
            if in_double {
                match ch {
                    '"' => {
                        in_double = false;
                        index += 1;
                    }
                    '\\' => index = self.consume_escape(index),
                    '$' => index = self.consume_dollar(index)?,
                    '`' => index = self.consume_backticks(index)?,
                    _ => index += ch.len_utf8(),
                }
                continue;
            }

            match ch {
                '\'' => {
                    in_single = true;
                    index += 1;
                }
                '"' => {
                    in_double = true;
                    index += 1;
                }
                '\\' => index = self.consume_escape(index),
                '$' => {
                    if self.char_at(index + 1) == Some('{') {
                        depth += 1;
                        index += 2;
                    } else {
                        index = self.consume_dollar(index)?;
                    }
                }
                '`' => index = self.consume_backticks(index)?,
                '}' => {
                    depth -= 1;
                    index += 1;
                    if depth == 0 {
                        return Ok(index);
                    }
                }
                _ => index += ch.len_utf8(),
            }
        }
        Err(self.error_at(
            self.position_at(start),
            "unterminated parameter expansion in arithmetic expression",
        ))
    }

    fn consume_command_substitution(&self, start: usize) -> Result<usize> {
        let mut index = start + 2;
        let mut depth = 1usize;
        let mut in_single = false;
        let mut in_double = false;

        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            if in_single {
                index += ch.len_utf8();
                if ch == '\'' {
                    in_single = false;
                }
                continue;
            }
            if in_double {
                match ch {
                    '"' => {
                        in_double = false;
                        index += 1;
                    }
                    '\\' => index = self.consume_escape(index),
                    '$' => index = self.consume_dollar(index)?,
                    '`' => index = self.consume_backticks(index)?,
                    _ => index += ch.len_utf8(),
                }
                continue;
            }

            match ch {
                '\'' => {
                    in_single = true;
                    index += 1;
                }
                '"' => {
                    in_double = true;
                    index += 1;
                }
                '\\' => index = self.consume_escape(index),
                '$' if self.char_at(index + 1) == Some('(') => {
                    if self.char_at(index + 2) == Some('(') {
                        index = self.consume_dollar_arithmetic(index)?;
                    } else {
                        depth += 1;
                        index += 2;
                    }
                }
                '`' => index = self.consume_backticks(index)?,
                '(' => index += 1,
                ')' => {
                    depth -= 1;
                    index += 1;
                    if depth == 0 {
                        return Ok(index);
                    }
                }
                _ => index += ch.len_utf8(),
            }
        }
        Err(self.error_at(
            self.position_at(start),
            "unterminated command substitution in arithmetic expression",
        ))
    }

    fn consume_dollar_arithmetic(&self, start: usize) -> Result<usize> {
        let mut index = start + 3;
        let mut depth = 2i32;
        let mut in_single = false;
        let mut in_double = false;

        while index < self.input.len() {
            let ch = self.require_char_at(index)?;
            if in_single {
                index += ch.len_utf8();
                if ch == '\'' {
                    in_single = false;
                }
                continue;
            }
            if in_double {
                match ch {
                    '"' => {
                        in_double = false;
                        index += 1;
                    }
                    '\\' => index = self.consume_escape(index),
                    '$' => index = self.consume_dollar(index)?,
                    '`' => index = self.consume_backticks(index)?,
                    _ => index += ch.len_utf8(),
                }
                continue;
            }

            match ch {
                '\'' => {
                    in_single = true;
                    index += 1;
                }
                '"' => {
                    in_double = true;
                    index += 1;
                }
                '\\' => index = self.consume_escape(index),
                '$' => index = self.consume_dollar(index)?,
                '`' => index = self.consume_backticks(index)?,
                '(' => {
                    depth += 1;
                    index += 1;
                }
                ')' => {
                    depth -= 1;
                    index += 1;
                    if depth == 0 {
                        return Ok(index);
                    }
                }
                _ => index += ch.len_utf8(),
            }
        }
        Err(self.error_at(self.position_at(start), "unterminated arithmetic expansion"))
    }

    fn consume_escape(&self, start: usize) -> usize {
        let mut index = start + 1;
        if let Some(ch) = self.char_at(index) {
            index += ch.len_utf8();
        }
        index
    }

    fn simple_token(&mut self, start: usize, width: usize, kind: TokenKind) -> Token {
        self.index = start + width;
        self.token_for_range(start, self.index, kind)
    }

    fn token_for_range(&self, start: usize, end: usize, kind: TokenKind) -> Token {
        Token {
            kind,
            span: self.span_for(start, end),
        }
    }

    fn skip_whitespace(&mut self) {
        loop {
            let Some(ch) = self.current_char() else {
                break;
            };

            if ch.is_whitespace() {
                self.index += ch.len_utf8();
                continue;
            }

            if self.input[self.index..].starts_with("\\\r\n") {
                self.index += "\\\r\n".len();
                continue;
            }

            if self.input[self.index..].starts_with("\\\n") {
                self.index += "\\\n".len();
                continue;
            }

            break;
        }
    }

    fn current_char(&self) -> Option<char> {
        self.char_at(self.index)
    }

    fn char_at(&self, index: usize) -> Option<char> {
        self.input.get(index..)?.chars().next()
    }

    fn consume_str(&mut self, value: &str) -> bool {
        if self.input[self.index..].starts_with(value) {
            self.index += value.len();
            true
        } else {
            false
        }
    }

    fn position_at(&self, index: usize) -> Position {
        self.base.start.advanced_by(&self.input[..index])
    }

    fn safe_position_at(&self, index: usize) -> Position {
        let safe_index = if index <= self.input.len() && self.input.is_char_boundary(index) {
            index
        } else {
            self.input.len()
        };
        self.position_at(safe_index)
    }

    fn require_current_char(&self) -> Result<char> {
        self.require_char_at(self.index)
    }

    fn require_char_at(&self, index: usize) -> Result<char> {
        self.char_at(index).ok_or_else(|| {
            self.error_at(
                self.safe_position_at(index),
                "internal arithmetic parser cursor became invalid",
            )
        })
    }

    fn span_for(&self, start: usize, end: usize) -> Span {
        Span::from_positions(self.position_at(start), self.position_at(end))
    }

    fn empty_span(&self) -> Span {
        Span::from_positions(
            self.position_at(self.input.len()),
            self.position_at(self.input.len()),
        )
    }

    fn is_arithmetic_boundary(&self, ch: char) -> bool {
        matches!(
            ch,
            '(' | ')'
                | '['
                | ']'
                | '?'
                | ':'
                | ','
                | '+'
                | '-'
                | '*'
                | '/'
                | '%'
                | '<'
                | '>'
                | '='
                | '!'
                | '~'
                | '&'
                | '^'
                | '|'
        )
    }

    fn error_at(&self, pos: Position, message: impl Into<String>) -> Error {
        Error::parse_at(message, pos.line, pos.column)
    }

    fn tick(&mut self) -> Result<()> {
        if self.fuel == 0 {
            return Err(Error::parse("arithmetic parser exceeded operation limit"));
        }
        self.fuel -= 1;
        Ok(())
    }
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_special_parameter(ch: char) -> bool {
    matches!(ch, '?' | '#' | '@' | '*' | '!' | '$' | '-')
}
