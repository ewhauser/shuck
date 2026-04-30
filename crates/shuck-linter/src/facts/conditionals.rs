#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalOperatorFamily {
    StringUnary,
    StringBinary,
    Regex,
    Logical,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct ConditionalOperandFact<'a> {
    expression: &'a ConditionalExpr,
    class: TestOperandClass,
    word: Option<&'a Word>,
    word_classification: Option<WordClassification>,
}

impl<'a> ConditionalOperandFact<'a> {
    pub fn expression(&self) -> &'a ConditionalExpr {
        self.expression
    }

    pub fn class(&self) -> TestOperandClass {
        self.class
    }

    pub fn word(&self) -> Option<&'a Word> {
        self.word
    }

    pub fn word_classification(&self) -> Option<WordClassification> {
        self.word_classification
    }

    pub fn quote(&self) -> Option<WordQuote> {
        self.word_classification
            .map(|classification| classification.quote)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConditionalBareWordFact<'a> {
    expression: &'a ConditionalExpr,
    operand: ConditionalOperandFact<'a>,
}

impl<'a> ConditionalBareWordFact<'a> {
    pub fn expression(&self) -> &'a ConditionalExpr {
        self.expression
    }

    pub fn operand(&self) -> ConditionalOperandFact<'a> {
        self.operand
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConditionalUnaryFact<'a> {
    expression: &'a ConditionalExpr,
    op: ConditionalUnaryOp,
    operator_family: ConditionalOperatorFamily,
    operand: ConditionalOperandFact<'a>,
}

impl<'a> ConditionalUnaryFact<'a> {
    pub fn expression(&self) -> &'a ConditionalExpr {
        self.expression
    }

    pub fn operator_span(&self) -> Span {
        let ConditionalExpr::Unary(expression) = self.expression else {
            unreachable!("conditional unary fact should wrap a unary expression");
        };

        expression.op_span
    }

    pub fn op(&self) -> ConditionalUnaryOp {
        self.op
    }

    pub fn operator_family(&self) -> ConditionalOperatorFamily {
        self.operator_family
    }

    pub fn operand(&self) -> ConditionalOperandFact<'a> {
        self.operand
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConditionalBinaryFact<'a> {
    expression: &'a ConditionalExpr,
    op: ConditionalBinaryOp,
    operator_family: ConditionalOperatorFamily,
    left: ConditionalOperandFact<'a>,
    right: ConditionalOperandFact<'a>,
}

