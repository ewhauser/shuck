use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, CaseItem, Command, CompoundCommand,
    ConditionalExpr, DeclClause, DeclOperand, Position, Script, Span, Word, WordPart,
};

use crate::{ParsedSyntax, SourceSpan};

/// Where an arithmetic region came from in the shell source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithmeticContextKind {
    Command,
    ForInit,
    ForCondition,
    ForStep,
    Expansion,
}

/// Variable access observed while analyzing an arithmetic region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithmeticEventKind {
    Read,
    Write,
}

/// A zero-copy variable access within an arithmetic region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithmeticVariableEvent<'a> {
    pub context: ArithmeticContextKind,
    pub context_span: SourceSpan,
    pub kind: ArithmeticEventKind,
    pub name: &'a str,
    pub name_span: SourceSpan,
}

impl ParsedSyntax {
    pub fn arithmetic_variable_events<'a>(
        &self,
        source: &'a str,
    ) -> Vec<ArithmeticVariableEvent<'a>> {
        collect_arithmetic_variable_events(source, &self.script)
    }
}

/// Collect ordered variable read/write events from arithmetic commands, arithmetic
/// `for` headers, and `$(( ... ))` expansions in the parsed script.
pub fn collect_arithmetic_variable_events<'a>(
    source: &'a str,
    script: &Script,
) -> Vec<ArithmeticVariableEvent<'a>> {
    let mut collector = ArithmeticCollector {
        source,
        events: Vec::new(),
    };
    collector.visit_script(script);
    collector.events
}

struct ArithmeticCollector<'a> {
    source: &'a str,
    events: Vec<ArithmeticVariableEvent<'a>>,
}

impl<'a> ArithmeticCollector<'a> {
    fn visit_script(&mut self, script: &Script) {
        for command in &script.commands {
            self.visit_command(command);
        }
    }

