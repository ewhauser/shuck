use shuck_ast::{
    BourneParameterExpansion, Command, CompoundCommand, ConditionalBinaryOp, ConditionalCommand,
    ConditionalExpr, ConditionalUnaryOp, ParameterExpansion, ParameterExpansionSyntax, Pattern,
    PatternPart, Span, VarRef, Word, WordPart, WordPartNode, ZshExpansionTarget,
};

use crate::{
    Checker, Rule, ShellDialect, SimpleTestFact, SimpleTestSyntax, Violation, static_word_text,
};

pub struct DoubleBracketInSh;
pub struct TestEqualityOperator;
pub struct IfElifBashTest;
pub struct ExtendedGlobInTest;
pub struct ArraySubscriptTest;
pub struct ArraySubscriptCondition;
pub struct ExtglobInTest;
pub struct GreaterThanInDoubleBracket;
pub struct RegexMatchInSh;
pub struct VTestInSh;
pub struct ATestInSh;
pub struct OptionTestInSh;
pub struct StickyBitTestInSh;
pub struct OwnershipTestInSh;

impl Violation for DoubleBracketInSh {
    fn rule() -> Rule {
        Rule::DoubleBracketInSh
    }

    fn message(&self) -> String {
        "`[[ ... ]]` is not available in POSIX sh".to_owned()
    }
}

impl Violation for TestEqualityOperator {
    fn rule() -> Rule {
        Rule::TestEqualityOperator
    }

    fn message(&self) -> String {
        "use `=` instead of `==` in POSIX test expressions".to_owned()
    }
}

impl Violation for IfElifBashTest {
    fn rule() -> Rule {
        Rule::IfElifBashTest
    }

    fn message(&self) -> String {
        "`elif` uses `[[ ... ]]`, which is not available in POSIX sh".to_owned()
    }
}

impl Violation for ExtendedGlobInTest {
    fn rule() -> Rule {
        Rule::ExtendedGlobInTest
    }

    fn message(&self) -> String {
        "extended glob patterns in `[[` matches are not portable to POSIX sh".to_owned()
    }
}

impl Violation for ArraySubscriptTest {
    fn rule() -> Rule {
        Rule::ArraySubscriptTest
    }

    fn message(&self) -> String {
        "array-style subscripts in test expressions are not portable to POSIX sh".to_owned()
    }
}

impl Violation for ArraySubscriptCondition {
    fn rule() -> Rule {
        Rule::ArraySubscriptCondition
    }

    fn message(&self) -> String {
        "array-style subscripts in `[[ ... ]]` are not portable to POSIX sh".to_owned()
    }
}

impl Violation for ExtglobInTest {
    fn rule() -> Rule {
        Rule::ExtglobInTest
    }

    fn message(&self) -> String {
        "extended glob syntax in test operands is not portable to POSIX sh".to_owned()
    }
}

impl Violation for GreaterThanInDoubleBracket {
    fn rule() -> Rule {
        Rule::GreaterThanInDoubleBracket
    }

    fn message(&self) -> String {
        "`>` inside `[[ ... ]]` is not a POSIX sh test operator".to_owned()
    }
}

impl Violation for RegexMatchInSh {
    fn rule() -> Rule {
        Rule::RegexMatchInSh
    }

    fn message(&self) -> String {
        "`=~` regex matching is not available in POSIX sh".to_owned()
    }
}

impl Violation for VTestInSh {
    fn rule() -> Rule {
        Rule::VTestInSh
    }

    fn message(&self) -> String {
        "`-v` tests are not available in POSIX sh".to_owned()
    }
}

impl Violation for ATestInSh {
    fn rule() -> Rule {
        Rule::ATestInSh
    }

    fn message(&self) -> String {
        "use `-e` instead of `-a` for file-existence checks in POSIX sh".to_owned()
    }
}

impl Violation for OptionTestInSh {
    fn rule() -> Rule {
        Rule::OptionTestInSh
    }

    fn message(&self) -> String {
        "`-o` option tests are not available in POSIX sh".to_owned()
    }
}

impl Violation for StickyBitTestInSh {
    fn rule() -> Rule {
        Rule::StickyBitTestInSh
    }

    fn message(&self) -> String {
        "`-k` file tests are not portable to POSIX sh".to_owned()
    }
}

impl Violation for OwnershipTestInSh {
    fn rule() -> Rule {
        Rule::OwnershipTestInSh
    }