impl<'a> ConditionalBinaryFact<'a> {
    pub fn expression(&self) -> &'a ConditionalExpr {
        self.expression
    }

    pub fn operator_span(&self) -> Span {
        let ConditionalExpr::Binary(expression) = self.expression else {
            unreachable!("conditional binary fact should wrap a binary expression");
        };

        expression.op_span
    }

    pub fn op(&self) -> ConditionalBinaryOp {
        self.op
    }

    pub fn operator_family(&self) -> ConditionalOperatorFamily {
        self.operator_family
    }

    pub fn left(&self) -> ConditionalOperandFact<'a> {
        self.left
    }

    pub fn right(&self) -> ConditionalOperandFact<'a> {
        self.right
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ConditionalNodeFact<'a> {
    BareWord(ConditionalBareWordFact<'a>),
    Unary(ConditionalUnaryFact<'a>),
    Binary(ConditionalBinaryFact<'a>),
    Other(&'a ConditionalExpr),
}

impl<'a> ConditionalNodeFact<'a> {
    pub fn expression(&self) -> &'a ConditionalExpr {
        match self {
            Self::BareWord(fact) => fact.expression(),
            Self::Unary(fact) => fact.expression(),
            Self::Binary(fact) => fact.expression(),
            Self::Other(expression) => expression,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalMixedLogicalOperatorFact {
    operator_span: Span,
    grouped_subexpression_spans: Box<[Span]>,
}

impl ConditionalMixedLogicalOperatorFact {
    pub fn operator_span(&self) -> Span {
        self.operator_span
    }

    pub fn grouped_subexpression_spans(&self) -> &[Span] {
        &self.grouped_subexpression_spans
    }
}

#[derive(Debug, Clone)]
pub struct ConditionalFact<'a> {
    nodes: Box<[ConditionalNodeFact<'a>]>,
    mixed_logical_operators: Box<[ConditionalMixedLogicalOperatorFact]>,
}

impl<'a> ConditionalFact<'a> {
    pub fn expression(&self) -> &'a ConditionalExpr {
        self.root().expression()
    }

    pub fn root(&self) -> &ConditionalNodeFact<'a> {
        &self.nodes[0]
    }

    pub fn nodes(&self) -> &[ConditionalNodeFact<'a>] {
        &self.nodes
    }

    pub fn mixed_logical_operators(&self) -> &[ConditionalMixedLogicalOperatorFact] {
        &self.mixed_logical_operators
    }

    pub fn regex_nodes(&self) -> impl Iterator<Item = &ConditionalBinaryFact<'a>> + '_ {
        self.nodes.iter().filter_map(|node| match node {
            ConditionalNodeFact::Binary(fact)
                if fact.operator_family() == ConditionalOperatorFamily::Regex =>
            {
                Some(fact)
            }
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ConditionalExpressionVisit<'a> {
    expression: &'a ConditionalExpr,
    parent_in_same_logical_group: bool,
}

impl<'a> ConditionalExpressionVisit<'a> {
    pub(crate) fn new(
        expression: &'a ConditionalExpr,
        parent_in_same_logical_group: bool,
    ) -> Self {
        Self {
            expression,
            parent_in_same_logical_group,
        }
    }
}

fn collect_command_substitution_command_span(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Command::Simple(command) = command
        && command.args.is_empty()
        && command_name_is_plain_command_substitution(&command.name, source)
    {
        spans.push(command.name.span);
    }
}

fn collect_c107_status_spans_in_simple_test(
    command: &shuck_ast::SimpleCommand,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if static_word_text(&command.name, source).as_deref() != Some("[") {
        return;
    }

    let Some((closing_bracket, operands)) = command.args.split_last() else {
        return;
    };
    if static_word_text(closing_bracket, source).as_deref() != Some("]") {
        return;
    }

    let operands = operands.iter().collect::<Vec<_>>();
    let effective_operand_offset = simple_test_effective_operand_offset(&operands, source);
    let effective_operands = &operands[effective_operand_offset..];
    if effective_operands.len() != 3 {
        return;
    }

    let Some(operator) = static_word_text(effective_operands[1], source) else {
        return;
    };
    if !matches!(
        operator.as_ref(),
        "=" | "==" | "!=" | "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge"
    ) {
        return;
    }

    let left_status = c107_status_word_span(effective_operands[0]);
    let right_status = c107_status_word_span(effective_operands[2]);
    let left_zero = c107_word_is_zero_literal(effective_operands[0], source);
    let right_zero = c107_word_is_zero_literal(effective_operands[2], source);

    if let Some(span) = left_status.filter(|_| right_zero) {
        spans.push(span);
    } else if let Some(span) = right_status.filter(|_| left_zero) {
        spans.push(span);
    }
}

fn collect_c107_status_spans_in_conditional_expr(
    expression: &ConditionalExpr,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(span) = c107_conditional_expr_status_span(expression, source) {
        spans.push(span);
    }
}

fn c107_conditional_expr_status_span(expression: &ConditionalExpr, source: &str) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expression) => {
            if matches!(
                expression.op,
                ConditionalBinaryOp::And | ConditionalBinaryOp::Or
            ) {
                return None;
            }
            if !matches!(
                expression.op,
                ConditionalBinaryOp::ArithmeticEq
                    | ConditionalBinaryOp::ArithmeticNe
                    | ConditionalBinaryOp::ArithmeticLe
                    | ConditionalBinaryOp::ArithmeticGe
                    | ConditionalBinaryOp::ArithmeticLt
                    | ConditionalBinaryOp::ArithmeticGt
                    | ConditionalBinaryOp::PatternEqShort
                    | ConditionalBinaryOp::PatternEq
                    | ConditionalBinaryOp::PatternNe
            ) {
                return None;
            }

            let left_status = c107_conditional_operand_status_span(&expression.left);
            let right_status = c107_conditional_operand_status_span(&expression.right);
            let left_zero = c107_conditional_expr_is_zero_literal(&expression.left, source);
            let right_zero = c107_conditional_expr_is_zero_literal(&expression.right, source);

            left_status
                .filter(|_| right_zero)
                .or_else(|| right_status.filter(|_| left_zero))
        }
        ConditionalExpr::Unary(expression) => {
            c107_conditional_expr_status_span(&expression.expr, source)
        }
        ConditionalExpr::Parenthesized(expression) => {
            c107_conditional_expr_status_span(&expression.expr, source)
        }
        ConditionalExpr::Word(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => None,
    }
}

fn c107_conditional_operand_status_span(expression: &ConditionalExpr) -> Option<Span> {
    match expression {
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => c107_status_word_span(word),
        ConditionalExpr::Pattern(pattern) => {
            pattern.parts.iter().find_map(|part| match &part.kind {
                PatternPart::Word(word) => c107_status_word_span(word),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_)
                | PatternPart::Group { .. } => None,
            })
        }
        ConditionalExpr::VarRef(reference) => {
            (reference.name.as_str() == "?").then_some(reference.span)
        }
        ConditionalExpr::Parenthesized(expression) => {
            c107_conditional_operand_status_span(&expression.expr)
        }
        ConditionalExpr::Unary(expression) => {
            c107_conditional_operand_status_span(&expression.expr)
        }
        ConditionalExpr::Binary(_) => None,
    }
}

fn c107_conditional_expr_is_zero_literal(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            c107_word_is_zero_literal(word, source)
        }
        ConditionalExpr::Pattern(pattern) => c107_pattern_is_zero_literal(pattern, source),
        ConditionalExpr::Parenthesized(expression) => {
            c107_conditional_expr_is_zero_literal(&expression.expr, source)
        }
        ConditionalExpr::Unary(expression) => {
            c107_conditional_expr_is_zero_literal(&expression.expr, source)
        }
        ConditionalExpr::VarRef(_) | ConditionalExpr::Binary(_) => false,
    }
}

fn collect_c107_status_spans_in_arithmetic_command(
    command: &shuck_ast::ArithmeticCommand,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(expression) = &command.expr_ast else {
        return;
    };

    if let Some(span) = c107_arithmetic_expr_status_span(expression, source) {
        spans.push(span);
    }
}

fn c107_arithmetic_expr_status_span(
    expression: &shuck_ast::ArithmeticExprNode,
    source: &str,
) -> Option<Span> {
    match &expression.kind {
        shuck_ast::ArithmeticExpr::Parenthesized { expression } => {
            c107_arithmetic_expr_status_span(expression, source)
        }
        shuck_ast::ArithmeticExpr::Unary { expr, .. } => {
            c107_arithmetic_expr_status_span(expr, source)
        }
        shuck_ast::ArithmeticExpr::Binary { left, op, right } => {
            if !matches!(
                op,
                shuck_ast::ArithmeticBinaryOp::LessThan
                    | shuck_ast::ArithmeticBinaryOp::LessThanOrEqual
                    | shuck_ast::ArithmeticBinaryOp::GreaterThan
                    | shuck_ast::ArithmeticBinaryOp::GreaterThanOrEqual
                    | shuck_ast::ArithmeticBinaryOp::Equal
                    | shuck_ast::ArithmeticBinaryOp::NotEqual
            ) {
                return None;
            }

            let left_status = c107_arithmetic_operand_status_span(left);
            let right_status = c107_arithmetic_operand_status_span(right);
            let left_zero = c107_arithmetic_expr_is_zero_literal(left, source);
            let right_zero = c107_arithmetic_expr_is_zero_literal(right, source);

            left_status
                .filter(|_| right_zero)
                .or_else(|| right_status.filter(|_| left_zero))
        }
        _ => None,
    }
}

fn c107_arithmetic_operand_status_span(expression: &shuck_ast::ArithmeticExprNode) -> Option<Span> {
    match &expression.kind {
        shuck_ast::ArithmeticExpr::ShellWord(word) => c107_status_word_span(word),
        shuck_ast::ArithmeticExpr::Parenthesized { expression } => {
            c107_arithmetic_operand_status_span(expression)
        }
        shuck_ast::ArithmeticExpr::Unary { expr, .. } => c107_arithmetic_operand_status_span(expr),
        _ => None,
    }
}

fn c107_arithmetic_expr_is_zero_literal(
    expression: &shuck_ast::ArithmeticExprNode,
    source: &str,
) -> bool {
    match &expression.kind {
        shuck_ast::ArithmeticExpr::Number(text) => text.slice(source).trim() == "0",
        shuck_ast::ArithmeticExpr::ShellWord(word) => c107_word_is_zero_literal(word, source),
        shuck_ast::ArithmeticExpr::Parenthesized { expression } => {
            c107_arithmetic_expr_is_zero_literal(expression, source)
        }
        shuck_ast::ArithmeticExpr::Unary { expr, .. } => {
            c107_arithmetic_expr_is_zero_literal(expr, source)
        }
        _ => false,
    }
}

fn c107_status_word_span(word: &Word) -> Option<Span> {
    word_is_standalone_status_capture(word).then_some(word.span)
}

fn c107_word_is_zero_literal(word: &Word, source: &str) -> bool {
    static_word_text(word, source).as_deref() == Some("0")
}

fn c107_pattern_is_zero_literal(pattern: &Pattern, source: &str) -> bool {
    match pattern.parts.as_slice() {
        [part] => match &part.kind {
            PatternPart::Literal(text) => text.as_str(source, part.span) == "0",
            PatternPart::Word(word) => c107_word_is_zero_literal(word, source),
            PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_)
            | PatternPart::Group { .. } => false,
        },
        _ => false,
    }
}