    fn visit_command(&mut self, command: &Command) {
        match command {
            Command::Simple(command) => {
                self.visit_word(&command.name);
                for word in &command.args {
                    self.visit_word(word);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            Command::Builtin(command) => self.visit_builtin(command),
            Command::Decl(command) => self.visit_decl(command),
            Command::Pipeline(pipeline) => {
                for command in &pipeline.commands {
                    self.visit_command(command);
                }
            }
            Command::List(list) => {
                self.visit_command(&list.first);
                for (_, command) in &list.rest {
                    self.visit_command(command);
                }
            }
            Command::Compound(compound, redirects) => {
                self.visit_compound(compound);
                for redirect in redirects {
                    self.visit_word(&redirect.target);
                }
            }
            Command::Function(function) => {
                self.visit_command(&function.body);
            }
        }
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand) {
        match command {
            BuiltinCommand::Break(command) => {
                if let Some(depth) = &command.depth {
                    self.visit_word(depth);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(depth) = &command.depth {
                    self.visit_word(depth);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(code) = &command.code {
                    self.visit_word(code);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(code) = &command.code {
                    self.visit_word(code);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
                for redirect in &command.redirects {
                    self.visit_word(&redirect.target);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
        }
    }

    fn visit_decl(&mut self, command: &DeclClause) {
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.visit_word(word),
                DeclOperand::Name(_) => {}
                DeclOperand::Assignment(assignment) => self.visit_assignment(assignment),
            }
        }
        for redirect in &command.redirects {
            self.visit_word(&redirect.target);
        }
        for assignment in &command.assignments {
            self.visit_assignment(assignment);
        }
    }

    fn visit_compound(&mut self, compound: &CompoundCommand) {
        match compound {
            CompoundCommand::If(command) => {
                for command in &command.condition {
                    self.visit_command(command);
                }
                for command in &command.then_branch {
                    self.visit_command(command);
                }
                for (condition, branch) in &command.elif_branches {
                    for command in condition {
                        self.visit_command(command);
                    }
                    for command in branch {
                        self.visit_command(command);
                    }
                }
                if let Some(branch) = &command.else_branch {
                    for command in branch {
                        self.visit_command(command);
                    }
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        self.visit_word(word);
                    }
                }
                for command in &command.body {
                    self.visit_command(command);
                }
            }
            CompoundCommand::ArithmeticFor(command) => {
                if let Some(span) = command.init_span {
                    self.analyze_region(span, ArithmeticContextKind::ForInit);
                }
                if let Some(span) = command.condition_span {
                    self.analyze_region(span, ArithmeticContextKind::ForCondition);
                }
                if let Some(span) = command.step_span {
                    self.analyze_region(span, ArithmeticContextKind::ForStep);
                }
                for command in &command.body {
                    self.visit_command(command);
                }
            }
            CompoundCommand::While(command) => {
                for command in &command.condition {
                    self.visit_command(command);
                }
                for command in &command.body {
                    self.visit_command(command);
                }
            }
            CompoundCommand::Until(command) => {
                for command in &command.condition {
                    self.visit_command(command);
                }
                for command in &command.body {
                    self.visit_command(command);
                }
            }
            CompoundCommand::Case(command) => {
                self.visit_word(&command.word);
                for item in &command.cases {
                    self.visit_case_item(item);
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    self.visit_word(word);
                }
                for command in &command.body {
                    self.visit_command(command);
                }
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                for command in commands {
                    self.visit_command(command);
                }
            }
            CompoundCommand::Arithmetic(command) => {
                if let Some(span) = command.expr_span {
                    self.analyze_region(span, ArithmeticContextKind::Command);
                }
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.visit_command(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.visit_conditional_expr(&command.expression);
            }
            CompoundCommand::Coproc(command) => {
                self.visit_command(&command.body);
            }
        }
    }

    fn visit_case_item(&mut self, item: &CaseItem) {
        for pattern in &item.patterns {
            self.visit_word(pattern);
        }
        for command in &item.commands {
            self.visit_command(command);
        }
    }

    fn visit_conditional_expr(&mut self, expr: &ConditionalExpr) {
        match expr {
            ConditionalExpr::Binary(expr) => {
                self.visit_conditional_expr(&expr.left);
                self.visit_conditional_expr(&expr.right);
            }
            ConditionalExpr::Unary(expr) => self.visit_conditional_expr(&expr.expr),
            ConditionalExpr::Parenthesized(expr) => self.visit_conditional_expr(&expr.expr),
            ConditionalExpr::Word(word)
            | ConditionalExpr::Pattern(word)
            | ConditionalExpr::Regex(word) => self.visit_word(word),
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.visit_word(word),
            AssignmentValue::Array(words) => {
                for word in words {
                    self.visit_word(word);
                }
            }
        }
    }

    fn visit_word(&mut self, word: &Word) {
        for (part, span) in word.parts_with_spans() {
            match part {
                WordPart::ArithmeticExpansion(_) => {
                    if let Some(span) = arithmetic_expansion_inner_span(span, self.source) {
                        self.analyze_region(span, ArithmeticContextKind::Expansion);
                    }
                }
                WordPart::CommandSubstitution(commands) => {
                    for command in commands {
                        self.visit_command(command);
                    }
                }
                WordPart::ProcessSubstitution { commands, .. } => {
                    for command in commands {
                        self.visit_command(command);
                    }
                }
                _ => {}
            }
        }
    }

    fn analyze_region(&mut self, span: Span, context: ArithmeticContextKind) {
        let source = span.slice(self.source);
        let tokens = tokenize(source, span.start);
        if tokens.is_empty() {
            return;
        }

        let mut parser = ExprParser::new(tokens);
        let expr = parser.parse_expression();
        emit_expr_events(&expr, context, span, &mut self.events);
    }
}

fn arithmetic_expansion_inner_span(part_span: Span, source: &str) -> Option<Span> {
    let outer = part_span.slice(source);
    if !outer.starts_with("$((") || !outer.ends_with("))") || outer.len() < 5 {
        return None;
    }

    let inner = &outer[3..outer.len() - 2];
    let start = part_span.start.advanced_by("$((");
    let end = start.advanced_by(inner);
    Some(Span::from_positions(start, end))
}

fn emit_expr_events<'a>(
    expr: &Expr<'a>,
    context: ArithmeticContextKind,
    context_span: Span,
    events: &mut Vec<ArithmeticVariableEvent<'a>>,
) {
    match expr {
        Expr::Target(target) => emit_target_read(target, context, context_span, events),
        Expr::Literal => {}
        Expr::Group(expr) | Expr::Unary(expr) => {
            emit_expr_events(expr, context, context_span, events)
        }
        Expr::Mutation(expr) => {
            if let Some(target) = extract_target(expr)
                && target.assignable
            {
                emit_target_index_reads(target, context, context_span, events);
                push_event(
                    target.ident,
                    ArithmeticEventKind::Read,
                    context,
                    context_span,
                    events,
                );
                push_event(
                    target.ident,
                    ArithmeticEventKind::Write,
                    context,
                    context_span,
                    events,
                );
            } else {
                emit_expr_events(expr, context, context_span, events);
            }
        }
        Expr::Binary { left, right } => {
            emit_expr_events(left, context, context_span, events);
            emit_expr_events(right, context, context_span, events);
        }
        Expr::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            emit_expr_events(condition, context, context_span, events);
            emit_expr_events(then_expr, context, context_span, events);
            emit_expr_events(else_expr, context, context_span, events);
        }
        Expr::Assignment { left, op, value } => {
            if let Some(target) = extract_target(left)
                && target.assignable
            {
                emit_target_index_reads(target, context, context_span, events);
                if op.is_compound() {
                    push_event(
                        target.ident,
                        ArithmeticEventKind::Read,
                        context,
                        context_span,
                        events,
                    );
                }
                emit_expr_events(value, context, context_span, events);
                push_event(
                    target.ident,
                    ArithmeticEventKind::Write,
                    context,
                    context_span,
                    events,
                );
            } else {
                emit_expr_events(left, context, context_span, events);
                emit_expr_events(value, context, context_span, events);
            }
        }
        Expr::Comma(exprs) => {
            for expr in exprs {
                emit_expr_events(expr, context, context_span, events);
            }
        }
    }
}

fn emit_target_read<'a>(
    target: &Target<'a>,
    context: ArithmeticContextKind,
    context_span: Span,
    events: &mut Vec<ArithmeticVariableEvent<'a>>,
) {
    emit_target_index_reads(target, context, context_span, events);
    push_event(
        target.ident,
        ArithmeticEventKind::Read,
        context,
        context_span,
        events,
    );
}

fn emit_target_index_reads<'a>(
    target: &Target<'a>,
    context: ArithmeticContextKind,
    context_span: Span,
    events: &mut Vec<ArithmeticVariableEvent<'a>>,
) {
    if let Some(index) = &target.index {
        emit_expr_events(index, context, context_span, events);
    }
}

fn push_event<'a>(
    ident: Ident<'a>,
    kind: ArithmeticEventKind,
    context: ArithmeticContextKind,
    context_span: Span,
    events: &mut Vec<ArithmeticVariableEvent<'a>>,
) {
    events.push(ArithmeticVariableEvent {
        context,
        context_span: context_span.into(),
        kind,
        name: ident.name,
        name_span: ident.span.into(),
    });
}

fn extract_target<'expr, 'a>(expr: &'expr Expr<'a>) -> Option<&'expr Target<'a>> {
    match expr {
        Expr::Target(target) => Some(target),
        Expr::Group(expr) => extract_target(expr),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct Ident<'a> {
    name: &'a str,
    span: Span,
}

#[derive(Debug, Clone)]
struct Target<'a> {
    ident: Ident<'a>,
    index: Option<Box<Expr<'a>>>,
    assignable: bool,
}

#[derive(Debug, Clone)]
enum Expr<'a> {
    Target(Target<'a>),
    Literal,
    Group(Box<Expr<'a>>),
    Unary(Box<Expr<'a>>),
    Mutation(Box<Expr<'a>>),
    Binary {
        left: Box<Expr<'a>>,
        right: Box<Expr<'a>>,
    },
    Ternary {
        condition: Box<Expr<'a>>,
        then_expr: Box<Expr<'a>>,
        else_expr: Box<Expr<'a>>,
    },
    Assignment {
        left: Box<Expr<'a>>,
        op: AssignmentOp,
        value: Box<Expr<'a>>,
    },
    Comma(Vec<Expr<'a>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssignmentOp {
    Assign,
    Compound,
}

impl AssignmentOp {
    fn from_operator(operator: Operator) -> Option<Self> {
        match operator {
            Operator::Assign => Some(Self::Assign),
            Operator::AddAssign
            | Operator::SubAssign
            | Operator::MulAssign
            | Operator::DivAssign
            | Operator::ModAssign
            | Operator::ShiftLeftAssign
            | Operator::ShiftRightAssign
            | Operator::BitAndAssign
            | Operator::BitXorAssign
            | Operator::BitOrAssign => Some(Self::Compound),
            _ => None,
        }
    }

    fn is_compound(self) -> bool {
        matches!(self, Self::Compound)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Increment,
    Decrement,
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    BitAndAssign,
    BitXorAssign,
    BitOrAssign,
    LogicalOr,
    LogicalAnd,
    BitOr,
    BitXor,
    BitAnd,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    ShiftLeft,
    ShiftRight,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy)]
struct Token<'a> {
    kind: TokenKind<'a>,
    span: Span,
}

#[derive(Debug, Clone, Copy)]
enum TokenKind<'a> {
    BareName(&'a str),
    ExpandedName(&'a str),
    Operator(Operator),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Question,
    Colon,
    Comma,
    Literal,
}

struct ExprParser<'a> {
    tokens: Vec<Token<'a>>,
    index: usize,
}

impl<'a> ExprParser<'a> {
    fn new(tokens: Vec<Token<'a>>) -> Self {
        Self { tokens, index: 0 }
    }

    fn parse_expression(&mut self) -> Expr<'a> {
        self.parse_comma()
    }

    fn parse_comma(&mut self) -> Expr<'a> {
        let mut exprs = vec![self.parse_assignment()];
        while self.match_comma() {
            exprs.push(self.parse_assignment());
        }
        if exprs.len() == 1 {
            exprs.pop().unwrap_or(Expr::Literal)
        } else {
            Expr::Comma(exprs)
        }
    }

    fn parse_assignment(&mut self) -> Expr<'a> {
        let left = self.parse_ternary();
        let Some(operator) = self.peek_operator() else {
            return left;
        };
        let Some(op) = AssignmentOp::from_operator(operator) else {
            return left;
        };
        self.advance();
        let value = self.parse_assignment();
        Expr::Assignment {
            left: Box::new(left),
            op,
            value: Box::new(value),
        }
    }

    fn parse_ternary(&mut self) -> Expr<'a> {
        let condition = self.parse_logical_or();
        if !self.match_question() {
            return condition;
        }
        let then_expr = self.parse_assignment();
        let else_expr = if self.match_colon() {
            self.parse_assignment()
        } else {
            Expr::Literal
        };
        Expr::Ternary {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        }
    }