    fn message(&self) -> String {
        "`-O` file tests are not portable to POSIX sh".to_owned()
    }
}

pub fn double_bracket_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| conditional_command(fact).is_some())
        .map(crate::CommandFact::span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || DoubleBracketInSh);
}

pub fn test_equality_operator(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let mut spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let Some(simple_test) = fact.simple_test() else {
                return Vec::new();
            };

            match simple_test.syntax() {
                SimpleTestSyntax::Test => {
                    (!simple_test_binary_operator_token_spans(simple_test, source, "==").is_empty())
                        .then(|| simple_test_command_span(fact, simple_test))
                        .flatten()
                        .into_iter()
                        .collect()
                }
                SimpleTestSyntax::Bracket => {
                    simple_test_binary_operator_token_spans(simple_test, source, "==")
                }
            }
        })
        .collect::<Vec<_>>();
    spans.extend(
        checker
            .facts()
            .commands()
            .iter()
            .filter_map(conditional_command)
            .flat_map(|command| {
                conditional_binary_operator_spans(
                    &command.expression,
                    ConditionalBinaryOp::PatternEq,
                )
            }),
    );

    checker.report_all_dedup(spans, || TestEqualityOperator);
}

pub fn if_elif_bash_test(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| checker.facts().is_elif_condition_command(fact.id()))
        .filter(|fact| conditional_command(fact).is_some())
        .map(crate::CommandFact::span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || IfElifBashTest);
}