fn collect_condition_status_capture_from_body(
    condition: &StmtSeq,
    body: &StmtSeq,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if !condition_terminals_are_test_commands(condition, source) {
        return;
    }

    let Some(first_stmt) = body.first() else {
        return;
    };

    let mut stmt_spans = Vec::new();
    collect_status_parameter_spans_in_stmt(first_stmt, source, &mut stmt_spans);
    if stmt_spans.is_empty()
        || branch_body_status_capture_is_exempt(first_stmt, body, source, &stmt_spans)
    {
        return;
    }

    spans.extend(stmt_spans);
}

pub(super) fn collect_condition_status_capture_from_sequence(
    commands: &StmtSeq,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let body_shape = BodyShapeAnalyzer::new(source);
    let mut segment_start = 0;
    for (index, stmt) in commands.iter().enumerate() {
        if !stmt_starts_sequence_barrier(stmt) {
            continue;
        }

        collect_condition_status_capture_from_sequence_segment(
            &commands.as_slice()[segment_start..index],
            source,
            spans,
            body_shape.sequence_tail_contains_nested_test_command(&commands.as_slice()[index..]),
            segment_start > 0,
        );
        segment_start = index + 1;
    }
    collect_condition_status_capture_from_sequence_segment(
        &commands.as_slice()[segment_start..],
        source,
        spans,
        false,
        segment_start > 0,
    );

    for (index, stmt) in commands.iter().enumerate() {
        if !body_shape.sequence_tail_contains_nested_test_command(&commands.as_slice()[index + 1..])
        {
            continue;
        }

        let Command::Compound(CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body)) =
            &stmt.command
        else {
            continue;
        };

        body_shape.collect_status_test_followup_chains_with_sibling_tail(body, spans);
    }
}

pub(super) fn collect_condition_status_capture_from_direct_body_sequences(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                collect_condition_status_capture_from_sequence(&command.then_branch, source, spans);
                for (_, branch) in &command.elif_branches {
                    collect_condition_status_capture_from_sequence(branch, source, spans);
                }
                if let Some(branch) = &command.else_branch {
                    collect_condition_status_capture_from_sequence(branch, source, spans);
                }
            }
            CompoundCommand::While(command) => {
                collect_condition_status_capture_from_sequence(&command.body, source, spans);
            }
            CompoundCommand::Until(command) => {
                collect_condition_status_capture_from_sequence(&command.body, source, spans);
            }
            CompoundCommand::For(command) => {
                collect_condition_status_capture_from_sequence(&command.body, source, spans);
            }
            CompoundCommand::Select(command) => {
                collect_condition_status_capture_from_sequence(&command.body, source, spans);
            }
            CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => {
                collect_condition_status_capture_from_sequence(body, source, spans);
            }
            CompoundCommand::Time(_) => {}
            CompoundCommand::Always(command) => {
                collect_condition_status_capture_from_sequence(&command.body, source, spans);
                collect_condition_status_capture_from_sequence(&command.always_body, source, spans);
            }
            CompoundCommand::Case(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Coproc(_) => {}
        },
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

fn collect_condition_status_capture_from_sequence_segment(
    commands: &[Stmt],
    source: &str,
    spans: &mut Vec<Span>,
    trailing_tail_has_test: bool,
    has_prior_barrier: bool,
) {
    let tail_contains_test = sequence_tail_test_index(commands, source);
    let mut recent = RecentSequenceStatus::default();
    let body_topology = BodyTopology::from_statements(commands);
    for (index, previous, current) in body_topology.indexed_sibling_pairs() {
        if let Some(before_previous) = body_topology.previous_sibling(index) {
            recent.push(before_previous, source);
        }
        let tail_has_test = tail_contains_test
            .get(index + 2)
            .copied()
            .unwrap_or(false);
        let effective_tail_has_test = trailing_tail_has_test || tail_has_test;
        let in_tbegin_region = recent.seen_tbegin;

        if stmt_is_assignment_only_unquoted_status_capture(current) {
            if stmt_is_standalone_non_status_test_command(previous, source)
                && in_tbegin_region
                && recent.status_capture_count >= 2
                && tail_has_test
            {
                collect_status_parameter_spans_in_stmt(current, source, spans);
            }
            continue;
        }

        if !stmt_is_plain_command_with_standalone_status_argument(current) {
            continue;
        }

        if stmt_is_status_based_test_command(previous, source) {
            if (!has_prior_barrier && effective_tail_has_test) || recent.contains_status_based_test
            {
                collect_status_parameter_spans_in_stmt(current, source, spans);
            }
            continue;
        }

        if !stmt_is_standalone_non_status_test_command(previous, source) {
            continue;
        }

        if in_tbegin_region
            && recent.has_prior_non_status_capture_pair
        {
            continue;
        }

        if effective_tail_has_test {
            collect_status_parameter_spans_in_stmt(current, source, spans);
        }
    }
}

#[derive(Default)]
struct RecentSequenceStatus<'a> {
    seen_tbegin: bool,
    status_capture_count: usize,
    contains_status_based_test: bool,
    has_prior_non_status_capture_pair: bool,
    last_stmt: Option<&'a Stmt>,
}

impl<'a> RecentSequenceStatus<'a> {
    fn push(&mut self, stmt: &'a Stmt, source: &str) {
        if stmt_is_named_simple_command(stmt, source, "tbegin") {
            self.seen_tbegin = true;
            self.status_capture_count = 0;
            self.contains_status_based_test = false;
            self.has_prior_non_status_capture_pair = false;
            self.last_stmt = None;
            return;
        }

        let contains_status_capture = stmt_contains_status_capture(stmt, source);
        if self
            .last_stmt
            .is_some_and(|previous| stmt_is_standalone_non_status_test_command(previous, source))
            && contains_status_capture
        {
            self.has_prior_non_status_capture_pair = true;
        }
        if contains_status_capture {
            self.status_capture_count += 1;
        }
        if stmt_is_status_based_test_command(stmt, source) {
            self.contains_status_based_test = true;
        }
        self.last_stmt = Some(stmt);
    }
}

fn sequence_tail_test_index(commands: &[Stmt], source: &str) -> Vec<bool> {
    let mut suffix = vec![false; commands.len() + 1];
    for (index, stmt) in commands.iter().enumerate().rev() {
        suffix[index] = suffix[index + 1] || stmt_terminals_are_test_commands(stmt, source);
    }
    suffix
}

fn branch_body_status_capture_is_exempt(
    first_stmt: &Stmt,
    body: &StmtSeq,
    source: &str,
    stmt_spans: &[Span],
) -> bool {
    if branch_body_preserves_status_in_plain_assignment(first_stmt, body, source, stmt_spans) {
        return true;
    }

    if branch_body_logs_status_then_immediately_exits(first_stmt, body, source, stmt_spans) {
        return true;
    }

    false
}

