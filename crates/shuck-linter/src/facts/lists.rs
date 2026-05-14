use super::*;

#[derive(Debug, Clone, Copy)]
pub struct ListOperatorFact {
    pub(crate) op: BinaryOp,
    pub(crate) span: Span,
}

impl ListOperatorFact {
    pub fn op(&self) -> BinaryOp {
        self.op
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSegmentKind {
    Condition,
    AssignmentOnly,
    Other,
}

#[derive(Debug, Clone)]
pub struct ListSegmentFact<'a> {
    command_id: CommandId,
    span: Span,
    kind: ListSegmentKind,
    assignment_target: Option<&'a str>,
    assignment_span: Option<Span>,
    assignment_is_declaration: bool,
}

impl<'a> ListSegmentFact<'a> {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> ListSegmentKind {
        self.kind
    }

    pub fn assignment_target(&self) -> Option<&'a str> {
        self.assignment_target
    }

    pub fn assignment_span(&self) -> Option<Span> {
        self.assignment_span
    }

    pub fn assignment_is_declaration(&self) -> bool {
        self.assignment_is_declaration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixedShortCircuitKind {
    TestChain,
    AssignmentTernary,
    Fallthrough,
}

#[derive(Debug, Clone)]
pub struct ListFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    operators: Box<[ListOperatorFact]>,
    segments: Box<[ListSegmentFact<'a>]>,
    mixed_short_circuit_span: Option<Span>,
    mixed_short_circuit_kind: Option<MixedShortCircuitKind>,
}

impl<'a> ListFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn command(&self) -> &'a BinaryCommand {
        self.command
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn operators(&self) -> &[ListOperatorFact] {
        &self.operators
    }

    pub fn segments(&self) -> &[ListSegmentFact<'a>] {
        &self.segments
    }

    pub fn mixed_short_circuit_span(&self) -> Option<Span> {
        self.mixed_short_circuit_span
    }

    pub fn mixed_short_circuit_kind(&self) -> Option<MixedShortCircuitKind> {
        self.mixed_short_circuit_kind
    }
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_list_facts<'a>(
    commands: &[CommandFact<'a>],
    command_fact_indices_by_id: &[Option<usize>],
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    source: &str,
) -> Vec<ListFact<'a>> {
    let command_relationships = CommandRelationshipContext::new(
        commands,
        command_fact_indices_by_id,
        command_ids_by_span,
        command_child_index,
    );
    let mut nested_list_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if BinaryCommandChain::logical_list(command).is_none() {
            continue;
        }

        record_nested_list_command(
            &command.left,
            fact.id(),
            command_relationships,
            &mut nested_list_commands,
        );
        record_nested_list_command(
            &command.right,
            fact.id(),
            command_relationships,
            &mut nested_list_commands,
        );
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if BinaryCommandChain::logical_list(command).is_none()
                || nested_list_commands.contains(&fact.id())
            {
                return None;
            }

            let mut operators = Vec::new();
            collect_short_circuit_operators(command, &mut operators);
            let segments =
                build_list_segment_facts(command, command_relationships, fact.id(), source)?;
            let mixed_short_circuit_span = mixed_short_circuit_operator_span(&operators);
            let mixed_short_circuit_kind = mixed_short_circuit_span
                .map(|_| classify_mixed_short_circuit_kind(&segments, &operators));

            Some(ListFact {
                key: fact.key(),
                command,
                operators: operators.into_boxed_slice(),
                segments,
                mixed_short_circuit_span,
                mixed_short_circuit_kind,
            })
        })
        .collect()
}

pub(crate) fn record_nested_list_command(
    stmt: &Stmt,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, '_>,
    nested_list_commands: &mut FxHashSet<CommandId>,
) {
    let Command::Binary(child) = &stmt.command else {
        return;
    };
    if BinaryCommandChain::logical_list(child).is_some()
        && let Some(child) = command_relationships.child_or_lookup_fact(parent_id, stmt)
    {
        nested_list_commands.insert(child.id());
    }
}

pub(crate) fn build_list_segment_facts<'a>(
    command: &BinaryCommand,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
) -> Option<Box<[ListSegmentFact<'a>]>> {
    let mut segments = Vec::new();
    collect_list_segment_facts(
        command,
        command_relationships,
        parent_id,
        source,
        &mut segments,
    )?;
    Some(segments.into_boxed_slice())
}