pub fn extended_glob_in_test(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .filter_map(|command| conditional_extglob_span(&command.expression, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExtendedGlobInTest);
}

pub fn array_subscript_test(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.simple_test())
        .flat_map(|fact| {
            fact.operands()
                .iter()
                .filter_map(|word| word_array_subscript_span(word, source))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArraySubscriptTest);
}

pub fn array_subscript_condition(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .filter_map(|command| conditional_array_subscript_span(&command.expression, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArraySubscriptCondition);
}

pub fn extglob_in_test(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.simple_test())
        .flat_map(|fact| {
            fact.operands()
                .iter()
                .filter_map(|word| word_extglob_span(word, source))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExtglobInTest);
}

pub fn greater_than_in_double_bracket(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .flat_map(|command| {
            conditional_binary_operator_spans(
                &command.expression,
                ConditionalBinaryOp::LexicalAfter,
            )
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GreaterThanInDoubleBracket);
}

pub fn regex_match_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .flat_map(|command| {
            conditional_binary_operator_spans(&command.expression, ConditionalBinaryOp::RegexMatch)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || RegexMatchInSh);
}

pub fn v_test_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .flat_map(|command| {
            conditional_unary_operator_spans(&command.expression, |operator, _| {
                operator == ConditionalUnaryOp::VariableSet
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || VTestInSh);
}

pub fn a_test_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .flat_map(|command| {
            conditional_unary_operator_spans(&command.expression, |operator, op_span| {
                operator == ConditionalUnaryOp::Exists && op_span.slice(source) == "-a"
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ATestInSh);
}

pub fn option_test_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(conditional_command)
        .flat_map(|command| {
            conditional_unary_operator_spans(&command.expression, |operator, _| {
                operator == ConditionalUnaryOp::OptionSet
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || OptionTestInSh);
}

pub fn sticky_bit_test_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let Some(simple_test) = fact.simple_test() else {
                return Vec::new();
            };

            simple_test_flag_spans(fact, simple_test, source, "-k")
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || StickyBitTestInSh);
}

pub fn ownership_test_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let Some(simple_test) = fact.simple_test() else {
                return Vec::new();
            };

            simple_test_flag_spans(fact, simple_test, source, "-O")
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || OwnershipTestInSh);
}

fn is_posix_sh_shell(shell: ShellDialect) -> bool {
    matches!(shell, ShellDialect::Sh | ShellDialect::Dash)
}

fn conditional_command<'a>(fact: &'a crate::CommandFact<'a>) -> Option<&'a ConditionalCommand> {
    let Command::Compound(CompoundCommand::Conditional(command)) = fact.command() else {
        return None;
    };

    Some(command)
}

fn simple_test_binary_operator_token_spans(
    fact: &SimpleTestFact<'_>,
    source: &str,
    token: &str,
) -> Vec<Span> {
    simple_test_operator_token_spans(fact, source, token, SimpleTestOperatorKind::Binary)
}

fn simple_test_unary_operator_token_spans(
    fact: &SimpleTestFact<'_>,
    source: &str,
    token: &str,
) -> Vec<Span> {
    simple_test_operator_token_spans(fact, source, token, SimpleTestOperatorKind::Unary)
}

fn simple_test_operator_token_spans(
    fact: &SimpleTestFact<'_>,
    source: &str,
    token: &str,
    kind: SimpleTestOperatorKind,
) -> Vec<Span> {
    let operands = fact.operands();
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < operands.len() {
        while index < operands.len()
            && is_simple_test_separator(static_word_text(operands[index], source).as_deref())
        {
            index += 1;
        }
        while index < operands.len()
            && static_word_text(operands[index], source).as_deref() == Some("!")
        {
            index += 1;
        }

        if index >= operands.len() {
            break;
        }

        if index + 2 < operands.len()
            && static_word_text(operands[index + 1], source)
                .as_deref()
                .is_some_and(is_simple_test_binary_operator)
        {
            if kind == SimpleTestOperatorKind::Binary
                && static_word_text(operands[index + 1], source).as_deref() == Some(token)
            {
                spans.push(operands[index + 1].span);
            }
            index += 3;
            continue;
        }

        if index + 1 < operands.len()
            && static_word_text(operands[index], source)
                .as_deref()
                .is_some_and(is_simple_test_unary_operator)
        {
            if kind == SimpleTestOperatorKind::Unary
                && static_word_text(operands[index], source).as_deref() == Some(token)
            {
                spans.push(operands[index].span);
            }
            index += 2;
            continue;
        }

        index += 1;
    }

    spans
}

fn is_simple_test_separator(token: Option<&str>) -> bool {
    matches!(token, Some("-a" | "-o" | "(" | ")" | "\\(" | "\\)"))
}

fn is_simple_test_binary_operator(token: &str) -> bool {
    matches!(
        token,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-ef"
            | "-nt"
            | "-ot"
    )
}

fn is_simple_test_unary_operator(token: &str) -> bool {
    matches!(
        token,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-h"
            | "-k"
            | "-L"
            | "-n"
            | "-N"
            | "-O"
            | "-p"
            | "-r"
            | "-s"
            | "-S"
            | "-t"
            | "-u"
            | "-v"
            | "-w"
            | "-x"
            | "-z"
    )
}

fn simple_test_flag_spans(
    command: &crate::CommandFact<'_>,
    fact: &SimpleTestFact<'_>,
    source: &str,
    token: &str,
) -> Vec<Span> {
    match fact.syntax() {
        SimpleTestSyntax::Test => (!simple_test_unary_operator_token_spans(fact, source, token)
            .is_empty())
        .then(|| simple_test_command_span(command, fact))
        .flatten()
        .into_iter()
        .collect::<Vec<_>>(),
        SimpleTestSyntax::Bracket => simple_test_unary_operator_token_spans(fact, source, token),
    }
}

fn simple_test_command_span(
    command: &crate::CommandFact<'_>,
    fact: &SimpleTestFact<'_>,
) -> Option<Span> {
    let name = command.body_name_word()?;
    let end = fact
        .operands()
        .last()
        .map(|word| word.span.end)
        .unwrap_or(name.span.end);
    Some(Span::from_positions(name.span.start, end))
}

fn conditional_extglob_span(expression: &ConditionalExpr, source: &str) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expr) => conditional_extglob_span(&expr.left, source)
            .or_else(|| conditional_extglob_span(&expr.right, source)),
        ConditionalExpr::Unary(expr) => conditional_extglob_span(&expr.expr, source),
        ConditionalExpr::Parenthesized(expr) => conditional_extglob_span(&expr.expr, source),
        ConditionalExpr::Pattern(pattern) => pattern_extglob_span(pattern, source),
        ConditionalExpr::Word(_) | ConditionalExpr::Regex(_) | ConditionalExpr::VarRef(_) => None,
    }
}

fn pattern_extglob_span(pattern: &Pattern, source: &str) -> Option<Span> {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Group { patterns, .. } => {
                return Some(part.span).or_else(|| {
                    patterns
                        .iter()
                        .find_map(|pattern| pattern_extglob_span(pattern, source))
                });
            }
            PatternPart::Word(word) => {
                if let Some(span) = word_extglob_span(word, source) {
                    return Some(span);
                }
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    None
}

fn conditional_array_subscript_span(expression: &ConditionalExpr, source: &str) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expr) => conditional_array_subscript_span(&expr.left, source)
            .or_else(|| conditional_array_subscript_span(&expr.right, source)),
        ConditionalExpr::Unary(expr) => conditional_array_subscript_span(&expr.expr, source),
        ConditionalExpr::Parenthesized(expr) => {
            conditional_array_subscript_span(&expr.expr, source)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            word_array_subscript_span(word, source)
        }
        ConditionalExpr::Pattern(pattern) => pattern_array_subscript_span(pattern, source),
        ConditionalExpr::VarRef(reference) => var_ref_subscript_span(reference),
    }
}