fn branch_body_preserves_status_in_plain_assignment(
    first_stmt: &Stmt,
    _body: &StmtSeq,
    source: &str,
    stmt_spans: &[Span],
) -> bool {
    let Some(_) = stmt_plain_assignment_only_name(first_stmt) else {
        return false;
    };
    if stmt_contains_unquoted_standalone_status_capture(first_stmt, source) || stmt_spans.is_empty()
    {
        return false;
    }
    true
}

fn branch_body_logs_status_then_immediately_exits(
    first_stmt: &Stmt,
    body: &StmtSeq,
    source: &str,
    stmt_spans: &[Span],
) -> bool {
    if stmt_spans.is_empty()
        || stmt_contains_unquoted_standalone_status_capture(first_stmt, source)
    {
        return false;
    }

    body.iter().nth(1).is_some_and(stmt_is_exit_or_return_builtin)
}

fn collect_precise_function_return_guard_suppressions_in_seq(
    commands: &StmtSeq,
    source: &str,
    spans: &mut Vec<Span>,
    in_function_like_body: bool,
) {
    let stmts = commands.as_slice();
    let body_shape = BodyShapeAnalyzer::new(source);
    for (index, stmt) in stmts.iter().enumerate() {
        if in_function_like_body && stmt_is_test_return_status_guard(stmt, source) {
            let previous_non_test_guard =
                index > 0 && stmt_is_non_test_return_status_guard(&stmts[index - 1], source);
            let later_unary_guard = stmts[index + 1..]
                .iter()
                .any(|stmt| stmt_is_unary_test_return_status_guard(stmt, source));
            let status_accumulator_guard =
                body_shape.stmt_starts_status_accumulator_return_guard(index, stmts);
            if status_accumulator_guard
                || (stmt_is_unary_test_return_status_guard(stmt, source)
                    && (previous_non_test_guard || later_unary_guard))
            {
                collect_status_parameter_spans_in_return_guard_stmt(stmt, source, spans);
            }
        }
    }
}

fn collect_precise_function_return_guard_suppressions_from_direct_body_sequences(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
    in_function_like_body: bool,
) {
    match command {
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.then_branch,
                    source,
                    spans,
                    in_function_like_body,
                );
                for (_, branch) in &command.elif_branches {
                    collect_precise_function_return_guard_suppressions_in_seq(
                        branch,
                        source,
                        spans,
                        in_function_like_body,
                    );
                }
                if let Some(branch) = &command.else_branch {
                    collect_precise_function_return_guard_suppressions_in_seq(
                        branch,
                        source,
                        spans,
                        in_function_like_body,
                    );
                }
            }
            CompoundCommand::While(command) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.body,
                    source,
                    spans,
                    in_function_like_body,
                );
            }
            CompoundCommand::Until(command) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.body,
                    source,
                    spans,
                    in_function_like_body,
                );
            }
            CompoundCommand::For(command) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.body,
                    source,
                    spans,
                    in_function_like_body,
                );
            }
            CompoundCommand::Select(command) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.body,
                    source,
                    spans,
                    in_function_like_body,
                );
            }
            CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    body,
                    source,
                    spans,
                    in_function_like_body,
                );
            }
            CompoundCommand::Time(_) => {}
            CompoundCommand::Always(command) => {
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.body,
                    source,
                    spans,
                    in_function_like_body,
                );
                collect_precise_function_return_guard_suppressions_in_seq(
                    &command.always_body,
                    source,
                    spans,
                    in_function_like_body,
                );
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    collect_precise_function_return_guard_suppressions_in_seq(
                        &case.body,
                        source,
                        spans,
                        in_function_like_body,
                    );
                }
            }
            CompoundCommand::Conditional(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Coproc(_) => {}
        },
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

fn condition_terminals_are_test_commands(condition: &StmtSeq, source: &str) -> bool {
    condition
        .last()
        .is_some_and(|stmt| stmt_terminals_are_test_commands(stmt, source))
}

fn stmt_is_standalone_non_status_test_command(stmt: &Stmt, source: &str) -> bool {
    stmt_terminals_are_test_commands(stmt, source) && !stmt_contains_status_capture(stmt, source)
}

fn stmt_is_status_based_test_command(stmt: &Stmt, source: &str) -> bool {
    stmt_terminals_are_test_commands(stmt, source) && stmt_contains_status_capture(stmt, source)
}

fn stmt_is_unary_test_return_status_guard(stmt: &Stmt, source: &str) -> bool {
    let Command::Binary(command) = &stmt.command else {
        return false;
    };
    matches!(command.op, BinaryOp::Or)
        && stmt_is_simple_unary_test_command(&command.left, source)
        && stmt_is_return_status_command(&command.right, source)
}

fn stmt_is_test_return_status_guard(stmt: &Stmt, source: &str) -> bool {
    let Command::Binary(command) = &stmt.command else {
        return false;
    };
    matches!(command.op, BinaryOp::Or)
        && stmt_terminals_are_test_commands(&command.left, source)
        && stmt_is_return_status_command(&command.right, source)
}

fn stmt_is_non_test_return_status_guard(stmt: &Stmt, source: &str) -> bool {
    let Command::Binary(command) = &stmt.command else {
        return false;
    };
    matches!(command.op, BinaryOp::Or)
        && !stmt_terminals_are_test_commands(&command.left, source)
        && stmt_is_return_status_command(&command.right, source)
}

fn stmt_is_return_status_command(stmt: &Stmt, source: &str) -> bool {
    if stmt.negated || !stmt.redirects.is_empty() {
        return false;
    }

    match &stmt.command {
        Command::Builtin(BuiltinCommand::Return(command)) => command
            .code
            .as_ref()
            .is_some_and(word_is_unquoted_standalone_status_capture),
        Command::Simple(command) => {
            static_word_text(&command.name, source).as_deref() == Some("return")
                && command.args.len() == 1
                && word_is_unquoted_standalone_status_capture(&command.args[0])
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn stmt_is_simple_unary_test_command(stmt: &Stmt, source: &str) -> bool {
    if stmt.negated {
        return false;
    }

    match &stmt.command {
        Command::Compound(CompoundCommand::Conditional(command)) => {
            matches!(command.expression, ConditionalExpr::Unary(_))
        }
        Command::Simple(command) => {
            let Some(name) = static_word_text(&command.name, source) else {
                return false;
            };
            if !matches!(name.as_ref(), "[" | "test") {
                return false;
            }
            let Some((closing_bracket, operands)) = command.args.split_last() else {
                return false;
            };
            let operand_count = if name.as_ref() == "[" {
                if static_word_text(closing_bracket, source).as_deref() != Some("]") {
                    return false;
                }
                operands.len()
            } else {
                command.args.len()
            };
            operand_count == 2
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn stmt_assignment_only_scalar_literal_name<'a>(
    stmt: &'a Stmt,
    source: &str,
    expected: &str,
) -> Option<&'a Name> {
    let name = stmt_plain_assignment_only_name(stmt)?;
    let Command::Simple(command) = &stmt.command else {
        return None;
    };
    let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
        return None;
    };
    (static_word_text(word, source).as_deref() == Some(expected)).then_some(name)
}

fn word_is_name_reference(word: &Word, name: &Name) -> bool {
    matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Variable(var),
            ..
        }] if var == name
    ) || matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Parameter(parameter),
            ..
        }] if matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if reference.name == *name && reference.subscript.is_none()
        )
    )
}

