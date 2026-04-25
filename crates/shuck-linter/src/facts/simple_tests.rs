#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleTestSyntax {
    Test,
    Bracket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleTestShape {
    Empty,
    Truthy,
    Unary,
    Binary,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleTestOperatorFamily {
    StringUnary,
    StringBinary,
    Other,
}

#[derive(Debug, Clone)]
pub struct SimpleTestFact<'a> {
    syntax: SimpleTestSyntax,
    operands: Box<[&'a Word]>,
    shape: SimpleTestShape,
    operator_family: SimpleTestOperatorFamily,
    effective_operand_offset: usize,
    effective_shape: SimpleTestShape,
    effective_operator_family: SimpleTestOperatorFamily,
    operand_classes: Box<[TestOperandClass]>,
    empty_test_suppressed: bool,
}

impl<'a> SimpleTestFact<'a> {
    pub fn syntax(&self) -> SimpleTestSyntax {
        self.syntax
    }

    pub fn operands(&self) -> &[&'a Word] {
        &self.operands
    }

    pub fn shape(&self) -> SimpleTestShape {
        self.shape
    }

    pub fn operator_family(&self) -> SimpleTestOperatorFamily {
        self.operator_family
    }

    pub fn is_effectively_negated(&self) -> bool {
        self.effective_operand_offset != 0
    }

    pub fn effective_operands(&self) -> &[&'a Word] {
        &self.operands[self.effective_operand_offset..]
    }

    pub fn effective_shape(&self) -> SimpleTestShape {
        self.effective_shape
    }

    pub fn effective_operator_family(&self) -> SimpleTestOperatorFamily {
        self.effective_operator_family
    }

    pub fn operand_classes(&self) -> &[TestOperandClass] {
        &self.operand_classes
    }

    pub fn operand_class(&self, index: usize) -> Option<TestOperandClass> {
        self.operand_classes.get(index).copied()
    }

    pub fn effective_operand_class(&self, index: usize) -> Option<TestOperandClass> {
        self.operand_classes
            .get(self.effective_operand_offset + index)
            .copied()
    }

    pub fn empty_test_suppressed(&self) -> bool {
        self.empty_test_suppressed
    }

    pub fn truthy_operand_class(&self) -> Option<TestOperandClass> {
        (self.shape == SimpleTestShape::Truthy)
            .then(|| self.operand_class(0))
            .flatten()
    }

    pub fn unary_operand_class(&self) -> Option<TestOperandClass> {
        (self.shape == SimpleTestShape::Unary)
            .then(|| self.operand_class(1))
            .flatten()
    }

    pub fn effective_operator_word(&self) -> Option<&'a Word> {
        match self.effective_shape {
            SimpleTestShape::Unary => self.effective_operands().first().copied(),
            SimpleTestShape::Binary => self.effective_operands().get(1).copied(),
            SimpleTestShape::Empty | SimpleTestShape::Truthy | SimpleTestShape::Other => None,
        }
    }

    pub fn escaped_negation_spans(&self, source: &str) -> Option<(Span, Span)> {
        let leading = self.operands.first().copied()?;
        if leading.span.slice(source) != "\\!" {
            return None;
        }

        escaped_negation_is_operator(self, source).then(|| {
            let diagnostic_span =
                if self.syntax == SimpleTestSyntax::Bracket && self.shape == SimpleTestShape::Binary
                {
                    self.operands
                        .get(1)
                        .copied()
                        .map_or(leading.span, |word| word.span)
                } else {
                    leading.span
                };
            let fix_start = leading.span.start;
            let fix_end = fix_start.advanced_by("\\");
            (diagnostic_span, Span::from_positions(fix_start, fix_end))
        })
    }

    pub fn compound_operator_spans(&self, source: &str) -> Vec<Span> {
        let Some((end, spans)) = simple_test_parse_logical_expression(self, 0, source) else {
            return Vec::new();
        };

        if end == self.effective_operands().len() {
            spans
        } else {
            Vec::new()
        }
    }

    pub fn truthy_expression_words(&'a self, source: &str) -> Vec<&'a Word> {
        simple_test_expressions(self, source)
            .into_iter()
            .filter_map(|expression| match expression {
                SimpleTestExpression::Truthy(word) => Some(word),
                SimpleTestExpression::StringUnary { .. }
                | SimpleTestExpression::StringBinary { .. } => None,
            })
            .collect()
    }

    pub fn operator_expression_operand_words(&self, source: &str) -> Vec<&'a Word> {
        simple_test_operator_expression_operand_words(self, source)
    }

    pub fn numeric_binary_expression_operand_words(&self, source: &str) -> Vec<&'a Word> {
        simple_test_numeric_binary_expression_operand_words(self, source)
    }

    pub fn string_unary_expression_words(&'a self, source: &str) -> Vec<(&'a Word, &'a Word)> {
        simple_test_expressions(self, source)
            .into_iter()
            .filter_map(|expression| match expression {
                SimpleTestExpression::StringUnary { operator, operand } => {
                    Some((operator, operand))
                }
                SimpleTestExpression::Truthy(_) | SimpleTestExpression::StringBinary { .. } => None,
            })
            .collect()
    }

    pub fn string_binary_expression_words(
        &'a self,
        source: &str,
    ) -> Vec<(&'a Word, &'a Word, &'a Word)> {
        simple_test_expressions(self, source)
            .into_iter()
            .filter_map(|expression| match expression {
                SimpleTestExpression::StringBinary {
                    left,
                    operator,
                    right,
                } => Some((left, operator, right)),
                SimpleTestExpression::Truthy(_) | SimpleTestExpression::StringUnary { .. } => None,
            })
            .collect()
    }

    pub fn is_abort_like_bracket_test(&self, source: &str) -> bool {
        if self.syntax != SimpleTestSyntax::Bracket
            || self.effective_shape != SimpleTestShape::Other
        {
            return false;
        }

        self.effective_operands()
            .iter()
            .enumerate()
            .any(|(index, word)| {
                self.effective_operand_class(index)
                    .is_some_and(|class| class.is_fixed_literal())
                    && matches!(
                        static_word_text(word, source).as_deref(),
                        Some("(") | Some(")")
                    )
            })
    }

    pub fn binary_operand_classes(&self) -> Option<(TestOperandClass, TestOperandClass)> {
        (self.shape == SimpleTestShape::Binary)
            .then(|| Some((self.operand_class(0)?, self.operand_class(2)?)))
            .flatten()
    }
}

