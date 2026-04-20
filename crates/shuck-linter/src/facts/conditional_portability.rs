use rustc_hash::FxHashSet;
use shuck_ast::{ConditionalBinaryOp, ConditionalUnaryOp, Span};

use super::{
    CommandFact, CommandId, ConditionalFact, ConditionalNodeFact, FactSpan, SimpleTestFact,
    SimpleTestSyntax, WordNode, WordOccurrence,
};
use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::span::{
    text_looks_like_caret_negated_bracket, word_caret_negated_bracket_spans,
    word_exactly_one_extglob_span,
};
use crate::{
    conditional_array_subscript_span, conditional_extglob_span, facts::occurrence_word,
    static_word_text, word_array_subscript_span, word_extglob_span,
};

#[derive(Debug, Clone, Default)]
pub struct ConditionalPortabilityFacts {
    double_bracket_in_sh: Vec<Span>,
    test_equality_operator: Vec<Span>,
    if_elif_bash_test: Vec<Span>,
    extglob_in_sh: Vec<Span>,
    caret_negation_in_bracket: Vec<Span>,
    array_subscript_test: Vec<Span>,
    array_subscript_condition: Vec<Span>,
    extglob_in_test: Vec<Span>,
    lexical_comparison_in_double_bracket: Vec<Span>,
    regex_match_in_sh: Vec<Span>,
    v_test_in_sh: Vec<Span>,
    a_test_in_sh: Vec<Span>,
    option_test_in_sh: Vec<Span>,
    sticky_bit_test_in_sh: Vec<Span>,
    ownership_test_in_sh: Vec<Span>,
}

impl ConditionalPortabilityFacts {
    pub fn double_bracket_in_sh(&self) -> &[Span] {
        &self.double_bracket_in_sh
    }

    pub fn test_equality_operator(&self) -> &[Span] {
        &self.test_equality_operator
    }

    pub fn if_elif_bash_test(&self) -> &[Span] {
        &self.if_elif_bash_test
    }

    pub fn extglob_in_sh(&self) -> &[Span] {
        &self.extglob_in_sh
    }

    pub fn caret_negation_in_bracket(&self) -> &[Span] {
        &self.caret_negation_in_bracket
    }

    pub fn array_subscript_test(&self) -> &[Span] {
        &self.array_subscript_test
    }

    pub fn array_subscript_condition(&self) -> &[Span] {
        &self.array_subscript_condition
    }

    pub fn extglob_in_test(&self) -> &[Span] {
        &self.extglob_in_test
    }

    pub fn lexical_comparison_in_double_bracket(&self) -> &[Span] {
        &self.lexical_comparison_in_double_bracket
    }

    pub fn regex_match_in_sh(&self) -> &[Span] {
        &self.regex_match_in_sh
    }

    pub fn v_test_in_sh(&self) -> &[Span] {
        &self.v_test_in_sh
    }

    pub fn a_test_in_sh(&self) -> &[Span] {
        &self.a_test_in_sh
    }

    pub fn option_test_in_sh(&self) -> &[Span] {
        &self.option_test_in_sh
    }

    pub fn sticky_bit_test_in_sh(&self) -> &[Span] {
        &self.sticky_bit_test_in_sh
    }

    pub fn ownership_test_in_sh(&self) -> &[Span] {
        &self.ownership_test_in_sh
    }
}

pub(super) struct ConditionalPortabilityInputs<'a> {
    pub word_nodes: &'a [WordNode<'a>],
    pub word_occurrences: &'a [WordOccurrence],
    pub pattern_exactly_one_extglob_spans: &'a [Span],
    pub pattern_charclass_spans: &'a [Span],
    pub nested_pattern_charclass_spans: &'a FxHashSet<FactSpan>,
}

pub(super) fn build_conditional_portability_facts<'a>(
    commands: &[CommandFact<'a>],
    elif_condition_command_ids: &FxHashSet<CommandId>,
    inputs: ConditionalPortabilityInputs<'a>,
    source: &str,
) -> ConditionalPortabilityFacts {
    let mut facts = ConditionalPortabilityFacts::default();

    for command in commands {
        if let Some(conditional) = command.conditional() {
            facts.double_bracket_in_sh.push(command.span());

            if elif_condition_command_ids.contains(&command.id()) {
                facts.if_elif_bash_test.push(command.span());
            }

            if let Some(span) = conditional_extglob_span(conditional.expression(), source) {
                facts.extglob_in_test.push(span);
            }

            if let Some(span) = conditional_array_subscript_span(conditional.expression(), source) {
                facts.array_subscript_condition.push(span);
            }

            collect_conditional_portability_spans(conditional, source, &mut facts);
        }

        if let Some(simple_test) = command.simple_test() {
            facts.array_subscript_test.extend(
                simple_test
                    .operands()
                    .iter()
                    .filter_map(|word| word_array_subscript_span(word, source)),
            );
            facts.extglob_in_test.extend(
                simple_test
                    .operands()
                    .iter()
                    .filter_map(|word| word_extglob_span(word, source)),
            );
            collect_simple_test_portability_spans(command, simple_test, source, &mut facts);
        }
    }

    facts
        .extglob_in_sh
        .extend(inputs.pattern_exactly_one_extglob_spans.iter().copied());

    facts.caret_negation_in_bracket.extend(
        inputs
            .pattern_charclass_spans
            .iter()
            .filter(|span| {
                !inputs
                    .nested_pattern_charclass_spans
                    .contains(&FactSpan::new(**span))
            })
            .filter(|span| text_looks_like_caret_negated_bracket(span.slice(source)))
            .copied(),
    );

    for fact in inputs.word_occurrences {
        let expansion_context = match fact.context {
            super::WordFactContext::Expansion(context) => Some(context),
            super::WordFactContext::CaseSubject | super::WordFactContext::ArithmeticCommand => None,
        };
        let word = occurrence_word(inputs.word_nodes, fact);
        if supports_extglob_portability_context(expansion_context)
            && let Some(span) = word_exactly_one_extglob_span(word, source)
        {
            facts.extglob_in_sh.push(span);
        }

        if supports_bracket_glob_portability_context(expansion_context) {
            facts
                .caret_negation_in_bracket
                .extend(word_caret_negated_bracket_spans(word, source));
        }
    }

    facts
}