fn stmt_contains_status_capture(stmt: &Stmt, source: &str) -> bool {
    let mut spans = Vec::new();
    collect_status_parameter_spans_in_stmt(stmt, source, &mut spans);
    !spans.is_empty()
}

fn stmt_contains_unquoted_standalone_status_capture(stmt: &Stmt, source: &str) -> bool {
    let mut spans = Vec::new();
    collect_unquoted_standalone_status_parameter_spans_in_stmt(stmt, source, &mut spans);
    !spans.is_empty()
}

fn stmt_starts_sequence_barrier(stmt: &Stmt) -> bool {
    matches!(
        &stmt.command,
        Command::Compound(
            CompoundCommand::If(_)
                | CompoundCommand::Case(_)
                | CompoundCommand::For(_)
                | CompoundCommand::Select(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::Always(_)
        )
    )
}

fn stmt_is_named_simple_command(stmt: &Stmt, source: &str, name: &str) -> bool {
    matches!(
        &stmt.command,
        Command::Simple(command)
            if static_word_text(&command.name, source).as_deref() == Some(name)
    )
}

fn stmt_plain_assignment_only_name(stmt: &Stmt) -> Option<&Name> {
    if stmt.negated || !stmt.redirects.is_empty() {
        return None;
    }

    let Command::Simple(command) = &stmt.command else {
        return None;
    };
    if !command.args.is_empty()
        || command.name.span.start.offset != command.name.span.end.offset
        || command.assignments.len() != 1
    {
        return None;
    }

    Some(&command.assignments[0].target.name)
}

fn stmt_is_assignment_only_unquoted_status_capture(stmt: &Stmt) -> bool {
    if stmt.negated || !stmt.redirects.is_empty() {
        return false;
    }

    let Command::Simple(command) = &stmt.command else {
        return false;
    };

    command.args.is_empty()
        && command.name.span.start.offset == command.name.span.end.offset
        && !command.assignments.is_empty()
        && command
            .assignments
            .iter()
            .all(|assignment| assignment_value_is_unquoted_status_capture(&assignment.value))
}

fn assignment_value_is_unquoted_status_capture(value: &AssignmentValue) -> bool {
    match value {
        AssignmentValue::Scalar(word) => word_is_unquoted_standalone_status_capture(word),
        AssignmentValue::Compound(_) => false,
    }
}

fn stmt_is_exit_or_return_builtin(stmt: &Stmt) -> bool {
    matches!(
        stmt.command,
        Command::Builtin(BuiltinCommand::Exit(_) | BuiltinCommand::Return(_))
    )
}

fn stmt_is_plain_command_with_standalone_status_argument(stmt: &Stmt) -> bool {
    if stmt.negated || !stmt.redirects.is_empty() {
        return false;
    }

    match &stmt.command {
        Command::Simple(command) => {
            command.name.span.start.offset != command.name.span.end.offset
                && (word_is_unquoted_standalone_status_capture(&command.name)
                    || command
                        .args
                        .iter()
                        .any(word_is_unquoted_standalone_status_capture))
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Return(command) => command
                .code
                .as_ref()
                .is_some_and(word_is_unquoted_standalone_status_capture),
            BuiltinCommand::Exit(command) => command
                .code
                .as_ref()
                .is_some_and(word_is_unquoted_standalone_status_capture),
            BuiltinCommand::Break(_)
            | BuiltinCommand::Continue(_) => false,
        },
        Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn word_is_unquoted_standalone_status_capture(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Variable(name),
            ..
        }] if name.as_str() == "?"
    ) || matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::Parameter(parameter),
            ..
        }] if matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if reference.name.as_str() == "?" && reference.subscript.is_none()
        )
    )
}

fn stmt_terminals_are_test_commands(stmt: &Stmt, source: &str) -> bool {
    if stmt.negated {
        return false;
    }

    command_terminals_are_test_commands(&stmt.command, source)
}

fn command_terminals_are_test_commands(command: &Command, source: &str) -> bool {
    match command {
        Command::Simple(command) => matches!(
            static_word_text(&command.name, source).as_deref(),
            Some("[") | Some("test")
        ),
        Command::Compound(CompoundCommand::Conditional(_)) => true,
        Command::Binary(command) => {
            let Some(chain) = BinaryCommandChain::logical_list(command) else {
                return false;
            };
            let mut all_segments_are_tests = true;
            chain.visit_segments(|stmt| {
                all_segments_are_tests &= stmt_terminals_are_test_commands(stmt, source);
            });
            all_segments_are_tests
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn collect_status_parameter_spans_in_stmt(stmt: &Stmt, source: &str, spans: &mut Vec<Span>) {
    collect_status_parameter_spans_in_command(&stmt.command, source, spans);
    for redirect in &stmt.redirects {
        if let Some(word) = redirect.word_target() {
            collect_status_parameter_spans_in_word(word, source, spans);
        }
    }
}

fn collect_status_parameter_spans_in_return_guard_stmt(
    stmt: &Stmt,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Command::Binary(command) = &stmt.command {
        collect_status_parameter_spans_in_stmt(&command.right, source, spans);
        return;
    }

    collect_status_parameter_spans_in_stmt(stmt, source, spans);
}

fn collect_unquoted_standalone_status_parameter_spans_in_stmt(
    stmt: &Stmt,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_standalone_status_parameter_spans_in_command(&stmt.command, source, spans);
    for redirect in &stmt.redirects {
        if let Some(word) = redirect.word_target() {
            collect_unquoted_standalone_status_parameter_spans_in_word(word, spans);
        }
    }
}

fn collect_unquoted_standalone_status_parameter_spans_in_command(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                if let AssignmentValue::Scalar(word) = &assignment.value {
                    collect_unquoted_standalone_status_parameter_spans_in_word(word, spans);
                }
            }
            collect_unquoted_standalone_status_parameter_spans_in_word(&command.name, spans);
            for word in &command.args {
                collect_unquoted_standalone_status_parameter_spans_in_word(word, spans);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    collect_unquoted_standalone_status_parameter_spans_in_word(word, spans);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    collect_unquoted_standalone_status_parameter_spans_in_word(word, spans);
                }
            }
            BuiltinCommand::Break(_)
            | BuiltinCommand::Continue(_) => {}
        },
        Command::Decl(command) => {
            collect_unquoted_standalone_status_parameter_spans_in_assignments(
                &command.assignments,
                spans,
            );
            for operand in &command.operands {
                if let DeclOperand::Assignment(assignment) = operand {
                    collect_unquoted_standalone_status_parameter_spans_in_assignment(
                        assignment, spans,
                    );
                }
            }
        }
        Command::Compound(command) => match command {
            CompoundCommand::Case(command) => {
                if let Some(first_stmt) = command
                    .cases
                    .iter()
                    .find_map(|case| case.body.first())
                {
                    collect_unquoted_standalone_status_parameter_spans_in_stmt(
                        first_stmt,
                        source,
                        spans,
                    );
                }
            }
            CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => {
                if let Some(first_stmt) = body.first() {
                    collect_unquoted_standalone_status_parameter_spans_in_stmt(
                        first_stmt,
                        source,
                        spans,
                    );
                }
            }
            CompoundCommand::Time(command) => {
                if let Some(inner) = &command.command {
                    collect_unquoted_standalone_status_parameter_spans_in_stmt(inner, source, spans);
                }
            }
            CompoundCommand::If(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::For(_)
            | CompoundCommand::Select(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Always(_) => {}
        },
        Command::Binary(command) => {
            collect_unquoted_standalone_status_parameter_spans_in_stmt(&command.left, source, spans);
        }
        Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

fn collect_unquoted_standalone_status_parameter_spans_in_assignments(
    assignments: &[Assignment],
    spans: &mut Vec<Span>,
) {
    for assignment in assignments {
        collect_unquoted_standalone_status_parameter_spans_in_assignment(assignment, spans);
    }
}

fn collect_unquoted_standalone_status_parameter_spans_in_assignment(
    assignment: &Assignment,
    spans: &mut Vec<Span>,
) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            collect_unquoted_standalone_status_parameter_spans_in_word(word, spans);
        }
        AssignmentValue::Compound(_) => {}
    }
}