    fn parse_logical_or(&mut self) -> Expr<'a> {
        self.parse_left_associative(Self::parse_logical_and, &[Operator::LogicalOr])
    }

    fn parse_logical_and(&mut self) -> Expr<'a> {
        self.parse_left_associative(Self::parse_bit_or, &[Operator::LogicalAnd])
    }

    fn parse_bit_or(&mut self) -> Expr<'a> {
        self.parse_left_associative(Self::parse_bit_xor, &[Operator::BitOr])
    }

    fn parse_bit_xor(&mut self) -> Expr<'a> {
        self.parse_left_associative(Self::parse_bit_and, &[Operator::BitXor])
    }

    fn parse_bit_and(&mut self) -> Expr<'a> {
        self.parse_left_associative(Self::parse_equality, &[Operator::BitAnd])
    }

    fn parse_equality(&mut self) -> Expr<'a> {
        self.parse_left_associative(
            Self::parse_relational,
            &[Operator::Equal, Operator::NotEqual],
        )
    }

    fn parse_relational(&mut self) -> Expr<'a> {
        self.parse_left_associative(
            Self::parse_shift,
            &[
                Operator::Less,
                Operator::LessEqual,
                Operator::Greater,
                Operator::GreaterEqual,
            ],
        )
    }

    fn parse_shift(&mut self) -> Expr<'a> {
        self.parse_left_associative(
            Self::parse_additive,
            &[Operator::ShiftLeft, Operator::ShiftRight],
        )
    }

    fn parse_additive(&mut self) -> Expr<'a> {
        self.parse_left_associative(Self::parse_multiplicative, &[Operator::Add, Operator::Sub])
    }

    fn parse_multiplicative(&mut self) -> Expr<'a> {
        self.parse_left_associative(
            Self::parse_unary,
            &[Operator::Mul, Operator::Div, Operator::Mod],
        )
    }

    fn parse_left_associative(
        &mut self,
        parse_next: fn(&mut Self) -> Expr<'a>,
        operators: &[Operator],
    ) -> Expr<'a> {
        let mut expr = parse_next(self);
        while let Some(operator) = self.peek_operator() {
            if !operators.contains(&operator) {
                break;
            }
            self.advance();
            let right = parse_next(self);
            expr = Expr::Binary {
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        expr
    }

    fn parse_unary(&mut self) -> Expr<'a> {
        match self.peek_operator() {
            Some(Operator::Increment) | Some(Operator::Decrement) => {
                self.advance();
                Expr::Mutation(Box::new(self.parse_unary()))
            }
            Some(Operator::Add)
            | Some(Operator::Sub)
            | Some(Operator::Not)
            | Some(Operator::BitNot) => {
                self.advance();
                Expr::Unary(Box::new(self.parse_unary()))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr<'a> {
        let mut expr = self.parse_primary();
        loop {
            if self.match_lbracket() {
                let index = self.parse_comma();
                self.match_rbracket();
                expr = if let Some(mut target) = into_target(expr) {
                    if target.index.is_none() {
                        target.index = Some(Box::new(index));
                        Expr::Target(target)
                    } else {
                        Expr::Literal
                    }
                } else {
                    Expr::Literal
                };
                continue;
            }

            match self.peek_operator() {
                Some(Operator::Increment) | Some(Operator::Decrement) => {
                    self.advance();
                    expr = Expr::Mutation(Box::new(expr));
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr<'a> {
        let Some(token) = self.peek() else {
            return Expr::Literal;
        };

        match token.kind {
            TokenKind::BareName(name) => {
                let span = token.span;
                self.advance();
                Expr::Target(Target {
                    ident: Ident { name, span },
                    index: None,
                    assignable: true,
                })
            }
            TokenKind::ExpandedName(name) => {
                let span = token.span;
                self.advance();
                Expr::Target(Target {
                    ident: Ident { name, span },
                    index: None,
                    assignable: false,
                })
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_comma();
                self.match_rparen();
                Expr::Group(Box::new(expr))
            }
            _ => {
                self.advance();
                Expr::Literal
            }
        }
    }

    fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.index).copied()
    }

    fn peek_operator(&self) -> Option<Operator> {
        match self.peek()?.kind {
            TokenKind::Operator(operator) => Some(operator),
            _ => None,
        }
    }

    fn advance(&mut self) -> Option<Token<'a>> {
        let token = self.peek()?;
        self.index += 1;
        Some(token)
    }

    fn match_kind(&mut self, kind: fn(TokenKind<'a>) -> bool) -> bool {
        if let Some(token) = self.peek()
            && kind(token.kind)
        {
            self.advance();
            return true;
        }
        false
    }

    fn match_question(&mut self) -> bool {
        self.match_kind(|kind| matches!(kind, TokenKind::Question))
    }

    fn match_colon(&mut self) -> bool {
        self.match_kind(|kind| matches!(kind, TokenKind::Colon))
    }

    fn match_comma(&mut self) -> bool {
        self.match_kind(|kind| matches!(kind, TokenKind::Comma))
    }

    fn match_lbracket(&mut self) -> bool {
        self.match_kind(|kind| matches!(kind, TokenKind::LBracket))
    }

    fn match_rbracket(&mut self) -> bool {
        self.match_kind(|kind| matches!(kind, TokenKind::RBracket))
    }

    fn match_rparen(&mut self) -> bool {
        self.match_kind(|kind| matches!(kind, TokenKind::RParen))
    }
}

fn into_target<'a>(expr: Expr<'a>) -> Option<Target<'a>> {
    match expr {
        Expr::Target(target) => Some(target),
        Expr::Group(expr) => into_target(*expr),
        _ => None,
    }
}