fn escaped_negation_is_operator(simple_test: &SimpleTestFact<'_>, source: &str) -> bool {
    match simple_test.shape() {
        SimpleTestShape::Unary => true,
        SimpleTestShape::Binary => simple_test
            .operands()
            .get(1)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .is_some_and(|operator| !simple_test_is_binary_operator(operator)),
        SimpleTestShape::Other => simple_test
            .operands()
            .get(2)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .is_some_and(simple_test_is_binary_operator),
        SimpleTestShape::Empty | SimpleTestShape::Truthy => false,
    }
}

enum SimpleTestExpression<'a> {
    Truthy(&'a Word),
    StringUnary {
        operator: &'a Word,
        operand: &'a Word,
    },
    StringBinary {
        left: &'a Word,
        operator: &'a Word,
        right: &'a Word,
    },
}

fn simple_test_operands<'a>(command: &'a SimpleCommand, source: &str) -> Option<&'a [Word]> {
    match static_word_text(&command.name, source).as_deref()? {
        "[" => {
            let (closing_bracket, operands) = command.args.split_last()?;
            (static_word_text(closing_bracket, source).as_deref() == Some("]")).then_some(operands)
        }
        "test" => Some(&command.args),
        _ => None,
    }
}

fn build_simple_test_fact<'a>(
    command: &'a Command,
    source: &str,
    file_context: &FileContext,
) -> Option<SimpleTestFact<'a>> {
    let Command::Simple(command) = command else {
        return None;
    };
    let syntax = match static_word_text(&command.name, source).as_deref()? {
        "test" => SimpleTestSyntax::Test,
        "[" => SimpleTestSyntax::Bracket,
        _ => return None,
    };
    let operands = match syntax {
        SimpleTestSyntax::Test => command.args.iter().collect::<Vec<_>>(),
        SimpleTestSyntax::Bracket => {
            let (closing_bracket, operands) = command.args.split_last()?;
            if static_word_text(closing_bracket, source).as_deref() != Some("]") {
                return None;
            }
            operands.iter().collect::<Vec<_>>()
        }
    };
    let shape = simple_test_shape(operands.len());
    let operator_family = simple_test_operator_family(&operands, shape, source);
    let effective_operand_offset = simple_test_effective_operand_offset(&operands, source);
    let effective_shape =
        simple_test_shape(operands.len().saturating_sub(effective_operand_offset));
    let effective_operator_family = simple_test_operator_family(
        &operands[effective_operand_offset..],
        effective_shape,
        source,
    );
    let operand_classes = operands
        .iter()
        .map(|word| classify_contextual_operand(word, source, ExpansionContext::CommandArgument))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    Some(SimpleTestFact {
        syntax,
        operands: operands.into_boxed_slice(),
        shape,
        operator_family,
        effective_operand_offset,
        effective_shape,
        effective_operator_family,
        operand_classes,
        empty_test_suppressed: file_context
            .span_intersects_kind(ContextRegionKind::ShellSpecParametersBlock, command.span),
    })
}

fn build_glued_closing_bracket_operand_span(command: &Command, source: &str) -> Option<Span> {
    build_glued_closing_bracket_operand_word(command, source)
        .map(|operand| Span::from_positions(operand.span.start, operand.span.start))
}

fn build_glued_closing_bracket_insert_offset(command: &Command, source: &str) -> Option<usize> {
    build_glued_closing_bracket_operand_word(command, source).map(|operand| operand.span.end.offset - 1)
}