fn pattern_array_subscript_span(pattern: &Pattern, source: &str) -> Option<Span> {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Group { patterns, .. } => {
                if let Some(span) = patterns
                    .iter()
                    .find_map(|pattern| pattern_array_subscript_span(pattern, source))
                {
                    return Some(span);
                }
            }
            PatternPart::Word(word) => {
                if let Some(span) = word_array_subscript_span(word, source) {
                    return Some(span);
                }
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    None
}

fn word_array_subscript_span(word: &Word, source: &str) -> Option<Span> {
    word_array_subscript_span_from_parts(&word.parts, source).or_else(|| {
        (!word.has_quoted_parts() && text_has_variable_subscript(word.span.slice(source)))
            .then_some(word.span)
    })
}

fn word_array_subscript_span_from_parts(parts: &[WordPartNode], source: &str) -> Option<Span> {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if let Some(span) = word_array_subscript_span_from_parts(parts, source) {
                    return Some(span);
                }
            }
            WordPart::Literal(_) => {
                if text_has_variable_subscript(part.span.slice(source)) {
                    return Some(part.span);
                }
            }
            WordPart::Parameter(parameter) => {
                if let Some(span) = parameter_array_subscript_span(parameter) {
                    return Some(span);
                }
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Substring { reference, .. }
            | WordPart::ArraySlice { reference, .. }
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                if let Some(span) = var_ref_subscript_span(reference) {
                    return Some(span);
                }
            }
            WordPart::ZshQualifiedGlob(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. } => {}
        }
    }

    None
}

fn parameter_array_subscript_span(parameter: &ParameterExpansion) -> Option<Span> {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Indirect { reference, .. }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                var_ref_subscript_span(reference)
            }
            BourneParameterExpansion::PrefixMatch { .. } => None,
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => var_ref_subscript_span(reference),
            ZshExpansionTarget::Nested(parameter) => parameter_array_subscript_span(parameter),
            ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => None,
        },
    }
}

fn var_ref_subscript_span(reference: &VarRef) -> Option<Span> {
    reference
        .subscript
        .as_ref()
        .filter(|subscript| subscript.selector().is_none())
        .map(|_| reference.span)
}

fn word_extglob_span(word: &Word, source: &str) -> Option<Span> {
    word_extglob_span_from_parts(&word.parts, source).or_else(|| {
        (!word.has_quoted_parts()
            && word_has_only_literal_parts(&word.parts)
            && text_looks_like_extglob(word.span.slice(source)))
        .then_some(word.span)
    })
}

fn word_extglob_span_from_parts(parts: &[WordPartNode], source: &str) -> Option<Span> {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {
                if text_looks_like_extglob(part.span.slice(source)) {
                    return Some(part.span);
                }
            }
            WordPart::DoubleQuoted { .. } | WordPart::SingleQuoted { .. } => {}
            WordPart::ZshQualifiedGlob(_)
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
        }
    }

    None
}

fn word_has_only_literal_parts(parts: &[WordPartNode]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part.kind, WordPart::Literal(_)))
}

fn text_has_variable_subscript(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'$' || byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        let next = index + 1;
        if next >= bytes.len() {
            break;
        }

        if bytes[next] == b'{' {
            let mut cursor = next + 1;
            while cursor < bytes.len() && bytes[cursor] != b'}' {
                if bytes[cursor] == b'['
                    && bytes[cursor + 1..].contains(&b']')
                    && bytes[cursor + 1..].contains(&b'}')
                {
                    return true;
                }
                cursor += 1;
            }
            index = cursor.saturating_add(1);
            continue;
        }

        if !is_name_start(bytes[next]) {
            index += 1;
            continue;
        }

        let mut cursor = next + 1;
        while cursor < bytes.len() && is_name_continue(bytes[cursor]) {
            cursor += 1;
        }

        if cursor < bytes.len() && bytes[cursor] == b'[' && bytes[cursor + 1..].contains(&b']') {
            return true;
        }

        index = cursor;
    }

    false
}