fn tokenize<'a>(input: &'a str, base: Position) -> Vec<Token<'a>> {
    let mut tokens = Vec::new();
    let mut index = 0usize;
    let mut cursor = base;

    while let Some(ch) = input[index..].chars().next() {
        if ch.is_whitespace() {
            cursor.advance(ch);
            index += ch.len_utf8();
            continue;
        }

        let start_index = index;
        let start = cursor;

        if is_identifier_start(ch) {
            consume_char(&mut index, &mut cursor, ch);
            while let Some(next) = input[index..].chars().next() {
                if !is_identifier_continue(next) {
                    break;
                }
                consume_char(&mut index, &mut cursor, next);
            }
            tokens.push(Token {
                kind: TokenKind::BareName(&input[start_index..index]),
                span: Span::from_positions(start, cursor),
            });
            continue;
        }

        if ch.is_ascii_digit() {
            consume_char(&mut index, &mut cursor, ch);
            while let Some(next) = input[index..].chars().next() {
                if !is_number_continue(next) {
                    break;
                }
                consume_char(&mut index, &mut cursor, next);
            }
            tokens.push(Token {
                kind: TokenKind::Literal,
                span: Span::from_positions(start, cursor),
            });
            continue;
        }

        match ch {
            '$' => {
                tokenize_dollar(input, &mut index, &mut cursor, &mut tokens);
            }
            '\'' => {
                consume_single_quoted(input, &mut index, &mut cursor);
                tokens.push(Token {
                    kind: TokenKind::Literal,
                    span: Span::from_positions(start, cursor),
                });
            }
            '"' => {
                consume_double_quoted(input, &mut index, &mut cursor);
                tokens.push(Token {
                    kind: TokenKind::Literal,
                    span: Span::from_positions(start, cursor),
                });
            }
            '(' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::LParen,
                    span: Span::from_positions(start, cursor),
                });
            }
            ')' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::RParen,
                    span: Span::from_positions(start, cursor),
                });
            }
            '[' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::LBracket,
                    span: Span::from_positions(start, cursor),
                });
            }
            ']' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::RBracket,
                    span: Span::from_positions(start, cursor),
                });
                if let Some('}') = input[index..].chars().next() {
                    consume_char(&mut index, &mut cursor, '}');
                }
            }
            '?' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::Question,
                    span: Span::from_positions(start, cursor),
                });
            }
            ':' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::Colon,
                    span: Span::from_positions(start, cursor),
                });
            }
            ',' => {
                consume_char(&mut index, &mut cursor, ch);
                tokens.push(Token {
                    kind: TokenKind::Comma,
                    span: Span::from_positions(start, cursor),
                });
            }
            '{' | '}' => {
                consume_char(&mut index, &mut cursor, ch);
            }
            _ => {
                if let Some((operator, width)) = operator_at(&input[index..]) {
                    for op_char in input[index..index + width].chars() {
                        consume_char(&mut index, &mut cursor, op_char);
                    }
                    tokens.push(Token {
                        kind: TokenKind::Operator(operator),
                        span: Span::from_positions(start, cursor),
                    });
                } else {
                    consume_char(&mut index, &mut cursor, ch);
                    tokens.push(Token {
                        kind: TokenKind::Literal,
                        span: Span::from_positions(start, cursor),
                    });
                }
            }
        }
    }

    tokens
}