pub(crate) fn collect_list_segment_facts<'a>(
    command: &BinaryCommand,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
    segments: &mut Vec<ListSegmentFact<'a>>,
) -> Option<()> {
    let chain = BinaryCommandChain::logical_list(command)?;
    let mut ok = true;
    chain.visit_segments(|stmt| {
        if ok {
            ok = push_list_stmt_segment_fact(
                stmt,
                command_relationships,
                parent_id,
                source,
                segments,
            )
            .is_some();
        }
    });
    ok.then_some(())
}

pub(crate) fn push_list_stmt_segment_fact<'a>(
    stmt: &Stmt,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
    segments: &mut Vec<ListSegmentFact<'a>>,
) -> Option<()> {
    let fact = command_relationships.child_or_lookup_fact(parent_id, stmt)?;
    let id = fact.id();
    let assignment_info = list_segment_assignment_info(fact);
    let assignment_target = assignment_info.as_ref().map(|info| info.target);
    let assignment_is_declaration = assignment_info
        .as_ref()
        .is_some_and(|info| info.is_declaration);

    segments.push(ListSegmentFact {
        command_id: id,
        span: fact.span_in_source(source),
        kind: list_segment_kind(fact),
        assignment_target,
        assignment_span: assignment_info.map(|info| info.span),
        assignment_is_declaration,
    });
    Some(())
}

pub(crate) fn list_segment_kind(fact: &CommandFact<'_>) -> ListSegmentKind {
    if list_segment_is_condition(fact) {
        ListSegmentKind::Condition
    } else if list_segment_assignment_target(fact).is_some() {
        ListSegmentKind::AssignmentOnly
    } else {
        ListSegmentKind::Other
    }
}

pub(crate) fn list_segment_is_condition(fact: &CommandFact<'_>) -> bool {
    fact.simple_test().is_some()
        || fact.conditional().is_some()
        || matches!(fact.effective_or_literal_name(), Some("true" | "false"))
}

pub(crate) fn list_segment_assignment_target<'a>(fact: &CommandFact<'a>) -> Option<&'a str> {
    list_segment_assignment_info(fact).map(|info| info.target)
}

#[derive(Clone, Copy)]
pub(crate) struct ListSegmentAssignmentInfo<'a> {
    target: &'a str,
    span: Span,
    is_declaration: bool,
}

pub(crate) fn list_segment_assignment_info<'a>(
    fact: &CommandFact<'a>,
) -> Option<ListSegmentAssignmentInfo<'a>> {
    match fact.command() {
        Command::Simple(command)
            if command.args.is_empty()
                && !command.assignments.is_empty()
                && fact.literal_name() == Some("") =>
        {
            single_assignment_info(&command.assignments)
        }
        Command::Decl(command) => declaration_assignment_info(command),
        _ => None,
    }
}

pub(crate) fn single_assignment_info<'a>(
    assignments: &'a [Assignment],
) -> Option<ListSegmentAssignmentInfo<'a>> {
    (assignments.len() == 1).then(|| ListSegmentAssignmentInfo {
        target: assignments[0].target.name.as_str(),
        span: assignments[0].span,
        is_declaration: false,
    })
}

pub(crate) fn declaration_assignment_info<'a>(
    command: &'a DeclClause,
) -> Option<ListSegmentAssignmentInfo<'a>> {
    if !command.assignments.is_empty() {
        return None;
    }

    let mut assignment = None;

    for operand in &command.operands {
        match operand {
            DeclOperand::Flag(_) => {}
            DeclOperand::Assignment(candidate) => {
                if assignment.replace(candidate).is_some() {
                    return None;
                }
            }
            DeclOperand::Name(_) | DeclOperand::Dynamic(_) => return None,
        }
    }

    assignment.map(|assignment| ListSegmentAssignmentInfo {
        target: assignment.target.name.as_str(),
        span: assignment.span,
        is_declaration: true,
    })
}