fn collect_unquoted_standalone_status_parameter_spans_in_word(word: &Word, spans: &mut Vec<Span>) {
    if word_is_unquoted_standalone_status_capture(word) {
        spans.push(word.span);
    }
}

fn collect_status_parameter_spans_in_command(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            collect_status_parameter_spans_in_assignments(&command.assignments, source, spans);
            collect_status_parameter_spans_in_word(&command.name, source, spans);
            for word in &command.args {
                collect_status_parameter_spans_in_word(word, source, spans);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                collect_status_parameter_spans_in_assignments(&command.assignments, source, spans);
                if let Some(word) = &command.depth {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Continue(command) => {
                collect_status_parameter_spans_in_assignments(&command.assignments, source, spans);
                if let Some(word) = &command.depth {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Return(command) => {
                collect_status_parameter_spans_in_assignments(&command.assignments, source, spans);
                if let Some(word) = &command.code {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Exit(command) => {
                collect_status_parameter_spans_in_assignments(&command.assignments, source, spans);
                if let Some(word) = &command.code {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
            }
        },
        Command::Decl(command) => {
            collect_status_parameter_spans_in_assignments(&command.assignments, source, spans);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    }
                    DeclOperand::Name(reference) => {
                        collect_status_parameter_spans_in_var_ref(reference, source, spans);
                    }
                    DeclOperand::Assignment(assignment) => {
                        collect_status_parameter_spans_in_assignment(assignment, source, spans);
                    }
                }
            }
        }
        Command::Binary(command) => {
            collect_status_parameter_spans_in_stmt(&command.left, source, spans);
        }
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                if let Some(first_stmt) = command.condition.first() {
                    collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
                }
            }
            CompoundCommand::While(command) => {
                if let Some(first_stmt) = command.condition.first() {
                    collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
                }
            }
            CompoundCommand::Until(command) => {
                if let Some(first_stmt) = command.condition.first() {
                    collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
                }
            }
            CompoundCommand::Case(command) => {
                collect_status_parameter_spans_in_word(&command.word, source, spans);
                for case in &command.cases {
                    if let Some(first_stmt) = case.body.first() {
                        collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
                    }
                }
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                if let Some(first_stmt) = body.first() {
                    collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
                }
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    collect_status_parameter_spans_in_stmt(command, source, spans);
                }
            }
            CompoundCommand::Conditional(command) => {
                collect_status_parameter_spans_in_conditional_expr(
                    &command.expression,
                    source,
                    spans,
                );
            }
            CompoundCommand::Coproc(command) => {
                collect_status_parameter_spans_in_stmt(&command.body, source, spans);
            }
            CompoundCommand::Always(command) => {
                if let Some(first_stmt) = command.body.first() {
                    collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
                }
            }
            CompoundCommand::For(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::Select(_)
            | CompoundCommand::Arithmetic(_) => {}
        },
        Command::Function(_) => {}
        Command::AnonymousFunction(command) => {
            collect_status_parameter_spans_in_stmt(&command.body, source, spans);
            for word in &command.args {
                collect_status_parameter_spans_in_word(word, source, spans);
            }
        }
    }
}

fn collect_status_parameter_spans_in_assignments(
    assignments: &[Assignment],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for assignment in assignments {
        collect_status_parameter_spans_in_assignment(assignment, source, spans);
    }
}

fn collect_status_parameter_spans_in_assignment(
    assignment: &Assignment,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_status_parameter_spans_in_var_ref(&assignment.target, source, spans);
    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            collect_status_parameter_spans_in_word(word, source, spans)
        }
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        visit_subscript_words(Some(key), source, &mut |word| {
                            collect_status_parameter_spans_in_word(word, source, spans);
                        });
                        collect_status_parameter_spans_in_word(value, source, spans);
                    }
                }
            }
        }
    }
}

fn collect_status_parameter_spans_in_var_ref(
    reference: &VarRef,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if reference.name.as_str() == "?" {
        spans.push(reference.span);
    }

    visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_status_parameter_spans_in_word(word, source, spans);
    });
}

fn collect_status_parameter_spans_in_word(word: &Word, source: &str, spans: &mut Vec<Span>) {
    for part in &word.parts {
        collect_status_parameter_spans_in_word_part(part, source, spans);
    }
}

