//! Linter-owned structural facts built once per file.
//!
//! `SemanticModel` remains the source of truth for bindings, references, scopes,
//! source references, the call graph, and flow-sensitive facts.
//! `LinterFacts` owns reusable linter-side summaries that are cheaper to build
//! once than to recompute in every rule: normalized commands, wrapper chains,
//! declaration summaries, option-shape summaries, and later word/expansion
//! facts.

mod command_flow;
mod conditional_portability;
mod escape_scan;
mod presence;
mod surface;

use self::{
    command_flow::{
        build_case_item_facts, build_for_header_facts, build_list_facts, build_pipeline_facts,
        build_select_header_facts, build_single_test_subshell_spans, build_statement_facts,
        build_subshell_test_group_spans, build_substitution_facts,
    },
    conditional_portability::build_conditional_portability_facts,
    escape_scan::{EscapeScanContext, EscapeScanMatch, build_escape_scan_matches},
    presence::build_presence_tested_names,
    surface::{
        SurfaceFragmentFacts, SurfaceFragmentSink, SurfaceScanContext,
        build_subscript_index_reference_spans,
    },
};
use crate::FileContext;
use crate::context::ContextRegionKind;
use crate::rules::common::expansion::{
    ExpansionAnalysis, ExpansionContext, RedirectTargetAnalysis, RuntimeLiteralAnalysis,
    SubstitutionOutputIntent, WordExpansionKind, WordLiteralness, WordSubstitutionShape,
    analyze_literal_runtime, analyze_redirect_target, analyze_word,
};
use crate::rules::common::span::expansion_part_spans;
use crate::rules::common::{
    command::{self, NormalizedCommand, WrapperKind},
    query::{self, CommandSubstitutionKind, CommandVisit, CommandWalkOptions},
    span,
    word::{
        TestOperandClass, WordClassification, WordQuote, classify_conditional_operand,
        classify_contextual_operand, classify_word, static_word_text,
    },
};
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue,
    ArithmeticPostfixOp, ArithmeticUnaryOp, ArrayElem, ArrayKind, Assignment, AssignmentValue,
    BinaryCommand, BinaryOp, BourneParameterExpansion, BraceQuoteContext, BraceSyntaxKind,
    BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command, CommandSubstitutionSyntax,
    CompoundCommand, ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, DeclClause,
    DeclOperand, File, ForCommand, FunctionDef, IfCommand, Name, ParameterExpansion,
    ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart, Position, Redirect, RedirectKind,
    SelectCommand, SimpleCommand, SourceText, Span, Stmt, StmtSeq, StmtTerminator, Subscript,
    TextRange, VarRef, WhileCommand, Word, WordPart, WordPartNode, ZshExpansionTarget,
    ZshGlobSegment, ZshQualifiedGlob,
};
use shuck_indexer::Indexer;
use shuck_semantic::{
    BindingAttributes, BindingId, BindingKind, ScopeId, SemanticModel, ZshOptionState,
};
use std::borrow::Cow;

pub use self::conditional_portability::ConditionalPortabilityFacts;
pub(crate) use self::escape_scan::EscapeScanSourceKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FactSpan {
    start: usize,
    end: usize,
}

impl FactSpan {
    pub fn new(span: Span) -> Self {
        Self {
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

impl From<Span> for FactSpan {
    fn from(span: Span) -> Self {
        Self::new(span)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(usize);

impl CommandId {
    fn new(index: usize) -> Self {
        Self(index)
    }

    fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CommandLookupKind {
    Simple,
    Builtin(BuiltinLookupKind),
    Decl,
    Binary,
    Compound(CompoundLookupKind),
    Function,
    AnonymousFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BuiltinLookupKind {
    Break,
    Continue,
    Return,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CompoundLookupKind {
    If,
    For,
    Repeat,
    Foreach,
    ArithmeticFor,
    While,
    Until,
    Case,
    Select,
    Subshell,
    BraceGroup,
    Arithmetic,
    Time,
    Conditional,
    Coproc,
    Always,
}

#[derive(Debug, Clone, Copy)]
struct CommandLookupEntry {
    kind: CommandLookupKind,
    id: CommandId,
}

type CommandLookupIndex = FxHashMap<FactSpan, Vec<CommandLookupEntry>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SudoFamilyInvoker {
    Sudo,
    Doas,
    Run0,
}

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

    pub fn compound_operator_spans(&self, source: &str) -> Vec<Span> {
        match self.effective_shape {
            SimpleTestShape::Binary => {
                return self
                    .effective_operator_word()
                    .into_iter()
                    .filter(|word| {
                        self.effective_operand_class(1)
                            .is_some_and(|class| class.is_fixed_literal())
                            && classify_word(word, source).quote == WordQuote::Unquoted
                    })
                    .filter_map(|word| {
                        static_word_text(word, source)
                            .is_some_and(|text| matches!(text.as_str(), "-a" | "-o"))
                            .then_some(word.span)
                    })
                    .collect();
            }
            SimpleTestShape::Other => {}
            SimpleTestShape::Empty | SimpleTestShape::Truthy | SimpleTestShape::Unary => {
                return Vec::new();
            }
        }

        self.effective_operands()
            .iter()
            .enumerate()
            .filter(|(index, _word)| {
                self.effective_operand_class(*index)
                    .is_some_and(|class| class.is_fixed_literal())
            })
            .map(|(_index, word)| (word, classify_word(word, source)))
            .filter_map(|(word, classification)| {
                (classification.quote == WordQuote::Unquoted)
                    .then_some(word)
                    .and_then(|word| {
                        static_word_text(word, source)
                            .is_some_and(|text| matches!(text.as_str(), "-a" | "-o"))
                            .then_some(word.span)
                    })
            })
            .collect()
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

    static_word_text(word, source)
}

fn simple_test_is_logical_connector(text: &str) -> bool {
    matches!(text, "-a" | "-o")
}

fn simple_test_is_string_unary_operator(text: &str) -> bool {
    matches!(text, "-n" | "-z")
}

fn simple_test_is_string_binary_operator(text: &str) -> bool {
    matches!(text, "=" | "==" | "!=" | "<" | ">")
}

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

#[derive(Debug, Clone)]
pub struct ConditionalFact<'a> {
    nodes: Box<[ConditionalNodeFact<'a>]>,
    mixed_logical_operator_spans: Box<[Span]>,
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

    pub fn mixed_logical_operator_spans(&self) -> &[Span] {
        &self.mixed_logical_operator_spans
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

#[derive(Debug, Clone)]
pub struct RedirectFact<'a> {
    redirect: &'a Redirect,
    target_span: Option<Span>,
    analysis: Option<RedirectTargetAnalysis>,
}

impl<'a> RedirectFact<'a> {
    pub fn redirect(&self) -> &'a Redirect {
        self.redirect
    }

    pub fn target_span(&self) -> Option<Span> {
        self.target_span
    }

    pub fn analysis(&self) -> Option<RedirectTargetAnalysis> {
        self.analysis
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PathWordFact<'a> {
    word: &'a Word,
    context: ExpansionContext,
}

impl<'a> PathWordFact<'a> {
    pub fn word(&self) -> &'a Word {
        self.word
    }

    pub fn context(&self) -> ExpansionContext {
        self.context
    }
}

#[derive(Debug, Clone)]
pub struct SingleQuotedFragmentFact {
    span: Span,
    dollar_quoted: bool,
    command_name: Option<Box<str>>,
    assignment_target: Option<Box<str>>,
    variable_set_operand: bool,
}

impl SingleQuotedFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn dollar_quoted(&self) -> bool {
        self.dollar_quoted
    }

    pub fn command_name(&self) -> Option<&str> {
        self.command_name.as_deref()
    }

    pub fn assignment_target(&self) -> Option<&str> {
        self.assignment_target.as_deref()
    }

    pub fn variable_set_operand(&self) -> bool {
        self.variable_set_operand
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DollarDoubleQuotedFragmentFact {
    span: Span,
}

impl DollarDoubleQuotedFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OpenDoubleQuoteFragmentFact {
    span: Span,
}

impl OpenDoubleQuoteFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SuspectClosingQuoteFragmentFact {
    span: Span,
}

impl SuspectClosingQuoteFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BacktickFragmentFact {
    span: Span,
}

impl BacktickFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LegacyArithmeticFragmentFact {
    span: Span,
}

impl LegacyArithmeticFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionalParameterFragmentKind {
    AboveNine,
    General,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionalParameterFragmentFact {
    span: Span,
    kind: PositionalParameterFragmentKind,
    guarded: bool,
}

impl PositionalParameterFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> PositionalParameterFragmentKind {
        self.kind
    }

    pub fn is_above_nine(&self) -> bool {
        self.kind == PositionalParameterFragmentKind::AboveNine
    }

    pub fn is_guarded(&self) -> bool {
        self.guarded
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NestedParameterExpansionFragmentFact {
    span: Span,
}

impl NestedParameterExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndirectExpansionFragmentFact {
    span: Span,
    array_keys: bool,
}

impl IndirectExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn array_keys(&self) -> bool {
        self.array_keys
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndexedArrayReferenceFragmentFact {
    span: Span,
}

impl IndexedArrayReferenceFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubstringExpansionFragmentFact {
    span: Span,
}

impl SubstringExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CaseModificationFragmentFact {
    span: Span,
}

impl CaseModificationFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReplacementExpansionFragmentFact {
    span: Span,
}

impl ReplacementExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StarGlobRemovalFragmentFact {
    span: Span,
}

impl StarGlobRemovalFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactContext {
    Expansion(ExpansionContext),
    CaseSubject,
    ArithmeticCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactHostKind {
    Direct,
    AssignmentTargetSubscript,
    DeclarationNameSubscript,
    ArrayKeySubscript,
    ConditionalVarRefSubscript,
}

#[derive(Debug, Clone)]
pub struct WordFact<'a> {
    key: FactSpan,
    word: Cow<'a, Word>,
    command_id: CommandId,
    nested_word_command: bool,
    context: WordFactContext,
    host_kind: WordFactHostKind,
    zsh_options: Option<ZshOptionState>,
    analysis: ExpansionAnalysis,
    runtime_literal: RuntimeLiteralAnalysis,
    operand_class: Option<TestOperandClass>,
    static_text: Option<Box<str>>,
    has_literal_affixes: bool,
    contains_shell_quoting_literals: bool,
    scalar_expansion_spans: Box<[Span]>,
    unquoted_scalar_expansion_spans: Box<[Span]>,
    array_expansion_spans: Box<[Span]>,
    all_elements_array_expansion_spans: Box<[Span]>,
    unquoted_all_elements_array_expansion_spans: Box<[Span]>,
    unquoted_array_expansion_spans: Box<[Span]>,
    command_substitution_spans: Box<[Span]>,
    unquoted_command_substitution_spans: Box<[Span]>,
    double_quoted_expansion_spans: Box<[Span]>,
    unquoted_literal_between_double_quoted_segments_spans: Box<[Span]>,
}

impl<'a> WordFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn word(&self) -> &Word {
        self.word.as_ref()
    }

    pub fn span(&self) -> Span {
        self.word.span
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn context(&self) -> WordFactContext {
        self.context
    }

    pub fn expansion_context(&self) -> Option<ExpansionContext> {
        match self.context {
            WordFactContext::Expansion(context) => Some(context),
            WordFactContext::CaseSubject => None,
            WordFactContext::ArithmeticCommand => None,
        }
    }

    pub fn is_case_subject(&self) -> bool {
        self.context == WordFactContext::CaseSubject
    }

    pub fn is_arithmetic_command(&self) -> bool {
        self.context == WordFactContext::ArithmeticCommand
    }

    pub fn host_kind(&self) -> WordFactHostKind {
        self.host_kind
    }

    pub fn analysis(&self) -> ExpansionAnalysis {
        self.analysis
    }

    pub fn runtime_literal(&self) -> RuntimeLiteralAnalysis {
        self.runtime_literal
    }

    pub fn zsh_options(&self) -> Option<&ZshOptionState> {
        self.zsh_options.as_ref()
    }

    pub fn classification(&self) -> WordClassification {
        word_classification_from_analysis(self.analysis)
    }

    pub fn operand_class(&self) -> Option<TestOperandClass> {
        self.operand_class
    }

    pub fn static_text(&self) -> Option<&str> {
        self.static_text.as_deref()
    }

    pub fn has_literal_affixes(&self) -> bool {
        self.has_literal_affixes
    }

    pub fn contains_shell_quoting_literals(&self) -> bool {
        self.contains_shell_quoting_literals
    }

    pub fn scalar_expansion_spans(&self) -> &[Span] {
        &self.scalar_expansion_spans
    }

    pub fn unquoted_scalar_expansion_spans(&self) -> &[Span] {
        &self.unquoted_scalar_expansion_spans
    }

    pub fn array_expansion_spans(&self) -> &[Span] {
        &self.array_expansion_spans
    }

    pub fn all_elements_array_expansion_spans(&self) -> &[Span] {
        &self.all_elements_array_expansion_spans
    }

    pub fn unquoted_all_elements_array_expansion_spans(&self) -> &[Span] {
        &self.unquoted_all_elements_array_expansion_spans
    }

    pub fn unquoted_array_expansion_spans(&self) -> &[Span] {
        &self.unquoted_array_expansion_spans
    }

    pub fn command_substitution_spans(&self) -> &[Span] {
        &self.command_substitution_spans
    }

    pub fn unquoted_command_substitution_spans(&self) -> &[Span] {
        &self.unquoted_command_substitution_spans
    }

    pub fn double_quoted_expansion_spans(&self) -> &[Span] {
        &self.double_quoted_expansion_spans
    }

    pub fn unquoted_literal_between_double_quoted_segments_spans(&self) -> &[Span] {
        &self.unquoted_literal_between_double_quoted_segments_spans
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionHostKind {
    CommandArgument,
    HereStringOperand,
    DeclarationAssignmentValue,
    AssignmentTargetSubscript,
    DeclarationNameSubscript,
    ArrayKeySubscript,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct SubstitutionFact {
    span: Span,
    kind: CommandSubstitutionKind,
    command_syntax: Option<CommandSubstitutionSyntax>,
    stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
    body_contains_ls: bool,
    body_contains_echo: bool,
    body_contains_grep: bool,
    body_has_multiple_statements: bool,
    body_is_pgrep_lookup: bool,
    body_is_seq_utility: bool,
    body_has_commands: bool,
    bash_file_slurp: bool,
    host_word_span: Span,
    host_kind: SubstitutionHostKind,
    unquoted_in_host: bool,
}

impl SubstitutionFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> CommandSubstitutionKind {
        self.kind
    }

    pub fn command_syntax(&self) -> Option<CommandSubstitutionSyntax> {
        self.command_syntax
    }

    pub fn uses_backtick_syntax(&self) -> bool {
        self.command_syntax == Some(CommandSubstitutionSyntax::Backtick)
    }

    pub fn stdout_intent(&self) -> SubstitutionOutputIntent {
        self.stdout_intent
    }

    pub fn has_stdout_redirect(&self) -> bool {
        self.has_stdout_redirect
    }

    pub fn body_contains_ls(&self) -> bool {
        self.body_contains_ls
    }

    pub fn body_contains_echo(&self) -> bool {
        self.body_contains_echo
    }

    pub fn body_contains_grep(&self) -> bool {
        self.body_contains_grep
    }

    pub fn body_has_multiple_statements(&self) -> bool {
        self.body_has_multiple_statements
    }
    pub fn body_is_pgrep_lookup(&self) -> bool {
        self.body_is_pgrep_lookup
    }

    pub fn body_is_seq_utility(&self) -> bool {
        self.body_is_seq_utility
    }

    pub fn body_has_commands(&self) -> bool {
        self.body_has_commands
    }
    pub fn is_bash_file_slurp(&self) -> bool {
        self.bash_file_slurp
    }
    pub fn host_word_span(&self) -> Span {
        self.host_word_span
    }

    pub fn host_kind(&self) -> SubstitutionHostKind {
        self.host_kind
    }

    pub fn unquoted_in_host(&self) -> bool {
        self.unquoted_in_host
    }

    pub fn stdout_is_captured(&self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Captured
    }

    pub fn stdout_is_discarded(&self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Discarded
    }

    pub fn stdout_is_rerouted(&self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Rerouted
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoopHeaderWordFact<'a> {
    word: &'a Word,
    classification: WordClassification,
    has_unquoted_command_substitution: bool,
    contains_ls_substitution: bool,
    contains_find_substitution: bool,
}

impl<'a> LoopHeaderWordFact<'a> {
    pub fn word(&self) -> &'a Word {
        self.word
    }

    pub fn span(&self) -> Span {
        self.word.span
    }

    pub fn classification(&self) -> WordClassification {
        self.classification
    }

    pub fn has_command_substitution(&self) -> bool {
        self.classification.has_command_substitution()
    }

    pub fn has_unquoted_command_substitution(&self) -> bool {
        self.has_unquoted_command_substitution
    }

    pub fn contains_ls_substitution(&self) -> bool {
        self.contains_ls_substitution
    }

    pub fn contains_find_substitution(&self) -> bool {
        self.contains_find_substitution
    }
}

#[derive(Debug, Clone)]
pub struct ForHeaderFact<'a> {
    command: &'a ForCommand,
    command_id: CommandId,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact<'a>]>,
}

impl<'a> ForHeaderFact<'a> {
    pub fn command(&self) -> &'a ForCommand {
        self.command
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn words(&self) -> &[LoopHeaderWordFact<'a>] {
        &self.words
    }

    pub fn has_command_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::has_command_substitution)
    }

    pub fn has_find_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::contains_find_substitution)
    }
}

#[derive(Debug, Clone)]
pub struct SelectHeaderFact<'a> {
    command: &'a SelectCommand,
    command_id: CommandId,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact<'a>]>,
}

impl<'a> SelectHeaderFact<'a> {
    pub fn command(&self) -> &'a SelectCommand {
        self.command
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn words(&self) -> &[LoopHeaderWordFact<'a>] {
        &self.words
    }

    pub fn has_command_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::has_command_substitution)
    }

    pub fn has_find_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::contains_find_substitution)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CaseItemFact<'a> {
    item: &'a CaseItem,
    command_id: CommandId,
}

impl<'a> CaseItemFact<'a> {
    pub fn item(&self) -> &'a CaseItem {
        self.item
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn terminator(&self) -> CaseTerminator {
        self.item.terminator
    }

    pub fn terminator_span(&self) -> Option<Span> {
        self.item.terminator_span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CasePatternShadowFact {
    shadowing_pattern_span: Span,
    shadowed_pattern_span: Span,
}

impl CasePatternShadowFact {
    pub fn shadowing_pattern_span(&self) -> Span {
        self.shadowing_pattern_span
    }

    pub fn shadowed_pattern_span(&self) -> Span {
        self.shadowed_pattern_span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetoptsOptionSpec {
    option: char,
    requires_argument: bool,
}

impl GetoptsOptionSpec {
    pub fn option(self) -> char {
        self.option
    }

    pub fn requires_argument(self) -> bool {
        self.requires_argument
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetoptsCaseLabelFact {
    label: char,
    span: Span,
    is_bare_single_letter: bool,
}

impl GetoptsCaseLabelFact {
    pub fn label(self) -> char {
        self.label
    }

    pub fn span(self) -> Span {
        self.span
    }

    pub fn is_bare_single_letter(self) -> bool {
        self.is_bare_single_letter
    }
}

#[derive(Debug, Clone)]
pub struct GetoptsCaseFact {
    case_span: Span,
    declared_options: Box<[GetoptsOptionSpec]>,
    handled_case_labels: Box<[GetoptsCaseLabelFact]>,
    unexpected_case_labels: Box<[GetoptsCaseLabelFact]>,
    invalid_case_pattern_spans: Box<[Span]>,
    has_fallback_pattern: bool,
    missing_options: Box<[GetoptsOptionSpec]>,
}

impl GetoptsCaseFact {
    pub fn case_span(&self) -> Span {
        self.case_span
    }

    pub fn declared_options(&self) -> &[GetoptsOptionSpec] {
        &self.declared_options
    }

    pub fn handled_case_labels(&self) -> &[GetoptsCaseLabelFact] {
        &self.handled_case_labels
    }

    pub fn unexpected_case_labels(&self) -> &[GetoptsCaseLabelFact] {
        &self.unexpected_case_labels
    }

    pub fn invalid_case_pattern_spans(&self) -> &[Span] {
        &self.invalid_case_pattern_spans
    }

    pub fn has_fallback_pattern(&self) -> bool {
        self.has_fallback_pattern
    }

    pub fn missing_options(&self) -> &[GetoptsOptionSpec] {
        &self.missing_options
    }
}

#[derive(Debug, Clone)]
pub struct FunctionHeaderFact<'a> {
    function: &'a FunctionDef,
    binding_id: Option<BindingId>,
    scope_id: Option<ScopeId>,
    call_arity: FunctionCallArityFacts,
}

impl<'a> FunctionHeaderFact<'a> {
    pub fn function(&self) -> &'a FunctionDef {
        self.function
    }

    pub fn static_name_entry(&self) -> Option<(&'a Name, Span)> {
        self.function.static_name_entries().next()
    }

    pub fn binding_id(&self) -> Option<BindingId> {
        self.binding_id
    }

    pub fn function_scope(&self) -> Option<ScopeId> {
        self.scope_id
    }

    pub fn call_arity(&self) -> &FunctionCallArityFacts {
        &self.call_arity
    }

    pub fn function_span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.function.span, source)
    }

    pub fn span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.function.header.span(), source)
    }

    pub fn uses_function_keyword(&self) -> bool {
        self.function.uses_function_keyword()
    }

    pub fn has_trailing_parens(&self) -> bool {
        self.function.has_trailing_parens()
    }

    pub fn function_keyword_span(&self) -> Option<Span> {
        self.function.header.function_keyword_span
    }

    pub fn trailing_parens_span(&self) -> Option<Span> {
        self.function.header.trailing_parens_span
    }
}

#[derive(Debug, Clone, Default)]
pub struct FunctionCallArityFacts {
    call_count: usize,
    min_arg_count: usize,
    max_arg_count: usize,
    zero_arg_call_spans: Vec<Span>,
}

impl FunctionCallArityFacts {
    pub fn call_count(&self) -> usize {
        self.call_count
    }

    pub fn min_arg_count(&self) -> Option<usize> {
        (self.call_count != 0).then_some(self.min_arg_count)
    }

    pub fn max_arg_count(&self) -> Option<usize> {
        (self.call_count != 0).then_some(self.max_arg_count)
    }

    pub fn called_only_without_args(&self) -> bool {
        self.call_count != 0 && self.max_arg_count == 0
    }

    pub fn zero_arg_call_spans(&self) -> &[Span] {
        &self.zero_arg_call_spans
    }

    fn record_call(&mut self, arg_count: usize, span: Span) {
        if self.call_count == 0 {
            self.min_arg_count = arg_count;
            self.max_arg_count = arg_count;
        } else {
            self.min_arg_count = self.min_arg_count.min(arg_count);
            self.max_arg_count = self.max_arg_count.max(arg_count);
        }
        if arg_count == 0 {
            self.zero_arg_call_spans.push(span);
        }
        self.call_count += 1;
    }
}

#[derive(Debug, Clone)]
pub struct PipelineSegmentFact<'a> {
    stmt: &'a Stmt,
    command_id: CommandId,
    literal_name: Option<Box<str>>,
    effective_name: Option<Box<str>>,
}

impl<'a> PipelineSegmentFact<'a> {
    pub fn stmt(&self) -> &'a Stmt {
        self.stmt
    }

    pub fn command(&self) -> &'a Command {
        &self.stmt.command
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn literal_name(&self) -> Option<&str> {
        self.literal_name.as_deref()
    }

    pub fn effective_name(&self) -> Option<&str> {
        self.effective_name.as_deref()
    }

    pub fn effective_or_literal_name(&self) -> Option<&str> {
        self.effective_name().or_else(|| self.literal_name())
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.effective_name() == Some(name)
    }

    pub fn static_utility_name(&self) -> Option<&str> {
        self.effective_or_literal_name()
    }

    pub fn static_utility_name_is(&self, name: &str) -> bool {
        self.static_utility_name() == Some(name)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PipelineOperatorFact {
    op: BinaryOp,
    span: Span,
}

impl PipelineOperatorFact {
    pub fn op(&self) -> BinaryOp {
        self.op
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone)]
pub struct PipelineFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    segments: Box<[PipelineSegmentFact<'a>]>,
    operators: Box<[PipelineOperatorFact]>,
}

impl<'a> PipelineFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn command(&self) -> &'a BinaryCommand {
        self.command
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn segments(&self) -> &[PipelineSegmentFact<'a>] {
        &self.segments
    }

    pub fn operators(&self) -> &[PipelineOperatorFact] {
        &self.operators
    }

    pub fn first_segment(&self) -> Option<&PipelineSegmentFact<'a>> {
        self.segments.first()
    }

    pub fn last_segment(&self) -> Option<&PipelineSegmentFact<'a>> {
        self.segments.last()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ListOperatorFact {
    op: BinaryOp,
    span: Span,
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
pub struct ListSegmentFact {
    command_id: CommandId,
    span: Span,
    kind: ListSegmentKind,
    assignment_target: Option<Box<str>>,
    assignment_span: Option<Span>,
}

impl ListSegmentFact {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> ListSegmentKind {
        self.kind
    }

    pub fn assignment_target(&self) -> Option<&str> {
        self.assignment_target.as_deref()
    }

    pub fn assignment_span(&self) -> Option<Span> {
        self.assignment_span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixedShortCircuitKind {
    TestChain,
    AssignmentTernary,
    Fallthrough,
}

#[derive(Debug, Clone)]
pub struct BindingValueFact<'a> {
    kind: BindingValueKind<'a>,
    conditional_assignment_shortcut: bool,
}

#[derive(Debug, Clone)]
enum BindingValueKind<'a> {
    Scalar(&'a Word),
    Loop(Box<[&'a Word]>),
}

impl<'a> BindingValueFact<'a> {
    fn scalar(word: &'a Word) -> Self {
        Self {
            kind: BindingValueKind::Scalar(word),
            conditional_assignment_shortcut: false,
        }
    }

    fn from_loop_words(words: Box<[&'a Word]>) -> Self {
        Self {
            kind: BindingValueKind::Loop(words),
            conditional_assignment_shortcut: false,
        }
    }

    pub fn scalar_word(&self) -> Option<&'a Word> {
        match &self.kind {
            BindingValueKind::Scalar(word) => Some(*word),
            BindingValueKind::Loop(_) => None,
        }
    }

    pub fn loop_words(&self) -> Option<&[&'a Word]> {
        match &self.kind {
            BindingValueKind::Scalar(_) => None,
            BindingValueKind::Loop(words) => Some(words.as_ref()),
        }
    }

    pub fn conditional_assignment_shortcut(&self) -> bool {
        self.conditional_assignment_shortcut
    }

    fn mark_conditional_assignment_shortcut(&mut self) {
        self.conditional_assignment_shortcut = true;
    }
}

#[derive(Debug, Clone)]
pub struct ListFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    operators: Box<[ListOperatorFact]>,
    segments: Box<[ListSegmentFact]>,
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

    pub fn segments(&self) -> &[ListSegmentFact] {
        &self.segments
    }

    pub fn mixed_short_circuit_span(&self) -> Option<Span> {
        self.mixed_short_circuit_span
    }

    pub fn mixed_short_circuit_kind(&self) -> Option<MixedShortCircuitKind> {
        self.mixed_short_circuit_kind
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StatementFact {
    body_span: Span,
    stmt_span: Span,
    command_id: CommandId,
}

impl StatementFact {
    pub fn body_span(&self) -> Span {
        self.body_span
    }

    pub fn stmt_span(&self) -> Span {
        self.stmt_span
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReadCommandFacts {
    pub uses_raw_input: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct SuCommandFacts {
    has_login_or_command_flag: bool,
}

impl SuCommandFacts {
    pub fn has_login_or_command_flag(self) -> bool {
        self.has_login_or_command_flag
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EchoCommandFacts<'a> {
    portability_flag_word: Option<&'a Word>,
    uses_escape_interpreting_flag: bool,
}

impl<'a> EchoCommandFacts<'a> {
    pub fn portability_flag_word(self) -> Option<&'a Word> {
        self.portability_flag_word
    }

    pub fn uses_escape_interpreting_flag(self) -> bool {
        self.uses_escape_interpreting_flag
    }
}

#[derive(Debug, Clone)]
pub struct TrCommandFacts<'a> {
    operand_words: Box<[&'a Word]>,
}

impl<'a> TrCommandFacts<'a> {
    pub fn operand_words(&self) -> &[&'a Word] {
        &self.operand_words
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SedCommandFacts {
    has_single_substitution_script: bool,
}

impl SedCommandFacts {
    pub fn has_single_substitution_script(self) -> bool {
        self.has_single_substitution_script
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PrintfCommandFacts<'a> {
    pub format_word: Option<&'a Word>,
    pub format_word_has_literal_percent: bool,
    pub uses_q_format: bool,
}

#[derive(Debug, Clone)]
pub struct UnsetCommandFacts<'a> {
    pub function_mode: bool,
    operand_words: Box<[&'a Word]>,
    operand_facts: Box<[UnsetOperandFact<'a>]>,
    prefix_match_operand_spans: Box<[Span]>,
    options_parseable: bool,
}

impl<'a> UnsetCommandFacts<'a> {
    pub fn operand_words(&self) -> &[&'a Word] {
        &self.operand_words
    }

    pub fn prefix_match_operand_spans(&self) -> &[Span] {
        &self.prefix_match_operand_spans
    }

    pub(crate) fn operand_facts(&self) -> &[UnsetOperandFact<'a>] {
        &self.operand_facts
    }

    pub fn targets_function_name(&self, source: &str, target_name: &str) -> bool {
        if !self.function_mode || !self.options_parseable {
            return false;
        }

        for word in self.operand_words() {
            let Some(text) = static_word_text(word, source) else {
                return false;
            };

            if text == target_name {
                return true;
            }
        }

        false
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UnsetOperandFact<'a> {
    word: &'a Word,
    array_subscript: Option<UnsetArraySubscriptFact>,
}

impl<'a> UnsetOperandFact<'a> {
    pub(crate) fn word(&self) -> &'a Word {
        self.word
    }

    pub(crate) fn array_subscript(&self) -> Option<&UnsetArraySubscriptFact> {
        self.array_subscript.as_ref()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UnsetArraySubscriptFact {
    name: Name,
    key_contains_quote: bool,
}

impl UnsetArraySubscriptFact {
    pub(crate) fn name(&self) -> &Name {
        &self.name
    }

    pub(crate) fn key_contains_quote(&self) -> bool {
        self.key_contains_quote
    }
}

#[derive(Debug, Clone)]
pub struct RmCommandFacts {
    dangerous_path_spans: Box<[Span]>,
}

impl RmCommandFacts {
    pub fn dangerous_path_spans(&self) -> &[Span] {
        &self.dangerous_path_spans
    }
}

#[derive(Debug, Clone)]
pub struct SshCommandFacts {
    local_expansion_spans: Box<[Span]>,
}

impl SshCommandFacts {
    pub fn local_expansion_spans(&self) -> &[Span] {
        &self.local_expansion_spans
    }
}

#[derive(Debug, Clone)]
pub struct FindCommandFacts {
    pub has_print0: bool,
    or_without_grouping_spans: Box<[Span]>,
    glob_pattern_operand_spans: Box<[Span]>,
}

impl FindCommandFacts {
    pub fn or_without_grouping_spans(&self) -> &[Span] {
        &self.or_without_grouping_spans
    }

    pub fn glob_pattern_operand_spans(&self) -> &[Span] {
        &self.glob_pattern_operand_spans
    }
}

#[derive(Debug, Clone)]
pub struct FindExecCommandFacts {
    argument_word_spans: Box<[Span]>,
}

impl FindExecCommandFacts {
    pub fn argument_word_spans(&self) -> &[Span] {
        &self.argument_word_spans
    }
}

#[derive(Debug, Clone)]
pub struct FindExecDirCommandFacts {
    shell_command_spans: Box<[Span]>,
}

impl FindExecDirCommandFacts {
    pub fn shell_command_spans(&self) -> &[Span] {
        &self.shell_command_spans
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MapfileCommandFacts {
    input_fd: Option<i32>,
}

impl MapfileCommandFacts {
    pub fn input_fd(self) -> Option<i32> {
        self.input_fd
    }
}

#[derive(Debug, Clone)]
pub struct XargsCommandFacts {
    pub uses_null_input: bool,
    inline_replace_option_spans: Box<[Span]>,
}

impl XargsCommandFacts {
    pub fn inline_replace_option_spans(&self) -> &[Span] {
        &self.inline_replace_option_spans
    }
}

#[derive(Debug, Clone)]
pub struct WaitCommandFacts {
    option_spans: Box<[Span]>,
}

impl WaitCommandFacts {
    pub fn option_spans(&self) -> &[Span] {
        &self.option_spans
    }
}

#[derive(Debug, Clone)]
pub struct LnCommandFacts<'a> {
    symlink_target_words: Box<[&'a Word]>,
}

impl<'a> LnCommandFacts<'a> {
    pub fn symlink_target_words(&self) -> &[&'a Word] {
        &self.symlink_target_words
    }
}

#[derive(Debug, Clone)]
pub struct GrepCommandFacts<'a> {
    pub uses_only_matching: bool,
    pub uses_fixed_strings: bool,
    patterns: Box<[GrepPatternFact<'a>]>,
}

impl<'a> GrepCommandFacts<'a> {
    pub fn patterns(&self) -> &[GrepPatternFact<'a>] {
        &self.patterns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrepPatternSourceKind {
    ImplicitOperand,
    ShortOptionSeparate,
    ShortOptionAttached,
    LongOptionSeparate,
    LongOptionAttached,
}

impl GrepPatternSourceKind {
    pub fn uses_separate_pattern_word(self) -> bool {
        matches!(
            self,
            Self::ImplicitOperand | Self::ShortOptionSeparate | Self::LongOptionSeparate
        )
    }
}

#[derive(Debug, Clone)]
pub struct GrepPatternFact<'a> {
    word: &'a Word,
    static_text: Option<Box<str>>,
    source_kind: GrepPatternSourceKind,
    starts_with_glob_style_star: bool,
}

impl<'a> GrepPatternFact<'a> {
    pub fn word(&self) -> &'a Word {
        self.word
    }

    pub fn span(&self) -> Span {
        self.word.span
    }

    pub fn static_text(&self) -> Option<&str> {
        self.static_text.as_deref()
    }

    pub fn source_kind(&self) -> GrepPatternSourceKind {
        self.source_kind
    }

    pub fn starts_with_glob_style_star(&self) -> bool {
        self.starts_with_glob_style_star
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PsCommandFacts {
    pub has_pid_selector: bool,
}

#[derive(Debug, Clone)]
pub struct SetCommandFacts {
    pub errexit_change: Option<bool>,
    pub errtrace_change: Option<bool>,
    pub pipefail_change: Option<bool>,
    resets_positional_parameters: bool,
    errtrace_option_spans: Box<[Span]>,
    pipefail_option_spans: Box<[Span]>,
    flags_without_prefix_spans: Box<[Span]>,
}

impl SetCommandFacts {
    pub fn resets_positional_parameters(&self) -> bool {
        self.resets_positional_parameters
    }

    pub fn errtrace_option_spans(&self) -> &[Span] {
        &self.errtrace_option_spans
    }

    pub fn pipefail_option_spans(&self) -> &[Span] {
        &self.pipefail_option_spans
    }

    pub fn flags_without_prefix_spans(&self) -> &[Span] {
        &self.flags_without_prefix_spans
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryChangeCommandKind {
    Cd,
    Pushd,
    Popd,
}

impl DirectoryChangeCommandKind {
    pub fn command_name(self) -> &'static str {
        match self {
            Self::Cd => "cd",
            Self::Pushd => "pushd",
            Self::Popd => "popd",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DirectoryChangeCommandFacts {
    kind: DirectoryChangeCommandKind,
    plain_directory_stack_marker: bool,
    manual_restore_candidate: bool,
}

impl DirectoryChangeCommandFacts {
    pub fn kind(&self) -> DirectoryChangeCommandKind {
        self.kind
    }

    pub fn command_name(&self) -> &'static str {
        self.kind.command_name()
    }

    pub fn is_plain_directory_stack_marker(&self) -> bool {
        self.plain_directory_stack_marker
    }

    pub fn is_manual_restore_candidate(&self) -> bool {
        self.manual_restore_candidate
    }
}

#[derive(Debug, Clone)]
pub struct ConfigureCommandFacts {
    misspelled_option_spans: Box<[Span]>,
}

impl ConfigureCommandFacts {
    pub fn misspelled_option_spans(&self) -> &[Span] {
        &self.misspelled_option_spans
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FunctionPositionalParameterFacts {
    required_arg_count: usize,
    uses_unprotected_positional_parameters: bool,
    resets_positional_parameters: bool,
}

impl FunctionPositionalParameterFacts {
    pub fn required_arg_count(&self) -> usize {
        self.required_arg_count
    }

    pub fn uses_positional_parameters(&self) -> bool {
        self.uses_unprotected_positional_parameters
    }

    pub fn uses_unprotected_positional_parameters(&self) -> bool {
        self.uses_unprotected_positional_parameters
    }

    pub fn resets_positional_parameters(&self) -> bool {
        self.resets_positional_parameters
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExprCommandFacts {
    pub uses_arithmetic_operator: bool,
    uses_substr_string_form: bool,
}

impl ExprCommandFacts {
    pub fn uses_arithmetic_operator(self) -> bool {
        self.uses_arithmetic_operator
    }

    pub fn uses_substr_string_form(self) -> bool {
        self.uses_substr_string_form
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExitCommandFacts<'a> {
    pub status_word: Option<&'a Word>,
    pub is_numeric_literal: bool,
    status_is_static: bool,
    status_has_literal_content: bool,
}

impl<'a> ExitCommandFacts<'a> {
    pub fn has_static_status(self) -> bool {
        self.status_is_static
    }

    pub fn has_invalid_status_argument(self) -> bool {
        self.status_word.is_some()
            && !self.is_numeric_literal
            && (self.status_is_static || self.status_has_literal_content)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SudoFamilyCommandFacts {
    pub invoker: SudoFamilyInvoker,
}

#[derive(Debug, Clone, Default)]
pub struct CommandOptionFacts<'a> {
    rm: Option<RmCommandFacts>,
    ssh: Option<SshCommandFacts>,
    read: Option<ReadCommandFacts>,
    su: Option<SuCommandFacts>,
    echo: Option<EchoCommandFacts<'a>>,
    sed: Option<SedCommandFacts>,
    tr: Option<TrCommandFacts<'a>>,
    printf: Option<PrintfCommandFacts<'a>>,
    unset: Option<UnsetCommandFacts<'a>>,
    find: Option<FindCommandFacts>,
    find_exec: Option<FindExecCommandFacts>,
    find_execdir: Option<FindExecDirCommandFacts>,
    mapfile: Option<MapfileCommandFacts>,
    xargs: Option<XargsCommandFacts>,
    wait: Option<WaitCommandFacts>,
    ln: Option<LnCommandFacts<'a>>,
    grep: Option<GrepCommandFacts<'a>>,
    ps: Option<PsCommandFacts>,
    set: Option<SetCommandFacts>,
    directory_change: Option<DirectoryChangeCommandFacts>,
    configure: Option<ConfigureCommandFacts>,
    expr: Option<ExprCommandFacts>,
    exit: Option<ExitCommandFacts<'a>>,
    sudo_family: Option<SudoFamilyCommandFacts>,
    nonportable_sh_builtin_option_span: Option<Span>,
    file_operand_words: Box<[&'a Word]>,
}

impl<'a> CommandOptionFacts<'a> {
    pub fn rm(&self) -> Option<&RmCommandFacts> {
        self.rm.as_ref()
    }

    pub fn ssh(&self) -> Option<&SshCommandFacts> {
        self.ssh.as_ref()
    }

    pub fn read(&self) -> Option<&ReadCommandFacts> {
        self.read.as_ref()
    }

    pub fn su(&self) -> Option<&SuCommandFacts> {
        self.su.as_ref()
    }

    pub fn echo(&self) -> Option<&EchoCommandFacts<'a>> {
        self.echo.as_ref()
    }

    pub fn sed(&self) -> Option<&SedCommandFacts> {
        self.sed.as_ref()
    }

    pub fn tr(&self) -> Option<&TrCommandFacts<'a>> {
        self.tr.as_ref()
    }

    pub fn printf(&self) -> Option<&PrintfCommandFacts<'a>> {
        self.printf.as_ref()
    }

    pub fn unset(&self) -> Option<&UnsetCommandFacts<'a>> {
        self.unset.as_ref()
    }

    pub fn find(&self) -> Option<&FindCommandFacts> {
        self.find.as_ref()
    }

    pub fn find_exec(&self) -> Option<&FindExecCommandFacts> {
        self.find_exec.as_ref()
    }

    pub fn find_execdir(&self) -> Option<&FindExecDirCommandFacts> {
        self.find_execdir.as_ref()
    }

    pub fn mapfile(&self) -> Option<&MapfileCommandFacts> {
        self.mapfile.as_ref()
    }

    pub fn xargs(&self) -> Option<&XargsCommandFacts> {
        self.xargs.as_ref()
    }

    pub fn wait(&self) -> Option<&WaitCommandFacts> {
        self.wait.as_ref()
    }

    pub fn ln(&self) -> Option<&LnCommandFacts<'a>> {
        self.ln.as_ref()
    }

    pub fn grep(&self) -> Option<&GrepCommandFacts<'a>> {
        self.grep.as_ref()
    }

    pub fn ps(&self) -> Option<&PsCommandFacts> {
        self.ps.as_ref()
    }

    pub fn set(&self) -> Option<&SetCommandFacts> {
        self.set.as_ref()
    }

    pub fn directory_change(&self) -> Option<&DirectoryChangeCommandFacts> {
        self.directory_change.as_ref()
    }

    pub fn configure(&self) -> Option<&ConfigureCommandFacts> {
        self.configure.as_ref()
    }

    pub fn expr(&self) -> Option<&ExprCommandFacts> {
        self.expr.as_ref()
    }

    pub fn exit(&self) -> Option<&ExitCommandFacts<'a>> {
        self.exit.as_ref()
    }

    pub fn sudo_family(&self) -> Option<&SudoFamilyCommandFacts> {
        self.sudo_family.as_ref()
    }

    pub fn nonportable_sh_builtin_option_span(&self) -> Option<Span> {
        self.nonportable_sh_builtin_option_span
    }

    pub fn file_operand_words(&self) -> &[&'a Word] {
        &self.file_operand_words
    }

    fn build(command: &'a Command, normalized: &NormalizedCommand<'a>, source: &str) -> Self {
        Self {
            rm: normalized
                .literal_name
                .as_deref()
                .is_some_and(|name| name == "rm" && normalized.wrappers.is_empty())
                .then(|| parse_rm_command(normalized.body_args(), source))
                .flatten(),
            ssh: (normalized.effective_name_is("ssh") && normalized.wrappers.is_empty())
                .then(|| parse_ssh_command(normalized.body_args(), source))
                .flatten(),
            read: normalized
                .effective_name_is("read")
                .then(|| ReadCommandFacts {
                    uses_raw_input: read_uses_raw_input(normalized.body_args(), source),
                }),
            su: normalized
                .effective_name_is("su")
                .then(|| parse_su_command(normalized.body_args(), source)),
            echo: normalized
                .effective_name_is("echo")
                .then(|| parse_echo_command(normalized.body_args(), source)),
            sed: normalized
                .effective_name_is("sed")
                .then(|| parse_sed_command(normalized.body_args(), source)),
            tr: (normalized.effective_name_is("tr") && normalized.wrappers.is_empty())
                .then(|| parse_tr_command(normalized.body_args(), source)),
            printf: normalized.effective_name_is("printf").then(|| {
                let format_word = printf_format_word(normalized.body_args(), source);
                PrintfCommandFacts {
                    format_word_has_literal_percent: format_word
                        .is_some_and(|word| printf_format_word_has_literal_percent(word, source)),
                    uses_q_format: format_word
                        .is_some_and(|word| printf_uses_q_format(word, source)),
                    format_word,
                }
            }),
            unset: normalized
                .effective_name_is("unset")
                .then(|| parse_unset_command(normalized.body_args(), source)),
            find: normalized
                .effective_name_is("find")
                .then(|| parse_find_command(normalized.body_args(), source)),
            find_exec: (normalized.has_wrapper(WrapperKind::FindExec)
                || normalized.has_wrapper(WrapperKind::FindExecDir))
            .then(|| FindExecCommandFacts {
                argument_word_spans: parse_find_exec_argument_word_spans(
                    normalized.body_args(),
                    source,
                )
                .into_boxed_slice(),
            }),
            find_execdir: normalized
                .has_wrapper(WrapperKind::FindExecDir)
                .then(|| {
                    parse_find_execdir_shell_command(
                        normalized.effective_name.as_deref(),
                        normalized.body_args(),
                        source,
                    )
                })
                .flatten(),
            mapfile: (normalized.effective_name_is("mapfile")
                || normalized.effective_name_is("readarray"))
            .then(|| parse_mapfile_command(normalized.body_args(), source)),
            xargs: normalized
                .effective_name_is("xargs")
                .then(|| parse_xargs_command(normalized.body_args(), source)),
            wait: normalized
                .effective_name_is("wait")
                .then(|| parse_wait_command(normalized.body_args(), source)),
            ln: normalized
                .effective_name_is("ln")
                .then(|| parse_ln_command(normalized.body_args(), source))
                .flatten(),
            grep: normalized
                .effective_name_is("grep")
                .then(|| parse_grep_command(normalized.body_args(), source))
                .flatten(),
            ps: normalized
                .effective_name_is("ps")
                .then(|| parse_ps_command(normalized.body_args(), source)),
            set: normalized
                .effective_name_is("set")
                .then(|| parse_set_command(normalized.body_args(), source)),
            directory_change: parse_directory_change_command(normalized, source),
            configure: normalized
                .effective_or_literal_name()
                .is_some_and(is_configure_command_name)
                .then(|| parse_configure_command(normalized.body_args(), source)),
            expr: normalized
                .effective_name_is("expr")
                .then_some(())
                .and_then(|_| parse_expr_command(normalized.body_args(), source)),
            exit: parse_exit_command(command, source),
            sudo_family: normalized.has_wrapper(WrapperKind::SudoFamily).then(|| {
                SudoFamilyCommandFacts {
                    invoker: detect_sudo_family_invoker(command, normalized, source)
                        .expect("sudo-family wrapper should preserve its invoker"),
                }
            }),
            nonportable_sh_builtin_option_span: first_nonportable_sh_builtin_option_span(
                normalized, source,
            ),
            file_operand_words: same_command_file_operand_words(
                normalized.effective_or_literal_name(),
                normalized.body_args(),
                source,
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandFact<'a> {
    id: CommandId,
    key: FactSpan,
    visit: CommandVisit<'a>,
    nested_word_command: bool,
    normalized: NormalizedCommand<'a>,
    zsh_options: Option<ZshOptionState>,
    redirect_facts: Box<[RedirectFact<'a>]>,
    substitution_facts: Box<[SubstitutionFact]>,
    options: CommandOptionFacts<'a>,
    scope_read_source_words: Box<[PathWordFact<'a>]>,
    glued_closing_bracket_operand_span: Option<Span>,
    simple_test: Option<SimpleTestFact<'a>>,
    conditional: Option<ConditionalFact<'a>>,
}

impl<'a> CommandFact<'a> {
    pub fn id(&self) -> CommandId {
        self.id
    }

    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn stmt(&self) -> &'a Stmt {
        self.visit.stmt
    }

    pub fn command(&self) -> &'a Command {
        self.visit.command
    }

    pub fn span(&self) -> Span {
        command_span(self.visit.command)
    }

    pub fn span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.span(), source)
    }

    pub fn redirects(&self) -> &'a [Redirect] {
        self.visit.redirects
    }

    pub fn zsh_options(&self) -> Option<&ZshOptionState> {
        self.zsh_options.as_ref()
    }

    pub fn redirect_facts(&self) -> &[RedirectFact<'a>] {
        &self.redirect_facts
    }

    pub fn substitution_facts(&self) -> &[SubstitutionFact] {
        &self.substitution_facts
    }

    pub fn normalized(&self) -> &NormalizedCommand<'a> {
        &self.normalized
    }

    pub fn options(&self) -> &CommandOptionFacts<'a> {
        &self.options
    }

    pub fn scope_read_source_words(&self) -> &[PathWordFact<'a>] {
        &self.scope_read_source_words
    }

    pub fn glued_closing_bracket_operand_span(&self) -> Option<Span> {
        self.glued_closing_bracket_operand_span
    }

    pub fn simple_test(&self) -> Option<&SimpleTestFact<'a>> {
        self.simple_test.as_ref()
    }

    pub fn conditional(&self) -> Option<&ConditionalFact<'a>> {
        self.conditional.as_ref()
    }

    pub fn literal_name(&self) -> Option<&str> {
        self.normalized.literal_name.as_deref()
    }

    pub fn effective_name(&self) -> Option<&str> {
        self.normalized.effective_name.as_deref()
    }

    pub fn effective_or_literal_name(&self) -> Option<&str> {
        self.normalized.effective_or_literal_name()
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.normalized.effective_name_is(name)
    }

    pub fn static_utility_name(&self) -> Option<&str> {
        self.effective_or_literal_name()
    }

    pub fn static_utility_name_is(&self, name: &str) -> bool {
        self.static_utility_name() == Some(name)
    }

    pub fn wrappers(&self) -> &[WrapperKind] {
        &self.normalized.wrappers
    }

    pub fn has_wrapper(&self, wrapper: WrapperKind) -> bool {
        self.normalized.has_wrapper(wrapper)
    }

    pub fn declaration(&self) -> Option<&command::NormalizedDeclaration<'a>> {
        self.normalized.declaration.as_ref()
    }

    pub fn body_span(&self) -> Span {
        self.normalized.body_span
    }

    pub fn body_name_word(&self) -> Option<&'a Word> {
        self.normalized.body_name_word()
    }

    pub fn body_word_span(&self) -> Option<Span> {
        self.normalized.body_word_span()
    }

    pub fn body_args(&self) -> &[&'a Word] {
        self.normalized.body_args()
    }

    pub fn file_operand_words(&self) -> &[&'a Word] {
        self.options.file_operand_words()
    }
}

#[derive(Debug, Clone)]
pub struct LinterFacts<'a> {
    commands: Vec<CommandFact<'a>>,
    structural_command_ids: Vec<CommandId>,
    #[cfg_attr(not(test), allow(dead_code))]
    command_ids_by_span: CommandLookupIndex,
    if_condition_command_ids: FxHashSet<CommandId>,
    elif_condition_command_ids: FxHashSet<CommandId>,
    binding_values: FxHashMap<BindingId, BindingValueFact<'a>>,
    broken_assoc_key_spans: Vec<Span>,
    comma_array_assignment_spans: Vec<Span>,
    ifs_literal_backslash_assignment_value_spans: Vec<Span>,
    env_prefix_assignment_scope_spans: Vec<Span>,
    presence_tested_names: FxHashSet<Name>,
    nested_presence_test_spans: FxHashMap<Name, Vec<Span>>,
    subscript_index_reference_spans: FxHashSet<FactSpan>,
    compound_assignment_value_word_spans: FxHashSet<FactSpan>,
    words: Vec<WordFact<'a>>,
    word_index: FxHashMap<FactSpan, Vec<usize>>,
    unquoted_command_argument_use_offsets: FxHashMap<Name, Vec<usize>>,
    array_assignment_split_word_indices: Vec<usize>,
    function_headers: Vec<FunctionHeaderFact<'a>>,
    function_body_without_braces_spans: Vec<Span>,
    function_parameter_fallback_spans: Vec<Span>,
    redundant_return_status_spans: Vec<Span>,
    for_headers: Vec<ForHeaderFact<'a>>,
    select_headers: Vec<SelectHeaderFact<'a>>,
    case_items: Vec<CaseItemFact<'a>>,
    case_pattern_shadows: Vec<CasePatternShadowFact>,
    getopts_cases: Vec<GetoptsCaseFact>,
    pipelines: Vec<PipelineFact<'a>>,
    lists: Vec<ListFact<'a>>,
    statement_facts: Vec<StatementFact>,
    single_test_subshell_spans: Vec<Span>,
    subshell_test_group_spans: Vec<Span>,
    indented_shebang_span: Option<Span>,
    space_after_hash_bang_span: Option<Span>,
    shebang_not_on_first_line_span: Option<Span>,
    missing_shebang_line_span: Option<Span>,
    duplicate_shebang_flag_span: Option<Span>,
    non_absolute_shebang_span: Option<Span>,
    errexit_enabled_anywhere: bool,
    commented_continuation_comment_spans: Vec<Span>,
    trailing_directive_comment_spans: Vec<Span>,
    condition_status_capture_spans: Vec<Span>,
    condition_command_substitution_spans: Vec<Span>,
    backtick_command_name_spans: Vec<Span>,
    dollar_question_after_command_spans: Vec<Span>,
    bare_command_name_assignment_spans: Vec<Span>,
    subshell_assignment_sites: Vec<NamedSpan>,
    subshell_later_use_sites: Vec<NamedSpan>,
    unused_heredoc_spans: Vec<Span>,
    heredoc_missing_end_spans: Vec<Span>,
    heredoc_closer_not_alone_spans: Vec<Span>,
    misquoted_heredoc_close_spans: Vec<Span>,
    heredoc_end_space_spans: Vec<Span>,
    echo_here_doc_spans: Vec<Span>,
    spaced_tabstrip_close_spans: Vec<Span>,
    plus_equals_assignment_spans: Vec<Span>,
    array_index_arithmetic_spans: Vec<Span>,
    arithmetic_score_line_spans: Vec<Span>,
    dollar_in_arithmetic_spans: Vec<Span>,
    arithmetic_command_substitution_spans: Vec<Span>,
    function_positional_parameter_facts: FxHashMap<ScopeId, FunctionPositionalParameterFacts>,
    single_quoted_fragments: Vec<SingleQuotedFragmentFact>,
    dollar_double_quoted_fragments: Vec<DollarDoubleQuotedFragmentFact>,
    open_double_quote_fragments: Vec<OpenDoubleQuoteFragmentFact>,
    suspect_closing_quote_fragments: Vec<SuspectClosingQuoteFragmentFact>,
    literal_brace_spans: Vec<Span>,
    backtick_fragments: Vec<BacktickFragmentFact>,
    legacy_arithmetic_fragments: Vec<LegacyArithmeticFragmentFact>,
    positional_parameter_fragments: Vec<PositionalParameterFragmentFact>,
    positional_parameter_operator_spans: Vec<Span>,
    double_paren_grouping_spans: Vec<Span>,
    arithmetic_for_update_operator_spans: Vec<Span>,
    base_prefix_arithmetic_spans: Vec<Span>,
    escape_scan_matches: Vec<EscapeScanMatch>,
    echo_backslash_escape_word_spans: Vec<Span>,
    echo_to_sed_substitution_spans: Vec<Span>,
    unicode_smart_quote_spans: Vec<Span>,
    pattern_exactly_one_extglob_spans: Vec<Span>,
    pattern_literal_spans: Vec<Span>,
    pattern_charclass_spans: Vec<Span>,
    nested_pattern_charclass_spans: FxHashSet<FactSpan>,
    nested_parameter_expansion_fragments: Vec<NestedParameterExpansionFragmentFact>,
    indirect_expansion_fragments: Vec<IndirectExpansionFragmentFact>,
    indexed_array_reference_fragments: Vec<IndexedArrayReferenceFragmentFact>,
    substring_expansion_fragments: Vec<SubstringExpansionFragmentFact>,
    case_modification_fragments: Vec<CaseModificationFragmentFact>,
    replacement_expansion_fragments: Vec<ReplacementExpansionFragmentFact>,
    star_glob_removal_fragments: Vec<StarGlobRemovalFragmentFact>,
    conditional_portability: ConditionalPortabilityFacts,
}

impl<'a> LinterFacts<'a> {
    pub fn build(
        file: &'a File,
        source: &'a str,
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        file_context: &'a FileContext,
    ) -> Self {
        LinterFactsBuilder::new(file, source, semantic, indexer, file_context).build()
    }

    pub fn commands(&self) -> &[CommandFact<'a>] {
        &self.commands
    }

    pub fn malformed_bracket_test_spans(&self, source: &str) -> Vec<Span> {
        self.commands
            .iter()
            .filter(|fact| fact.static_utility_name_is("["))
            .filter(|fact| {
                fact.body_args()
                    .last()
                    .and_then(|word| static_word_text(word, source))
                    .as_deref()
                    != Some("]")
            })
            .map(|fact| fact.body_name_word().map_or(fact.span(), |word| word.span))
            .collect()
    }

    pub fn abort_like_bracket_test_spans(&self, source: &str) -> Vec<Span> {
        self.commands
            .iter()
            .filter_map(|fact| {
                let simple_test = fact.simple_test()?;
                simple_test
                    .is_abort_like_bracket_test(source)
                    .then_some(simple_test)
            })
            .map(|simple_test| {
                simple_test
                    .effective_operator_word()
                    .map_or_else(|| simple_test.operands()[0].span, |word| word.span)
            })
            .collect()
    }

    pub fn function_positional_parameter_facts(
        &self,
        scope: ScopeId,
    ) -> FunctionPositionalParameterFacts {
        self.function_positional_parameter_facts
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub fn structural_commands(&self) -> impl Iterator<Item = &CommandFact<'a>> + '_ {
        self.structural_command_ids
            .iter()
            .copied()
            .map(|id| self.command(id))
    }

    pub fn command(&self, id: CommandId) -> &CommandFact<'a> {
        &self.commands[id.index()]
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn command_id_for_stmt(&self, stmt: &Stmt) -> Option<CommandId> {
        self.command_id_for_command(&stmt.command)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn command_id_for_command(&self, command: &Command) -> Option<CommandId> {
        command_id_for_command(command, &self.command_ids_by_span)
    }

    pub fn binding_value(&self, binding_id: BindingId) -> Option<&BindingValueFact<'a>> {
        self.binding_values.get(&binding_id)
    }

    pub(crate) fn binding_values(&self) -> &FxHashMap<BindingId, BindingValueFact<'a>> {
        &self.binding_values
    }

    pub fn broken_assoc_key_spans(&self) -> &[Span] {
        &self.broken_assoc_key_spans
    }

    pub fn comma_array_assignment_spans(&self) -> &[Span] {
        &self.comma_array_assignment_spans
    }

    pub fn ifs_literal_backslash_assignment_value_spans(&self) -> &[Span] {
        &self.ifs_literal_backslash_assignment_value_spans
    }

    pub fn env_prefix_assignment_scope_spans(&self) -> &[Span] {
        &self.env_prefix_assignment_scope_spans
    }

    pub fn is_if_condition_command(&self, id: CommandId) -> bool {
        self.if_condition_command_ids.contains(&id)
    }

    pub fn is_elif_condition_command(&self, id: CommandId) -> bool {
        self.elif_condition_command_ids.contains(&id)
    }

    pub fn presence_tested_names(&self) -> &FxHashSet<Name> {
        &self.presence_tested_names
    }

    pub fn is_presence_tested_name(&self, name: &Name, span: Span) -> bool {
        self.presence_tested_names.contains(name)
            || self
                .nested_presence_test_spans
                .get(name)
                .is_some_and(|spans| {
                    spans
                        .iter()
                        .copied()
                        .any(|outer| contains_span(outer, span))
                })
    }

    pub fn is_subscript_index_reference(&self, span: Span) -> bool {
        self.subscript_index_reference_spans
            .contains(&FactSpan::new(span))
    }

    pub fn word_facts(&self) -> &[WordFact<'a>] {
        &self.words
    }

    pub fn is_compound_assignment_value_word(&self, fact: &WordFact<'_>) -> bool {
        self.compound_assignment_value_word_spans
            .contains(&fact.key())
    }

    pub fn expansion_word_facts(
        &self,
        context: ExpansionContext,
    ) -> impl Iterator<Item = &WordFact<'a>> + '_ {
        self.words
            .iter()
            .filter(move |fact| fact.expansion_context() == Some(context))
    }

    pub fn case_subject_facts(&self) -> impl Iterator<Item = &WordFact<'a>> + '_ {
        self.words.iter().filter(|fact| fact.is_case_subject())
    }

    pub fn word_fact(&self, span: Span, context: WordFactContext) -> Option<&WordFact<'a>> {
        self.word_index
            .get(&FactSpan::new(span))
            .into_iter()
            .flat_map(|indices| indices.iter())
            .map(|&index| &self.words[index])
            .find(|fact| fact.context() == context)
    }

    pub fn any_word_fact(&self, span: Span) -> Option<&WordFact<'a>> {
        self.word_index
            .get(&FactSpan::new(span))
            .and_then(|indices| indices.first().copied())
            .map(|index| &self.words[index])
    }

    pub fn has_later_unquoted_command_argument_use(
        &self,
        name: &Name,
        after_offset: usize,
    ) -> bool {
        self.unquoted_command_argument_use_offsets
            .get(name)
            .is_some_and(|offsets| {
                offsets.partition_point(|offset| *offset <= after_offset) < offsets.len()
            })
    }

    pub fn array_assignment_split_word_facts(&self) -> impl Iterator<Item = &WordFact<'a>> + '_ {
        self.array_assignment_split_word_indices
            .iter()
            .map(|&index| &self.words[index])
    }

    pub fn function_headers(&self) -> &[FunctionHeaderFact<'a>] {
        &self.function_headers
    }

    pub fn function_body_without_braces_spans(&self) -> &[Span] {
        &self.function_body_without_braces_spans
    }

    pub fn function_parameter_fallback_spans(&self) -> &[Span] {
        &self.function_parameter_fallback_spans
    }

    pub fn redundant_return_status_spans(&self) -> &[Span] {
        &self.redundant_return_status_spans
    }

    pub fn for_headers(&self) -> &[ForHeaderFact<'a>] {
        &self.for_headers
    }

    pub fn select_headers(&self) -> &[SelectHeaderFact<'a>] {
        &self.select_headers
    }

    pub fn case_items(&self) -> &[CaseItemFact<'a>] {
        &self.case_items
    }

    pub fn case_pattern_shadows(&self) -> &[CasePatternShadowFact] {
        &self.case_pattern_shadows
    }

    pub fn getopts_cases(&self) -> &[GetoptsCaseFact] {
        &self.getopts_cases
    }

    pub fn pipelines(&self) -> &[PipelineFact<'a>] {
        &self.pipelines
    }

    pub fn lists(&self) -> &[ListFact<'a>] {
        &self.lists
    }

    pub fn statement_facts(&self) -> &[StatementFact] {
        &self.statement_facts
    }

    pub fn single_test_subshell_spans(&self) -> &[Span] {
        &self.single_test_subshell_spans
    }

    pub fn subshell_test_group_spans(&self) -> &[Span] {
        &self.subshell_test_group_spans
    }

    pub fn indented_shebang_span(&self) -> Option<Span> {
        self.indented_shebang_span
    }

    pub fn space_after_hash_bang_span(&self) -> Option<Span> {
        self.space_after_hash_bang_span
    }

    pub fn shebang_not_on_first_line_span(&self) -> Option<Span> {
        self.shebang_not_on_first_line_span
    }

    pub fn missing_shebang_line_span(&self) -> Option<Span> {
        self.missing_shebang_line_span
    }

    pub fn duplicate_shebang_flag_span(&self) -> Option<Span> {
        self.duplicate_shebang_flag_span
    }

    pub fn non_absolute_shebang_span(&self) -> Option<Span> {
        self.non_absolute_shebang_span
    }

    pub fn errexit_enabled_anywhere(&self) -> bool {
        self.errexit_enabled_anywhere
    }

    pub fn commented_continuation_comment_spans(&self) -> &[Span] {
        &self.commented_continuation_comment_spans
    }

    pub fn trailing_directive_comment_spans(&self) -> &[Span] {
        &self.trailing_directive_comment_spans
    }

    pub fn condition_status_capture_spans(&self) -> &[Span] {
        &self.condition_status_capture_spans
    }

    pub fn condition_command_substitution_spans(&self) -> &[Span] {
        &self.condition_command_substitution_spans
    }

    pub fn backtick_command_name_spans(&self) -> &[Span] {
        &self.backtick_command_name_spans
    }

    pub fn dollar_question_after_command_spans(&self) -> &[Span] {
        &self.dollar_question_after_command_spans
    }

    pub fn bare_command_name_assignment_spans(&self) -> &[Span] {
        &self.bare_command_name_assignment_spans
    }

    pub fn subshell_assignment_sites(&self) -> &[NamedSpan] {
        &self.subshell_assignment_sites
    }

    pub fn subshell_later_use_sites(&self) -> &[NamedSpan] {
        &self.subshell_later_use_sites
    }

    pub fn unused_heredoc_spans(&self) -> &[Span] {
        &self.unused_heredoc_spans
    }

    pub fn heredoc_missing_end_spans(&self) -> &[Span] {
        &self.heredoc_missing_end_spans
    }

    pub fn heredoc_closer_not_alone_spans(&self) -> &[Span] {
        &self.heredoc_closer_not_alone_spans
    }

    pub fn misquoted_heredoc_close_spans(&self) -> &[Span] {
        &self.misquoted_heredoc_close_spans
    }

    pub fn heredoc_end_space_spans(&self) -> &[Span] {
        &self.heredoc_end_space_spans
    }

    pub fn echo_here_doc_spans(&self) -> &[Span] {
        &self.echo_here_doc_spans
    }

    pub fn spaced_tabstrip_close_spans(&self) -> &[Span] {
        &self.spaced_tabstrip_close_spans
    }

    pub fn plus_equals_assignment_spans(&self) -> &[Span] {
        &self.plus_equals_assignment_spans
    }

    pub fn array_index_arithmetic_spans(&self) -> &[Span] {
        &self.array_index_arithmetic_spans
    }

    pub fn arithmetic_score_line_spans(&self) -> &[Span] {
        &self.arithmetic_score_line_spans
    }

    pub fn dollar_in_arithmetic_spans(&self) -> &[Span] {
        &self.dollar_in_arithmetic_spans
    }

    pub fn single_quoted_fragments(&self) -> &[SingleQuotedFragmentFact] {
        &self.single_quoted_fragments
    }

    pub fn dollar_double_quoted_fragments(&self) -> &[DollarDoubleQuotedFragmentFact] {
        &self.dollar_double_quoted_fragments
    }

    pub fn open_double_quote_fragments(&self) -> &[OpenDoubleQuoteFragmentFact] {
        &self.open_double_quote_fragments
    }

    pub fn suspect_closing_quote_fragments(&self) -> &[SuspectClosingQuoteFragmentFact] {
        &self.suspect_closing_quote_fragments
    }

    pub fn literal_brace_spans(&self) -> &[Span] {
        &self.literal_brace_spans
    }

    pub fn backtick_fragments(&self) -> &[BacktickFragmentFact] {
        &self.backtick_fragments
    }

    pub fn legacy_arithmetic_fragments(&self) -> &[LegacyArithmeticFragmentFact] {
        &self.legacy_arithmetic_fragments
    }

    pub fn positional_parameter_fragments(&self) -> &[PositionalParameterFragmentFact] {
        &self.positional_parameter_fragments
    }

    pub fn positional_parameter_operator_spans(&self) -> &[Span] {
        &self.positional_parameter_operator_spans
    }

    pub fn double_paren_grouping_spans(&self) -> &[Span] {
        &self.double_paren_grouping_spans
    }

    pub fn arithmetic_for_update_operator_spans(&self) -> &[Span] {
        &self.arithmetic_for_update_operator_spans
    }

    pub fn base_prefix_arithmetic_spans(&self) -> &[Span] {
        &self.base_prefix_arithmetic_spans
    }

    pub(crate) fn escape_scan_matches(&self) -> &[EscapeScanMatch] {
        &self.escape_scan_matches
    }

    pub fn echo_backslash_escape_word_spans(&self) -> &[Span] {
        &self.echo_backslash_escape_word_spans
    }

    pub fn echo_to_sed_substitution_spans(&self) -> &[Span] {
        &self.echo_to_sed_substitution_spans
    }

    pub fn arithmetic_command_substitution_spans(&self) -> &[Span] {
        &self.arithmetic_command_substitution_spans
    }
    pub fn unicode_smart_quote_spans(&self) -> &[Span] {
        &self.unicode_smart_quote_spans
    }

    pub fn pattern_exactly_one_extglob_spans(&self) -> &[Span] {
        &self.pattern_exactly_one_extglob_spans
    }

    pub fn pattern_literal_spans(&self) -> &[Span] {
        &self.pattern_literal_spans
    }

    pub fn pattern_charclass_spans(&self) -> &[Span] {
        &self.pattern_charclass_spans
    }

    pub fn is_nested_pattern_charclass_span(&self, span: Span) -> bool {
        self.nested_pattern_charclass_spans
            .contains(&FactSpan::new(span))
    }

    pub fn nested_parameter_expansion_fragments(&self) -> &[NestedParameterExpansionFragmentFact] {
        &self.nested_parameter_expansion_fragments
    }

    pub fn indirect_expansion_fragments(&self) -> &[IndirectExpansionFragmentFact] {
        &self.indirect_expansion_fragments
    }

    pub fn indexed_array_reference_fragments(&self) -> &[IndexedArrayReferenceFragmentFact] {
        &self.indexed_array_reference_fragments
    }

    pub fn substring_expansion_fragments(&self) -> &[SubstringExpansionFragmentFact] {
        &self.substring_expansion_fragments
    }

    pub fn case_modification_fragments(&self) -> &[CaseModificationFragmentFact] {
        &self.case_modification_fragments
    }

    pub fn replacement_expansion_fragments(&self) -> &[ReplacementExpansionFragmentFact] {
        &self.replacement_expansion_fragments
    }

    pub fn star_glob_removal_fragments(&self) -> &[StarGlobRemovalFragmentFact] {
        &self.star_glob_removal_fragments
    }

    pub fn conditional_portability(&self) -> &ConditionalPortabilityFacts {
        &self.conditional_portability
    }
}

struct LinterFactsBuilder<'a> {
    file: &'a File,
    source: &'a str,
    semantic: &'a SemanticModel,
    _indexer: &'a Indexer,
    _file_context: &'a FileContext,
}

#[derive(Debug, Default)]
struct ArithmeticFactSummary {
    array_index_arithmetic_spans: Vec<Span>,
    arithmetic_score_line_spans: Vec<Span>,
    dollar_in_arithmetic_spans: Vec<Span>,
    arithmetic_command_substitution_spans: Vec<Span>,
}

#[derive(Debug, Default)]
struct HeredocFactSummary {
    unused_heredoc_spans: Vec<Span>,
    heredoc_missing_end_spans: Vec<Span>,
    heredoc_closer_not_alone_spans: Vec<Span>,
    misquoted_heredoc_close_spans: Vec<Span>,
    heredoc_end_space_spans: Vec<Span>,
    echo_here_doc_spans: Vec<Span>,
    spaced_tabstrip_close_spans: Vec<Span>,
}

impl<'a> LinterFactsBuilder<'a> {
    fn new(
        file: &'a File,
        source: &'a str,
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        file_context: &'a FileContext,
    ) -> Self {
        Self {
            file,
            source,
            semantic,
            _indexer: indexer,
            _file_context: file_context,
        }
    }

    fn build(self) -> LinterFacts<'a> {
        let mut commands = Vec::new();
        let mut structural_command_ids = Vec::new();
        let mut command_ids_by_span = CommandLookupIndex::default();
        let mut if_condition_command_ids = FxHashSet::default();
        let mut elif_condition_command_ids = FxHashSet::default();
        let mut binding_values = FxHashMap::default();
        let mut broken_assoc_key_spans = Vec::new();
        let mut comma_array_assignment_spans = Vec::new();
        let mut ifs_literal_backslash_assignment_value_spans = Vec::new();
        let mut words = Vec::new();
        let mut compound_assignment_value_word_spans = FxHashSet::default();
        let mut array_assignment_split_word_indices = Vec::new();
        let mut pattern_exactly_one_extglob_spans = Vec::new();
        let mut pattern_literal_spans = Vec::new();
        let mut pattern_charclass_spans = Vec::new();
        let mut arithmetic_summary = ArithmeticFactSummary::default();
        let mut surface_fragments = SurfaceFragmentFacts::default();
        let mut functions = Vec::new();
        let mut function_body_without_braces_spans = Vec::new();
        let mut redundant_return_status_spans = Vec::new();
        let mut getopts_cases = Vec::new();
        let mut condition_status_capture_spans = Vec::new();
        let mut condition_command_substitution_spans = Vec::new();

        for traversed in query::iter_commands_with_context(
            &self.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        ) {
            let visit = traversed.visit;
            let context = traversed.context;
            let key = FactSpan::new(command_span(visit.command));
            let id = CommandId::new(commands.len());
            let lookup_kind = command_lookup_kind(visit.command);
            let entries = command_ids_by_span.entry(key).or_default();
            let previous = entries.iter().find(|entry| entry.kind == lookup_kind);
            debug_assert!(previous.is_none(), "duplicate command lookup key");
            entries.push(CommandLookupEntry {
                kind: lookup_kind,
                id,
            });

            if context.in_if_condition {
                if_condition_command_ids.insert(id);
            }
            if context.in_elif_condition {
                elif_condition_command_ids.insert(id);
            }

            collect_binding_values(
                visit.command,
                self.semantic,
                self.source,
                &mut binding_values,
            );
            collect_broken_assoc_key_spans(visit.command, self.source, &mut broken_assoc_key_spans);
            collect_comma_array_assignment_spans(
                visit.command,
                self.source,
                &mut comma_array_assignment_spans,
            );
            collect_ifs_literal_backslash_assignment_value_spans(
                visit.command,
                self.source,
                &mut ifs_literal_backslash_assignment_value_spans,
            );
            let normalized = command::normalize_command(visit.command, self.source);
            let command_zsh_options = effective_command_zsh_options(
                self.semantic,
                command_span(visit.command).start.offset,
                &normalized,
            );
            let nested_word_command = context.nested_word_command;
            if !nested_word_command {
                structural_command_ids.push(id);
            }
            let collected_words = build_word_facts_for_command(
                visit,
                self.source,
                self.semantic,
                id,
                nested_word_command,
                &normalized,
            );
            compound_assignment_value_word_spans
                .extend(collected_words.compound_assignment_value_word_spans);
            let word_index_offset = words.len();
            array_assignment_split_word_indices.extend(
                collected_words
                    .array_assignment_split_word_indices
                    .iter()
                    .map(|index| word_index_offset + *index),
            );
            words.extend(collected_words.facts);
            pattern_literal_spans.extend(collected_words.pattern_literal_spans);
            pattern_charclass_spans.extend(collected_words.pattern_charclass_spans);
            arithmetic_summary
                .array_index_arithmetic_spans
                .extend(collected_words.arithmetic.array_index_arithmetic_spans);
            arithmetic_summary
                .arithmetic_score_line_spans
                .extend(collected_words.arithmetic.arithmetic_score_line_spans);
            arithmetic_summary
                .dollar_in_arithmetic_spans
                .extend(collected_words.arithmetic.dollar_in_arithmetic_spans);
            arithmetic_summary
                .arithmetic_command_substitution_spans
                .extend(
                    collected_words
                        .arithmetic
                        .arithmetic_command_substitution_spans,
                );
            extend_surface_fragment_facts(&mut surface_fragments, collected_words.surface);
            let redirect_facts =
                build_redirect_facts(visit.redirects, self.source, command_zsh_options.as_ref());
            let options = CommandOptionFacts::build(visit.command, &normalized, self.source);
            let glued_closing_bracket_operand_span =
                build_glued_closing_bracket_operand_span(visit.command, self.source);
            let simple_test =
                build_simple_test_fact(visit.command, self.source, self._file_context);
            let conditional = build_conditional_fact(visit.command, self.source);
            commands.push(CommandFact {
                id,
                key,
                visit,
                nested_word_command,
                normalized,
                zsh_options: command_zsh_options,
                redirect_facts,
                substitution_facts: Vec::new().into_boxed_slice(),
                options,
                scope_read_source_words: Vec::new().into_boxed_slice(),
                glued_closing_bracket_operand_span,
                simple_test,
                conditional,
            });

            if let Command::Function(function) = visit.command {
                functions.push(function);
                if let Some(span) = function_body_without_braces_span(function) {
                    function_body_without_braces_spans.push(span);
                }
                collect_terminal_redundant_return_status_spans(
                    function,
                    &mut redundant_return_status_spans,
                );
            }

            if !nested_word_command {
                match visit.command {
                    Command::Compound(CompoundCommand::If(command)) => {
                        collect_condition_status_capture_from_body(
                            &command.condition,
                            &command.then_branch,
                            self.source,
                            &mut condition_status_capture_spans,
                        );

                        let mut previous_condition = &command.condition;
                        for (condition, branch) in &command.elif_branches {
                            collect_condition_status_capture_from_body(
                                previous_condition,
                                condition,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
                            collect_condition_status_capture_from_body(
                                condition,
                                branch,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
                            previous_condition = condition;
                        }

                        if let Some(else_branch) = &command.else_branch {
                            collect_condition_status_capture_from_body(
                                previous_condition,
                                else_branch,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
                        }
                    }
                    Command::Compound(CompoundCommand::While(command)) => {
                        collect_condition_status_capture_from_body(
                            &command.condition,
                            &command.body,
                            self.source,
                            &mut condition_status_capture_spans,
                        );
                        if let Some(case) = build_getopts_case_fact_for_while(command, self.source)
                        {
                            getopts_cases.push(case);
                        }
                    }
                    Command::Compound(CompoundCommand::Until(command)) => {
                        collect_condition_status_capture_from_body(
                            &command.condition,
                            &command.body,
                            self.source,
                            &mut condition_status_capture_spans,
                        );
                    }
                    Command::Binary(command)
                        if matches!(command.op, BinaryOp::And | BinaryOp::Or) =>
                    {
                        if stmt_terminals_are_test_commands(&command.left, self.source) {
                            collect_status_parameter_spans_in_stmt(
                                &command.right,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
                        }
                    }
                    Command::Simple(_)
                    | Command::Builtin(_)
                    | Command::Decl(_)
                    | Command::Binary(_)
                    | Command::Compound(_)
                    | Command::Function(_)
                    | Command::AnonymousFunction(_) => {}
                }
            }

            match visit.command {
                Command::Compound(CompoundCommand::If(command)) => {
                    collect_condition_command_substitution_from_body(
                        &command.condition,
                        self.source,
                        &mut condition_command_substitution_spans,
                    );

                    for (condition, _) in &command.elif_branches {
                        collect_condition_command_substitution_from_body(
                            condition,
                            self.source,
                            &mut condition_command_substitution_spans,
                        );
                    }
                }
                Command::Compound(CompoundCommand::While(command)) => {
                    collect_condition_command_substitution_from_body(
                        &command.condition,
                        self.source,
                        &mut condition_command_substitution_spans,
                    );
                }
                Command::Compound(CompoundCommand::Until(command)) => {
                    collect_condition_command_substitution_from_body(
                        &command.condition,
                        self.source,
                        &mut condition_command_substitution_spans,
                    );
                }
                Command::Simple(_)
                | Command::Builtin(_)
                | Command::Decl(_)
                | Command::Binary(_)
                | Command::Compound(_)
                | Command::Function(_)
                | Command::AnonymousFunction(_) => {}
            }
        }

        let substitution_facts =
            build_substitution_facts(&commands, &command_ids_by_span, self.source);
        for (fact, substitutions) in commands.iter_mut().zip(substitution_facts) {
            fact.substitution_facts = substitutions;
        }

        let presence_tested_names = build_presence_tested_names(&commands, self.source);
        let function_headers =
            build_function_header_facts(self.semantic, &functions, &commands, self.source);
        sort_and_dedup_spans(&mut condition_status_capture_spans);
        sort_and_dedup_spans(&mut condition_command_substitution_spans);
        let function_parameter_fallback_spans = build_function_parameter_fallback_spans(
            &commands,
            &structural_command_ids,
            self.source,
        );
        let for_headers = build_for_header_facts(&commands, &command_ids_by_span, self.source);
        let select_headers =
            build_select_header_facts(&commands, &command_ids_by_span, self.source);
        let case_items = build_case_item_facts(&commands);
        let case_pattern_shadows = build_case_pattern_shadow_facts(&commands, self.source);
        let pipelines = build_pipeline_facts(&commands, &command_ids_by_span);
        let scope_read_source_words =
            build_scope_read_source_words(&commands, &pipelines, &if_condition_command_ids);
        for (fact, words) in commands.iter_mut().zip(scope_read_source_words) {
            fact.scope_read_source_words = words;
        }
        let lists = build_list_facts(&commands, &command_ids_by_span, self.source);
        annotate_conditional_assignment_shortcuts(self.semantic, &lists, &mut binding_values);
        let statement_facts =
            build_statement_facts(&commands, &command_ids_by_span, &self.file.body);
        let single_test_subshell_spans =
            build_single_test_subshell_spans(&commands, &command_ids_by_span, self.source);
        let subshell_test_group_spans =
            build_subshell_test_group_spans(&commands, &command_ids_by_span, self.source);
        let shebang_header_facts = build_shebang_header_facts(self.source);
        let errexit_enabled_anywhere = shebang_header_facts.enables_errexit
            || commands
                .iter()
                .filter_map(|fact| fact.options().set())
                .any(|set| set.errexit_change == Some(true));
        let commented_continuation_comment_spans =
            build_commented_continuation_comment_spans(self.source, self._indexer);
        let trailing_directive_comment_spans =
            build_trailing_directive_comment_spans(self.source, self._indexer);
        let backtick_command_name_spans = build_backtick_command_name_spans(&commands);
        let dollar_question_after_command_spans =
            build_dollar_question_after_command_spans(&self.file.body, self.source);
        let nonpersistent_assignment_spans =
            build_nonpersistent_assignment_spans(self.semantic, &commands);
        let heredoc_summary =
            build_heredoc_fact_summary(&commands, self.source, self.file.span.end.offset);
        let plus_equals_assignment_spans = build_plus_equals_assignment_spans(&commands);
        let literal_brace_spans = build_literal_brace_spans(
            &words,
            &commands,
            self.source,
            self._indexer.region_index().heredoc_ranges(),
        );
        let SurfaceFragmentFacts {
            single_quoted,
            dollar_double_quoted,
            open_double_quotes,
            suspect_closing_quotes,
            backticks,
            legacy_arithmetic,
            positional_parameters,
            positional_parameter_operator_spans,
            unicode_smart_quote_spans,
            pattern_exactly_one_extglob_spans: surface_pattern_exactly_one_extglob_spans,
            pattern_charclass_spans: surface_pattern_charclass_spans,
            nested_pattern_charclass_spans,
            nested_parameter_expansions,
            indirect_expansions,
            indexed_array_references,
            substring_expansions,
            case_modifications,
            replacement_expansions,
            star_glob_removals,
            subscript_spans,
        } = surface_fragments;
        let function_positional_parameter_facts = build_function_positional_parameter_facts(
            self.semantic,
            &commands,
            &positional_parameters,
        );
        let double_paren_grouping_spans = build_double_paren_grouping_spans(&commands, self.source);
        let arithmetic_for_update_operator_spans =
            build_arithmetic_for_update_operator_spans(&commands, self.source);
        let base_prefix_arithmetic_spans =
            build_base_prefix_arithmetic_spans(&self.file.body, self.source);
        let subscript_index_reference_spans =
            build_subscript_index_reference_spans(self.semantic, &subscript_spans);
        pattern_exactly_one_extglob_spans.extend(surface_pattern_exactly_one_extglob_spans);
        pattern_charclass_spans.extend(surface_pattern_charclass_spans);
        let escape_scan_matches = build_escape_scan_matches(
            &commands,
            &words,
            &pattern_literal_spans,
            &pattern_charclass_spans,
            &single_quoted,
            &backticks,
            EscapeScanContext {
                source: self.source,
                file_context: self._file_context,
            },
        );
        let echo_backslash_escape_word_spans =
            build_echo_backslash_escape_word_spans(&commands, self.source);
        let nested_pattern_charclass_spans = nested_pattern_charclass_spans
            .into_iter()
            .map(FactSpan::new)
            .collect();
        let conditional_portability = build_conditional_portability_facts(
            &commands,
            &elif_condition_command_ids,
            &words,
            &pattern_exactly_one_extglob_spans,
            &pattern_charclass_spans,
            &nested_pattern_charclass_spans,
            self.source,
        );
        let env_prefix_assignment_scope_spans =
            build_env_prefix_assignment_scope_spans(self.semantic, self.source, &commands);
        let mut word_index = FxHashMap::<FactSpan, Vec<usize>>::default();
        for (index, fact) in words.iter().enumerate() {
            word_index.entry(fact.key()).or_default().push(index);
        }
        let echo_to_sed_substitution_spans = build_echo_to_sed_substitution_spans(
            &commands,
            &pipelines,
            &backticks,
            &words,
            &word_index,
            self.source,
        );
        let bare_command_name_assignment_spans =
            build_bare_command_name_assignment_spans(&commands, &words, &word_index, self.source);
        let unquoted_command_argument_use_offsets =
            build_unquoted_command_argument_use_offsets(self.semantic, &words);

        LinterFacts {
            commands,
            structural_command_ids,
            command_ids_by_span,
            if_condition_command_ids,
            elif_condition_command_ids,
            binding_values,
            broken_assoc_key_spans,
            comma_array_assignment_spans,
            ifs_literal_backslash_assignment_value_spans,
            env_prefix_assignment_scope_spans,
            presence_tested_names: presence_tested_names.global_names,
            nested_presence_test_spans: presence_tested_names.nested_command_spans_by_name,
            subscript_index_reference_spans,
            compound_assignment_value_word_spans,
            words,
            word_index,
            unquoted_command_argument_use_offsets,
            array_assignment_split_word_indices,
            function_headers,
            function_body_without_braces_spans,
            function_parameter_fallback_spans,
            redundant_return_status_spans,
            for_headers,
            select_headers,
            case_items,
            case_pattern_shadows,
            getopts_cases,
            pipelines,
            lists,
            statement_facts,
            single_test_subshell_spans,
            subshell_test_group_spans,
            indented_shebang_span: shebang_header_facts.indented_shebang_span,
            space_after_hash_bang_span: shebang_header_facts.space_after_hash_bang_span,
            shebang_not_on_first_line_span: shebang_header_facts.shebang_not_on_first_line_span,
            missing_shebang_line_span: shebang_header_facts.missing_shebang_line_span,
            duplicate_shebang_flag_span: shebang_header_facts.duplicate_shebang_flag_span,
            non_absolute_shebang_span: shebang_header_facts.non_absolute_shebang_span,
            errexit_enabled_anywhere,
            commented_continuation_comment_spans,
            trailing_directive_comment_spans,
            condition_status_capture_spans,
            condition_command_substitution_spans,
            backtick_command_name_spans,
            dollar_question_after_command_spans,
            bare_command_name_assignment_spans,
            subshell_assignment_sites: nonpersistent_assignment_spans.subshell_assignment_sites,
            subshell_later_use_sites: nonpersistent_assignment_spans.subshell_later_use_sites,
            unused_heredoc_spans: heredoc_summary.unused_heredoc_spans,
            heredoc_missing_end_spans: heredoc_summary.heredoc_missing_end_spans,
            heredoc_closer_not_alone_spans: heredoc_summary.heredoc_closer_not_alone_spans,
            misquoted_heredoc_close_spans: heredoc_summary.misquoted_heredoc_close_spans,
            heredoc_end_space_spans: heredoc_summary.heredoc_end_space_spans,
            echo_here_doc_spans: heredoc_summary.echo_here_doc_spans,
            spaced_tabstrip_close_spans: heredoc_summary.spaced_tabstrip_close_spans,
            plus_equals_assignment_spans,
            array_index_arithmetic_spans: arithmetic_summary.array_index_arithmetic_spans,
            arithmetic_score_line_spans: arithmetic_summary.arithmetic_score_line_spans,
            dollar_in_arithmetic_spans: arithmetic_summary.dollar_in_arithmetic_spans,
            arithmetic_command_substitution_spans: arithmetic_summary
                .arithmetic_command_substitution_spans,
            function_positional_parameter_facts,
            single_quoted_fragments: single_quoted,
            dollar_double_quoted_fragments: dollar_double_quoted,
            open_double_quote_fragments: open_double_quotes,
            suspect_closing_quote_fragments: suspect_closing_quotes,
            literal_brace_spans,
            backtick_fragments: backticks,
            legacy_arithmetic_fragments: legacy_arithmetic,
            positional_parameter_fragments: positional_parameters,
            positional_parameter_operator_spans,
            double_paren_grouping_spans,
            arithmetic_for_update_operator_spans,
            base_prefix_arithmetic_spans,
            escape_scan_matches,
            echo_backslash_escape_word_spans,
            echo_to_sed_substitution_spans,
            unicode_smart_quote_spans,
            pattern_exactly_one_extglob_spans,
            pattern_literal_spans,
            pattern_charclass_spans,
            nested_pattern_charclass_spans,
            nested_parameter_expansion_fragments: nested_parameter_expansions,
            indirect_expansion_fragments: indirect_expansions,
            indexed_array_reference_fragments: indexed_array_references,
            substring_expansion_fragments: substring_expansions,
            case_modification_fragments: case_modifications,
            replacement_expansion_fragments: replacement_expansions,
            star_glob_removal_fragments: star_glob_removals,
            conditional_portability,
        }
    }
}

fn build_echo_backslash_escape_word_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("echo") && fact.wrappers().is_empty())
        .filter(|fact| !echo_uses_escape_interpreting_flag(fact))
        .flat_map(|fact| fact.body_args().iter().copied())
        .filter(|word| word_contains_echo_backslash_escape(word, source))
        .map(|word| word.span)
        .collect::<Vec<_>>();

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans
}

fn build_echo_to_sed_substitution_spans<'a>(
    commands: &[CommandFact<'a>],
    pipelines: &[PipelineFact<'a>],
    backticks: &[BacktickFragmentFact],
    words: &[WordFact<'a>],
    word_index: &FxHashMap<FactSpan, Vec<usize>>,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut pipeline_sed_command_ids = FxHashSet::default();

    for pipeline in pipelines {
        if let Some(span) =
            sc2001_like_pipeline_span(commands, pipeline, backticks, words, word_index, source)
        {
            spans.push(span);
            if let Some(last_segment) = pipeline.last_segment() {
                pipeline_sed_command_ids.insert(last_segment.command_id());
            }
        }
    }

    spans.extend(commands.iter().filter_map(|command| {
        (!pipeline_sed_command_ids.contains(&command.id()))
            .then(|| sc2001_like_here_string_span(command, backticks, source))
            .flatten()
    }));

    sort_and_dedup_spans(&mut spans);
    spans
}

fn sc2001_like_pipeline_span<'a>(
    commands: &[CommandFact<'a>],
    pipeline: &PipelineFact<'a>,
    backticks: &[BacktickFragmentFact],
    words: &[WordFact<'a>],
    word_index: &FxHashMap<FactSpan, Vec<usize>>,
    source: &str,
) -> Option<Span> {
    let [left_segment, right_segment] = pipeline.segments() else {
        return None;
    };

    let left = command_fact(commands, left_segment.command_id());
    let right = command_fact(commands, right_segment.command_id());

    if !command_is_plain_named(left, "echo") || !command_is_plain_named(right, "sed") {
        return None;
    }

    if left
        .options()
        .echo()
        .and_then(|echo| echo.portability_flag_word())
        .is_some()
    {
        return None;
    }

    if !command_has_sc2001_like_sed_script(right, backticks, source) {
        return None;
    }

    let [argument] = left.body_args() else {
        return None;
    };

    let word_fact = word_fact_with_context(
        words,
        word_index,
        argument.span,
        WordFactContext::Expansion(ExpansionContext::CommandArgument),
    )?;

    if word_fact.static_text().is_some() {
        return None;
    }

    if word_fact.scalar_expansion_spans().is_empty()
        && word_fact.array_expansion_spans().is_empty()
        && word_fact.command_substitution_spans().is_empty()
    {
        return None;
    }

    if word_fact.has_literal_affixes() && !word_fact_is_pure_quoted_dynamic(word_fact, source) {
        return None;
    }

    Some(pipeline_span_with_shellcheck_tail(
        commands, pipeline, source,
    ))
}

fn sc2001_like_here_string_span(
    command: &CommandFact<'_>,
    backticks: &[BacktickFragmentFact],
    source: &str,
) -> Option<Span> {
    if !command_is_plain_named(command, "sed") {
        return None;
    }

    if !command_has_sc2001_like_sed_script(command, backticks, source) {
        return None;
    }

    let mut here_strings = command
        .redirect_facts()
        .iter()
        .filter(|redirect| redirect.redirect().kind == RedirectKind::HereString);
    here_strings.next()?;
    if here_strings.next().is_some() {
        return None;
    }

    command_span_with_redirects_and_shellcheck_tail(command, source)
}

fn command_is_plain_named(command: &CommandFact<'_>, name: &str) -> bool {
    command.effective_name_is(name) && command.wrappers().is_empty()
}

fn command_has_sc2001_like_sed_script(
    command: &CommandFact<'_>,
    backticks: &[BacktickFragmentFact],
    source: &str,
) -> bool {
    command
        .options()
        .sed()
        .is_some_and(|sed| sed.has_single_substitution_script())
        || (command_is_inside_backtick_fragment(command, backticks)
            && sed_has_single_substitution_script(
                command.body_args(),
                source,
                SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes,
            ))
}

fn command_is_inside_backtick_fragment(
    command: &CommandFact<'_>,
    backticks: &[BacktickFragmentFact],
) -> bool {
    let span = command.span();
    backticks.iter().any(|fragment| {
        let fragment_span = fragment.span();
        fragment_span.start.offset <= span.start.offset
            && fragment_span.end.offset >= span.end.offset
    })
}

fn word_fact_with_context<'a>(
    words: &'a [WordFact<'a>],
    word_index: &FxHashMap<FactSpan, Vec<usize>>,
    span: Span,
    context: WordFactContext,
) -> Option<&'a WordFact<'a>> {
    word_index
        .get(&FactSpan::new(span))
        .into_iter()
        .flat_map(|indices| indices.iter())
        .map(|&index| &words[index])
        .find(|fact| fact.context() == context)
}

fn word_fact_is_pure_quoted_dynamic(fact: &WordFact<'_>, source: &str) -> bool {
    !span::word_double_quoted_scalar_only_expansion_spans(fact.word()).is_empty()
        || !span::word_quoted_all_elements_array_slice_spans(fact.word()).is_empty()
        || word_fact_is_double_quoted_command_substitution_only(fact, source)
        || word_fact_is_backtick_escaped_double_quoted_dynamic(fact, source)
}

fn build_unquoted_literal_between_double_quoted_segments_spans(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let nested_fragment_parts = mixed_quote_word_parts_inside_nested_shell_fragments(word, source);

    let mut spans = word
        .parts
        .windows(3)
        .enumerate()
        .filter_map(|(window_index, window)| {
            let [left, middle, right] = window else {
                return None;
            };
            let WordPart::DoubleQuoted {
                parts: left_inner, ..
            } = &left.kind
            else {
                return None;
            };
            let WordPart::Literal(text) = &middle.kind else {
                return None;
            };
            let WordPart::DoubleQuoted {
                parts: right_inner, ..
            } = &right.kind
            else {
                return None;
            };

            let neighbor_has_literal =
                mixed_quote_double_quoted_parts_contain_literal_content(left_inner)
                    || mixed_quote_double_quoted_parts_contain_literal_content(right_inner);
            let middle_is_nested = nested_fragment_parts
                .get(window_index + 1)
                .copied()
                .unwrap_or(false);
            (neighbor_has_literal
                && !middle_is_nested
                && mixed_quote_literal_is_warnable_between_double_quotes(
                    text.as_str(source, middle.span),
                ))
            .then_some(middle.span)
        })
        .collect::<Vec<_>>();

    if let Some(span) = mixed_quote_trailing_line_join_between_double_quotes_span(word, source)
        && !spans.contains(&span)
    {
        spans.push(span);
    }

    spans
}

fn mixed_quote_double_quoted_parts_contain_literal_content(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => {
            mixed_quote_double_quoted_parts_contain_literal_content(parts)
        }
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
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
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

fn mixed_quote_literal_is_warnable_between_double_quotes(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    if text == "\"" {
        return true;
    }

    if matches!(text, "\\\n" | "\\\r\n") {
        return true;
    }

    if text == "/,/" {
        return true;
    }

    if text.chars().all(|ch| matches!(ch, '\\' | '"')) && text.contains('\\') {
        return true;
    }

    if text.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return !text.chars().any(char::is_whitespace);
    }

    if text.chars().all(|ch| ch == ':') {
        return text.len() > 1;
    }

    text.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '+' | '-' | '%' | ':')
    })
}

fn mixed_quote_word_parts_inside_nested_shell_fragments(word: &Word, source: &str) -> Vec<bool> {
    let mut command_depth = 0i32;
    let mut parameter_depth = 0i32;
    let mut nested = Vec::with_capacity(word.parts.len());

    for part in &word.parts {
        nested.push(command_depth > 0 || parameter_depth > 0);

        let (command_delta, parameter_delta) =
            mixed_quote_shell_fragment_balance_delta_for_part(part, source);
        command_depth += command_delta;
        parameter_depth += parameter_delta;
        command_depth = command_depth.max(0);
        parameter_depth = parameter_depth.max(0);
    }

    nested
}

fn mixed_quote_shell_fragment_balance_delta_for_part(
    part: &WordPartNode,
    source: &str,
) -> (i32, i32) {
    match &part.kind {
        WordPart::CommandSubstitution {
            syntax: CommandSubstitutionSyntax::Backtick,
            ..
        } => {
            let text = part.span.slice(source);
            let body = text
                .strip_prefix('`')
                .and_then(|text| text.strip_suffix('`'))
                .unwrap_or(text);
            mixed_quote_shell_fragment_balance_delta(body, true)
        }
        WordPart::ProcessSubstitution { .. } => {
            mixed_quote_shell_fragment_balance_delta(part.span.slice(source), true)
        }
        _ => mixed_quote_shell_fragment_balance_delta(part.span.slice(source), false),
    }
}

fn mixed_quote_shell_fragment_balance_delta(
    text: &str,
    allow_top_level_command_comments: bool,
) -> (i32, i32) {
    let mut command_delta = 0i32;
    let mut parameter_delta = 0i32;
    let mut chars = text.chars().peekable();
    let mut escaped = false;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut in_comment = false;
    let mut previous_char = None;

    while let Some(ch) = chars.next() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                previous_char = Some(ch);
            }
            continue;
        }

        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            previous_char = Some(ch);
            continue;
        }

        if escaped {
            escaped = false;
            previous_char = Some(ch);
            continue;
        }

        if ch == '\'' && !in_double_quotes {
            in_single_quotes = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '"' {
            in_double_quotes = !in_double_quotes;
            previous_char = Some(ch);
            continue;
        }

        if ch == '\\' {
            escaped = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '#'
            && !in_double_quotes
            && mixed_quote_shell_comment_can_start(
                command_delta,
                allow_top_level_command_comments,
                previous_char,
            )
        {
            in_comment = true;
            continue;
        }

        if ch == '$' {
            match chars.peek().copied() {
                Some('(') => {
                    command_delta += 1;
                    chars.next();
                    previous_char = Some('(');
                    continue;
                }
                Some('{') => {
                    parameter_delta += 1;
                    chars.next();
                    previous_char = Some('{');
                    continue;
                }
                _ => {}
            }
        }

        match ch {
            ')' => command_delta -= 1,
            '}' => parameter_delta -= 1,
            _ => {}
        }

        previous_char = Some(ch);
    }

    (command_delta, parameter_delta)
}

fn mixed_quote_shell_comment_can_start(
    command_depth: i32,
    allow_top_level_command_comments: bool,
    previous_char: Option<char>,
) -> bool {
    (command_depth > 0 || allow_top_level_command_comments)
        && previous_char.is_none_or(|ch| {
            ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>')
        })
}

fn mixed_quote_trailing_line_join_between_double_quotes_span(
    word: &Word,
    source: &str,
) -> Option<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return None;
    }

    let text = word.span.slice(source);
    let (prefix, suffix) = if let Some(prefix) = text.strip_suffix("\\\n") {
        (prefix, "\\\n")
    } else if let Some(prefix) = text.strip_suffix("\\\r\n") {
        (prefix, "\\\r\n")
    } else {
        return None;
    };

    if !source[word.span.end.offset..].starts_with('"') {
        return None;
    }

    let start = word.span.start.advanced_by(prefix);
    Some(Span::from_positions(start, start.advanced_by(suffix)))
}

fn build_bare_command_name_assignment_spans<'a>(
    commands: &[CommandFact<'a>],
    words: &[WordFact<'a>],
    word_index: &FxHashMap<FactSpan, Vec<usize>>,
    source: &str,
) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|command| bare_command_name_assignment_span(command, words, word_index, source))
        .collect()
}

fn bare_command_name_assignment_span<'a>(
    command: &CommandFact<'a>,
    words: &[WordFact<'a>],
    word_index: &FxHashMap<FactSpan, Vec<usize>>,
    source: &str,
) -> Option<Span> {
    let (assignment, anchor_full_command) = match command.command() {
        Command::Simple(simple) if simple.assignments.len() == 1 => (
            &simple.assignments[0],
            !simple.name.span.slice(source).is_empty(),
        ),
        Command::Builtin(BuiltinCommand::Break(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Builtin(BuiltinCommand::Continue(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Builtin(BuiltinCommand::Return(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Builtin(BuiltinCommand::Exit(builtin)) if builtin.assignments.len() == 1 => {
            (&builtin.assignments[0], true)
        }
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => return None,
    };

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    let fact = word_fact_with_context(
        words,
        word_index,
        word.span,
        WordFactContext::Expansion(ExpansionContext::AssignmentValue),
    )?;
    if fact.classification().quote != WordQuote::Unquoted
        || !fact.classification().is_fixed_literal()
    {
        return None;
    }

    let text = fact.static_text()?;
    if !is_bare_command_name_assignment_value(text) {
        return None;
    }

    Some(if anchor_full_command {
        anchored_assignment_command_span(command, assignment, source)
    } else {
        assignment_target_span(assignment)
    })
}

fn anchored_assignment_command_span(
    command: &CommandFact<'_>,
    assignment: &Assignment,
    source: &str,
) -> Span {
    match command.command() {
        Command::Builtin(_) => return command.span_in_source(source),
        Command::Simple(simple) => {
            let end = simple
                .args
                .last()
                .map(|word| word.span.end)
                .unwrap_or(simple.name.span.end);

            return Span {
                start: assignment.span.start,
                end,
            };
        }
        Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }

    Span {
        start: assignment.span.start,
        end: assignment.span.end,
    }
}

fn assignment_target_span(assignment: &Assignment) -> Span {
    assignment.target.subscript.as_ref().map_or_else(
        || assignment.target.name_span,
        |subscript| {
            Span::from_positions(
                assignment.target.name_span.start,
                subscript.span().end.advanced_by("]"),
            )
        },
    )
}

fn is_bare_command_name_assignment_value(text: &str) -> bool {
    matches!(
        text,
        "admin"
            | "alias"
            | "awk"
            | "basename"
            | "bg"
            | "break"
            | "c99"
            | "cat"
            | "cd"
            | "cflow"
            | "chmod"
            | "chown"
            | "cksum"
            | "cmp"
            | "comm"
            | "command"
            | "compress"
            | "continue"
            | "cp"
            | "csplit"
            | "ctags"
            | "cut"
            | "cxref"
            | "date"
            | "dd"
            | "delta"
            | "df"
            | "dirname"
            | "du"
            | "echo"
            | "env"
            | "eval"
            | "ex"
            | "exec"
            | "exit"
            | "expand"
            | "export"
            | "expr"
            | "file"
            | "fg"
            | "find"
            | "fold"
            | "getopts"
            | "grep"
            | "hash"
            | "head"
            | "jobs"
            | "join"
            | "kill"
            | "link"
            | "ln"
            | "ls"
            | "m4"
            | "make"
            | "mkdir"
            | "mkfifo"
            | "more"
            | "mv"
            | "nm"
            | "nice"
            | "nl"
            | "nohup"
            | "od"
            | "paste"
            | "patch"
            | "pathchk"
            | "pax"
            | "printf"
            | "pwd"
            | "read"
            | "readonly"
            | "renice"
            | "return"
            | "rm"
            | "rmdir"
            | "sed"
            | "set"
            | "shift"
            | "sh"
            | "sleep"
            | "sort"
            | "split"
            | "strings"
            | "tail"
            | "test"
            | "time"
            | "touch"
            | "tr"
            | "trap"
            | "tty"
            | "type"
            | "ulimit"
            | "umask"
            | "unalias"
            | "uname"
            | "unexpand"
            | "uniq"
            | "unlink"
            | "unset"
            | "wait"
            | "wc"
            | "xargs"
            | "zcat"
    )
}

fn word_fact_is_double_quoted_command_substitution_only(fact: &WordFact<'_>, source: &str) -> bool {
    let [command_substitution] = fact.command_substitution_spans() else {
        return false;
    };

    if !fact.scalar_expansion_spans().is_empty() || !fact.array_expansion_spans().is_empty() {
        return false;
    }

    let word_text = fact.span().slice(source);
    word_text.len() == command_substitution.slice(source).len() + 2
        && word_text.starts_with('"')
        && word_text.ends_with('"')
        && &word_text[1..word_text.len() - 1] == command_substitution.slice(source)
}

fn word_fact_is_backtick_escaped_double_quoted_dynamic(fact: &WordFact<'_>, source: &str) -> bool {
    let word_text = fact.span().slice(source);
    if !word_text.starts_with("\\\"") || !word_text.ends_with("\\\"") {
        return false;
    }

    let inner = &word_text[2..word_text.len() - 2];
    match (
        fact.scalar_expansion_spans(),
        fact.array_expansion_spans(),
        fact.command_substitution_spans(),
    ) {
        ([scalar], [], []) => inner == scalar.slice(source),
        ([], [array], []) => inner == array.slice(source),
        ([], [], [command_substitution]) => inner == command_substitution.slice(source),
        _ => false,
    }
}

fn pipeline_span_with_shellcheck_tail(
    commands: &[CommandFact<'_>],
    pipeline: &PipelineFact<'_>,
    source: &str,
) -> Span {
    let first = command_fact(
        commands,
        pipeline
            .first_segment()
            .expect("pipeline has segments")
            .command_id(),
    );
    let last = command_fact(
        commands,
        pipeline
            .last_segment()
            .expect("pipeline has segments")
            .command_id(),
    );
    let last_end = last.span_in_source(source).end;
    let end = extend_over_shellcheck_trailing_inline_space(last_end, source);

    Span::from_positions(
        first
            .body_name_word()
            .expect("plain echo command should have a body name")
            .span
            .start,
        end,
    )
}

fn command_span_with_redirects_and_shellcheck_tail(
    command: &CommandFact<'_>,
    source: &str,
) -> Option<Span> {
    let body_name = command.body_name_word()?;
    let mut end = body_name.span.end;

    for word in command.body_args() {
        if word.span.end.offset > end.offset {
            end = word.span.end;
        }
    }

    for redirect in command.redirect_facts() {
        let redirect_end = redirect.redirect().span.end;
        if redirect_end.offset > end.offset {
            end = redirect_end;
        }
    }

    Some(Span::from_positions(
        body_name.span.start,
        extend_over_shellcheck_trailing_inline_space(end, source),
    ))
}

fn extend_over_shellcheck_trailing_inline_space(end: Position, source: &str) -> Position {
    let tail = &source[end.offset..];
    let spaces_len = tail
        .char_indices()
        .take_while(|(_, ch)| matches!(ch, ' ' | '\t'))
        .last()
        .map_or(0, |(index, ch)| index + ch.len_utf8());

    if spaces_len == 0 {
        return end;
    }

    let rest = &tail[spaces_len..];
    if rest.is_empty()
        || rest.starts_with('\n')
        || rest.starts_with('\r')
        || rest.starts_with(')')
        || rest.starts_with(']')
        || rest.starts_with('}')
    {
        end.advanced_by(&tail[..spaces_len])
    } else {
        end
    }
}

fn build_unquoted_command_argument_use_offsets(
    semantic: &SemanticModel,
    words: &[WordFact<'_>],
) -> FxHashMap<Name, Vec<usize>> {
    let unquoted_command_argument_word_spans = words
        .iter()
        .filter(|fact| fact.expansion_context() == Some(ExpansionContext::CommandArgument))
        .filter(|fact| fact.classification().quote == WordQuote::Unquoted)
        .map(WordFact::span)
        .collect::<Vec<_>>();
    if unquoted_command_argument_word_spans.is_empty() {
        return FxHashMap::default();
    }

    let references = semantic.references();
    let mut reference_indices = references
        .iter()
        .enumerate()
        .filter(|(_, reference)| {
            !matches!(
                reference.kind,
                shuck_semantic::ReferenceKind::DeclarationName
            )
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    reference_indices.sort_unstable_by_key(|&index| references[index].span.start.offset);

    let mut offsets_by_name = FxHashMap::<Name, Vec<usize>>::default();
    for word_span in unquoted_command_argument_word_spans {
        let first_reference = reference_indices
            .partition_point(|&index| references[index].span.start.offset < word_span.start.offset);
        for &index in &reference_indices[first_reference..] {
            let reference = &references[index];
            if reference.span.start.offset > word_span.end.offset {
                break;
            }
            if !contains_span(word_span, reference.span) {
                continue;
            }

            offsets_by_name
                .entry(reference.name.clone())
                .or_default()
                .push(word_span.start.offset);
        }
    }

    for offsets in offsets_by_name.values_mut() {
        offsets.sort_unstable();
        offsets.dedup();
    }

    offsets_by_name
}

fn build_env_prefix_assignment_scope_spans(
    _semantic: &SemanticModel,
    source: &str,
    commands: &[CommandFact<'_>],
) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut seen = FxHashSet::default();
    for command in commands {
        if command_is_assignment_only(command, source) {
            continue;
        }

        let assignments = query::command_assignments(command.command());
        let broken_legacy_bracket_tail = match command.command() {
            Command::Simple(simple) => broken_legacy_bracket_tail(simple, source),
            Command::Builtin(_)
            | Command::Decl(_)
            | Command::Binary(_)
            | Command::Compound(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => None,
        };
        for (index, assignment) in assignments.iter().enumerate() {
            let span_key = FactSpan::new(assignment.target.name_span);
            if seen.contains(&span_key) {
                continue;
            }

            let identity_self_copy = assignment_is_identity_self_copy(assignment);
            let earlier_prefix_uses_name = assignments.iter().take(index).any(|other| {
                assignment_mentions_name_outside_nested_commands(other, &assignment.target.name)
            });
            let later_prefix_uses_name =
                assignments
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .any(|(other_index, other)| {
                        assignment_mentions_name_outside_nested_commands(
                            other,
                            &assignment.target.name,
                        ) || match (command.command(), broken_legacy_bracket_tail) {
                            (Command::Simple(simple), Some(tail))
                                if tail.assignment_index == other_index =>
                            {
                                broken_legacy_bracket_tail_mentions_name(
                                    simple,
                                    tail,
                                    &assignment.target.name,
                                )
                            }
                            (
                                Command::Builtin(_)
                                | Command::Decl(_)
                                | Command::Binary(_)
                                | Command::Compound(_)
                                | Command::Function(_)
                                | Command::AnonymousFunction(_),
                                _,
                            )
                            | (Command::Simple(_), _) => false,
                        }
                    });
            let body_uses_name = command_body_mentions_name_outside_nested_commands(
                command,
                source,
                &assignment.target.name,
            );

            if earlier_prefix_uses_name
                || later_prefix_uses_name
                || (body_uses_name && !identity_self_copy)
            {
                seen.insert(span_key);
                spans.push(assignment.target.name_span);
            }
        }
    }

    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

#[derive(Debug, Clone, Copy)]
struct BrokenLegacyBracketTail {
    assignment_index: usize,
    synthetic_word_count: usize,
}

fn command_is_assignment_only(fact: &CommandFact<'_>, source: &str) -> bool {
    match fact.command() {
        Command::Simple(command) if !command.assignments.is_empty() => {
            fact.literal_name() == Some("")
                || broken_legacy_bracket_tail(command, source)
                    .is_some_and(|tail| tail.synthetic_word_count == command.args.len() + 1)
        }
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn broken_legacy_bracket_tail(
    command: &SimpleCommand,
    source: &str,
) -> Option<BrokenLegacyBracketTail> {
    let assignment_index = command.assignments.len().checked_sub(1)?;
    if !assignment_is_broken_legacy_bracket_arithmetic(&command.assignments[assignment_index]) {
        return None;
    }

    let synthetic_word_count = std::iter::once(&command.name)
        .chain(command.args.iter())
        .position(|word| static_word_text(word, source).as_deref() == Some("]"))?
        + 1;

    Some(BrokenLegacyBracketTail {
        assignment_index,
        synthetic_word_count,
    })
}

fn assignment_is_broken_legacy_bracket_arithmetic(assignment: &Assignment) -> bool {
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return false;
    };
    let [part] = word.parts.as_slice() else {
        return false;
    };
    matches!(
        &part.kind,
        WordPart::ArithmeticExpansion {
            syntax: ArithmeticExpansionSyntax::LegacyBracket,
            expression_ast: None,
            ..
        }
    )
}

fn assignment_mentions_name_outside_nested_commands(assignment: &Assignment, name: &Name) -> bool {
    subscript_mentions_name(assignment.target.subscript.as_ref(), name)
        || match &assignment.value {
            AssignmentValue::Scalar(word) => word_mentions_name_outside_nested_commands(word, name),
            AssignmentValue::Compound(array) => {
                array.elements.iter().any(|element| match element {
                    ArrayElem::Sequential(word) => {
                        word_mentions_name_outside_nested_commands(word, name)
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        subscript_mentions_name(Some(key), name)
                            || word_mentions_name_outside_nested_commands(value, name)
                    }
                })
            }
        }
}

fn command_body_mentions_name_outside_nested_commands(
    fact: &CommandFact<'_>,
    source: &str,
    name: &Name,
) -> bool {
    let redirects_mention_name = fact
        .redirects()
        .iter()
        .filter_map(Redirect::word_target)
        .any(|word| word_mentions_name_outside_nested_commands(word, name));

    match fact.command() {
        Command::Simple(command) => {
            simple_command_body_words(command, source)
                .any(|word| word_mentions_name_outside_nested_commands(word, name))
                || redirects_mention_name
        }
        Command::Builtin(command) => {
            builtin_words(command)
                .into_iter()
                .any(|word| word_mentions_name_outside_nested_commands(word, name))
                || redirects_mention_name
        }
        Command::Decl(command) => {
            command.operands.iter().any(|operand| match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                    word_mentions_name_outside_nested_commands(word, name)
                }
                DeclOperand::Assignment(assignment) => {
                    assignment_mentions_name_outside_nested_commands(assignment, name)
                }
                DeclOperand::Name(_) => false,
            }) || redirects_mention_name
        }
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn simple_command_body_words<'a>(
    command: &'a SimpleCommand,
    source: &'a str,
) -> impl Iterator<Item = &'a Word> {
    let skip =
        broken_legacy_bracket_tail(command, source).map_or(0, |tail| tail.synthetic_word_count);
    std::iter::once(&command.name)
        .chain(command.args.iter())
        .skip(skip)
}

fn broken_legacy_bracket_tail_mentions_name(
    command: &SimpleCommand,
    tail: BrokenLegacyBracketTail,
    name: &Name,
) -> bool {
    std::iter::once(&command.name)
        .chain(command.args.iter())
        .take(tail.synthetic_word_count.saturating_sub(1))
        .any(|word| word_mentions_name_outside_nested_commands(word, name))
}

fn builtin_words(command: &BuiltinCommand) -> Vec<&Word> {
    let mut words = Vec::new();
    match command {
        BuiltinCommand::Break(command) => {
            if let Some(word) = &command.depth {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
        BuiltinCommand::Continue(command) => {
            if let Some(word) = &command.depth {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
        BuiltinCommand::Return(command) => {
            if let Some(word) = &command.code {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
        BuiltinCommand::Exit(command) => {
            if let Some(word) = &command.code {
                words.push(word);
            }
            words.extend(command.extra_args.iter());
        }
    }
    words
}

fn assignment_is_identity_self_copy(assignment: &Assignment) -> bool {
    if assignment.append {
        return false;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return false;
    };
    word_is_identity_self_copy(word, &assignment.target.name)
}

fn word_is_identity_self_copy(word: &Word, name: &Name) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_identity_self_copy(&part.kind, name)
}

fn word_part_is_identity_self_copy(part: &WordPart, name: &Name) -> bool {
    match part {
        WordPart::Variable(variable) => variable == name,
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return false;
            };
            word_part_is_identity_self_copy(&part.kind, name)
        }
        WordPart::Parameter(parameter) => parameter_is_plain_access_to_name(parameter, name),
        _ => false,
    }
}

fn parameter_is_plain_access_to_name(parameter: &ParameterExpansion, name: &Name) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.subscript.is_none() =>
        {
            &reference.name == name
        }
        ParameterExpansionSyntax::Zsh(syntax)
            if syntax.operation.is_none()
                && matches!(&syntax.target, ZshExpansionTarget::Reference(reference) if reference.subscript.is_none() && &reference.name == name) =>
        {
            true
        }
        _ => false,
    }
}

fn subscript_mentions_name(subscript: Option<&Subscript>, name: &Name) -> bool {
    let Some(subscript) = subscript else {
        return false;
    };

    subscript
        .word_ast
        .as_ref()
        .is_some_and(|word| word_mentions_name_outside_nested_commands(word, name))
        || subscript
            .arithmetic_ast
            .as_ref()
            .is_some_and(|expr| arithmetic_mentions_name_outside_nested_commands(expr, name))
}

fn word_mentions_name_outside_nested_commands(word: &Word, name: &Name) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_mentions_name_outside_nested_commands(&part.kind, name))
}

fn word_part_mentions_name_outside_nested_commands(part: &WordPart, name: &Name) -> bool {
    match part {
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::PrefixMatch { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_mentions_name_outside_nested_commands(&part.kind, name)),
        WordPart::Variable(variable) => variable == name,
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => false,
        WordPart::ArithmeticExpansion {
            expression_ast,
            expression_word_ast,
            ..
        } => {
            expression_ast
                .as_ref()
                .is_some_and(|expr| arithmetic_mentions_name_outside_nested_commands(expr, name))
                || word_mentions_name_outside_nested_commands(expression_word_ast, name)
        }
        WordPart::Parameter(parameter) => {
            parameter_mentions_name_outside_nested_commands(parameter, name)
        }
        WordPart::ParameterExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            var_ref_mentions_name_outside_nested_commands(reference, name)
                || operand_word_ast
                    .as_ref()
                    .is_some_and(|word| word_mentions_name_outside_nested_commands(word, name))
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            var_ref_mentions_name_outside_nested_commands(reference, name)
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
            var_ref_mentions_name_outside_nested_commands(reference, name)
                || offset_ast.as_ref().is_some_and(|expr| {
                    arithmetic_mentions_name_outside_nested_commands(expr, name)
                })
                || word_mentions_name_outside_nested_commands(offset_word_ast, name)
                || length_ast.as_ref().is_some_and(|expr| {
                    arithmetic_mentions_name_outside_nested_commands(expr, name)
                })
                || length_word_ast
                    .as_ref()
                    .is_some_and(|word| word_mentions_name_outside_nested_commands(word, name))
        }
        WordPart::IndirectExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            var_ref_mentions_name_outside_nested_commands(reference, name)
                || operand_word_ast
                    .as_ref()
                    .is_some_and(|word| word_mentions_name_outside_nested_commands(word, name))
        }
    }
}

fn parameter_mentions_name_outside_nested_commands(
    parameter: &ParameterExpansion,
    name: &Name,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                var_ref_mentions_name_outside_nested_commands(reference, name)
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansion::Operation {
                reference,
                operand_word_ast,
                ..
            } => {
                var_ref_mentions_name_outside_nested_commands(reference, name)
                    || operand_word_ast
                        .as_ref()
                        .is_some_and(|word| word_mentions_name_outside_nested_commands(word, name))
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                var_ref_mentions_name_outside_nested_commands(reference, name)
                    || offset_ast.as_ref().is_some_and(|expr| {
                        arithmetic_mentions_name_outside_nested_commands(expr, name)
                    })
                    || length_ast.as_ref().is_some_and(|expr| {
                        arithmetic_mentions_name_outside_nested_commands(expr, name)
                    })
            }
            BourneParameterExpansion::PrefixMatch { .. } => false,
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            zsh_target_mentions_name_outside_nested_commands(&syntax.target, name)
        }
    }
}

fn zsh_target_mentions_name_outside_nested_commands(
    target: &ZshExpansionTarget,
    name: &Name,
) -> bool {
    match target {
        ZshExpansionTarget::Reference(reference) => {
            var_ref_mentions_name_outside_nested_commands(reference, name)
        }
        ZshExpansionTarget::Nested(parameter) => {
            parameter_mentions_name_outside_nested_commands(parameter, name)
        }
        ZshExpansionTarget::Word(word) => word_mentions_name_outside_nested_commands(word, name),
        ZshExpansionTarget::Empty => false,
    }
}

fn var_ref_mentions_name_outside_nested_commands(reference: &VarRef, name: &Name) -> bool {
    reference.name == *name || subscript_mentions_name(reference.subscript.as_ref(), name)
}

fn arithmetic_mentions_name_outside_nested_commands(
    expression: &ArithmeticExprNode,
    name: &Name,
) -> bool {
    match &expression.kind {
        ArithmeticExpr::Number(_) => false,
        ArithmeticExpr::Variable(variable) => variable == name,
        ArithmeticExpr::Indexed {
            name: variable,
            index,
        } => variable == name || arithmetic_mentions_name_outside_nested_commands(index, name),
        ArithmeticExpr::ShellWord(word) => word_mentions_name_outside_nested_commands(word, name),
        ArithmeticExpr::Parenthesized { expression } => {
            arithmetic_mentions_name_outside_nested_commands(expression, name)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            arithmetic_mentions_name_outside_nested_commands(expr, name)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            arithmetic_mentions_name_outside_nested_commands(left, name)
                || arithmetic_mentions_name_outside_nested_commands(right, name)
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            arithmetic_mentions_name_outside_nested_commands(condition, name)
                || arithmetic_mentions_name_outside_nested_commands(then_expr, name)
                || arithmetic_mentions_name_outside_nested_commands(else_expr, name)
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            arithmetic_lvalue_mentions_name_outside_nested_commands(target, name)
                || arithmetic_mentions_name_outside_nested_commands(value, name)
        }
    }
}

fn arithmetic_lvalue_mentions_name_outside_nested_commands(
    target: &ArithmeticLvalue,
    name: &Name,
) -> bool {
    match target {
        ArithmeticLvalue::Variable(variable) => variable == name,
        ArithmeticLvalue::Indexed {
            name: variable,
            index,
        } => variable == name || arithmetic_mentions_name_outside_nested_commands(index, name),
    }
}

fn echo_uses_escape_interpreting_flag(command: &CommandFact<'_>) -> bool {
    command
        .options()
        .echo()
        .is_some_and(|echo| echo.uses_escape_interpreting_flag())
}

fn word_contains_echo_backslash_escape(word: &Word, source: &str) -> bool {
    word_parts_contain_echo_backslash_escape(&word.parts, source, false)
}

fn word_parts_contain_echo_backslash_escape(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => {
            let core_text = if in_double_quotes {
                text.as_str(source, part.span)
            } else {
                part.span.slice(source)
            };
            let quote_like_text = text.as_str(source, part.span);

            text_contains_echo_backslash_escape(core_text, echo_escape_is_core_family)
                || text_contains_echo_backslash_escape(quote_like_text, echo_escape_is_quote_like)
        }
        WordPart::SingleQuoted { value, .. } => {
            text_contains_echo_backslash_escape(value.slice(source), echo_escape_is_core_family)
        }
        WordPart::DoubleQuoted { parts, .. } => {
            word_parts_contain_echo_backslash_escape(parts, source, true)
        }
        _ => false,
    })
}

fn echo_escape_is_core_family(byte: u8) -> bool {
    matches!(
        byte,
        b'a' | b'b' | b'e' | b'f' | b'n' | b'r' | b't' | b'v' | b'x' | b'0'..=b'9'
    )
}

fn echo_escape_is_quote_like(byte: u8) -> bool {
    matches!(byte, b'`' | b'\'')
}

fn text_contains_echo_backslash_escape(text: &str, is_sensitive: fn(u8) -> bool) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        let Some(&escaped_byte) = bytes.get(index) else {
            continue;
        };

        if index > run_start && is_sensitive(escaped_byte) {
            return true;
        }
    }

    false
}

fn build_heredoc_fact_summary(
    commands: &[CommandFact<'_>],
    source: &str,
    file_end: usize,
) -> HeredocFactSummary {
    let mut summary = HeredocFactSummary::default();

    for command in commands {
        let unused_heredoc_command = command.literal_name() == Some("")
            && command.body_span().start.offset == command.body_span().end.offset;
        let echo_here_doc_command = command.effective_name_is("echo")
            && command
                .redirects()
                .iter()
                .any(|redirect| is_heredoc_redirect_kind(redirect.kind));

        if echo_here_doc_command {
            summary
                .echo_here_doc_spans
                .push(command.span_in_source(source));
        }

        for redirect in command.redirects() {
            if !is_heredoc_redirect_kind(redirect.kind) {
                continue;
            }

            if unused_heredoc_command {
                summary.unused_heredoc_spans.push(redirect.span);
            }

            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let reaches_file_end = heredoc.body.span.end.offset == file_end;
            if reaches_file_end {
                summary.heredoc_missing_end_spans.push(redirect.span);
            }

            let delimiter = heredoc.delimiter.cooked.as_str();
            if delimiter.is_empty() {
                continue;
            }

            if let Some(span) = heredoc_end_space_span(
                heredoc.body.span,
                delimiter,
                heredoc.delimiter.strip_tabs,
                source,
            ) {
                summary.heredoc_end_space_spans.push(span);
            }

            if redirect.kind == RedirectKind::HereDocStrip {
                summary
                    .spaced_tabstrip_close_spans
                    .extend(spaced_tabstrip_close_spans(
                        heredoc.body.span,
                        delimiter,
                        source,
                    ));
            }

            if !reaches_file_end {
                continue;
            }

            if let Some(span) = heredoc_closer_not_alone_span(
                heredoc.body.span,
                delimiter,
                heredoc.delimiter.strip_tabs,
                source,
            ) {
                summary.heredoc_closer_not_alone_spans.push(span);
            }

            if has_misquoted_heredoc_close(
                heredoc.body.span,
                delimiter,
                heredoc.delimiter.strip_tabs,
                source,
            ) {
                summary.misquoted_heredoc_close_spans.push(redirect.span);
            }
        }
    }

    summary
}

fn is_heredoc_redirect_kind(kind: RedirectKind) -> bool {
    matches!(kind, RedirectKind::HereDoc | RedirectKind::HereDocStrip)
}

fn heredoc_closer_not_alone_span(
    body_span: Span,
    delimiter: &str,
    strip_tabs: bool,
    source: &str,
) -> Option<Span> {
    let mut line_start_offset = body_span.start.offset;
    for raw_line in body_span.slice(source).split_inclusive('\n') {
        let (candidate_line, tab_prefix_len) = normalized_heredoc_line(raw_line, strip_tabs);
        if !candidate_line.ends_with(delimiter)
            || is_quoted_delimiter_variant(candidate_line, delimiter)
        {
            line_start_offset += raw_line.len();
            continue;
        }

        let prefix = &candidate_line[..candidate_line.len() - delimiter.len()];
        if !prefix.chars().any(|ch| !ch.is_whitespace()) {
            line_start_offset += raw_line.len();
            continue;
        }

        let delimiter_start_offset = line_start_offset + tab_prefix_len + prefix.len();
        let delimiter_end_offset = delimiter_start_offset + delimiter.len();
        let start = position_at_offset(source, delimiter_start_offset)?;
        let end = position_at_offset(source, delimiter_end_offset)?;
        return Some(Span::from_positions(start, end));
    }

    None
}

fn has_misquoted_heredoc_close(
    body_span: Span,
    delimiter: &str,
    strip_tabs: bool,
    source: &str,
) -> bool {
    body_span
        .slice(source)
        .split_inclusive('\n')
        .map(|raw_line| normalized_heredoc_line(raw_line, strip_tabs).0)
        .filter(|candidate_line| *candidate_line != delimiter)
        .any(|candidate_line| is_quoted_delimiter_variant(candidate_line, delimiter))
}

fn heredoc_end_space_span(
    body_span: Span,
    delimiter: &str,
    strip_tabs: bool,
    source: &str,
) -> Option<Span> {
    let mut line_start_offset = body_span.start.offset;
    for raw_line in body_span.slice(source).split_inclusive('\n') {
        let (candidate_line, tab_prefix_len) = normalized_heredoc_line(raw_line, strip_tabs);
        let Some(trailing) = candidate_line.strip_prefix(delimiter) else {
            line_start_offset += raw_line.len();
            continue;
        };
        if trailing.is_empty() || !trailing.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            line_start_offset += raw_line.len();
            continue;
        }

        let trailing_start_offset = line_start_offset + tab_prefix_len + delimiter.len();
        let trailing_end_offset = trailing_start_offset + trailing.len();
        let start = position_at_offset(source, trailing_start_offset)?;
        let end = position_at_offset(source, trailing_end_offset)?;
        return Some(Span::from_positions(start, end));
    }

    None
}

fn spaced_tabstrip_close_spans(body_span: Span, delimiter: &str, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut line_start_offset = body_span.start.offset;
    for raw_line in body_span.slice(source).split_inclusive('\n') {
        let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        if is_spaced_tabstrip_close_line(line_without_newline, delimiter)
            && let Some(position) = position_at_offset(source, line_start_offset)
        {
            spans.push(Span::from_positions(position, position));
        }
        line_start_offset += raw_line.len();
    }

    spans
}

fn normalized_heredoc_line(raw_line: &str, strip_tabs: bool) -> (&str, usize) {
    let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
    if strip_tabs {
        let trimmed = line_without_newline.trim_start_matches('\t');
        (trimmed, line_without_newline.len() - trimmed.len())
    } else {
        (line_without_newline, 0)
    }
}

fn is_quoted_delimiter_variant(candidate_line: &str, delimiter: &str) -> bool {
    candidate_line != delimiter && trim_quote_like_wrappers(candidate_line) == delimiter
}

fn trim_quote_like_wrappers(text: &str) -> &str {
    text.trim_matches(|ch| matches!(ch, '\'' | '"' | '\\'))
}

fn is_spaced_tabstrip_close_line(line: &str, delimiter: &str) -> bool {
    if line.trim_start_matches('\t') == delimiter {
        return false;
    }

    let line_without_trailing_ws = line.trim_end_matches([' ', '\t']);
    let leading_len = line_without_trailing_ws.len()
        - line_without_trailing_ws
            .trim_start_matches([' ', '\t'])
            .len();
    if leading_len == 0 {
        return false;
    }

    let leading = &line_without_trailing_ws[..leading_len];
    let rest = &line_without_trailing_ws[leading_len..];
    leading.contains(' ') && rest == delimiter
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

fn build_plus_equals_assignment_spans(commands: &[CommandFact<'_>]) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        collect_plus_equals_assignment_spans_in_command(fact.command(), &mut spans);
    }

    spans
}

fn collect_plus_equals_assignment_spans_in_command(command: &Command, spans: &mut Vec<Span>) {
    match command {
        Command::Simple(command) => {
            collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
            BuiltinCommand::Continue(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
            BuiltinCommand::Return(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
            BuiltinCommand::Exit(command) => {
                collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            }
        },
        Command::Decl(command) => {
            collect_plus_equals_assignment_spans_in_assignments(&command.assignments, spans);
            for operand in &command.operands {
                if let DeclOperand::Assignment(assignment) = operand {
                    collect_plus_equals_assignment_span(assignment, spans);
                }
            }
        }
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

fn collect_plus_equals_assignment_spans_in_assignments(
    assignments: &[Assignment],
    spans: &mut Vec<Span>,
) {
    for assignment in assignments {
        collect_plus_equals_assignment_span(assignment, spans);
    }
}

fn collect_plus_equals_assignment_span(assignment: &Assignment, spans: &mut Vec<Span>) {
    if !assignment.append {
        return;
    }

    let target = &assignment.target;
    let end = target
        .subscript
        .as_ref()
        .map(|subscript| subscript.syntax_source_text().span().end.advanced_by("]"))
        .unwrap_or(target.name_span.end);
    spans.push(Span::from_positions(target.name_span.start, end));
}
fn build_base_prefix_arithmetic_spans(body: &StmtSeq, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    for visit in query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    ) {
        collect_base_prefix_spans_in_command(visit.command, source, &mut spans);
        for redirect in visit.redirects {
            if let Some(word) = redirect.word_target() {
                collect_base_prefix_spans_in_word(word, source, &mut spans);
            }
        }
    }

    spans
}

fn collect_base_prefix_spans_in_command(command: &Command, source: &str, spans: &mut Vec<Span>) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                collect_base_prefix_spans_in_assignment(assignment, source, spans);
            }
            collect_base_prefix_spans_in_word(&command.name, source, spans);
            for word in &command.args {
                collect_base_prefix_spans_in_word(word, source, spans);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.depth {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.depth {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.code {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.code {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
        },
        Command::Decl(command) => {
            for assignment in &command.assignments {
                collect_base_prefix_spans_in_assignment(assignment, source, spans);
            }
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_base_prefix_spans_in_word(word, source, spans);
                    }
                    DeclOperand::Assignment(assignment) => {
                        collect_base_prefix_spans_in_assignment(assignment, source, spans);
                    }
                    DeclOperand::Name(_) => {}
                }
            }
        }
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        collect_base_prefix_spans_in_word(word, source, spans);
                    }
                }
            }
            CompoundCommand::Repeat(command) => {
                collect_base_prefix_spans_in_word(&command.count, source, spans);
            }
            CompoundCommand::Foreach(command) => {
                for word in &command.words {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            CompoundCommand::Arithmetic(command) => {
                if let Some(expression) = &command.expr_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.expr_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
            }
            CompoundCommand::ArithmeticFor(command) => {
                if let Some(expression) = &command.init_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.init_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
                if let Some(expression) = &command.condition_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.condition_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
                if let Some(expression) = &command.step_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.step_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
            }
            CompoundCommand::Case(command) => {
                collect_base_prefix_spans_in_word(&command.word, source, spans);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        collect_base_prefix_spans_in_pattern(pattern, source, spans);
                    }
                    collect_base_prefix_spans_in_stmt_seq(&item.body, source, spans);
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                collect_base_prefix_spans_in_stmt_seq(&command.body, source, spans);
            }
            CompoundCommand::If(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Always(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Time(_) => {}
        },
        Command::Binary(_) | Command::Function(_) | Command::AnonymousFunction(_) => {}
    }
}

fn collect_base_prefix_spans_in_assignment(
    assignment: &Assignment,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_base_prefix_spans_in_var_ref(&assignment.target, source, spans);

    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_base_prefix_spans_in_word(word, source, spans),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_base_prefix_spans_in_word(word, source, spans);
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        collect_base_prefix_spans_in_subscript(Some(key), source, spans);
                        collect_base_prefix_spans_in_word(value, source, spans);
                    }
                }
            }
        }
    }
}

fn collect_base_prefix_spans_in_word(word: &Word, source: &str, spans: &mut Vec<Span>) {
    for part in &word.parts {
        collect_base_prefix_spans_in_word_part(part, source, spans);
    }
}

fn collect_base_prefix_spans_in_word_part(
    part: &WordPartNode,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                collect_base_prefix_spans_in_word_part(part, source, spans);
            }
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            ..
        } => {
            if let Some(expression) = expression_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_text(expression.span(), source, spans);
            }
        }
        WordPart::Parameter(parameter) => {
            collect_base_prefix_spans_in_parameter_expansion(parameter, source, spans);
        }
        WordPart::ParameterExpansion { reference, .. }
        | WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::IndirectExpansion { reference, .. }
        | WordPart::Transformation { reference, .. } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
        }
        WordPart::Substring {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
            if let Some(expression) = offset_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_text(offset.span(), source, spans);
            }
            if let Some(expression) = length_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else if let Some(length) = length {
                collect_base_prefix_spans_in_text(length.span(), source, spans);
            }
        }
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::PrefixMatch { .. } => {}
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            collect_base_prefix_spans_in_stmt_seq(body, source, spans);
        }
    }
}

fn collect_base_prefix_spans_in_parameter_expansion(
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
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
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
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                collect_base_prefix_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    spans,
                );
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                if let Some(expression) = offset_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                }
                if let Some(expression) = length_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_base_prefix_spans_in_zsh_target(&syntax.target, source, spans);
            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::Slice { .. }
                    | shuck_ast::ZshExpansionOperation::PatternOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { .. }
                    | shuck_ast::ZshExpansionOperation::ReplacementOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Unknown { .. } => {}
                }
            }
        }
    }
}

fn collect_base_prefix_spans_in_zsh_target(
    target: &shuck_ast::ZshExpansionTarget,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match target {
        shuck_ast::ZshExpansionTarget::Reference(reference) => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Nested(parameter) => {
            collect_base_prefix_spans_in_parameter_expansion(parameter, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Word(word) => {
            collect_base_prefix_spans_in_word(word, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Empty => {}
    }
}

fn collect_base_prefix_spans_in_stmt_seq(body: &StmtSeq, source: &str, spans: &mut Vec<Span>) {
    for stmt in &body.stmts {
        collect_base_prefix_spans_in_command(&stmt.command, source, spans);
    }
}

fn collect_base_prefix_spans_in_pattern(pattern: &Pattern, source: &str, spans: &mut Vec<Span>) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_base_prefix_spans_in_pattern(pattern, source, spans);
                }
            }
            PatternPart::Word(word) => collect_base_prefix_spans_in_word(word, source, spans),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_base_prefix_spans_in_var_ref(reference: &VarRef, source: &str, spans: &mut Vec<Span>) {
    collect_base_prefix_spans_in_subscript(reference.subscript.as_ref(), source, spans);
}

fn collect_base_prefix_spans_in_subscript(
    subscript: Option<&Subscript>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(expression) = subscript.and_then(|subscript| subscript.arithmetic_ast.as_ref()) {
        collect_base_prefix_spans_in_arithmetic(expression, source, spans);
    }
}

fn collect_base_prefix_spans_in_arithmetic(
    expression: &ArithmeticExprNode,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_base_prefix_spans_in_text(expression.span, source, spans);
}

fn collect_base_prefix_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(text) = text else {
        return;
    };
    let snippet = text.slice(source);
    if !snippet.contains('#') {
        return;
    }

    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        return;
    };
    collect_base_prefix_spans_in_word(word, source, spans);
}

fn collect_base_prefix_spans_in_text(span: Span, source: &str, spans: &mut Vec<Span>) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        if index > 0 {
            let previous = bytes[index - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' {
                index += 1;
                continue;
            }
        }

        let mut prefix_end = index;
        while prefix_end < bytes.len() && bytes[prefix_end].is_ascii_digit() {
            prefix_end += 1;
        }

        if prefix_end == bytes.len() || bytes[prefix_end] != b'#' {
            index = prefix_end.max(index + 1);
            continue;
        }

        let mut match_end = prefix_end + 1;
        while match_end < bytes.len() {
            let byte = bytes[match_end];
            if byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_') {
                match_end += 1;
            } else {
                break;
            }
        }

        let start = span.start.advanced_by(&text[..index]);
        let end = start.advanced_by(&text[index..match_end]);
        spans.push(Span::from_positions(start, end));
        index = match_end;
    }
}

fn build_function_header_facts<'a>(
    semantic: &SemanticModel,
    functions: &[&'a FunctionDef],
    commands: &[CommandFact<'a>],
    source: &str,
) -> Vec<FunctionHeaderFact<'a>> {
    let call_arity_by_binding =
        build_function_call_arity_facts(semantic, functions, commands, source);
    functions
        .iter()
        .copied()
        .map(|function| {
            let binding_id = function_header_binding_id(semantic, function);
            let scope_id = binding_id
                .and_then(|binding_id| function_header_scope_id(semantic, function, binding_id));
            let call_arity = binding_id
                .and_then(|binding_id| call_arity_by_binding.get(&binding_id).cloned())
                .unwrap_or_default();

            FunctionHeaderFact {
                function,
                binding_id,
                scope_id,
                call_arity,
            }
        })
        .collect()
}

fn build_function_parameter_fallback_spans(
    commands: &[CommandFact<'_>],
    structural_command_ids: &[CommandId],
    source: &str,
) -> Vec<Span> {
    let structural_commands = structural_command_ids
        .iter()
        .copied()
        .map(|id| &commands[id.index()])
        .collect::<Vec<_>>();

    structural_commands
        .windows(2)
        .filter_map(|pair| function_parameter_fallback_span(pair, source))
        .collect()
}

fn function_parameter_fallback_span(pair: &[&CommandFact<'_>], source: &str) -> Option<Span> {
    let [first, second] = pair else {
        return None;
    };
    let name = first.normalized().effective_or_literal_name()?;
    if !is_plausible_shell_function_name(name) || !first.normalized().body_args().is_empty() {
        return None;
    }
    if !matches!(first.command(), Command::Simple(_)) {
        return None;
    }
    let Command::Compound(CompoundCommand::Subshell(commands)) = second.command() else {
        return None;
    };
    if commands.is_empty() {
        return None;
    }
    if first.span().start.line != second.span().start.line {
        return None;
    }
    let tail = source.get(second.span().end.offset..)?;
    if !matches!(next_function_body_delimiter(tail), Some('{') | Some('(')) {
        return None;
    }
    let text = first.span().slice(source);
    let relative = text.find('(')?;
    let start = first.span().start.advanced_by(&text[..relative]);
    Some(Span::from_positions(start, start.advanced_by("(")))
}
fn build_function_call_arity_facts<'a>(
    semantic: &SemanticModel,
    functions: &[&FunctionDef],
    commands: &[CommandFact<'a>],
    source: &str,
) -> FxHashMap<BindingId, FunctionCallArityFacts> {
    let mut facts = FxHashMap::<BindingId, FunctionCallArityFacts>::default();
    let mut seen_names = FxHashSet::default();

    for function in functions {
        let Some((name, _)) = function.static_name_entries().next() else {
            continue;
        };
        if !seen_names.insert(name.clone()) {
            continue;
        }

        for command in commands {
            if !command.wrappers().is_empty()
                || command.effective_or_literal_name() != Some(name.as_str())
            {
                continue;
            }
            let Some(name_word) = command.body_name_word() else {
                continue;
            };
            let Some(binding_id) = visible_function_binding_for_call_offset(
                semantic,
                name,
                name_word.span.start.offset,
            ) else {
                continue;
            };
            facts
                .entry(binding_id)
                .or_default()
                .record_call(function_call_arg_count(command, source), name_word.span);
        }
    }

    facts
}

fn function_call_arg_count(command: &CommandFact<'_>, source: &str) -> usize {
    let arg_count = command.body_args().len();
    if arg_count != 0 || !command.redirects().is_empty() || !command.is_nested_word_command() {
        return arg_count;
    }

    let Some(name_word) = command.body_name_word() else {
        return 0;
    };
    let stmt_span = trim_trailing_whitespace_span(command.stmt().span, source);
    let tail = if stmt_span.end.offset > name_word.span.end.offset {
        trim_shell_layout_prefix(&source[name_word.span.end.offset..stmt_span.end.offset])
    } else {
        trim_shell_layout_prefix(&source[name_word.span.end.offset..])
    };
    if tail.is_empty() {
        return 0;
    }
    if matches!(
        tail.as_bytes().first(),
        Some(b')' | b';' | b'|' | b'&' | b'<' | b'>' | b'#' | b'`')
    ) {
        return 0;
    }

    1
}

fn function_header_binding_id(
    semantic: &SemanticModel,
    function: &FunctionDef,
) -> Option<BindingId> {
    let (name, name_span) = function.static_name_entries().next()?;
    semantic
        .function_definitions(name)
        .iter()
        .copied()
        .find(|binding_id| semantic.binding(*binding_id).span == name_span)
}

fn function_header_scope_id(
    semantic: &SemanticModel,
    function: &FunctionDef,
    binding_id: BindingId,
) -> Option<ScopeId> {
    let (name, _) = function.static_name_entries().next()?;
    let binding = semantic.binding(binding_id);

    semantic.scopes().iter().find_map(|scope| {
        let shuck_semantic::ScopeKind::Function(function_scope) = &scope.kind else {
            return None;
        };
        (scope.parent == Some(binding.scope)
            && scope.span == function.body.span
            && function_scope.contains_name(name))
        .then_some(scope.id)
    })
}

fn visible_function_binding_for_call_offset(
    semantic: &SemanticModel,
    name: &Name,
    site_offset: usize,
) -> Option<BindingId> {
    let scopes = semantic
        .ancestor_scopes(semantic.scope_at(site_offset))
        .collect::<Vec<_>>();

    scopes
        .iter()
        .copied()
        .find_map(|scope| {
            semantic
                .function_definitions(name)
                .iter()
                .copied()
                .filter(|candidate| semantic.binding(*candidate).scope == scope)
                .filter(|candidate| semantic.binding(*candidate).span.start.offset < site_offset)
                .max_by_key(|candidate| semantic.binding(*candidate).span.start.offset)
        })
        .or_else(|| {
            scopes.iter().copied().find_map(|scope| {
                semantic
                    .function_definitions(name)
                    .iter()
                    .copied()
                    .filter(|candidate| semantic.binding(*candidate).scope == scope)
                    .min_by_key(|candidate| semantic.binding(*candidate).span.start.offset)
            })
        })
}
fn function_body_without_braces_span(function: &FunctionDef) -> Option<Span> {
    match &function.body.command {
        Command::Compound(CompoundCommand::BraceGroup(_)) => None,
        Command::Compound(_) => Some(function.body.span),
        Command::Simple(_)
        | Command::Decl(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => None,
    }
}

fn next_function_body_delimiter(text: &str) -> Option<char> {
    let mut tail = text;

    loop {
        tail = trim_shell_layout_prefix(tail);

        if let Some(rest) = tail.strip_prefix('#') {
            tail = rest.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }

        return tail.chars().next();
    }
}

fn trim_shell_layout_prefix(text: &str) -> &str {
    let mut tail = text;

    loop {
        tail = tail.trim_start_matches([' ', '\t', '\r', '\n']);

        if let Some(rest) = tail
            .strip_prefix("\\\r\n")
            .or_else(|| tail.strip_prefix("\\\n"))
        {
            tail = rest;
            continue;
        }

        return tail;
    }
}

fn is_plausible_shell_function_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !matches!(first, 'a'..='z' | 'A'..='Z' | '_') {
        return false;
    }
    if !name
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-'))
    {
        return false;
    }
    !matches!(
        name,
        "!" | "{"
            | "}"
            | "if"
            | "then"
            | "else"
            | "elif"
            | "fi"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "for"
            | "in"
            | "while"
            | "until"
            | "time"
            | "[["
            | "]]"
            | "function"
            | "select"
            | "coproc"
    )
}

fn collect_terminal_redundant_return_status_spans(function: &FunctionDef, spans: &mut Vec<Span>) {
    collect_terminal_redundant_return_status_spans_in_stmt(&function.body, spans);
}

fn collect_terminal_redundant_return_status_spans_in_stmt(stmt: &Stmt, spans: &mut Vec<Span>) {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            collect_terminal_redundant_return_status_spans_in_seq(commands, spans);
        }
        Command::Compound(CompoundCommand::If(command)) => {
            collect_terminal_redundant_return_status_spans_in_if(command, spans);
        }
        Command::Simple(_)
        | Command::Decl(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

fn collect_terminal_redundant_return_status_spans_in_if(
    command: &IfCommand,
    spans: &mut Vec<Span>,
) {
    collect_terminal_redundant_return_status_spans_in_seq(&command.then_branch, spans);
    for (_, branch) in &command.elif_branches {
        collect_terminal_redundant_return_status_spans_in_seq(branch, spans);
    }
    if let Some(branch) = &command.else_branch {
        collect_terminal_redundant_return_status_spans_in_seq(branch, spans);
    }
}

fn collect_terminal_redundant_return_status_spans_in_seq(
    commands: &StmtSeq,
    spans: &mut Vec<Span>,
) {
    if let Some(span) = terminal_redundant_return_status_span(commands) {
        spans.push(span);
    }

    let Some(last) = commands.last() else {
        return;
    };
    if last.negated || matches!(last.terminator, Some(StmtTerminator::Background(_))) {
        return;
    }
    collect_terminal_redundant_return_status_spans_in_stmt(last, spans);
}

fn terminal_redundant_return_status_span(commands: &StmtSeq) -> Option<Span> {
    let [.., previous, last] = commands.as_slice() else {
        return None;
    };
    if !stmt_is_terminal_status_propagating_command(previous) {
        return None;
    }
    if last.negated || matches!(last.terminator, Some(StmtTerminator::Background(_))) {
        return None;
    }

    let Command::Builtin(BuiltinCommand::Return(command)) = &last.command else {
        return None;
    };
    if !command.extra_args.is_empty()
        || !command.assignments.is_empty()
        || !last.redirects.is_empty()
    {
        return None;
    }
    let code = command.code.as_ref()?;
    crate::word_is_standalone_status_capture(code).then_some(code.span)
}

fn stmt_is_terminal_status_propagating_command(stmt: &Stmt) -> bool {
    if stmt.negated || matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
        return false;
    }

    !matches!(stmt.command, Command::Builtin(_))
}

fn build_function_positional_parameter_facts(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    positional_parameter_fragments: &[PositionalParameterFragmentFact],
) -> FxHashMap<ScopeId, FunctionPositionalParameterFacts> {
    let mut facts: FxHashMap<ScopeId, FunctionPositionalParameterFacts> = FxHashMap::default();
    let mut local_reset_offsets_by_scope: FxHashMap<ScopeId, Vec<usize>> = FxHashMap::default();

    for command in commands {
        if !command
            .options()
            .set()
            .is_some_and(|set| set.resets_positional_parameters())
        {
            continue;
        }

        let offset = command.span().start.offset;
        if let Some(scope) = innermost_nonpersistent_scope_within_function(semantic, offset) {
            local_reset_offsets_by_scope
                .entry(scope)
                .or_default()
                .push(offset);
        }
    }

    for reference in semantic.references() {
        if reference_has_local_positional_reset(
            semantic,
            reference.span.start.offset,
            &local_reset_offsets_by_scope,
        ) {
            continue;
        }

        let Some(index) = positional_parameter_index(reference.name.as_str()) else {
            let Some(uses_positional_parameters) =
                special_positional_parameter_name(reference.name.as_str())
            else {
                continue;
            };

            if semantic.is_guarded_parameter_reference(reference.id) {
                continue;
            }

            let Some(scope) = enclosing_function_scope(semantic, reference.span.start.offset)
            else {
                continue;
            };

            if uses_positional_parameters {
                facts
                    .entry(scope)
                    .or_default()
                    .uses_unprotected_positional_parameters = true;
            }
            continue;
        };
        if semantic.is_guarded_parameter_reference(reference.id) {
            continue;
        }

        let Some(scope) = enclosing_function_scope(semantic, reference.span.start.offset) else {
            continue;
        };

        let entry = facts.entry(scope).or_default();
        entry.required_arg_count = entry.required_arg_count.max(index);
        entry.uses_unprotected_positional_parameters = true;
    }

    for fragment in positional_parameter_fragments {
        if fragment.is_guarded() {
            continue;
        }

        if reference_has_local_positional_reset(
            semantic,
            fragment.span().start.offset,
            &local_reset_offsets_by_scope,
        ) {
            continue;
        }

        let Some(scope) = enclosing_function_scope(semantic, fragment.span().start.offset) else {
            continue;
        };

        facts
            .entry(scope)
            .or_default()
            .uses_unprotected_positional_parameters = true;
    }

    for command in commands {
        let Some(scope) =
            enclosing_function_scope_for_positional_reset(semantic, command.span().start.offset)
        else {
            continue;
        };

        if command
            .options()
            .set()
            .is_some_and(|set| set.resets_positional_parameters())
        {
            facts.entry(scope).or_default().resets_positional_parameters = true;
        }
    }

    facts
}

fn enclosing_function_scope(semantic: &SemanticModel, offset: usize) -> Option<ScopeId> {
    let scope = semantic.scope_at(offset);
    semantic.ancestor_scopes(scope).find(|scope| {
        matches!(
            semantic.scope_kind(*scope),
            shuck_semantic::ScopeKind::Function(_)
        )
    })
}

fn enclosing_function_scope_for_positional_reset(
    semantic: &SemanticModel,
    offset: usize,
) -> Option<ScopeId> {
    let scope = semantic.scope_at(offset);

    for scope in semantic.ancestor_scopes(scope) {
        match semantic.scope_kind(scope) {
            shuck_semantic::ScopeKind::Function(_) => return Some(scope),
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution
            | shuck_semantic::ScopeKind::Pipeline => return None,
            shuck_semantic::ScopeKind::File => {}
        }
    }

    None
}

fn innermost_nonpersistent_scope_within_function(
    semantic: &SemanticModel,
    offset: usize,
) -> Option<ScopeId> {
    let scope = semantic.scope_at(offset);

    for scope in semantic.ancestor_scopes(scope) {
        match semantic.scope_kind(scope) {
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution
            | shuck_semantic::ScopeKind::Pipeline => return Some(scope),
            shuck_semantic::ScopeKind::Function(_) => return None,
            shuck_semantic::ScopeKind::File => {}
        }
    }

    None
}

fn reference_has_local_positional_reset(
    semantic: &SemanticModel,
    offset: usize,
    local_reset_offsets_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
) -> bool {
    let scope = semantic.scope_at(offset);

    for scope in semantic.ancestor_scopes(scope) {
        match semantic.scope_kind(scope) {
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution
            | shuck_semantic::ScopeKind::Pipeline => {
                if local_reset_offsets_by_scope
                    .get(&scope)
                    .is_some_and(|offsets| {
                        offsets.iter().any(|reset_offset| *reset_offset < offset)
                    })
                {
                    return true;
                }
            }
            shuck_semantic::ScopeKind::Function(_) => return false,
            shuck_semantic::ScopeKind::File => {}
        }
    }

    false
}

fn positional_parameter_index(name: &str) -> Option<usize> {
    if name == "0" || matches!(name, "@" | "*" | "#") {
        return None;
    }
    if name.chars().all(|ch| ch.is_ascii_digit()) {
        name.parse::<usize>().ok()
    } else {
        None
    }
}

fn special_positional_parameter_name(name: &str) -> Option<bool> {
    match name {
        "@" | "*" | "#" => Some(true),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ShebangHeaderFacts {
    indented_shebang_span: Option<Span>,
    space_after_hash_bang_span: Option<Span>,
    shebang_not_on_first_line_span: Option<Span>,
    missing_shebang_line_span: Option<Span>,
    duplicate_shebang_flag_span: Option<Span>,
    non_absolute_shebang_span: Option<Span>,
    enables_errexit: bool,
}

fn build_shebang_header_facts(source: &str) -> ShebangHeaderFacts {
    let mut source_lines = source_lines_with_offsets(source).enumerate();
    let Some((_, (first_line_offset, first_line_text))) = source_lines.next() else {
        return ShebangHeaderFacts::default();
    };
    let first_line = first_line_text.trim_end_matches('\r');
    let mut indented_shebang_span = None;
    let mut space_after_hash_bang_span = None;
    let mut shebang_not_on_first_line_span = None;

    for (line_index, (offset, raw_line)) in
        std::iter::once((0, (first_line_offset, first_line_text))).chain(source_lines)
    {
        let line = raw_line.trim_end_matches('\r');
        let header_like = source_line_is_header_like(line);
        let shebang_candidate = source_line_has_shebang_candidate(line);
        let indented_candidate = source_line_has_leading_whitespace_before_shebang_candidate(line);
        let space_after_hash_offset = shebang_space_after_hash_offset_in_line(line);
        let line_number = line_index + 1;

        if indented_shebang_span.is_none() && indented_candidate {
            indented_shebang_span = Some(point_span(line_number, 1, offset));
        }
        if space_after_hash_bang_span.is_none()
            && let Some(space_offset) = space_after_hash_offset
        {
            space_after_hash_bang_span = Some(point_span(
                line_number,
                space_offset + 1,
                offset + space_offset,
            ));
        }
        if line_index > 0 && shebang_candidate {
            shebang_not_on_first_line_span = Some(point_span(line_number, 1, offset));
        }

        if shebang_candidate || !header_like {
            break;
        }
    }

    let first_line_shellcheck_shell_directive = first_line
        .strip_prefix('#')
        .map(str::trim_start)
        .is_some_and(|comment| {
            comment
                .to_ascii_lowercase()
                .starts_with("shellcheck shell=")
        });
    let missing_shebang_line_span = (!first_line.trim_start().starts_with("#!")
        && space_after_hash_bang_span.is_none()
        && shebang_not_on_first_line_span.is_none()
        && !first_line_shellcheck_shell_directive
        && first_line.trim_start().starts_with('#'))
    .then(|| line_span(1, first_line_offset, first_line));

    let shebang_words = first_line
        .strip_prefix("#!")
        .map(parse_shebang_words)
        .unwrap_or_default();

    let duplicate_shebang_flag_span =
        shebang_duplicate_flag(&shebang_words).map(|_| line_span(1, first_line_offset, first_line));

    let non_absolute_shebang_span = shebang_words.first().and_then(|interpreter| {
        if interpreter.starts_with('/') || *interpreter == "/usr/bin/env" {
            return None;
        }
        if has_header_shellcheck_shell_directive(source) {
            return None;
        }
        Some(line_span(1, first_line_offset, first_line))
    });
    let enables_errexit = first_nonempty_source_line(source)
        .and_then(|(_, line)| line.trim_end_matches('\r').strip_prefix("#!"))
        .map(parse_shebang_words)
        .is_some_and(|words| shebang_enables_errexit(&words));

    ShebangHeaderFacts {
        indented_shebang_span,
        space_after_hash_bang_span,
        shebang_not_on_first_line_span,
        missing_shebang_line_span,
        duplicate_shebang_flag_span,
        non_absolute_shebang_span,
        enables_errexit,
    }
}

fn source_line_is_header_like(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn source_line_has_shebang_candidate(line: &str) -> bool {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    trimmed.starts_with("#!") || shebang_space_after_hash_offset_in_line(trimmed).is_some()
}

fn source_line_has_leading_whitespace_before_shebang_candidate(line: &str) -> bool {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    trimmed.len() != line.len() && source_line_has_shebang_candidate(line)
}

fn shebang_space_after_hash_offset_in_line(line: &str) -> Option<usize> {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    let leading_whitespace_len = line.len().saturating_sub(trimmed.len());
    let rest = trimmed.strip_prefix('#')?;
    let whitespace_len = rest
        .len()
        .saturating_sub(rest.trim_start_matches(char::is_whitespace).len());
    (whitespace_len > 0 && rest[whitespace_len..].starts_with('!'))
        .then_some(leading_whitespace_len + 1)
}

fn point_span(line_number: usize, column: usize, offset: usize) -> Span {
    Span::at(Position {
        line: line_number,
        column,
        offset,
    })
}

fn parse_shebang_words(shebang: &str) -> Vec<&str> {
    shebang.split_whitespace().collect()
}

fn source_lines_with_offsets(source: &str) -> impl Iterator<Item = (usize, &str)> + '_ {
    source
        .split_inclusive('\n')
        .scan(0usize, |offset, raw_line| {
            let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
            let line_offset = *offset;
            *offset += raw_line.len();
            Some((line_offset, line))
        })
}

fn first_nonempty_source_line(source: &str) -> Option<(usize, &str)> {
    source_lines_with_offsets(source).find(|(_, line)| !line.trim().is_empty())
}

fn shebang_duplicate_flag<'a>(shebang_words: &[&'a str]) -> Option<&'a str> {
    let mut seen = FxHashSet::default();

    shebang_words
        .iter()
        .copied()
        .skip(1)
        .find(|word| word.starts_with('-') && !seen.insert(*word))
}

fn shebang_enables_errexit(shebang_words: &[&str]) -> bool {
    let mut words = shebang_words.iter().copied().peekable();
    while let Some(word) = words.next() {
        if shebang_short_option_cluster_enables_errexit(word) {
            return true;
        }
        if word == "-o" && matches!(words.peek(), Some(&"errexit")) {
            return true;
        }
        if word == "-oerrexit" {
            return true;
        }
    }

    false
}

fn shebang_short_option_cluster_enables_errexit(word: &str) -> bool {
    let Some(flags) = word.strip_prefix('-') else {
        return false;
    };

    if word == "-" || word == "--" || word.starts_with("--") {
        return false;
    }

    flags.chars().all(|char| char.is_ascii_alphabetic()) && flags.contains('e')
}

fn line_span(line_number: usize, offset: usize, line: &str) -> Span {
    let start = Position {
        line: line_number,
        column: 1,
        offset,
    };
    let end = start.advanced_by(line);
    Span::from_positions(start, end)
}

fn build_commented_continuation_comment_spans(source: &str, indexer: &Indexer) -> Vec<Span> {
    let line_index = indexer.line_index();
    let comment_index = indexer.comment_index();

    indexer
        .continuation_line_starts()
        .iter()
        .filter_map(|&line_start_offset| {
            let line = line_index.line_number(line_start_offset);
            let comment = comment_index
                .comments_on_line(line)
                .iter()
                .find(|comment| comment.is_own_line)?;
            let line_start = usize::from(line_index.line_start(line)?);
            let line_end = usize::from(line_index.line_range(line, source)?.end());
            let comment_start = usize::from(comment.range.start());
            if comment_start < line_start || comment_start >= line_end || line_end > source.len() {
                return None;
            }
            let comment_text = &source[comment_start..line_end];
            let trimmed_comment_text = comment_text.trim_end_matches([' ', '\t', '\r']);
            if !trimmed_comment_text.ends_with('\\') {
                return None;
            }
            let caret_offset = comment_start + trimmed_comment_text.len();

            let line_start_position = Position {
                line,
                column: 1,
                offset: line_start,
            };
            let caret = line_start_position.advanced_by(&source[line_start..caret_offset]);
            Some(Span::at(caret))
        })
        .collect()
}

fn build_trailing_directive_comment_spans(source: &str, indexer: &Indexer) -> Vec<Span> {
    let line_index = indexer.line_index();

    indexer
        .comment_index()
        .comments()
        .iter()
        .filter_map(|comment| {
            if comment.is_own_line {
                return None;
            }

            let line = line_index.line_number(comment.range.start());
            let line_start = usize::from(line_index.line_start(line)?);
            let line_end = usize::from(line_index.line_range(line, source)?.end());
            let comment_start = usize::from(comment.range.start());
            let comment_end = usize::from(comment.range.end())
                .min(line_end)
                .min(source.len());
            if comment_start < line_start || comment_start >= comment_end {
                return None;
            }
            if directive_can_apply_to_following_command(&source[line_start..comment_start]) {
                return None;
            }

            let comment_text = &source[comment_start..comment_end];
            if !is_inline_shellcheck_directive(comment_text) {
                return None;
            }

            let line_start_position = Position {
                line,
                column: 1,
                offset: line_start,
            };
            let start = line_start_position.advanced_by(&source[line_start..comment_start]);
            let end = start.advanced_by("#");
            Some(Span::from_positions(start, end))
        })
        .collect()
}

fn build_literal_brace_spans(
    words: &[WordFact<'_>],
    commands: &[CommandFact<'_>],
    source: &str,
    heredoc_ranges: &[TextRange],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in words {
        if fact.expansion_context() == Some(ExpansionContext::RegexOperand) {
            continue;
        }

        let is_find_exec_placeholder_word = is_find_exec_placeholder_word(commands, fact, source);
        let is_xargs_replacement_word = is_xargs_replacement_word(commands, fact, source);
        let direct_spans = fact
            .word()
            .brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.quote_context == BraceQuoteContext::Unquoted)
            .filter(|brace| {
                matches!(
                    brace.kind,
                    BraceSyntaxKind::Literal | BraceSyntaxKind::TemplatePlaceholder
                ) || brace_syntax_with_whitespace_is_literal(*brace, source)
            })
            .filter(|brace| {
                brace.span.slice(source) != "{}"
                    && !brace_span_has_escaped_dollar_prefix(brace.span, source)
                    && !is_find_exec_placeholder_word
                    && !is_xargs_replacement_word
            })
            .flat_map(|brace| brace_character_spans(brace.span, source))
            .filter(|span| {
                !span_inside_nested_escaped_parameter_template(fact.word(), *span, source)
            })
            .collect::<Vec<_>>();
        spans.extend(direct_spans);

        if !is_find_exec_placeholder_word && !is_xargs_replacement_word {
            let unclassified = unclassified_literal_brace_spans(fact.word(), source)
                .into_iter()
                .filter(|span| {
                    !span_inside_nested_escaped_parameter_template(fact.word(), *span, source)
                })
                .collect::<Vec<_>>();
            spans.extend(unclassified);
            let escaped = escaped_parameter_expansion_brace_edge_spans(fact.word(), source)
                .into_iter()
                .filter(|span| {
                    !span_inside_nested_escaped_parameter_template(fact.word(), *span, source)
                })
                .collect::<Vec<_>>();
            spans.extend(escaped);
        }
    }

    spans.extend(uncovered_command_brace_spans(
        commands,
        source,
        heredoc_ranges,
    ));
    spans.extend(unmatched_command_substitution_brace_spans(
        commands,
        source,
        heredoc_ranges,
    ));
    spans
}

fn is_find_exec_placeholder_word(
    commands: &[CommandFact<'_>],
    fact: &WordFact<'_>,
    source: &str,
) -> bool {
    if !word_is_empty_brace_pair_variant(fact.word(), source) {
        return false;
    }
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = &commands[fact.command_id().index()];
    if command.has_wrapper(WrapperKind::FindExec) || command.has_wrapper(WrapperKind::FindExecDir) {
        return true;
    }

    commands.iter().any(|command| {
        command.stmt().span.start.offset <= fact.span().start.offset
            && command.stmt().span.end.offset >= fact.span().end.offset
            && is_find_exec_command(command, source)
    }) || line_has_find_exec_placeholder_context(source, fact.span())
}

fn is_find_exec_command(command: &CommandFact<'_>, source: &str) -> bool {
    let is_find = command.static_utility_name_is("find")
        || command.body_name_word().is_some_and(|name_word| {
            name_word
                .span
                .slice(source)
                .rsplit('/')
                .next()
                .is_some_and(|name| name == "find")
        });
    if !is_find {
        return false;
    }

    let has_exec_flag = command.body_args().iter().any(|arg| {
        matches!(
            arg.span.slice(source),
            "-exec" | "-execdir" | "-ok" | "-okdir"
        )
    });
    let has_exec_terminator = command
        .body_args()
        .iter()
        .any(|arg| matches!(arg.span.slice(source), "+" | "\\;"));

    has_exec_flag && has_exec_terminator
}

fn line_has_find_exec_placeholder_context(source: &str, brace_span: Span) -> bool {
    let Some(line_text) = source.lines().nth(brace_span.start.line.saturating_sub(1)) else {
        return false;
    };
    let line_start_offset = source
        .lines()
        .take(brace_span.start.line.saturating_sub(1))
        .map(|line| line.len() + '\n'.len_utf8())
        .sum::<usize>();
    let Some(relative_start) = brace_span.start.offset.checked_sub(line_start_offset) else {
        return false;
    };
    let Some(relative_end) = brace_span.end.offset.checked_sub(line_start_offset) else {
        return false;
    };
    if relative_end > line_text.len() {
        return false;
    }

    let prefix = &line_text[..relative_start];
    let suffix = &line_text[relative_end..];
    let first_word = shellish_words(prefix).into_iter().next();
    let has_exec_flag_before = shellish_words(prefix)
        .into_iter()
        .any(|word| matches!(word, "-exec" | "-execdir" | "-ok" | "-okdir"));
    let has_exec_terminator_after = shellish_words(suffix)
        .into_iter()
        .any(|word| matches!(word, "+" | "\\;"));

    first_word
        .and_then(|word| word.rsplit('/').next())
        .is_some_and(|word| word == "find")
        && has_exec_flag_before
        && has_exec_terminator_after
}

fn is_xargs_replacement_word(
    commands: &[CommandFact<'_>],
    fact: &WordFact<'_>,
    source: &str,
) -> bool {
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = &commands[fact.command_id().index()];
    if !command.effective_name_is("xargs") {
        return false;
    }

    xargs_replacement_spans(command.body_args(), source)
        .into_iter()
        .any(|span| span == fact.word().span)
}

fn xargs_replacement_spans(args: &[&Word], source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            if let Some(replacement) = long.strip_prefix("replace=") {
                if !replacement.is_empty() {
                    spans.push(word.span);
                }
                index += 1;
                continue;
            }

            if long == "replace" {
                let Some(next_word) = args.get(index + 1) else {
                    break;
                };
                spans.push(next_word.span);
                index += 2;
                continue;
            }

            let consume_next_argument = xargs_long_option_requires_separate_argument(long);
            index += 1;
            if consume_next_argument {
                index += 1;
            }
            continue;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;

        while let Some(flag) = chars.next() {
            match flag {
                'i' => {
                    if chars.peek().is_some() {
                        spans.push(word.span);
                    }
                    break;
                }
                'I' => {
                    if chars.peek().is_some() {
                        spans.push(word.span);
                    } else {
                        let Some(next_word) = args.get(index + 1) else {
                            return spans;
                        };
                        spans.push(next_word.span);
                        consume_next_argument = true;
                    }
                    break;
                }
                _ => match xargs_short_option_argument_style(flag) {
                    XargsShortOptionArgumentStyle::None => {}
                    XargsShortOptionArgumentStyle::OptionalInlineOnly => break,
                    XargsShortOptionArgumentStyle::Required => {
                        if chars.peek().is_none() {
                            consume_next_argument = true;
                        }
                        break;
                    }
                },
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    spans
}

fn shellish_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = None;

    for (index, ch) in text.char_indices() {
        let is_word =
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '/' | '\\' | ';' | '.');
        if is_word {
            if start.is_none() {
                start = Some(index);
            }
        } else if let Some(word_start) = start.take() {
            words.push(&text[word_start..index]);
        }
    }

    if let Some(word_start) = start {
        words.push(&text[word_start..]);
    }

    words
}

fn brace_character_spans(span: Span, source: &str) -> Vec<Span> {
    let text = span.slice(source);
    text.char_indices()
        .filter(|&(_, ch)| matches!(ch, '{' | '}'))
        .filter_map(|(offset, _)| {
            let absolute_offset = span.start.offset + offset;
            if has_odd_backslash_run_before(source, absolute_offset) {
                return None;
            }
            let position = span.start.advanced_by(&text[..offset]);
            Some(Span::from_positions(position, position))
        })
        .collect()
}

fn brace_span_has_escaped_dollar_prefix(span: Span, source: &str) -> bool {
    let span_text = span.slice(source);
    if span_text.starts_with("${") {
        return has_odd_backslash_run_before(source, span.start.offset);
    }

    has_escaped_dollar_before(source, span.start.offset)
}

fn brace_syntax_with_whitespace_is_literal(brace: shuck_ast::BraceSyntax, source: &str) -> bool {
    if !matches!(brace.kind, BraceSyntaxKind::Expansion(_)) {
        return false;
    }

    #[derive(Clone, Copy)]
    enum QuoteState {
        Single,
        Double,
    }

    let text = brace.span.slice(source);
    let mut index = 0usize;
    let mut quote_state = None;

    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if let Some(state) = quote_state {
            match state {
                QuoteState::Single => {
                    if ch == '\'' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
                QuoteState::Double => {
                    if ch == '\\' {
                        index += ch_len;
                        if let Some(escaped) = text[index..].chars().next() {
                            index += escaped.len_utf8();
                        }
                        continue;
                    }
                    if ch == '"' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
            }
        }

        if ch == '\\' {
            index += ch_len;
            if text[index..].starts_with("\r\n") {
                index += "\r\n".len();
                continue;
            }
            if text[index..].starts_with('\n') {
                index += '\n'.len_utf8();
                continue;
            }
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '\'' {
            quote_state = Some(QuoteState::Single);
            index += ch_len;
            continue;
        }

        if ch == '"' {
            quote_state = Some(QuoteState::Double);
            index += ch_len;
            continue;
        }

        if ch.is_whitespace() {
            return true;
        }

        index += ch_len;
    }

    false
}

fn word_is_empty_brace_pair_variant(word: &Word, source: &str) -> bool {
    matches!(word.span.slice(source), "{}" | "\\{\\}")
}

fn unclassified_literal_brace_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut excluded = Vec::new();
    collect_dynamic_brace_exclusions(
        &word.parts,
        span.start.offset,
        span.end.offset,
        source,
        &mut excluded,
    );
    excluded.extend(
        word.brace_syntax()
            .iter()
            .map(|brace| DynamicBraceExcludedSpan {
                start_offset: brace.span.start.offset - span.start.offset,
                end_offset: brace.span.end.offset - span.start.offset,
                kind: DynamicBraceExcludedSpanKind::RuntimeShellSyntax,
            }),
    );
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));

    let mut spans = Vec::new();
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut unmatched_opens = Vec::new();

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }
            if excluded_span.start_offset > index {
                break;
            }

            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if text[index..].starts_with("\\${")
            && let Some(end_offset) =
                find_runtime_parameter_closing_brace(text, index + '\\'.len_utf8())
        {
            index = end_offset;
            continue;
        }

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '{' {
            unmatched_opens.push(index);
        } else if ch == '}' && unmatched_opens.pop().is_none() {
            let position = span.start.advanced_by(&text[..index]);
            spans.push(Span::from_positions(position, position));
        }

        index += ch_len;
    }

    spans.extend(unmatched_opens.into_iter().map(|offset| {
        let position = span.start.advanced_by(&text[..offset]);
        Span::from_positions(position, position)
    }));

    spans
}

fn uncovered_command_brace_spans(
    commands: &[CommandFact<'_>],
    source: &str,
    heredoc_ranges: &[TextRange],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for command in commands {
        let Command::Simple(simple) = command.command() else {
            continue;
        };
        let command_span = command.span();
        let mut covered = Vec::new();

        if !simple.name.span.slice(source).is_empty() {
            covered.push(simple.name.span);
        }
        covered.extend(simple.args.iter().map(|word| word.span));
        covered.extend(simple.assignments.iter().map(|assignment| assignment.span));
        covered.extend(command.redirects().iter().map(|redirect| redirect.span));
        covered.extend(command.substitution_facts().iter().map(|fact| fact.span()));
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect.fd_var_span),
        );
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect_fd_var_brace_span(redirect, source)),
        );
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect.heredoc().map(|heredoc| heredoc.body.span)),
        );
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect.fd_var_span),
        );

        if covered.is_empty() {
            continue;
        }

        covered.sort_by_key(|span| (span.start.offset, span.end.offset));

        let mut cursor = command_span.start.offset;
        for span in covered {
            if span.start.offset > cursor {
                spans.extend(raw_literal_brace_spans(
                    command_span,
                    cursor,
                    span.start.offset,
                    source,
                    RawLiteralBraceScanMode::All,
                    heredoc_ranges,
                ));
            }
            cursor = cursor.max(span.end.offset);
        }

        if command_span.end.offset > cursor {
            spans.extend(raw_literal_brace_spans(
                command_span,
                cursor,
                command_span.end.offset,
                source,
                RawLiteralBraceScanMode::All,
                heredoc_ranges,
            ));
        }
    }

    spans
}

fn redirect_fd_var_brace_span(redirect: &Redirect, source: &str) -> Option<Span> {
    let fd_var_span = redirect.fd_var_span?;
    let start_offset = fd_var_span.start.offset.checked_sub('{'.len_utf8())?;
    let end_offset = fd_var_span.end.offset.checked_add('}'.len_utf8())?;
    if source.get(start_offset..fd_var_span.start.offset)? != "{" {
        return None;
    }
    if source.get(fd_var_span.end.offset..end_offset)? != "}" {
        return None;
    }

    Some(Span::from_positions(
        Position {
            line: fd_var_span.start.line,
            column: fd_var_span.start.column.checked_sub(1)?,
            offset: start_offset,
        },
        Position {
            line: fd_var_span.end.line,
            column: fd_var_span.end.column + 1,
            offset: end_offset,
        },
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawLiteralBraceScanMode {
    All,
    UnmatchedOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawLiteralBraceQuoteState {
    Single,
    Double,
}

fn raw_literal_brace_spans(
    container_span: Span,
    scan_start: usize,
    scan_end: usize,
    source: &str,
    mode: RawLiteralBraceScanMode,
    excluded_ranges: &[TextRange],
) -> Vec<Span> {
    let mut relevant_excluded = excluded_ranges
        .iter()
        .filter_map(|range| {
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            if end <= scan_start || start >= scan_end {
                return None;
            }
            Some((start.max(scan_start), end.min(scan_end)))
        })
        .collect::<Vec<_>>();
    relevant_excluded.sort_unstable_by_key(|&(start, end)| (start, end));

    let mut spans = Vec::new();
    let mut unmatched_opens = Vec::new();
    let mut cursor = scan_start;
    for (start, end) in relevant_excluded {
        if start > cursor {
            spans.extend(raw_literal_brace_spans_without_exclusions(
                container_span,
                cursor,
                start,
                source,
                mode,
                &mut unmatched_opens,
            ));
        }
        cursor = cursor.max(end);
    }

    if scan_end > cursor {
        spans.extend(raw_literal_brace_spans_without_exclusions(
            container_span,
            cursor,
            scan_end,
            source,
            mode,
            &mut unmatched_opens,
        ));
    }

    if mode == RawLiteralBraceScanMode::UnmatchedOnly {
        spans.extend(unmatched_opens);
    }

    spans
}

fn raw_literal_brace_spans_without_exclusions(
    container_span: Span,
    scan_start: usize,
    scan_end: usize,
    source: &str,
    mode: RawLiteralBraceScanMode,
    unmatched_opens: &mut Vec<Span>,
) -> Vec<Span> {
    let text = &source[scan_start..scan_end];
    if text.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut index = 0usize;
    let mut quote_state = None;
    let mut in_comment = false;

    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if in_comment {
            if ch == '\n' {
                in_comment = false;
            }
            index += ch_len;
            continue;
        }

        if let Some(state) = quote_state {
            match state {
                RawLiteralBraceQuoteState::Single => {
                    if ch == '\'' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
                RawLiteralBraceQuoteState::Double => {
                    if ch == '\\' {
                        index += ch_len;
                        if let Some(escaped) = text[index..].chars().next() {
                            index += escaped.len_utf8();
                        }
                        continue;
                    }
                    if ch == '"' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
            }
        }

        if text[index..].starts_with("${")
            && let Some(end_offset) = find_runtime_parameter_closing_brace(text, index)
        {
            index = end_offset;
            continue;
        }

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '#' {
            in_comment = true;
            index += ch_len;
            continue;
        }

        if ch == '\'' {
            quote_state = Some(RawLiteralBraceQuoteState::Single);
            index += ch_len;
            continue;
        }

        if ch == '"' {
            quote_state = Some(RawLiteralBraceQuoteState::Double);
            index += ch_len;
            continue;
        }

        if matches!(ch, '{' | '}') {
            if mode == RawLiteralBraceScanMode::UnmatchedOnly
                && brace_at_command_start(text, index, ch)
            {
                index += ch_len;
                continue;
            }

            let absolute_offset = scan_start + index;
            let position = container_span
                .start
                .advanced_by(&source[container_span.start.offset..absolute_offset]);
            let span = Span::from_positions(position, position);
            match mode {
                RawLiteralBraceScanMode::All => spans.push(span),
                RawLiteralBraceScanMode::UnmatchedOnly => {
                    if ch == '{' {
                        unmatched_opens.push(span);
                    } else if unmatched_opens.pop().is_none() {
                        spans.push(span);
                    }
                }
            }
        }

        index += ch_len;
    }

    spans
}

fn brace_at_command_start(text: &str, index: usize, ch: char) -> bool {
    match ch {
        '{' => opening_brace_starts_shell_group(text, index),
        '}' => closing_brace_ends_shell_group(text, index),
        _ => false,
    }
}

fn opening_brace_starts_shell_group(text: &str, index: usize) -> bool {
    let Some(next) = text[index + '{'.len_utf8()..].chars().next() else {
        return false;
    };
    if !next.is_whitespace() {
        return false;
    }

    let prefix = text[..index].trim_end_matches([' ', '\t']);
    let Some(last) = prefix.chars().next_back() else {
        return true;
    };

    match last {
        '\n' | '&' | '|' | '(' | ')' => true,
        ';' => prefix.chars().rev().nth(1) != Some('\\'),
        'o' => prefix.ends_with("do"),
        'n' => prefix.ends_with("then"),
        'e' => prefix.ends_with("else"),
        'f' => prefix.ends_with("elif"),
        _ => false,
    }
}

fn closing_brace_ends_shell_group(text: &str, index: usize) -> bool {
    let prefix = text[..index].trim_end_matches([' ', '\t']);
    let Some(last) = prefix.chars().next_back() else {
        return true;
    };

    match last {
        '\n' | '&' | '|' | '(' => true,
        ';' => prefix.chars().rev().nth(1) != Some('\\'),
        _ => false,
    }
}

fn unmatched_command_substitution_brace_spans(
    commands: &[CommandFact<'_>],
    source: &str,
    heredoc_ranges: &[TextRange],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for substitution in commands
        .iter()
        .flat_map(|command| command.substitution_facts())
    {
        let Some((container_span, body_start, body_end)) =
            command_substitution_body_offsets(substitution.span(), source)
        else {
            continue;
        };

        if body_end > body_start {
            spans.extend(raw_literal_brace_spans(
                container_span,
                body_start,
                body_end,
                source,
                RawLiteralBraceScanMode::UnmatchedOnly,
                heredoc_ranges,
            ));
        }
    }

    spans
}

fn command_substitution_body_offsets(span: Span, source: &str) -> Option<(Span, usize, usize)> {
    let text = span.slice(source);
    if text.starts_with("$(") && text.ends_with(')') && text.len() >= 3 {
        return Some((
            span,
            span.start.offset + "$(".len(),
            span.end.offset - ')'.len_utf8(),
        ));
    }
    if text.starts_with('`') && text.ends_with('`') && text.len() >= 2 {
        return Some((
            span,
            span.start.offset + '`'.len_utf8(),
            span.end.offset - '`'.len_utf8(),
        ));
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct LiteralBraceCandidate {
    open_offset: usize,
    after_escaped_dollar: bool,
    has_excluded_content_inside: bool,
    has_nested_parameter_inside: bool,
    has_runtime_shell_sigil_inside: bool,
    has_brace_expansion_delimiter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicBraceExcludedSpanKind {
    Quoted,
    RuntimeShellSyntax,
}

#[derive(Debug, Clone, Copy)]
struct DynamicBraceExcludedSpan {
    start_offset: usize,
    end_offset: usize,
    kind: DynamicBraceExcludedSpanKind,
}

fn escaped_parameter_expansion_brace_edge_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut spans = Vec::new();
    let mut literal_stack: Vec<LiteralBraceCandidate> = Vec::new();
    let mut excluded = Vec::new();
    collect_dynamic_brace_exclusions(
        &word.parts,
        span.start.offset,
        span.end.offset,
        source,
        &mut excluded,
    );
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut previous_char = None;
    let mut previous_char_escaped = false;

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }

            if excluded_span.start_offset > index {
                break;
            }

            if excluded_span.kind == DynamicBraceExcludedSpanKind::RuntimeShellSyntax
                && let Some(current) = literal_stack.last_mut()
            {
                current.has_runtime_shell_sigil_inside = true;
            }
            if let Some(current) = literal_stack.last_mut() {
                current.has_excluded_content_inside = true;
            }
            if excluded_span.kind == DynamicBraceExcludedSpanKind::RuntimeShellSyntax
                && excluded_runtime_syntax_has_escaped_dollar_prefix(
                    text,
                    excluded_span.start_offset,
                    excluded_span.end_offset,
                )
            {
                let excluded_text = &text[excluded_span.start_offset..excluded_span.end_offset];
                let open_offset = if excluded_text.starts_with("${") {
                    Some(excluded_span.start_offset + '$'.len_utf8())
                } else if excluded_text.starts_with('{') {
                    Some(excluded_span.start_offset)
                } else {
                    None
                };
                if let Some(open_offset) = open_offset
                    && excluded_text.ends_with('}')
                    && excluded_span.end_offset > open_offset + 1
                {
                    let open = span.start.advanced_by(&text[..open_offset]);
                    let close = span
                        .start
                        .advanced_by(&text[..excluded_span.end_offset - '}'.len_utf8()]);
                    spans.push(Span::from_positions(open, open));
                    spans.push(Span::from_positions(close, close));
                }
            }
            previous_char = None;
            previous_char_escaped = false;
            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                previous_char = Some(escaped);
                previous_char_escaped = true;
                index += escaped.len_utf8();
            } else {
                previous_char = Some('\\');
                previous_char_escaped = false;
            }
            continue;
        }

        if ch == '{' {
            if previous_char == Some('$')
                && !previous_char_escaped
                && let Some(candidate) = literal_stack.last_mut()
            {
                candidate.has_nested_parameter_inside = true;
            }
            literal_stack.push(LiteralBraceCandidate {
                open_offset: index,
                after_escaped_dollar: previous_char == Some('$') && previous_char_escaped,
                has_excluded_content_inside: false,
                has_nested_parameter_inside: false,
                has_runtime_shell_sigil_inside: false,
                has_brace_expansion_delimiter: false,
            });
        } else if ch == ','
            && let Some(candidate) = literal_stack.last_mut()
        {
            candidate.has_brace_expansion_delimiter = true;
        } else if ch == '.'
            && previous_char == Some('.')
            && !previous_char_escaped
            && let Some(candidate) = literal_stack.last_mut()
        {
            candidate.has_brace_expansion_delimiter = true;
        } else if ch == '}'
            && let Some(candidate) = literal_stack.pop()
            && index > candidate.open_offset + 1
            && (candidate.after_escaped_dollar
                || candidate.has_excluded_content_inside
                || candidate.has_runtime_shell_sigil_inside)
            && !(candidate.after_escaped_dollar && candidate.has_nested_parameter_inside)
            && !candidate.has_brace_expansion_delimiter
            && !brace_pair_matches_nonliteral_syntax(word, candidate.open_offset, index)
        {
            let open = span.start.advanced_by(&text[..candidate.open_offset]);
            let close = span.start.advanced_by(&text[..index]);
            spans.push(Span::from_positions(open, open));
            spans.push(Span::from_positions(close, close));
        }

        previous_char = Some(ch);
        previous_char_escaped = false;
        index += ch_len;
    }

    spans.extend(raw_escaped_parameter_brace_edge_spans(word, source));
    spans
}

fn excluded_runtime_syntax_has_escaped_dollar_prefix(
    text: &str,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    let start_offset = start_offset.min(text.len());
    let end_offset = end_offset.min(text.len());
    if start_offset >= end_offset {
        return false;
    }

    let excluded_text = &text[start_offset..end_offset];
    if excluded_text.starts_with("${") {
        return has_odd_backslash_run_before(text, start_offset);
    }
    if excluded_text.starts_with('{') {
        return has_escaped_dollar_before(text, start_offset);
    }
    false
}

fn has_odd_backslash_run_before(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    text[..offset]
        .chars()
        .rev()
        .take_while(|&ch| ch == '\\')
        .count()
        % 2
        == 1
}

fn has_escaped_dollar_before(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    let prefix = &text[..offset];
    let Some((dollar_offset, '$')) = prefix.char_indices().next_back() else {
        return false;
    };

    has_odd_backslash_run_before(text, dollar_offset)
}

fn collect_dynamic_brace_exclusions(
    parts: &[WordPartNode],
    word_base_offset: usize,
    word_end_offset: usize,
    source: &str,
    out: &mut Vec<DynamicBraceExcludedSpan>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {}
            WordPart::DoubleQuoted { .. } if !part.span.slice(source).starts_with("\\\"") => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::DoubleQuoted { .. } => {}
            WordPart::SingleQuoted { .. } => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
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
            | WordPart::ZshQualifiedGlob(_) => out.push(runtime_shell_dynamic_brace_exclusion(
                part,
                word_base_offset,
                word_end_offset,
                source,
            )),
        }
    }
}

fn runtime_shell_dynamic_brace_exclusion(
    part: &WordPartNode,
    word_base_offset: usize,
    word_end_offset: usize,
    source: &str,
) -> DynamicBraceExcludedSpan {
    let start_offset = part.span.start.offset - word_base_offset;
    let mut end_offset = part.span.end.offset - word_base_offset;
    let part_text = part.span.slice(source);
    let word_text = &source[word_base_offset..word_end_offset.min(source.len())];

    if let Some(relative_parameter_start) = part_text.find("${") {
        end_offset = find_runtime_parameter_closing_brace(
            word_text,
            start_offset + relative_parameter_start,
        )
        .map_or(end_offset, |closing_offset| end_offset.max(closing_offset));
    }

    DynamicBraceExcludedSpan {
        start_offset,
        end_offset,
        kind: DynamicBraceExcludedSpanKind::RuntimeShellSyntax,
    }
}

fn find_runtime_parameter_closing_brace(text: &str, start_offset: usize) -> Option<usize> {
    if start_offset >= text.len() || !text[start_offset..].starts_with("${") {
        return None;
    }

    let mut index = start_offset + "${".len();
    let mut depth = 1usize;

    while index < text.len() {
        if text[index..].starts_with("${") {
            depth += 1;
            index += "${".len();
            continue;
        }

        let ch = text[index..].chars().next()?;

        if ch == '\\' {
            index += ch.len_utf8();
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '}' {
            depth -= 1;
            index += ch.len_utf8();
            if depth == 0 {
                return Some(index);
            }
            continue;
        }

        index += ch.len_utf8();
    }

    None
}

fn raw_escaped_parameter_brace_edge_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut excluded = Vec::new();
    collect_raw_escaped_parameter_exclusions(&word.parts, span.start.offset, source, &mut excluded);
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));

    let mut spans = Vec::new();
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut previous_char = None;
    let mut previous_char_escaped = false;
    let mut escaped_parameter_stack: Vec<(usize, bool)> = Vec::new();
    let mut parameter_depth = 0usize;

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }
            if excluded_span.start_offset > index {
                break;
            }

            previous_char = None;
            previous_char_escaped = false;
            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                previous_char = Some(escaped);
                previous_char_escaped = true;
                index += escaped.len_utf8();
            } else {
                previous_char = Some('\\');
                previous_char_escaped = false;
            }
            continue;
        }

        if ch == '{' {
            if previous_char == Some('$') && previous_char_escaped {
                escaped_parameter_stack.push((index, false));
            } else if previous_char == Some('$') && !previous_char_escaped {
                if let Some((_, has_nested_parameter_inside)) = escaped_parameter_stack.last_mut() {
                    *has_nested_parameter_inside = true;
                }
                parameter_depth += 1;
            }
        } else if ch == '}' {
            if parameter_depth > 0 {
                parameter_depth -= 1;
            } else if let Some((open_offset, has_nested_parameter_inside)) =
                escaped_parameter_stack.pop()
                && !has_nested_parameter_inside
                && !brace_pair_matches_nonliteral_syntax(word, open_offset, index)
            {
                let open = span.start.advanced_by(&text[..open_offset]);
                let close = span.start.advanced_by(&text[..index]);
                spans.push(Span::from_positions(open, open));
                spans.push(Span::from_positions(close, close));
            }
        }

        previous_char = Some(ch);
        previous_char_escaped = false;
        index += ch_len;
    }

    spans
}

fn brace_pair_matches_nonliteral_syntax(
    word: &Word,
    open_offset: usize,
    close_offset: usize,
) -> bool {
    let absolute_open_offset = word.span.start.offset + open_offset;
    let absolute_close_offset = word.span.start.offset + close_offset + '}'.len_utf8();

    word.brace_syntax().iter().any(|brace| {
        brace.kind != BraceSyntaxKind::Literal
            && brace.span.start.offset == absolute_open_offset
            && brace.span.end.offset == absolute_close_offset
    })
}

fn span_inside_nested_escaped_parameter_template(word: &Word, span: Span, source: &str) -> bool {
    if span.start.offset < word.span.start.offset || span.start.offset >= word.span.end.offset {
        return false;
    }

    let text = word.span.slice(source);
    let relative_offset = span.start.offset - word.span.start.offset;
    let mut index = 0usize;

    while index < text.len() {
        if text[index..].starts_with("\\${")
            && let Some(end_offset) =
                find_runtime_parameter_closing_brace(text, index + '\\'.len_utf8())
        {
            let body_start = index + "\\${".len();
            let body_end = end_offset.saturating_sub('}'.len_utf8());
            let has_nested_parameter =
                body_start < body_end && text[body_start..body_end].contains("${");
            let open_brace_offset = index + "\\$".len();
            if has_nested_parameter
                && relative_offset > open_brace_offset
                && relative_offset < end_offset.saturating_sub('}'.len_utf8())
            {
                return true;
            }
            index = end_offset;
            continue;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        index += ch_len;
    }

    false
}

fn collect_raw_escaped_parameter_exclusions(
    parts: &[WordPartNode],
    word_base_offset: usize,
    source: &str,
    out: &mut Vec<DynamicBraceExcludedSpan>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_)
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
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::DoubleQuoted { .. } if !part.span.slice(source).starts_with("\\\"") => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::DoubleQuoted { .. } => {}
            WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. } => out.push(DynamicBraceExcludedSpan {
                start_offset: part.span.start.offset - word_base_offset,
                end_offset: part.span.end.offset - word_base_offset,
                kind: DynamicBraceExcludedSpanKind::Quoted,
            }),
        }
    }
}

fn directive_can_apply_to_following_command(prefix: &str) -> bool {
    let trimmed = prefix.trim_end();
    trimmed.ends_with(';')
        || trimmed.ends_with('{')
        || trimmed.ends_with('(')
        || ends_with_keyword(trimmed, "then")
        || ends_with_keyword(trimmed, "do")
        || ends_with_keyword(trimmed, "else")
}

fn is_inline_shellcheck_directive(comment_text: &str) -> bool {
    let body = comment_text
        .trim_start()
        .trim_start_matches('#')
        .trim_start();
    let Some(remainder) = strip_prefix_ignore_ascii_case(body, "shellcheck") else {
        return false;
    };
    let Some(first) = remainder.chars().next() else {
        return false;
    };
    if !first.is_ascii_whitespace() {
        return false;
    }

    let mut body = remainder;
    if let Some((before, _)) = body.split_once('#') {
        body = before;
    }

    body.split_ascii_whitespace().any(|part| {
        [
            "disable=",
            "enable=",
            "disable-file=",
            "source=",
            "shell=",
            "external-sources=",
        ]
        .into_iter()
        .any(|prefix| {
            strip_prefix_ignore_ascii_case(part, prefix)
                .is_some_and(|value| !value.trim().is_empty())
        })
    })
}

fn ends_with_keyword(text: &str, keyword: &str) -> bool {
    text == keyword
        || text
            .strip_suffix(keyword)
            .and_then(|prefix| prefix.chars().last())
            .is_some_and(|ch| ch.is_ascii_whitespace())
}

fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = text.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn build_double_paren_grouping_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Compound(CompoundCommand::Subshell(_)) => {
                double_paren_grouping_anchor(fact.span(), source)
            }
            _ => None,
        })
        .collect()
}

fn build_arithmetic_for_update_operator_spans(
    commands: &[CommandFact<'_>],
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        let Command::Compound(CompoundCommand::ArithmeticFor(command)) = fact.command() else {
            continue;
        };

        collect_arithmetic_update_operator_spans(command.init_ast.as_ref(), source, &mut spans);
        collect_arithmetic_update_operator_spans(
            command.condition_ast.as_ref(),
            source,
            &mut spans,
        );
        collect_arithmetic_update_operator_spans(command.step_ast.as_ref(), source, &mut spans);
    }

    spans
}

fn collect_arithmetic_update_operator_spans(
    expression: Option<&ArithmeticExprNode>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(expression) = expression else {
        return;
    };

    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) | ArithmeticExpr::ShellWord(_) => {}
        ArithmeticExpr::Indexed { index, .. } => {
            collect_arithmetic_update_operator_spans(Some(index), source, spans);
        }
        ArithmeticExpr::Parenthesized { expression } => {
            collect_arithmetic_update_operator_spans(Some(expression), source, spans);
        }
        ArithmeticExpr::Unary { op, expr } => {
            if matches!(
                op,
                ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
            ) {
                spans.push(find_operator_span(
                    expression.span,
                    source,
                    match op {
                        ArithmeticUnaryOp::PreIncrement => "++",
                        ArithmeticUnaryOp::PreDecrement => "--",
                        ArithmeticUnaryOp::Plus
                        | ArithmeticUnaryOp::Minus
                        | ArithmeticUnaryOp::LogicalNot
                        | ArithmeticUnaryOp::BitwiseNot => unreachable!(),
                    },
                    true,
                ));
            }
            collect_arithmetic_update_operator_spans(Some(expr), source, spans);
        }
        ArithmeticExpr::Postfix { expr, op } => {
            spans.push(find_operator_span(
                expression.span,
                source,
                match op {
                    ArithmeticPostfixOp::Increment => "++",
                    ArithmeticPostfixOp::Decrement => "--",
                },
                false,
            ));
            collect_arithmetic_update_operator_spans(Some(expr), source, spans);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_update_operator_spans(Some(left), source, spans);
            collect_arithmetic_update_operator_spans(Some(right), source, spans);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_update_operator_spans(Some(condition), source, spans);
            collect_arithmetic_update_operator_spans(Some(then_expr), source, spans);
            collect_arithmetic_update_operator_spans(Some(else_expr), source, spans);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_arithmetic_lvalue_update_operator_spans(target, source, spans);
            collect_arithmetic_update_operator_spans(Some(value), source, spans);
        }
    }
}

fn collect_arithmetic_lvalue_update_operator_spans(
    target: &ArithmeticLvalue,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            collect_arithmetic_update_operator_spans(Some(index), source, spans);
        }
    }
}

fn find_operator_span(expression_span: Span, source: &str, operator: &str, first: bool) -> Span {
    let expression = expression_span.slice(source);
    let offset = if first {
        expression
            .find(operator)
            .expect("expected prefix update operator in arithmetic expression")
    } else {
        expression
            .rfind(operator)
            .expect("expected postfix update operator in arithmetic expression")
    };
    let start = expression_span.start.advanced_by(&expression[..offset]);
    Span::from_positions(start, start.advanced_by(operator))
}

fn double_paren_grouping_anchor(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let body_start = if let Some(stripped) = text.strip_prefix("((") {
        (text.len() - stripped.len()) + stripped.find(|char: char| !char.is_whitespace())?
    } else if text.starts_with('(')
        && span.start.offset > 0
        && source.as_bytes().get(span.start.offset - 1) == Some(&b'(')
    {
        let stripped = text.strip_prefix('(')?;
        (text.len() - stripped.len()) + stripped.find(|char: char| !char.is_whitespace())?
    } else {
        return None;
    };

    let body = &text[body_start..];
    let has_grouping_operator =
        body.contains("||") || body.contains("&&") || body.contains('|') || body.contains(';');
    if !has_grouping_operator {
        return None;
    }

    let command_offset = body.find(|char: char| char == '_' || char.is_ascii_alphabetic())?;
    let command_start = span.start.advanced_by(&text[..body_start + command_offset]);
    let head = &body[command_offset..];
    let first_char_len = head.chars().next()?.len_utf8();
    let command_end = command_start.advanced_by(&head[..first_char_len]);
    Some(Span::from_positions(command_start, command_end))
}

fn has_header_shellcheck_shell_directive(source: &str) -> bool {
    for line in source.lines().skip(1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix('#') {
            let body = comment.trim_start().to_ascii_lowercase();
            if body.starts_with("shellcheck shell=") {
                return true;
            }
            continue;
        }
        break;
    }

    false
}

fn build_backtick_command_name_spans(commands: &[CommandFact<'_>]) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Simple(command) => plain_backtick_command_name_span(&command.name),
            _ => None,
        })
        .collect::<Vec<_>>();

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

fn plain_backtick_command_name_span(word: &Word) -> Option<Span> {
    let [part] = word.parts.as_slice() else {
        return None;
    };

    match &part.kind {
        WordPart::CommandSubstitution {
            syntax: CommandSubstitutionSyntax::Backtick,
            ..
        } => Some(part.span),
        _ => None,
    }
}

fn collect_condition_command_substitution_from_body(
    condition: &StmtSeq,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for stmt in condition.iter() {
        collect_terminal_command_substitution_spans_in_stmt(stmt, source, spans);
    }
}

fn collect_terminal_command_substitution_spans_in_stmt(
    stmt: &Stmt,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_terminal_command_substitution_spans_in_command(&stmt.command, source, spans);
}

fn collect_terminal_command_substitution_spans_in_command(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            if command_name_is_plain_command_substitution(&command.name, source) {
                spans.push(command.name.span);
            }
        }
        Command::Binary(command) => {
            collect_terminal_command_substitution_spans_in_stmt(&command.left, source, spans);
            collect_terminal_command_substitution_spans_in_stmt(&command.right, source, spans);
        }
        Command::Compound(CompoundCommand::Subshell(body))
        | Command::Compound(CompoundCommand::BraceGroup(body)) => {
            for stmt in body.iter() {
                collect_terminal_command_substitution_spans_in_stmt(stmt, source, spans);
            }
        }
        Command::Compound(CompoundCommand::Time(command)) => {
            if let Some(inner) = &command.command {
                collect_terminal_command_substitution_spans_in_stmt(inner, source, spans);
            }
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
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

fn build_dollar_question_after_command_spans(commands: &StmtSeq, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_dollar_question_after_command_spans_in_seq(commands, source, true, &mut spans);

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

fn build_nonpersistent_assignment_spans(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
) -> NonpersistentAssignmentSpans {
    let mut candidate_bindings_by_scope: FxHashMap<
        (Name, usize, usize),
        CandidateSubshellAssignment,
    > = FxHashMap::default();
    let mut persistent_reset_offsets_by_name: FxHashMap<Name, Vec<usize>> = FxHashMap::default();
    let mut command_id_query_offsets = Vec::new();
    let mut relevant_references = Vec::new();
    let mut relevant_synthetic_reads = Vec::new();

    for binding in semantic.bindings() {
        if !is_reportable_subshell_assignment(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }

        let Some(nonpersistent_scope) =
            innermost_nonpersistent_scope_span(semantic, binding.span.start.offset)
        else {
            continue;
        };

        candidate_bindings_by_scope
            .entry((
                binding.name.clone(),
                nonpersistent_scope.span.start.offset,
                nonpersistent_scope.span.end.offset,
            ))
            .or_insert(CandidateSubshellAssignment {
                binding_id: binding.id,
                assignment_span: binding.span,
                subshell_start: nonpersistent_scope.span.start.offset,
                subshell_end: nonpersistent_scope.span.end.offset,
            });
    }

    let mut candidate_bindings_by_name: FxHashMap<Name, Vec<CandidateSubshellAssignment>> =
        FxHashMap::default();
    for ((name, _, _), candidate) in candidate_bindings_by_scope {
        candidate_bindings_by_name
            .entry(name)
            .or_default()
            .push(candidate);
    }
    for candidates in candidate_bindings_by_name.values_mut() {
        candidates.sort_by_key(|candidate| {
            (
                candidate.subshell_end,
                candidate.assignment_span.start.offset,
                candidate.assignment_span.end.offset,
            )
        });
    }

    for binding in semantic.bindings() {
        if !is_persistent_subshell_reset_binding(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }
        if is_within_any_nonpersistent_scope(semantic, binding.span.start.offset) {
            continue;
        }
        persistent_reset_offsets_by_name
            .entry(binding.name.clone())
            .or_default()
            .push(binding.span.start.offset);
        command_id_query_offsets.push(binding.span.start.offset);
    }

    for reference in semantic.references() {
        if matches!(
            reference.kind,
            shuck_semantic::ReferenceKind::DeclarationName
        ) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&reference.name) {
            continue;
        }
        if candidate_bindings_by_name.contains_key(&reference.name) {
            command_id_query_offsets.push(reference.span.start.offset);
            relevant_references.push(reference);
        }
    }

    for synthetic_read in semantic.synthetic_reads() {
        if !is_reportable_nonpersistent_assignment_name(synthetic_read.name()) {
            continue;
        }
        if candidate_bindings_by_name.contains_key(synthetic_read.name()) {
            command_id_query_offsets.push(synthetic_read.span().start.offset);
            relevant_synthetic_reads.push(synthetic_read);
        }
    }

    let innermost_command_ids_by_offset =
        build_innermost_command_ids_by_offset(commands, command_id_query_offsets);
    let commands_by_id = commands
        .iter()
        .map(|command| (command.id(), command))
        .collect::<FxHashMap<_, _>>();
    let command_end_offsets = commands
        .iter()
        .map(|command| (command.id(), command.span().end.offset))
        .collect::<FxHashMap<_, _>>();
    let persistent_reset_offsets_by_name: FxHashMap<Name, Vec<PersistentReset>> =
        persistent_reset_offsets_by_name
            .into_iter()
            .map(|(name, offsets)| {
                let resets = offsets
                    .into_iter()
                    .map(|offset| {
                        let command_id = precomputed_command_id_for_offset(
                            &innermost_command_ids_by_offset,
                            offset,
                        );
                        let command_end_offset = command_id
                            .and_then(|id| command_end_offsets.get(&id).copied())
                            .unwrap_or(offset);

                        PersistentReset {
                            offset,
                            command_id,
                            command_end_offset,
                        }
                    })
                    .collect();
                (name, resets)
            })
            .collect();

    let mut later_use_sites = Vec::new();
    let mut assignment_sites = Vec::new();
    for reference in relevant_references {
        let Some(candidate_ids) = candidate_bindings_by_name.get(&reference.name) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(&reference.name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let event_command_id = precomputed_command_id_for_offset(
            &innermost_command_ids_by_offset,
            reference.span.start.offset,
        );
        let resolved = semantic.resolved_binding(reference.id);
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            reference.span.start.offset > candidate.subshell_end
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    reference.span.start.offset,
                    event_command_id,
                )
                && resolved.is_none_or(|resolved| {
                    resolved.id != candidate.binding_id
                        && resolved.span.start.offset < candidate.subshell_start
                })
        }) {
            assignment_sites.push(NamedSpan {
                name: reference.name.clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: reference.name.clone(),
                span: reference.span,
            });
        }
    }

    for synthetic_read in relevant_synthetic_reads {
        let Some(candidate_ids) = candidate_bindings_by_name.get(synthetic_read.name()) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(synthetic_read.name())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let synthetic_command_id = precomputed_command_id_for_offset(
            &innermost_command_ids_by_offset,
            synthetic_read.span().start.offset,
        );
        let same_command_prefix_reset = synthetic_command_id
            .and_then(|id| commands_by_id.get(&id).copied())
            .is_some_and(|command| {
                command_prefix_assignments_reset_name(command.command(), synthetic_read.name())
            });
        let synthetic_command_end_offset = synthetic_command_id
            .and_then(|id| command_end_offsets.get(&id).copied())
            .unwrap_or(synthetic_read.span().start.offset);
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            synthetic_read.span().start.offset > candidate.subshell_end
                && !same_command_prefix_reset
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    synthetic_command_end_offset,
                    None,
                )
        }) {
            assignment_sites.push(NamedSpan {
                name: synthetic_read.name().clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: synthetic_read.name().clone(),
                span: synthetic_read.span(),
            });
        }
    }

    for binding in semantic.bindings() {
        if !is_reportable_subshell_later_use_binding(binding.kind, binding.attributes) {
            continue;
        }
        if !is_reportable_nonpersistent_assignment_name(&binding.name) {
            continue;
        }

        let Some(candidate_ids) = candidate_bindings_by_name.get(&binding.name) else {
            continue;
        };

        let reset_offsets = persistent_reset_offsets_by_name
            .get(&binding.name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let Some(candidate) = candidate_ids.iter().rev().find(|candidate| {
            binding.span.start.offset > candidate.subshell_end
                && !has_intervening_persistent_reset(
                    reset_offsets,
                    candidate.subshell_end,
                    binding.span.start.offset,
                    None,
                )
        }) {
            assignment_sites.push(NamedSpan {
                name: binding.name.clone(),
                span: candidate.assignment_span,
            });
            later_use_sites.push(NamedSpan {
                name: binding.name.clone(),
                span: binding.span,
            });
        }
    }

    let mut seen = FxHashSet::default();
    later_use_sites.retain(|site| seen.insert((FactSpan::new(site.span), site.name.clone())));
    later_use_sites.sort_by_key(|site| (site.span.start.offset, site.span.end.offset));

    seen.clear();
    assignment_sites.retain(|site| seen.insert((FactSpan::new(site.span), site.name.clone())));
    assignment_sites.sort_by_key(|site| (site.span.start.offset, site.span.end.offset));

    NonpersistentAssignmentSpans {
        subshell_assignment_sites: assignment_sites,
        subshell_later_use_sites: later_use_sites,
    }
}

fn is_reportable_subshell_assignment(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => !attributes.contains(BindingAttributes::LOCAL),
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
                && !attributes.contains(BindingAttributes::LOCAL)
        }
        BindingKind::Imported => false,
        BindingKind::FunctionDefinition | BindingKind::Nameref => false,
    }
}

fn is_reportable_subshell_later_use_binding(
    kind: BindingKind,
    attributes: BindingAttributes,
) -> bool {
    match kind {
        BindingKind::AppendAssignment => true,
        BindingKind::ArithmeticAssignment => true,
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
                && !attributes.contains(BindingAttributes::LOCAL)
        }
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::FunctionDefinition
        | BindingKind::Imported
        | BindingKind::Nameref => false,
    }
}

fn is_reportable_nonpersistent_assignment_name(name: &Name) -> bool {
    name.as_str() != "IFS"
}

#[derive(Debug, Clone, Copy)]
struct CandidateSubshellAssignment {
    binding_id: shuck_semantic::BindingId,
    assignment_span: Span,
    subshell_start: usize,
    subshell_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct NonpersistentScopeSpan {
    span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSpan {
    pub name: Name,
    pub span: Span,
}

#[derive(Debug, Default)]
struct NonpersistentAssignmentSpans {
    subshell_assignment_sites: Vec<NamedSpan>,
    subshell_later_use_sites: Vec<NamedSpan>,
}

#[derive(Debug, Clone, Copy)]
struct PersistentReset {
    offset: usize,
    command_id: Option<CommandId>,
    command_end_offset: usize,
}

fn innermost_nonpersistent_scope_span(
    semantic: &SemanticModel,
    offset: usize,
) -> Option<NonpersistentScopeSpan> {
    let scope = innermost_nonpersistent_scope_within_function(semantic, offset)?;
    let span = semantic
        .scopes()
        .iter()
        .find(|candidate| candidate.id == scope)
        .map(|candidate| candidate.span)?;
    match semantic.scope_kind(scope) {
        shuck_semantic::ScopeKind::Pipeline
        | shuck_semantic::ScopeKind::Subshell
        | shuck_semantic::ScopeKind::CommandSubstitution => {}
        shuck_semantic::ScopeKind::Function(_) | shuck_semantic::ScopeKind::File => return None,
    }

    Some(NonpersistentScopeSpan { span })
}

fn is_within_any_nonpersistent_scope(semantic: &SemanticModel, offset: usize) -> bool {
    let scope = semantic.scope_at(offset);

    semantic.ancestor_scopes(scope).any(|scope| {
        matches!(
            semantic.scope_kind(scope),
            shuck_semantic::ScopeKind::Subshell
                | shuck_semantic::ScopeKind::CommandSubstitution
                | shuck_semantic::ScopeKind::Pipeline
        )
    })
}

fn is_persistent_subshell_reset_binding(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => !attributes.contains(BindingAttributes::LOCAL),
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
                && !attributes.contains(BindingAttributes::LOCAL)
        }
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

fn has_intervening_persistent_reset(
    resets: &[PersistentReset],
    candidate_end: usize,
    event_offset: usize,
    event_command_id: Option<CommandId>,
) -> bool {
    resets.iter().any(|reset| {
        let effective_offset = if reset.offset > candidate_end {
            reset.offset
        } else {
            reset.command_end_offset
        };

        effective_offset > candidate_end
            && effective_offset < event_offset
            && event_command_id.is_none_or(|event_id| reset.command_id != Some(event_id))
    })
}

fn command_prefix_assignments_reset_name(command: &Command, name: &Name) -> bool {
    query::command_assignments(command)
        .iter()
        .any(|assignment| assignment.target.name == *name)
}

fn build_innermost_command_ids_by_offset(
    commands: &[CommandFact<'_>],
    mut offsets: Vec<usize>,
) -> FxHashMap<usize, Option<CommandId>> {
    if offsets.is_empty() {
        return FxHashMap::default();
    }

    offsets.sort_unstable();
    offsets.dedup();

    let mut command_spans = commands
        .iter()
        .map(|command| (command.span(), command.id()))
        .collect::<Vec<_>>();
    if command_spans
        .windows(2)
        .any(|window| compare_command_offset_entries(window[0], window[1]).is_gt())
    {
        command_spans.sort_unstable_by(|left, right| compare_command_offset_entries(*left, *right));
    }

    let mut command_ids_by_offset = FxHashMap::default();
    let mut active_commands = Vec::new();
    let mut next_command = 0;
    for offset in offsets {
        pop_finished_commands(&mut active_commands, offset);

        while let Some((span, id)) = command_spans.get(next_command).copied() {
            if span.start.offset > offset {
                break;
            }

            pop_finished_commands(&mut active_commands, span.start.offset);
            active_commands.push(OpenCommand {
                end_offset: span.end.offset,
                id,
            });
            next_command += 1;
        }

        pop_finished_commands(&mut active_commands, offset);
        command_ids_by_offset.insert(offset, active_commands.last().map(|command| command.id));
    }

    command_ids_by_offset
}

fn compare_command_offset_entries(
    (left_span, left_id): (Span, CommandId),
    (right_span, right_id): (Span, CommandId),
) -> std::cmp::Ordering {
    left_span
        .start
        .offset
        .cmp(&right_span.start.offset)
        .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
        .then_with(|| right_id.index().cmp(&left_id.index()))
}

fn precomputed_command_id_for_offset(
    command_ids_by_offset: &FxHashMap<usize, Option<CommandId>>,
    offset: usize,
) -> Option<CommandId> {
    command_ids_by_offset.get(&offset).copied().unwrap_or(None)
}

#[derive(Debug, Clone, Copy)]
struct OpenCommand {
    end_offset: usize,
    id: CommandId,
}

fn pop_finished_commands(active_commands: &mut Vec<OpenCommand>, offset: usize) {
    while active_commands
        .last()
        .is_some_and(|command| command.end_offset < offset)
    {
        active_commands.pop();
    }
}

fn collect_dollar_question_after_command_spans_in_seq(
    commands: &StmtSeq,
    source: &str,
    mut status_available: bool,
    spans: &mut Vec<Span>,
) {
    for stmt in commands.iter() {
        collect_dollar_question_after_command_spans_in_stmt(stmt, source, status_available, spans);
        status_available = true;
    }
}

fn collect_dollar_question_after_command_spans_in_stmt(
    stmt: &Stmt,
    source: &str,
    status_available: bool,
    spans: &mut Vec<Span>,
) {
    collect_dollar_question_after_command_spans_in_command(
        &stmt.command,
        source,
        status_available,
        spans,
    );
}

fn collect_dollar_question_after_command_spans_in_command(
    command: &Command,
    source: &str,
    status_available: bool,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            if status_available {
                collect_c107_status_spans_in_simple_test(command, source, spans);
            }
        }
        Command::Compound(command) => match command {
            CompoundCommand::If(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.condition,
                    source,
                    status_available,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.then_branch,
                    source,
                    true,
                    spans,
                );
                for (condition, body) in &command.elif_branches {
                    collect_dollar_question_after_command_spans_in_seq(
                        condition, source, true, spans,
                    );
                    collect_dollar_question_after_command_spans_in_seq(body, source, true, spans);
                }
                if let Some(else_branch) = &command.else_branch {
                    collect_dollar_question_after_command_spans_in_seq(
                        else_branch,
                        source,
                        true,
                        spans,
                    );
                }
            }
            CompoundCommand::For(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Repeat(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Foreach(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::ArithmeticFor(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::While(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.condition,
                    source,
                    status_available,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Until(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.condition,
                    source,
                    status_available,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    collect_dollar_question_after_command_spans_in_seq(
                        &case.body, source, true, spans,
                    );
                }
            }
            CompoundCommand::Select(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                collect_dollar_question_after_command_spans_in_seq(body, source, true, spans);
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    collect_dollar_question_after_command_spans_in_stmt(
                        command,
                        source,
                        status_available,
                        spans,
                    );
                }
            }
            CompoundCommand::Conditional(command) => {
                if status_available {
                    collect_c107_status_spans_in_conditional_expr(
                        &command.expression,
                        source,
                        spans,
                    );
                }
            }
            CompoundCommand::Arithmetic(command) => {
                if status_available {
                    collect_c107_status_spans_in_arithmetic_command(command, source, spans);
                }
            }
            CompoundCommand::Coproc(command) => {
                collect_dollar_question_after_command_spans_in_stmt(
                    &command.body,
                    source,
                    true,
                    spans,
                );
            }
            CompoundCommand::Always(command) => {
                collect_dollar_question_after_command_spans_in_seq(
                    &command.body,
                    source,
                    true,
                    spans,
                );
                collect_dollar_question_after_command_spans_in_seq(
                    &command.always_body,
                    source,
                    true,
                    spans,
                );
            }
        },
        Command::Binary(command) => {
            collect_dollar_question_after_command_spans_in_stmt(
                &command.left,
                source,
                status_available,
                spans,
            );
            collect_dollar_question_after_command_spans_in_stmt(
                &command.right,
                source,
                true,
                spans,
            );
        }
        Command::AnonymousFunction(command) => {
            collect_dollar_question_after_command_spans_in_function_body(
                &command.body,
                source,
                spans,
            );
        }
        Command::Function(command) => {
            collect_dollar_question_after_command_spans_in_function_body(
                &command.body,
                source,
                spans,
            );
        }
        Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_dollar_question_after_command_spans_in_function_body(
    stmt: &Stmt,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(body))
        | Command::Compound(CompoundCommand::Subshell(body)) => {
            collect_dollar_question_after_command_spans_in_seq(body, source, false, spans);
        }
        _ => collect_dollar_question_after_command_spans_in_stmt(stmt, source, false, spans),
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
        operator.as_str(),
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
    crate::word_is_standalone_status_capture(word).then_some(word.span)
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

    collect_status_parameter_spans_in_stmt(first_stmt, source, spans);
}

fn condition_terminals_are_test_commands(condition: &StmtSeq, source: &str) -> bool {
    condition
        .last()
        .is_some_and(|stmt| stmt_terminals_are_test_commands(stmt, source))
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
        Command::Binary(command) if matches!(command.op, BinaryOp::And | BinaryOp::Or) => {
            stmt_terminals_are_test_commands(&command.left, source)
                && stmt_terminals_are_test_commands(&command.right, source)
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
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
                        query::visit_subscript_words(Some(key), source, &mut |word| {
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

    query::visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
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
                query::visit_arithmetic_words(expression, &mut |word| {
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
                query::visit_arithmetic_words(offset_ast, &mut |word| {
                    collect_status_parameter_spans_in_word(word, source, spans);
                });
            } else {
                collect_status_parameter_spans_in_word(offset_word_ast, source, spans);
            }
            match (length_ast.as_ref(), length_word_ast.as_ref()) {
                (Some(length_ast), _) => {
                    query::visit_arithmetic_words(length_ast, &mut |word| {
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
                    query::visit_arithmetic_words(offset_ast, &mut |word| {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    });
                } else {
                    collect_status_parameter_spans_in_word(offset_word_ast, source, spans);
                }

                match (length_ast.as_ref(), length_word_ast.as_ref()) {
                    (Some(length_ast), _) => {
                        query::visit_arithmetic_words(length_ast, &mut |word| {
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

fn build_redirect_facts<'a>(
    redirects: &'a [Redirect],
    source: &str,
    zsh_options: Option<&ZshOptionState>,
) -> Box<[RedirectFact<'a>]> {
    redirects
        .iter()
        .map(|redirect| RedirectFact {
            redirect,
            target_span: redirect.word_target().map(|word| word.span),
            analysis: analyze_redirect_target(redirect, source, zsh_options),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn effective_command_zsh_options(
    semantic: &SemanticModel,
    offset: usize,
    normalized: &NormalizedCommand<'_>,
) -> Option<ZshOptionState> {
    let mut options = semantic.zsh_options_at(offset).cloned();
    if normalized.has_wrapper(WrapperKind::Noglob)
        && let Some(options) = options.as_mut()
    {
        options.glob = shuck_semantic::OptionValue::Off;
    }
    options
}

fn build_word_facts_for_command<'a>(
    visit: CommandVisit<'a>,
    source: &'a str,
    semantic: &'a SemanticModel,
    command_id: CommandId,
    nested_word_command: bool,
    normalized: &NormalizedCommand<'a>,
) -> CollectedWordFacts<'a> {
    let mut collector = WordFactCollector::new(
        source,
        semantic,
        command_id,
        nested_word_command,
        normalized,
    );
    collector.collect_command(visit.command, visit.redirects);
    collector.finish()
}

struct CollectedWordFacts<'a> {
    facts: Vec<WordFact<'a>>,
    compound_assignment_value_word_spans: FxHashSet<FactSpan>,
    array_assignment_split_word_indices: Vec<usize>,
    pattern_literal_spans: Vec<Span>,
    pattern_charclass_spans: Vec<Span>,
    arithmetic: ArithmeticFactSummary,
    surface: SurfaceFragmentFacts,
}

fn extend_surface_fragment_facts(target: &mut SurfaceFragmentFacts, source: SurfaceFragmentFacts) {
    target.single_quoted.extend(source.single_quoted);
    target
        .dollar_double_quoted
        .extend(source.dollar_double_quoted);
    target.open_double_quotes.extend(source.open_double_quotes);
    target
        .suspect_closing_quotes
        .extend(source.suspect_closing_quotes);
    target.backticks.extend(source.backticks);
    target.legacy_arithmetic.extend(source.legacy_arithmetic);
    target
        .positional_parameters
        .extend(source.positional_parameters);
    target
        .positional_parameter_operator_spans
        .extend(source.positional_parameter_operator_spans);
    target
        .unicode_smart_quote_spans
        .extend(source.unicode_smart_quote_spans);
    target
        .pattern_exactly_one_extglob_spans
        .extend(source.pattern_exactly_one_extglob_spans);
    target
        .pattern_charclass_spans
        .extend(source.pattern_charclass_spans);
    target
        .nested_pattern_charclass_spans
        .extend(source.nested_pattern_charclass_spans);
    target
        .nested_parameter_expansions
        .extend(source.nested_parameter_expansions);
    target
        .indirect_expansions
        .extend(source.indirect_expansions);
    target
        .indexed_array_references
        .extend(source.indexed_array_references);
    target
        .substring_expansions
        .extend(source.substring_expansions);
    target.case_modifications.extend(source.case_modifications);
    target
        .replacement_expansions
        .extend(source.replacement_expansions);
    target.star_glob_removals.extend(source.star_glob_removals);
    target.subscript_spans.extend(source.subscript_spans);
}

struct WordFactCollector<'a> {
    source: &'a str,
    command_id: CommandId,
    nested_word_command: bool,
    surface_command_name: Option<Box<str>>,
    command_zsh_options: Option<ZshOptionState>,
    facts: Vec<WordFact<'a>>,
    array_assignment_split_word_indices: Vec<usize>,
    seen: FxHashSet<(FactSpan, WordFactContext, WordFactHostKind)>,
    compound_assignment_value_word_spans: FxHashSet<FactSpan>,
    pattern_literal_spans: Vec<Span>,
    pattern_charclass_spans: Vec<Span>,
    arithmetic: ArithmeticFactSummary,
    surface: SurfaceFragmentSink<'a>,
}

impl<'a> WordFactCollector<'a> {
    fn new(
        source: &'a str,
        semantic: &'a SemanticModel,
        command_id: CommandId,
        nested_word_command: bool,
        normalized: &NormalizedCommand<'a>,
    ) -> Self {
        Self {
            source,
            command_id,
            nested_word_command,
            surface_command_name: normalized
                .effective_or_literal_name()
                .map(str::to_owned)
                .map(String::into_boxed_str),
            command_zsh_options: effective_command_zsh_options(
                semantic,
                normalized.body_span.start.offset,
                normalized,
            ),
            facts: Vec::new(),
            array_assignment_split_word_indices: Vec::new(),
            seen: FxHashSet::default(),
            compound_assignment_value_word_spans: FxHashSet::default(),
            pattern_literal_spans: Vec::new(),
            pattern_charclass_spans: Vec::new(),
            arithmetic: ArithmeticFactSummary::default(),
            surface: SurfaceFragmentSink::new(source),
        }
    }

    fn finish(self) -> CollectedWordFacts<'a> {
        CollectedWordFacts {
            facts: self.facts,
            compound_assignment_value_word_spans: self.compound_assignment_value_word_spans,
            array_assignment_split_word_indices: self.array_assignment_split_word_indices,
            pattern_literal_spans: self.pattern_literal_spans,
            pattern_charclass_spans: self.pattern_charclass_spans,
            arithmetic: self.arithmetic,
            surface: self.surface.finish(),
        }
    }

    fn collect_command(&mut self, command: &'a Command, redirects: &'a [Redirect]) {
        self.collect_command_name_context_word(command);
        self.collect_argument_context_words(command);
        self.collect_expansion_assignment_value_words(command);
        let surface_command_name = self.surface_command_name.clone();
        let surface_context =
            SurfaceScanContext::new(surface_command_name.as_deref(), self.nested_word_command);

        if let Command::Compound(command) = command {
            match command {
                CompoundCommand::For(command) => {
                    if let Some(words) = &command.words {
                        self.surface.collect_words(words, surface_context);
                        for word in words {
                            self.push_word(
                                word,
                                WordFactContext::Expansion(ExpansionContext::ForList),
                                WordFactHostKind::Direct,
                            );
                        }
                    }
                }
                CompoundCommand::Repeat(command) => {
                    self.surface.collect_word(&command.count, surface_context);
                    self.push_word(
                        &command.count,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        WordFactHostKind::Direct,
                    );
                }
                CompoundCommand::Foreach(command) => {
                    self.surface.collect_words(&command.words, surface_context);
                    for word in &command.words {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::ForList),
                            WordFactHostKind::Direct,
                        );
                    }
                }
                CompoundCommand::Select(command) => {
                    self.surface.collect_words(&command.words, surface_context);
                    for word in &command.words {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::SelectList),
                            WordFactHostKind::Direct,
                        );
                    }
                }
                CompoundCommand::Case(command) => {
                    self.surface.collect_word(&command.word, surface_context);
                    self.push_word(
                        &command.word,
                        WordFactContext::CaseSubject,
                        WordFactHostKind::Direct,
                    );
                    for case in &command.cases {
                        for pattern in &case.patterns {
                            self.surface.collect_pattern(
                                pattern,
                                surface_context.with_pattern_charclass_scan(),
                            );
                            self.collect_pattern_context_words(
                                pattern,
                                WordFactContext::Expansion(ExpansionContext::CasePattern),
                                WordFactHostKind::Direct,
                            );
                        }
                    }
                }
                CompoundCommand::Conditional(command) => {
                    self.collect_conditional_expansion_words(
                        &command.expression,
                        SurfaceScanContext::new(None, self.nested_word_command),
                    );
                }
                CompoundCommand::Arithmetic(command) => {
                    if let Some(expression) = &command.expr_ast {
                        collect_arithmetic_command_spans(
                            expression,
                            self.source,
                            &mut self.arithmetic.dollar_in_arithmetic_spans,
                            &mut self.arithmetic.arithmetic_command_substitution_spans,
                        );
                    }
                }
                CompoundCommand::ArithmeticFor(command) => {
                    for expression in [
                        command.init_ast.as_ref(),
                        command.condition_ast.as_ref(),
                        command.step_ast.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        collect_arithmetic_command_spans(
                            expression,
                            self.source,
                            &mut self.arithmetic.dollar_in_arithmetic_spans,
                            &mut self.arithmetic.arithmetic_command_substitution_spans,
                        );
                    }
                }
                CompoundCommand::If(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::Subshell(_)
                | CompoundCommand::BraceGroup(_)
                | CompoundCommand::Always(_)
                | CompoundCommand::Coproc(_)
                | CompoundCommand::Time(_) => {}
            }
        }

        self.surface.collect_redirects(
            redirects,
            SurfaceScanContext::new(None, self.nested_word_command),
        );
        for redirect in redirects {
            let Some(context) = ExpansionContext::from_redirect_kind(redirect.kind) else {
                continue;
            };
            let word = redirect
                .word_target()
                .expect("expected non-heredoc redirect target");
            self.push_word(
                word,
                WordFactContext::Expansion(context),
                WordFactHostKind::Direct,
            );
        }

        if let Some(action) = trap_action_word(command, self.source) {
            self.push_word(
                action,
                WordFactContext::Expansion(ExpansionContext::TrapAction),
                WordFactHostKind::Direct,
            );
        }
    }

    fn collect_command_name_context_word(&mut self, command: &'a Command) {
        let surface_command_name = self.surface_command_name.clone();
        let surface_context =
            SurfaceScanContext::new(surface_command_name.as_deref(), self.nested_word_command);
        match command {
            Command::Simple(command) => {
                self.surface.collect_word(&command.name, surface_context);
                if static_word_text(&command.name, self.source).is_none() {
                    self.push_word(
                        &command.name,
                        WordFactContext::Expansion(ExpansionContext::CommandName),
                        WordFactHostKind::Direct,
                    );
                }
            }
            Command::Function(function) => {
                for entry in &function.header.entries {
                    self.surface.collect_word(&entry.word, surface_context);
                    if static_word_text(&entry.word, self.source).is_none() {
                        self.push_word(
                            &entry.word,
                            WordFactContext::Expansion(ExpansionContext::CommandName),
                            WordFactHostKind::Direct,
                        );
                    }
                }
            }
            Command::Builtin(_)
            | Command::Decl(_)
            | Command::Binary(_)
            | Command::Compound(_)
            | Command::AnonymousFunction(_) => {}
        }
    }

    fn collect_argument_context_words(&mut self, command: &'a Command) {
        match command {
            Command::Simple(command) => {
                let surface_command_name = self.surface_command_name.clone();
                let surface_context = SurfaceScanContext::new(
                    surface_command_name.as_deref(),
                    self.nested_word_command,
                );
                let trap_command =
                    static_word_text(&command.name, self.source).as_deref() == Some("trap");
                let variable_set_operand =
                    surface::simple_command_variable_set_operand(command, self.source);
                let mut saw_open_double_quote = false;
                if surface_command_name.as_deref() == Some("unset") {
                    for word in &command.args {
                        self.surface.record_unset_array_target_word(word);
                    }
                }
                if matches!(surface_command_name.as_deref(), Some("echo" | "printf")) {
                    self.surface
                        .collect_split_suspect_closing_quote_fragment_in_words(&command.args);
                }
                for word in &command.args {
                    let base_surface_word_context = if variable_set_operand
                        .is_some_and(|operand| std::ptr::eq(word, operand))
                    {
                        surface_context.variable_set_operand()
                    } else {
                        surface_context
                    };
                    let surface_word_context = if saw_open_double_quote
                        && !surface::word_has_reopened_double_quote_window(
                            word,
                            self.source,
                            surface_command_name.as_deref(),
                        ) {
                        base_surface_word_context.without_open_double_quote_scan()
                    } else {
                        base_surface_word_context
                    };
                    saw_open_double_quote |= self.surface.collect_word(word, surface_word_context);
                    if !trap_command {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                }
            }
            Command::Builtin(command) => match command {
                BuiltinCommand::Break(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.depth {
                        self.surface.collect_word(word, surface_context);
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.surface
                        .collect_words(&command.extra_args, surface_context);
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
                BuiltinCommand::Continue(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.depth {
                        self.surface.collect_word(word, surface_context);
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.surface
                        .collect_words(&command.extra_args, surface_context);
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
                BuiltinCommand::Return(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.code {
                        self.surface.collect_word(word, surface_context);
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.surface
                        .collect_words(&command.extra_args, surface_context);
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
                BuiltinCommand::Exit(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.code {
                        self.surface.collect_word(word, surface_context);
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.surface
                        .collect_words(&command.extra_args, surface_context);
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
            },
            Command::Decl(command) => {
                let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) => {
                            self.surface.collect_word(word, surface_context);
                        }
                        DeclOperand::Dynamic(word) => {
                            self.surface.collect_word(word, surface_context);
                            self.push_word(
                                word,
                                WordFactContext::Expansion(ExpansionContext::CommandArgument),
                                WordFactHostKind::Direct,
                            );
                        }
                        DeclOperand::Name(_) | DeclOperand::Assignment(_) => {}
                    }
                }
            }
            Command::Binary(_) | Command::Compound(_) | Command::Function(_) => {}
            Command::AnonymousFunction(function) => {
                self.surface.collect_words(
                    &function.args,
                    SurfaceScanContext::new(None, self.nested_word_command),
                );
                self.collect_words_with_context(
                    &function.args,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                );
            }
        }
    }

    fn collect_expansion_assignment_value_words(&mut self, command: &'a Command) {
        for assignment in query::command_assignments(command) {
            self.collect_expansion_assignment_words(
                assignment,
                WordFactContext::Expansion(ExpansionContext::AssignmentValue),
            );
        }

        for operand in query::declaration_operands(command) {
            match operand {
                DeclOperand::Name(reference) => {
                    self.surface.record_var_ref_subscript(reference);
                    query::visit_var_ref_subscript_words_with_source(
                        reference,
                        self.source,
                        &mut |word| {
                            self.surface.collect_word(
                                word,
                                SurfaceScanContext::new(None, self.nested_word_command),
                            );
                            self.push_owned_word(
                                word.clone(),
                                WordFactContext::Expansion(
                                    ExpansionContext::DeclarationAssignmentValue,
                                ),
                                WordFactHostKind::DeclarationNameSubscript,
                            );
                        },
                    );
                }
                DeclOperand::Assignment(assignment) => {
                    self.collect_expansion_assignment_words(
                        assignment,
                        WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue),
                    );
                }
                DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => {}
            }
        }
    }

    fn collect_expansion_assignment_words(
        &mut self,
        assignment: &'a Assignment,
        context: WordFactContext,
    ) {
        let surface_context = SurfaceScanContext::new(None, self.nested_word_command)
            .with_assignment_target(assignment.target.name.as_str());
        self.surface.record_var_ref_subscript(&assignment.target);
        query::visit_var_ref_subscript_words_with_source(
            &assignment.target,
            self.source,
            &mut |word| {
                self.surface.collect_word(word, surface_context);
                self.push_owned_word(
                    word.clone(),
                    context,
                    WordFactHostKind::AssignmentTargetSubscript,
                );
            },
        );

        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.surface.collect_word(word, surface_context);
                self.push_word(word, context, WordFactHostKind::Direct);
            }
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => {
                            self.surface.collect_word(word, surface_context);
                            self.compound_assignment_value_word_spans
                                .insert(FactSpan::new(word.span));
                            if let Some(index) =
                                self.push_word(word, context, WordFactHostKind::Direct)
                            {
                                self.array_assignment_split_word_indices.push(index);
                            }
                        }
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            self.surface.record_subscript(Some(key));
                            query::visit_subscript_words(Some(key), self.source, &mut |word| {
                                self.surface.collect_word(word, surface_context);
                                self.push_owned_word(
                                    word.clone(),
                                    context,
                                    WordFactHostKind::ArrayKeySubscript,
                                );
                            });
                            self.surface.collect_word(value, surface_context);
                            self.compound_assignment_value_word_spans
                                .insert(FactSpan::new(value.span));
                            self.push_word(value, context, WordFactHostKind::Direct);
                        }
                    }
                }
            }
        }
    }

    fn collect_words_with_context(&mut self, words: &'a [Word], context: WordFactContext) {
        for word in words {
            self.push_word(word, context, WordFactHostKind::Direct);
        }
    }

    fn collect_pattern_context_words(
        &mut self,
        pattern: &Pattern,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        let is_case_pattern = matches!(
            context,
            WordFactContext::Expansion(ExpansionContext::CasePattern)
        );
        if is_case_pattern && !pattern_contains_word_or_group(pattern) {
            self.pattern_literal_spans.push(pattern.span);
        }
        for (part, _span) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_pattern_context_words(pattern, context, host_kind);
                    }
                }
                PatternPart::Word(word) => {
                    self.push_owned_word(word.clone(), context, host_kind);
                }
                PatternPart::Literal(_) | PatternPart::CharClass(_) if is_case_pattern => {}
                PatternPart::AnyString | PatternPart::AnyChar => {}
                PatternPart::Literal(_) | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn collect_zsh_qualified_glob_context_words(
        &mut self,
        glob: &ZshQualifiedGlob,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        for segment in &glob.segments {
            if let ZshGlobSegment::Pattern(pattern) = segment {
                self.collect_pattern_context_words(pattern, context, host_kind);
            }
        }
    }

    fn collect_conditional_expansion_words(
        &mut self,
        expression: &'a ConditionalExpr,
        surface_context: SurfaceScanContext<'_>,
    ) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.collect_conditional_expansion_words(&expr.left, surface_context);
                self.collect_conditional_expansion_words(&expr.right, surface_context);
            }
            ConditionalExpr::Unary(expr) => self.collect_conditional_expansion_words(
                &expr.expr,
                if expr.op == ConditionalUnaryOp::VariableSet {
                    surface_context.variable_set_operand()
                } else {
                    surface_context
                },
            ),
            ConditionalExpr::Parenthesized(expr) => {
                self.collect_conditional_expansion_words(&expr.expr, surface_context)
            }
            ConditionalExpr::Word(word) => {
                self.surface.collect_word(word, surface_context);
                self.push_word(
                    word,
                    WordFactContext::Expansion(ExpansionContext::StringTestOperand),
                    WordFactHostKind::Direct,
                );
            }
            ConditionalExpr::Regex(word) => {
                self.surface.collect_word(word, surface_context);
                self.push_word(
                    word,
                    WordFactContext::Expansion(ExpansionContext::RegexOperand),
                    WordFactHostKind::Direct,
                );
            }
            ConditionalExpr::Pattern(pattern) => {
                let pattern_context = surface_context.with_pattern_charclass_scan();
                self.surface.collect_pattern(pattern, pattern_context);
                self.collect_pattern_context_words(
                    pattern,
                    WordFactContext::Expansion(ExpansionContext::ConditionalPattern),
                    WordFactHostKind::Direct,
                );
            }
            ConditionalExpr::VarRef(reference) => {
                self.surface.record_var_ref_subscript(reference);
                query::visit_var_ref_subscript_words_with_source(
                    reference,
                    self.source,
                    &mut |word| {
                        self.surface.collect_word(word, surface_context);
                        self.push_owned_word(
                            word.clone(),
                            WordFactContext::Expansion(
                                ExpansionContext::ConditionalVarRefSubscript,
                            ),
                            WordFactHostKind::ConditionalVarRefSubscript,
                        );
                    },
                );
            }
        }
    }

    fn collect_word_parameter_patterns(
        &mut self,
        parts: &[WordPartNode],
        host_kind: WordFactHostKind,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::ZshQualifiedGlob(glob) => self.collect_zsh_qualified_glob_context_words(
                    glob,
                    WordFactContext::Expansion(ExpansionContext::ParameterPattern),
                    host_kind,
                ),
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_word_parameter_patterns(parts, host_kind)
                }
                WordPart::Parameter(parameter) => {
                    if let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                        operator,
                        ..
                    }) = &parameter.syntax
                    {
                        self.collect_parameter_operator_patterns(operator, host_kind);
                    }
                }
                WordPart::ParameterExpansion { operator, .. } => {
                    self.collect_parameter_operator_patterns(operator, host_kind)
                }
                WordPart::IndirectExpansion {
                    operator: Some(operator),
                    ..
                } => self.collect_parameter_operator_patterns(operator, host_kind),
                WordPart::Literal(_)
                | WordPart::SingleQuoted { .. }
                | WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { operator: None, .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => {}
            }
        }
    }

    fn collect_parameter_operator_patterns(
        &mut self,
        operator: &ParameterOp,
        host_kind: WordFactHostKind,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern }
            | ParameterOp::ReplaceFirst { pattern, .. }
            | ParameterOp::ReplaceAll { pattern, .. } => self.collect_pattern_context_words(
                pattern,
                WordFactContext::Expansion(ExpansionContext::ParameterPattern),
                host_kind,
            ),
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

    fn push_word(
        &mut self,
        word: &'a Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) -> Option<usize> {
        self.push_cow_word(Cow::Borrowed(word), context, host_kind)
    }

    fn push_owned_word(
        &mut self,
        word: Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) -> Option<usize> {
        self.push_cow_word(Cow::Owned(word), context, host_kind)
    }

    fn push_cow_word(
        &mut self,
        word: Cow<'a, Word>,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) -> Option<usize> {
        let word_ref = word.as_ref();
        let key = FactSpan::new(word_ref.span);
        if !self.seen.insert((key, context, host_kind)) {
            return None;
        }

        self.collect_word_parameter_patterns(&word_ref.parts, host_kind);
        self.collect_arithmetic_summary(word_ref, context, host_kind);

        let zsh_options = self.command_zsh_options.clone();
        let analysis = analyze_word(word_ref, self.source, zsh_options.as_ref());
        let runtime_literal = match context {
            WordFactContext::Expansion(context) => {
                analyze_literal_runtime(word_ref, self.source, context, zsh_options.as_ref())
            }
            WordFactContext::CaseSubject | WordFactContext::ArithmeticCommand => {
                RuntimeLiteralAnalysis::default()
            }
        };
        let operand_class = match context {
            WordFactContext::Expansion(context) if word_context_supports_operand_class(context) => {
                Some(
                    if analysis.literalness == WordLiteralness::Expanded
                        || runtime_literal.is_runtime_sensitive()
                    {
                        TestOperandClass::RuntimeSensitive
                    } else {
                        TestOperandClass::FixedLiteral
                    },
                )
            }
            WordFactContext::Expansion(_)
            | WordFactContext::CaseSubject
            | WordFactContext::ArithmeticCommand => None,
        };
        let index = self.facts.len();
        self.facts.push(WordFact {
            key,
            static_text: static_word_text(word_ref, self.source).map(String::into_boxed_str),
            has_literal_affixes: word_has_literal_affixes(word_ref),
            contains_shell_quoting_literals: word_contains_shell_quoting_literals(
                word_ref,
                self.source,
            ),
            scalar_expansion_spans: span::scalar_expansion_part_spans(word_ref, self.source)
                .into_boxed_slice(),
            unquoted_scalar_expansion_spans: span::unquoted_scalar_expansion_part_spans(
                word_ref,
                self.source,
            )
            .into_boxed_slice(),
            array_expansion_spans: span::array_expansion_part_spans(word_ref, self.source)
                .into_boxed_slice(),
            all_elements_array_expansion_spans: span::all_elements_array_expansion_part_spans(
                word_ref,
                self.source,
            )
            .into_boxed_slice(),
            unquoted_all_elements_array_expansion_spans:
                span::unquoted_all_elements_array_expansion_part_spans(word_ref, self.source)
                    .into_boxed_slice(),
            unquoted_array_expansion_spans: span::unquoted_array_expansion_part_spans(
                word_ref,
                self.source,
            )
            .into_boxed_slice(),
            command_substitution_spans: span::command_substitution_part_spans_in_source(
                word_ref,
                self.source,
            )
            .into_boxed_slice(),
            unquoted_command_substitution_spans:
                span::unquoted_command_substitution_part_spans_in_source(word_ref, self.source)
                    .into_boxed_slice(),
            double_quoted_expansion_spans: double_quoted_expansion_part_spans(word_ref)
                .into_boxed_slice(),
            unquoted_literal_between_double_quoted_segments_spans:
                build_unquoted_literal_between_double_quoted_segments_spans(word_ref, self.source)
                    .into_boxed_slice(),
            word,
            command_id: self.command_id,
            nested_word_command: self.nested_word_command,
            context,
            host_kind,
            zsh_options,
            analysis,
            runtime_literal,
            operand_class,
        });
        Some(index)
    }

    fn collect_arithmetic_summary(
        &mut self,
        word: &Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        if matches!(
            host_kind,
            WordFactHostKind::AssignmentTargetSubscript
                | WordFactHostKind::DeclarationNameSubscript
                | WordFactHostKind::ArrayKeySubscript
                | WordFactHostKind::ConditionalVarRefSubscript
        ) {
            self.arithmetic
                .array_index_arithmetic_spans
                .extend(span::arithmetic_expansion_part_spans(word));
        }
        if host_kind == WordFactHostKind::Direct
            && matches!(
                context,
                WordFactContext::Expansion(ExpansionContext::AssignmentValue)
                    | WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue)
            )
        {
            self.arithmetic
                .arithmetic_score_line_spans
                .extend(span::parenthesized_arithmetic_expansion_part_spans(word));
        }

        collect_arithmetic_expansion_spans_from_parts(
            &word.parts,
            self.source,
            host_kind == WordFactHostKind::Direct,
            &mut self.arithmetic.dollar_in_arithmetic_spans,
            &mut self.arithmetic.arithmetic_command_substitution_spans,
        );

        if host_kind == WordFactHostKind::Direct
            && word_needs_wrapped_arithmetic_fallback(word, self.source)
        {
            collect_wrapped_arithmetic_spans_in_word(
                word,
                self.source,
                &mut self.arithmetic.dollar_in_arithmetic_spans,
                &mut self.arithmetic.arithmetic_command_substitution_spans,
            );
        }
    }
}

fn pattern_contains_word_or_group(pattern: &Pattern) -> bool {
    pattern.parts.iter().any(|part| match &part.kind {
        PatternPart::Word(_) => true,
        PatternPart::Group { patterns, .. } => patterns.iter().any(pattern_contains_word_or_group),
        PatternPart::Literal(_)
        | PatternPart::AnyString
        | PatternPart::AnyChar
        | PatternPart::CharClass(_) => false,
    })
}

#[derive(Debug, Clone)]
struct StaticCasePatternMatcher {
    tokens: Vec<CasePatternToken>,
    min_len: usize,
    max_len: Option<usize>,
    literal_prefix: Box<str>,
    literal_suffix: Box<str>,
    literal_symbols: Box<[char]>,
    start_states: Box<[usize]>,
}

#[derive(Debug, Clone)]
struct StaticCasePatternSummary {
    min_len: usize,
    max_len: Option<usize>,
    literal_prefix: Box<str>,
    literal_suffix: Box<str>,
    literal_symbols: Box<[char]>,
    start_states: Box<[usize]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasePatternToken {
    Literal(char),
    AnyChar,
    AnyString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasePatternSymbol {
    Literal(char),
    Other,
}

#[derive(Debug, Clone)]
struct ReachableCasePattern {
    span: Span,
    matcher: StaticCasePatternMatcher,
}

impl StaticCasePatternMatcher {
    fn from_pattern(pattern: &Pattern, source: &str) -> Option<Self> {
        ensure_case_pattern_is_statically_analyzable(pattern, source)?;

        let mut tokens = Vec::new();
        collect_static_case_pattern_tokens(pattern.span.slice(source), &mut tokens)?;
        let StaticCasePatternSummary {
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
        } = summarize_static_case_pattern_tokens(&tokens);
        Some(Self {
            tokens,
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
        })
    }

    fn subsumes(&self, other: &Self) -> bool {
        if !self.could_subsume(other) {
            return false;
        }

        let symbols = merged_case_pattern_symbols(
            self.literal_symbols.as_ref(),
            other.literal_symbols.as_ref(),
        );

        let start = (self.start_states.to_vec(), other.start_states.to_vec());
        let mut seen = FxHashSet::default();
        let mut worklist = vec![start.clone()];
        seen.insert(start);

        while let Some((left, right)) = worklist.pop() {
            if other.is_accepting(&right) && !self.is_accepting(&left) {
                return false;
            }

            for symbol in symbols.iter().copied() {
                let next_right = other.advance(&right, symbol);
                if next_right.is_empty() {
                    continue;
                }

                let next_left = self.advance(&left, symbol);
                if seen.insert((next_left.clone(), next_right.clone())) {
                    worklist.push((next_left, next_right));
                }
            }
        }

        true
    }

    fn could_subsume(&self, other: &Self) -> bool {
        if self.min_len > other.min_len {
            return false;
        }
        match (self.max_len, other.max_len) {
            (Some(_), None) => return false,
            (Some(self_max), Some(other_max)) if self_max < other_max => return false,
            (Some(_), Some(_)) | (None, Some(_)) | (None, None) => {}
        }
        if !self.literal_prefix.is_empty()
            && !other
                .literal_prefix
                .starts_with(self.literal_prefix.as_ref())
        {
            return false;
        }
        if !self.literal_suffix.is_empty()
            && !other.literal_suffix.ends_with(self.literal_suffix.as_ref())
        {
            return false;
        }

        true
    }

    fn advance(&self, states: &[usize], symbol: CasePatternSymbol) -> Vec<usize> {
        let mut next = Vec::new();

        for &state in states {
            let Some(token) = self.tokens.get(state) else {
                continue;
            };

            match token {
                CasePatternToken::Literal(expected) if matches!(symbol, CasePatternSymbol::Literal(actual) if actual == *expected) =>
                {
                    next.push(state + 1);
                }
                CasePatternToken::AnyChar => next.push(state + 1),
                CasePatternToken::AnyString => next.push(state),
                CasePatternToken::Literal(_) => {}
            }
        }

        if next.is_empty() {
            return Vec::new();
        }

        self.epsilon_closure(next)
    }

    fn epsilon_closure(&self, seeds: impl IntoIterator<Item = usize>) -> Vec<usize> {
        case_pattern_epsilon_closure(&self.tokens, seeds)
    }

    fn is_accepting(&self, states: &[usize]) -> bool {
        states.contains(&self.tokens.len())
    }
}

fn summarize_static_case_pattern_tokens(tokens: &[CasePatternToken]) -> StaticCasePatternSummary {
    let mut min_len = 0usize;
    let mut max_len = Some(0usize);
    let mut literal_prefix = String::new();
    let mut saw_wildcard = false;
    let mut literal_suffix_reversed = String::new();
    let mut saw_suffix_wildcard = false;
    let mut literal_symbols = Vec::new();

    for token in tokens {
        match token {
            CasePatternToken::Literal(ch) => {
                min_len += 1;
                if let Some(max_len) = &mut max_len {
                    *max_len += 1;
                }
                if !saw_wildcard {
                    literal_prefix.push(*ch);
                }
                literal_symbols.push(*ch);
            }
            CasePatternToken::AnyChar => {
                min_len += 1;
                if let Some(max_len) = &mut max_len {
                    *max_len += 1;
                }
                saw_wildcard = true;
            }
            CasePatternToken::AnyString => {
                max_len = None;
                saw_wildcard = true;
            }
        }
    }

    for token in tokens.iter().rev() {
        match token {
            CasePatternToken::Literal(ch) if !saw_suffix_wildcard => {
                literal_suffix_reversed.push(*ch);
            }
            CasePatternToken::Literal(_)
            | CasePatternToken::AnyChar
            | CasePatternToken::AnyString => {
                saw_suffix_wildcard = true;
            }
        }
    }

    literal_symbols.sort_unstable();
    literal_symbols.dedup();

    StaticCasePatternSummary {
        min_len,
        max_len,
        literal_prefix: literal_prefix.into_boxed_str(),
        literal_suffix: literal_suffix_reversed
            .chars()
            .rev()
            .collect::<String>()
            .into_boxed_str(),
        literal_symbols: literal_symbols.into_boxed_slice(),
        start_states: case_pattern_epsilon_closure(tokens, [0]).into_boxed_slice(),
    }
}

fn case_pattern_epsilon_closure(
    tokens: &[CasePatternToken],
    seeds: impl IntoIterator<Item = usize>,
) -> Vec<usize> {
    let mut seen = vec![false; tokens.len() + 1];
    let mut stack = Vec::new();

    for state in seeds {
        if state <= tokens.len() && !seen[state] {
            seen[state] = true;
            stack.push(state);
        }
    }

    while let Some(state) = stack.pop() {
        if matches!(tokens.get(state), Some(CasePatternToken::AnyString)) {
            let next = state + 1;
            if !seen[next] {
                seen[next] = true;
                stack.push(next);
            }
        }
    }

    seen.into_iter()
        .enumerate()
        .filter_map(|(index, present)| present.then_some(index))
        .collect()
}

fn merged_case_pattern_symbols(left: &[char], right: &[char]) -> Vec<CasePatternSymbol> {
    let mut symbols = Vec::with_capacity(left.len() + right.len() + 1);
    let mut left_index = 0usize;
    let mut right_index = 0usize;

    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => {
                symbols.push(CasePatternSymbol::Literal(left[left_index]));
                left_index += 1;
            }
            std::cmp::Ordering::Greater => {
                symbols.push(CasePatternSymbol::Literal(right[right_index]));
                right_index += 1;
            }
            std::cmp::Ordering::Equal => {
                symbols.push(CasePatternSymbol::Literal(left[left_index]));
                left_index += 1;
                right_index += 1;
            }
        }
    }

    for &symbol in &left[left_index..] {
        symbols.push(CasePatternSymbol::Literal(symbol));
    }
    for &symbol in &right[right_index..] {
        symbols.push(CasePatternSymbol::Literal(symbol));
    }
    symbols.push(CasePatternSymbol::Other);

    symbols
}

fn ensure_case_pattern_is_statically_analyzable(pattern: &Pattern, source: &str) -> Option<()> {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => {}
            PatternPart::Word(word) => {
                static_word_text(word, source)?;
            }
            PatternPart::Group { .. } | PatternPart::CharClass(_) => return None,
        }
    }

    Some(())
}

fn collect_static_case_pattern_tokens(
    pattern_syntax: &str,
    out: &mut Vec<CasePatternToken>,
) -> Option<()> {
    let mut chars = pattern_syntax.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(escaped) => push_case_pattern_literal_tokens_char(escaped, out),
                None => push_case_pattern_literal_tokens_char('\\', out),
            },
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    push_case_pattern_literal_tokens_char(quoted, out);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('\n') => {}
                            Some(escaped @ ('$' | '`' | '"' | '\\')) => {
                                push_case_pattern_literal_tokens_char(escaped, out);
                            }
                            Some(other) => {
                                push_case_pattern_literal_tokens_char('\\', out);
                                push_case_pattern_literal_tokens_char(other, out);
                            }
                            None => push_case_pattern_literal_tokens_char('\\', out),
                        },
                        _ => push_case_pattern_literal_tokens_char(quoted, out),
                    }
                }
            }
            '[' => return None,
            '?' => {
                if chars.peek() == Some(&'(') {
                    return None;
                }
                push_case_pattern_token(out, CasePatternToken::AnyChar);
            }
            '*' => {
                if chars.peek() == Some(&'(') {
                    return None;
                }
                push_case_pattern_token(out, CasePatternToken::AnyString);
            }
            '+' | '@' | '!' if chars.peek() == Some(&'(') => return None,
            '$' | '`' => return None,
            other => push_case_pattern_literal_tokens_char(other, out),
        }
    }
    Some(())
}

fn push_case_pattern_literal_tokens_char(ch: char, out: &mut Vec<CasePatternToken>) {
    out.push(CasePatternToken::Literal(ch));
}

fn push_case_pattern_token(out: &mut Vec<CasePatternToken>, token: CasePatternToken) {
    if matches!(token, CasePatternToken::AnyString)
        && matches!(out.last(), Some(CasePatternToken::AnyString))
    {
        return;
    }

    out.push(token);
}

fn build_case_pattern_shadow_facts(
    commands: &[CommandFact<'_>],
    source: &str,
) -> Vec<CasePatternShadowFact> {
    let mut shadows = Vec::new();

    for fact in commands {
        let Command::Compound(CompoundCommand::Case(command)) = fact.command() else {
            continue;
        };

        let mut prior_arm_patterns = Vec::<ReachableCasePattern>::new();
        let mut fallthrough_arm_patterns = Vec::<ReachableCasePattern>::new();
        let mut spent_shadowing_patterns = FxHashSet::default();

        for item in &command.cases {
            let mut same_item_patterns = Vec::<ReachableCasePattern>::new();

            for pattern in &item.patterns {
                let Some(matcher) = StaticCasePatternMatcher::from_pattern(pattern, source) else {
                    continue;
                };

                for previous in prior_arm_patterns
                    .iter()
                    .chain(fallthrough_arm_patterns.iter())
                    .chain(same_item_patterns.iter())
                {
                    if spent_shadowing_patterns.contains(&FactSpan::new(previous.span)) {
                        continue;
                    }

                    if previous.matcher.subsumes(&matcher) {
                        shadows.push(CasePatternShadowFact {
                            shadowing_pattern_span: previous.span,
                            shadowed_pattern_span: pattern.span,
                        });
                        spent_shadowing_patterns.insert(FactSpan::new(previous.span));
                        break;
                    }
                }

                same_item_patterns.push(ReachableCasePattern {
                    span: pattern.span,
                    matcher,
                });
            }

            match item.terminator {
                CaseTerminator::Break => {
                    prior_arm_patterns.append(&mut fallthrough_arm_patterns);
                    prior_arm_patterns.extend(same_item_patterns);
                }
                CaseTerminator::FallThrough => {
                    fallthrough_arm_patterns.extend(same_item_patterns);
                }
                CaseTerminator::Continue | CaseTerminator::ContinueMatching => {
                    fallthrough_arm_patterns.clear();
                }
            }
        }
    }

    shadows
}

#[derive(Debug, Clone)]
struct ParsedGetoptsCommand {
    declared_options: Vec<GetoptsOptionSpec>,
    target_name: Name,
}

#[derive(Debug, Clone)]
struct GetoptsCaseMatch {
    case_span: Span,
    handled_case_labels: Vec<GetoptsCaseLabelFact>,
    invalid_case_pattern_spans: Vec<Span>,
    has_fallback_pattern: bool,
    has_unknown_coverage: bool,
}

fn build_getopts_case_fact_for_while(
    command: &WhileCommand,
    source: &str,
) -> Option<GetoptsCaseFact> {
    let parsed = parse_getopts_command_from_condition(&command.condition, source)?;
    let GetoptsCaseMatch {
        case_span,
        handled_case_labels,
        invalid_case_pattern_spans,
        has_fallback_pattern,
        has_unknown_coverage,
    } = first_getopts_case_match(&command.body, parsed.target_name.as_str(), source)?;

    let handled = handled_case_labels
        .iter()
        .map(|label| label.label)
        .collect::<FxHashSet<_>>();
    let declared = parsed
        .declared_options
        .iter()
        .map(|option| option.option)
        .collect::<FxHashSet<_>>();
    let unexpected_case_labels = handled_case_labels
        .iter()
        .copied()
        .filter(|label| !declared.contains(&label.label()))
        .filter(|label| !matches!(label.label(), '?' | ':'))
        .collect::<Vec<_>>();
    let missing_options = if has_fallback_pattern || has_unknown_coverage {
        Vec::new()
    } else {
        parsed
            .declared_options
            .iter()
            .copied()
            .filter(|option| !handled.contains(&option.option))
            .collect::<Vec<_>>()
    };

    Some(GetoptsCaseFact {
        case_span,
        declared_options: parsed.declared_options.into_boxed_slice(),
        handled_case_labels: handled_case_labels.into_boxed_slice(),
        unexpected_case_labels: unexpected_case_labels.into_boxed_slice(),
        invalid_case_pattern_spans: invalid_case_pattern_spans.into_boxed_slice(),
        has_fallback_pattern,
        missing_options: missing_options.into_boxed_slice(),
    })
}

fn parse_getopts_command_from_condition(
    condition: &StmtSeq,
    source: &str,
) -> Option<ParsedGetoptsCommand> {
    let stmt = condition.last()?;
    let normalized = command::normalize_command(&stmt.command, source);
    if !normalized.effective_name_is("getopts") {
        return None;
    }

    let args = normalized.body_args();
    let option_string = static_word_text(args.first()?, source)?;
    let target_text = static_word_text(args.get(1)?, source)?;
    if !is_shell_variable_name(&target_text) {
        return None;
    }

    let declared_options = parse_getopts_option_specs(&option_string);
    Some(ParsedGetoptsCommand {
        declared_options,
        target_name: Name::from(target_text),
    })
}

fn parse_getopts_option_specs(option_string: &str) -> Vec<GetoptsOptionSpec> {
    let mut specs = Vec::new();
    let mut seen = FxHashSet::default();
    let mut chars = option_string.chars().peekable();

    if chars.peek() == Some(&':') {
        chars.next();
    }

    while let Some(option) = chars.next() {
        if option == ':' {
            continue;
        }

        let requires_argument = chars.peek() == Some(&':');
        if requires_argument {
            chars.next();
        }

        if seen.insert(option) {
            specs.push(GetoptsOptionSpec {
                option,
                requires_argument,
            });
        }
    }

    specs
}

fn first_getopts_case_match(
    body: &StmtSeq,
    target_name: &str,
    source: &str,
) -> Option<GetoptsCaseMatch> {
    first_getopts_case_match_in_commands(body, target_name, source)
}

fn first_getopts_case_match_in_commands(
    commands: &StmtSeq,
    target_name: &str,
    source: &str,
) -> Option<GetoptsCaseMatch> {
    commands
        .iter()
        .find_map(|stmt| first_getopts_case_match_in_command(&stmt.command, target_name, source))
}

fn first_getopts_case_match_in_command(
    command: &Command,
    target_name: &str,
    source: &str,
) -> Option<GetoptsCaseMatch> {
    match command {
        Command::Binary(command) => first_getopts_case_match_in_command(
            &command.left.command,
            target_name,
            source,
        )
        .or_else(|| {
            matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll).then(|| {
                first_getopts_case_match_in_command(&command.right.command, target_name, source)
            })?
        }),
        Command::Compound(CompoundCommand::Case(command))
            if case_subject_variable_name(&command.word) == Some(target_name) =>
        {
            Some(build_getopts_case_match(command, source))
        }
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            first_getopts_case_match_in_commands(commands, target_name, source)
        }
        // Helper definitions are not part of the executable getopts dispatch path.
        Command::Function(_) | Command::AnonymousFunction(_) => None,
        Command::Compound(_) | Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => None,
    }
}

fn build_getopts_case_match(command: &CaseCommand, source: &str) -> GetoptsCaseMatch {
    let mut has_fallback_pattern = false;
    let mut has_unknown_coverage = false;
    let mut invalid_case_pattern_spans = Vec::new();
    let labels = command
        .cases
        .iter()
        .flat_map(|item| item.patterns.iter())
        .filter_map(
            |pattern| match classify_getopts_case_pattern(pattern, source) {
                GetoptsCasePatternKind::Fallback => {
                    has_fallback_pattern = true;
                    None
                }
                GetoptsCasePatternKind::SingleLabel(label) => Some(label),
                GetoptsCasePatternKind::InvalidStaticPattern(span) => {
                    invalid_case_pattern_spans.push(span);
                    None
                }
                GetoptsCasePatternKind::UnknownCoverage => {
                    has_unknown_coverage = true;
                    None
                }
            },
        )
        .collect::<Vec<_>>();
    GetoptsCaseMatch {
        case_span: command.span,
        handled_case_labels: labels,
        invalid_case_pattern_spans,
        has_fallback_pattern,
        has_unknown_coverage,
    }
}

enum GetoptsCasePatternKind {
    Fallback,
    SingleLabel(GetoptsCaseLabelFact),
    InvalidStaticPattern(Span),
    UnknownCoverage,
}

fn classify_getopts_case_pattern(pattern: &Pattern, source: &str) -> GetoptsCasePatternKind {
    if getopts_case_pattern_is_fallback(pattern, source) {
        return GetoptsCasePatternKind::Fallback;
    }

    let Some(text) = static_case_pattern_text(pattern, source) else {
        return GetoptsCasePatternKind::UnknownCoverage;
    };
    let mut chars = text.chars();
    let Some(label) = chars.next() else {
        return GetoptsCasePatternKind::UnknownCoverage;
    };
    if chars.next().is_some() {
        return GetoptsCasePatternKind::InvalidStaticPattern(pattern.span);
    }

    let is_bare_single_letter = label.is_ascii_alphabetic() && pattern.span.slice(source) == text;
    GetoptsCasePatternKind::SingleLabel(GetoptsCaseLabelFact {
        label,
        span: pattern.span,
        is_bare_single_letter,
    })
}

fn getopts_case_pattern_is_fallback(pattern: &Pattern, source: &str) -> bool {
    let mut tokens = Vec::new();
    if collect_static_case_pattern_tokens(pattern.span.slice(source), &mut tokens).is_none() {
        return false;
    }

    matches!(
        tokens.as_slice(),
        [CasePatternToken::AnyString] | [CasePatternToken::AnyChar]
    )
}

fn static_case_pattern_text(pattern: &Pattern, source: &str) -> Option<String> {
    ensure_case_pattern_is_statically_analyzable(pattern, source)?;

    let mut tokens = Vec::new();
    collect_static_case_pattern_tokens(pattern.span.slice(source), &mut tokens)?;
    tokens
        .into_iter()
        .map(|token| match token {
            CasePatternToken::Literal(ch) => Some(ch),
            CasePatternToken::AnyChar | CasePatternToken::AnyString => None,
        })
        .collect()
}

fn case_subject_variable_name(word: &Word) -> Option<&str> {
    standalone_variable_name_from_word_parts(&word.parts)
}

fn standalone_variable_name_from_word_parts(parts: &[WordPartNode]) -> Option<&str> {
    let [part] = parts else {
        return None;
    };

    match &part.kind {
        WordPart::Variable(name) => Some(name.as_str()),
        WordPart::Parameter(parameter) => match parameter.bourne() {
            Some(BourneParameterExpansion::Access { reference })
                if reference.subscript.is_none() =>
            {
                Some(reference.name.as_str())
            }
            _ => None,
        },
        WordPart::DoubleQuoted { parts, .. } => standalone_variable_name_from_word_parts(parts),
        WordPart::Literal(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::SingleQuoted { .. }
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
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => None,
    }
}

fn word_context_supports_operand_class(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::AssignmentValue
            | ExpansionContext::DeclarationAssignmentValue
            | ExpansionContext::RedirectTarget(_)
            | ExpansionContext::StringTestOperand
            | ExpansionContext::RegexOperand
            | ExpansionContext::CasePattern
            | ExpansionContext::ConditionalPattern
            | ExpansionContext::ParameterPattern
    )
}

fn word_has_literal_affixes(word: &Word) -> bool {
    word.parts.iter().any(|part| {
        matches!(
            part.kind,
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
        )
    })
}

fn word_contains_shell_quoting_literals(word: &Word, source: &str) -> bool {
    word_parts_contain_shell_quoting_literals(&word.parts, source)
}

fn word_parts_contain_shell_quoting_literals(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => {
            text_contains_shell_quoting_literals(text.as_str(source, part.span))
        }
        WordPart::SingleQuoted { value, .. } => {
            text_contains_shell_quoting_literals(value.slice(source))
        }
        WordPart::DoubleQuoted { parts, .. } => {
            word_parts_contain_shell_quoting_literals(parts, source)
        }
        _ => false,
    })
}

fn text_contains_shell_quoting_literals(text: &str) -> bool {
    if text.contains(['"', '\'']) {
        return true;
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '\\' {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && chars[end] == '\\' {
            end += 1;
        }
        if chars
            .get(end)
            .is_some_and(|next| next.is_whitespace() || matches!(next, '"' | '\''))
        {
            return true;
        }

        index = end;
    }

    false
}

fn is_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

fn is_arithmetic_variable_reference_word(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [part] if match &part.kind {
        WordPart::Variable(name) => is_shell_variable_name(name.as_str()),
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if is_shell_variable_name(reference.name.as_str()) && reference.subscript.is_none()
        ),
        _ => false,
    })
}

fn collect_arithmetic_command_spans(
    expression: &ArithmeticExprNode,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    query::visit_arithmetic_words(expression, &mut |word| {
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            true,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

fn collect_arithmetic_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    let Some(text) = text else {
        return;
    };
    if !text.slice(source).contains('$') {
        return;
    }

    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        return;
    };
    collect_arithmetic_expansion_spans_from_parts(
        &word.parts,
        source,
        collect_dollar_spans,
        dollar_spans,
        command_substitution_spans,
    );
}

fn collect_dollar_prefixed_arithmetic_variable_spans(
    span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }

        let Some(next) = bytes.get(index + 1).copied() else {
            break;
        };

        let match_end = if next == b'{' {
            let name_start = index + 2;
            let Some(first) = bytes.get(name_start).copied() else {
                index += 1;
                continue;
            };
            if !(first == b'_' || first.is_ascii_alphabetic()) {
                index += 1;
                continue;
            }

            let mut name_end = name_start + 1;
            while let Some(byte) = bytes.get(name_end).copied() {
                if byte == b'_' || byte.is_ascii_alphanumeric() {
                    name_end += 1;
                } else {
                    break;
                }
            }

            if bytes.get(name_end) != Some(&b'}') {
                index += 1;
                continue;
            }

            name_end + 1
        } else if next == b'_' || next.is_ascii_alphabetic() {
            let mut name_end = index + 2;
            while let Some(byte) = bytes.get(name_end).copied() {
                if byte == b'_' || byte.is_ascii_alphanumeric() {
                    name_end += 1;
                } else {
                    break;
                }
            }
            name_end
        } else {
            index += 1;
            continue;
        };

        let start = span.start.advanced_by(&text[..index]);
        let end = start.advanced_by(&text[index..match_end]);
        spans.push(Span::from_positions(start, end));
        index = match_end;
    }
}

fn collect_wrapped_arithmetic_spans_in_word(
    word: &Word,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    let text = word.span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 2 < bytes.len() {
        if bytes[index] != b'$' || bytes[index + 1] != b'(' || bytes[index + 2] != b'(' {
            index += 1;
            continue;
        }

        let mut depth = 1usize;
        let mut cursor = index + 3;
        let mut matched = false;

        while cursor < bytes.len() {
            if cursor + 2 < bytes.len()
                && bytes[cursor] == b'$'
                && bytes[cursor + 1] == b'('
                && bytes[cursor + 2] == b'('
            {
                depth += 1;
                cursor += 3;
                continue;
            }

            match bytes[cursor] {
                b'(' => {
                    depth += 1;
                    cursor += 1;
                }
                b')' => {
                    if depth == 1 && cursor + 1 < bytes.len() && bytes[cursor + 1] == b')' {
                        let expr_start = index + 3;
                        let expr_end = cursor;
                        let start = word.span.start.advanced_by(&text[..expr_start]);
                        let end = start.advanced_by(&text[expr_start..expr_end]);
                        let expression_span = Span::from_positions(start, end);
                        collect_dollar_prefixed_arithmetic_variable_spans(
                            expression_span,
                            source,
                            dollar_spans,
                        );
                        collect_wrapped_arithmetic_command_substitution_spans(
                            expression_span,
                            source,
                            command_substitution_spans,
                        );
                        index = cursor + 2;
                        matched = true;
                        break;
                    }

                    depth = depth.saturating_sub(1);
                    cursor += 1;
                }
                _ => {
                    cursor += 1;
                }
            }
        }

        if !matched {
            break;
        }
    }
}

fn collect_wrapped_arithmetic_command_substitution_spans(
    span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if !is_unescaped_dollar(bytes, index)
            || bytes[index + 1] != b'('
            || bytes.get(index + 2) == Some(&b'(')
        {
            index += 1;
            continue;
        }

        let Some(end) = find_command_substitution_end(bytes, index) else {
            break;
        };

        let start = span.start.advanced_by(&text[..index]);
        let end_pos = start.advanced_by(&text[index..end]);
        spans.push(Span::from_positions(start, end_pos));
        index = end;
    }
}

fn is_unescaped_dollar(bytes: &[u8], index: usize) -> bool {
    if bytes.get(index) != Some(&b'$') {
        return false;
    }

    let mut backslash_count = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslash_count += 1;
        cursor -= 1;
    }

    backslash_count.is_multiple_of(2)
}

fn find_command_substitution_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut cursor = start + 2;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(bytes, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 => return Some(cursor + 1),
            b')' => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

fn find_wrapped_arithmetic_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut cursor = start + 3;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(bytes, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 && cursor + 1 < bytes.len() && bytes[cursor + 1] == b')' => {
                return Some(cursor + 2);
            }
            b')' if paren_depth > 0 => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

fn skip_single_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\'' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

fn skip_double_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'"' => return Some(cursor + 1),
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            _ => cursor += 1,
        }
    }

    None
}

fn skip_backticks(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }
        if bytes[cursor] == b'`' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

fn word_needs_wrapped_arithmetic_fallback(word: &Word, source: &str) -> bool {
    parts_need_wrapped_arithmetic_fallback(&word.parts, source)
}

fn parts_need_wrapped_arithmetic_fallback(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            parts_need_wrapped_arithmetic_fallback(parts, source)
        }
        WordPart::Substring {
            offset_ast: None,
            offset,
            ..
        }
        | WordPart::ArraySlice {
            offset_ast: None,
            offset,
            ..
        } => offset.is_source_backed() && offset.slice(source).starts_with("$(("),
        WordPart::Parameter(parameter) => {
            parameter_needs_wrapped_arithmetic_fallback(parameter, source)
        }
        _ => false,
    })
}

fn parameter_needs_wrapped_arithmetic_fallback(
    parameter: &ParameterExpansion,
    source: &str,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
            offset_ast: None,
            offset,
            ..
        }) => offset.is_source_backed() && offset.slice(source).starts_with("$(("),
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => {
                parameter_needs_wrapped_arithmetic_fallback(parameter, source)
            }
            ZshExpansionTarget::Word(word) => word_needs_wrapped_arithmetic_fallback(word, source),
            ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Empty => false,
        },
        _ => false,
    }
}

fn collect_arithmetic_context_spans_in_word(
    word: &Word,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    if collect_dollar_spans && is_arithmetic_variable_reference_word(word) {
        dollar_spans.push(word.span);
    }

    for part in &word.parts {
        if let WordPart::CommandSubstitution { .. } = &part.kind {
            command_substitution_spans.push(part.span);
        }
    }

    collect_arithmetic_expansion_spans_from_parts(
        &word.parts,
        source,
        collect_dollar_spans,
        dollar_spans,
        command_substitution_spans,
    );
}

fn collect_arithmetic_spans_in_parameter_operator(
    operator: &ParameterOp,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    match operator {
        ParameterOp::ReplaceFirst {
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            replacement_word_ast,
            ..
        } => collect_arithmetic_expansion_spans_from_parts(
            &replacement_word_ast.parts,
            source,
            collect_dollar_spans,
            dollar_spans,
            command_substitution_spans,
        ),
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::RemovePrefixShort { .. }
        | ParameterOp::RemovePrefixLong { .. }
        | ParameterOp::RemoveSuffixShort { .. }
        | ParameterOp::RemoveSuffixLong { .. }
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

fn collect_arithmetic_expansion_spans_from_parts(
    parts: &[WordPartNode],
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_arithmetic_expansion_spans_from_parts(
                parts,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast {
                    query::visit_arithmetic_words(expression, &mut |word| {
                        collect_arithmetic_context_spans_in_word(
                            word,
                            source,
                            collect_dollar_spans,
                            dollar_spans,
                            command_substitution_spans,
                        );
                    });
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &expression_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            WordPart::Parameter(parameter) => collect_arithmetic_spans_in_parameter_expansion(
                parameter,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_parameter_operator(
                    operator,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => collect_arithmetic_spans_in_var_ref(
                reference,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
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
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                if let Some(expression) = offset_ast {
                    collect_arithmetic_command_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_command_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_expansion_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_arithmetic_spans_in_var_ref(
    reference: &VarRef,
    source: &str,
    _collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    query::visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            false,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

fn collect_arithmetic_spans_in_parameter_expansion(
    parameter: &ParameterExpansion,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_parameter_operator(
                    operator,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
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
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                if let Some(expression) = offset_ast {
                    collect_arithmetic_command_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_command_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_expansion_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => collect_arithmetic_spans_in_var_ref(
                reference,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            ZshExpansionTarget::Nested(parameter) => {
                collect_arithmetic_spans_in_parameter_expansion(
                    parameter,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                )
            }
            ZshExpansionTarget::Word(word) => collect_arithmetic_expansion_spans_from_parts(
                &word.parts,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            ZshExpansionTarget::Empty => {}
        },
    }
}

fn word_classification_from_analysis(analysis: ExpansionAnalysis) -> WordClassification {
    WordClassification {
        quote: analysis.quote,
        literalness: analysis.literalness,
        expansion_kind: match (analysis.has_scalar_expansion(), analysis.array_valued) {
            (false, false) => WordExpansionKind::None,
            (true, false) => WordExpansionKind::Scalar,
            (false, true) => WordExpansionKind::Array,
            (true, true) => WordExpansionKind::Mixed,
        },
        substitution_shape: if analysis.substitution_shape == WordSubstitutionShape::None {
            WordSubstitutionShape::None
        } else if analysis.substitution_shape == WordSubstitutionShape::Plain {
            WordSubstitutionShape::Plain
        } else {
            WordSubstitutionShape::Mixed
        },
    }
}

fn double_quoted_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_expansion_spans(&word.parts, false, &mut spans);
    spans
}

fn collect_double_quoted_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_double_quoted_expansion_spans(parts, true, spans);
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
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
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_)
                if inside_double_quotes =>
            {
                spans.push(part.span)
            }
            WordPart::Literal(_) => {}
            _ => {}
        }
    }
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

    glued_closing_bracket_unary_operand_span(&args, source)
}

fn glued_closing_bracket_unary_operand_span(args: &[&Word], source: &str) -> Option<Span> {
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
        .then_some(Span::from_positions(operand.span.start, operand.span.start));
    };

    (simple_test_is_unary_operator(first.span.slice(source))
        && second
            .span
            .slice(source)
            .strip_suffix(']')
            .is_some_and(|prefix| !prefix.is_empty()))
    .then_some(Span::from_positions(second.span.start, second.span.start))
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

fn build_conditional_fact<'a>(command: &'a Command, source: &str) -> Option<ConditionalFact<'a>> {
    let Command::Compound(CompoundCommand::Conditional(command)) = command else {
        return None;
    };
    let mut nodes = Vec::new();
    collect_conditional_nodes(&command.expression, source, &mut nodes);
    let mut mixed_logical_operator_spans = Vec::new();
    collect_mixed_logical_operator_spans(
        &command.expression,
        false,
        &mut mixed_logical_operator_spans,
    );
    (!nodes.is_empty()).then_some(ConditionalFact {
        nodes: nodes.into_boxed_slice(),
        mixed_logical_operator_spans: mixed_logical_operator_spans.into_boxed_slice(),
    })
}

fn collect_mixed_logical_operator_spans(
    expression: &ConditionalExpr,
    parent_in_same_logical_group: bool,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Parenthesized(parenthesized) => {
            collect_mixed_logical_operator_spans(&parenthesized.expr, false, spans);
        }
        ConditionalExpr::Unary(unary) => {
            collect_mixed_logical_operator_spans(&unary.expr, false, spans);
        }
        ConditionalExpr::Binary(binary) => {
            let left_continues_group = matches!(
                binary.left.as_ref(),
                ConditionalExpr::Binary(left)
                    if matches!(left.op, ConditionalBinaryOp::And | ConditionalBinaryOp::Or)
            );
            let right_continues_group = matches!(
                binary.right.as_ref(),
                ConditionalExpr::Binary(right)
                    if matches!(right.op, ConditionalBinaryOp::And | ConditionalBinaryOp::Or)
            );

            collect_mixed_logical_operator_spans(&binary.left, left_continues_group, spans);
            collect_mixed_logical_operator_spans(&binary.right, right_continues_group, spans);

            if matches!(
                binary.op,
                ConditionalBinaryOp::And | ConditionalBinaryOp::Or
            ) && !parent_in_same_logical_group
                && logical_operator_mask(expression) == (LOGICAL_AND_MASK | LOGICAL_OR_MASK)
            {
                spans.push(binary.op_span);
            }
        }
        ConditionalExpr::Word(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => {}
    }
}

const LOGICAL_AND_MASK: u8 = 0b01;
const LOGICAL_OR_MASK: u8 = 0b10;

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

fn collect_conditional_nodes<'a>(
    expression: &'a ConditionalExpr,
    source: &str,
    nodes: &mut Vec<ConditionalNodeFact<'a>>,
) {
    let expression = strip_parenthesized_conditionals(expression);
    nodes.push(build_conditional_node(expression, source));

    match expression {
        ConditionalExpr::Binary(expression) => {
            collect_conditional_nodes(&expression.left, source, nodes);
            collect_conditional_nodes(&expression.right, source, nodes);
        }
        ConditionalExpr::Unary(expression) => {
            collect_conditional_nodes(&expression.expr, source, nodes);
        }
        ConditionalExpr::Parenthesized(_) => unreachable!("parentheses should be stripped"),
        ConditionalExpr::Word(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => {}
    }
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

fn read_uses_raw_input(args: &[&Word], source: &str) -> bool {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;
        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'r' {
                return true;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        index += 1;
    }

    false
}

fn parse_echo_command<'a>(args: &[&'a Word], source: &str) -> EchoCommandFacts<'a> {
    let mut portability_flag_word = None;
    let mut uses_escape_interpreting_flag = false;

    for word in args {
        if !classify_word(word, source).is_fixed_literal() {
            break;
        }

        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if !is_echo_portability_flag(text.as_str()) {
            break;
        }

        portability_flag_word.get_or_insert(*word);
        uses_escape_interpreting_flag |= text.contains('e');
    }

    EchoCommandFacts {
        portability_flag_word,
        uses_escape_interpreting_flag,
    }
}

fn parse_sed_command(args: &[&Word], source: &str) -> SedCommandFacts {
    SedCommandFacts {
        has_single_substitution_script: sed_has_single_substitution_script(
            args,
            source,
            SedScriptQuoteMode::ShellOnly,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SedScriptQuoteMode {
    ShellOnly,
    AllowBacktickEscapedDoubleQuotes,
}

fn sed_script_text<'a>(
    args: &[&Word],
    source: &'a str,
    quote_mode: SedScriptQuoteMode,
) -> Option<Cow<'a, str>> {
    match args {
        [script] => Some(Cow::Borrowed(strip_matching_sed_script_quotes_in_source(
            script.span.slice(source),
            quote_mode,
        ))),
        [first, .., last]
            if quote_mode == SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes
                && first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"") =>
        {
            let mut text = String::new();
            for (index, word) in args.iter().enumerate() {
                if index != 0 {
                    text.push(' ');
                }
                text.push_str(word.span.slice(source));
            }
            Some(Cow::Owned(
                strip_backtick_escaped_double_quotes_in_source(&text).to_owned(),
            ))
        }
        _ => None,
    }
}

fn sed_has_single_substitution_script(
    args: &[&Word],
    source: &str,
    quote_mode: SedScriptQuoteMode,
) -> bool {
    sed_script_text(args, source, quote_mode)
        .or_else(|| match args {
            [flag, words @ ..] if static_word_text(flag, source).as_deref() == Some("-e") => {
                sed_script_text(words, source, quote_mode)
            }
            _ => None,
        })
        .as_deref()
        .is_some_and(is_simple_sed_substitution_script)
}

fn is_echo_portability_flag(text: &str) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };

    !flags.is_empty()
        && flags
            .bytes()
            .all(|byte| matches!(byte, b'n' | b'e' | b'E' | b's'))
}

fn is_simple_sed_substitution_script(text: &str) -> bool {
    let Some(remainder) = text.strip_prefix('s') else {
        return false;
    };

    let Some(delimiter) = remainder.chars().next() else {
        return false;
    };
    if delimiter.is_whitespace() || delimiter == '\\' {
        return false;
    }

    let pattern_start = 1 + delimiter.len_utf8();
    let Some((pattern_end, pattern_has_escaped_delimiter)) =
        find_sed_substitution_section(text, pattern_start, delimiter)
    else {
        return false;
    };
    let replacement_start = pattern_end + delimiter.len_utf8();
    let Some((replacement_end, replacement_has_escaped_delimiter)) =
        find_sed_substitution_section(text, replacement_start, delimiter)
    else {
        return false;
    };

    let flags = &text[replacement_end + delimiter.len_utf8()..];
    if flags.chars().any(|ch| ch.is_whitespace() || ch == ';') {
        return false;
    }

    let pattern = &text[pattern_start..pattern_end];
    let replacement = &text[replacement_start..replacement_end];
    !pattern_has_escaped_delimiter
        && !replacement_has_escaped_delimiter
        && !uses_delimiter_sensitive_match_escape(pattern, replacement, delimiter)
}

fn find_sed_substitution_section(
    text: &str,
    start: usize,
    delimiter: char,
) -> Option<(usize, bool)> {
    let _ = text.get(start..)?;
    let mut index = start;
    let mut saw_escaped_delimiter = false;
    let mut escaped = false;
    let mut character_class_contents = None;

    while index < text.len() {
        let mut chars = text[index..].chars();
        let Some(ch) = chars.next() else {
            break;
        };
        let next = index + ch.len_utf8();

        if let Some(contents) = character_class_contents.as_mut() {
            if escaped {
                if ch == delimiter {
                    saw_escaped_delimiter = true;
                }
                *contents += 1;
                escaped = false;
            } else {
                match ch {
                    '\\' => {
                        escaped = true;
                    }
                    '^' if *contents == 0 => {}
                    ']' if *contents > 0 => {
                        character_class_contents = None;
                    }
                    _ => {
                        *contents += 1;
                    }
                }
            }
            index = next;
            continue;
        }

        if escaped {
            if ch == delimiter {
                saw_escaped_delimiter = true;
            }
            escaped = false;
            index = next;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
            }
            '[' => {
                character_class_contents = Some(0);
            }
            ch if ch == delimiter => return Some((index, saw_escaped_delimiter)),
            _ => {}
        }
        index = next;
    }

    None
}

fn uses_delimiter_sensitive_match_escape(
    pattern: &str,
    replacement: &str,
    delimiter: char,
) -> bool {
    delimiter == '/'
        && pattern.contains(delimiter)
        && is_backslash_prefixed_match_escape(replacement)
}

fn is_backslash_prefixed_match_escape(replacement: &str) -> bool {
    replacement == r"\\&"
        || replacement.strip_prefix(r"\\").is_some_and(|rest| {
            matches!(rest.as_bytes(), [b'\\', b'1'..=b'9', ..])
                && rest[1..].bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn strip_matching_sed_script_quotes_in_source(text: &str, quote_mode: SedScriptQuoteMode) -> &str {
    if quote_mode == SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes
        && text.len() >= 4
        && text.starts_with("\\\"")
        && text.ends_with("\\\"")
    {
        strip_backtick_escaped_double_quotes_in_source(text)
    } else {
        strip_shell_matching_quotes_in_source(text)
    }
}

fn strip_shell_matching_quotes_in_source(text: &str) -> &str {
    if text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')))
    {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn strip_backtick_escaped_double_quotes_in_source(text: &str) -> &str {
    debug_assert!(text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\""));
    &text[2..text.len() - 2]
}

fn parse_tr_command<'a>(args: &[&'a Word], source: &str) -> TrCommandFacts<'a> {
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !is_tr_option(text.as_str()) {
            break;
        }

        index += 1;
    }

    TrCommandFacts {
        operand_words: args[index..].iter().copied().collect(),
    }
}

fn is_tr_option(text: &str) -> bool {
    text.starts_with('-')
        && text != "-"
        && !text.starts_with("--")
        && text[1..]
            .bytes()
            .all(|byte| matches!(byte, b'c' | b'C' | b'd' | b's' | b't'))
}

fn word_starts_with_literal_dash(word: &Word, source: &str) -> bool {
    matches!(
        word.parts_with_spans().next(),
        Some((WordPart::Literal(text), span)) if text.as_str(source, span).starts_with('-')
    )
}

fn parse_rm_command(args: &[&Word], source: &str) -> Option<RmCommandFacts> {
    let mut index = 0usize;
    let mut recursive = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if text == "--recursive"
            || (text.starts_with('-') && !text.starts_with("--") && text[1..].contains('r'))
            || (text.starts_with('-') && !text.starts_with("--") && text[1..].contains('R'))
        {
            recursive = true;
        }

        index += 1;
    }

    if !recursive {
        return None;
    }

    let dangerous_path_spans = args[index..]
        .iter()
        .filter(|word| rm_path_is_dangerous(word, source))
        .map(|word| word.span)
        .collect::<Vec<_>>();

    (!dangerous_path_spans.is_empty()).then_some(RmCommandFacts {
        dangerous_path_spans: dangerous_path_spans.into_boxed_slice(),
    })
}

fn parse_ssh_command(args: &[&Word], source: &str) -> Option<SshCommandFacts> {
    let remote_args = ssh_remote_args(args, source)?;
    if remote_args.is_empty() {
        return None;
    }

    let local_expansion_spans = remote_args
        .iter()
        .flat_map(|word| expansion_part_spans(word))
        .collect::<Vec<_>>();

    Some(SshCommandFacts {
        local_expansion_spans: local_expansion_spans.into_boxed_slice(),
    })
}

fn parse_su_command(args: &[&Word], source: &str) -> SuCommandFacts {
    let mut pending_option_arg = false;
    let mut saw_user = false;
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if pending_option_arg {
                pending_option_arg = false;
            } else if saw_user {
                break;
            } else {
                saw_user = true;
            }
            index += 1;
            continue;
        };

        if pending_option_arg {
            pending_option_arg = false;
            index += 1;
            continue;
        }

        match text.as_str() {
            "-" | "-l" | "--login" => {
                return SuCommandFacts {
                    has_login_or_command_flag: true,
                };
            }
            "--" => {
                break;
            }
            "--command" => {
                if args.get(index + 1).is_some() {
                    return SuCommandFacts {
                        has_login_or_command_flag: true,
                    };
                }
            }
            _ if text.starts_with("--command=") => {
                if text.len() > "--command=".len() {
                    return SuCommandFacts {
                        has_login_or_command_flag: true,
                    };
                }
            }
            _ if su_long_option_takes_argument(text.as_str()) => {
                pending_option_arg = true;
                index += 1;
                continue;
            }
            _ => {}
        }

        if text.starts_with("--") {
            index += 1;
            continue;
        }

        if !text.starts_with('-') {
            if saw_user {
                break;
            }
            saw_user = true;
            index += 1;
            continue;
        }

        let mut flags = text[1..].chars().peekable();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => {
                    return SuCommandFacts {
                        has_login_or_command_flag: true,
                    };
                }
                'c' => {
                    if flags.peek().is_some() || args.get(index + 1).is_some() {
                        return SuCommandFacts {
                            has_login_or_command_flag: true,
                        };
                    }
                }
                flag if su_short_option_takes_argument(flag) => {
                    if flags.peek().is_none() {
                        pending_option_arg = true;
                    }
                    break;
                }
                _ => {}
            }
        }

        index += 1;
    }

    SuCommandFacts {
        has_login_or_command_flag: false,
    }
}

fn su_long_option_takes_argument(text: &str) -> bool {
    matches!(
        text,
        "--group" | "--supp-group" | "--shell" | "--whitelist-environment"
    )
}

fn su_short_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'C' | 'g' | 'G' | 's' | 'w')
}

fn ssh_remote_args<'a>(args: &'a [&'a Word], source: &str) -> Option<&'a [&'a Word]> {
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        let consumes_next = ssh_option_consumes_next_argument(text.as_str())?;
        index += 1;
        if consumes_next {
            args.get(index)?;
            index += 1;
        }
    }

    let _destination = args.get(index)?;
    Some(&args[index + 1..])
}

fn ssh_option_consumes_next_argument(text: &str) -> Option<bool> {
    if !text.starts_with('-') || text == "-" {
        return Some(false);
    }
    if text == "--" {
        return Some(false);
    }

    let bytes = text.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() {
        let flag = bytes[index];
        if ssh_option_requires_argument(flag) {
            return Some(index + 1 == bytes.len());
        }
        if !ssh_option_is_flag(flag) {
            return None;
        }
        index += 1;
    }

    Some(false)
}

fn ssh_option_requires_argument(flag: u8) -> bool {
    matches!(
        flag,
        b'B' | b'b'
            | b'c'
            | b'D'
            | b'E'
            | b'e'
            | b'F'
            | b'I'
            | b'i'
            | b'J'
            | b'L'
            | b'l'
            | b'm'
            | b'O'
            | b'o'
            | b'p'
            | b'P'
            | b'Q'
            | b'R'
            | b'S'
            | b'W'
            | b'w'
    )
}

fn ssh_option_is_flag(flag: u8) -> bool {
    ssh_option_requires_argument(flag)
        || matches!(
            flag,
            b'4' | b'6'
                | b'A'
                | b'a'
                | b'C'
                | b'f'
                | b'G'
                | b'g'
                | b'K'
                | b'k'
                | b'M'
                | b'N'
                | b'n'
                | b'q'
                | b's'
                | b'T'
                | b't'
                | b'V'
                | b'v'
                | b'X'
                | b'x'
                | b'Y'
                | b'y'
        )
}

fn rm_path_is_dangerous(word: &Word, source: &str) -> bool {
    let segments = rm_path_segments(word, source);
    if segments.is_empty() || !segments[0].has_unsafe_param {
        return false;
    }

    let brace_expansion_active = word.has_active_brace_expansion();
    let mut saw_literal_barrier = false;
    let mut saw_pure_unsafe = false;
    let mut tail_start = 1usize;

    for (index, segment) in segments.iter().enumerate().skip(1) {
        if rm_path_segment_is_pure_unsafe_parameter(segment) {
            if saw_literal_barrier {
                return false;
            }
            saw_pure_unsafe = true;
            tail_start = index + 1;
            continue;
        }

        if segment.has_literal_text || segment.has_other_dynamic || segment.has_unsafe_param {
            if saw_pure_unsafe {
                let tail = rm_path_tail_text(&segments[index..]);
                return rm_path_tail_is_dangerous(&tail, brace_expansion_active);
            }

            saw_literal_barrier = true;
        }
    }

    if saw_pure_unsafe {
        let tail = rm_path_tail_text(&segments[tail_start..]);
        return tail.is_empty() || rm_path_tail_is_dangerous(&tail, brace_expansion_active);
    }

    let tail = rm_path_tail_text(&segments[1..]);
    !tail.is_empty() && rm_path_tail_is_dangerous(&tail, brace_expansion_active)
}

#[derive(Debug, Default)]
struct RmPathSegment {
    has_unsafe_param: bool,
    has_literal_text: bool,
    has_other_dynamic: bool,
    text: String,
}

fn rm_path_segments(word: &Word, source: &str) -> Vec<RmPathSegment> {
    let mut segments = vec![RmPathSegment::default()];
    append_rm_path_segments(&mut segments, &word.parts, source);
    segments
}

fn append_rm_path_segments(
    segments: &mut Vec<RmPathSegment>,
    parts: &[WordPartNode],
    source: &str,
) {
    for part in parts {
        append_rm_path_part(segments, &part.kind, part.span, source);
    }
}

fn append_rm_path_part(
    segments: &mut Vec<RmPathSegment>,
    part: &WordPart,
    span: Span,
    source: &str,
) {
    match part {
        WordPart::Literal(text) => append_rm_path_literal(segments, text.as_str(source, span)),
        WordPart::SingleQuoted {
            value,
            dollar: false,
        } => append_rm_path_literal(segments, value.slice(source)),
        WordPart::SingleQuoted { dollar: true, .. } => {
            current_rm_path_segment(segments).has_other_dynamic = true;
        }
        WordPart::DoubleQuoted { parts, .. } => append_rm_path_segments(segments, parts, source),
        WordPart::Variable(_) => {
            current_rm_path_segment(segments).has_unsafe_param = true;
        }
        WordPart::Parameter(parameter) => {
            if rm_path_parameter_expansion_is_unsafe(parameter) {
                current_rm_path_segment(segments).has_unsafe_param = true;
            }
        }
        WordPart::ParameterExpansion {
            operator,
            colon_variant: _,
            ..
        } => {
            if rm_path_parameter_op_is_unsafe(operator) {
                current_rm_path_segment(segments).has_unsafe_param = true;
            }
        }
        WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ZshQualifiedGlob(_) => {
            current_rm_path_segment(segments).has_other_dynamic = true;
        }
    }
}

fn rm_path_parameter_expansion_is_unsafe(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { .. } => true,
            BourneParameterExpansion::Indirect {
                operator,
                operand: _,
                colon_variant: _,
                ..
            } => operator.as_ref().is_none_or(rm_path_parameter_op_is_unsafe),
            BourneParameterExpansion::Operation { operator, .. } => {
                rm_path_parameter_op_is_unsafe(operator)
            }
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. } => false,
        },
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn rm_path_parameter_op_is_unsafe(operator: &ParameterOp) -> bool {
    !matches!(
        operator,
        ParameterOp::UseDefault | ParameterOp::AssignDefault | ParameterOp::Error
    )
}

fn append_rm_path_literal(segments: &mut Vec<RmPathSegment>, text: &str) {
    for character in text.chars() {
        if character == '/' {
            segments.push(RmPathSegment::default());
            continue;
        }

        let segment = current_rm_path_segment(segments);
        segment.has_literal_text = true;
        segment.text.push(character);
    }
}

fn current_rm_path_segment(segments: &mut [RmPathSegment]) -> &mut RmPathSegment {
    segments
        .last_mut()
        .expect("rm path segments always start non-empty")
}

fn rm_path_segment_is_pure_unsafe_parameter(segment: &RmPathSegment) -> bool {
    segment.has_unsafe_param && !segment.has_literal_text && !segment.has_other_dynamic
}

fn rm_path_tail_text(segments: &[RmPathSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

const RM_DANGEROUS_LITERAL_SUFFIXES: &[&str] = &[
    "bin",
    "boot",
    "dev",
    "etc",
    "home",
    "lib",
    "opt",
    "usr/bin",
    "usr/local",
    "usr/share",
    "var",
];

fn rm_path_tail_is_dangerous(tail: &str, brace_expansion_active: bool) -> bool {
    if brace_expansion_active && let Some((prefix, inner, suffix)) = split_brace_expansion(tail) {
        return split_brace_alternatives(inner)
            .into_iter()
            .any(|alternative| {
                rm_path_tail_is_dangerous(&format!("{prefix}{alternative}{suffix}"), true)
            });
    }

    let tail = tail.trim_start_matches('/');
    if tail.is_empty() {
        return false;
    }

    if let Some(prefix) = tail.strip_suffix('*') {
        let prefix = prefix.trim_end_matches('/');
        return prefix.is_empty() || RM_DANGEROUS_LITERAL_SUFFIXES.contains(&prefix);
    }

    RM_DANGEROUS_LITERAL_SUFFIXES.contains(&tail)
}

fn split_brace_expansion(text: &str) -> Option<(&str, &str, &str)> {
    let bytes = text.as_bytes();
    let open = bytes.iter().position(|byte| *byte == b'{')?;
    let mut depth = 0usize;

    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((&text[..open], &text[open + 1..index], &text[index + 1..]));
                }
            }
            _ => {}
        }
    }

    None
}

fn split_brace_alternatives(text: &str) -> Vec<&str> {
    let mut alternatives = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;

    for (index, byte) in text.as_bytes().iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                alternatives.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }

    alternatives.push(&text[start..]);
    alternatives
}

fn parse_grep_command<'a>(args: &[&'a Word], source: &str) -> Option<GrepCommandFacts<'a>> {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;
    let mut uses_only_matching = false;
    let mut uses_fixed_strings = false;
    let mut explicit_pattern_source = false;
    let mut patterns = Vec::new();

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;

        if text == "--only-matching" {
            uses_only_matching = true;
            index += 1;
            continue;
        }

        if text == "--fixed-strings" {
            uses_fixed_strings = true;
            index += 1;
            continue;
        }

        if matches!(
            text.as_str(),
            "--basic-regexp" | "--extended-regexp" | "--perl-regexp"
        ) {
            uses_fixed_strings = false;
            index += 1;
            continue;
        }

        if text == "--regexp" {
            explicit_pattern_source = true;
            if let Some(pattern_word) = args.get(index + 1) {
                patterns.push(grep_pattern_fact(
                    pattern_word,
                    source,
                    GrepPatternSourceKind::LongOptionSeparate,
                ));
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if text.starts_with("--regexp=") {
            explicit_pattern_source = true;
            patterns.push(grep_prefixed_pattern_fact(
                word,
                source,
                "--regexp=".len(),
                GrepPatternSourceKind::LongOptionAttached,
            ));
            index += 1;
            continue;
        }

        if text == "--file" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        if text.starts_with("--file=") {
            explicit_pattern_source = true;
            index += 1;
            continue;
        }

        if text.starts_with("--") {
            index += if grep_long_option_takes_argument(text.as_str())
                && args.get(index + 1).is_some()
            {
                2
            } else {
                1
            };
            continue;
        }

        if text == "-e" {
            explicit_pattern_source = true;
            if let Some(pattern_word) = args.get(index + 1) {
                patterns.push(grep_pattern_fact(
                    pattern_word,
                    source,
                    GrepPatternSourceKind::ShortOptionSeparate,
                ));
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if text == "-f" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == 'o' {
                uses_only_matching = true;
            }

            if flag == 'F' {
                uses_fixed_strings = true;
            }

            if matches!(flag, 'E' | 'G' | 'P') {
                uses_fixed_strings = false;
            }

            if flag == 'e' {
                explicit_pattern_source = true;
                if chars.peek().is_some() {
                    patterns.push(grep_prefixed_pattern_fact(
                        word,
                        source,
                        2,
                        GrepPatternSourceKind::ShortOptionAttached,
                    ));
                } else if let Some(pattern_word) = args.get(index + 1) {
                    patterns.push(grep_pattern_fact(
                        pattern_word,
                        source,
                        GrepPatternSourceKind::ShortOptionSeparate,
                    ));
                    consume_next_argument = true;
                }
                break;
            }

            if grep_option_takes_argument(flag) {
                if flag == 'f' {
                    explicit_pattern_source = true;
                }
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    if !explicit_pattern_source && let Some(pattern_word) = args.get(index) {
        patterns.push(grep_pattern_fact(
            pattern_word,
            source,
            GrepPatternSourceKind::ImplicitOperand,
        ));
    }

    Some(GrepCommandFacts {
        uses_only_matching,
        uses_fixed_strings,
        patterns: patterns.into_boxed_slice(),
    })
}

fn same_command_file_operand_words<'a>(
    command_name: Option<&str>,
    args: &[&'a Word],
    source: &str,
) -> Box<[&'a Word]> {
    match command_name {
        Some("grep") => grep_file_operand_words(args, source).into_boxed_slice(),
        Some("sed") => {
            let skip_initial_positionals =
                usize::from(!sed_has_explicit_script_source(args, source));
            collect_file_operand_words_after_prefix(
                args,
                source,
                skip_initial_positionals,
                |text| match text {
                    "-e" | "-f" | "--expression" | "--file" => Some(OperandArgAction::SkipNext),
                    _ if text.starts_with("--expression=") || text.starts_with("--file=") => None,
                    _ => None,
                },
            )
            .into_boxed_slice()
        }
        Some("awk") => {
            let skip_initial_positionals = usize::from(!awk_has_file_program_source(args, source));
            collect_file_operand_words_after_prefix(
                args,
                source,
                skip_initial_positionals,
                |text| match text {
                    "-f" | "-v" | "--file" | "--assign" => Some(OperandArgAction::SkipNext),
                    _ if text.starts_with("--file=") || text.starts_with("--assign=") => None,
                    _ => None,
                },
            )
            .into_boxed_slice()
        }
        Some("unzip") => {
            collect_file_operand_words_after_prefix(args, source, 1, |text| match text {
                "-d" | "--d" | "--directory" => Some(OperandArgAction::SkipNext),
                _ if text.starts_with("--directory=") => None,
                _ => None,
            })
            .into_boxed_slice()
        }
        Some("sort") => {
            collect_file_operand_words_after_prefix(args, source, 0, |text| match text {
                "-o" | "--output" => Some(OperandArgAction::SkipNext),
                _ if text.starts_with("--output=") => None,
                _ => None,
            })
            .into_boxed_slice()
        }
        Some("jq") => jq_file_operand_words(args, source).into_boxed_slice(),
        Some("bsdtar") | Some("tar") => {
            collect_file_operand_words_after_prefix(args, source, 0, |text| match text {
                "--exclude" => Some(OperandArgAction::IncludeNext),
                _ if text.starts_with("--exclude=") => None,
                _ => None,
            })
            .into_boxed_slice()
        }
        Some(
            "cat" | "cp" | "mv" | "head" | "tail" | "cut" | "uniq" | "comm" | "join" | "paste",
        ) => collect_file_operand_words_after_prefix(args, source, 0, |_| None).into_boxed_slice(),
        _ => Vec::new().into_boxed_slice(),
    }
}

fn sed_has_explicit_script_source(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|text| {
            matches!(text.as_str(), "-e" | "-f" | "--expression" | "--file")
                || text.starts_with("--expression=")
                || text.starts_with("--file=")
                || short_option_cluster_contains_flag(text.as_str(), 'e')
                || short_option_cluster_contains_flag(text.as_str(), 'f')
        })
}

fn awk_has_file_program_source(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|text| {
            matches!(text.as_str(), "-f" | "--file")
                || text.starts_with("--file=")
                || short_option_cluster_contains_flag(text.as_str(), 'f')
        })
}

fn short_option_cluster_contains_flag(text: &str, flag: char) -> bool {
    let Some(cluster) = text.strip_prefix('-') else {
        return false;
    };

    !cluster.is_empty() && !cluster.starts_with('-') && cluster.contains(flag)
}

fn build_scope_read_source_words<'a>(
    commands: &[CommandFact<'a>],
    pipelines: &[PipelineFact<'a>],
    if_condition_command_ids: &FxHashSet<CommandId>,
) -> Vec<Box<[PathWordFact<'a>]>> {
    let mut words_by_command = vec![Vec::new(); commands.len()];

    for command in commands {
        let mut scope_words = own_scope_read_source_words(command, if_condition_command_ids);
        if command_has_file_output_redirect(command) {
            scope_words.extend(nested_scope_read_source_words(
                commands,
                command,
                if_condition_command_ids,
            ));
        }
        dedup_path_words(&mut scope_words);
        words_by_command[command.id().index()] = scope_words;
    }

    for pipeline in pipelines {
        let writer_ids = pipeline
            .segments()
            .iter()
            .map(|segment| segment.command_id())
            .filter(|id| {
                commands
                    .get(id.index())
                    .is_some_and(command_has_file_output_redirect)
            })
            .collect::<Vec<_>>();
        if writer_ids.is_empty() {
            continue;
        }

        let mut pipeline_words = commands
            .iter()
            .filter(|command| contains_span(pipeline.span(), command.span()))
            .flat_map(|command| own_scope_read_source_words(command, if_condition_command_ids))
            .collect::<Vec<_>>();
        dedup_path_words(&mut pipeline_words);

        for writer_id in writer_ids {
            words_by_command[writer_id.index()].extend(pipeline_words.iter().copied());
            dedup_path_words(&mut words_by_command[writer_id.index()]);
        }
    }

    words_by_command
        .into_iter()
        .map(Vec::into_boxed_slice)
        .collect()
}

fn own_scope_read_source_words<'a>(
    command: &CommandFact<'a>,
    if_condition_command_ids: &FxHashSet<CommandId>,
) -> Vec<PathWordFact<'a>> {
    let mut words = command_file_operand_words(command)
        .into_iter()
        .map(|word| PathWordFact {
            word,
            context: ExpansionContext::CommandArgument,
        })
        .collect::<Vec<_>>();
    words.extend(command_redirect_read_source_words(command));
    if !if_condition_command_ids.contains(&command.id()) {
        words.extend(command_conditional_path_words(command));
    }
    words
}

fn nested_scope_read_source_words<'a>(
    commands: &[CommandFact<'a>],
    command: &CommandFact<'a>,
    if_condition_command_ids: &FxHashSet<CommandId>,
) -> Vec<PathWordFact<'a>> {
    commands
        .iter()
        .filter(|other| other.id() != command.id() && contains_span(command.span(), other.span()))
        .flat_map(|other| own_scope_read_source_words(other, if_condition_command_ids))
        .collect()
}

fn dedup_path_words(words: &mut Vec<PathWordFact<'_>>) {
    let mut seen = FxHashSet::<(FactSpan, ExpansionContext)>::default();
    words.retain(|fact| seen.insert((FactSpan::new(fact.word().span), fact.context())));
}

fn command_has_file_output_redirect(command: &CommandFact<'_>) -> bool {
    command.redirect_facts().iter().any(|redirect| {
        matches!(
            redirect.redirect().kind,
            RedirectKind::Output
                | RedirectKind::Clobber
                | RedirectKind::Append
                | RedirectKind::OutputBoth
        ) && redirect
            .analysis()
            .is_some_and(|analysis| analysis.is_file_target())
    })
}

fn command_file_operand_words<'a>(command: &CommandFact<'a>) -> Vec<&'a Word> {
    command.file_operand_words().to_vec()
}

fn command_redirect_read_source_words<'a>(command: &CommandFact<'a>) -> Vec<PathWordFact<'a>> {
    command
        .redirect_facts()
        .iter()
        .filter_map(|redirect| {
            if !matches!(
                redirect.redirect().kind,
                RedirectKind::Input | RedirectKind::ReadWrite
            ) {
                return None;
            }

            Some(PathWordFact {
                word: redirect.redirect().word_target()?,
                context: ExpansionContext::from_redirect_kind(redirect.redirect().kind)
                    .expect("input redirects should carry a word target context"),
            })
        })
        .collect()
}

fn command_conditional_path_words<'a>(command: &CommandFact<'a>) -> Vec<PathWordFact<'a>> {
    let mut words = Vec::new();

    if let Some(conditional) = command.conditional() {
        for node in conditional.nodes() {
            match node {
                ConditionalNodeFact::Binary(binary)
                    if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
                {
                    if let Some(word) = binary.left().word() {
                        words.push(PathWordFact {
                            word,
                            context: ExpansionContext::StringTestOperand,
                        });
                    }
                    if let Some(word) = binary.right().word() {
                        words.push(PathWordFact {
                            word,
                            context: ExpansionContext::StringTestOperand,
                        });
                    }
                }
                ConditionalNodeFact::Binary(_) => {}
                ConditionalNodeFact::BareWord(_) | ConditionalNodeFact::Other(_) => {}
                ConditionalNodeFact::Unary(_) => {}
            }
        }
    }

    words
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperandArgAction {
    IncludeNext,
    SkipNext,
}

fn collect_file_operand_words_after_prefix<'a>(
    args: &[&'a Word],
    source: &str,
    skip_initial_positionals: usize,
    mut option_arg_action: impl FnMut(&str) -> Option<OperandArgAction>,
) -> Vec<&'a Word> {
    let mut operands = Vec::new();
    let mut index = 0usize;
    let mut options_open = true;
    let mut pending_option_arg_action: Option<OperandArgAction> = None;
    let mut remaining_prefix_words = skip_initial_positionals;

    while let Some(word) = args.get(index) {
        if let Some(action) = pending_option_arg_action.take() {
            if matches!(action, OperandArgAction::IncludeNext) {
                operands.push(*word);
            }
            index += 1;
            continue;
        }

        let Some(text) = static_word_text(word, source) else {
            if options_open && word_starts_with_literal_dash(word, source) {
                index += 1;
                continue;
            }

            if options_open {
                options_open = false;
            }

            if remaining_prefix_words > 0 {
                remaining_prefix_words -= 1;
                index += 1;
                continue;
            }

            operands.push(*word);
            index += 1;
            continue;
        };

        if options_open && text == "--" {
            options_open = false;
            index += 1;
            continue;
        }

        if options_open && text.starts_with('-') && text != "-" {
            if let Some(action) = option_arg_action(text.as_str()) {
                pending_option_arg_action = Some(action);
            }
            index += 1;
            continue;
        }

        options_open = false;
        if remaining_prefix_words > 0 {
            remaining_prefix_words -= 1;
            index += 1;
            continue;
        }

        operands.push(*word);
        index += 1;
    }

    operands
}

fn jq_file_operand_words<'a>(args: &[&'a Word], source: &str) -> Vec<&'a Word> {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PendingOptionArgs {
        Skip(usize),
        NamedFileSource { seen_name: bool },
    }

    let mut operands = Vec::new();
    let mut index = 0usize;
    let mut options_open = true;
    let mut pending_args: Option<PendingOptionArgs> = None;
    let mut filter_from_file = false;
    let mut null_input = false;
    let mut consumed_filter = false;
    let mut positional_args_mode = false;

    while let Some(word) = args.get(index) {
        if let Some(pending) = pending_args {
            match pending {
                PendingOptionArgs::Skip(remaining) => {
                    if remaining > 1 {
                        pending_args = Some(PendingOptionArgs::Skip(remaining - 1));
                    } else {
                        pending_args = None;
                    }
                    index += 1;
                    continue;
                }
                PendingOptionArgs::NamedFileSource { seen_name: false } => {
                    pending_args = Some(PendingOptionArgs::NamedFileSource { seen_name: true });
                    index += 1;
                    continue;
                }
                PendingOptionArgs::NamedFileSource { seen_name: true } => {
                    operands.push(*word);
                    pending_args = None;
                    index += 1;
                    continue;
                }
            }
        }

        let Some(text) = static_word_text(word, source) else {
            if options_open && word_starts_with_literal_dash(word, source) {
                index += 1;
                continue;
            }

            options_open = false;
            if !consumed_filter && !filter_from_file {
                consumed_filter = true;
                index += 1;
                continue;
            }

            if !null_input && !positional_args_mode {
                operands.push(*word);
            }
            index += 1;
            continue;
        };

        if options_open && text == "--" {
            options_open = false;
            index += 1;
            continue;
        }

        if options_open && text.starts_with('-') && text != "-" {
            if !text.starts_with("--")
                && let Some(cluster) = text.strip_prefix('-')
            {
                let mut cluster_chars = cluster.chars().peekable();
                while let Some(flag) = cluster_chars.next() {
                    match flag {
                        'n' => {
                            null_input = true;
                        }
                        'f' => {
                            filter_from_file = true;
                            consumed_filter = true;
                            if cluster_chars.peek().is_none() {
                                pending_args = Some(PendingOptionArgs::Skip(1));
                            }
                            break;
                        }
                        'L' => {
                            if cluster_chars.peek().is_none() {
                                pending_args = Some(PendingOptionArgs::Skip(1));
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }

            match text.as_str() {
                "-n" | "--null-input" => {
                    null_input = true;
                }
                "-f" | "--from-file" => {
                    filter_from_file = true;
                    consumed_filter = true;
                    pending_args = Some(PendingOptionArgs::Skip(1));
                }
                "--arg" | "--argjson" => {
                    pending_args = Some(PendingOptionArgs::Skip(2));
                }
                "--rawfile" | "--slurpfile" | "--argfile" => {
                    pending_args = Some(PendingOptionArgs::NamedFileSource { seen_name: false });
                }
                "--indent" => {
                    pending_args = Some(PendingOptionArgs::Skip(1));
                }
                "-L" | "--library-path" => {
                    pending_args = Some(PendingOptionArgs::Skip(1));
                }
                "--args" | "--jsonargs" => {
                    positional_args_mode = true;
                }
                _ if text.starts_with("--from-file=") => {
                    filter_from_file = true;
                    consumed_filter = true;
                }
                _ if text.starts_with("--library-path=")
                    || text.starts_with("--arg=")
                    || text.starts_with("--argjson=")
                    || text.starts_with("--args=")
                    || text.starts_with("--jsonargs=") => {}
                _ => {}
            }
            index += 1;
            continue;
        }

        options_open = false;
        if !consumed_filter && !filter_from_file {
            consumed_filter = true;
            index += 1;
            continue;
        }

        if !null_input && !positional_args_mode {
            operands.push(*word);
        }
        index += 1;
    }

    operands
}

fn grep_file_operand_words<'a>(args: &[&'a Word], source: &str) -> Vec<&'a Word> {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;
    let mut explicit_pattern_source = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;

        if text == "--only-matching"
            || text == "--fixed-strings"
            || matches!(
                text.as_str(),
                "--basic-regexp" | "--extended-regexp" | "--perl-regexp"
            )
        {
            index += 1;
            continue;
        }

        if text == "--regexp" || text == "--file" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        if text.starts_with("--regexp=") || text.starts_with("--file=") {
            explicit_pattern_source = true;
            index += 1;
            continue;
        }

        if text.starts_with("--") {
            index += if grep_long_option_takes_argument(text.as_str())
                && args.get(index + 1).is_some()
            {
                2
            } else {
                1
            };
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == 'e' {
                explicit_pattern_source = true;
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }

            if flag == 'f' {
                explicit_pattern_source = true;
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }

            if grep_option_takes_argument(flag) {
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    if !explicit_pattern_source && args.get(index).is_some() {
        index += 1;
    }

    args.get(index..).unwrap_or(&[]).to_vec()
}

fn grep_pattern_fact<'a>(
    word: &'a Word,
    source: &str,
    source_kind: GrepPatternSourceKind,
) -> GrepPatternFact<'a> {
    grep_prefixed_pattern_fact(word, source, 0, source_kind)
}

fn grep_prefixed_pattern_fact<'a>(
    word: &'a Word,
    source: &str,
    prefix_len: usize,
    source_kind: GrepPatternSourceKind,
) -> GrepPatternFact<'a> {
    let static_text = static_word_text(word, source)
        .and_then(|text| text.get(prefix_len..).map(str::to_owned))
        .map(String::into_boxed_str);
    let starts_with_glob_style_star = static_text
        .as_deref()
        .is_some_and(|text| text.starts_with('*') || text == "^*");

    GrepPatternFact {
        word,
        static_text,
        source_kind,
        starts_with_glob_style_star,
    }
}

fn grep_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'A' | 'B' | 'C' | 'D' | 'd' | 'e' | 'f' | 'm')
}

fn grep_long_option_takes_argument(option: &str) -> bool {
    let Some(name) = option.strip_prefix("--") else {
        return false;
    };
    if name.contains('=') {
        return false;
    }

    matches!(
        name,
        "after-context"
            | "before-context"
            | "binary-files"
            | "context"
            | "devices"
            | "directories"
            | "exclude"
            | "exclude-dir"
            | "exclude-from"
            | "file"
            | "group-separator"
            | "include"
            | "label"
            | "max-count"
            | "regexp"
    )
}

fn parse_ps_command(args: &[&Word], source: &str) -> PsCommandFacts {
    let mut has_pid_selector = false;
    let mut pending_option_arg = false;
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if let Some(expects_argument) = dynamic_ps_pid_selector(word, source) {
                has_pid_selector = true;
                pending_option_arg = expects_argument;
                index += 1;
                continue;
            }

            if word_starts_with_literal_dash(word, source) {
                pending_option_arg = true;
                index += 1;
                continue;
            }

            if pending_option_arg {
                pending_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            break;
        }

        if matches!(text.as_str(), "p" | "q") {
            has_pid_selector = true;
            pending_option_arg = true;
            index += 1;
            continue;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_option_arg {
                pending_option_arg = false;
                index += 1;
                continue;
            }

            if text != "-" && ps_bare_pid_selector(text.as_str()) {
                has_pid_selector = true;
                index += 1;
                continue;
            }

            if ps_bsd_option_cluster(text.as_str()) {
                index += 1;
                continue;
            }

            break;
        }

        pending_option_arg = false;

        if text == "-p"
            || text == "-q"
            || text == "--pid"
            || text == "--ppid"
            || text == "--quick-pid"
        {
            has_pid_selector = true;
            pending_option_arg = true;
            index += 1;
            continue;
        }

        if text.starts_with("--pid=")
            || text.starts_with("--ppid=")
            || text.starts_with("--quick-pid=")
            || (text.starts_with("-p") && text.len() > 2)
            || (text.starts_with("-q") && text.len() > 2)
        {
            has_pid_selector = true;
            index += 1;
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'p' || flag == 'q' {
                has_pid_selector = true;
            }

            if ps_option_takes_argument(flag) {
                if chars.peek().is_none() {
                    pending_option_arg = true;
                }
                break;
            }
        }

        index += 1;
    }

    PsCommandFacts { has_pid_selector }
}

fn dynamic_ps_pid_selector(word: &Word, source: &str) -> Option<bool> {
    let prefix = leading_literal_word_prefix(word, source);
    if prefix.is_empty() {
        return None;
    }

    let has_attached_value = word.span.slice(source).len() > prefix.len();

    match prefix.as_str() {
        "p" | "q" | "-p" | "-q" | "--pid" | "--ppid" | "--quick-pid" => Some(!has_attached_value),
        _ if prefix.starts_with("--pid=")
            || prefix.starts_with("--ppid=")
            || prefix.starts_with("--quick-pid=")
            || (prefix.starts_with("-p") && prefix.len() > 2)
            || (prefix.starts_with("-q") && prefix.len() > 2) =>
        {
            Some(false)
        }
        _ => None,
    }
}

fn ps_bare_pid_selector(text: &str) -> bool {
    text.split(',')
        .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn ps_bsd_option_cluster(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|ch| {
            matches!(
                ch,
                'A' | 'a'
                    | 'C'
                    | 'c'
                    | 'E'
                    | 'e'
                    | 'f'
                    | 'g'
                    | 'h'
                    | 'j'
                    | 'l'
                    | 'L'
                    | 'M'
                    | 'm'
                    | 'r'
                    | 'S'
                    | 'T'
                    | 'u'
                    | 'v'
                    | 'w'
                    | 'X'
                    | 'x'
            )
        })
}

fn ps_option_takes_argument(flag: char) -> bool {
    matches!(
        flag,
        'C' | 'G' | 'N' | 'O' | 'U' | 'g' | 'o' | 'p' | 'q' | 't' | 'u'
    )
}

fn option_takes_argument(flag: char) -> bool {
    matches!(flag, 'a' | 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
}

#[derive(Debug, Clone, Copy)]
enum ShBuiltinOptionPolicy {
    Any,
    AllowOnly(&'static str),
}

fn first_nonportable_sh_builtin_option_span(
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<Span> {
    match normalized.effective_name.as_deref()? {
        "read" => first_nonportable_sh_option_span_in_words(
            normalized.body_args(),
            source,
            ShBuiltinOptionPolicy::AllowOnly("r"),
        ),
        "export" => normalized
            .declaration
            .as_ref()
            .and_then(|declaration| {
                matches!(declaration.kind, command::DeclarationKind::Export).then(|| ())
            })
            .map_or_else(
                || {
                    first_nonportable_sh_option_span_in_words(
                        normalized.body_args(),
                        source,
                        ShBuiltinOptionPolicy::AllowOnly("p"),
                    )
                },
                |_| {
                    let operands = normalized
                        .declaration
                        .as_ref()
                        .expect("checked export declaration")
                        .operands;
                    first_nonportable_sh_export_option_span(operands, source)
                },
            ),
        "ulimit" => first_nonportable_sh_option_span_in_words(
            normalized.body_args(),
            source,
            ShBuiltinOptionPolicy::AllowOnly("f"),
        ),
        "printf" | "trap" | "type" | "wait" => first_nonportable_sh_option_span_in_words(
            normalized.body_args(),
            source,
            ShBuiltinOptionPolicy::Any,
        ),
        _ => None,
    }
}

fn first_nonportable_sh_export_option_span(operands: &[DeclOperand], source: &str) -> Option<Span> {
    for operand in operands {
        match operand {
            DeclOperand::Flag(word) => {
                let text = static_word_text(word, source)?;

                if text == "--" {
                    return None;
                }

                if !text.starts_with('-') || text == "-" {
                    return None;
                }

                if sh_builtin_option_word_is_portable(
                    text.as_str(),
                    ShBuiltinOptionPolicy::AllowOnly("p"),
                ) {
                    continue;
                }

                return Some(word.span);
            }
            DeclOperand::Dynamic(word) => {
                return word_starts_with_literal_dash(word, source).then_some(word.span);
            }
            DeclOperand::Name(_) | DeclOperand::Assignment(_) => return None,
        }
    }

    None
}

fn first_nonportable_sh_option_span_in_words(
    args: &[&Word],
    source: &str,
    policy: ShBuiltinOptionPolicy,
) -> Option<Span> {
    for word in args {
        let Some(text) = static_word_text(word, source) else {
            return word_starts_with_literal_dash(word, source).then_some(word.span);
        };

        if text == "--" {
            return None;
        }

        if !text.starts_with('-') || text == "-" {
            return None;
        }

        if sh_builtin_option_word_is_portable(text.as_str(), policy) {
            continue;
        }

        return Some(word.span);
    }

    None
}

fn sh_builtin_option_word_is_portable(text: &str, policy: ShBuiltinOptionPolicy) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };

    if flags.is_empty() {
        return false;
    }

    match policy {
        ShBuiltinOptionPolicy::Any => false,
        ShBuiltinOptionPolicy::AllowOnly(allowed_flags) => {
            flags.chars().all(|flag| allowed_flags.contains(flag))
        }
    }
}

fn printf_format_word<'a>(args: &[&'a Word], source: &str) -> Option<&'a Word> {
    let mut index = 0usize;

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    if let Some(option) = args
        .get(index)
        .and_then(|word| static_word_text(word, source))
    {
        if option == "-v" {
            index += 2;
        } else if option.starts_with("-v") && option.len() > 2 {
            index += 1;
        }
    }

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    args.get(index).copied()
}

fn printf_format_word_has_literal_percent(word: &Word, source: &str) -> bool {
    word_parts_have_literal_percent(&word.parts, source)
}

fn word_parts_have_literal_percent(parts: &[WordPartNode], source: &str) -> bool {
    parts
        .iter()
        .any(|part| word_part_has_literal_percent(part, source))
}

fn word_part_has_literal_percent(part: &WordPartNode, source: &str) -> bool {
    match &part.kind {
        WordPart::Literal(text) => text.as_str(source, part.span).contains('%'),
        WordPart::ZshQualifiedGlob(_) => part.span.slice(source).contains('%'),
        WordPart::SingleQuoted { value, .. } => value.slice(source).contains('%'),
        WordPart::DoubleQuoted { parts, .. } => word_parts_have_literal_percent(parts, source),
        WordPart::Variable(_)
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
        | WordPart::Transformation { .. } => false,
    }
}

fn printf_uses_q_format(word: &Word, source: &str) -> bool {
    let Some(text) = static_word_text(word, source) else {
        return false;
    };

    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }

        index += 1;
        if index >= bytes.len() {
            break;
        }

        if bytes[index] == b'%' {
            index += 1;
            continue;
        }

        while index < bytes.len() && matches!(bytes[index], b'-' | b'+' | b' ' | b'#' | b'0') {
            index += 1;
        }

        if index < bytes.len() && bytes[index] == b'*' {
            index += 1;
        } else {
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
        }

        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            if index < bytes.len() && bytes[index] == b'*' {
                index += 1;
            } else {
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }

        if index + 1 < bytes.len()
            && ((bytes[index] == b'h' && bytes[index + 1] == b'h')
                || (bytes[index] == b'l' && bytes[index + 1] == b'l'))
        {
            index += 2;
        } else if index < bytes.len()
            && matches!(bytes[index], b'h' | b'l' | b'j' | b'z' | b't' | b'L')
        {
            index += 1;
        }

        if index < bytes.len() && bytes[index] == b'q' {
            return true;
        }
    }

    false
}

fn parse_unset_command<'a>(args: &[&'a Word], source: &str) -> UnsetCommandFacts<'a> {
    let mut function_mode = false;
    let mut parsing_options = true;
    let mut options_parseable = true;
    let mut operands = Vec::new();
    let mut operand_facts = Vec::new();
    let mut prefix_match_operand_spans = Vec::new();

    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if parsing_options {
                options_parseable = false;
                parsing_options = false;
            }

            collect_word_prefix_match_spans(word, &mut prefix_match_operand_spans);
            operands.push(*word);
            operand_facts.push(parse_unset_operand_fact(word, source));
            continue;
        };

        if parsing_options {
            if text == "--" {
                parsing_options = false;
                continue;
            }

            if text.starts_with('-') && text != "-" {
                if text[1..].chars().any(|flag| flag == 'f') {
                    function_mode = true;
                }
                continue;
            }

            parsing_options = false;
        }

        collect_word_prefix_match_spans(word, &mut prefix_match_operand_spans);
        operands.push(*word);
        operand_facts.push(parse_unset_operand_fact(word, source));
    }

    UnsetCommandFacts {
        function_mode,
        operand_words: operands.into_boxed_slice(),
        operand_facts: operand_facts.into_boxed_slice(),
        prefix_match_operand_spans: prefix_match_operand_spans.into_boxed_slice(),
        options_parseable,
    }
}

fn parse_unset_operand_fact<'a>(word: &'a Word, source: &str) -> UnsetOperandFact<'a> {
    UnsetOperandFact {
        word,
        array_subscript: parse_unset_array_subscript(word.span.slice(source)),
    }
}

fn parse_unset_array_subscript(text: &str) -> Option<UnsetArraySubscriptFact> {
    let (name, key_with_bracket) = text.split_once('[')?;
    let key = key_with_bracket.strip_suffix(']')?;
    is_shell_variable_name(name).then(|| UnsetArraySubscriptFact {
        name: Name::from(name),
        key_contains_quote: key.chars().any(|ch| ch == '\'' || ch == '"'),
    })
}

fn collect_word_prefix_match_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_prefix_match_spans(&word.parts, spans);
}

fn collect_prefix_match_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_prefix_match_spans(parts, spans),
            WordPart::PrefixMatch { .. } => spans.push(part.span),
            WordPart::Parameter(parameter)
                if matches!(
                    parameter.bourne(),
                    Some(BourneParameterExpansion::PrefixMatch { .. })
                ) =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

fn parse_find_execdir_shell_command(
    shell_name: Option<&str>,
    args: &[&Word],
    source: &str,
) -> Option<FindExecDirCommandFacts> {
    if !matches!(shell_name, Some("sh" | "bash" | "dash" | "ksh")) {
        return None;
    }

    let shell_command_spans = args
        .windows(2)
        .filter_map(|pair| {
            let flag = static_word_text(pair[0], source)?;
            if !shell_flag_contains_command_string(flag.as_str()) {
                return None;
            }
            let script = pair[1];
            script
                .span
                .slice(source)
                .contains("{}")
                .then_some(script.span)
        })
        .collect::<Vec<_>>();

    (!shell_command_spans.is_empty()).then_some(FindExecDirCommandFacts {
        shell_command_spans: shell_command_spans.into_boxed_slice(),
    })
}

fn parse_find_exec_argument_word_spans(args: &[&Word], source: &str) -> Vec<Span> {
    let semicolon_terminator_index = args
        .iter()
        .position(|word| matches!(static_word_text(word, source).as_deref(), Some(";" | "\\;")));
    let plus_terminator_index = args
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            (static_word_text(word, source).as_deref() == Some("+")).then_some(index)
        })
        .next_back();
    let terminator_index = semicolon_terminator_index.or(plus_terminator_index);

    args.iter()
        .take(terminator_index.unwrap_or(args.len()))
        .map(|word| word.span)
        .collect()
}

fn parse_find_command(args: &[&Word], source: &str) -> FindCommandFacts {
    let mut has_print0 = false;
    let mut or_without_grouping_spans = Vec::new();
    let mut glob_pattern_operand_spans = Vec::new();
    let mut group_stack = vec![FindGroupState::default()];
    let mut pending_argument: Option<FindPendingArgument> = None;

    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if let Some(state) = pending_argument {
                if state.expects_pattern_operand()
                    && !span::word_unquoted_glob_pattern_spans(word, source).is_empty()
                {
                    glob_pattern_operand_spans.push(word.span);
                }
                pending_argument = state.after_consuming_dynamic();
            }
            continue;
        };

        if let Some(state) = pending_argument {
            if state.expects_pattern_operand()
                && !span::word_unquoted_glob_pattern_spans(word, source).is_empty()
            {
                glob_pattern_operand_spans.push(word.span);
            }
            pending_argument = state.after_consuming(text.as_str());
            continue;
        }

        if text == "-print0" {
            has_print0 = true;
        }

        if is_find_group_open_token(text.as_str()) {
            group_stack.push(FindGroupState::default());
            continue;
        }

        if is_find_group_close_token(text.as_str()) {
            if let Some(child) = (group_stack.len() > 1).then(|| group_stack.pop()).flatten() {
                group_stack
                    .last_mut()
                    .expect("group stack retains the root frame")
                    .incorporate_group(child, &mut or_without_grouping_spans);
            }
            continue;
        }

        let state = group_stack
            .last_mut()
            .expect("group stack retains the root frame");

        if is_find_or_token(text.as_str()) {
            state.note_or();
            continue;
        }

        if is_find_and_token(text.as_str()) {
            state.note_and();
            continue;
        }

        if is_find_branch_action_token(text.as_str()) {
            state.note_action(
                word.span,
                is_find_reportable_action_token(text.as_str()),
                &mut or_without_grouping_spans,
            );
            pending_argument = find_pending_argument(text.as_str());
            continue;
        }

        if is_find_predicate_token(text.as_str()) {
            state.note_predicate();
            pending_argument = find_pending_argument(text.as_str());
        }
    }

    FindCommandFacts {
        has_print0,
        or_without_grouping_spans: or_without_grouping_spans.into_boxed_slice(),
        glob_pattern_operand_spans: glob_pattern_operand_spans.into_boxed_slice(),
    }
}

#[derive(Debug, Clone, Copy)]
enum FindPendingArgument {
    Words {
        remaining: usize,
        pattern_operand: bool,
    },
    UntilExecTerminator,
}

impl FindPendingArgument {
    fn after_consuming(self, token: &str) -> Option<Self> {
        match self {
            Self::Words {
                remaining,
                pattern_operand: _,
            } => remaining.checked_sub(1).and_then(|next| {
                (next > 0).then_some(Self::Words {
                    remaining: next,
                    pattern_operand: false,
                })
            }),
            Self::UntilExecTerminator => {
                (!matches!(token, ";" | "\\;" | "+")).then_some(Self::UntilExecTerminator)
            }
        }
    }

    fn after_consuming_dynamic(self) -> Option<Self> {
        match self {
            Self::Words {
                remaining,
                pattern_operand: _,
            } => remaining.checked_sub(1).and_then(|next| {
                (next > 0).then_some(Self::Words {
                    remaining: next,
                    pattern_operand: false,
                })
            }),
            Self::UntilExecTerminator => Some(Self::UntilExecTerminator),
        }
    }

    fn expects_pattern_operand(self) -> bool {
        matches!(
            self,
            Self::Words {
                pattern_operand: true,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FindGroupState {
    saw_or: bool,
    saw_action_before_current_branch: bool,
    current_branch_has_predicate: bool,
    current_branch_has_explicit_and: bool,
    has_any_predicate: bool,
    has_any_action: bool,
    first_action_span_for_parent: Option<Span>,
}

impl FindGroupState {
    fn current_branch_can_bind_action(&self) -> bool {
        !self.current_branch_has_explicit_and && (self.current_branch_has_predicate || self.saw_or)
    }

    fn note_or(&mut self) {
        self.saw_or = true;
        self.current_branch_has_predicate = false;
        self.current_branch_has_explicit_and = false;
    }

    fn note_and(&mut self) {
        self.current_branch_has_explicit_and = true;
    }

    fn note_predicate(&mut self) {
        self.current_branch_has_predicate = true;
        self.has_any_predicate = true;
    }

    fn note_action(&mut self, span: Span, reportable: bool, spans: &mut Vec<Span>) {
        if reportable
            && self.saw_or
            && !self.saw_action_before_current_branch
            && self.current_branch_can_bind_action()
        {
            spans.push(span);
        }

        if reportable
            && self.first_action_span_for_parent.is_none()
            && self.current_branch_can_bind_action()
        {
            self.first_action_span_for_parent = Some(span);
        }

        self.saw_action_before_current_branch = true;
        self.has_any_action = true;
    }

    fn incorporate_group(&mut self, child: Self, spans: &mut Vec<Span>) {
        if child.has_any_predicate {
            self.note_predicate();
        }

        if let Some(span) = child.first_action_span_for_parent {
            self.note_action(span, true, spans);
            return;
        }

        if child.has_any_action {
            self.saw_action_before_current_branch = true;
            self.has_any_action = true;
        }
    }
}

fn is_find_group_open_token(token: &str) -> bool {
    matches!(token, "(" | "\\(" | "-(")
}

fn is_find_group_close_token(token: &str) -> bool {
    matches!(token, ")" | "\\)" | "-)")
}

fn is_find_or_token(token: &str) -> bool {
    matches!(token, "-o" | "-or")
}

fn is_find_and_token(token: &str) -> bool {
    matches!(token, "-a" | "-and" | ",")
}

fn is_find_action_token(token: &str) -> bool {
    matches!(
        token,
        "-delete"
            | "-exec"
            | "-execdir"
            | "-ok"
            | "-okdir"
            | "-print"
            | "-print0"
            | "-printf"
            | "-ls"
            | "-fls"
            | "-fprint"
            | "-fprint0"
            | "-fprintf"
    )
}

fn is_find_branch_action_token(token: &str) -> bool {
    is_find_reportable_action_token(token) || matches!(token, "-prune" | "-quit")
}

fn is_find_reportable_action_token(token: &str) -> bool {
    is_find_action_token(token)
}

fn find_pending_argument(token: &str) -> Option<FindPendingArgument> {
    match token {
        "-fls" | "-fprint" | "-fprint0" | "-printf" => Some(FindPendingArgument::Words {
            remaining: 1,
            pattern_operand: false,
        }),
        "-fprintf" => Some(FindPendingArgument::Words {
            remaining: 2,
            pattern_operand: false,
        }),
        "-exec" | "-execdir" | "-ok" | "-okdir" => Some(FindPendingArgument::UntilExecTerminator),
        "-amin" | "-anewer" | "-atime" | "-cmin" | "-cnewer" | "-context" | "-fstype" | "-gid"
        | "-group" | "-ilname" | "-iname" | "-inum" | "-ipath" | "-iregex" | "-links"
        | "-lname" | "-maxdepth" | "-mindepth" | "-mmin" | "-mtime" | "-name" | "-newer"
        | "-path" | "-perm" | "-regex" | "-samefile" | "-size" | "-type" | "-uid" | "-used"
        | "-user" | "-wholename" | "-iwholename" | "-xtype" | "-files0-from" => {
            Some(FindPendingArgument::Words {
                remaining: 1,
                pattern_operand: is_find_pattern_predicate_token(token),
            })
        }
        token if token.starts_with("-newer") && token.len() > "-newer".len() => {
            Some(FindPendingArgument::Words {
                remaining: 1,
                pattern_operand: false,
            })
        }
        _ => None,
    }
}

fn is_find_pattern_predicate_token(token: &str) -> bool {
    matches!(
        token,
        "-name"
            | "-iname"
            | "-path"
            | "-ipath"
            | "-regex"
            | "-iregex"
            | "-lname"
            | "-ilname"
            | "-wholename"
            | "-iwholename"
    )
}

fn is_find_predicate_token(token: &str) -> bool {
    token.starts_with('-')
        && !is_find_branch_action_token(token)
        && !is_find_or_token(token)
        && !is_find_and_token(token)
        && !is_find_group_open_token(token)
        && !is_find_group_close_token(token)
        && !matches!(token, "-not")
}

fn shell_flag_contains_command_string(flag: &str) -> bool {
    let Some(cluster) = flag.strip_prefix('-') else {
        return false;
    };
    !cluster.is_empty()
        && !cluster.starts_with('-')
        && cluster.bytes().all(shell_short_flag_is_clusterable)
        && cluster.bytes().any(|byte| byte == b'c')
}

fn shell_short_flag_is_clusterable(flag: u8) -> bool {
    matches!(
        flag,
        b'a' | b'b'
            | b'c'
            | b'e'
            | b'f'
            | b'h'
            | b'i'
            | b'k'
            | b'l'
            | b'm'
            | b'n'
            | b'p'
            | b'r'
            | b's'
            | b't'
            | b'u'
            | b'v'
            | b'x'
    )
}

fn parse_set_command(args: &[&Word], source: &str) -> SetCommandFacts {
    let mut errexit_change = None;
    let mut errtrace_change = None;
    let mut pipefail_change = None;
    let mut resets_positional_parameters = false;
    let mut errtrace_option_spans = Vec::new();
    let mut pipefail_option_spans = Vec::new();
    let mut flags_without_prefix_spans = Vec::new();
    let mut index = 0usize;

    if args.len() >= 2
        && let Some(first_word) = args.first().copied()
        && classify_word(first_word, source).quote == WordQuote::Unquoted
        && let Some(first_text) = static_word_text(first_word, source)
        && first_text != "--"
        && !first_text.starts_with('-')
        && !first_text.starts_with('+')
        && is_shell_variable_name(first_text.as_str())
    {
        flags_without_prefix_spans.push(first_word.span);
    }

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            resets_positional_parameters = true;
            break;
        };

        if text == "--" {
            resets_positional_parameters = true;
            break;
        }

        match text.as_str() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if text.starts_with('+') {
                    resets_positional_parameters = true;
                }
                let Some(name_word) = args.get(index + 1) else {
                    break;
                };
                let Some(name) = static_word_text(name_word, source) else {
                    break;
                };

                if name == "errexit" {
                    errexit_change = Some(enable);
                } else if name == "errtrace" {
                    errtrace_change = Some(enable);
                    errtrace_option_spans.push(name_word.span);
                } else if name == "pipefail" {
                    pipefail_change = Some(enable);
                    pipefail_option_spans.push(name_word.span);
                }
                index += 2;
                continue;
            }
            _ => {}
        }

        let Some(flags) = text.strip_prefix('-').or_else(|| text.strip_prefix('+')) else {
            resets_positional_parameters = true;
            break;
        };
        if flags.is_empty() {
            break;
        }
        if text.starts_with('+') {
            resets_positional_parameters = true;
        }

        if flags.chars().any(|flag| flag == 'e') {
            errexit_change = Some(text.starts_with('-'));
        }
        if flags.chars().any(|flag| flag == 'E') {
            errtrace_change = Some(text.starts_with('-'));
            errtrace_option_spans.push(word.span);
        }

        if flags.chars().any(|flag| flag == 'o') {
            let enable = text.starts_with('-');
            let Some(name_word) = args.get(index + 1) else {
                break;
            };
            let Some(name) = static_word_text(name_word, source) else {
                break;
            };

            if name == "errtrace" {
                errtrace_change = Some(enable);
                errtrace_option_spans.push(name_word.span);
            } else if name == "pipefail" {
                pipefail_change = Some(enable);
                pipefail_option_spans.push(name_word.span);
            }
            index += 2;
            continue;
        }

        index += 1;
    }

    SetCommandFacts {
        errexit_change,
        errtrace_change,
        pipefail_change,
        resets_positional_parameters,
        errtrace_option_spans: errtrace_option_spans.into_boxed_slice(),
        pipefail_option_spans: pipefail_option_spans.into_boxed_slice(),
        flags_without_prefix_spans: flags_without_prefix_spans.into_boxed_slice(),
    }
}

fn parse_directory_change_command(
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<DirectoryChangeCommandFacts> {
    let kind = match normalized.effective_name.as_deref() {
        Some("cd") => DirectoryChangeCommandKind::Cd,
        Some("pushd") => DirectoryChangeCommandKind::Pushd,
        Some("popd") => DirectoryChangeCommandKind::Popd,
        _ => return None,
    };

    let target = normalized
        .body_args()
        .first()
        .and_then(|word| static_word_text(word, source));

    let plain_directory_stack_marker = matches!(
        kind,
        DirectoryChangeCommandKind::Cd | DirectoryChangeCommandKind::Pushd
    ) && normalized.wrappers.is_empty()
        && target
            .as_ref()
            .is_some_and(|target| is_directory_stack_marker(target.as_str()));

    let manual_restore_candidate = kind == DirectoryChangeCommandKind::Cd
        && target
            .as_ref()
            .is_some_and(|target| matches!(target.as_str(), ".." | "-"));

    Some(DirectoryChangeCommandFacts {
        kind,
        plain_directory_stack_marker,
        manual_restore_candidate,
    })
}

fn is_directory_stack_marker(target: &str) -> bool {
    if !target.is_empty() && target.chars().all(|ch| ch == '/') {
        return true;
    }

    let mut saw_segment = false;
    for segment in target.split('/').filter(|segment| !segment.is_empty()) {
        saw_segment = true;
        if segment != "." && segment != ".." {
            return false;
        }
    }

    saw_segment
}

fn is_configure_command_name(name: &str) -> bool {
    name == "configure" || name.ends_with("/configure")
}

fn parse_configure_command(args: &[&Word], source: &str) -> ConfigureCommandFacts {
    let misspelled_option_spans = args
        .iter()
        .filter_map(|word| {
            let option_name = configure_option_name(word, source)?;
            configure_option_misspelling(option_name.as_str())
                .and_then(|_| configure_option_name_span(word, source, option_name.as_str()))
        })
        .collect::<Vec<_>>();

    ConfigureCommandFacts {
        misspelled_option_spans: misspelled_option_spans.into_boxed_slice(),
    }
}

fn configure_option_name(word: &Word, source: &str) -> Option<String> {
    let prefix = leading_literal_word_prefix(word, source);
    let option_name = prefix
        .split_once('=')
        .map_or(prefix.as_str(), |(name, _)| name);
    option_name
        .starts_with("--")
        .then(|| option_name.to_owned())
}

fn configure_option_name_span(word: &Word, source: &str, option_name: &str) -> Option<Span> {
    let text = word.span.slice(source);
    let relative_start = text.find(option_name)?;
    let start = word.span.start.advanced_by(&text[..relative_start]);
    let end = start.advanced_by(option_name);
    Some(Span::from_positions(start, end))
}

fn configure_option_misspelling(option_name: &str) -> Option<&'static str> {
    match option_name {
        "--with-optmizer" => Some("--with-optimizer"),
        "--without-optmizer" => Some("--without-optimizer"),
        "--enable-optmizer" => Some("--enable-optimizer"),
        "--disable-optmizer" => Some("--disable-optimizer"),
        _ => None,
    }
}

pub fn leading_literal_word_prefix(word: &Word, source: &str) -> String {
    let mut prefix = String::new();
    collect_leading_literal_word_parts(&word.parts, source, &mut prefix);
    prefix
}

fn collect_leading_literal_word_parts(
    parts: &[WordPartNode],
    source: &str,
    prefix: &mut String,
) -> bool {
    for part in parts {
        if !collect_leading_literal_word_part(part, source, prefix) {
            return false;
        }
    }
    true
}

fn collect_leading_literal_word_part(
    part: &WordPartNode,
    source: &str,
    prefix: &mut String,
) -> bool {
    match &part.kind {
        WordPart::Literal(text) => {
            prefix.push_str(text.as_str(source, part.span));
            true
        }
        WordPart::SingleQuoted { value, .. } => {
            prefix.push_str(value.slice(source));
            true
        }
        WordPart::DoubleQuoted { parts, .. } => {
            collect_leading_literal_word_parts(parts, source, prefix)
        }
        _ => false,
    }
}

fn parse_wait_command(args: &[&Word], source: &str) -> WaitCommandFacts {
    let mut option_spans = Vec::new();
    let mut index = 0;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if text.starts_with('-') && text != "-" {
            option_spans.push(word.span);
            index += 1;
            if wait_option_consumes_argument(&text) {
                index += 1;
            }
            continue;
        }

        break;
    }

    WaitCommandFacts {
        option_spans: option_spans.into_boxed_slice(),
    }
}

fn parse_ln_command<'a>(args: &[&'a Word], source: &str) -> Option<LnCommandFacts<'a>> {
    let mut index = 0usize;
    let mut saw_symbolic_flag = false;
    let mut target_directory_mode = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            match long {
                "symbolic" => saw_symbolic_flag = true,
                "target-directory" => {
                    target_directory_mode = true;
                    index += 1;
                    args.get(index)?;
                }
                "suffix" => {
                    index += 1;
                    args.get(index)?;
                }
                "backup"
                | "directory"
                | "force"
                | "interactive"
                | "logical"
                | "no-dereference"
                | "no-target-directory"
                | "physical"
                | "relative"
                | "verbose" => {}
                _ if long.starts_with("target-directory=") => {
                    target_directory_mode = true;
                }
                _ if long.starts_with("suffix=") => {}
                _ => return None,
            }

            index += 1;
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            match flag {
                's' => saw_symbolic_flag = true,
                't' => {
                    target_directory_mode = true;
                    if chars.peek().is_none() {
                        index += 1;
                        args.get(index)?;
                    }
                    break;
                }
                'S' => {
                    if chars.peek().is_none() {
                        index += 1;
                        args.get(index)?;
                    }
                    break;
                }
                'b' | 'd' | 'f' | 'F' | 'i' | 'L' | 'n' | 'P' | 'r' | 'T' | 'v' => {}
                _ => return None,
            }
        }

        index += 1;
    }

    if !saw_symbolic_flag {
        return None;
    }

    let operands = &args[index..];
    if operands.is_empty() {
        return None;
    }

    Some(LnCommandFacts {
        symlink_target_words: if target_directory_mode {
            operands.to_vec().into_boxed_slice()
        } else {
            vec![operands[0]].into_boxed_slice()
        },
    })
}

fn wait_option_consumes_argument(text: &str) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };
    let Some(p_index) = flags.find('p') else {
        return false;
    };

    p_index + 1 == flags.len()
}

fn parse_mapfile_command(args: &[&Word], source: &str) -> MapfileCommandFacts {
    let mut input_fd = Some(0);
    let mut index = 0;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" || !text.starts_with('-') || text == "-" || text.starts_with("--") {
            break;
        }

        let flags = &text[1..];
        let mut recognized = true;

        for (offset, flag) in flags.char_indices() {
            if !matches!(flag, 't' | 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's') {
                recognized = false;
                break;
            }

            if !mapfile_option_takes_argument(flag) {
                continue;
            }

            let remainder = &flags[offset + flag.len_utf8()..];
            let argument = if remainder.is_empty() {
                index += 1;
                args.get(index)
                    .and_then(|next| static_word_text(next, source))
            } else {
                Some(remainder.to_owned())
            };

            if flag == 'u' {
                input_fd = argument.and_then(|value| value.parse::<i32>().ok());
            }

            break;
        }

        if !recognized {
            break;
        }

        index += 1;
    }

    MapfileCommandFacts { input_fd }
}

fn mapfile_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's')
}

fn parse_xargs_command(args: &[&Word], source: &str) -> XargsCommandFacts {
    let mut uses_null_input = false;
    let mut inline_replace_option_spans = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                break;
            }
            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            if long == "null" {
                uses_null_input = true;
            }

            let consume_next_argument = xargs_long_option_requires_separate_argument(long);
            index += 1;
            if consume_next_argument {
                index += 1;
            }
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == '0' {
                uses_null_input = true;
            }
            if flag == 'i' {
                inline_replace_option_spans.push(word.span);
            }

            match xargs_short_option_argument_style(flag) {
                XargsShortOptionArgumentStyle::None => {}
                XargsShortOptionArgumentStyle::OptionalInlineOnly => break,
                XargsShortOptionArgumentStyle::Required => {
                    if chars.peek().is_none() {
                        consume_next_argument = true;
                    }
                    break;
                }
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    XargsCommandFacts {
        uses_null_input,
        inline_replace_option_spans: inline_replace_option_spans.into_boxed_slice(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XargsShortOptionArgumentStyle {
    None,
    OptionalInlineOnly,
    Required,
}

fn xargs_short_option_argument_style(flag: char) -> XargsShortOptionArgumentStyle {
    match flag {
        'e' | 'i' | 'l' => XargsShortOptionArgumentStyle::OptionalInlineOnly,
        'a' | 'E' | 'I' | 'L' | 'n' | 'P' | 's' | 'd' => XargsShortOptionArgumentStyle::Required,
        _ => XargsShortOptionArgumentStyle::None,
    }
}

fn xargs_long_option_requires_separate_argument(option: &str) -> bool {
    if option.contains('=') {
        return false;
    }

    matches!(
        option,
        "arg-file"
            | "delimiter"
            | "max-args"
            | "max-chars"
            | "max-lines"
            | "max-procs"
            | "process-slot-var"
    )
}

fn parse_expr_command(args: &[&Word], source: &str) -> Option<ExprCommandFacts> {
    Some(ExprCommandFacts {
        uses_arithmetic_operator: !expr_uses_string_form(args, source),
        uses_substr_string_form: expr_uses_substr_string_form(args, source),
    })
}

fn expr_uses_string_form(args: &[&Word], source: &str) -> bool {
    matches!(
        args.first()
            .and_then(|word| static_word_text(word, source))
            .as_deref(),
        Some("length" | "index" | "match" | "substr")
    ) || args
        .get(1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        .is_some_and(|text| matches!(text, ":" | "=" | "!=" | "<" | ">" | "<=" | ">=" | "=="))
}

fn expr_uses_substr_string_form(args: &[&Word], source: &str) -> bool {
    args.first()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("substr")
}

fn parse_exit_command<'a>(command: &'a Command, source: &str) -> Option<ExitCommandFacts<'a>> {
    let Command::Builtin(BuiltinCommand::Exit(exit)) = command else {
        return None;
    };
    let Some(status_word) = exit.code.as_ref() else {
        return Some(ExitCommandFacts {
            status_word: None,
            is_numeric_literal: false,
            status_is_static: false,
            status_has_literal_content: false,
        });
    };
    let status_text = static_word_text(status_word, source);

    Some(ExitCommandFacts {
        status_word: Some(status_word),
        is_numeric_literal: status_text.as_deref().is_some_and(|text| {
            !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
        }),
        status_is_static: status_text.is_some(),
        status_has_literal_content: word_contains_literal_content(status_word, source),
    })
}

fn word_contains_literal_content(word: &Word, source: &str) -> bool {
    word_parts_contain_literal_content(&word.parts, source)
}

fn word_parts_contain_literal_content(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => !text.as_str(source, part.span).is_empty(),
        WordPart::SingleQuoted { value, .. } => !value.slice(source).is_empty(),
        WordPart::DoubleQuoted { parts, .. } => word_parts_contain_literal_content(parts, source),
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
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
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

fn detect_sudo_family_invoker(
    command: &Command,
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<SudoFamilyInvoker> {
    let Command::Simple(command) = command else {
        return None;
    };
    let body_start = normalized.body_span.start.offset;
    let scan_all_words = normalized.body_words.is_empty();

    std::iter::once(&command.name)
        .chain(command.args.iter())
        // Unresolved sudo-family wrappers intentionally keep the wrapper marker
        // even when there is no statically known inner command.
        .take_while(|word| scan_all_words || word.span.start.offset < body_start)
        .filter_map(|word| static_word_text(word, source))
        .filter_map(|word| match word.as_str() {
            "sudo" => Some(SudoFamilyInvoker::Sudo),
            "doas" => Some(SudoFamilyInvoker::Doas),
            "run0" => Some(SudoFamilyInvoker::Run0),
            _ => None,
        })
        .last()
}

fn trap_action_word<'a>(command: &'a Command, source: &str) -> Option<&'a Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    if static_word_text(&command.name, source).as_deref() != Some("trap") {
        return None;
    }

    let mut start = 0usize;

    if let Some(first) = command
        .args
        .first()
        .and_then(|word| static_word_text(word, source))
    {
        match first.as_str() {
            "-p" | "-l" => return None,
            "--" => start = 1,
            _ => {}
        }
    }

    let action = command.args.get(start)?;
    command.args.get(start + 1)?;
    Some(action)
}

fn collect_binding_values<'a>(
    command: &'a Command,
    semantic: &SemanticModel,
    source: &str,
    binding_values: &mut FxHashMap<BindingId, BindingValueFact<'a>>,
) {
    let assignments = match command {
        Command::Simple(simple) if simple.name.span.slice(source).is_empty() => &simple.assignments,
        Command::Builtin(_) | Command::Decl(_) => query::command_assignments(command),
        Command::Simple(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => &[],
    };

    for assignment in assignments {
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        if let Some(binding_id) = binding_value_definition_id_for_name(
            semantic,
            &assignment.target.name,
            assignment.target.name_span,
        ) {
            binding_values.insert(binding_id, BindingValueFact::scalar(word));
        }
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        if let Some(binding_id) = binding_value_definition_id_for_name(
            semantic,
            &assignment.target.name,
            assignment.target.name_span,
        ) {
            binding_values.insert(binding_id, BindingValueFact::scalar(word));
        }
    }

    match command {
        Command::Compound(CompoundCommand::For(command)) => {
            let Some(words) = &command.words else {
                return;
            };
            let values = words.iter().collect::<Vec<_>>().into_boxed_slice();
            for target in &command.targets {
                if let Some(name) = &target.name
                    && let Some(binding_id) =
                        binding_value_definition_id_for_name(semantic, name, target.span)
                {
                    binding_values.insert(
                        binding_id,
                        BindingValueFact::from_loop_words(values.clone()),
                    );
                }
            }
        }
        Command::Compound(CompoundCommand::Foreach(command)) => {
            if let Some(binding_id) = binding_value_definition_id_for_name(
                semantic,
                &command.variable,
                command.variable_span,
            ) {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::from_loop_words(
                        command.words.iter().collect::<Vec<_>>().into_boxed_slice(),
                    ),
                );
            }
        }
        Command::Compound(CompoundCommand::Select(command)) => {
            if let Some(binding_id) = binding_value_definition_id_for_name(
                semantic,
                &command.variable,
                command.variable_span,
            ) {
                binding_values.insert(
                    binding_id,
                    BindingValueFact::from_loop_words(
                        command.words.iter().collect::<Vec<_>>().into_boxed_slice(),
                    ),
                );
            }
        }
        _ => {}
    }
}

fn binding_value_definition_id_for_name(
    semantic: &SemanticModel,
    name: &Name,
    span: Span,
) -> Option<BindingId> {
    semantic
        .bindings_for(name)
        .iter()
        .rev()
        .copied()
        .find(|binding_id| semantic.binding(*binding_id).span == span)
}

fn binding_value_visible_id_for_name(
    semantic: &SemanticModel,
    name: &Name,
    span: Span,
) -> Option<BindingId> {
    semantic
        .visible_binding(name, span)
        .map(|binding| binding.id)
}

fn annotate_conditional_assignment_shortcuts<'a>(
    semantic: &SemanticModel,
    lists: &[ListFact<'a>],
    binding_values: &mut FxHashMap<BindingId, BindingValueFact<'a>>,
) {
    for list in lists.iter().filter(|list| {
        list.mixed_short_circuit_kind() == Some(MixedShortCircuitKind::AssignmentTernary)
    }) {
        for segment in list.segments() {
            let Some(target) = segment.assignment_target() else {
                continue;
            };
            let Some(span) = segment.assignment_span() else {
                continue;
            };
            let Some(binding_id) =
                binding_value_visible_id_for_name(semantic, &Name::from(target), span)
            else {
                continue;
            };
            if let Some(binding_value) = binding_values.get_mut(&binding_id) {
                binding_value.mark_conditional_assignment_shortcut();
            }
        }
    }
}

fn collect_broken_assoc_key_spans(command: &Command, source: &str, spans: &mut Vec<Span>) {
    for assignment in query::command_assignments(command) {
        collect_broken_assoc_key_spans_in_assignment(assignment, source, spans);
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        collect_broken_assoc_key_spans_in_assignment(assignment, source, spans);
    }
}

fn collect_broken_assoc_key_spans_in_assignment(
    assignment: &Assignment,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let AssignmentValue::Compound(array) = &assignment.value else {
        return;
    };
    if array.kind == ArrayKind::Indexed {
        return;
    }

    for element in &array.elements {
        let ArrayElem::Sequential(word) = element else {
            continue;
        };
        if has_unclosed_assoc_key_prefix(word, source) {
            spans.push(word.span);
        }
    }
}

fn has_unclosed_assoc_key_prefix(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    if !text.starts_with('[') {
        return false;
    }

    let mut excluded = expansion_part_spans(word);
    excluded.sort_by_key(|span| span.start.offset);
    let mut excluded = excluded.into_iter().peekable();

    let mut bracket_depth = 0_i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut saw_equals = false;

    for (offset, ch) in text.char_indices() {
        let absolute_offset = word.span.start.offset + offset;
        while matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.end.offset
        ) {
            excluded.next();
        }
        if matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.start.offset && absolute_offset < span.end.offset
        ) {
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => {
                escaped = true;
                continue;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                continue;
            }
            '"' if !in_single => {
                in_double = !in_double;
                continue;
            }
            _ => {}
        }

        if in_single || in_double {
            continue;
        }

        match ch {
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => {
                bracket_depth -= 1;
                if bracket_depth == 0 {
                    return false;
                }
            }
            '=' if bracket_depth > 0 => saw_equals = true,
            _ => {}
        }
    }

    saw_equals
}

fn collect_comma_array_assignment_spans(command: &Command, source: &str, spans: &mut Vec<Span>) {
    for assignment in query::command_assignments(command) {
        if let Some(span) = comma_array_assignment_span(assignment, source) {
            spans.push(span);
        }
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        if let Some(span) = comma_array_assignment_span(assignment, source) {
            spans.push(span);
        }
    }
}

fn collect_ifs_literal_backslash_assignment_value_spans(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for assignment in query::command_assignments(command) {
        if let Some(span) = ifs_literal_backslash_assignment_value_span(assignment, source) {
            spans.push(span);
        }
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        if let Some(span) = ifs_literal_backslash_assignment_value_span(assignment, source) {
            spans.push(span);
        }
    }
}

fn ifs_literal_backslash_assignment_value_span(
    assignment: &Assignment,
    source: &str,
) -> Option<Span> {
    if assignment.target.name.as_str() != "IFS" {
        return None;
    }

    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };

    static_word_text(word, source)
        .is_some_and(|text| text.contains('\\'))
        .then_some(word.span)
}

fn comma_array_assignment_span(assignment: &Assignment, source: &str) -> Option<Span> {
    let AssignmentValue::Compound(array) = &assignment.value else {
        return None;
    };
    if !array_value_has_unquoted_comma(array, source) {
        return None;
    }

    compound_assignment_paren_span(assignment, source)
}

fn array_value_has_unquoted_comma(array: &shuck_ast::ArrayExpr, source: &str) -> bool {
    let _ = source;
    array
        .elements
        .iter()
        .any(|element| element.value().has_top_level_unquoted_comma())
}

fn compound_assignment_paren_span(assignment: &Assignment, source: &str) -> Option<Span> {
    let AssignmentValue::Compound(_) = &assignment.value else {
        return None;
    };

    let text = assignment.span.slice(source);
    let equals = text.find('=')?;
    let open = text[equals + 1..].find('(')? + equals + 1;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }

    let start = assignment.span.start.advanced_by(&text[..open]);
    let end = assignment
        .span
        .start
        .advanced_by(&text[..close + ')'.len_utf8()]);
    Some(Span::from_positions(start, end))
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => builtin_span(command),
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(command) => compound_span(command),
        Command::Function(command) => command.span,
        Command::AnonymousFunction(command) => command.span,
    }
}

fn command_lookup_kind(command: &Command) -> CommandLookupKind {
    match command {
        Command::Simple(_) => CommandLookupKind::Simple,
        Command::Builtin(command) => CommandLookupKind::Builtin(match command {
            BuiltinCommand::Break(_) => BuiltinLookupKind::Break,
            BuiltinCommand::Continue(_) => BuiltinLookupKind::Continue,
            BuiltinCommand::Return(_) => BuiltinLookupKind::Return,
            BuiltinCommand::Exit(_) => BuiltinLookupKind::Exit,
        }),
        Command::Decl(_) => CommandLookupKind::Decl,
        Command::Binary(_) => CommandLookupKind::Binary,
        Command::Compound(command) => CommandLookupKind::Compound(match command {
            CompoundCommand::If(_) => CompoundLookupKind::If,
            CompoundCommand::For(_) => CompoundLookupKind::For,
            CompoundCommand::Repeat(_) => CompoundLookupKind::Repeat,
            CompoundCommand::Foreach(_) => CompoundLookupKind::Foreach,
            CompoundCommand::ArithmeticFor(_) => CompoundLookupKind::ArithmeticFor,
            CompoundCommand::While(_) => CompoundLookupKind::While,
            CompoundCommand::Until(_) => CompoundLookupKind::Until,
            CompoundCommand::Case(_) => CompoundLookupKind::Case,
            CompoundCommand::Select(_) => CompoundLookupKind::Select,
            CompoundCommand::Subshell(_) => CompoundLookupKind::Subshell,
            CompoundCommand::BraceGroup(_) => CompoundLookupKind::BraceGroup,
            CompoundCommand::Arithmetic(_) => CompoundLookupKind::Arithmetic,
            CompoundCommand::Time(_) => CompoundLookupKind::Time,
            CompoundCommand::Conditional(_) => CompoundLookupKind::Conditional,
            CompoundCommand::Coproc(_) => CompoundLookupKind::Coproc,
            CompoundCommand::Always(_) => CompoundLookupKind::Always,
        }),
        Command::Function(_) => CommandLookupKind::Function,
        Command::AnonymousFunction(_) => CommandLookupKind::AnonymousFunction,
    }
}

fn command_id_for_command(
    command: &Command,
    command_ids_by_span: &CommandLookupIndex,
) -> Option<CommandId> {
    command_ids_by_span
        .get(&FactSpan::new(command_span(command)))
        .and_then(|entries| {
            let kind = command_lookup_kind(command);
            entries
                .iter()
                .find(|entry| entry.kind == kind)
                .map(|entry| entry.id)
        })
}

fn command_fact<'a>(commands: &'a [CommandFact<'a>], id: CommandId) -> &'a CommandFact<'a> {
    &commands[id.index()]
}

fn sort_and_dedup_spans(spans: &mut Vec<Span>) {
    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
}

fn trim_trailing_whitespace_span(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let trimmed = text.trim_end_matches(char::is_whitespace);
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}

fn command_fact_for_command<'a>(
    command: &Command,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<&'a CommandFact<'a>> {
    command_id_for_command(command, command_ids_by_span).map(|id| command_fact(commands, id))
}

fn command_fact_for_stmt<'a>(
    stmt: &Stmt,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<&'a CommandFact<'a>> {
    command_fact_for_command(&stmt.command, commands, command_ids_by_span)
}

fn builtin_span(command: &BuiltinCommand) -> Span {
    match command {
        BuiltinCommand::Break(command) => command.span,
        BuiltinCommand::Continue(command) => command.span,
        BuiltinCommand::Return(command) => command.span,
        BuiltinCommand::Exit(command) => command.span,
    }
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            commands.span
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_ast::{BinaryOp, CommandSubstitutionSyntax, ConditionalBinaryOp, Name};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
    use shuck_semantic::{BindingAttributes, SemanticModel};

    use super::{
        CommandId, ConditionalNodeFact, ConditionalOperatorFamily, GrepPatternSourceKind,
        LinterFacts, SimpleTestOperatorFamily, SimpleTestShape, SimpleTestSyntax,
        SubstitutionHostKind, SudoFamilyInvoker, WordFactHostKind,
    };
    use crate::facts::PositionalParameterFragmentKind;
    use crate::rules::common::command::WrapperKind;
    use crate::rules::common::expansion::{ExpansionContext, SubstitutionOutputIntent};
    use crate::{ShellDialect, classify_file_context};

    fn with_facts_dialect(
        source: &str,
        path: Option<&Path>,
        parse_dialect: ParseShellDialect,
        shell: ShellDialect,
        visit: impl FnOnce(&shuck_parser::parser::ParseResult, &LinterFacts<'_>),
    ) {
        let output = Parser::with_dialect(source, parse_dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, path, shell);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        visit(&output, &facts);
    }

    fn with_facts(
        source: &str,
        path: Option<&Path>,
        visit: impl FnOnce(&shuck_parser::parser::ParseResult, &LinterFacts<'_>),
    ) {
        with_facts_dialect(
            source,
            path,
            ParseShellDialect::Bash,
            ShellDialect::Bash,
            visit,
        );
    }

    #[test]
    fn commented_continuation_facts_ignore_plain_comment_only_lines() {
        let source = "#!/bin/sh\necho hello \\\n  #world\n  foo\n";

        with_facts(source, None, |_, facts| {
            assert!(facts.commented_continuation_comment_spans().is_empty());
        });
    }

    #[test]
    fn commented_continuation_facts_anchor_at_comment_backslash() {
        let source = "#!/bin/sh\necho hello \\\n  #world \\\n  foo\n";

        with_facts(source, None, |_, facts| {
            let spans = facts.commented_continuation_comment_spans();
            assert_eq!(spans.len(), 1);
            assert_eq!(spans[0].start.line, 3);
            assert_eq!(spans[0].start.column, 11);
            assert_eq!(spans[0].start, spans[0].end);
            assert_eq!(
                &source[spans[0].start.offset - 1..spans[0].start.offset],
                "\\"
            );
        });
    }

    #[test]
    fn builds_command_facts_for_wrapped_and_nested_commands() {
        let source = "#!/bin/bash\ncommand printf '%s\\n' \"$(echo hi)\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let outer = facts
            .structural_commands()
            .find(|fact| fact.effective_name_is("printf"))
            .expect("expected structural printf fact");

        assert_eq!(facts.commands().len(), 2);
        assert_eq!(outer.literal_name(), Some("command"));
        assert_eq!(outer.effective_name(), Some("printf"));
        assert_eq!(outer.wrappers(), &[WrapperKind::Command]);
        assert!(!outer.is_nested_word_command());
        assert_eq!(
            outer
                .options()
                .printf()
                .and_then(|printf| printf.format_word)
                .map(|word| word.span.slice(source)),
            Some("'%s\\n'")
        );

        let nested = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("echo"))
            .expect("expected nested echo fact");
        assert!(nested.is_nested_word_command());
        assert_eq!(
            facts
                .commands()
                .iter()
                .map(|fact| fact.id())
                .collect::<Vec<_>>(),
            vec![CommandId::new(0), CommandId::new(1)]
        );
    }

    #[test]
    fn function_header_fact_span_in_source_stops_at_header() {
        let source = "#!/bin/bash\nfunction wrapped()\n{\n  printf '%s\\n' hi\n}\n";

        with_facts(source, None, |_, facts| {
            let header = facts
                .function_headers()
                .first()
                .expect("expected function header fact");

            assert_eq!(
                header.span_in_source(source).slice(source),
                "function wrapped()"
            );
        });
    }

    #[test]
    fn function_header_fact_tracks_binding_scope_and_call_arity() {
        let source = "#!/bin/sh\ngreet ok\ngreet() { echo \"$1\"; }\ngreet\n";

        with_facts(source, None, |_, facts| {
            let header = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(name, _)| name == "greet")
                })
                .expect("expected greet header fact");

            assert!(header.binding_id().is_some());
            assert!(header.function_scope().is_some());
            assert_eq!(header.call_arity().call_count(), 2);
            assert_eq!(header.call_arity().min_arg_count(), Some(0));
            assert_eq!(header.call_arity().max_arg_count(), Some(1));
            assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
            assert_eq!(
                header.call_arity().zero_arg_call_spans()[0].slice(source),
                "greet"
            );
        });
    }

    #[test]
    fn function_header_fact_tracks_call_arity_inside_parameter_expansion_defaults() {
        let source = "\
#!/usr/bin/env bash
GetBuildVersion() {
  local build_revision=\"${1}\"
  printf '%s\n' \"$build_revision\"
}
BUILD_VERSION=\"${BUILD_VERSION:-\"$(GetBuildVersion \"${BUILD_REVISION}\")\"}\"
";

        with_facts(source, None, |_, facts| {
            let header = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(name, _)| name == "GetBuildVersion")
                })
                .expect("expected GetBuildVersion header fact");

            assert_eq!(header.call_arity().call_count(), 1);
            assert_eq!(header.call_arity().min_arg_count(), Some(1));
            assert_eq!(header.call_arity().max_arg_count(), Some(1));
            assert!(header.call_arity().zero_arg_call_spans().is_empty());
        });
    }

    #[test]
    fn function_header_fact_ignores_wrapper_resolved_targets_for_call_arity() {
        let source = "\
#!/usr/bin/env bash
greet() { printf '%s\n' \"$1\"; }
command greet ok
greet
";

        with_facts(source, None, |_, facts| {
            let header = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(name, _)| name == "greet")
                })
                .expect("expected greet header fact");

            assert_eq!(header.call_arity().call_count(), 1);
            assert_eq!(header.call_arity().min_arg_count(), Some(0));
            assert_eq!(header.call_arity().max_arg_count(), Some(0));
            assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
            assert_eq!(
                header.call_arity().zero_arg_call_spans()[0].slice(source),
                "greet"
            );
        });
    }

    #[test]
    fn function_header_fact_counts_quoted_static_calls_in_call_arity() {
        let source = "\
#!/usr/bin/env bash
greet() { printf '%s\n' \"$1\"; }
\"greet\" ok
greet
";

        with_facts(source, None, |_, facts| {
            let header = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(name, _)| name == "greet")
                })
                .expect("expected greet header fact");

            assert_eq!(header.call_arity().call_count(), 2);
            assert_eq!(header.call_arity().min_arg_count(), Some(0));
            assert_eq!(header.call_arity().max_arg_count(), Some(1));
            assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
            assert_eq!(
                header.call_arity().zero_arg_call_spans()[0].slice(source),
                "greet"
            );
        });
    }

    #[test]
    fn function_header_fact_tracks_zero_arg_backtick_calls() {
        let source = "\
#!/bin/sh
greet() { printf '%s\n' \"$1\"; }
value=\"`greet`\"
";

        with_facts(source, None, |_, facts| {
            let header = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(name, _)| name == "greet")
                })
                .expect("expected greet header fact");

            assert_eq!(header.call_arity().call_count(), 1);
            assert_eq!(header.call_arity().min_arg_count(), Some(0));
            assert_eq!(header.call_arity().max_arg_count(), Some(0));
            assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
            assert_eq!(
                header.call_arity().zero_arg_call_spans()[0].slice(source),
                "greet"
            );
        });
    }

    #[test]
    fn builds_function_style_spans() {
        let source = "\
#!/bin/bash
f() [[ -n \"$x\" ]]
g() {
  if cond; then
    false
    return $?
  fi
}
h() {
  if cond; then
    false
    return $?
  fi
  echo done
}
i() {
  false
  return $? 5
}
j() {
  false
  x=1 return $?
}
k() {
  false
  return $? >out
}
l() {
  ! {
    false
    return $?
  }
}
m() {
  {
    false
    return $?
  } &
}
n() {
  if cond; then
    false
  fi
  return $?
}
o() {
  : | false
  return $?
}
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .function_body_without_braces_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["[[ -n \"$x\" ]]"]
            );
            assert_eq!(
                facts
                    .redundant_return_status_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$?", "$?", "$?"]
            );
        });
    }

    #[test]
    fn exposes_structural_commands_and_id_lookups() {
        let source = "#!/bin/bash\necho \"$(printf x)\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let structural = facts
            .structural_commands()
            .map(|fact| fact.effective_or_literal_name().unwrap().to_owned())
            .collect::<Vec<_>>();
        let all = facts
            .commands()
            .iter()
            .map(|fact| fact.effective_or_literal_name().unwrap().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(structural, vec!["echo"]);
        assert_eq!(all, vec!["echo", "printf"]);

        let echo_id = facts
            .command_id_for_stmt(&output.file.body[0])
            .expect("expected command id for top-level stmt");
        assert_eq!(echo_id, CommandId::new(0));
        assert_eq!(
            facts.command(echo_id).effective_or_literal_name(),
            Some("echo")
        );
        assert_eq!(
            facts.command_id_for_command(&output.file.body[0].command),
            Some(echo_id)
        );
    }

    #[test]
    fn precomputes_innermost_command_ids_for_nested_offsets() {
        let source = "#!/bin/bash\necho \"$(printf '%s' \"$(uname)\")\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let outer_id = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_or_literal_name() == Some("echo"))
            .map(|fact| fact.id())
            .expect("expected outer echo command");
        let middle_id = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_or_literal_name() == Some("printf"))
            .map(|fact| fact.id())
            .expect("expected nested printf command");
        let inner_id = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_or_literal_name() == Some("uname"))
            .map(|fact| fact.id())
            .expect("expected nested uname command");

        let command_ids_by_offset = super::build_innermost_command_ids_by_offset(
            facts.commands(),
            vec![
                source.find("echo").expect("expected echo offset"),
                source.find("printf").expect("expected printf offset"),
                source.find("uname").expect("expected uname offset"),
            ],
        );

        assert_eq!(
            super::precomputed_command_id_for_offset(
                &command_ids_by_offset,
                source.find("echo").expect("expected echo offset"),
            ),
            Some(outer_id)
        );
        assert_eq!(
            super::precomputed_command_id_for_offset(
                &command_ids_by_offset,
                source.find("printf").expect("expected printf offset"),
            ),
            Some(middle_id)
        );
        assert_eq!(
            super::precomputed_command_id_for_offset(
                &command_ids_by_offset,
                source.find("uname").expect("expected uname offset"),
            ),
            Some(inner_id)
        );
    }

    #[test]
    fn tracks_nested_commands_inside_if_and_elif_conditions() {
        let source = "\
#!/bin/bash
if \"$( [[ -f if_path ]] )\"; then
  :
elif \"$( [[ -f elif_path ]] )\"; then
  :
fi
";

        with_facts(source, None, |_, facts| {
            let if_nested = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ -f if_path ]]")
                .expect("expected nested if condition command");
            assert!(if_nested.scope_read_source_words().is_empty());
            assert!(facts.is_if_condition_command(if_nested.id()));
            assert!(!facts.is_elif_condition_command(if_nested.id()));

            let elif_nested = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ -f elif_path ]]")
                .expect("expected nested elif condition command");
            assert!(elif_nested.scope_read_source_words().is_empty());
            assert!(facts.is_elif_condition_command(elif_nested.id()));
        });
    }

    #[test]
    fn includes_nested_jq_file_operands_in_writer_scope_reads() {
        let source = "#!/bin/bash\ncat <<<$(jq '.dns={}' \"$cfg\") >\"$cfg\"\n";

        with_facts(source, None, |_, facts| {
            let jq = facts
                .commands()
                .iter()
                .find(|fact| fact.effective_name_is("jq"))
                .expect("expected nested jq command");
            assert_eq!(
                jq.file_operand_words()
                    .iter()
                    .map(|word| word.span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["\"$cfg\""]
            );

            let cat = facts
                .structural_commands()
                .find(|fact| fact.effective_name_is("cat"))
                .expect("expected structural cat command");
            assert_eq!(
                cat.scope_read_source_words()
                    .iter()
                    .map(|fact| fact.word().span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["\"$cfg\""]
            );
        });
    }

    #[test]
    fn parses_jq_input_modes_into_file_operands() {
        let source = "\
#!/bin/bash
jq --args '$ARGS.positional[0]' \"$cfg\"
jq --jsonargs '$ARGS.positional[0]' \"$cfg\"
jq --indent 2 --args '$ARGS.positional[0]' \"$cfg\"
jq --rawfile cfg \"$cfg\" '.dns=$cfg'
jq --slurpfile cfg \"$cfg\" '.dns=$cfg'
jq --argfile cfg \"$cfg\" '.dns=$cfg'
jq -nc '.x=1' \"$cfg\"
jq -Lnewmods '.x=1' \"$cfg\"
";

        with_facts(source, None, |_, facts| {
            let jq_commands = facts
                .structural_commands()
                .filter(|fact| fact.effective_name_is("jq"))
                .collect::<Vec<_>>();

            assert_eq!(
                jq_commands
                    .iter()
                    .map(|command| {
                        command
                            .file_operand_words()
                            .iter()
                            .map(|word| word.span.slice(source))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
                vec![
                    Vec::<&str>::new(),
                    Vec::<&str>::new(),
                    Vec::<&str>::new(),
                    vec!["\"$cfg\""],
                    vec!["\"$cfg\""],
                    vec!["\"$cfg\""],
                    Vec::<&str>::new(),
                    vec!["\"$cfg\""],
                ]
            );
        });
    }

    #[test]
    fn tracks_nested_if_and_elif_conditions_inside_while_conditions() {
        let source = "\
#!/bin/bash
while if \"$( [[ -f if_path ]] )\"; then
  :
elif \"$( [[ -f elif_path ]] )\"; then
  :
fi; do
  :
done
";

        with_facts(source, None, |_, facts| {
            let if_nested = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ -f if_path ]]")
                .expect("expected nested if condition command");
            assert!(if_nested.scope_read_source_words().is_empty());
            assert!(facts.is_if_condition_command(if_nested.id()));
            assert!(!facts.is_elif_condition_command(if_nested.id()));

            let elif_nested = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ -f elif_path ]]")
                .expect("expected nested elif condition command");
            assert!(elif_nested.scope_read_source_words().is_empty());
            assert!(facts.is_elif_condition_command(elif_nested.id()));
        });
    }

    #[test]
    fn tracks_nested_while_and_until_conditions_inside_if_and_elif_conditions() {
        let source = "\
#!/bin/bash
if while [[ -f if_path ]]; do
  :
done; then
  :
elif until [[ -f elif_path ]]; do
  :
done; then
  :
fi
";

        with_facts(source, None, |_, facts| {
            let if_nested = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ -f if_path ]]")
                .expect("expected nested while condition command");
            assert!(if_nested.scope_read_source_words().is_empty());
            assert!(facts.is_if_condition_command(if_nested.id()));
            assert!(!facts.is_elif_condition_command(if_nested.id()));

            let elif_nested = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ -f elif_path ]]")
                .expect("expected nested until condition command");
            assert!(elif_nested.scope_read_source_words().is_empty());
            assert!(facts.is_if_condition_command(elif_nested.id()));
            assert!(facts.is_elif_condition_command(elif_nested.id()));
        });
    }

    #[test]
    fn preserves_condition_related_span_outputs() {
        let source = "\
#!/bin/bash
if [[ -f foo ]]; then
  echo $?
elif [[ -f bar ]]; then
  echo $?
elif [ $? -eq 1 ]; then
  :
fi
while test -f baz; do
  echo $?
done
if [[ -n $mode ]]; then
  case $mode in
    foo) tend $? ;;
  esac
fi
if $(printf one); then
  :
fi
while $(printf two); do
  :
done
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .condition_command_substitution_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$(printf one)", "$(printf two)"]
            );
            assert_eq!(
                facts
                    .condition_status_capture_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$?", "$?", "$?", "$?", "$?"]
            );
        });
    }

    #[test]
    fn collects_c107_status_checks_in_reportable_test_contexts() {
        let source = "\
#!/bin/bash
run
if [ $? -ne 0 ]; then :; fi
[ $? -ne 0 ]
run && [ $? -eq 0 ]
run || [ $? -ne 0 ]
if (( $? != 0 )); then :; fi
while [[ $? -ne 0 ]]; do break; done
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .dollar_question_after_command_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$?", "$?", "$?", "$?", "$?", "$?"]
            );
        });
    }

    #[test]
    fn keeps_c107_off_plain_function_entry_checks() {
        let source = "\
#!/bin/bash
check_status() {
  if [ $? -ne 0 ]; then :; fi
  [ $? -ne 0 ]
  run && [ $? -ne 0 ]
}
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .dollar_question_after_command_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$?", "$?"]
            );
        });
    }

    #[test]
    fn keeps_getopts_case_facts_when_loop_body_has_nested_command_content() {
        let source = "\
#!/bin/bash
while getopts 'ab' opt; do
  echo \"$(printf warmup)\"
  case \"$opt\" in
    a)
      ;;
    b)
      echo \"$(printf body)\"
      ;;
  esac
done
";

        with_facts(source, None, |_, facts| {
            let [case] = facts.getopts_cases() else {
                panic!("expected one getopts case fact");
            };

            assert_eq!(
                case.case_span().slice(source).trim_end(),
                "case \"$opt\" in\n    a)\n      ;;\n    b)\n      echo \"$(printf body)\"\n      ;;\n  esac"
            );
            assert_eq!(
                case.handled_case_labels()
                    .iter()
                    .map(|label| label.label())
                    .collect::<Vec<_>>(),
                vec!['a', 'b']
            );
            assert!(case.missing_options().is_empty());
        });
    }

    #[test]
    fn indexes_scalar_bindings_from_assignments_and_declarations() {
        let source = "#!/bin/bash\nfoo=1\nprintf '%s\\n' \"$foo\"\nexport bar=2\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        match &output.file.body[0].command {
            shuck_ast::Command::Simple(_) => {}
            _ => panic!("expected simple command"),
        };
        let first_binding_id = semantic.bindings_for(&Name::from("foo"))[0];
        assert_eq!(
            facts
                .binding_value(first_binding_id)
                .and_then(|value| value.scalar_word())
                .map(|word| word.span.slice(source)),
            Some("1")
        );

        match &output.file.body[2].command {
            shuck_ast::Command::Decl(command) => match &command.operands[0] {
                shuck_ast::DeclOperand::Assignment(_) => {}
                _ => panic!("expected declaration assignment"),
            },
            _ => panic!("expected declaration command"),
        };
        let second_binding_id = semantic.bindings_for(&Name::from("bar"))[0];
        assert_eq!(
            facts
                .binding_value(second_binding_id)
                .and_then(|value| value.scalar_word())
                .map(|word| word.span.slice(source)),
            Some("2")
        );
    }

    #[test]
    fn indexes_loop_bindings_from_for_words() {
        let source = "#!/bin/bash\nfor i in 16 32 64; do printf '%s\\n' \"$i\"; done\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let loop_binding_span = match &output.file.body[0].command {
            shuck_ast::Command::Compound(shuck_ast::CompoundCommand::For(command)) => {
                command.targets[0].span
            }
            _ => panic!("expected for command"),
        };
        let loop_binding_id = semantic
            .visible_binding(&Name::from("i"), loop_binding_span)
            .expect("expected i loop binding")
            .id;

        assert_eq!(
            facts
                .binding_value(loop_binding_id)
                .and_then(|value| value.loop_words())
                .expect("expected loop binding values")
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["16", "32", "64"]
        );
    }

    #[test]
    fn marks_conditional_assignment_shortcuts_on_binding_values() {
        let source =
            "#!/bin/bash\ntrue && w='-w' || w=''\nif true; then flag='-f'; else flag=''; fi\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let shortcut_bindings = semantic
            .bindings_for(&Name::from("w"))
            .iter()
            .copied()
            .map(|binding_id| {
                facts
                    .binding_value(binding_id)
                    .expect("expected w binding value fact")
                    .conditional_assignment_shortcut()
            })
            .collect::<Vec<_>>();
        assert_eq!(shortcut_bindings, vec![true, true]);

        let flag_bindings = semantic
            .bindings_for(&Name::from("flag"))
            .iter()
            .copied()
            .map(|binding_id| {
                facts
                    .binding_value(binding_id)
                    .expect("expected flag binding value fact")
                    .conditional_assignment_shortcut()
            })
            .collect::<Vec<_>>();
        assert_eq!(flag_bindings, vec![false, false]);
    }

    #[test]
    fn ignores_command_prefix_assignments_when_indexing_binding_values() {
        let source = "\
#!/bin/bash
foo=stable
foo=ephemeral tool
printf '%s\\n' \"$foo\"
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let foo_bindings = semantic.bindings_for(&Name::from("foo"));
        assert_eq!(foo_bindings.len(), 1);
        assert_eq!(
            facts
                .binding_value(foo_bindings[0])
                .and_then(|value| value.scalar_word())
                .map(|word| word.span.slice(source)),
            Some("stable")
        );
    }

    #[test]
    fn declaration_assignment_values_attach_to_the_declared_binding() {
        let source = "\
#!/bin/bash
f() {
  (
    value=shadow
    local value=chosen
    printf '%s\\n' \"$value\"
  )
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let shadow_binding = semantic
            .bindings_for(&Name::from("value"))
            .iter()
            .copied()
            .find(|binding_id| semantic.binding(*binding_id).attributes.is_empty())
            .expect("expected subshell shadow binding");
        let local_binding = semantic
            .bindings_for(&Name::from("value"))
            .iter()
            .copied()
            .find(|binding_id| {
                semantic
                    .binding(*binding_id)
                    .attributes
                    .contains(BindingAttributes::LOCAL)
            })
            .expect("expected local declaration binding");

        assert_eq!(
            facts
                .binding_value(shadow_binding)
                .and_then(|value| value.scalar_word())
                .map(|word| word.span.slice(source)),
            Some("shadow")
        );
        assert_eq!(
            facts
                .binding_value(local_binding)
                .and_then(|value| value.scalar_word())
                .map(|word| word.span.slice(source)),
            Some("chosen")
        );
    }

    #[test]
    fn collects_plus_equals_assignment_spans() {
        let source = "\
#!/bin/sh
x+=64
arr+=(one two)
readonly r+=1
index[1+2]+=3
complex[$((i+=1))]+=x
(( i += 1 ))
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .plus_equals_assignment_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["x", "arr", "r", "index[1+2]", "complex[$((i+=1))]"]
            );
        });
    }

    #[test]
    fn collects_broken_assoc_key_spans_from_compound_array_assignments() {
        let source = "#!/bin/bash\ndeclare -A table=([left]=1 [right=2)\nother=([ok]=1 [broken=2)\ndeclare -A third=([$(echo ])=3)\ndeclare -A valid=([$(printf key)]=4)\ndeclare -a nums=([0]=1 [1=2)\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert_eq!(
            facts
                .broken_assoc_key_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[right=2", "[broken=2", "[$(echo ])=3"]
        );
    }

    #[test]
    fn collects_comma_array_assignment_spans_from_compound_values() {
        let source = "#!/bin/bash\na=(alpha,beta)\nb=(\"alpha,beta\")\nc=({x,y})\nd=([k]=v, [q]=w)\ne=(x,$y)\nf=(x\\, y)\ng=({$XDG_CONFIG_HOME,$HOME}/{alacritty,}/{.,}alacritty.ym?)\nh=(foo,{x,y},bar)\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert_eq!(
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "(alpha,beta)",
                "([k]=v, [q]=w)",
                "(x,$y)",
                "(x\\, y)",
                "(foo,{x,y},bar)"
            ]
        );
    }

    #[test]
    fn collects_ifs_literal_backslash_assignment_value_spans() {
        let source = "\
#!/bin/bash
IFS='\\n'
export IFS=\"x\\n\"
while IFS='\\ \\|\\ ' read -r serial board_serial; do
  :
done < /dev/null
declare IFS='prefix\\nsuffix'
IFS=$'\\n'
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .ifs_literal_backslash_assignment_value_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["'\\n'", "\"x\\n\"", "'\\ \\|\\ '", "'prefix\\nsuffix'"]
            );
        });
    }

    #[test]
    fn ignores_commas_after_even_backslashes_before_quote_regions() {
        let source = "#!/bin/bash\na=(x\\\\\",y\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_ansi_c_quoted_array_elements() {
        let source = "#!/bin/bash\na=($'a\\'b,c')\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_quoted_command_substitution_array_elements() {
        let source = "#!/bin/bash\nf() {\n\tlocal -a graphql_request=(\n\t\t-X POST\n\t\t-d \"$(\n\t\t\tcat <<-EOF | tr '\\n' ' '\n\t\t\t\t{\"query\":\"field, direction\"}\n\t\t\tEOF\n\t\t)\"\n\t)\n}\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_separator_started_command_substitution_comments() {
        let source =
            "#!/bin/bash\na=(\"$(printf '%s' x;# comment with ) and ,\nprintf '%s' y\n)\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_grouped_command_substitution_comments() {
        let source = "#!/bin/bash\na=(\"$( (# comment with )\nprintf %s 1,2\n) )\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_compact_grouped_command_substitution_comments() {
        let source = "#!/bin/bash\na=(\"$( (#comment with )\nprintf %s 1,2\n) )\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_command_substitution_case_patterns() {
        let source = "#!/bin/bash\na=(\"$(case $kind in\nalpha) printf %s 1,2 ;;\nesac)\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_piped_heredoc_command_substitution_array_elements() {
        let source = "#!/bin/bash\na=(\"$(cat <<EOF|tr '\\n' ' '\n{\"query\":\"field, direction\"}\nEOF\n)\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_parameter_expansions_with_right_parens_in_command_substitutions() {
        let source = "#!/bin/bash\na=($(printf %s ${x//foo/)},1))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_parameter_expansions_with_literal_braces() {
        let source = "#!/bin/bash\na=(${x/a,b/{})\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_parameter_expansions_with_ansi_c_single_quotes() {
        let source = "#!/bin/bash\na=(${x/$'a\\'b'/c,d})\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_case_pattern_comments_after_right_parens() {
        let source = "#!/bin/bash\na=($(case $kind in\na)# comment with esac )\nprintf %s 1,2 ;;\nesac\n))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_process_substitution_array_elements() {
        let source = "#!/bin/bash\na=(<(printf %s 1,2))\nb=(>(printf %s 3,4))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_comments_after_quoted_double_parens() {
        let source = "#!/bin/bash\na=($(printf '((' # comment with )\nprintf %s 1,2\n))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_arithmetic_shift_command_substitutions() {
        let source = "#!/bin/bash\na=($( ((x<<2))\nprintf %s 1,2\n))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_multiline_command_substitution_scanner_edge_cases() {
        let source = "\
#!/bin/bash
a=($(printf '((' # comment with )
printf %s 1,2
))
b=($( ((x<<2))
printf %s 3,4
))
c=($( (case $kind in
a) printf %s 5,6 ;;
esac
) ))
d=(\"$( (#comment with )
printf %s 7,8
) )\")
e=($(printf %s 9,10; echo case in))
f=($(printf %s $'a\\'b'; printf %s 11,12))
g=($(printf %s `echo foo)`; printf %s 13,14))
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_nested_case_patterns_in_command_substitutions() {
        let source = "#!/bin/bash\na=($( (case $kind in\na) printf %s 1,2 ;;\nesac\n) ))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_command_substitutions_with_plain_case_words() {
        let source = "#!/bin/bash\na=($(printf %s 1,2; echo case in))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_command_substitutions_with_ansi_c_single_quotes() {
        let source = "#!/bin/bash\na=($(printf %s $'a\\'b'; printf %s 1,2))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_command_substitutions_with_backticks() {
        let source = "#!/bin/bash\na=($(printf %s `echo foo)`; printf %s 1,2))\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_backticks_inside_parameter_expansions() {
        let source = "#!/bin/bash\na=(${x/`echo }`/a,b})\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_inside_process_substitutions_inside_parameter_expansions() {
        let source = "#!/bin/bash\na=(${x/<(echo })/foo,bar})\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_after_backticks_inside_parameter_expansions_in_command_substitutions() {
        let source = "#!/bin/bash\na=(\"$(printf %s ${x/`echo }`/foo)},1)\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_commas_after_process_substitutions_inside_parameter_expansions_in_command_substitutions()
     {
        let source = "#!/bin/bash\na=(\"$(printf %s ${x/<(echo })/foo)},1)\")\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(
            facts.comma_array_assignment_spans().is_empty(),
            "{:#?}",
            facts
                .comma_array_assignment_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn summarizes_command_options_and_invokers() {
        let source = "#!/bin/bash\nread -r name\necho -ne hi\necho '-I' hi\necho \"\\\\n\"\necho \\x41\necho \"prefix $VAR \\\\0 suffix\"\ncommand echo \\n\nsed 's/foo/bar/'\nsed -e 's/foo/bar/'\nsed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\1:'\nsed 's/[]\\[^$.*/]/\\\\&/g'\nsed 's/\\([/&]\\)/\\\\\\1/g'\nsed -n 's/foo/bar/p'\nsed --expression 's/foo/bar/'\nsed -r 's/foo/bar/'\nsed \\\"s/foo/bar/\\\"\ntr -ds a-z A-Z\ntr -- 'a-z' xyz\nprintf -v out \"$fmt\" value\nprintf '%q\\n' foo\nprintf '%*q\\n' 10 bar\nunset -f curl other\nfind . -print0 | xargs -0 rm\nfind . -type d -name CVS | xargs -iX rm -rf X\nfind . -type d -name CVS | xargs --replace rm -rf {}\nfind . -name a -o -name b -print\nfind . -name *.cfg\nfind . -name \"$prefix\"*.jar\nfind . -wholename */tmp/*\nfind . -name \\*.ignore\nfind . -type f*\nrm -rf \"$dir\"/*\nrm -rf \"$dir\"/sub/*\nrm -rf \"$dir\"/lib\nrm -rf \"$dir\"/*.log\nrm -rf \"$rootdir/$md_type/$to\"\nrm -rf \"$configdir/all/retroarch/$dir\"\nrm -rf \"$md_inst/\"*\nwait -n\nwait -- -n\ngrep -o content file | wc -l\nexit foo\nset -eEo pipefail\nset euox pipefail\n./configure --with-optmizer=${CFLAGS}\nconfigure \"--enable-optmizer=${CFLAGS}\"\n./configure --with-optimizer=${CFLAGS}\nps -p 1 -o comm=\nps p 123 -o comm=\nps -ef\ndoas printf '%s\\n' hi\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let read = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("read"))
            .expect("expected read fact");
        assert_eq!(
            read.options().read().map(|read| read.uses_raw_input),
            Some(true)
        );

        let echo = facts
            .commands()
            .iter()
            .find(|fact| {
                fact.effective_name_is("echo")
                    && fact
                        .options()
                        .echo()
                        .and_then(|echo| echo.portability_flag_word())
                        .is_some()
            })
            .and_then(|fact| fact.options().echo())
            .expect("expected echo facts");
        assert_eq!(
            echo.portability_flag_word()
                .map(|word| word.span.slice(source)),
            Some("-ne")
        );
        assert_eq!(
            facts
                .commands()
                .iter()
                .filter(|fact| fact.effective_name_is("echo"))
                .nth(1)
                .and_then(|fact| fact.options().echo())
                .and_then(|echo| echo.portability_flag_word())
                .map(|word| word.span.slice(source)),
            None
        );
        assert_eq!(
            facts
                .echo_backslash_escape_word_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"\\\\n\"", "\\x41", "\"prefix $VAR \\\\0 suffix\""]
        );

        let sed_commands = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("sed"))
            .collect::<Vec<_>>();
        assert_eq!(sed_commands.len(), 9);
        assert!(
            sed_commands[0]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            sed_commands[1]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            sed_commands[2]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            !sed_commands[3]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            !sed_commands[4]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            !sed_commands[5]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            !sed_commands[6]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            !sed_commands[7]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );
        assert!(
            !sed_commands[8]
                .options()
                .sed()
                .is_some_and(|sed| sed.has_single_substitution_script())
        );

        let tr = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("tr") && fact.options().tr().is_some())
            .and_then(|fact| fact.options().tr())
            .expect("expected tr facts");
        assert_eq!(
            tr.operand_words()
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a-z", "A-Z"]
        );
        let quoted_tr = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("tr"))
            .nth(1)
            .and_then(|fact| fact.options().tr())
            .expect("expected second tr facts");
        assert_eq!(
            quoted_tr
                .operand_words()
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'a-z'", "xyz"]
        );

        let printf = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("printf") && fact.options().printf().is_some())
            .expect("expected printf fact");
        assert_eq!(
            printf
                .options()
                .printf()
                .and_then(|printf| printf.format_word)
                .map(|word| word.span.slice(source)),
            Some("\"$fmt\"")
        );
        assert!(
            printf
                .options()
                .printf()
                .is_some_and(|printf| !printf.format_word_has_literal_percent)
        );
        assert!(
            !printf
                .options()
                .printf()
                .is_some_and(|printf| printf.uses_q_format)
        );

        let q_printf = facts
            .commands()
            .iter()
            .find(|fact| {
                fact.effective_name_is("printf")
                    && fact
                        .options()
                        .printf()
                        .is_some_and(|printf| printf.uses_q_format)
            })
            .and_then(|fact| fact.options().printf())
            .expect("expected q printf facts");
        assert!(q_printf.uses_q_format);
        assert_eq!(
            q_printf.format_word.map(|word| word.span.slice(source)),
            Some("'%q\\n'")
        );

        let star_q_printf = facts
            .commands()
            .iter()
            .find(|fact| {
                fact.effective_name_is("printf")
                    && fact
                        .options()
                        .printf()
                        .and_then(|printf| printf.format_word)
                        .map(|word| word.span.slice(source))
                        == Some("'%*q\\n'")
            })
            .and_then(|fact| fact.options().printf())
            .expect("expected star-width q printf facts");
        assert!(star_q_printf.uses_q_format);
        assert!(star_q_printf.format_word_has_literal_percent);

        let unset = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("unset"))
            .and_then(|fact| fact.options().unset())
            .expect("expected unset facts");
        assert!(unset.function_mode);
        assert!(unset.targets_function_name(source, "curl"));
        assert!(!unset.targets_function_name(source, "missing"));

        let find = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("find"))
            .and_then(|fact| fact.options().find())
            .expect("expected find facts");
        assert!(find.has_print0);
        let find_or_without_grouping_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("find"))
            .filter_map(|fact| fact.options().find())
            .flat_map(|find| find.or_without_grouping_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(find_or_without_grouping_spans, vec!["-print"]);
        let find_glob_pattern_operand_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("find"))
            .filter_map(|fact| fact.options().find())
            .flat_map(|find| find.glob_pattern_operand_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(
            find_glob_pattern_operand_spans,
            vec!["*.cfg", "\"$prefix\"*.jar", "*/tmp/*"]
        );

        let find_execdir = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
            .and_then(|fact| fact.options().find_execdir());
        assert!(
            find_execdir.is_none(),
            "fixture without execdir should not match"
        );

        let xargs = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("xargs"))
            .and_then(|fact| fact.options().xargs())
            .expect("expected xargs facts");
        assert!(xargs.uses_null_input);
        let inline_replace_option_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("xargs"))
            .filter_map(|fact| fact.options().xargs())
            .flat_map(|xargs| xargs.inline_replace_option_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(inline_replace_option_spans, vec!["-iX"]);

        let wait = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("wait") && fact.options().wait().is_some())
            .and_then(|fact| fact.options().wait())
            .expect("expected wait facts");
        assert_eq!(
            wait.option_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-n"]
        );

        let set = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("set"))
            .and_then(|fact| fact.options().set())
            .expect("expected set facts");
        assert_eq!(set.errexit_change, Some(true));
        assert_eq!(set.errtrace_change, Some(true));
        assert_eq!(set.pipefail_change, Some(true));
        assert_eq!(
            set.errtrace_option_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-eEo"]
        );
        assert_eq!(
            set.pipefail_option_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["pipefail"]
        );
        let set_without_prefix_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("set"))
            .filter_map(|fact| fact.options().set())
            .flat_map(|set| set.flags_without_prefix_spans().iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(
            set_without_prefix_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["euox"]
        );
        let configure_option_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().configure())
            .flat_map(|configure| configure.misspelled_option_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(
            configure_option_spans,
            vec!["--with-optmizer", "--enable-optmizer"]
        );
        let ps_pid_selector_flags = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("ps"))
            .filter_map(|fact| fact.options().ps().map(|ps| ps.has_pid_selector))
            .collect::<Vec<_>>();
        assert_eq!(ps_pid_selector_flags, vec![true, true, false]);
        let rm_spans = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().rm())
            .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(
            rm_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"$dir\"/*",
                "\"$dir\"/lib",
                "\"$rootdir/$md_type/$to\"",
                "\"$md_inst/\"*"
            ]
        );
        let grep = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("grep"))
            .and_then(|fact| fact.options().grep())
            .expect("expected grep facts");
        assert!(grep.uses_only_matching);
        assert!(!grep.uses_fixed_strings);
        assert_eq!(
            grep.patterns()
                .iter()
                .map(|pattern| pattern.span().slice(source))
                .collect::<Vec<_>>(),
            vec!["content"]
        );

        let exit = facts
            .commands()
            .iter()
            .find(|fact| fact.options().exit().is_some())
            .and_then(|fact| fact.options().exit())
            .expect("expected exit facts");
        assert_eq!(
            exit.status_word.map(|word| word.span.slice(source)),
            Some("foo")
        );
        assert!(exit.has_static_status());
        assert!(!exit.is_numeric_literal);
        assert!(exit.has_invalid_status_argument());

        let doas = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::SudoFamily))
            .and_then(|fact| fact.options().sudo_family())
            .expect("expected sudo-family facts");
        assert_eq!(doas.invoker, SudoFamilyInvoker::Doas);
    }

    #[test]
    fn tracks_printf_formats_with_and_without_literal_percents() {
        let source = "printf \"$fmt\" value\nprintf \"${left}${right}\" value\nprintf \"${fmt:-%s}\" value\nprintf \"$(echo %s)\" value\nprintf \"pre$foo\" value\nprintf \"%${width}s\\n\" value\nprintf \"${color}%s${reset}\" value\nprintf \"$fmt%s\" value\nprintf '%s\\n' value\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let printfs = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("printf"))
            .filter_map(|fact| fact.options().printf())
            .map(|printf| {
                (
                    printf
                        .format_word
                        .map(|word| word.span.slice(source))
                        .expect("expected format word"),
                    printf.format_word_has_literal_percent,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            printfs,
            vec![
                ("\"$fmt\"", false),
                ("\"${left}${right}\"", false),
                ("\"${fmt:-%s}\"", false),
                ("\"$(echo %s)\"", false),
                ("\"pre$foo\"", false),
                ("\"%${width}s\\n\"", true),
                ("\"${color}%s${reset}\"", true),
                ("\"$fmt%s\"", true),
                ("'%s\\n'", true),
            ]
        );
    }

    #[test]
    fn builds_echo_to_sed_substitution_spans() {
        let source = "\
#!/bin/bash
echo $value | sed 's/foo/bar/'
echo \"$value\" | sed 's/foo/bar/g'
echo \"$value\" | sed 's§foo§bar§'
echo ${items[@]} | sed -e 's/foo/bar/2'
result=$(echo \"$(printf %s foo)\" | sed 's/foo/bar/')
COMMAND=$(echo \"$COMMAND\" | sed \"s#\\(--appendconfig *[^ $]*\\)#\\1'|'$conf#\")
RUNTIME=$(echo $OUT | sed \"s|$OUT|\\$this_dir|g\")
escaped_hostname=$(echo \"$hostname\" | sed 's/[]\\[\\.^$*+?{}()|]/\\\\&/g')
value=$(sed 's/[\\.|$(){}?+*^]/\\\\&/g' <<<\"$value\")
echo \"$value\" | sed 's/a/b/' <<<\"$shadow\"
CFLAGS=\"`echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS//\\\"`\"
OPTFLAG=\"`echo \\\"$CFLAGS\\\" | sed 's/^.*\\(-O[^ ]\\).*$/\\1/'`\"
EC2_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\1:'`\"
ESCAPED_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e \\\"s/foo/bar/\\\"`\"
echo \"$caps_add\" | sed 's/^/  /' \t
trimmed=$(sed 's/[[:space:]]*$//' <<<\"$value\")
literal=$(sed 's/[[:space:]]*$//' <<<literal)
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .echo_to_sed_substitution_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "echo $value | sed 's/foo/bar/'",
                    "echo \"$value\" | sed 's/foo/bar/g'",
                    "echo \"$value\" | sed 's§foo§bar§'",
                    "echo ${items[@]} | sed -e 's/foo/bar/2'",
                    "echo \"$(printf %s foo)\" | sed 's/foo/bar/'",
                    "echo \"$COMMAND\" | sed \"s#\\(--appendconfig *[^ $]*\\)#\\1'|'$conf#\"",
                    "echo $OUT | sed \"s|$OUT|\\$this_dir|g\"",
                    "echo \"$hostname\" | sed 's/[]\\[\\.^$*+?{}()|]/\\\\&/g'",
                    "sed 's/[\\.|$(){}?+*^]/\\\\&/g' <<<\"$value\"",
                    "echo \"$value\" | sed 's/a/b/' <<<\"$shadow\"",
                    "echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS//\\\"",
                    "echo \\\"$CFLAGS\\\" | sed 's/^.*\\(-O[^ ]\\).*$/\\1/'",
                    "echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\1:'",
                    "echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e \\\"s/foo/bar/\\\"",
                    "echo \"$caps_add\" | sed 's/^/  /' \t",
                    "sed 's/[[:space:]]*$//' <<<\"$value\"",
                    "sed 's/[[:space:]]*$//' <<<literal",
                ]
            );
        });
    }

    #[test]
    fn skips_nonmatching_echo_to_sed_substitution_shapes() {
        let source = "\
#!/bin/bash
echo literal | sed 's/foo/bar/'
echo prefix${value}suffix | sed 's/foo/bar/'
echo \"prefix${value}\" | sed 's/foo/bar/'
echo $left $right | sed 's/foo/bar/'
echo \"$left $right\" | sed 's/foo/bar/'
echo -n $value | sed 's/foo/bar/'
echo $value | sed -n 's/foo/bar/p'
echo $value | sed --expression 's/foo/bar/'
echo $value | sed -es/foo/bar/
echo $value | sed 's/foo/bar/' | cat
echo \"$ENDPOINT\" | sed 's/[:\\/]/_/g'
echo \"$PAYLOAD\" | sed 's/\\//-/g'
echo $PACKAGE_NAME | sed 's/\\./\\//g'
echo \"$value\" | sed \\\"s/foo/bar/\\\"
echo \"$key\" | sed 's/[]\\[^$.*/]/\\\\&/g'
echo \"${ENTRY}\" | sed 's/\\([/&]\\)/\\\\\\1/g'
sed 's/[]\\[^$.*/]/\\\\&/g' <<<\"$key\"
sed 's/\\([/&]\\)/\\\\\\1/g' <<<\"${ENTRY}\"
printf '%s\\n' \"$value\" | sed 's/foo/bar/'
echo \"prefix$(printf %s foo)\" | sed 's/foo/bar/'
";

        with_facts(source, None, |_, facts| {
            assert!(facts.echo_to_sed_substitution_spans().is_empty());
        });
    }

    #[test]
    fn summarizes_ln_symlink_target_operands() {
        let source = "\
#!/bin/bash
ln -s ../../alpha alpha-link
ln -st/tmp ../../beta ../../gamma
ln --symbolic --target-directory=/tmp ../../delta ../../epsilon
ln -s -- ../../zeta zeta-link
ln -sT ../../eta eta-link
command ln -s ../../wrapped wrapped
ln ../../hard hard-link
ln -t /tmp ../../theta
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let symlink_targets = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("ln"))
            .map(|fact| {
                fact.options().ln().map(|ln| {
                    ln.symlink_target_words()
                        .iter()
                        .map(|word| word.span.slice(source))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();

        assert_eq!(
            symlink_targets,
            vec![
                Some(vec!["../../alpha"]),
                Some(vec!["../../beta", "../../gamma"]),
                Some(vec!["../../delta", "../../epsilon"]),
                Some(vec!["../../zeta"]),
                Some(vec!["../../eta"]),
                Some(vec!["../../wrapped"]),
                None,
                None,
            ]
        );
    }

    #[test]
    fn preserves_dynamic_unset_operands_after_option_parsing_stops() {
        let source = "\
#!/bin/bash
declare -A parts
key=one
unset parts[\"$key\"] extra
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let unset = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("unset"))
            .and_then(|fact| fact.options().unset())
            .expect("expected unset facts");

        assert_eq!(
            unset
                .operand_words()
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["parts[\"$key\"]", "extra"]
        );
    }

    #[test]
    fn records_unset_array_subscript_details_in_operand_facts() {
        let source = "\
#!/bin/bash
declare -A parts
declare -a nums
key=one
unset parts[\"$key\"] plain \"parts[safe]\" 'parts[also_safe]' nums[1]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let unset = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("unset"))
            .and_then(|fact| fact.options().unset())
            .expect("expected unset facts");

        let operand_subscripts = unset
            .operand_facts()
            .iter()
            .map(|operand| {
                operand.array_subscript().map(|subscript| {
                    (
                        operand.word().span.slice(source),
                        subscript.name().as_str().to_owned(),
                        subscript.key_contains_quote(),
                    )
                })
            })
            .collect::<Vec<_>>();

        assert_eq!(
            operand_subscripts,
            vec![
                Some(("parts[\"$key\"]", "parts".to_owned(), true)),
                None,
                None,
                None,
                Some(("nums[1]", "nums".to_owned(), false)),
            ]
        );
    }

    #[test]
    fn collects_prefix_match_spans_from_unset_operands() {
        let source = "\
#!/bin/sh
unset -v \"${!prefix_@}\" x${!prefix_*} \"${!name}\" \"${!arr[@]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let unset = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("unset"))
            .and_then(|fact| fact.options().unset())
            .expect("expected unset facts");

        assert_eq!(
            unset
                .prefix_match_operand_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!prefix_@}", "${!prefix_*}"]
        );
    }

    #[test]
    fn tracks_mapfile_input_fd_and_grouped_find_or_branches() {
        let source = "#!/bin/bash\nmapfile -u 3 -t files 3< <(printf '%s\\n' hi)\nfind . \\( -name a -o -name b -print \\)\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let mapfile = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("mapfile"))
            .and_then(|fact| fact.options().mapfile())
            .expect("expected mapfile facts");
        assert_eq!(mapfile.input_fd(), Some(3));

        let dynamic_source = "#!/bin/bash\nmapfile -u \"$fd\" -t files < <(printf '%s\\n' hi)\n";
        let dynamic_output = Parser::new(dynamic_source).parse().unwrap();
        let dynamic_indexer = Indexer::new(dynamic_source, &dynamic_output);
        let dynamic_semantic =
            SemanticModel::build(&dynamic_output.file, dynamic_source, &dynamic_indexer);
        let dynamic_file_context = classify_file_context(dynamic_source, None, ShellDialect::Bash);
        let dynamic_facts = LinterFacts::build(
            &dynamic_output.file,
            dynamic_source,
            &dynamic_semantic,
            &dynamic_indexer,
            &dynamic_file_context,
        );

        let dynamic_mapfile = dynamic_facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("mapfile"))
            .and_then(|fact| fact.options().mapfile())
            .expect("expected dynamic mapfile facts");
        assert_eq!(dynamic_mapfile.input_fd(), None);

        let find_or_without_grouping_spans = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("find"))
            .filter_map(|fact| fact.options().find())
            .flat_map(|find| find.or_without_grouping_spans().iter().copied())
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(find_or_without_grouping_spans, vec!["-print"]);
    }

    #[test]
    fn parses_grep_pattern_words_from_flags_and_operands() {
        let source = "\
#!/bin/bash
grep item,[0-4] data.txt
grep -e item* data.txt
grep -eitem* data.txt
grep -oe item* data.txt
grep --regexp='a[b]c' data.txt
grep --regexp item? data.txt
grep --regexp=foo* data.txt
grep -eo item* data.txt
grep -F -- item* data.txt
grep -f patterns.txt item* data.txt
grep -F -E foo*bar data.txt
grep -E -F foo*bar data.txt
grep --exclude '*.txt' foo* data.txt
grep --label stdin foo* data.txt
grep --color foo* data.txt
grep --context 3 foo* data.txt
grep --regexp='*start' data.txt
grep -e'*start' data.txt
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let grep_patterns = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("grep"))
            .filter_map(|fact| fact.options().grep())
            .map(|grep| {
                (
                    grep.patterns()
                        .iter()
                        .map(|pattern| {
                            (
                                pattern.span().slice(source),
                                pattern.static_text(),
                                pattern.source_kind(),
                            )
                        })
                        .collect::<Vec<_>>(),
                    grep.uses_fixed_strings,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            grep_patterns,
            vec![
                (
                    vec![(
                        "item,[0-4]",
                        Some("item,[0-4]"),
                        GrepPatternSourceKind::ImplicitOperand,
                    )],
                    false,
                ),
                (
                    vec![(
                        "item*",
                        Some("item*"),
                        GrepPatternSourceKind::ShortOptionSeparate,
                    )],
                    false,
                ),
                (
                    vec![(
                        "-eitem*",
                        Some("item*"),
                        GrepPatternSourceKind::ShortOptionAttached,
                    )],
                    false,
                ),
                (
                    vec![(
                        "item*",
                        Some("item*"),
                        GrepPatternSourceKind::ShortOptionSeparate,
                    )],
                    false,
                ),
                (
                    vec![(
                        "--regexp='a[b]c'",
                        Some("a[b]c"),
                        GrepPatternSourceKind::LongOptionAttached,
                    )],
                    false,
                ),
                (
                    vec![(
                        "item?",
                        Some("item?"),
                        GrepPatternSourceKind::LongOptionSeparate,
                    )],
                    false,
                ),
                (
                    vec![(
                        "--regexp=foo*",
                        Some("foo*"),
                        GrepPatternSourceKind::LongOptionAttached,
                    )],
                    false,
                ),
                (
                    vec![("-eo", Some("o"), GrepPatternSourceKind::ShortOptionAttached,)],
                    false,
                ),
                (
                    vec![(
                        "item*",
                        Some("item*"),
                        GrepPatternSourceKind::ImplicitOperand,
                    )],
                    true,
                ),
                (Vec::new(), false),
                (
                    vec![(
                        "foo*bar",
                        Some("foo*bar"),
                        GrepPatternSourceKind::ImplicitOperand,
                    )],
                    false,
                ),
                (
                    vec![(
                        "foo*bar",
                        Some("foo*bar"),
                        GrepPatternSourceKind::ImplicitOperand,
                    )],
                    true,
                ),
                (
                    vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                    false,
                ),
                (
                    vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                    false,
                ),
                (
                    vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                    false,
                ),
                (
                    vec![("foo*", Some("foo*"), GrepPatternSourceKind::ImplicitOperand,)],
                    false,
                ),
                (
                    vec![(
                        "--regexp='*start'",
                        Some("*start"),
                        GrepPatternSourceKind::LongOptionAttached,
                    )],
                    false,
                ),
                (
                    vec![(
                        "-e'*start'",
                        Some("*start"),
                        GrepPatternSourceKind::ShortOptionAttached,
                    )],
                    false,
                ),
            ]
        );
    }

    #[test]
    fn grep_pattern_facts_track_leading_glob_style_star_prefixes() {
        let source = "\
#!/bin/bash
grep '*start' data.txt
grep ''*user data.txt
grep '^*' data.txt
grep '^*foo' data.txt
grep --regexp='*start' data.txt
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let grep_patterns = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("grep"))
            .filter_map(|fact| fact.options().grep())
            .flat_map(|grep| grep.patterns().iter())
            .map(|pattern| {
                (
                    pattern.span().slice(source),
                    pattern.static_text(),
                    pattern.starts_with_glob_style_star(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            grep_patterns,
            vec![
                ("'*start'", Some("*start"), true),
                ("''*user", Some("*user"), true),
                ("'^*'", Some("^*"), true),
                ("'^*foo'", Some("^*foo"), false),
                ("--regexp='*start'", Some("*start"), true),
            ]
        );
    }

    #[test]
    fn attached_short_e_patterns_do_not_accidentally_toggle_only_matching() {
        let source = "\
#!/bin/bash
grep -oe item* data.txt
grep -eo item* data.txt
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let grep_modes = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("grep"))
            .filter_map(|fact| fact.options().grep())
            .map(|grep| grep.uses_only_matching)
            .collect::<Vec<_>>();

        assert_eq!(grep_modes, vec![true, false]);
    }

    #[test]
    fn tracks_dynamic_ps_pid_selectors() {
        let source = "\
#!/bin/bash
ps -p\"$pid\" -o comm=
ps --pid=\"$pid\" -o comm=
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let ps_commands = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("ps"))
            .collect::<Vec<_>>();

        assert_eq!(ps_commands.len(), 2);
        assert!(
            ps_commands
                .iter()
                .all(|fact| fact.options().ps().is_some_and(|ps| ps.has_pid_selector))
        );
    }

    #[test]
    fn tracks_bare_ps_pid_operands() {
        let source = "\
#!/bin/bash
ps 1 -o comm=
ps 1,2 -o comm=
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let ps_commands = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("ps"))
            .collect::<Vec<_>>();

        assert_eq!(ps_commands.len(), 2);
        assert!(
            ps_commands
                .iter()
                .all(|fact| fact.options().ps().is_some_and(|ps| ps.has_pid_selector))
        );
    }

    #[test]
    fn tracks_ps_pid_selectors_after_bsd_style_clusters() {
        let source = "\
#!/bin/bash
ps aux -p 1 -o comm=
ps ax -q 1 -o comm=
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let ps_commands = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("ps"))
            .collect::<Vec<_>>();

        assert_eq!(ps_commands.len(), 2);
        assert!(
            ps_commands
                .iter()
                .all(|fact| fact.options().ps().is_some_and(|ps| ps.has_pid_selector))
        );
    }

    #[test]
    fn collects_base_prefix_arithmetic_spans_across_arithmetic_nodes() {
        let source = "\
#!/bin/bash
echo $((10#123))
echo ${foo:10#1:2}
: > \"$((10#1))\"
echo ${foo:-$((10#1))}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert_eq!(
            facts
                .base_prefix_arithmetic_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["10#123", "10#1", "10#1", "10#1"]
        );
    }

    #[test]
    fn ignores_base_prefix_like_parameter_trim_operands() {
        let source = "\
#!/bin/bash
: \"${progname:=\"${0##*/}\"}\"
echo ${foo:-${1##*/}}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(facts.base_prefix_arithmetic_spans().is_empty());
    }

    #[test]
    fn builds_find_execdir_command_facts_for_shell_targets() {
        let source = "\
#!/bin/sh
# shellcheck disable=2086,2154
find $dir -type f -name \"rename*\" -execdir sh -c 'mv {} $(echo {} | sed \"s|rename|perl-rename|\")' \\;
";

        with_facts(source, None, |_, facts| {
            let find = facts
                .commands()
                .iter()
                .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
                .expect("expected find -execdir fact");

            assert_eq!(find.effective_name(), Some("sh"));
            assert_eq!(find.wrappers(), &[WrapperKind::FindExecDir]);

            let find_execdir = find
                .options()
                .find_execdir()
                .expect("expected shell command fact for find -execdir");
            assert_eq!(
                find_execdir
                    .shell_command_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["'mv {} $(echo {} | sed \"s|rename|perl-rename|\")'"]
            );
        });
    }

    #[test]
    fn builds_find_exec_argument_word_spans_for_wrapped_commands() {
        let source = "\
#!/bin/sh
find \"$root\"/*.py -exec echo \"$prefix\"*.tmp {} \\; -name '*.cfg'
result=$(find . -type d -name fuzz -exec dirname $(readlink -f {}) \\;)
find . -execdir sh -c 'printf \"%s\\n\" {}' {} \\;
";

        with_facts(source, None, |_, facts| {
            let top_level_find_exec = facts
                .commands()
                .iter()
                .find(|fact| {
                    fact.has_wrapper(WrapperKind::FindExec)
                        && !fact.is_nested_word_command()
                        && fact.effective_name_is("echo")
                })
                .expect("expected top-level find -exec fact");
            assert_eq!(
                top_level_find_exec
                    .options()
                    .find_exec()
                    .expect("expected find -exec facts")
                    .argument_word_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["\"$prefix\"*.tmp", "{}"]
            );

            let nested_find_exec = facts
                .commands()
                .iter()
                .find(|fact| {
                    fact.has_wrapper(WrapperKind::FindExec)
                        && fact.is_nested_word_command()
                        && fact.effective_name_is("dirname")
                })
                .expect("expected nested find -exec fact");
            assert_eq!(
                nested_find_exec
                    .options()
                    .find_exec()
                    .expect("expected nested find -exec facts")
                    .argument_word_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$(readlink -f {})"]
            );

            let plus_argument_find_exec = facts
                .commands()
                .iter()
                .find(|fact| {
                    fact.has_wrapper(WrapperKind::FindExec)
                        && !fact.is_nested_word_command()
                        && fact.effective_name_is("echo")
                        && fact.options().find_exec().is_some_and(|find_exec| {
                            find_exec
                                .argument_word_spans()
                                .iter()
                                .any(|span| span.slice(source) == "\"$prefix\"*.tmp")
                        })
                })
                .expect("expected semicolon-terminated find -exec fact");
            assert_eq!(
                plus_argument_find_exec
                    .options()
                    .find_exec()
                    .expect("expected find -exec facts")
                    .argument_word_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["\"$prefix\"*.tmp", "{}"]
            );

            let find_execdir = facts
                .commands()
                .iter()
                .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
                .expect("expected find -execdir fact");
            assert_eq!(
                find_execdir
                    .options()
                    .find_exec()
                    .expect("expected find -exec facts for execdir wrapper")
                    .argument_word_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["-c", "'printf \"%s\\n\" {}'", "{}"]
            );
        });
    }

    #[test]
    fn keeps_plus_arguments_before_semicolon_terminated_find_exec() {
        let source = "#!/bin/sh\nfind . -exec echo + *.tmp {} \\;\n";

        with_facts(source, None, |_, facts| {
            let find_exec = facts
                .commands()
                .iter()
                .find(|fact| fact.has_wrapper(WrapperKind::FindExec))
                .expect("expected find -exec fact");

            assert_eq!(
                find_exec
                    .options()
                    .find_exec()
                    .expect("expected find -exec facts")
                    .argument_word_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["+", "*.tmp", "{}"]
            );
        });
    }

    #[test]
    fn builds_find_execdir_command_facts_for_bundled_shell_c_flags() {
        let source = "\
#!/bin/sh
find . -execdir sh -ec 'mv {} \"$target\"' \\;
";

        with_facts(source, None, |_, facts| {
            let find = facts
                .commands()
                .iter()
                .find(|fact| fact.has_wrapper(WrapperKind::FindExecDir))
                .expect("expected find -execdir fact");

            let find_execdir = find
                .options()
                .find_execdir()
                .expect("expected shell command fact for bundled -c flags");
            assert_eq!(
                find_execdir
                    .shell_command_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["'mv {} \"$target\"'"]
            );
        });
    }

    #[test]
    fn summarizes_builtin_wrapped_reads() {
        let source = "#!/bin/bash\nbuiltin read response\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let read = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("read"))
            .expect("expected builtin-wrapped read fact");

        assert_eq!(read.wrappers(), &[WrapperKind::Builtin]);
        assert_eq!(
            read.options().read().map(|read| read.uses_raw_input),
            Some(false)
        );
    }

    #[test]
    fn summarizes_su_login_and_command_forms() {
        let source = "\
#!/bin/bash
su root
su root -c id
su \"$user\" -s /bin/sh -c \"$cmd\"
su -s /bin/sh root
su -
su --login root
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let su_flags = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("su"))
            .map(|fact| fact.options().su().map(|su| su.has_login_or_command_flag()))
            .collect::<Vec<_>>();

        assert_eq!(
            su_flags,
            vec![
                Some(false),
                Some(true),
                Some(true),
                Some(false),
                Some(true),
                Some(true)
            ]
        );
    }

    #[test]
    fn keeps_incomplete_su_command_flags_unsafe() {
        let source = "\
#!/bin/bash
su -c
su --command
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let su_flags = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("su"))
            .map(|fact| fact.options().su().map(|su| su.has_login_or_command_flag()))
            .collect::<Vec<_>>();

        assert_eq!(su_flags, vec![Some(false), Some(false)]);
    }

    #[test]
    fn stops_treating_su_args_after_double_dash_as_flags() {
        let source = "\
#!/bin/bash
su -- root echo -c hi
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let su_flags = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("su"))
            .map(|fact| fact.options().su().map(|su| su.has_login_or_command_flag()))
            .collect::<Vec<_>>();

        assert_eq!(su_flags, vec![Some(false)]);
    }

    #[test]
    fn stops_treating_su_forwarded_command_args_as_flags() {
        let source = "\
#!/bin/bash
su root bash -c 'id'
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let su_flags = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("su"))
            .map(|fact| fact.options().su().map(|su| su.has_login_or_command_flag()))
            .collect::<Vec<_>>();

        assert_eq!(su_flags, vec![Some(false)]);
    }

    #[test]
    fn summarizes_first_nonportable_sh_builtin_option_words() {
        let source = "\
#!/bin/sh
read -r name
read -p prompt name
read -\"$mode\" name
printf -v out '%s' foo
printf -- -v out
export -p
export -fn foo
command export -fn foo
trap -p EXIT
trap -- -p EXIT
wait -n
wait -p jobid -n
ulimit -f
ulimit -n
type -P printf
";

        with_facts_dialect(
            source,
            None,
            ParseShellDialect::Bash,
            ShellDialect::Sh,
            |_, facts| {
                let spans = facts
                    .commands()
                    .iter()
                    .filter_map(|fact| {
                        fact.options()
                            .nonportable_sh_builtin_option_span()
                            .map(|span| span.slice(source))
                    })
                    .collect::<Vec<_>>();

                assert_eq!(
                    spans,
                    vec![
                        "-p",
                        "-\"$mode\"",
                        "-v",
                        "-fn",
                        "-fn",
                        "-p",
                        "-n",
                        "-p",
                        "-n",
                        "-P"
                    ]
                );
            },
        );
    }

    #[test]
    fn set_command_flags_without_prefix_ignore_quoted_literals() {
        let source = "\
set foo bar
set \"foo\" bar
set f\"oo\" bar
set 'foo' bar
";

        with_facts(source, None, |_, facts| {
            let set_without_prefix_spans = facts
                .commands()
                .iter()
                .filter(|fact| fact.effective_name_is("set"))
                .filter_map(|fact| fact.options().set())
                .flat_map(|set| set.flags_without_prefix_spans().iter().copied())
                .collect::<Vec<_>>();

            assert_eq!(
                set_without_prefix_spans
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["foo"]
            );
        });
    }

    #[test]
    fn summarizes_directory_change_commands_and_errexit_hints() {
        let source = "\n#!/bin/bash -eu\ncd ../..\ncd -\nbuiltin cd /\npushd ..\npopd\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(facts.errexit_enabled_anywhere());

        let directory_changes = facts
            .commands()
            .iter()
            .filter_map(|fact| {
                fact.options().directory_change().map(|directory_change| {
                    (
                        directory_change.command_name(),
                        directory_change.is_plain_directory_stack_marker(),
                        directory_change.is_manual_restore_candidate(),
                        fact.wrappers().to_vec(),
                    )
                })
            })
            .collect::<Vec<_>>();

        assert_eq!(
            directory_changes,
            vec![
                ("cd", true, false, vec![]),
                ("cd", false, true, vec![]),
                ("cd", false, false, vec![WrapperKind::Builtin]),
                ("pushd", true, false, vec![]),
                ("popd", false, false, vec![])
            ]
        );
    }

    #[test]
    fn does_not_treat_long_shebang_options_as_errexit() {
        let source = "#!/bin/bash --noprofile\ncd /tmp\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        assert!(!facts.errexit_enabled_anywhere());
    }

    #[test]
    fn keeps_read_raw_input_when_option_flags_are_dynamic() {
        let source = "#!/bin/bash\nbuiltin read -${_read_char_flag} 1 -s -r anykey\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let read = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("read"))
            .expect("expected dynamic-option read fact");

        assert_eq!(read.wrappers(), &[WrapperKind::Builtin]);
        assert_eq!(
            read.options().read().map(|read| read.uses_raw_input),
            Some(true)
        );
    }

    #[test]
    fn resolves_sudo_family_invokers_through_outer_wrappers() {
        let source = "#!/bin/bash\ncommand sudo tee out.txt\ncommand run0 tee out.txt\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let invokers = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().sudo_family().map(|sudo| sudo.invoker))
            .collect::<Vec<_>>();

        assert_eq!(
            invokers,
            vec![SudoFamilyInvoker::Sudo, SudoFamilyInvoker::Run0]
        );
    }

    #[test]
    fn resolves_sudo_family_invokers_when_wrapper_target_is_unresolved() {
        let source = "\
#!/bin/bash
sudo \"$tool\" > out.txt
sudo -V
command run0 --version
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let invokers = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().sudo_family().map(|sudo| sudo.invoker))
            .collect::<Vec<_>>();

        assert_eq!(
            invokers,
            vec![
                SudoFamilyInvoker::Sudo,
                SudoFamilyInvoker::Sudo,
                SudoFamilyInvoker::Run0,
            ]
        );
    }

    #[test]
    fn parses_long_xargs_null_mode_and_numeric_exit_status() {
        let source = "#!/bin/bash\nfind . -print0 | xargs --null rm\nexit 42\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let xargs = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("xargs"))
            .and_then(|fact| fact.options().xargs())
            .expect("expected xargs facts");
        assert!(xargs.uses_null_input);
        assert!(xargs.inline_replace_option_spans().is_empty());

        let exit = facts
            .commands()
            .iter()
            .find(|fact| fact.options().exit().is_some())
            .and_then(|fact| fact.options().exit())
            .expect("expected exit facts");
        assert_eq!(
            exit.status_word.map(|word| word.span.slice(source)),
            Some("42")
        );
        assert!(exit.has_static_status());
        assert!(exit.is_numeric_literal);
        assert!(!exit.has_invalid_status_argument());
    }

    #[test]
    fn parses_mixed_and_pure_dynamic_exit_status_shapes() {
        let source = "\
#!/bin/bash
code=3
other=4
exit \"message $code\"
exit \"123$code\"
exit \"$code\"
exit \"${code}${other}\"
exit \"$(printf '%s' 3)\"
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let exits = facts
            .commands()
            .iter()
            .filter_map(|fact| fact.options().exit())
            .collect::<Vec<_>>();

        assert_eq!(
            exits
                .iter()
                .map(|exit| exit.status_word.map(|word| word.span.slice(source)))
                .collect::<Vec<_>>(),
            vec![
                Some("\"message $code\""),
                Some("\"123$code\""),
                Some("\"$code\""),
                Some("\"${code}${other}\""),
                Some("\"$(printf '%s' 3)\""),
            ]
        );
        assert_eq!(
            exits
                .iter()
                .map(|exit| exit.has_invalid_status_argument())
                .collect::<Vec<_>>(),
            vec![true, true, false, false, false]
        );
    }

    #[test]
    fn keeps_parsing_xargs_flags_after_optional_argument_forms() {
        let source = "\
#!/bin/bash
find . -print0 | xargs -l -0 rm
find . -print0 | xargs --eof --null rm
xargs -i0 echo
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let xargs_facts = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("xargs"))
            .filter_map(|fact| fact.options().xargs())
            .collect::<Vec<_>>();

        assert_eq!(xargs_facts.len(), 3);
        assert!(xargs_facts[0].uses_null_input);
        assert!(xargs_facts[1].uses_null_input);
        assert!(!xargs_facts[2].uses_null_input);
        assert_eq!(
            xargs_facts[2]
                .inline_replace_option_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-i0"]
        );
    }

    #[test]
    fn does_not_consume_null_mode_after_optional_long_eof() {
        let source = "#!/bin/bash\nfind . -print0 | xargs --eof --null rm\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let xargs = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("xargs"))
            .and_then(|fact| fact.options().xargs())
            .expect("expected xargs facts");

        assert!(xargs.uses_null_input);
    }

    #[test]
    fn keeps_parsing_xargs_flags_after_arg_file() {
        let source = "\
#!/bin/bash
find . -print0 | xargs -a inputs -0 rm
xargs -a inputs -iX echo X
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let xargs_facts = facts
            .commands()
            .iter()
            .filter(|fact| fact.effective_name_is("xargs"))
            .filter_map(|fact| fact.options().xargs())
            .collect::<Vec<_>>();

        assert_eq!(xargs_facts.len(), 2);
        assert!(xargs_facts[0].uses_null_input);
        assert_eq!(
            xargs_facts[1]
                .inline_replace_option_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-iX"]
        );
    }

    #[test]
    fn builds_redirect_facts_with_cached_target_analysis() {
        let source = "#!/bin/bash\necho hi 2>&3 >/dev/null >> \"$((i++))\"\n";

        with_facts(source, None, |_, facts| {
            let command = facts
                .structural_commands()
                .find(|fact| fact.effective_name_is("echo"))
                .expect("expected echo fact");

            let redirects = command.redirect_facts();
            assert_eq!(redirects.len(), 3);

            let descriptor_dup = &redirects[0];
            assert!(
                descriptor_dup
                    .analysis()
                    .is_some_and(|analysis| analysis.is_descriptor_dup())
            );
            assert_eq!(
                descriptor_dup
                    .analysis()
                    .and_then(|analysis| analysis.numeric_descriptor_target),
                Some(3)
            );

            let dev_null = &redirects[1];
            assert_eq!(
                dev_null.target_span().map(|span| span.slice(source)),
                Some("/dev/null")
            );
            assert!(
                dev_null
                    .analysis()
                    .is_some_and(|analysis| analysis.is_definitely_dev_null())
            );

            let arithmetic = &redirects[2];
            assert_eq!(
                arithmetic.target_span().map(|span| span.slice(source)),
                Some("\"$((i++))\"")
            );
            assert!(
                arithmetic
                    .analysis()
                    .is_some_and(|analysis| { analysis.expansion.hazards.arithmetic_expansion })
            );
        });
    }

    #[test]
    fn builds_substitution_facts_with_intent_and_host_kinds() {
        let source = "\
#!/bin/bash
printf '%s\\n' $(printf arg) \"$(printf quoted)\"
local decl_assign=$(printf decl-assign)
name[$(printf assign)]=1
declare arr[$(printf decl-name)]
declare other=$(printf decl-assign-2)
declare -A map=([$(printf key)]=1)
cat <<<$(printf here)
out=$(printf hi > out.txt)
drop=$(printf hi >/dev/null 2>&1)
mixed=$(jq -r . <<< \"$status\" || die >&2)
x=$(echo direct)
y=$(foo $(echo nested))
z=$(ls layout.*.h | cut -d. -f2 | xargs echo)
";

        with_facts(source, None, |_, facts| {
            let substitutions = facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts().iter().copied())
                .map(|fact| {
                    (
                        fact.span().slice(source).to_owned(),
                        fact.stdout_intent(),
                        fact.host_kind(),
                        fact.unquoted_in_host(),
                        fact.body_contains_ls(),
                        fact.body_contains_echo(),
                    )
                })
                .collect::<Vec<_>>();

            assert!(substitutions.contains(&(
                "$(printf arg)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::CommandArgument,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf decl-assign)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::DeclarationAssignmentValue,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf quoted)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::CommandArgument,
                false,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf here)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::HereStringOperand,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf assign)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::AssignmentTargetSubscript,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf decl-name)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::DeclarationNameSubscript,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf key)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::ArrayKeySubscript,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf hi > out.txt)".to_owned(),
                SubstitutionOutputIntent::Rerouted,
                SubstitutionHostKind::Other,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf hi >/dev/null 2>&1)".to_owned(),
                SubstitutionOutputIntent::Discarded,
                SubstitutionHostKind::Other,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(jq -r . <<< \"$status\" || die >&2)".to_owned(),
                SubstitutionOutputIntent::Mixed,
                SubstitutionHostKind::Other,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(echo direct)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::Other,
                true,
                false,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(foo $(echo nested))".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::Other,
                true,
                false,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(echo nested)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::CommandArgument,
                true,
                false,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(ls layout.*.h | cut -d. -f2 | xargs echo)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::Other,
                true,
                true,
                false,
            )));
        });
    }

    #[test]
    fn tracks_backtick_syntax_in_substitution_facts() {
        let source = "\
#!/bin/sh
printf '%s\\n' `date` $(uname) <(cat /etc/hosts)
";

        with_facts(source, None, |_, facts| {
            let substitutions = facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts().iter().copied())
                .map(|fact| {
                    (
                        fact.span().slice(source).to_owned(),
                        fact.command_syntax(),
                        fact.uses_backtick_syntax(),
                    )
                })
                .collect::<Vec<_>>();

            assert!(substitutions.contains(&(
                "`date`".to_owned(),
                Some(CommandSubstitutionSyntax::Backtick),
                true,
            )));
            assert!(substitutions.contains(&(
                "$(uname)".to_owned(),
                Some(CommandSubstitutionSyntax::DollarParen),
                false,
            )));
            assert!(substitutions.contains(&("<(cat /etc/hosts)".to_owned(), None, false,)));
        });
    }

    #[test]
    fn tracks_multi_statement_substitution_bodies() {
        let source = "\
#!/bin/sh
single=$(printf '%s\\n' ok)
multiple=$(printf '%s\\n' one; printf '%s\\n' two)
conditional=$( [[ -n $value ]] && printf '%s\\n' ok )
";

        with_facts(source, None, |_, facts| {
            let substitutions = facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts().iter().copied())
                .map(|fact| {
                    (
                        fact.span().slice(source).to_owned(),
                        fact.body_has_multiple_statements(),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>();

            assert_eq!(substitutions.get("$(printf '%s\\n' ok)"), Some(&false));
            assert_eq!(
                substitutions.get("$(printf '%s\\n' one; printf '%s\\n' two)"),
                Some(&true)
            );
            assert_eq!(
                substitutions.get("$( [[ -n $value ]] && printf '%s\\n' ok )"),
                Some(&false)
            );
        });
    }

    #[test]
    fn identifies_command_substitutions_that_echo_plain_text_or_expansions() {
        let source = "\
#!/bin/sh
plain=$(echo foo)
expanded=$(echo $foo)
quoted=$(echo \"$foo\")
var_suffix=$(echo foo$foo)
command_subst=$(echo foo $(date))
option_like=$(echo -en \"\\001\")
glob_like=$(echo O*)
brace_like=$(echo {a,b})
";

        with_facts(source, None, |_, facts| {
            let substitutions = facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts().iter().copied())
                .map(|fact| {
                    (
                        fact.span().slice(source).to_owned(),
                        fact.body_contains_echo(),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>();

            assert_eq!(substitutions.get("$(echo foo)"), Some(&true));
            assert_eq!(substitutions.get("$(echo $foo)"), Some(&true));
            assert_eq!(substitutions.get("$(echo \"$foo\")"), Some(&true));
            assert_eq!(substitutions.get("$(echo foo$foo)"), Some(&true));
            assert_eq!(substitutions.get("$(echo foo $(date))"), Some(&true));
            assert_eq!(substitutions.get("$(echo -en \"\\001\")"), Some(&false));
            assert_eq!(substitutions.get("$(echo O*)"), Some(&false));
            assert_eq!(substitutions.get("$(echo {a,b})"), Some(&false));
        });
    }

    #[test]
    fn identifies_command_substitutions_that_grep_output_directly() {
        let source = "\
#!/bin/sh
plain=$(grep foo input.txt)
quiet=$(grep -q foo input.txt)
egrep_plain=$(egrep foo input.txt)
fgrep_plain=$(fgrep foo input.txt)
nested_pipeline=$(echo foo | grep foo input.txt)
escaped_pipeline=$(echo foo | \\grep foo input.txt)
nested=$(foo $(grep foo input.txt))
mixed=$(grep foo input.txt)$(date)
pipeline=$(grep foo input.txt | wc -l)
sequence=$(foo; grep foo input.txt)
and_chain=$(foo && grep foo input.txt)
legacy=`nvm ls | grep '^ *\\.'`
";

        with_facts(source, None, |_, facts| {
            let substitutions = facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts().iter().copied())
                .map(|fact| {
                    (
                        fact.span().slice(source).to_owned(),
                        fact.body_contains_grep(),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>();

            assert_eq!(substitutions.get("$(grep foo input.txt)"), Some(&true));
            assert_eq!(substitutions.get("$(grep -q foo input.txt)"), Some(&true));
            assert_eq!(substitutions.get("$(egrep foo input.txt)"), Some(&true));
            assert_eq!(substitutions.get("$(fgrep foo input.txt)"), Some(&true));
            assert_eq!(
                substitutions.get("$(echo foo | grep foo input.txt)"),
                Some(&true)
            );
            assert_eq!(
                substitutions.get("$(echo foo | \\grep foo input.txt)"),
                Some(&true)
            );
            assert_eq!(
                substitutions.get("$(foo $(grep foo input.txt))"),
                Some(&false)
            );
            assert_eq!(
                substitutions.get("$(grep foo input.txt | wc -l)"),
                Some(&false)
            );
            assert_eq!(
                substitutions.get("$(foo; grep foo input.txt)"),
                Some(&false)
            );
            assert_eq!(
                substitutions.get("$(foo && grep foo input.txt)"),
                Some(&false)
            );
            assert_eq!(substitutions.get("`nvm ls | grep '^ *\\.'`"), Some(&true));
        });
    }

    #[test]
    fn marks_redirect_only_input_command_substitutions_as_bash_file_slurps() {
        let source = "\
#!/bin/bash
printf '%s\\n' $(<input.txt) \"$( < spaced.txt )\" $(0< fd.txt) $(< quiet.txt 2>/dev/null) $(< muted.txt >/dev/null) $(< closed.txt 0<&-) $(cat < portable.txt) $(> out.txt) $(foo=bar)
";

        with_facts(source, None, |_, facts| {
            let substitutions = facts
                .commands()
                .iter()
                .flat_map(|fact| fact.substitution_facts().iter().copied())
                .map(|fact| {
                    (
                        fact.span().slice(source).to_owned(),
                        fact.is_bash_file_slurp(),
                    )
                })
                .collect::<Vec<_>>();

            assert!(substitutions.contains(&("$(<input.txt)".to_owned(), true)));
            assert!(
                substitutions
                    .contains(&("\"$( < spaced.txt )\"".trim_matches('"').to_owned(), true))
            );
            assert!(substitutions.contains(&("$(0< fd.txt)".to_owned(), true)));
            assert!(substitutions.contains(&("$(< quiet.txt 2>/dev/null)".to_owned(), false)));
            assert!(substitutions.contains(&("$(< muted.txt >/dev/null)".to_owned(), false)));
            assert!(substitutions.contains(&("$(< closed.txt 0<&-)".to_owned(), false)));
            assert!(substitutions.contains(&("$(cat < portable.txt)".to_owned(), false)));
            assert!(substitutions.contains(&("$(> out.txt)".to_owned(), false)));
            assert!(substitutions.contains(&("$(foo=bar)".to_owned(), false)));
        });
    }

    #[test]
    fn builds_simple_test_facts_with_shapes_and_closing_bracket_validation() {
        let source = "\
#!/bin/sh
test
[ foo ]
[ -n foo ]
[ left = right ]
[ ! = right ]
[ ! -n foo ]
[ ! left = right ]
[ foo -eq 1 ]
[ missing
";

        with_facts(source, None, |_, facts| {
            let commands = facts
                .structural_commands()
                .map(|fact| (fact.span().slice(source).trim_end().to_owned(), fact))
                .collect::<Vec<_>>();

            let empty = commands
                .iter()
                .find(|(text, _)| text == "test")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected test fact");
            assert_eq!(empty.syntax(), SimpleTestSyntax::Test);
            assert_eq!(empty.shape(), SimpleTestShape::Empty);

            let truthy = commands
                .iter()
                .find(|(text, _)| text == "[ foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected truthy test fact");
            assert_eq!(truthy.syntax(), SimpleTestSyntax::Bracket);
            assert_eq!(truthy.shape(), SimpleTestShape::Truthy);
            assert!(
                truthy
                    .truthy_operand_class()
                    .is_some_and(|class| class.is_fixed_literal())
            );

            let unary = commands
                .iter()
                .find(|(text, _)| text == "[ -n foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected unary test fact");
            assert_eq!(unary.shape(), SimpleTestShape::Unary);
            assert_eq!(
                unary.operator_family(),
                SimpleTestOperatorFamily::StringUnary
            );
            assert!(
                unary
                    .unary_operand_class()
                    .is_some_and(|class| class.is_fixed_literal())
            );

            let binary = commands
                .iter()
                .find(|(text, _)| text == "[ left = right ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected binary test fact");
            assert_eq!(binary.shape(), SimpleTestShape::Binary);
            assert_eq!(
                binary.operator_family(),
                SimpleTestOperatorFamily::StringBinary
            );
            assert!(
                binary
                    .binary_operand_classes()
                    .is_some_and(
                        |(left, right)| left.is_fixed_literal() && right.is_fixed_literal()
                    )
            );

            let literal_bang_binary = commands
                .iter()
                .find(|(text, _)| text == "[ ! = right ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected literal bang binary test fact");
            assert_eq!(literal_bang_binary.shape(), SimpleTestShape::Binary);
            assert!(!literal_bang_binary.is_effectively_negated());
            assert_eq!(
                literal_bang_binary.effective_shape(),
                SimpleTestShape::Binary
            );
            assert_eq!(
                literal_bang_binary.effective_operator_family(),
                SimpleTestOperatorFamily::StringBinary
            );
            assert!(
                literal_bang_binary
                    .effective_operand_class(0)
                    .zip(literal_bang_binary.effective_operand_class(2))
                    .is_some_and(|(left, right)| {
                        left.is_fixed_literal() && right.is_fixed_literal()
                    })
            );

            let negated_unary = commands
                .iter()
                .find(|(text, _)| text == "[ ! -n foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected negated unary test fact");
            assert_eq!(negated_unary.shape(), SimpleTestShape::Binary);
            assert!(negated_unary.is_effectively_negated());
            assert_eq!(negated_unary.effective_shape(), SimpleTestShape::Unary);
            assert_eq!(
                negated_unary.effective_operator_family(),
                SimpleTestOperatorFamily::StringUnary
            );
            assert!(
                negated_unary
                    .effective_operand_class(1)
                    .is_some_and(|class| class.is_fixed_literal())
            );

            let negated_binary = commands
                .iter()
                .find(|(text, _)| text == "[ ! left = right ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected negated binary test fact");
            assert_eq!(negated_binary.shape(), SimpleTestShape::Other);
            assert!(negated_binary.is_effectively_negated());
            assert_eq!(negated_binary.effective_shape(), SimpleTestShape::Binary);
            assert_eq!(
                negated_binary.effective_operator_family(),
                SimpleTestOperatorFamily::StringBinary
            );
            assert!(
                negated_binary
                    .effective_operand_class(0)
                    .zip(negated_binary.effective_operand_class(2))
                    .is_some_and(|(left, right)| {
                        left.is_fixed_literal() && right.is_fixed_literal()
                    })
            );

            let non_string_binary = commands
                .iter()
                .find(|(text, _)| text == "[ foo -eq 1 ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected numeric test fact");
            assert_eq!(non_string_binary.shape(), SimpleTestShape::Binary);
            assert_eq!(
                non_string_binary.operator_family(),
                SimpleTestOperatorFamily::Other
            );

            let missing_closer = commands
                .iter()
                .find(|(text, _)| text == "[ missing")
                .map(|(_, fact)| fact.simple_test());
            assert!(matches!(missing_closer, Some(None)));
        });
    }

    #[test]
    fn simple_test_fact_tracks_truthy_string_unary_and_string_binary_subexpressions() {
        let source = "\
#!/bin/sh
[ foo ]
[ ! bar ]
[ -z baz ]
[ ! -n qux ]
[ \"-n\" ]
[ \"-n\" foo ]
[ \"!\" \"-n\" qux ]
[ -a foo ]
[ -o foo ]
[ ! -a baz ]
[ ! -o quux ]
[ foo -o -z baz ]
[ -a foo -o -z baz ]
[ foo \"-o\" \"-z\" baz ]
[ -f file -a ! -z baz ]
[ lhs = rhs ]
[ lhs = rhs -a -z baz ]
";

        with_facts(source, None, |_, facts| {
            let commands = facts
                .structural_commands()
                .map(|fact| (fact.span().slice(source).trim_end().to_owned(), fact))
                .collect::<Vec<_>>();

            let truthy = commands
                .iter()
                .find(|(text, _)| text == "[ foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected truthy test fact");
            assert_eq!(
                truthy
                    .truthy_expression_words(source)
                    .into_iter()
                    .map(|word| word.span.slice(source).to_owned())
                    .collect::<Vec<_>>(),
                vec!["foo"]
            );

            let negated_truthy = commands
                .iter()
                .find(|(text, _)| text == "[ ! bar ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected negated truthy test fact");
            assert_eq!(
                negated_truthy
                    .truthy_expression_words(source)
                    .into_iter()
                    .map(|word| word.span.slice(source).to_owned())
                    .collect::<Vec<_>>(),
                vec!["bar"]
            );

            let unary = commands
                .iter()
                .find(|(text, _)| text == "[ -z baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected unary test fact");
            assert_eq!(
                unary
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("-z".to_owned(), "baz".to_owned())]
            );

            let negated_unary = commands
                .iter()
                .find(|(text, _)| text == "[ ! -n qux ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected negated unary test fact");
            assert_eq!(
                negated_unary
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("-n".to_owned(), "qux".to_owned())]
            );

            let quoted_literal = commands
                .iter()
                .find(|(text, _)| text == "[ \"-n\" ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected quoted literal test fact");
            assert_eq!(
                quoted_literal
                    .truthy_expression_words(source)
                    .into_iter()
                    .map(|word| word.span.slice(source).to_owned())
                    .collect::<Vec<_>>(),
                vec!["\"-n\""]
            );
            assert!(
                quoted_literal
                    .string_unary_expression_words(source)
                    .is_empty()
            );

            let negated_unary_a = commands
                .iter()
                .find(|(text, _)| text == "[ ! -a baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected negated unary -a test fact");
            assert!(negated_unary_a.truthy_expression_words(source).is_empty());
            assert!(
                negated_unary_a
                    .string_unary_expression_words(source)
                    .is_empty()
            );

            let negated_unary_o = commands
                .iter()
                .find(|(text, _)| text == "[ ! -o quux ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected negated unary -o test fact");
            assert!(negated_unary_o.truthy_expression_words(source).is_empty());
            assert!(
                negated_unary_o
                    .string_unary_expression_words(source)
                    .is_empty()
            );

            let quoted_unary = commands
                .iter()
                .find(|(text, _)| text == "[ \"-n\" foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected quoted unary test fact");
            assert_eq!(
                quoted_unary
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("\"-n\"".to_owned(), "foo".to_owned())]
            );

            let quoted_negated_unary = commands
                .iter()
                .find(|(text, _)| text == "[ \"!\" \"-n\" qux ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected quoted negated unary test fact");
            assert_eq!(
                quoted_negated_unary
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("\"-n\"".to_owned(), "qux".to_owned())]
            );

            let unary_and = commands
                .iter()
                .find(|(text, _)| text == "[ -a foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected unary -a test fact");
            assert!(unary_and.truthy_expression_words(source).is_empty());
            assert!(unary_and.string_unary_expression_words(source).is_empty());

            let unary_or = commands
                .iter()
                .find(|(text, _)| text == "[ -o foo ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected unary -o test fact");
            assert!(unary_or.truthy_expression_words(source).is_empty());
            assert!(unary_or.string_unary_expression_words(source).is_empty());

            let mixed = commands
                .iter()
                .find(|(text, _)| text == "[ foo -o -z baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected mixed test fact");
            assert_eq!(
                mixed
                    .truthy_expression_words(source)
                    .into_iter()
                    .map(|word| word.span.slice(source).to_owned())
                    .collect::<Vec<_>>(),
                vec!["foo"]
            );
            assert_eq!(
                mixed
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("-z".to_owned(), "baz".to_owned())]
            );

            let unary_and_then_connector = commands
                .iter()
                .find(|(text, _)| text == "[ -a foo -o -z baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected unary -a with connector test fact");
            assert!(
                unary_and_then_connector
                    .truthy_expression_words(source)
                    .is_empty()
            );
            assert_eq!(
                unary_and_then_connector
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("-z".to_owned(), "baz".to_owned())]
            );

            let quoted_connector = commands
                .iter()
                .find(|(text, _)| text == "[ foo \"-o\" \"-z\" baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected quoted connector test fact");
            assert_eq!(
                quoted_connector
                    .truthy_expression_words(source)
                    .into_iter()
                    .map(|word| word.span.slice(source).to_owned())
                    .collect::<Vec<_>>(),
                vec!["foo"]
            );
            assert_eq!(
                quoted_connector
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("\"-z\"".to_owned(), "baz".to_owned())]
            );

            let chained = commands
                .iter()
                .find(|(text, _)| text == "[ -f file -a ! -z baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected chained test fact");
            assert_eq!(
                chained
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("-z".to_owned(), "baz".to_owned())]
            );

            let binary = commands
                .iter()
                .find(|(text, _)| text == "[ lhs = rhs ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected binary test fact");
            assert!(binary.truthy_expression_words(source).is_empty());
            assert!(binary.string_unary_expression_words(source).is_empty());
            assert_eq!(
                binary
                    .string_binary_expression_words(source)
                    .into_iter()
                    .map(|(left, operator, right)| {
                        (
                            left.span.slice(source).to_owned(),
                            operator.span.slice(source).to_owned(),
                            right.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("lhs".to_owned(), "=".to_owned(), "rhs".to_owned())]
            );

            let binary_then_unary = commands
                .iter()
                .find(|(text, _)| text == "[ lhs = rhs -a -z baz ]")
                .and_then(|(_, fact)| fact.simple_test())
                .expect("expected binary plus unary test fact");
            assert_eq!(
                binary_then_unary
                    .string_binary_expression_words(source)
                    .into_iter()
                    .map(|(left, operator, right)| {
                        (
                            left.span.slice(source).to_owned(),
                            operator.span.slice(source).to_owned(),
                            right.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("lhs".to_owned(), "=".to_owned(), "rhs".to_owned())]
            );
            assert_eq!(
                binary_then_unary
                    .string_unary_expression_words(source)
                    .into_iter()
                    .map(|(operator, operand)| {
                        (
                            operator.span.slice(source).to_owned(),
                            operand.span.slice(source).to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![("-z".to_owned(), "baz".to_owned())]
            );
        });
    }

    #[test]
    fn records_glued_closing_bracket_operand_spans_for_unary_tests() {
        let source = "\
#!/bin/sh
[ -d /tmp]
[ ! -a /tmp]
[ \"$dir\" = /tmp]
[ -d /tmp ]
";

        with_facts(source, None, |_, facts| {
            let commands = facts.structural_commands().collect::<Vec<_>>();
            assert_eq!(commands.len(), 4);

            assert_eq!(
                commands[0]
                    .glued_closing_bracket_operand_span()
                    .map(|span| (span.start.line, span.start.column)),
                Some((2, 6))
            );
            assert_eq!(
                commands[1]
                    .glued_closing_bracket_operand_span()
                    .map(|span| (span.start.line, span.start.column)),
                Some((3, 8))
            );
            assert_eq!(commands[2].glued_closing_bracket_operand_span(), None);
            assert_eq!(commands[3].glued_closing_bracket_operand_span(), None);
        });
    }

    #[test]
    fn collects_bare_command_name_assignment_spans() {
        let source = "\
#!/bin/sh
tool=grep
paths[$path]=set
tool=sh printf '%s\\n' hi
pager=cat \"$1\" -u perl
tool=\"grep\"
tool=git
tool=grep other=set printf '%s\\n' hi
f() {
  state=sh return 0
}
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .bare_command_name_assignment_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "tool",
                    "paths[$path]",
                    "tool=sh printf '%s\\n' hi",
                    "pager=cat \"$1\" -u perl",
                    "state=sh return 0",
                ]
            );
        });
    }

    #[test]
    fn marks_shellspec_parameter_region_empty_tests_as_suppressed() {
        let source = "\
Describe 'clone'
Parameters
  test
End

test
";

        with_facts(
            source,
            Some(Path::new(
                "/tmp/ko1nksm__shellspec__spec__core__clone_spec.sh",
            )),
            |_, facts| {
                let mut tests = facts
                    .structural_commands()
                    .filter_map(|fact| fact.simple_test().map(|simple| (fact.span(), simple)))
                    .collect::<Vec<_>>();
                tests.sort_by_key(|(span, _)| span.start.line);

                assert_eq!(tests.len(), 2);
                assert!(tests[0].1.empty_test_suppressed());
                assert!(!tests[1].1.empty_test_suppressed());
            },
        );
    }

    #[test]
    fn builds_loop_header_pipeline_and_list_facts() {
        let source = "\
#!/bin/bash
for file in $(printf '%s\\n' one two) \"$(command find . -type f)\" literal; do :; done
select choice in $(printf '%s\\n' a b) \"$(find . -type f)\" literal; do :; done
printf '%s\\n' 123 |& command kill -9 | tee out.txt
summary=$(printf '%s\\n' 456 | kill -TERM)
echo \"$(for nested in $(printf nested); do :; done)\"
true && false || printf '%s\\n' fallback
";

        with_facts(source, None, |_, facts| {
            assert_eq!(facts.for_headers().len(), 2);

            let top_level_for = &facts.for_headers()[0];
            assert!(!top_level_for.is_nested_word_command());
            assert_eq!(top_level_for.words().len(), 3);
            assert!(top_level_for.words()[0].has_unquoted_command_substitution());
            assert!(top_level_for.words()[1].contains_find_substitution());
            assert!(top_level_for.has_command_substitution());
            assert!(top_level_for.has_find_substitution());

            let nested_for = &facts.for_headers()[1];
            assert!(nested_for.is_nested_word_command());
            assert!(nested_for.words()[0].has_unquoted_command_substitution());

            let select = &facts.select_headers()[0];
            assert_eq!(select.words().len(), 3);
            assert!(select.words()[0].has_command_substitution());
            assert!(select.words()[1].contains_find_substitution());

            let pipeline_segments = facts
                .pipelines()
                .iter()
                .map(|pipeline| {
                    pipeline
                        .segments()
                        .iter()
                        .map(|segment| {
                            segment
                                .effective_or_literal_name()
                                .expect("expected normalized pipeline segment name")
                                .to_owned()
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                pipeline_segments,
                vec![
                    vec!["printf".to_owned(), "kill".to_owned(), "tee".to_owned()],
                    vec!["printf".to_owned(), "kill".to_owned()],
                ]
            );

            let first_pipeline = &facts.pipelines()[0];
            assert_eq!(
                first_pipeline
                    .operators()
                    .iter()
                    .map(|operator| operator.op())
                    .collect::<Vec<_>>(),
                vec![BinaryOp::PipeAll, BinaryOp::Pipe]
            );
            let first_segment = &first_pipeline.segments()[0];
            assert_eq!(
                facts
                    .command(first_segment.command_id())
                    .effective_or_literal_name(),
                Some("printf")
            );

            let list = facts.lists().first().expect("expected list fact");
            assert_eq!(
                list.operators()
                    .iter()
                    .map(|operator| operator.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["&&", "||"]
            );
            assert_eq!(
                list.mixed_short_circuit_span()
                    .map(|span| span.slice(source)),
                Some("&&")
            );
            assert_eq!(
                list.mixed_short_circuit_kind(),
                Some(crate::facts::MixedShortCircuitKind::Fallthrough)
            );
            assert_eq!(
                list.segments()
                    .iter()
                    .map(|segment| segment.kind())
                    .collect::<Vec<_>>(),
                vec![
                    crate::facts::ListSegmentKind::Condition,
                    crate::facts::ListSegmentKind::Condition,
                    crate::facts::ListSegmentKind::Other,
                ]
            );
        });
    }

    #[test]
    fn classifies_mixed_short_circuit_lists_by_shape() {
        let source = "\
#!/bin/sh
[ \"$x\" = foo ] && [ \"$x\" = bar ] || [ \"$x\" = baz ]
[ -n \"$x\" ] && out=foo || out=bar
[ -n \"$x\" ] || out=foo && out=bar
[ \"$dir\" = vendor ] && mv go-* \"$dir\" || mv pkg-* \"$dir\"
";

        with_facts(source, None, |_, facts| {
            assert_eq!(facts.lists().len(), 4);
            assert_eq!(
                facts
                    .lists()
                    .iter()
                    .map(|list| list.mixed_short_circuit_kind())
                    .collect::<Vec<_>>(),
                vec![
                    Some(crate::facts::MixedShortCircuitKind::TestChain),
                    Some(crate::facts::MixedShortCircuitKind::AssignmentTernary),
                    Some(crate::facts::MixedShortCircuitKind::Fallthrough),
                    Some(crate::facts::MixedShortCircuitKind::Fallthrough),
                ]
            );
        });
    }

    #[test]
    fn flagged_declaration_assignments_still_classify_as_assignment_segments() {
        let source = "\
#!/bin/bash
[ -n \"$x\" ] && declare -r out=foo || declare -r out=bar
true && declare -x flag=1
";

        with_facts(source, None, |_, facts| {
            assert_eq!(facts.lists().len(), 2);

            let ternary = &facts.lists()[0];
            assert_eq!(
                ternary.mixed_short_circuit_kind(),
                Some(crate::facts::MixedShortCircuitKind::AssignmentTernary)
            );
            assert_eq!(
                ternary
                    .segments()
                    .iter()
                    .map(|segment| segment.assignment_target())
                    .collect::<Vec<_>>(),
                vec![None, Some("out"), Some("out")]
            );

            let shortcut = &facts.lists()[1];
            assert_eq!(
                shortcut
                    .segments()
                    .iter()
                    .map(|segment| segment.kind())
                    .collect::<Vec<_>>(),
                vec![
                    crate::facts::ListSegmentKind::Condition,
                    crate::facts::ListSegmentKind::AssignmentOnly,
                ]
            );
            assert_eq!(shortcut.segments()[1].assignment_target(), Some("flag"));
        });
    }

    #[test]
    fn builds_loop_header_ls_substitution_detection() {
        let source = "\
#!/bin/bash
for entry in $(ls); do :; done
for entry in $(command ls); do :; done
for entry in $(find . -type f); do :; done
";

        with_facts(source, None, |_, facts| {
            let words = facts.for_headers()[0].words();
            assert!(words[0].has_unquoted_command_substitution());
            assert!(words[0].contains_ls_substitution());

            let command_ls = facts.for_headers()[1].words();
            assert!(command_ls[0].has_unquoted_command_substitution());
            assert!(!command_ls[0].contains_ls_substitution());

            let find_words = facts.for_headers()[2].words();
            assert!(find_words[0].has_unquoted_command_substitution());
            assert!(!find_words[0].contains_ls_substitution());
        });
    }

    #[test]
    fn zsh_for_headers_only_track_iteration_words() {
        let source = "\
#!/usr/bin/env zsh
for key value in $(printf '%s\\n' a b) literal; do :; done
for version ($versions); do :; done
";

        with_facts_dialect(
            source,
            Some(Path::new("script.zsh")),
            ParseShellDialect::Zsh,
            ShellDialect::Zsh,
            |_, facts| {
                assert_eq!(facts.for_headers().len(), 2);

                let first = &facts.for_headers()[0];
                assert_eq!(first.words().len(), 2);
                assert!(first.words()[0].has_command_substitution());
                assert_eq!(
                    first
                        .words()
                        .iter()
                        .map(|word| word.word().span.slice(source))
                        .collect::<Vec<_>>(),
                    vec!["$(printf '%s\\n' a b)", "literal"]
                );

                let second = &facts.for_headers()[1];
                assert_eq!(second.words().len(), 1);
                assert_eq!(second.words()[0].word().span.slice(source), "$versions");
            },
        );
    }

    #[test]
    fn builds_conditional_facts_with_root_normalization_and_nested_inventory() {
        let source = "\
#!/bin/bash
[[ ( ( -z foo ) ) ]]
[[ foo && -n \"$bar\" && left == right && $value =~ ^\"foo\"bar$ && left == *.sh && left == $rhs ]]
";

        with_facts(source, None, |_, facts| {
            let conditionals = facts
                .structural_commands()
                .filter_map(|fact| fact.conditional())
                .collect::<Vec<_>>();

            let root_unary = conditionals[0];
            match root_unary.root() {
                ConditionalNodeFact::Unary(unary) => {
                    assert_eq!(
                        unary.operator_family(),
                        ConditionalOperatorFamily::StringUnary
                    );
                    assert!(unary.operand().class().is_fixed_literal());
                }
                other => panic!("expected unary root, got {other:?}"),
            }

            let logical = conditionals[1];
            match logical.root() {
                ConditionalNodeFact::Binary(binary) => {
                    assert_eq!(binary.operator_family(), ConditionalOperatorFamily::Logical);
                }
                other => panic!("expected logical root, got {other:?}"),
            }

            assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::BareWord(word) if word.operand().class().is_fixed_literal())));
            assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::Unary(unary) if unary.operator_family() == ConditionalOperatorFamily::StringUnary)));
            assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::Binary(binary) if binary.operator_family() == ConditionalOperatorFamily::StringBinary && binary.right().class().is_fixed_literal())));
            assert!(logical.nodes().iter().any(|node| matches!(node, ConditionalNodeFact::Binary(binary) if binary.operator_family() == ConditionalOperatorFamily::StringBinary && !binary.right().class().is_fixed_literal())));
            assert!(logical.nodes().iter().any(|node| matches!(
                node,
                ConditionalNodeFact::Binary(binary)
                    if matches!(binary.op(), ConditionalBinaryOp::PatternEq)
                        && binary
                            .right()
                            .word()
                            .is_some_and(|word| word.span.slice(source) == "$rhs")
            )));

            let regex = logical.regex_nodes().next().expect("expected regex node");
            assert_eq!(regex.operator_family(), ConditionalOperatorFamily::Regex);
            assert_eq!(
                regex.right().word().map(|word| word.span.slice(source)),
                Some("^\"foo\"bar$")
            );
            assert!(logical.mixed_logical_operator_spans().is_empty());
            assert!(
                regex
                    .right()
                    .quote()
                    .is_some_and(|quote| quote != crate::rules::common::word::WordQuote::Unquoted)
            );
        });
    }

    #[test]
    fn tab_stripped_heredoc_substitutions_after_earlier_heredocs_keep_command_spans_intact() {
        let source = "\
#!/bin/bash
case \"${tag_type}\" in
  newest-tag)
\t:
\t;;
  latest-release-tag)
\t:
\t;;
  latest-regex)
\t:
\t;;
  *)
\ttermux_error_exit <<-EndOfError
\t\tERROR: Invalid TERMUX_PKG_UPDATE_TAG_TYPE: '${tag_type}'.
\t\tAllowed values: 'newest-tag', 'latest-release-tag', 'latest-regex'.
\tEndOfError
\t;;
esac

case \"${http_code}\" in
  404)
\ttermux_error_exit <<-EndOfError
\t\tNo '${tag_type}' found. (${api_url})
\t\tHTTP code: ${http_code}
\t\tTry using '$(
\t\t\tif [[ \"${tag_type}\" == \"newest-tag\" ]]; then
\t\t\t\techo \"latest-release-tag\"
\t\t\telse
\t\t\t\techo \"newest-tag\"
\t\t\tfi
\t\t)'.
\tEndOfError
\t;;
esac
";

        with_facts(source, None, |_, facts| {
            let conditional = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "[[ \"${tag_type}\" == \"newest-tag\" ]]")
                .expect("expected nested heredoc conditional command");

            let conditional_fact = conditional
                .conditional()
                .expect("expected conditional fact for nested heredoc command");

            match conditional_fact.root() {
                ConditionalNodeFact::Binary(binary) => {
                    assert_eq!(
                        binary.operator_family(),
                        ConditionalOperatorFamily::StringBinary
                    );
                    assert_eq!(
                        binary.left().word().map(|word| word.span.slice(source)),
                        Some("\"${tag_type}\"")
                    );
                    assert_eq!(
                        binary.right().word().map(|word| word.span.slice(source)),
                        Some("\"newest-tag\"")
                    );
                }
                other => panic!("expected binary root, got {other:?}"),
            }

            let latest_release = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "echo \"latest-release-tag\"\n")
                .expect("expected latest-release echo command");
            assert!(latest_release.simple_test().is_none());

            let newest_tag = facts
                .commands()
                .iter()
                .find(|fact| fact.span().slice(source) == "echo \"newest-tag\"\n")
                .expect("expected newest-tag echo command");
            assert!(newest_tag.simple_test().is_none());
        });
    }

    #[test]
    fn keeps_parenthesized_logical_groups_separate_for_mixed_operator_detection() {
        let source = "\
#!/bin/bash
[[ -n $a && -n $b || -n $c ]]
[[ -n $a && ( -n $b || -n $c ) ]]
[[ ( -n $a && -n $b || -n $c ) && -n $d ]]
";

        with_facts(source, None, |_, facts| {
            let conditionals = facts
                .structural_commands()
                .filter_map(|fact| fact.conditional())
                .collect::<Vec<_>>();

            assert_eq!(
                conditionals[0]
                    .mixed_logical_operator_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["||"]
            );
            assert!(conditionals[1].mixed_logical_operator_spans().is_empty());
            assert_eq!(
                conditionals[2]
                    .mixed_logical_operator_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["||"]
            );
        });
    }

    #[test]
    fn builds_conditional_portability_fact_buckets_from_shared_command_scans() {
        let source = "\
#!/bin/bash
if test left == right; then
  :
elif [[ $x == y ]]; then
  :
fi
[ left == right ]
[[ $OSTYPE == *@(linux|freebsd)* ]]
[ \"$x\" = @(foo|bar) ]
[[ $words[2] = */ ]]
[ $tools[kops] ]
[[ $x > y ]]
[[ $x =~ y ]]
[[ -v assoc[$key] ]]
[[ -a file ]]
[[ -o noclobber ]]
[ -k \"$file\" ]
test -O \"$file\"
";

        with_facts(source, None, |_, facts| {
            let portability = facts.conditional_portability();

            assert_eq!(portability.double_bracket_in_sh().len(), 8);
            assert_eq!(
                portability
                    .if_elif_bash_test()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["[[ $x == y ]]"]
            );
            assert_eq!(
                portability
                    .test_equality_operator()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["test left == right", "==", "==", "=="]
            );
            assert_eq!(
                portability
                    .extglob_in_test()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["@(linux|freebsd)", "@(foo|bar)"]
            );
            assert_eq!(
                portability
                    .array_subscript_test()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$tools[kops]"]
            );
            assert_eq!(
                portability
                    .array_subscript_condition()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$words[2]", "assoc[$key]"]
            );
            assert_eq!(
                portability
                    .greater_than_in_double_bracket()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec![">"]
            );
            assert_eq!(
                portability
                    .regex_match_in_sh()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["=~"]
            );
            assert_eq!(
                portability
                    .v_test_in_sh()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["-v"]
            );
            assert_eq!(
                portability
                    .a_test_in_sh()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["-a"]
            );
            assert_eq!(
                portability
                    .option_test_in_sh()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["-o"]
            );
            assert_eq!(
                portability
                    .sticky_bit_test_in_sh()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["-k"]
            );
            assert_eq!(
                portability
                    .ownership_test_in_sh()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["test -O \"$file\""]
            );
        });
    }

    #[test]
    fn builds_conditional_portability_pattern_buckets_from_surface_and_word_sources() {
        let source = "\
#!/bin/bash
echo @(foo|bar)
case \"$x\" in @(zip|tar)) : ;; esac
trimmed=${name%@($suffix|zz)}
echo [^a]*
trimmed=${value#[^b]*}
for item in [^c]*; do :; done
";

        with_facts(source, None, |_, facts| {
            let extglobs = facts
                .conditional_portability()
                .extglob_in_sh()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();
            assert!(extglobs.contains(&"@(foo|bar)"));
            assert!(extglobs.contains(&"@(zip|tar)"));
            assert!(extglobs.contains(&"@($suffix|zz)"));

            let caret_negations = facts
                .conditional_portability()
                .caret_negation_in_bracket()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();
            assert!(caret_negations.contains(&"[^a]"));
            assert!(caret_negations.contains(&"[^b]"));
            assert!(caret_negations.contains(&"[^c]"));
        });
    }

    #[test]
    fn builds_surface_fragment_facts_and_static_utility_names() {
        let source = "\
#!/bin/bash
echo \"prefix `date` suffix\"
echo \"$[1 + 2]\"
arr[$10]=1
declare other[$10]=1
echo \"$(( x $1 y ))\"
PS4='$prompt'
command jq '$__loc__'
test -v '$name'
printf '%s\n' $'tab\t'
echo $\"Usage: $0 {start|stop}\"
printf '%s\n' \"${!name}\" \"${!arr[*]}\"
printf '%s\n' \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${#arr[0]}\" \"${#arr[@]}\" \"${arr[0]%x}\" \"${arr[0]:2}\" \"${arr[0]//x/y}\" \"${arr[0]:-fallback}\" \"${!arr[0]}\"
printf '%s\n' \"${name:2}\" \"${1:1}\" \"${name::2}\" \"${@:1}\" \"${*:1:2}\" \"${arr[@]:1}\" \"${arr[0]:1}\"
printf '%s\n' \"${@:-fallback}\" \"${name:-$10}\"
printf '%s\n' \"${name:-${@}}\"
printf '%s\n' \"${name^}\" \"${name^^pattern}\" \"${name,}\" \"${arr[0]^^}\" \"${arr[@],,}\" \"${!name^^}\" \"${name//x/y}\"
printf '%s\n' \"${name/a/b}\" \"${name//a}\" \"${arr[0]//a/b}\" \"${arr[@]/a/b}\" \"${arr[*]//a}\" \"${!name//a/b}\"
if [ \"$(dpkg-query -W -f '${db:Status-Status}\\n' package 2>/dev/null)\" != \"installed\" ]; then :; fi
cat <<EOF
Expected: '${expected_commit::7}'
#define LAST_COMMIT_POSITION \"2311 ${GN_COMMIT:0:12}\"
Replacement: '${name//before/after}'
EOF
printf '%s\\n' 123 | command kill -9
echo \"#!/bin/bash
if [[ \"$@\" =~ x ]]; then :; fi
\"
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .backtick_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["`date`"]
            );
            assert_eq!(
                facts
                    .legacy_arithmetic_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["$[1 + 2]"]
            );
            assert_eq!(
                facts
                    .positional_parameter_fragments()
                    .iter()
                    .map(|fragment| {
                        (
                            fragment.span().slice(source),
                            fragment.kind(),
                            fragment.is_above_nine(),
                            fragment.is_guarded(),
                        )
                    })
                    .collect::<Vec<_>>(),
                vec![
                    (
                        "$10",
                        PositionalParameterFragmentKind::AboveNine,
                        true,
                        false
                    ),
                    (
                        "$10",
                        PositionalParameterFragmentKind::AboveNine,
                        true,
                        false
                    ),
                    (
                        "${@:1}",
                        PositionalParameterFragmentKind::General,
                        false,
                        false
                    ),
                    (
                        "${*:1:2}",
                        PositionalParameterFragmentKind::General,
                        false,
                        false
                    ),
                    (
                        "${@:-fallback}",
                        PositionalParameterFragmentKind::General,
                        false,
                        true
                    ),
                    (
                        "$10",
                        PositionalParameterFragmentKind::AboveNine,
                        true,
                        true
                    ),
                    (
                        "${@}",
                        PositionalParameterFragmentKind::General,
                        false,
                        true
                    ),
                ]
            );
            assert_eq!(
                facts
                    .open_double_quote_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec![""]
            );
            assert_eq!(
                facts
                    .suspect_closing_quote_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec![""]
            );
            assert_eq!(facts.positional_parameter_operator_spans().len(), 1);
            let operator_span = facts.positional_parameter_operator_spans()[0];
            assert_eq!(operator_span.start.line, 6);
            assert_eq!(operator_span.start.column, 7);
            assert_eq!(operator_span.end, operator_span.start);

            let single_quoted = facts
                .single_quoted_fragments()
                .iter()
                .map(|fragment| {
                    (
                        fragment.span().slice(source).to_owned(),
                        fragment.dollar_quoted(),
                        fragment.command_name().map(str::to_owned),
                        fragment.assignment_target().map(str::to_owned),
                        fragment.variable_set_operand(),
                    )
                })
                .collect::<Vec<_>>();
            assert!(single_quoted.iter().any(
                |(text, _, _, assignment_target, variable_set_operand)| {
                    text == "'$prompt'"
                        && assignment_target.as_deref() == Some("PS4")
                        && !variable_set_operand
                }
            ));
            assert!(single_quoted.contains(&(
                "'$__loc__'".to_owned(),
                false,
                Some("jq".to_owned()),
                None,
                false,
            )));
            assert!(single_quoted.contains(&(
                "'$name'".to_owned(),
                false,
                Some("test".to_owned()),
                None,
                true,
            )));
            assert!(single_quoted.iter().any(
                |(text, dollar_quoted, _, _, variable_set_operand)| {
                    text.starts_with("$'tab") && *dollar_quoted && !variable_set_operand
                }
            ));
            assert_eq!(
                facts
                    .dollar_double_quoted_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["$\"Usage: $0 {start|stop}\""]
            );
            assert_eq!(
                facts
                    .indirect_expansion_fragments()
                    .iter()
                    .map(|fragment| (fragment.span().slice(source), fragment.array_keys()))
                    .collect::<Vec<_>>(),
                vec![
                    ("${!name}", false),
                    ("${!arr[*]}", true),
                    ("${!arr[0]}", false),
                    ("${!name//a/b}", false),
                ]
            );
            assert_eq!(
                facts
                    .indexed_array_reference_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["${arr[0]}", "${arr[@]}", "${arr[*]}"]
            );
            assert_eq!(
                facts
                    .substring_expansion_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "${name:2}",
                    "${1:1}",
                    "${name::2}",
                    "${@:1}",
                    "${*:1:2}",
                    "${expected_commit::7}",
                    "${GN_COMMIT:0:12}",
                ]
            );
            assert_eq!(
                facts
                    .case_modification_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "${name^}",
                    "${name^^pattern}",
                    "${name,}",
                    "${arr[0]^^}",
                    "${arr[@],,}",
                ]
            );
            assert_eq!(
                facts
                    .replacement_expansion_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "${arr[0]//x/y}",
                    "${name//x/y}",
                    "${name/a/b}",
                    "${name//a}",
                    "${arr[0]//a/b}",
                    "${arr[@]/a/b}",
                    "${arr[*]//a}",
                    "${name//before/after}",
                ]
            );

            let jq = facts
                .structural_commands()
                .find(|fact| fact.static_utility_name_is("jq"))
                .expect("expected jq command fact");
            assert_eq!(jq.static_utility_name(), Some("jq"));

            let tail = facts
                .pipelines()
                .first()
                .and_then(|pipeline| pipeline.last_segment())
                .expect("expected pipeline tail");
            assert_eq!(tail.static_utility_name(), Some("kill"));
            assert!(tail.static_utility_name_is("kill"));
        });
    }

    #[test]
    fn open_double_quote_surface_facts_track_live_expansion_gaps() {
        let source = "\
#!/bin/bash
echo \"#!/bin/bash

# LLVMFuzzerTestOneInput for fuzzer detection.
this_dir=\\$(dirname \"\\$0\")
if [[ \"\\$@\" =~ (^| )-runs=[0-9]+($| ) ]]
then
  mem_settings='-Xmx1900m:-Xss900k'
else
  mem_settings='-Xmx2048m:-Xss1024k'
fi

LD_LIBRARY_PATH=\"$JVM_LD_LIBRARY_PATH\":\\$this_dir \\
  \\$this_dir/jazzer_driver                        \\
  --agent_path=\\$this_dir/jazzer_agent_deploy.jar \\
  --cp=$RUNTIME_CLASSPATH                         \\
  --target_class=$fuzzer_basename                 \\
  --jvm_args=\"\\$mem_settings\"                     \\
  \\$@\" > $OUT/$fuzzer_basename
";

        with_facts(source, None, |_, facts| {
            let open = facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();
            let close = facts
                .suspect_closing_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();

            assert_eq!(open, vec![(6, 11)]);
            assert_eq!(close, vec![(13, 17)]);
        });
    }

    #[test]
    fn open_double_quote_surface_facts_ignore_escaped_literal_gaps() {
        let source = "\
#!/bin/bash
echo \"#!/bin/bash
# LLVMFuzzerTestOneInput for fuzzer detection.
this_dir=\\$(dirname \"\\$0\")
mem_settings='-Xmx2048m:-Xss1024k'
if [[ \"\\$@\" =~ (^| )-runs=[0-9]+($| ) ]]; then
  mem_settings='-Xmx1900m:-Xss900k'
fi
LD_LIBRARY_PATH=\\\"\\$JVM_LD_LIBRARY_PATH\\\":\\$this_dir \\
\\$this_dir/jazzer_driver --agent_path=\\$this_dir/jazzer_agent_deploy.jar \\
--cp=$RUNTIME_CLASSPATH \\
--target_class=$fuzzer_basename \\
--jvm_args=\"\\$mem_settings\" \\
\"\\$@\"\" > $OUT/$fuzzer_basename
";

        with_facts(source, None, |_, facts| {
            assert!(facts.open_double_quote_fragments().is_empty());
            assert!(facts.suspect_closing_quote_fragments().is_empty());
        });
    }

    #[test]
    fn open_double_quote_surface_facts_track_literal_gap_fragments() {
        let source = "\
#!/bin/sh
echo \"help text
say \"configure\" now
\"
";

        with_facts(source, None, |_, facts| {
            let open = facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();
            let close = facts
                .suspect_closing_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();

            assert_eq!(open, vec![(2, 6)]);
            assert_eq!(close, vec![(3, 5)]);
        });
    }

    #[test]
    fn open_double_quote_surface_facts_track_backslash_prefixed_literal_gap_fragments() {
        let source = "\
#!/bin/sh
echo \"line one
line two\"\\foo\"tail\"
";

        with_facts(source, None, |_, facts| {
            let open = facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();
            let close = facts
                .suspect_closing_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();

            assert_eq!(open, vec![(2, 6)]);
            assert_eq!(close, vec![(3, 9)]);
        });
    }

    #[test]
    fn open_double_quote_surface_facts_ignore_empty_prefix_multiline_quotes_with_literal_suffix() {
        let source = "\
#!/bin/sh
echo \"\"\"line one
line two\"suffix
";

        with_facts(source, None, |_, facts| {
            assert!(facts.open_double_quote_fragments().is_empty());
            assert!(facts.suspect_closing_quote_fragments().is_empty());
        });
    }

    #[test]
    fn open_double_quote_surface_facts_ignore_valid_multiline_quotes_with_suffix_expansion() {
        let source = "\
#!/bin/sh
echo \"line one
line two\"$suffix
";

        with_facts(source, None, |_, facts| {
            assert!(facts.open_double_quote_fragments().is_empty());
            assert!(facts.suspect_closing_quote_fragments().is_empty());
        });
    }

    #[test]
    fn open_double_quote_surface_facts_ignore_empty_prefix_multiline_quotes_with_suffix_expansion()
    {
        let source = "\
#!/bin/sh
echo \"\"\"line one
line two\"$suffix
";

        with_facts(source, None, |_, facts| {
            assert!(facts.open_double_quote_fragments().is_empty());
            assert!(facts.suspect_closing_quote_fragments().is_empty());
        });
    }

    #[test]
    fn open_double_quote_surface_facts_report_only_first_fragment_per_word() {
        let source = "\
#!/bin/sh
echo \"help text
say \"configure\" now
then \"install\" later
\"\"\"
";

        with_facts(source, None, |_, facts| {
            let open = facts
                .open_double_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();
            let close = facts
                .suspect_closing_quote_fragments()
                .iter()
                .map(|fragment| (fragment.span().start.line, fragment.span().start.column))
                .collect::<Vec<_>>();

            assert_eq!(open, vec![(2, 6)]);
            assert_eq!(close, vec![(3, 5)]);
        });
    }

    #[test]
    fn builds_double_paren_grouping_spans() {
        let source = "\
#!/bin/sh
((ps aux | grep foo) || kill \"$pid\") 2>/dev/null
(( i += 1 ))
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .double_paren_grouping_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["p"]
            );
        });
    }

    #[test]
    fn builds_unicode_smart_quote_spans_for_unquoted_words() {
        let source = "\
#!/bin/sh
echo “hello”
echo \"hello “world”\"
echo 'hello ‘world’'
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .unicode_smart_quote_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["“", "”"]
            );
        });
    }

    #[test]
    fn traces_case_pattern_spans_for_escaped_char_classes() {
        let source = "\
#!/bin/sh
case x in *[!a-zA-Z0-9._/+\\-]*) continue ;; esac
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .pattern_literal_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["*[!a-zA-Z0-9._/+\\-]*"]
            );
            assert!(facts.pattern_charclass_spans().is_empty());
        });
    }

    #[test]
    fn marks_subscript_index_references_without_span_scanning() {
        let source = "#!/bin/bash\nprintf '%s\\n' \"${arr[$idx]}\" \"$free\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let idx_reference = semantic
            .references()
            .iter()
            .find(|reference| reference.name.as_str() == "idx")
            .expect("expected idx reference");
        let free_reference = semantic
            .references()
            .iter()
            .find(|reference| reference.name.as_str() == "free")
            .expect("expected free reference");

        assert!(facts.is_subscript_index_reference(idx_reference.span));
        assert!(!facts.is_subscript_index_reference(free_reference.span));
    }

    #[test]
    fn tracks_command_prefix_assignments_reused_later_in_the_same_command() {
        let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --with-optmizer=${CFLAGS}
PATH=/tmp \"$PATH\"/bin/tool
A=1 B=\"$A\" C=\"$B\" cmd
foo=\"$foo\" bar=\"$foo\" cmd
foo=1 export \"$foo\"
foo=1 bar[$foo]=x cmd
FOO=tmp cmd >\"$FOO\"
foo=\"$foo\" cmd
foo=1 cmd \"$(printf %s \"$foo\")\"
foo=1 foo=2 cmd
foo=1 bar=\"$foo\"
COUNTDOWN=$[ $COUNTDOWN - 1 ]
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo \"$COUNTDOWN\"
X=1 A=$[ $X + 1 ] true
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .env_prefix_assignment_scope_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "CFLAGS",
                    "PATH",
                    "A",
                    "B",
                    "foo",
                    "foo",
                    "foo",
                    "FOO",
                    "COUNTDOWN",
                    "X"
                ]
            );
        });
    }

    #[test]
    fn builds_word_facts_with_contexts_hosts_and_anchor_spans() {
        let source = "\
#!/bin/bash
case literal in
  @($pat|$(printf '%s' case))) : ;;
esac
trap \"echo $x $(date)\" EXIT
declare declared[$(printf decl-name-subscript)]
declare arr[$(printf decl-subscript)]=\"${name%$suffix}\"
target[$(printf assign-subscript)]=1
declare -A map=([$(printf key-subscript)]=1)
[[ -v assoc[\"$(printf cond-subscript)\"] ]]
printf '%s\\n' prefix${name}suffix ${items[@]}
";

        with_facts(source, None, |_, facts| {
            let case_subject = facts
                .case_subject_facts()
                .find(|fact| fact.span().slice(source) == "literal")
                .expect("expected case subject fact");
            assert!(case_subject.is_case_subject());
            assert!(case_subject.classification().is_fixed_literal());

            let trap_action = facts
                .expansion_word_facts(ExpansionContext::TrapAction)
                .next()
                .expect("expected trap action fact");
            assert_eq!(
                trap_action
                    .double_quoted_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$x", "$(date)"]
            );

            let declaration_name_subscript = facts
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
                .find(|fact| fact.span().slice(source) == "$(printf decl-name-subscript)")
                .expect("expected declaration name subscript fact");
            assert_eq!(
                declaration_name_subscript.host_kind(),
                WordFactHostKind::DeclarationNameSubscript
            );
            assert_eq!(
                declaration_name_subscript
                    .command_substitution_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$(printf decl-name-subscript)"]
            );

            let declaration_assignment_subscript = facts
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
                .find(|fact| fact.span().slice(source) == "$(printf decl-subscript)")
                .expect("expected declaration assignment subscript fact");
            assert_eq!(
                declaration_assignment_subscript.host_kind(),
                WordFactHostKind::AssignmentTargetSubscript
            );

            let assignment_subscript = facts
                .expansion_word_facts(ExpansionContext::AssignmentValue)
                .find(|fact| fact.span().slice(source) == "$(printf assign-subscript)")
                .expect("expected assignment subscript fact");
            assert_eq!(
                assignment_subscript.host_kind(),
                WordFactHostKind::AssignmentTargetSubscript
            );

            let array_key = facts
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
                .find(|fact| fact.span().slice(source) == "$(printf key-subscript)")
                .expect("expected array key fact");
            assert_eq!(array_key.host_kind(), WordFactHostKind::ArrayKeySubscript);

            let conditional_subscript = facts
                .expansion_word_facts(ExpansionContext::ConditionalVarRefSubscript)
                .find(|fact| fact.span().slice(source) == "\"$(printf cond-subscript)\"")
                .expect("expected conditional subscript fact");
            assert_eq!(
                conditional_subscript.host_kind(),
                WordFactHostKind::ConditionalVarRefSubscript
            );

            let parameter_pattern = facts
                .expansion_word_facts(ExpansionContext::ParameterPattern)
                .find(|fact| fact.span().slice(source) == "$suffix")
                .expect("expected parameter pattern fact");
            assert!(parameter_pattern.classification().is_expanded());
            assert_eq!(
                facts
                    .expansion_word_facts(ExpansionContext::ParameterPattern)
                    .filter(|fact| fact.span().slice(source) == "$suffix")
                    .count(),
                1
            );

            let scalar = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "prefix${name}suffix")
                .expect("expected mixed command argument fact");
            assert!(scalar.has_literal_affixes());
            assert_eq!(
                scalar
                    .scalar_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["${name}"]
            );

            let array = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "${items[@]}")
                .expect("expected array argument fact");
            assert_eq!(
                array
                    .unquoted_array_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["${items[@]}"]
            );
        });
    }

    #[test]
    fn collects_dollar_spans_for_wrapped_substring_offset_arithmetic() {
        let source =
            "#!/bin/bash\nrest=abcdef\nlen=2\nprintf '%s\\n' \"${rest:$((${#rest}-$len))}\"\n";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .dollar_in_arithmetic_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();
            let words = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .map(|fact| {
                    format!(
                        "{} {:?} {:?}",
                        fact.span().slice(source),
                        fact.host_kind(),
                        fact.word().parts
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["$len"], "command words: {words:?}");
        });
    }

    #[test]
    fn collects_dollar_spans_for_wrapped_substring_length_arithmetic() {
        let source = "#!/bin/bash\nstring=abcdef\nwidth=10\nprintf '%s\\n' \"${string:0:$(( $width - 4 ))}\"\n";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .dollar_in_arithmetic_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();
            let words = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .map(|fact| {
                    format!(
                        "{} {:?} {:?}",
                        fact.span().slice(source),
                        fact.host_kind(),
                        fact.word().parts
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["$width"], "command words: {words:?}");
        });
    }

    #[test]
    fn collects_dollar_spans_for_parameter_replacement_arithmetic() {
        let source = "#!/bin/bash\noffset=1\nindex=2\nline=x\necho \"${line/ $index / $(($offset + $index)) }\"\n";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .dollar_in_arithmetic_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();
            let words = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .map(|fact| {
                    format!(
                        "{} {:?} {:?}",
                        fact.span().slice(source),
                        fact.host_kind(),
                        fact.word().parts
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["$offset", "$index"], "command words: {words:?}");
        });
    }

    #[test]
    fn collects_command_substitution_spans_for_wrapped_substring_offset_arithmetic() {
        let source =
            "#!/bin/bash\nrest=abcdef\nprintf '%s\\n' \"${rest:$((${#rest}-$(printf 1)))}\"\n";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .arithmetic_command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["$(printf 1)"]);
        });
    }

    #[test]
    fn ignores_quoted_dollar_words_in_arithmetic_command_contexts() {
        let source = "#!/bin/bash\n(( \"$x\" + 1 ))\nfor (( i=\"$y\"; i < 3; i++ )); do :; done\n";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .dollar_in_arithmetic_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();

            assert!(spans.is_empty(), "unexpected spans: {spans:?}");
        });
    }

    #[test]
    fn ignores_escaped_command_substitution_tokens_in_wrapped_substring_offset_arithmetic() {
        let source = "#!/bin/bash\ns=abcdef\ni=1\nprintf '%s\\n' \"${s:$(($i+\\$(printf 1)))}\"\n";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .arithmetic_command_substitution_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>();

            assert!(spans.is_empty(), "unexpected spans: {spans:?}");
        });
    }

    #[test]
    fn builds_word_facts_for_zsh_qualified_globs() {
        let source = "#!/usr/bin/env zsh\nprint -- prefix*(.N)\n";

        with_facts_dialect(
            source,
            None,
            ParseShellDialect::Zsh,
            ShellDialect::Zsh,
            |_, facts| {
                let glob = facts
                    .expansion_word_facts(ExpansionContext::CommandArgument)
                    .find(|fact| fact.span().slice(source) == "prefix*(.N)")
                    .expect("expected zsh glob fact");

                assert!(glob.classification().is_expanded());
                assert!(glob.analysis().hazards.pathname_matching);
            },
        );
    }

    #[test]
    fn builds_word_facts_for_special_parameter_arguments() {
        let source = "\
#!/bin/bash
printf '%s\\n' $0 $1 $* $@
";

        with_facts(source, None, |_, facts| {
            let argument_words = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .map(|fact| fact.span().slice(source).to_owned())
                .collect::<Vec<_>>();

            assert!(argument_words.contains(&"$0".to_owned()));
            assert!(argument_words.contains(&"$1".to_owned()));
            assert!(argument_words.contains(&"$*".to_owned()));
            assert!(argument_words.contains(&"$@".to_owned()));
        });
    }

    #[test]
    fn builds_word_facts_for_filename_builder_command_substitutions() {
        let source = "\
#!/bin/bash
/sbin/makepkg -l y -c n $OUTPUT/$PRGNAM-$VERSION\\_$(echo ${KERNEL} | tr '-' '_')-$ARCH-$BUILD$TAG.$PKGTYPE
";

        with_facts(source, None, |_, facts| {
            let fact = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| {
                    fact.span()
                        .slice(source)
                        .contains("$(echo ${KERNEL} | tr '-' '_')")
                })
                .expect("expected makepkg output argument fact");

            assert_eq!(
                fact.unquoted_command_substitution_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$(echo ${KERNEL} | tr '-' '_')"],
                "parts: {:?}",
                fact.word().parts
            );
        });
    }

    #[test]
    fn builds_word_facts_for_quoted_all_elements_array_expansions() {
        let source = "\
#!/bin/bash
eval \"${shims[@]}\"
";

        with_facts(source, None, |_, facts| {
            let fact = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source) == "\"${shims[@]}\"")
                .expect("expected eval argument fact");

            assert_eq!(
                fact.all_elements_array_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["${shims[@]}"],
                "parts: {:?}",
                fact.word().parts
            );
        });
    }

    #[test]
    fn builds_word_facts_for_conditional_patterns() {
        let source = "\
#!/bin/bash
if [[ x == *${shims[@]}* ]]; then :; fi
";

        with_facts(source, None, |_, facts| {
            let fact = facts
                .expansion_word_facts(ExpansionContext::ConditionalPattern)
                .find(|fact| fact.span().slice(source) == "${shims[@]}")
                .expect("expected conditional pattern word fact");

            assert_eq!(
                fact.all_elements_array_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["${shims[@]}"]
            );
        });
    }

    #[test]
    fn builds_word_facts_for_mixed_quoted_all_elements_array_expansions() {
        let source = "\
#!/bin/bash
shims=(a)
eval \"conda_shim() { case \\\"\\${1##*/}\\\" in ${shims[@]} *) return 1;; esac }\"
";

        with_facts(source, None, |_, facts| {
            let fact = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .find(|fact| fact.span().slice(source).contains("${shims[@]}"))
                .expect("expected eval argument fact");

            assert_eq!(
                fact.all_elements_array_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["${shims[@]}"]
            );
        });
    }

    #[test]
    fn builds_word_facts_for_unquoted_all_elements_array_expansions() {
        let source = "\
#!/bin/bash
printf '%s\\n' $@ ${@:2} ${items[@]} ${items[@]:1} ${!items[@]} ${items[@]/#/#} ${items[@]@Q} ${items[@]:-fallback} ${items[@]:+fallback} \"$@\" \"${items[@]}\" $* ${items[*]} ${1+\"$@\"}
";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .flat_map(|fact| {
                    fact.unquoted_all_elements_array_expansion_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(
                spans,
                vec![
                    "$@",
                    "${@:2}",
                    "${items[@]}",
                    "${items[@]:1}",
                    "${!items[@]}",
                    "${items[@]/#/#}",
                    "${items[@]@Q}",
                    "${items[@]:-fallback}"
                ]
            );
        });
    }

    #[test]
    fn builds_word_facts_for_unquoted_literals_between_reopened_double_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"foo\"bar\"baz\" \"foo\"-\"bar\" \"foo\"$(printf '%s' x)\"bar\" \"$left\"-\"$right\" x=\"$(cmd \"a\".\"b\")\" '$('\"foo\"parenmid\"baz\" '${'\"foo\"bracemid\"baz\"
";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
                .flat_map(|fact| {
                    fact.unquoted_literal_between_double_quoted_segments_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["bar", "-", "parenmid", "bracemid", "."]);
        });
    }

    #[test]
    fn builds_word_facts_ignore_comment_text_in_nested_fragment_scan() {
        let source = "\
#!/bin/bash
echo $(echo x # $(
 )\"foo\"bar\"baz\"
";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .flat_map(|fact| {
                    fact.unquoted_literal_between_double_quoted_segments_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["bar"]);
        });
    }

    #[test]
    fn builds_word_facts_ignore_comment_text_in_backtick_fragment_scan() {
        let source = "\
#!/bin/bash
echo `echo x # $(
`\"foo\"bar\"baz\"
";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .flat_map(|fact| {
                    fact.unquoted_literal_between_double_quoted_segments_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["bar"]);
        });
    }

    #[test]
    fn builds_word_facts_ignore_hashes_inside_nested_double_quotes() {
        let source = "\
#!/bin/bash
echo $(printf \"%s\" \"x # $(printf y)\")\"foo\"bar\"baz\"
";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .flat_map(|fact| {
                    fact.unquoted_literal_between_double_quoted_segments_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["bar"]);
        });
    }

    #[test]
    fn builds_word_facts_ignore_comment_text_in_process_substitution_scan() {
        let source = "\
#!/bin/bash
echo <(echo x # ${
 )\"foo\"bar\"baz\"
";

        with_facts(source, None, |_, facts| {
            let spans = facts
                .expansion_word_facts(ExpansionContext::CommandArgument)
                .flat_map(|fact| {
                    fact.unquoted_literal_between_double_quoted_segments_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            assert_eq!(spans, vec!["bar"]);
        });
    }

    #[test]
    fn builds_array_assignment_split_word_facts() {
        let source = "\
#!/bin/bash
scalar=$x
arr=($x \"$y\" prefix$z $(cmd) \"${items[@]}\" ${items[@]})
declare declared=($alpha \"$(cmd)\" ${beta})
declare -A map=([k]=$v)
arr+=($tail)
";

        with_facts(source, None, |_, facts| {
            let split_words = facts
                .array_assignment_split_word_facts()
                .map(|fact| fact.span().slice(source).to_owned())
                .collect::<Vec<_>>();

            assert_eq!(
                split_words,
                vec![
                    "$x",
                    "\"$y\"",
                    "prefix$z",
                    "$(cmd)",
                    "\"${items[@]}\"",
                    "${items[@]}",
                    "$alpha",
                    "\"$(cmd)\"",
                    "${beta}",
                    "$tail",
                ]
            );

            let unquoted_scalar = facts
                .array_assignment_split_word_facts()
                .flat_map(|fact| {
                    fact.unquoted_scalar_expansion_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                unquoted_scalar,
                vec!["$x", "$z", "$alpha", "${beta}", "$tail"]
            );

            let unquoted_array = facts
                .array_assignment_split_word_facts()
                .flat_map(|fact| {
                    fact.unquoted_array_expansion_spans()
                        .iter()
                        .map(|span| span.slice(source).to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(unquoted_array, vec!["${items[@]}"]);
        });
    }

    #[test]
    fn array_assignment_split_facts_track_command_substitution_boundaries() {
        let source = "\
#!/bin/bash
arr=(\"$(printf '%s\\n' \"$x\")\")
";

        with_facts(source, None, |_, facts| {
            let split_facts = facts
                .array_assignment_split_word_facts()
                .collect::<Vec<_>>();
            assert_eq!(split_facts.len(), 1);
            let fact = split_facts[0];

            assert_eq!(
                fact.command_substitution_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$(printf '%s\\n' \"$x\")"]
            );
            assert_eq!(
                fact.unquoted_scalar_expansion_spans()
                    .iter()
                    .map(|span| span.slice(source))
                    .collect::<Vec<_>>(),
                vec!["$x"]
            );
        });
    }

    #[test]
    fn array_assignment_split_facts_keep_heredoc_substitutions_as_single_words() {
        let source = "\
#!/bin/bash
arr=(\"$(
  cat <<-EOF
    repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\")
EOF
)\")
";

        with_facts(source, None, |_, facts| {
            let split_words = facts
                .array_assignment_split_word_facts()
                .map(|fact| fact.span().slice(source).to_owned())
                .collect::<Vec<_>>();

            assert_eq!(
                split_words,
                vec![
                    "\"$(\n  cat <<-EOF\n    repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\")\nEOF\n)\""
                ]
            );
        });
    }

    #[test]
    fn array_assignment_split_facts_keep_pipelined_heredoc_substitutions_as_single_words() {
        let source = "\
#!/bin/bash
arr=(\"$(
  cat <<-EOF | tr '\\n' ' '
    {
      \\\"query\\\": \\\"query {
        repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {
          refs(refPrefix: \\\"refs/tags/\\\")
        }
      }\\\"
    }
EOF
)\")
";

        with_facts(source, None, |_, facts| {
            let split_words = facts
                .array_assignment_split_word_facts()
                .map(|fact| fact.span().slice(source).to_owned())
                .collect::<Vec<_>>();

            assert_eq!(
                split_words,
                vec![
                    "\"$(\n  cat <<-EOF | tr '\\n' ' '\n    {\n      \\\"query\\\": \\\"query {\n        repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {\n          refs(refPrefix: \\\"refs/tags/\\\")\n        }\n      }\\\"\n    }\nEOF\n)\""
                ]
            );
        });
    }

    #[test]
    fn array_assignment_split_facts_track_realistic_pipelined_heredoc_substitutions() {
        let source = r#"# shellcheck shell=bash
project=owner/repo
graphql_request=(
  -X POST
  -d "$(
    cat <<-EOF | tr '\n' ' '
      {
        "query": "query {
          repository(owner: \"${project%/*}\", name: \"${project##*/}\") {
            refs(refPrefix: \"refs/tags/\")
          }
        }"
      }
EOF
  )"
)
"#;

        with_facts(source, None, |_, facts| {
            let split_facts = facts
                .array_assignment_split_word_facts()
                .collect::<Vec<_>>();

            assert_eq!(
                split_facts
                    .iter()
                    .map(|fact| fact.span().slice(source))
                    .collect::<Vec<_>>(),
                vec![
                    "-X",
                    "POST",
                    "-d",
                    "\"$(\n    cat <<-EOF | tr '\\n' ' '\n      {\n        \"query\": \"query {\n          repository(owner: \\\"${project%/*}\\\", name: \\\"${project##*/}\\\") {\n            refs(refPrefix: \\\"refs/tags/\\\")\n          }\n        }\"\n      }\nEOF\n  )\"",
                ]
            );
        });
    }

    #[test]
    fn surface_facts_track_parameter_operations_in_expanding_heredocs() {
        let source = "\
cat <<EOF
${name:2}
${arr[0]//x/y}
${name^^pattern}
EOF
";

        with_facts(source, None, |_, facts| {
            assert_eq!(
                facts
                    .substring_expansion_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["${name:2}"]
            );
            assert_eq!(
                facts
                    .replacement_expansion_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["${arr[0]//x/y}"]
            );
            assert_eq!(
                facts
                    .case_modification_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["${name^^pattern}"]
            );
        });
    }

    #[test]
    fn shared_command_traversal_collects_word_facts_and_surface_fragments() {
        let source = "\
#!/bin/bash
printf '%s\\n' ${name%$suffix} `printf backtick`
";

        with_facts(source, None, |_, facts| {
            let parameter_pattern = facts
                .expansion_word_facts(ExpansionContext::ParameterPattern)
                .find(|fact| fact.span().slice(source) == "$suffix")
                .expect("expected parameter pattern fact");
            assert_eq!(parameter_pattern.host_kind(), WordFactHostKind::Direct);

            assert_eq!(
                facts
                    .backtick_fragments()
                    .iter()
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["`printf backtick`"]
            );
        });
    }
}