pub(crate) fn classify_mixed_short_circuit_kind(
    segments: &[ListSegmentFact<'_>],
    operators: &[ListOperatorFact],
) -> MixedShortCircuitKind {
    if segments
        .iter()
        .all(|segment| segment.kind() == ListSegmentKind::Condition)
    {
        MixedShortCircuitKind::TestChain
    } else if matches_assignment_ternary(segments, operators) {
        MixedShortCircuitKind::AssignmentTernary
    } else {
        MixedShortCircuitKind::Fallthrough
    }
}

pub(crate) fn matches_assignment_ternary(
    segments: &[ListSegmentFact<'_>],
    operators: &[ListOperatorFact],
) -> bool {
    let [condition, then_branch, else_branch] = segments else {
        return false;
    };
    let [first_operator, second_operator] = operators else {
        return false;
    };

    condition.kind() == ListSegmentKind::Condition
        && first_operator.op() == BinaryOp::And
        && second_operator.op() == BinaryOp::Or
        && then_branch.kind() == ListSegmentKind::AssignmentOnly
        && else_branch.kind() == ListSegmentKind::AssignmentOnly
        && then_branch.assignment_target().is_some()
        && then_branch.assignment_target() == else_branch.assignment_target()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NegativeComparison {
    runtime_operand: String,
    literal_operand: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericLiteralNormalization {
    None,
    Decimal,
    ShellArithmetic,
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_tautology_chain_operator_spans<'a>(
    commands: &[CommandFact<'a>],
    command_fact_indices_by_id: &[Option<usize>],
    lists: &[ListFact<'a>],
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for list in lists {
        if !list
            .operators()
            .iter()
            .all(|operator| operator.op() == BinaryOp::Or)
        {
            continue;
        }

        let mut prior_comparisons: Vec<NegativeComparison> = Vec::new();
        for (segment_index, segment) in list.segments().iter().enumerate() {
            let command = command_fact(commands, command_fact_indices_by_id, segment.command_id());
            let Some(comparison) = negative_comparison_for_tautology(command, source) else {
                continue;
            };

            if prior_comparisons.iter().any(|prior| {
                prior.runtime_operand == comparison.runtime_operand
                    && prior.literal_operand != comparison.literal_operand
            }) && let Some(operator) = segment_index
                .checked_sub(1)
                .and_then(|index| list.operators().get(index))
            {
                spans.push(operator.span());
            }

            prior_comparisons.push(comparison);
        }
    }

    sort_and_dedup_spans(&mut spans);
    spans
}

fn negative_comparison_for_tautology(
    command: &CommandFact<'_>,
    source: &str,
) -> Option<NegativeComparison> {
    command
        .simple_test()
        .and_then(|simple_test| simple_test_negative_comparison(simple_test, source))
        .or_else(|| {
            command
                .conditional()
                .and_then(|conditional| conditional_negative_comparison(conditional, source))
        })
}

fn simple_test_negative_comparison(
    simple_test: &SimpleTestFact<'_>,
    source: &str,
) -> Option<NegativeComparison> {
    if simple_test.syntax() != SimpleTestSyntax::Bracket
        || simple_test.effective_operands().len() != 3
    {
        return None;
    }

    let operator = simple_test_effective_operand_text(simple_test, 1, source)?;
    if !matches!(operator.as_str(), "!=" | "-ne") {
        return None;
    }
    let numeric_normalization = if operator == "-ne" {
        NumericLiteralNormalization::Decimal
    } else {
        NumericLiteralNormalization::None
    };

    negative_comparison_from_words(
        simple_test.effective_operands()[0],
        simple_test.effective_operand_class(0)?,
        simple_test.effective_operands()[2],
        simple_test.effective_operand_class(2)?,
        source,
        false,
        numeric_normalization,
    )
}

fn conditional_negative_comparison(
    conditional: &ConditionalFact<'_>,
    source: &str,
) -> Option<NegativeComparison> {
    let ConditionalNodeFact::Binary(binary) = conditional.root() else {
        return None;
    };

    let (skip_pattern_literals, numeric_normalization) = match binary.op() {
        ConditionalBinaryOp::PatternNe => (true, NumericLiteralNormalization::None),
        ConditionalBinaryOp::ArithmeticNe => (false, NumericLiteralNormalization::ShellArithmetic),
        _ => return None,
    };

    negative_comparison_from_operands(
        binary.left(),
        binary.right(),
        source,
        skip_pattern_literals,
        numeric_normalization,
    )
}

fn negative_comparison_from_operands(
    left: ConditionalOperandFact<'_>,
    right: ConditionalOperandFact<'_>,
    source: &str,
    skip_pattern_literals: bool,
    numeric_normalization: NumericLiteralNormalization,
) -> Option<NegativeComparison> {
    match (left.class(), right.class()) {
        (TestOperandClass::FixedLiteral, TestOperandClass::RuntimeSensitive) => {
            let literal = conditional_operand_literal_text(left, source, skip_pattern_literals)?;
            Some(NegativeComparison {
                runtime_operand: conditional_operand_source_text(right, source).to_owned(),
                literal_operand: normalize_tautology_literal(literal, numeric_normalization)?,
            })
        }
        (TestOperandClass::RuntimeSensitive, TestOperandClass::FixedLiteral) => {
            let literal = conditional_operand_literal_text(right, source, skip_pattern_literals)?;
            Some(NegativeComparison {
                runtime_operand: conditional_operand_source_text(left, source).to_owned(),
                literal_operand: normalize_tautology_literal(literal, numeric_normalization)?,
            })
        }
        _ => None,
    }
}

fn negative_comparison_from_words(
    left: &Word,
    left_class: TestOperandClass,
    right: &Word,
    right_class: TestOperandClass,
    source: &str,
    skip_pattern_literals: bool,
    numeric_normalization: NumericLiteralNormalization,
) -> Option<NegativeComparison> {
    match (left_class, right_class) {
        (TestOperandClass::FixedLiteral, TestOperandClass::RuntimeSensitive) => {
            let literal = word_literal_text(left, source, skip_pattern_literals)?;
            Some(NegativeComparison {
                runtime_operand: right.span.slice(source).to_owned(),
                literal_operand: normalize_tautology_literal(literal, numeric_normalization)?,
            })
        }
        (TestOperandClass::RuntimeSensitive, TestOperandClass::FixedLiteral) => {
            let literal = word_literal_text(right, source, skip_pattern_literals)?;
            Some(NegativeComparison {
                runtime_operand: left.span.slice(source).to_owned(),
                literal_operand: normalize_tautology_literal(literal, numeric_normalization)?,
            })
        }
        _ => None,
    }
}

fn conditional_operand_literal_text(
    operand: ConditionalOperandFact<'_>,
    source: &str,
    skip_pattern_literals: bool,
) -> Option<String> {
    let text = if let Some(word) = operand.word() {
        word_literal_text(word, source, skip_pattern_literals)?
    } else {
        let text = operand.expression().span().slice(source).to_owned();
        if skip_pattern_literals && looks_like_conditional_pattern_literal(&text) {
            return None;
        }
        text
    };

    Some(text)
}

fn conditional_operand_source_text<'a>(
    operand: ConditionalOperandFact<'_>,
    source: &'a str,
) -> &'a str {
    operand.expression().span().slice(source)
}

fn word_literal_text(word: &Word, source: &str, skip_pattern_literals: bool) -> Option<String> {
    let text = static_word_text(word, source)?;
    if skip_pattern_literals && word_has_unquoted_conditional_pattern_literal(word, source) {
        return None;
    }

    Some(text.into_owned())
}

fn normalize_tautology_literal(
    literal: String,
    numeric_normalization: NumericLiteralNormalization,
) -> Option<String> {
    match numeric_normalization {
        NumericLiteralNormalization::None => Some(literal),
        NumericLiteralNormalization::Decimal => normalize_decimal_integer_literal(&literal),
        NumericLiteralNormalization::ShellArithmetic => {
            normalize_shell_arithmetic_integer_literal(&literal)
        }
    }
}

fn normalize_decimal_integer_literal(text: &str) -> Option<String> {
    let (negative, unsigned) = if let Some(digits) = text.strip_prefix('-') {
        (true, digits)
    } else if let Some(digits) = text.strip_prefix('+') {
        (false, digits)
    } else {
        (false, text)
    };

    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let value = parse_unsigned_digits(unsigned, 10)?;
    if negative {
        value.checked_neg().map(|value| value.to_string())
    } else {
        Some(value.to_string())
    }
}

fn normalize_shell_arithmetic_integer_literal(text: &str) -> Option<String> {
    parse_shell_arithmetic_integer_value(text).map(|value| value.to_string())
}

fn parse_shell_arithmetic_integer_value(text: &str) -> Option<i128> {
    let (negative, unsigned) = if let Some(digits) = text.strip_prefix('-') {
        (true, digits)
    } else if let Some(digits) = text.strip_prefix('+') {
        (false, digits)
    } else {
        (false, text)
    };
    if unsigned.is_empty() {
        return None;
    }

    let value = if let Some((base_text, digits)) = unsigned.split_once('#') {
        let base = parse_unsigned_digits(base_text, 10)?;
        if !(2..=64).contains(&base) || digits.is_empty() {
            return None;
        }
        parse_arithmetic_digits(digits, base as u32)?
    } else if let Some(digits) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        if digits.is_empty() {
            return None;
        }
        parse_arithmetic_digits(digits, 16)?
    } else if unsigned.len() > 1 && unsigned.starts_with('0') {
        parse_arithmetic_digits(unsigned, 8)?
    } else {
        parse_unsigned_digits(unsigned, 10)?
    };

    if negative {
        value.checked_neg()
    } else {
        Some(value)
    }
}

fn parse_unsigned_digits(digits: &str, radix: u32) -> Option<i128> {
    if digits.is_empty() {
        return None;
    }
    parse_arithmetic_digits(digits, radix)
}

fn parse_arithmetic_digits(digits: &str, radix: u32) -> Option<i128> {
    let mut value = 0i128;
    let radix_value = i128::from(radix);
    for byte in digits.bytes() {
        let digit = i128::from(arithmetic_digit_value(byte, radix)?);
        if digit >= radix_value {
            return None;
        }
        value = value.checked_mul(radix_value)?.checked_add(digit)?;
    }
    Some(value)
}

fn arithmetic_digit_value(byte: u8, radix: u32) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' if radix <= 36 => Some(u32::from(byte - b'A') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 36),
        b'@' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

fn word_has_unquoted_conditional_pattern_literal(word: &Word, source: &str) -> bool {
    !word_spans::word_unquoted_glob_pattern_spans(word, source).is_empty()
        || word_has_unquoted_conditional_extglob_operator(&word.parts, source, false)
}

fn word_has_unquoted_conditional_extglob_operator(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
) -> bool {
    let mut literal_run_start = None::<usize>;
    let mut literal_run_end = None::<usize>;

    let flush_literal_run = |literal_run_start: &mut Option<usize>,
                             literal_run_end: &mut Option<usize>| {
        let Some(start_index) = literal_run_start.take() else {
            return false;
        };
        let Some(end_index) = literal_run_end.take() else {
            return false;
        };
        let start = parts[start_index].span.start;
        let end = parts[end_index].span.end;
        unquoted_text_has_conditional_extglob_operator(
            Span::from_positions(start, end).slice(source),
        )
    };

    for (index, part) in parts.iter().enumerate() {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if flush_literal_run(&mut literal_run_start, &mut literal_run_end) {
                    return true;
                }
                if word_has_unquoted_conditional_extglob_operator(parts, source, true) {
                    return true;
                }
            }
            WordPart::Literal(_) if !in_double_quotes => {
                literal_run_start.get_or_insert(index);
                literal_run_end = Some(index);
            }
            WordPart::Literal(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                if flush_literal_run(&mut literal_run_start, &mut literal_run_end) {
                    return true;
                }
            }
        }
    }

    flush_literal_run(&mut literal_run_start, &mut literal_run_end)
}

fn unquoted_text_has_conditional_extglob_operator(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if matches!(bytes[index], b'@' | b'!' | b'+')
            && bytes[index + 1] == b'('
            && !byte_is_escaped(bytes, index)
        {
            return true;
        }
        index += 1;
    }
    false
}

fn byte_is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut slash_count = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        slash_count += 1;
        cursor -= 1;
    }
    slash_count % 2 == 1
}

fn looks_like_conditional_pattern_literal(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '*' | '?' | '[' | ']') {
            return true;
        }

        if matches!(ch, '@' | '!' | '+') && matches!(chars.peek(), Some('(')) {
            return true;
        }
    }

    false
}