fn collect_status_parameter_spans_in_word_part(
    part: &WordPartNode,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } | WordPart::ZshQualifiedGlob(_) => {}
        WordPart::DoubleQuoted { parts, .. } => {
            for nested_part in parts {
                collect_status_parameter_spans_in_word_part(nested_part, source, spans);
            }
        }
        WordPart::Variable(name) => {
            if name.as_str() == "?" {
                spans.push(part.span);
            }
        }
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            if let Some(first_stmt) = body.first() {
                collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
            }
        }
        WordPart::ArithmeticExpansion {
            expression_ast,
            expression_word_ast,
            ..
        } => {
            if let Some(expression) = expression_ast {
                visit_arithmetic_words(expression, &mut |word| {
                    collect_status_parameter_spans_in_word(word, source, spans);
                });
            } else {
                collect_status_parameter_spans_in_word(expression_word_ast, source, spans);
            }
        }
        WordPart::Parameter(parameter) => {
            collect_status_parameter_spans_in_parameter_expansion(parameter, source, spans);
        }
        WordPart::ParameterExpansion {
            reference,
            operand,
            operand_word_ast,
            ..
        }
        | WordPart::IndirectExpansion {
            reference,
            operand,
            operand_word_ast,
            ..
        } => {
            if reference.name.as_str() == "?" {
                spans.push(part.span);
            }
            collect_status_parameter_spans_in_var_ref(reference, source, spans);
            collect_status_parameter_spans_in_fragment(
                operand_word_ast.as_ref(),
                operand.as_ref(),
                source,
                spans,
            );
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            if reference.name.as_str() == "?" {
                spans.push(part.span);
            }
            collect_status_parameter_spans_in_var_ref(reference, source, spans);
        }
        WordPart::Substring {
            reference,
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset_ast,
            offset_word_ast,
            length_ast,
            length_word_ast,
            ..
        } => {
            if reference.name.as_str() == "?" {
                spans.push(part.span);
            }
            collect_status_parameter_spans_in_var_ref(reference, source, spans);
            if let Some(offset_ast) = offset_ast {
                visit_arithmetic_words(offset_ast, &mut |word| {
                    collect_status_parameter_spans_in_word(word, source, spans);
                });
            } else {
                collect_status_parameter_spans_in_word(offset_word_ast, source, spans);
            }
            match (length_ast.as_ref(), length_word_ast.as_ref()) {
                (Some(length_ast), _) => {
                    visit_arithmetic_words(length_ast, &mut |word| {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    });
                }
                (None, Some(length_word_ast)) => {
                    collect_status_parameter_spans_in_word(length_word_ast, source, spans);
                }
                (None, None) => {}
            }
        }
        WordPart::PrefixMatch { .. } => {}
    }
}

fn collect_status_parameter_spans_in_parameter_expansion(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_status_parameter_spans_in_var_ref(reference, source, spans);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansion::Operation {
                reference,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_status_parameter_spans_in_var_ref(reference, source, spans);
                collect_status_parameter_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    spans,
                );
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_status_parameter_spans_in_var_ref(reference, source, spans);
                if let Some(offset_ast) = offset_ast {
                    visit_arithmetic_words(offset_ast, &mut |word| {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    });
                } else {
                    collect_status_parameter_spans_in_word(offset_word_ast, source, spans);
                }

                match (length_ast.as_ref(), length_word_ast.as_ref()) {
                    (Some(length_ast), _) => {
                        visit_arithmetic_words(length_ast, &mut |word| {
                            collect_status_parameter_spans_in_word(word, source, spans);
                        });
                    }
                    (None, Some(length_word_ast)) => {
                        collect_status_parameter_spans_in_word(length_word_ast, source, spans);
                    }
                    (None, None) => {}
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_status_parameter_spans_in_zsh_target(&syntax.target, source, spans);

            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::PatternOperation { operand, .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { operand, .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { operand, .. } => {
                        collect_status_parameter_spans_in_fragment(
                            operation.operand_word_ast(),
                            Some(operand),
                            source,
                            spans,
                        );
                    }
                    shuck_ast::ZshExpansionOperation::ReplacementOperation {
                        pattern,
                        replacement,
                        ..
                    } => {
                        collect_status_parameter_spans_in_fragment(
                            operation.pattern_word_ast(),
                            Some(pattern),
                            source,
                            spans,
                        );
                        collect_status_parameter_spans_in_fragment(
                            operation.replacement_word_ast(),
                            replacement.as_ref(),
                            source,
                            spans,
                        );
                    }
                    shuck_ast::ZshExpansionOperation::Slice { offset, length, .. } => {
                        collect_status_parameter_spans_in_fragment(
                            operation.offset_word_ast(),
                            Some(offset),
                            source,
                            spans,
                        );
                        collect_status_parameter_spans_in_fragment(
                            operation.length_word_ast(),
                            length.as_ref(),
                            source,
                            spans,
                        );
                    }
                    shuck_ast::ZshExpansionOperation::Unknown { text, .. } => {
                        collect_status_parameter_spans_in_fragment(
                            operation.operand_word_ast(),
                            Some(text),
                            source,
                            spans,
                        );
                    }
                }
            }
        }
    }
}

fn collect_status_parameter_spans_in_zsh_target(
    target: &ZshExpansionTarget,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match target {
        ZshExpansionTarget::Reference(reference) => {
            collect_status_parameter_spans_in_var_ref(reference, source, spans);
        }
        ZshExpansionTarget::Nested(parameter) => {
            collect_status_parameter_spans_in_parameter_expansion(parameter, source, spans);
        }
        ZshExpansionTarget::Word(word) => {
            collect_status_parameter_spans_in_word(word, source, spans);
        }
        ZshExpansionTarget::Empty => {}
    }
}

fn collect_status_parameter_spans_in_conditional_expr(
    expression: &ConditionalExpr,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Binary(expression) => {
            collect_status_parameter_spans_in_conditional_expr(&expression.left, source, spans);
            collect_status_parameter_spans_in_conditional_expr(&expression.right, source, spans);
        }
        ConditionalExpr::Unary(expression) => {
            collect_status_parameter_spans_in_conditional_expr(&expression.expr, source, spans);
        }
        ConditionalExpr::Parenthesized(expression) => {
            collect_status_parameter_spans_in_conditional_expr(&expression.expr, source, spans);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_status_parameter_spans_in_word(word, source, spans);
        }
        ConditionalExpr::Pattern(pattern) => {
            for part in &pattern.parts {
                if let PatternPart::Word(word) = &part.kind {
                    collect_status_parameter_spans_in_word(word, source, spans);
                }
            }
        }
        ConditionalExpr::VarRef(reference) => {
            collect_status_parameter_spans_in_var_ref(reference, source, spans);
        }
    }
}

fn collect_status_parameter_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(text) = text else {
        return;
    };
    let snippet = text.slice(source);
    if !snippet.contains("$?") {
        return;
    }
    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        return;
    };
    collect_status_parameter_spans_in_word(word, source, spans);
}

fn build_conditional_fact<'a>(
    expression_visits: &[ConditionalExpressionVisit<'a>],
    source: &str,
) -> Option<ConditionalFact<'a>> {
    if expression_visits.is_empty() {
        return None;
    }

    let mut nodes = Vec::with_capacity(expression_visits.len());
    let mut mixed_logical_operators = Vec::new();
    for visit in expression_visits {
        nodes.push(build_conditional_node(visit.expression, source));
        collect_mixed_logical_operator(
            visit.expression,
            visit.parent_in_same_logical_group,
            &mut mixed_logical_operators,
        );
    }

    (!nodes.is_empty()).then_some(ConditionalFact {
        nodes: nodes.into_boxed_slice(),
        mixed_logical_operators: mixed_logical_operators.into_boxed_slice(),
    })
}

