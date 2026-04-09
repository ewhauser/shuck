use shuck_ast::{ConditionalBinaryOp, ConditionalUnaryOp, Span};

use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::span::{
    conditional_exactly_one_extglob_span, text_looks_like_caret_negated_bracket,
    word_caret_negated_bracket_spans, word_exactly_one_extglob_span,
};
use crate::{
    Checker, ConditionalNodeFact, Rule, ShellDialect, SimpleTestFact, SimpleTestSyntax, Violation,
    conditional_array_subscript_span, conditional_extglob_span, static_word_text,
    word_array_subscript_span, word_extglob_span,
};

pub struct DoubleBracketInSh;
pub struct TestEqualityOperator;
pub struct IfElifBashTest;
pub struct ExtendedGlobInTest;
pub struct ExtglobInSh;
pub struct CaretNegationInBracket;
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

impl Violation for ExtglobInSh {
    fn rule() -> Rule {
        Rule::ExtglobInSh
    }

    fn message(&self) -> String {
        "extended glob syntax is not available in POSIX sh".to_owned()
    }
}

impl Violation for CaretNegationInBracket {
    fn rule() -> Rule {
        Rule::CaretNegationInBracket
    }

    fn message(&self) -> String {
        "caret negation in bracket expressions is not portable to POSIX sh".to_owned()
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
        .filter(|fact| fact.conditional().is_some())
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
            .filter_map(|fact| fact.conditional())
            .flat_map(|conditional| {
                conditional_binary_operator_spans(conditional, ConditionalBinaryOp::PatternEq)
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
        .filter(|fact| fact.conditional().is_some())
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
        .filter_map(|fact| fact.conditional())
        .filter_map(|conditional| conditional_extglob_span(conditional.expression(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExtendedGlobInTest);
}

pub fn extglob_in_sh(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let mut spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| fact.expansion_context() != Some(ExpansionContext::CasePattern))
        .filter_map(|fact| word_exactly_one_extglob_span(fact.word(), source))
        .collect::<Vec<_>>();
    spans.extend(
        checker
            .facts()
            .commands()
            .iter()
            .filter_map(|fact| fact.conditional())
            .filter_map(|conditional| {
                conditional_exactly_one_extglob_span(conditional.expression(), source)
            }),
    );

    checker.report_all_dedup(spans, || ExtglobInSh);
}

pub fn caret_negation_in_bracket(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let source = checker.source();
    let mut spans = checker
        .facts()
        .pattern_charclass_spans()
        .iter()
        .filter(|span| text_looks_like_caret_negated_bracket(span.slice(source)))
        .copied()
        .collect::<Vec<_>>();
    spans.extend(
        checker
            .facts()
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && !fact.is_nested_word_command()
            })
            .flat_map(|fact| word_caret_negated_bracket_spans(fact.word(), source)),
    );

    checker.report_all_dedup(spans, || CaretNegationInBracket);
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
        .filter_map(|fact| fact.conditional())
        .filter_map(|conditional| {
            conditional_array_subscript_span(conditional.expression(), source)
        })
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
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional_binary_operator_spans(conditional, ConditionalBinaryOp::LexicalAfter)
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
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional_binary_operator_spans(conditional, ConditionalBinaryOp::RegexMatch)
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
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional_unary_operator_spans(conditional, |operator, _| {
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
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional_unary_operator_spans(conditional, |operator, op_span| {
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
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional_unary_operator_spans(conditional, |operator, _| {
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

fn conditional_binary_operator_spans(
    conditional: &crate::ConditionalFact<'_>,
    operator: ConditionalBinaryOp,
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary) if binary.op() == operator => {
                Some(binary.operator_span())
            }
            _ => None,
        })
        .collect()
}

fn conditional_unary_operator_spans(
    conditional: &crate::ConditionalFact<'_>,
    predicate: impl Fn(ConditionalUnaryOp, Span) -> bool + Copy,
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary) if predicate(unary.op(), unary.operator_span()) => {
                Some(unary.operator_span())
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SimpleTestOperatorKind {
    Unary,
    Binary,
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_at_extglob_in_posix_shells() {
        let source = "#!/bin/sh\necho @(foo|bar)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
    }

    #[test]
    fn reports_at_extglob_in_conditional_patterns_in_posix_shells() {
        let source = "#!/bin/sh\n[[ $OSTYPE == *@(linux|freebsd)* ]] || exit 1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
        assert_eq!(diagnostics[0].span.slice(source), "@(linux|freebsd)");
    }

    #[test]
    fn reports_caret_negation_in_bracket_in_posix_shells() {
        let source = "\
#!/bin/sh
echo [^a]*
case x in
  [^a]*) : ;;
esac
[[ $x = [^a]* ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert_eq!(diagnostics.len(), 3);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule == Rule::CaretNegationInBracket)
        );
    }

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
                Rule::ExtglobInSh,
                Rule::CaretNegationInBracket,
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