fn build_glued_closing_bracket_operand_word<'a>(
    command: &'a Command,
    source: &str,
) -> Option<&'a Word> {
    let Command::Simple(command) = command else {
        return None;
    };
    if static_word_text(&command.name, source).as_deref() != Some("[") {
        return None;
    }

    let args = command.args.iter().collect::<Vec<_>>();
    let last = args.last()?;
    let text = last.span.slice(source);
    if text == "]" || !text.ends_with(']') || text.ends_with("\\]") {
        return None;
    }

    glued_closing_bracket_unary_operand(&args, source)
}

fn glued_closing_bracket_unary_operand<'a>(args: &[&'a Word], source: &str) -> Option<&'a Word> {
    let [first, second] = args else {
        let [bang, operator, operand] = args else {
            return None;
        };
        return (bang.span.slice(source) == "!"
            && simple_test_is_unary_operator(operator.span.slice(source))
            && operand
                .span
                .slice(source)
                .strip_suffix(']')
                .is_some_and(|prefix| !prefix.is_empty()))
        .then_some(*operand);
    };

    (simple_test_is_unary_operator(first.span.slice(source))
        && second
            .span
            .slice(source)
            .strip_suffix(']')
            .is_some_and(|prefix| !prefix.is_empty()))
    .then_some(*second)
}

fn simple_test_shape(operand_count: usize) -> SimpleTestShape {
    match operand_count {
        0 => SimpleTestShape::Empty,
        1 => SimpleTestShape::Truthy,
        2 => SimpleTestShape::Unary,
        3 => SimpleTestShape::Binary,
        _ => SimpleTestShape::Other,
    }
}

fn simple_test_effective_operand_offset(operands: &[&Word], source: &str) -> usize {
    if operands
        .first()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        != Some("!")
    {
        return 0;
    }

    match operands.len() {
        0 | 1 => 0,
        3 if operands
            .get(1)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .is_some_and(simple_test_is_binary_operator) =>
        {
            0
        }
        _ => 1,
    }
}

fn simple_test_is_binary_operator(operator: &str) -> bool {
    matches!(
        operator,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-gt"
            | "-ge"
            | "-lt"
            | "-le"
            | "-nt"
            | "-ot"
            | "-ef"
            | "-a"
            | "-o"
    )
}

fn simple_test_is_unary_operator(operator: &str) -> bool {
    matches!(
        operator,
        "-e" | "-a"
            | "-f"
            | "-d"
            | "-c"
            | "-b"
            | "-p"
            | "-S"
            | "-L"
            | "-h"
            | "-k"
            | "-g"
            | "-u"
            | "-G"
            | "-O"
            | "-N"
            | "-r"
            | "-w"
            | "-x"
            | "-s"
            | "-t"
            | "-z"
            | "-n"
            | "-o"
            | "-v"
            | "-R"
    )
}

fn simple_test_operator_family(
    operands: &[&Word],
    shape: SimpleTestShape,
    source: &str,
) -> SimpleTestOperatorFamily {
    match shape {
        SimpleTestShape::Unary => operands
            .first()
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .map_or(
                SimpleTestOperatorFamily::Other,
                simple_test_unary_operator_family,
            ),
        SimpleTestShape::Binary => operands
            .get(1)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .map_or(
                SimpleTestOperatorFamily::Other,
                simple_test_binary_operator_family,
            ),
        _ => SimpleTestOperatorFamily::Other,
    }
}

fn simple_test_unary_operator_family(operator: &str) -> SimpleTestOperatorFamily {
    if matches!(operator, "-n" | "-z") {
        SimpleTestOperatorFamily::StringUnary
    } else {
        SimpleTestOperatorFamily::Other
    }
}

fn simple_test_binary_operator_family(operator: &str) -> SimpleTestOperatorFamily {
    if matches!(operator, "=" | "==" | "!=" | "<" | ">") {
        SimpleTestOperatorFamily::StringBinary
    } else {
        SimpleTestOperatorFamily::Other
    }
}

fn simple_test_expressions<'a>(
    simple_test: &'a SimpleTestFact<'a>,
    source: &str,
) -> Vec<SimpleTestExpression<'a>> {
    let operands = simple_test.effective_operands();
    let mut expressions = Vec::new();
    let mut segment_start = 0;

    for index in 0..=operands.len() {
        let is_connector = index < operands.len()
            && simple_test_effective_operand_text(simple_test, index, source)
                .as_deref()
                .is_some_and(simple_test_is_logical_connector);
        let splits_segment = is_connector
            && simple_test_segment_is_expression(simple_test, segment_start, index, source);
        if !splits_segment && index != operands.len() {
            continue;
        }

        if let Some(expression) =
            parse_simple_test_expression_segment(simple_test, segment_start, index, source)
        {
            expressions.push(expression);
        }

        segment_start = index + 1;
    }

    expressions
}