fn tokenize_dollar<'a>(
    input: &'a str,
    index: &mut usize,
    cursor: &mut Position,
    tokens: &mut Vec<Token<'a>>,
) {
    let start = *cursor;
    consume_char(index, cursor, '$');

    let Some(next) = input[*index..].chars().next() else {
        return;
    };

    if next == '{' {
        consume_char(index, cursor, '{');

        if let Some(prefix) = input[*index..].chars().next()
            && matches!(prefix, '#' | '!')
        {
            consume_char(index, cursor, prefix);
        }

        let Some(name_start_ch) = input[*index..].chars().next() else {
            return;
        };
        if !is_identifier_start(name_start_ch) {
            skip_until_closing_brace(input, index, cursor);
            return;
        }

        let name_start = *cursor;
        let name_offset = *index;
        consume_char(index, cursor, name_start_ch);
        while let Some(ch) = input[*index..].chars().next() {
            if !is_identifier_continue(ch) {
                break;
            }
            consume_char(index, cursor, ch);
        }

        tokens.push(Token {
            kind: TokenKind::ExpandedName(&input[name_offset..*index]),
            span: Span::from_positions(name_start, *cursor),
        });

        match input[*index..].chars().next() {
            Some('[') => {
                let bracket_start = *cursor;
                consume_char(index, cursor, '[');
                tokens.push(Token {
                    kind: TokenKind::LBracket,
                    span: Span::from_positions(bracket_start, *cursor),
                });
            }
            Some('}') => {
                consume_char(index, cursor, '}');
            }
            Some(_) => skip_until_closing_brace(input, index, cursor),
            None => {}
        }
        return;
    }

    if is_identifier_start(next) {
        let name_start = *cursor;
        let name_offset = *index;
        consume_char(index, cursor, next);
        while let Some(ch) = input[*index..].chars().next() {
            if !is_identifier_continue(ch) {
                break;
            }
            consume_char(index, cursor, ch);
        }

        tokens.push(Token {
            kind: TokenKind::ExpandedName(&input[name_offset..*index]),
            span: Span::from_positions(name_start, *cursor),
        });
        return;
    }

    if next == '(' {
        consume_char(index, cursor, '(');
        if let Some('(') = input[*index..].chars().next() {
            consume_balanced_arithmetic_expansion(input, index, cursor);
        } else {
            consume_balanced_command_substitution(input, index, cursor);
        }
        tokens.push(Token {
            kind: TokenKind::Literal,
            span: Span::from_positions(start, *cursor),
        });
        return;
    }

    tokens.push(Token {
        kind: TokenKind::Literal,
        span: Span::from_positions(start, *cursor),
    });
}