fn command_name_is_plain_command_substitution(word: &Word, source: &str) -> bool {
    let analysis = analyze_word(word, source, None);
    analysis.substitution_shape == WordSubstitutionShape::Plain
        && analysis.quote == WordQuote::Unquoted
        && matches!(
            word.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::CommandSubstitution {
                    syntax: CommandSubstitutionSyntax::DollarParen,
                    ..
                },
                ..
            }]
        )
}

fn collect_mixed_logical_operator(
    expression: &ConditionalExpr,
    parent_in_same_logical_group: bool,
    operators: &mut Vec<ConditionalMixedLogicalOperatorFact>,
) {
    let ConditionalExpr::Binary(binary) = expression else {
        return;
    };

    if conditional_binary_op_is_logical(binary.op)
        && !parent_in_same_logical_group
        && logical_operator_mask(expression) == (LOGICAL_AND_MASK | LOGICAL_OR_MASK)
    {
        let grouped_subexpression_spans =
            mixed_logical_grouped_subexpression_spans(expression, binary.op);
        debug_assert!(
            !grouped_subexpression_spans.is_empty(),
            "mixed logical operators should expose at least one grouping span"
        );
        operators.push(ConditionalMixedLogicalOperatorFact {
            operator_span: binary.op_span,
            grouped_subexpression_spans: grouped_subexpression_spans.into_boxed_slice(),
        });
    }
}

const LOGICAL_AND_MASK: u8 = 0b01;
const LOGICAL_OR_MASK: u8 = 0b10;

fn mixed_logical_grouped_subexpression_spans(
    expression: &ConditionalExpr,
    group_op: ConditionalBinaryOp,
) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_mixed_logical_grouped_subexpression_spans(expression, group_op, &mut spans);
    spans
}

fn collect_mixed_logical_grouped_subexpression_spans(
    expression: &ConditionalExpr,
    group_op: ConditionalBinaryOp,
    spans: &mut Vec<Span>,
) {
    let ConditionalExpr::Binary(binary) = expression else {
        return;
    };
    if !conditional_binary_op_is_logical(binary.op) {
        return;
    }

    if binary.op != group_op {
        spans.push(expression.span());
        return;
    }

    collect_mixed_logical_grouped_subexpression_spans(&binary.left, group_op, spans);
    collect_mixed_logical_grouped_subexpression_spans(&binary.right, group_op, spans);
}

fn logical_operator_mask(expression: &ConditionalExpr) -> u8 {
    match expression {
        ConditionalExpr::Parenthesized(_) => 0,
        ConditionalExpr::Unary(unary) => logical_operator_mask(&unary.expr),
        ConditionalExpr::Binary(binary) => {
            let own = match binary.op {
                ConditionalBinaryOp::And => LOGICAL_AND_MASK,
                ConditionalBinaryOp::Or => LOGICAL_OR_MASK,
                _ => 0,
            };

            own | logical_operator_mask(&binary.left) | logical_operator_mask(&binary.right)
        }
        ConditionalExpr::Word(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => 0,
    }
}

fn conditional_binary_op_is_logical(operator: ConditionalBinaryOp) -> bool {
    matches!(operator, ConditionalBinaryOp::And | ConditionalBinaryOp::Or)
}

fn build_conditional_node<'a>(
    expression: &'a ConditionalExpr,
    source: &str,
) -> ConditionalNodeFact<'a> {
    match expression {
        ConditionalExpr::Word(_) => ConditionalNodeFact::BareWord(ConditionalBareWordFact {
            expression,
            operand: build_conditional_operand_fact(expression, source),
        }),
        ConditionalExpr::Unary(unary) => ConditionalNodeFact::Unary(ConditionalUnaryFact {
            expression,
            op: unary.op,
            operator_family: conditional_unary_operator_family(unary.op),
            operand: build_conditional_operand_fact(&unary.expr, source),
        }),
        ConditionalExpr::Binary(binary) => ConditionalNodeFact::Binary(ConditionalBinaryFact {
            expression,
            op: binary.op,
            operator_family: conditional_binary_operator_family(binary.op),
            left: build_conditional_operand_fact(&binary.left, source),
            right: build_conditional_operand_fact(&binary.right, source),
        }),
        ConditionalExpr::Parenthesized(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => ConditionalNodeFact::Other(expression),
    }
}

fn build_conditional_operand_fact<'a>(
    expression: &'a ConditionalExpr,
    source: &str,
) -> ConditionalOperandFact<'a> {
    let expression = strip_parenthesized_conditionals(expression);
    let word = match expression {
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => Some(word),
        ConditionalExpr::Pattern(pattern) => conditional_pattern_single_word(pattern),
        ConditionalExpr::Binary(_)
        | ConditionalExpr::Unary(_)
        | ConditionalExpr::Parenthesized(_)
        | ConditionalExpr::VarRef(_) => None,
    };

    ConditionalOperandFact {
        expression,
        class: classify_conditional_operand(expression, source),
        word,
        word_classification: word.map(|word| classify_word(word, source)),
    }
}

fn conditional_pattern_single_word(pattern: &Pattern) -> Option<&Word> {
    match pattern.parts.as_slice() {
        [part] => match &part.kind {
            PatternPart::Word(word) => Some(word),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_)
            | PatternPart::Group { .. } => None,
        },
        _ => None,
    }
}

fn strip_parenthesized_conditionals(mut expression: &ConditionalExpr) -> &ConditionalExpr {
    while let ConditionalExpr::Parenthesized(parenthesized) = expression {
        expression = &parenthesized.expr;
    }

    expression
}

fn conditional_unary_operator_family(operator: ConditionalUnaryOp) -> ConditionalOperatorFamily {
    if matches!(
        operator,
        ConditionalUnaryOp::EmptyString | ConditionalUnaryOp::NonEmptyString
    ) {
        ConditionalOperatorFamily::StringUnary
    } else {
        ConditionalOperatorFamily::Other
    }
}

fn conditional_binary_operator_family(operator: ConditionalBinaryOp) -> ConditionalOperatorFamily {
    match operator {
        ConditionalBinaryOp::RegexMatch => ConditionalOperatorFamily::Regex,
        ConditionalBinaryOp::And | ConditionalBinaryOp::Or => ConditionalOperatorFamily::Logical,
        ConditionalBinaryOp::PatternEqShort
        | ConditionalBinaryOp::PatternEq
        | ConditionalBinaryOp::PatternNe
        | ConditionalBinaryOp::LexicalBefore
        | ConditionalBinaryOp::LexicalAfter => ConditionalOperatorFamily::StringBinary,
        ConditionalBinaryOp::NewerThan
        | ConditionalBinaryOp::OlderThan
        | ConditionalBinaryOp::SameFile
        | ConditionalBinaryOp::ArithmeticEq
        | ConditionalBinaryOp::ArithmeticNe
        | ConditionalBinaryOp::ArithmeticLe
        | ConditionalBinaryOp::ArithmeticGe
        | ConditionalBinaryOp::ArithmeticLt
        | ConditionalBinaryOp::ArithmeticGt => ConditionalOperatorFamily::Other,
    }
}