fn simple_test_operator_expression_operand_words<'a>(
    simple_test: &SimpleTestFact<'a>,
    source: &str,
) -> Vec<&'a Word> {
    let operands = simple_test.effective_operands();
    let mut expression_operands = Vec::new();
    let mut segment_start = 0;

    for index in 0..=operands.len() {
        let is_connector = index < operands.len()
            && simple_test_effective_operand_text(simple_test, index, source)
                .as_deref()
                .is_some_and(simple_test_is_logical_connector);
        let splits_segment = is_connector
            && simple_test_segment_is_expression(simple_test, segment_start, index, source);
        if !splits_segment && index != operands.len() {
            continue;
        }

        collect_simple_test_operator_expression_operand_words(
            simple_test,
            segment_start,
            index,
            source,
            &mut expression_operands,
        );

        segment_start = index + 1;
    }

    expression_operands
}

fn simple_test_numeric_binary_expression_operand_words<'a>(
    simple_test: &SimpleTestFact<'a>,
    source: &str,
) -> Vec<&'a Word> {
    let operands = simple_test.effective_operands();
    let mut expression_operands = Vec::new();
    let mut segment_start = 0;

    for index in 0..=operands.len() {
        let is_connector = index < operands.len()
            && simple_test_effective_operand_text(simple_test, index, source)
                .as_deref()
                .is_some_and(simple_test_is_logical_connector);
        let splits_segment = is_connector
            && simple_test_segment_is_expression(simple_test, segment_start, index, source);
        if !splits_segment && index != operands.len() {
            continue;
        }

        collect_simple_test_numeric_binary_expression_operand_words(
            simple_test,
            segment_start,
            index,
            source,
            &mut expression_operands,
        );

        segment_start = index + 1;
    }

    expression_operands
}

fn collect_simple_test_operator_expression_operand_words<'a>(
    simple_test: &SimpleTestFact<'a>,
    start: usize,
    end: usize,
    source: &str,
    expression_operands: &mut Vec<&'a Word>,
) {
    if start >= end {
        return;
    }

    let segment = &simple_test.effective_operands()[start..end];
    let mut expression_start = 0;
    while expression_start + 1 < segment.len()
        && simple_test_effective_operand_text(simple_test, start + expression_start, source)
            .as_deref()
            == Some("!")
    {
        expression_start += 1;
    }

    let expression = &segment[expression_start..];
    match expression {
        [operator, operand]
            if simple_test_effective_operand_text(
                simple_test,
                start + expression_start,
                source,
            )
            .as_deref()
            .is_some_and(simple_test_is_unary_operator) =>
        {
            expression_operands.push(operand);
        }
        [left, operator, right]
            if simple_test_effective_operand_text(
                simple_test,
                start + expression_start + 1,
                source,
            )
            .as_deref()
            .is_some_and(simple_test_is_nonlogical_binary_operator) =>
        {
            expression_operands.push(left);
            expression_operands.push(right);
        }
        [..] => {}
    }
}

fn collect_simple_test_numeric_binary_expression_operand_words<'a>(
    simple_test: &SimpleTestFact<'a>,
    start: usize,
    end: usize,
    source: &str,
    expression_operands: &mut Vec<&'a Word>,
) {
    if start >= end {
        return;
    }

    let segment = &simple_test.effective_operands()[start..end];
    let mut expression_start = 0;
    while expression_start + 1 < segment.len()
        && simple_test_effective_operand_text(simple_test, start + expression_start, source)
            .as_deref()
            == Some("!")
    {
        expression_start += 1;
    }

    let expression = &segment[expression_start..];
    match expression {
        [left, operator, right]
            if simple_test_effective_operand_text(
                simple_test,
                start + expression_start + 1,
                source,
            )
            .as_deref()
            .is_some_and(simple_test_is_numeric_binary_operator) =>
        {
            expression_operands.push(left);
            expression_operands.push(right);
        }
        [..] => {}
    }
}

fn simple_test_segment_is_expression(
    simple_test: &SimpleTestFact<'_>,
    start: usize,
    end: usize,
    source: &str,
) -> bool {
    if start >= end {
        return false;
    }

    let segment = &simple_test.effective_operands()[start..end];
    let mut expression_start = 0;
    while expression_start + 1 < segment.len()
        && simple_test_effective_operand_text(simple_test, start + expression_start, source)
            .as_deref()
            == Some("!")
    {
        expression_start += 1;
    }

    let expression_len = segment.len() - expression_start;
    match expression_len {
        1 => {
            let word = segment[expression_start];
            !(simple_test_effective_operand_text(simple_test, start + expression_start, source)
                .as_deref()
                == Some("!")
                && classify_word(word, source).quote == WordQuote::Unquoted)
        }
        2 => simple_test_effective_operand_text(simple_test, start + expression_start, source)
            .as_deref()
            .is_some_and(simple_test_is_unary_operator),
        3 => simple_test_effective_operand_text(simple_test, start + expression_start + 1, source)
            .as_deref()
            .is_some_and(simple_test_is_binary_operator),
        _ => false,
    }
}