fn operator_at(input: &str) -> Option<(Operator, usize)> {
    const OPERATORS: &[(&str, Operator)] = &[
        ("<<=", Operator::ShiftLeftAssign),
        (">>=", Operator::ShiftRightAssign),
        ("++", Operator::Increment),
        ("--", Operator::Decrement),
        ("+=", Operator::AddAssign),
        ("-=", Operator::SubAssign),
        ("*=", Operator::MulAssign),
        ("/=", Operator::DivAssign),
        ("%=", Operator::ModAssign),
        ("&=", Operator::BitAndAssign),
        ("^=", Operator::BitXorAssign),
        ("|=", Operator::BitOrAssign),
        ("&&", Operator::LogicalAnd),
        ("||", Operator::LogicalOr),
        ("==", Operator::Equal),
        ("!=", Operator::NotEqual),
        ("<=", Operator::LessEqual),
        (">=", Operator::GreaterEqual),
        ("<<", Operator::ShiftLeft),
        (">>", Operator::ShiftRight),
        ("=", Operator::Assign),
        ("<", Operator::Less),
        (">", Operator::Greater),
        ("+", Operator::Add),
        ("-", Operator::Sub),
        ("*", Operator::Mul),
        ("/", Operator::Div),
        ("%", Operator::Mod),
        ("!", Operator::Not),
        ("~", Operator::BitNot),
        ("&", Operator::BitAnd),
        ("^", Operator::BitXor),
        ("|", Operator::BitOr),
    ];

    OPERATORS
        .iter()
        .find_map(|(text, operator)| input.starts_with(text).then_some((*operator, text.len())))
}