fn text_looks_like_extglob(text: &str) -> bool {
    let bytes = text.as_bytes();
    if text_has_parenthesized_alternation(bytes) {
        return true;
    }

    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if !is_extglob_operator(bytes[index])
            || bytes[index + 1] != b'('
            || byte_is_backslash_escaped(bytes, index)
        {
            index += 1;
            continue;
        }

        return matching_group_end(bytes, index + 1).is_some();
    }

    false
}

fn text_has_parenthesized_alternation(bytes: &[u8]) -> bool {
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'(' || byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        let Some(close) = matching_group_end(bytes, index) else {
            index += 1;
            continue;
        };

        if bytes[index + 1..close]
            .iter()
            .enumerate()
            .any(|(offset, byte)| {
                *byte == b'|' && !byte_is_backslash_escaped(bytes, index + 1 + offset)
            })
        {
            return true;
        }

        index = close + 1;
    }

    false
}

fn matching_group_end(bytes: &[u8], open_index: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut cursor = open_index + 1;

    while cursor < bytes.len() {
        if byte_is_backslash_escaped(bytes, cursor) {
            cursor += 1;
            continue;
        }

        match bytes[cursor] {
            b'(' => {
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => {}
        }

        cursor += 1;
    }

    None
}

fn byte_is_backslash_escaped(bytes: &[u8], index: usize) -> bool {
    let mut cursor = index;
    let mut backslashes = 0usize;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }

    backslashes % 2 == 1
}

fn is_extglob_operator(byte: u8) -> bool {
    matches!(byte, b'@' | b'?' | b'+' | b'*' | b'!')
}

fn conditional_binary_operator_spans(
    expression: &ConditionalExpr,
    operator: ConditionalBinaryOp,
) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_conditional_binary_operator_spans(expression, operator, &mut spans);
    spans
}

fn collect_conditional_binary_operator_spans(
    expression: &ConditionalExpr,
    operator: ConditionalBinaryOp,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            if expr.op == operator {
                spans.push(expr.op_span);
            }
            collect_conditional_binary_operator_spans(&expr.left, operator, spans);
            collect_conditional_binary_operator_spans(&expr.right, operator, spans);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_binary_operator_spans(&expr.expr, operator, spans);
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_binary_operator_spans(&expr.expr, operator, spans);
        }
        ConditionalExpr::Word(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::VarRef(_) => {}
    }
}

fn conditional_unary_operator_spans(
    expression: &ConditionalExpr,
    predicate: impl Fn(ConditionalUnaryOp, Span) -> bool + Copy,
) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_conditional_unary_operator_spans(expression, predicate, &mut spans);
    spans
}

fn collect_conditional_unary_operator_spans(
    expression: &ConditionalExpr,
    predicate: impl Fn(ConditionalUnaryOp, Span) -> bool + Copy,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_unary_operator_spans(&expr.left, predicate, spans);
            collect_conditional_unary_operator_spans(&expr.right, predicate, spans);
        }
        ConditionalExpr::Unary(expr) => {
            if predicate(expr.op, expr.op_span) {
                spans.push(expr.op_span);
            }
            collect_conditional_unary_operator_spans(&expr.expr, predicate, spans);
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_unary_operator_spans(&expr.expr, predicate, spans);
        }
        ConditionalExpr::Word(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::VarRef(_) => {}
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SimpleTestOperatorKind {
    Unary,
    Binary,
}

fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_name_continue(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn sh_portability_rules_ignore_bash_shells() {
        let source = "\
#!/bin/bash
if [[ -v assoc[$key] && $term == @(foo|bar) && $# > 1 ]]; then
  :
fi
[ \"$1\" == foo ]
[ -k \"$file\" ]
[ \"$x\" = (foo|bar)* ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::DoubleBracketInSh,
                Rule::TestEqualityOperator,
                Rule::IfElifBashTest,
                Rule::ExtendedGlobInTest,
                Rule::ArraySubscriptTest,
                Rule::ArraySubscriptCondition,
                Rule::ExtglobInTest,
                Rule::GreaterThanInDoubleBracket,
                Rule::RegexMatchInSh,
                Rule::VTestInSh,
                Rule::ATestInSh,
                Rule::OptionTestInSh,
                Rule::StickyBitTestInSh,
                Rule::OwnershipTestInSh,
            ])
            .with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