fn parse_simple_test_expression_segment<'a>(
    simple_test: &'a SimpleTestFact<'a>,
    start: usize,
    end: usize,
    source: &str,
) -> Option<SimpleTestExpression<'a>> {
    if start >= end {
        return None;
    }

    let segment = &simple_test.effective_operands()[start..end];
    let mut expression_start = 0;
    while expression_start + 1 < segment.len()
        && simple_test_effective_operand_text(simple_test, start + expression_start, source)
            .as_deref()
            == Some("!")
    {
        expression_start += 1;
    }

    let expression = &segment[expression_start..];
    match expression {
        [word] => Some(SimpleTestExpression::Truthy(word)),
        [operator, operand]
            if simple_test_effective_operand_text(
                simple_test,
                start + expression_start,
                source,
            )
            .as_deref()
            .is_some_and(simple_test_is_string_unary_operator) =>
        {
            Some(SimpleTestExpression::StringUnary { operator, operand })
        }
        [left, operator, right]
            if simple_test_effective_operand_text(
                simple_test,
                start + expression_start + 1,
                source,
            )
            .as_deref()
            .is_some_and(simple_test_is_string_binary_operator) =>
        {
            Some(SimpleTestExpression::StringBinary {
                left,
                operator,
                right,
            })
        }
        [] | [_, _, ..] => None,
    }
}

fn simple_test_effective_operand_text(
    simple_test: &SimpleTestFact<'_>,
    index: usize,
    source: &str,
) -> Option<String> {
    let word = simple_test.effective_operands().get(index).copied()?;
    let class = simple_test.effective_operand_class(index)?;
    if !class.is_fixed_literal() {
        return None;
    }

    let mut text = static_word_text(word, source)?;
    if classify_word(word, source).quote == WordQuote::Unquoted {
        match text.as_ref() {
            r"\(" => text = "(".into(),
            r"\)" => text = ")".into(),
            r"\!" => text = "!".into(),
            _ => {}
        }
    }

    Some(text.into_owned())
}

fn simple_test_effective_unquoted_operand_text(
    simple_test: &SimpleTestFact<'_>,
    index: usize,
    source: &str,
) -> Option<String> {
    let word = simple_test.effective_operands().get(index).copied()?;
    (classify_word(word, source).quote == WordQuote::Unquoted)
        .then(|| simple_test_effective_operand_text(simple_test, index, source))
        .flatten()
}

fn simple_test_is_logical_connector(text: &str) -> bool {
    matches!(text, "-a" | "-o")
}

fn simple_test_is_logical_negation(
    simple_test: &SimpleTestFact<'_>,
    index: usize,
    source: &str,
) -> bool {
    if simple_test_effective_unquoted_operand_text(simple_test, index, source).as_deref()
        == Some("!")
    {
        return true;
    }

    simple_test_effective_operand_text(simple_test, index, source).as_deref() == Some("!")
        && simple_test_effective_operand_text(simple_test, index + 1, source).as_deref()
            != Some("(")
}

fn simple_test_is_string_unary_operator(text: &str) -> bool {
    matches!(text, "-n" | "-z")
}

fn simple_test_is_string_binary_operator(text: &str) -> bool {
    matches!(text, "=" | "==" | "!=" | "<" | ">")
}

fn simple_test_parse_logical_expression(
    simple_test: &SimpleTestFact<'_>,
    start: usize,
    source: &str,
) -> Option<(usize, Vec<Span>)> {
    let (mut index, mut spans) = simple_test_parse_logical_term(simple_test, start, source)?;

    loop {
        let Some(connector_span) = simple_test_logical_connector_span(simple_test, index, source)
        else {
            break;
        };
        let (next_index, next_spans) =
            simple_test_parse_logical_term(simple_test, index + 1, source)?;
        spans.push(connector_span);
        spans.extend(next_spans);
        index = next_index;
    }

    Some((index, spans))
}

fn simple_test_parse_logical_term(
    simple_test: &SimpleTestFact<'_>,
    start: usize,
    source: &str,
) -> Option<(usize, Vec<Span>)> {
    let mut index = start;

    while index + 1 < simple_test.effective_operands().len()
        && simple_test_is_logical_negation(simple_test, index, source)
    {
        index += 1;
    }

    simple_test_parse_logical_primary(simple_test, index, source)
}

fn simple_test_parse_logical_primary(
    simple_test: &SimpleTestFact<'_>,
    index: usize,
    source: &str,
) -> Option<(usize, Vec<Span>)> {
    if simple_test_effective_operand_text(simple_test, index, source).as_deref() == Some("(")
        && let Some((end, spans)) =
            simple_test_parse_logical_expression(simple_test, index + 1, source)
        && simple_test_effective_operand_text(simple_test, end, source).as_deref() == Some(")")
    {
        return Some((end + 1, spans));
    }

    if simple_test_effective_operand_text(simple_test, index, source)
        .as_deref()
        .is_some_and(simple_test_is_unary_operator)
        && simple_test.effective_operands().get(index + 1).is_some()
    {
        return Some((index + 2, Vec::new()));
    }

    if simple_test_effective_operand_text(simple_test, index + 1, source)
        .as_deref()
        .is_some_and(simple_test_is_nonlogical_binary_operator)
        && simple_test.effective_operands().get(index + 2).is_some()
    {
        return Some((index + 3, Vec::new()));
    }

    simple_test
        .effective_operands()
        .get(index)
        .map(|_| (index + 1, Vec::new()))
}