fn consume_char(index: &mut usize, cursor: &mut Position, ch: char) {
    *index += ch.len_utf8();
    cursor.advance(ch);
}

fn consume_single_quoted(input: &str, index: &mut usize, cursor: &mut Position) {
    consume_char(index, cursor, '\'');
    while let Some(ch) = input[*index..].chars().next() {
        consume_char(index, cursor, ch);
        if ch == '\'' {
            break;
        }
    }
}

fn consume_double_quoted(input: &str, index: &mut usize, cursor: &mut Position) {
    consume_char(index, cursor, '"');
    let mut escaped = false;
    while let Some(ch) = input[*index..].chars().next() {
        consume_char(index, cursor, ch);
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            break;
        }
    }
}

fn consume_balanced_command_substitution(input: &str, index: &mut usize, cursor: &mut Position) {
    let mut depth = 1usize;
    while let Some(ch) = input[*index..].chars().next() {
        consume_char(index, cursor, ch);
        match ch {
            '\'' => consume_single_quoted(input, index, cursor),
            '"' => consume_double_quoted(input, index, cursor),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
}

fn consume_balanced_arithmetic_expansion(input: &str, index: &mut usize, cursor: &mut Position) {
    consume_char(index, cursor, '(');
    let mut depth = 1usize;
    while let Some(ch) = input[*index..].chars().next() {
        consume_char(index, cursor, ch);
        if ch == '(' {
            depth += 1;
            continue;
        }
        if ch == ')' {
            if let Some(')') = input[*index..].chars().next() {
                consume_char(index, cursor, ')');
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
                continue;
            }
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
}

fn skip_until_closing_brace(input: &str, index: &mut usize, cursor: &mut Position) {
    while let Some(ch) = input[*index..].chars().next() {
        consume_char(index, cursor, ch);
        if ch == '}' {
            break;
        }
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_number_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '#' | '.')
}

#[cfg(test)]
mod tests {
    use super::{
        ArithmeticContextKind, ArithmeticEventKind, ArithmeticVariableEvent,
        collect_arithmetic_variable_events,
    };
    use crate::{ParseOptions, parse};

    fn events(input: &str) -> Vec<ArithmeticVariableEvent<'_>> {
        let parsed = parse(input, ParseOptions::default()).unwrap();
        collect_arithmetic_variable_events(input, &parsed.script)
    }

    #[test]
    fn collects_reads_and_writes_from_arithmetic_command() {
        let events = events("(( total += items[i] + bonus ))\n");
        let actual: Vec<(&str, ArithmeticEventKind, ArithmeticContextKind)> = events
            .iter()
            .map(|event| (event.name, event.kind, event.context))
            .collect();

        assert_eq!(
            actual,
            vec![
                (
                    "total",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::Command
                ),
                (
                    "i",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::Command
                ),
                (
                    "items",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::Command
                ),
                (
                    "bonus",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::Command
                ),
                (
                    "total",
                    ArithmeticEventKind::Write,
                    ArithmeticContextKind::Command
                ),
            ]
        );
    }

    #[test]
    fn collects_events_from_arithmetic_for_header_regions() {
        let events = events("for (( i = start ; i < limit ; i++ )); do echo ok; done\n");
        let actual: Vec<(&str, ArithmeticEventKind, ArithmeticContextKind)> = events
            .iter()
            .map(|event| (event.name, event.kind, event.context))
            .collect();

        assert_eq!(
            actual,
            vec![
                (
                    "start",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::ForInit
                ),
                (
                    "i",
                    ArithmeticEventKind::Write,
                    ArithmeticContextKind::ForInit
                ),
                (
                    "i",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::ForCondition
                ),
                (
                    "limit",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::ForCondition
                ),
                (
                    "i",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::ForStep
                ),
                (
                    "i",
                    ArithmeticEventKind::Write,
                    ArithmeticContextKind::ForStep
                ),
            ]
        );
    }

    #[test]
    fn collects_events_from_arithmetic_expansions() {
        let events = events("echo $(( total = total + step ))\n");
        let actual: Vec<(&str, ArithmeticEventKind, ArithmeticContextKind)> = events
            .iter()
            .map(|event| (event.name, event.kind, event.context))
            .collect();

        assert_eq!(
            actual,
            vec![
                (
                    "total",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::Expansion
                ),
                (
                    "step",
                    ArithmeticEventKind::Read,
                    ArithmeticContextKind::Expansion
                ),
                (
                    "total",
                    ArithmeticEventKind::Write,
                    ArithmeticContextKind::Expansion
                ),
            ]
        );
    }

    #[test]
    fn reads_array_indexes_before_writing_target() {
        let events = events("(( arr[idx + 1] = value ))\n");
        let actual: Vec<(&str, ArithmeticEventKind, usize)> = events
            .iter()
            .map(|event| (event.name, event.kind, event.name_span.start.column))
            .collect();

        assert_eq!(
            actual,
            vec![
                ("idx", ArithmeticEventKind::Read, 8),
                ("value", ArithmeticEventKind::Read, 19),
                ("arr", ArithmeticEventKind::Write, 4),
            ]
        );
    }
}