fn collect_conditional_portability_spans(
    conditional: &ConditionalFact<'_>,
    source: &str,
    facts: &mut ConditionalPortabilityFacts,
) {
    for node in conditional.nodes() {
        match node {
            ConditionalNodeFact::Binary(binary) => match binary.op() {
                ConditionalBinaryOp::PatternEq => {
                    facts.test_equality_operator.push(binary.operator_span());
                }
                ConditionalBinaryOp::LexicalBefore | ConditionalBinaryOp::LexicalAfter => {
                    facts
                        .lexical_comparison_in_double_bracket
                        .push(binary.operator_span());
                }
                ConditionalBinaryOp::RegexMatch => {
                    facts.regex_match_in_sh.push(binary.operator_span());
                }
                _ => {}
            },
            ConditionalNodeFact::Unary(unary) => match unary.op() {
                ConditionalUnaryOp::VariableSet => {
                    facts.v_test_in_sh.push(unary.operator_span());
                }
                ConditionalUnaryOp::Exists if unary.operator_span().slice(source) == "-a" => {
                    facts.a_test_in_sh.push(unary.operator_span());
                }
                ConditionalUnaryOp::OptionSet => {
                    facts.option_test_in_sh.push(unary.operator_span());
                }
                _ => {}
            },
            ConditionalNodeFact::BareWord(_) | ConditionalNodeFact::Other(_) => {}
        }
    }
}

fn collect_simple_test_portability_spans(
    command: &CommandFact<'_>,
    fact: &SimpleTestFact<'_>,
    source: &str,
    facts: &mut ConditionalPortabilityFacts,
) {
    let operands = fact.operands();
    let operand_texts = operands
        .iter()
        .map(|word| static_word_text(word, source))
        .collect::<Vec<_>>();

    let mut has_eqeq = false;
    let mut has_sticky_bit = false;
    let mut has_ownership = false;
    let mut index = 0usize;

    while index < operands.len() {
        while index < operands.len() && is_simple_test_separator(operand_texts[index].as_deref()) {
            index += 1;
        }
        while index < operands.len() && operand_texts[index].as_deref() == Some("!") {
            index += 1;
        }

        if index >= operands.len() {
            break;
        }

        if index + 2 < operands.len()
            && operand_texts[index + 1]
                .as_deref()
                .is_some_and(is_simple_test_binary_operator)
        {
            if operand_texts[index + 1].as_deref() == Some("==") {
                match fact.syntax() {
                    SimpleTestSyntax::Test => has_eqeq = true,
                    SimpleTestSyntax::Bracket => {
                        facts.test_equality_operator.push(operands[index + 1].span);
                    }
                }
            }
            index += 3;
            continue;
        }

        if index + 1 < operands.len()
            && operand_texts[index]
                .as_deref()
                .is_some_and(is_simple_test_unary_operator)
        {
            match operand_texts[index].as_deref() {
                Some("-k") => match fact.syntax() {
                    SimpleTestSyntax::Test => has_sticky_bit = true,
                    SimpleTestSyntax::Bracket => {
                        facts.sticky_bit_test_in_sh.push(operands[index].span);
                    }
                },
                Some("-O") => match fact.syntax() {
                    SimpleTestSyntax::Test => has_ownership = true,
                    SimpleTestSyntax::Bracket => {
                        facts.ownership_test_in_sh.push(operands[index].span);
                    }
                },
                _ => {}
            }

            index += 2;
            continue;
        }

        index += 1;
    }

    if fact.syntax() == SimpleTestSyntax::Test {
        let Some(command_span) = simple_test_command_span(command, fact) else {
            return;
        };

        if has_eqeq {
            facts.test_equality_operator.push(command_span);
        }
        if has_sticky_bit {
            facts.sticky_bit_test_in_sh.push(command_span);
        }
        if has_ownership {
            facts.ownership_test_in_sh.push(command_span);
        }
    }
}

fn supports_extglob_portability_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(
            ExpansionContext::CommandName
                | ExpansionContext::CommandArgument
                | ExpansionContext::ForList
                | ExpansionContext::SelectList
        )
    )
}

fn supports_bracket_glob_portability_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(
            ExpansionContext::CommandArgument
                | ExpansionContext::ForList
                | ExpansionContext::SelectList
        )
    )
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

fn simple_test_command_span(command: &CommandFact<'_>, fact: &SimpleTestFact<'_>) -> Option<Span> {
    let name = command.body_name_word()?;
    let end = fact
        .operands()
        .last()
        .map(|word| word.span.end)
        .unwrap_or(name.span.end);
    Some(Span::from_positions(name.span.start, end))
}