fn simple_test_logical_connector_span(
    simple_test: &SimpleTestFact<'_>,
    index: usize,
    source: &str,
) -> Option<Span> {
    let word = simple_test.effective_operands().get(index).copied()?;
    simple_test_effective_unquoted_operand_text(simple_test, index, source)
        .as_deref()
        .is_some_and(simple_test_is_logical_connector)
        .then_some(word.span)
}

fn simple_test_is_nonlogical_binary_operator(text: &str) -> bool {
    simple_test_is_binary_operator(text) && !simple_test_is_logical_connector(text)
}

fn simple_test_is_numeric_binary_operator(text: &str) -> bool {
    matches!(text, "-eq" | "-ne" | "-gt" | "-ge" | "-lt" | "-le")
}

pub(super) fn build_single_test_subshell_spans<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    source: &str,
) -> Vec<Span> {
    let command_relationships =
        CommandRelationshipContext::new(commands, command_ids_by_span, command_child_index);
    commands
        .iter()
        .filter_map(|fact| single_test_subshell_span(fact, command_relationships, source))
        .collect()
}

pub(super) fn build_subshell_test_group_spans<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    source: &str,
) -> Vec<Span> {
    let command_relationships =
        CommandRelationshipContext::new(commands, command_ids_by_span, command_child_index);
    commands
        .iter()
        .filter_map(|fact| subshell_test_group_span(fact, command_relationships, source))
        .collect()
}

fn single_test_subshell_span<'a>(
    fact: &CommandFact<'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> Option<Span> {
    let condition = match fact.command() {
        Command::Compound(CompoundCommand::If(command)) => &command.condition,
        Command::Compound(CompoundCommand::While(command)) => &command.condition,
        Command::Compound(CompoundCommand::Until(command)) => &command.condition,
        _ => return None,
    };

    let [stmt] = condition.as_slice() else {
        return None;
    };

    let condition_fact = command_relationships.child_or_lookup_fact(fact.id(), stmt)?;
    let Command::Compound(CompoundCommand::Subshell(body)) = condition_fact.command() else {
        return None;
    };

    let [body_stmt] = body.as_slice() else {
        return None;
    };

    let body_fact = command_relationships.child_or_lookup_fact(condition_fact.id(), body_stmt)?;
    let simple_test = is_test_like_command(body_fact);
    if stmt.negated && !simple_test {
        return None;
    }

    if !simple_test && !is_test_condition_fact(body_fact, command_relationships) {
        return None;
    }

    Some(subshell_anchor_span(
        condition_fact.span_in_source(source),
        source,
    ))
}

fn is_test_like_command(fact: &CommandFact<'_>) -> bool {
    fact.wrappers()
        .iter()
        .all(|wrapper| matches!(wrapper, WrapperKind::Command | WrapperKind::Builtin))
        && (fact.effective_name_is("test")
            || fact.effective_name_is("[")
            || matches!(
                fact.command(),
                Command::Compound(CompoundCommand::Conditional(_))
            ))
}

fn is_test_condition_fact<'a>(
    fact: &CommandFact<'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> bool {
    match fact.command() {
        Command::Binary(binary) if matches!(binary.op, BinaryOp::And | BinaryOp::Or) => {
            let Some(left) = command_relationships.child_or_lookup_fact(fact.id(), &binary.left)
            else {
                return false;
            };
            let Some(right) = command_relationships.child_or_lookup_fact(fact.id(), &binary.right)
            else {
                return false;
            };
            is_test_condition_fact(left, command_relationships)
                && is_test_condition_fact(right, command_relationships)
        }
        _ => is_test_like_command(fact),
    }
}

fn subshell_test_group_span<'a>(
    fact: &CommandFact<'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> Option<Span> {
    let Command::Compound(CompoundCommand::Subshell(body)) = fact.command() else {
        return None;
    };

    if !subshell_body_contains_grouped_tests(body, fact.id(), command_relationships) {
        return None;
    }

    Some(subshell_anchor_span(fact.span(), source))
}

fn subshell_body_contains_grouped_tests<'a>(
    body: &StmtSeq,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> bool {
    subshell_body_analysis(body, parent_id, command_relationships)
        .is_some_and(|analysis| analysis.has_grouping && analysis.test_count > 0)
}

#[derive(Debug, Default, Clone, Copy)]
struct GroupedTestAnalysis {
    test_count: usize,
    has_grouping: bool,
}

fn subshell_stmt_analysis<'a>(
    stmt: &Stmt,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> Option<GroupedTestAnalysis> {
    let fact = command_relationships.child_or_lookup_fact(parent_id, stmt)?;
    subshell_command_analysis(fact, command_relationships)
}

fn subshell_command_analysis<'a>(
    fact: &CommandFact<'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> Option<GroupedTestAnalysis> {
    match fact.command() {
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Compound(CompoundCommand::Conditional(_)) => {
            if is_test_like_command(fact) {
                return Some(GroupedTestAnalysis {
                    test_count: 1,
                    has_grouping: false,
                });
            }
            None
        }
        Command::Compound(CompoundCommand::BraceGroup(body)) => {
            let inner = subshell_body_analysis(body, fact.id(), command_relationships)?;
            Some(GroupedTestAnalysis {
                test_count: inner.test_count,
                has_grouping: true,
            })
        }
        Command::Compound(CompoundCommand::Subshell(body)) => {
            let inner = subshell_body_analysis(body, fact.id(), command_relationships)?;
            Some(GroupedTestAnalysis {
                test_count: inner.test_count,
                has_grouping: inner.has_grouping,
            })
        }
        Command::Binary(binary) if matches!(binary.op, BinaryOp::And | BinaryOp::Or) => {
            let left =
                subshell_stmt_analysis(&binary.left, fact.id(), command_relationships)?;
            let right =
                subshell_stmt_analysis(&binary.right, fact.id(), command_relationships)?;
            Some(GroupedTestAnalysis {
                test_count: left.test_count + right.test_count,
                has_grouping: true,
            })
        }
        _ => None,
    }
}

fn subshell_body_analysis<'a>(
    body: &StmtSeq,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> Option<GroupedTestAnalysis> {
    let mut analysis = GroupedTestAnalysis::default();

    if body.stmts.len() > 1 {
        analysis.has_grouping = true;
    }

    for stmt in &body.stmts {
        let stmt_analysis = subshell_stmt_analysis(stmt, parent_id, command_relationships)?;
        analysis.test_count += stmt_analysis.test_count;
        analysis.has_grouping |= stmt_analysis.has_grouping;
    }

    Some(analysis)
}

fn subshell_anchor_span(span: Span, source: &str) -> Span {
    let Some(open_paren_offset) = leading_open_paren_offset(source, span.start.offset) else {
        return span;
    };

    let end_offset = trim_trailing_whitespace_offset(source, span.end.offset);
    Span::from_positions(
        position_at_offset_strict(source, open_paren_offset),
        position_at_offset_strict(source, end_offset),
    )
}

fn leading_open_paren_offset(source: &str, start_offset: usize) -> Option<usize> {
    for (offset, ch) in source[..start_offset].char_indices().rev() {
        if ch.is_whitespace() {
            continue;
        }

        if ch == '(' {
            return Some(offset);
        }

        return None;
    }

    None
}

fn position_at_offset_strict(source: &str, target_offset: usize) -> Position {
    source[..target_offset]
        .chars()
        .fold(Position::new(), |mut position, ch| {
            position.advance(ch);
            position
        })
}

fn trim_trailing_whitespace_offset(source: &str, end_offset: usize) -> usize {
    for (offset, ch) in source[..end_offset].char_indices().rev() {
        if ch.is_whitespace() {
            continue;
        }

        return offset + ch.len_utf8();
    }

    end_offset
}

fn collect_short_circuit_operators(command: &BinaryCommand, operators: &mut Vec<ListOperatorFact>) {
    if let Command::Binary(left) = &command.left.command
        && matches!(left.op, BinaryOp::And | BinaryOp::Or)
    {
        collect_short_circuit_operators(left, operators);
    }

    if matches!(command.op, BinaryOp::And | BinaryOp::Or) {
        operators.push(ListOperatorFact {
            op: command.op,
            span: command.op_span,
        });
    }

    if let Command::Binary(right) = &command.right.command
        && matches!(right.op, BinaryOp::And | BinaryOp::Or)
    {
        collect_short_circuit_operators(right, operators);
    }
}

fn mixed_short_circuit_operator_span(operators: &[ListOperatorFact]) -> Option<Span> {
    let mut previous = operators.first()?;

    for operator in operators.iter().skip(1) {
        if previous.op() != operator.op() {
            return Some(previous.span());
        }

        previous = operator;
    }

    None
}

fn word_contains_find_substitution<'a>(
    word: &'a Word,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    word.parts
        .iter()
        .any(|part| part_contains_find_substitution(&part.kind, commands, command_ids_by_span))
}

fn word_contains_line_oriented_substitution<'a>(
    word: &'a Word,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    word.parts.iter().any(|part| {
        part_contains_line_oriented_substitution(&part.kind, commands, command_ids_by_span)
    })
}

fn word_contains_command_substitution_named<'a>(
    word: &'a Word,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    word.parts.iter().any(|part| {
        part_contains_command_substitution_named(&part.kind, name, commands, command_ids_by_span)
    })
}

fn part_contains_command_substitution_named<'a>(
    part: &WordPart,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts.iter().any(|part| {
            part_contains_command_substitution_named(
                &part.kind,
                name,
                commands,
                command_ids_by_span,
            )
        }),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            substitution_body_is_simple_command_named(body, name, commands, command_ids_by_span)
        }
        _ => false,
    }
}

fn part_contains_line_oriented_substitution<'a>(
    part: &WordPart,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts.iter().any(|part| {
            part_contains_line_oriented_substitution(
                &part.kind,
                commands,
                command_ids_by_span,
            )
        }),
        WordPart::CommandSubstitution { body, .. } => {
            substitution_body_is_line_oriented(body, commands, command_ids_by_span)
        }
        _ => false,
    }
}

fn part_contains_find_substitution<'a>(
    part: &WordPart,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_find_substitution(&part.kind, commands, command_ids_by_span)),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            substitution_body_is_find(body, commands, command_ids_by_span)
        }
        _ => false,
    }
}

fn substitution_body_is_find<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(body.as_slice(), [stmt] if stmt_invokes_find(stmt, commands, command_ids_by_span))
}

fn substitution_body_is_line_oriented<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(
        body.as_slice(),
        [stmt] if command_is_line_oriented_substitution_body(&stmt.command, commands, command_ids_by_span)
    )
}

fn substitution_body_is_pgrep_lookup<'a>(
    body: &'a StmtSeq,
    commands: CommandFacts<'_, 'a>,
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(
        body.as_slice(),
        [stmt]
            if stmt_effective_or_literal_basename_is_ref(stmt, "pgrep", commands, command_ids_by_span)
    )
}

fn substitution_body_is_seq_utility<'a>(
    body: &'a StmtSeq,
    commands: CommandFacts<'_, 'a>,
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(
        body.as_slice(),
        [stmt] if stmt_effective_or_literal_basename_is_ref(stmt, "seq", commands, command_ids_by_span)
    )
}

fn substitution_body_is_simple_command_named<'a>(
    body: &'a StmtSeq,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(body.as_slice(), [stmt] if stmt_literal_name_is(stmt, name, commands, command_ids_by_span))
}

fn command_is_line_oriented_substitution_body<'a>(
    command: &'a Command,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    match command {
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {
            command_fact_for_command(command, commands, command_ids_by_span)
                .is_some_and(command_fact_is_line_oriented_utility)
        }
        Command::Binary(binary) => match binary.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => {
                command_is_line_oriented_substitution_body(
                    &binary.left.command,
                    commands,
                    command_ids_by_span,
                ) && command_is_line_oriented_substitution_body(
                    &binary.right.command,
                    commands,
                    command_ids_by_span,
                )
            }
            BinaryOp::And | BinaryOp::Or => false,
        },
        Command::Compound(CompoundCommand::Time(command)) => command
            .command
            .as_deref()
            .is_some_and(|stmt| {
                command_is_line_oriented_substitution_body(
                    &stmt.command,
                    commands,
                    command_ids_by_span,
                )
            }),
        Command::Compound(
            CompoundCommand::If(_)
            | CompoundCommand::For(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Case(_)
            | CompoundCommand::Select(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Always(_),
        ) => false,
        Command::Function(_) | Command::AnonymousFunction(_) => false,
    }
}

fn command_fact_is_line_oriented_utility(fact: &CommandFact<'_>) -> bool {
    if command_fact_invokes_find(fact) {
        return false;
    }

    fact.effective_or_literal_name().is_some_and(|name| {
        matches!(
            name.rsplit('/').next().unwrap_or(name),
            "cat" | "grep" | "egrep" | "fgrep" | "awk" | "sed" | "cut" | "sort"
        )
    })
}

fn stmt_invokes_find<'a>(
    stmt: &'a Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    command_fact_for_stmt(stmt, commands, command_ids_by_span)
        .is_some_and(command_fact_invokes_find)
}

fn command_fact_invokes_find(fact: &CommandFact<'_>) -> bool {
    command_name_matches_basename(fact.literal_name(), "find")
        || command_name_matches_basename(fact.effective_name(), "find")
        || fact.has_wrapper(WrapperKind::FindExec)
        || fact.has_wrapper(WrapperKind::FindExecDir)
}

fn command_name_matches_basename(name: Option<&str>, expected: &str) -> bool {
    name.is_some_and(|name| name == expected || name.rsplit('/').next() == Some(expected))
}

fn stmt_literal_name_is<'a>(
    stmt: &'a Stmt,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    command_fact_for_stmt(stmt, commands, command_ids_by_span).and_then(CommandFact::literal_name)
        == Some(name)
}

fn stmt_effective_or_literal_basename_is_ref<'a>(
    stmt: &'a Stmt,
    name: &str,
    commands: CommandFacts<'_, 'a>,
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    command_fact_ref_for_stmt(stmt, commands, command_ids_by_span)
        .and_then(CommandFactRef::effective_or_literal_name)
        .is_some_and(|command_name| {
            command_name == name || command_name.rsplit('/').next() == Some(name)
        })
}
