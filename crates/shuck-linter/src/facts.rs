//! Linter-owned structural facts built once per file.
//!
//! `SemanticModel` remains the source of truth for bindings, references, scopes,
//! source references, the call graph, and flow-sensitive facts.
//! `LinterFacts` owns reusable linter-side summaries that are cheaper to build
//! once than to recompute in every rule: normalized commands, wrapper chains,
//! declaration summaries, option-shape summaries, and later word/expansion
//! facts.

mod command_flow;
mod presence;
mod surface;

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem,
    Assignment, AssignmentValue, BinaryCommand, BinaryOp, BourneParameterExpansion, BuiltinCommand,
    CaseItem, CaseTerminator, Command, CommandSubstitutionSyntax, CompoundCommand,
    ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, DeclClause, DeclOperand, File,
    ForCommand, FunctionDef, Name, ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart,
    Position, Redirect, RedirectKind, SelectCommand, SimpleCommand, SourceText, Span, Stmt,
    StmtSeq, Subscript, VarRef, Word, WordPart, WordPartNode, ZshExpansionTarget, ZshGlobSegment,
    ZshQualifiedGlob,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use shuck_semantic::SemanticModel;
use std::borrow::Cow;

use self::{
    command_flow::{
        build_case_item_facts, build_for_header_facts, build_list_facts, build_pipeline_facts,
        build_select_header_facts, build_single_test_subshell_spans,
        build_subshell_test_group_spans, build_substitution_facts,
    },
    presence::build_presence_tested_names,
    surface::{build_subscript_index_reference_spans, build_surface_fragment_facts},
};
use crate::FileContext;
use crate::context::ContextRegionKind;
use crate::rules::common::expansion::{
    ExpansionAnalysis, ExpansionContext, RedirectTargetAnalysis, SubstitutionOutputIntent,
    WordExpansionKind, WordLiteralness, WordSubstitutionShape, analyze_literal_runtime,
    analyze_redirect_target, analyze_word,
};
use crate::rules::common::{
    command::{self, NormalizedCommand, WrapperKind},
    query::{self, CommandSubstitutionKind, CommandVisit, CommandWalkOptions},
    span,
    word::{
        TestOperandClass, WordClassification, WordQuote, classify_conditional_operand,
        classify_contextual_operand, classify_word, static_word_text,
    },
};

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

    pub fn operand_classes(&self) -> &[TestOperandClass] {
        &self.operand_classes
    }

    pub fn operand_class(&self, index: usize) -> Option<TestOperandClass> {
        self.operand_classes.get(index).copied()
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

    pub fn binary_operand_classes(&self) -> Option<(TestOperandClass, TestOperandClass)> {
        (self.shape == SimpleTestShape::Binary)
            .then(|| Some((self.operand_class(0)?, self.operand_class(2)?)))
            .flatten()
    }
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

#[derive(Debug, Clone)]
pub struct SingleQuotedFragmentFact {
    span: Span,
    command_name: Option<Box<str>>,
    assignment_target: Option<Box<str>>,
    variable_set_operand: bool,
}

impl SingleQuotedFragmentFact {
    pub fn span(&self) -> Span {
        self.span
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
pub struct OpenDoubleQuoteFragmentFact {
    span: Span,
}

impl OpenDoubleQuoteFragmentFact {
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

#[derive(Debug, Clone, Copy)]
pub struct PositionalParameterFragmentFact {
    span: Span,
}

impl PositionalParameterFragmentFact {
    pub fn span(&self) -> Span {
        self.span
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactContext {
    Expansion(ExpansionContext),
    CaseSubject,
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
    analysis: ExpansionAnalysis,
    operand_class: Option<TestOperandClass>,
    static_text: Option<Box<str>>,
    has_literal_affixes: bool,
    scalar_expansion_spans: Box<[Span]>,
    array_expansion_spans: Box<[Span]>,
    unquoted_array_expansion_spans: Box<[Span]>,
    command_substitution_spans: Box<[Span]>,
    unquoted_command_substitution_spans: Box<[Span]>,
    double_quoted_expansion_spans: Box<[Span]>,
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
        }
    }

    pub fn is_case_subject(&self) -> bool {
        self.context == WordFactContext::CaseSubject
    }

    pub fn host_kind(&self) -> WordFactHostKind {
        self.host_kind
    }

    pub fn analysis(&self) -> ExpansionAnalysis {
        self.analysis
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

    pub fn scalar_expansion_spans(&self) -> &[Span] {
        &self.scalar_expansion_spans
    }

    pub fn array_expansion_spans(&self) -> &[Span] {
        &self.array_expansion_spans
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionHostKind {
    CommandArgument,
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
    stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
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

    pub fn stdout_intent(&self) -> SubstitutionOutputIntent {
        self.stdout_intent
    }

    pub fn has_stdout_redirect(&self) -> bool {
        self.has_stdout_redirect
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
pub struct FunctionHeaderFact<'a> {
    function: &'a FunctionDef,
}

impl<'a> FunctionHeaderFact<'a> {
    pub fn function(&self) -> &'a FunctionDef {
        self.function
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

#[derive(Debug, Clone)]
pub struct ListFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    operators: Box<[ListOperatorFact]>,
    mixed_short_circuit_span: Option<Span>,
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

    pub fn mixed_short_circuit_span(&self) -> Option<Span> {
        self.mixed_short_circuit_span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReadCommandFacts {
    pub uses_raw_input: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PrintfCommandFacts<'a> {
    pub format_word: Option<&'a Word>,
}

#[derive(Debug, Clone)]
pub struct UnsetCommandFacts<'a> {
    pub function_mode: bool,
    operand_words: Box<[&'a Word]>,
    options_parseable: bool,
}

impl<'a> UnsetCommandFacts<'a> {
    pub fn operand_words(&self) -> &[&'a Word] {
        &self.operand_words
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

#[derive(Debug, Clone, Copy)]
pub struct FindCommandFacts {
    pub has_print0: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct XargsCommandFacts {
    pub uses_null_input: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct GrepCommandFacts {
    pub uses_only_matching: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct SetCommandFacts {
    pub errexit_change: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExprCommandFacts {
    pub uses_arithmetic_operator: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ExitCommandFacts<'a> {
    pub status_word: Option<&'a Word>,
    pub is_numeric_literal: bool,
    status_is_static: bool,
}

impl<'a> ExitCommandFacts<'a> {
    pub fn has_static_status(self) -> bool {
        self.status_is_static
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SudoFamilyCommandFacts {
    pub invoker: SudoFamilyInvoker,
}

#[derive(Debug, Clone, Default)]
pub struct CommandOptionFacts<'a> {
    read: Option<ReadCommandFacts>,
    printf: Option<PrintfCommandFacts<'a>>,
    unset: Option<UnsetCommandFacts<'a>>,
    find: Option<FindCommandFacts>,
    xargs: Option<XargsCommandFacts>,
    grep: Option<GrepCommandFacts>,
    set: Option<SetCommandFacts>,
    expr: Option<ExprCommandFacts>,
    exit: Option<ExitCommandFacts<'a>>,
    sudo_family: Option<SudoFamilyCommandFacts>,
}

impl<'a> CommandOptionFacts<'a> {
    pub fn read(&self) -> Option<&ReadCommandFacts> {
        self.read.as_ref()
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

    pub fn xargs(&self) -> Option<&XargsCommandFacts> {
        self.xargs.as_ref()
    }

    pub fn grep(&self) -> Option<&GrepCommandFacts> {
        self.grep.as_ref()
    }

    pub fn set(&self) -> Option<&SetCommandFacts> {
        self.set.as_ref()
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

    fn build(command: &'a Command, normalized: &NormalizedCommand<'a>, source: &str) -> Self {
        Self {
            read: normalized
                .effective_name_is("read")
                .then(|| ReadCommandFacts {
                    uses_raw_input: read_uses_raw_input(normalized.body_args(), source),
                }),
            printf: normalized
                .effective_name_is("printf")
                .then(|| PrintfCommandFacts {
                    format_word: printf_format_word(normalized.body_args(), source),
                }),
            unset: normalized
                .effective_name_is("unset")
                .then(|| parse_unset_command(normalized.body_args(), source)),
            find: normalized
                .effective_name_is("find")
                .then(|| FindCommandFacts {
                    has_print0: normalized
                        .body_args()
                        .iter()
                        .filter_map(|word| static_word_text(word, source))
                        .any(|arg| arg == "-print0"),
                }),
            xargs: normalized
                .effective_name_is("xargs")
                .then(|| XargsCommandFacts {
                    uses_null_input: normalized
                        .body_args()
                        .iter()
                        .filter_map(|word| static_word_text(word, source))
                        .any(|arg| {
                            arg == "--null"
                                || (arg.starts_with('-')
                                    && !arg.starts_with("--")
                                    && arg[1..].contains('0'))
                        }),
                }),
            grep: normalized
                .effective_name_is("grep")
                .then(|| parse_grep_command(normalized.body_args(), source))
                .flatten(),
            set: normalized
                .effective_name_is("set")
                .then(|| parse_set_command(normalized.body_args(), source)),
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
    redirect_facts: Box<[RedirectFact<'a>]>,
    substitution_facts: Box<[SubstitutionFact]>,
    options: CommandOptionFacts<'a>,
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
}

#[derive(Debug, Clone)]
pub struct LinterFacts<'a> {
    commands: Vec<CommandFact<'a>>,
    structural_command_ids: Vec<CommandId>,
    #[cfg_attr(not(test), allow(dead_code))]
    command_ids_by_span: CommandLookupIndex,
    elif_condition_command_ids: FxHashSet<CommandId>,
    scalar_bindings: FxHashMap<FactSpan, &'a Word>,
    presence_tested_names: FxHashSet<Name>,
    subscript_index_reference_spans: FxHashSet<FactSpan>,
    words: Vec<WordFact<'a>>,
    word_index: FxHashMap<FactSpan, Vec<usize>>,
    function_headers: Vec<FunctionHeaderFact<'a>>,
    for_headers: Vec<ForHeaderFact<'a>>,
    select_headers: Vec<SelectHeaderFact<'a>>,
    case_items: Vec<CaseItemFact<'a>>,
    pipelines: Vec<PipelineFact<'a>>,
    lists: Vec<ListFact<'a>>,
    single_test_subshell_spans: Vec<Span>,
    subshell_test_group_spans: Vec<Span>,
    non_absolute_shebang_span: Option<Span>,
    condition_status_capture_spans: Vec<Span>,
    single_quoted_fragments: Vec<SingleQuotedFragmentFact>,
    open_double_quote_fragments: Vec<OpenDoubleQuoteFragmentFact>,
    backtick_fragments: Vec<BacktickFragmentFact>,
    legacy_arithmetic_fragments: Vec<LegacyArithmeticFragmentFact>,
    positional_parameter_fragments: Vec<PositionalParameterFragmentFact>,
    positional_parameter_operator_spans: Vec<Span>,
    double_paren_grouping_spans: Vec<Span>,
    unicode_smart_quote_spans: Vec<Span>,
    pattern_literal_spans: Vec<Span>,
    pattern_charclass_spans: Vec<Span>,
    nested_parameter_expansion_fragments: Vec<NestedParameterExpansionFragmentFact>,
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

    pub fn scalar_binding_value(&self, span: Span) -> Option<&'a Word> {
        self.scalar_bindings.get(&FactSpan::new(span)).copied()
    }

    pub(crate) fn scalar_binding_values(&self) -> &FxHashMap<FactSpan, &'a Word> {
        &self.scalar_bindings
    }

    pub fn is_elif_condition_command(&self, id: CommandId) -> bool {
        self.elif_condition_command_ids.contains(&id)
    }

    pub fn presence_tested_names(&self) -> &FxHashSet<Name> {
        &self.presence_tested_names
    }

    pub fn is_subscript_index_reference(&self, span: Span) -> bool {
        self.subscript_index_reference_spans
            .contains(&FactSpan::new(span))
    }

    pub fn word_facts(&self) -> &[WordFact<'a>] {
        &self.words
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

    pub fn function_headers(&self) -> &[FunctionHeaderFact<'a>] {
        &self.function_headers
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

    pub fn pipelines(&self) -> &[PipelineFact<'a>] {
        &self.pipelines
    }

    pub fn lists(&self) -> &[ListFact<'a>] {
        &self.lists
    }

    pub fn single_test_subshell_spans(&self) -> &[Span] {
        &self.single_test_subshell_spans
    }

    pub fn subshell_test_group_spans(&self) -> &[Span] {
        &self.subshell_test_group_spans
    }

    pub fn non_absolute_shebang_span(&self) -> Option<Span> {
        self.non_absolute_shebang_span
    }

    pub fn condition_status_capture_spans(&self) -> &[Span] {
        &self.condition_status_capture_spans
    }

    pub fn single_quoted_fragments(&self) -> &[SingleQuotedFragmentFact] {
        &self.single_quoted_fragments
    }

    pub fn open_double_quote_fragments(&self) -> &[OpenDoubleQuoteFragmentFact] {
        &self.open_double_quote_fragments
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

    pub fn unicode_smart_quote_spans(&self) -> &[Span] {
        &self.unicode_smart_quote_spans
    }

    pub fn pattern_literal_spans(&self) -> &[Span] {
        &self.pattern_literal_spans
    }

    pub fn pattern_charclass_spans(&self) -> &[Span] {
        &self.pattern_charclass_spans
    }

    pub fn nested_parameter_expansion_fragments(&self) -> &[NestedParameterExpansionFragmentFact] {
        &self.nested_parameter_expansion_fragments
    }
}

struct LinterFactsBuilder<'a> {
    file: &'a File,
    source: &'a str,
    _semantic: &'a SemanticModel,
    _indexer: &'a Indexer,
    _file_context: &'a FileContext,
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
            _semantic: semantic,
            _indexer: indexer,
            _file_context: file_context,
        }
    }

    fn build(self) -> LinterFacts<'a> {
        let structural_commands = query::iter_commands(
            &self.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: false,
            },
        )
        .map(|visit| FactSpan::new(command_span(visit.command)))
        .collect::<FxHashSet<_>>();
        let mut commands = Vec::new();
        let mut structural_command_ids = Vec::new();
        let mut command_ids_by_span = CommandLookupIndex::default();
        let mut scalar_bindings = FxHashMap::default();
        let mut words = Vec::new();
        let mut pattern_literal_spans = Vec::new();
        let mut pattern_charclass_spans = Vec::new();

        for visit in query::iter_commands(
            &self.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        ) {
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

            collect_scalar_bindings(visit.command, &mut scalar_bindings);
            let normalized = command::normalize_command(visit.command, self.source);
            let nested_word_command = !structural_commands.contains(&key);
            if !nested_word_command {
                structural_command_ids.push(id);
            }
            let (command_words, command_pattern_literal_spans, command_pattern_charclass_spans) =
                build_word_facts_for_command(visit, self.source, id, nested_word_command);
            words.extend(command_words);
            pattern_literal_spans.extend(command_pattern_literal_spans);
            pattern_charclass_spans.extend(command_pattern_charclass_spans);
            let redirect_facts = build_redirect_facts(visit.redirects, self.source);
            let options = CommandOptionFacts::build(visit.command, &normalized, self.source);
            let simple_test =
                build_simple_test_fact(visit.command, self.source, self._file_context);
            let conditional = build_conditional_fact(visit.command, self.source);
            commands.push(CommandFact {
                id,
                key,
                visit,
                nested_word_command,
                normalized,
                redirect_facts,
                substitution_facts: Vec::new().into_boxed_slice(),
                options,
                simple_test,
                conditional,
            });
        }

        let substitution_facts =
            build_substitution_facts(&commands, &command_ids_by_span, self.source);
        for (fact, substitutions) in commands.iter_mut().zip(substitution_facts) {
            fact.substitution_facts = substitutions;
        }

        let elif_condition_command_ids =
            build_elif_condition_command_ids(&self.file.body, &command_ids_by_span);
        let presence_tested_names = build_presence_tested_names(&commands, self.source);
        let function_headers = build_function_header_facts(&self.file.body);
        let for_headers = build_for_header_facts(&commands, &command_ids_by_span, self.source);
        let select_headers =
            build_select_header_facts(&commands, &command_ids_by_span, self.source);
        let case_items = build_case_item_facts(&commands);
        let pipelines = build_pipeline_facts(&commands, &command_ids_by_span);
        let lists = build_list_facts(&commands, &command_ids_by_span);
        let single_test_subshell_spans =
            build_single_test_subshell_spans(&commands, &command_ids_by_span, self.source);
        let subshell_test_group_spans =
            build_subshell_test_group_spans(&commands, &command_ids_by_span, self.source);
        let non_absolute_shebang_span = build_non_absolute_shebang_span(self.source);
        let condition_status_capture_spans =
            build_condition_status_capture_spans(&self.file.body, self.source);
        let surface_fragments =
            build_surface_fragment_facts(self.file, &commands, &command_ids_by_span, self.source);
        let double_paren_grouping_spans = build_double_paren_grouping_spans(&commands, self.source);
        let subscript_index_reference_spans = build_subscript_index_reference_spans(
            self._semantic,
            &surface_fragments.subscript_spans,
        );
        let mut word_index = FxHashMap::<FactSpan, Vec<usize>>::default();
        for (index, fact) in words.iter().enumerate() {
            word_index.entry(fact.key()).or_default().push(index);
        }

        LinterFacts {
            commands,
            structural_command_ids,
            command_ids_by_span,
            elif_condition_command_ids,
            scalar_bindings,
            presence_tested_names,
            subscript_index_reference_spans,
            words,
            word_index,
            function_headers,
            for_headers,
            select_headers,
            case_items,
            pipelines,
            lists,
            single_test_subshell_spans,
            subshell_test_group_spans,
            non_absolute_shebang_span,
            condition_status_capture_spans,
            single_quoted_fragments: surface_fragments.single_quoted,
            open_double_quote_fragments: surface_fragments.open_double_quotes,
            backtick_fragments: surface_fragments.backticks,
            legacy_arithmetic_fragments: surface_fragments.legacy_arithmetic,
            positional_parameter_fragments: surface_fragments.positional_parameters,
            positional_parameter_operator_spans: surface_fragments
                .positional_parameter_operator_spans,
            double_paren_grouping_spans,
            unicode_smart_quote_spans: surface_fragments.unicode_smart_quote_spans,
            pattern_literal_spans,
            pattern_charclass_spans,
            nested_parameter_expansion_fragments: surface_fragments.nested_parameter_expansions,
        }
    }
}

fn build_function_header_facts(body: &StmtSeq) -> Vec<FunctionHeaderFact<'_>> {
    query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    )
    .filter_map(|visit| match visit.command {
        Command::Function(function) => Some(FunctionHeaderFact { function }),
        Command::Simple(_)
        | Command::Decl(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::AnonymousFunction(_) => None,
    })
    .collect()
}

fn build_non_absolute_shebang_span(source: &str) -> Option<Span> {
    let first_line = source.lines().next()?;
    let shebang = first_line.strip_prefix("#!")?;
    let interpreter = shebang.split_whitespace().next()?;

    if interpreter.starts_with('/') || interpreter == "/usr/bin/env" {
        return None;
    }
    if has_header_shellcheck_shell_directive(source) {
        return None;
    }

    let line = first_line.trim_end_matches('\r');
    let start = Position {
        line: 1,
        column: 1,
        offset: 0,
    };
    let end = start.advanced_by(line);
    Some(Span::from_positions(start, end))
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

fn build_elif_condition_command_ids(
    commands: &StmtSeq,
    command_ids_by_span: &CommandLookupIndex,
) -> FxHashSet<CommandId> {
    let mut ids = FxHashSet::default();

    for visit in query::iter_commands(
        commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    ) {
        let Command::Compound(CompoundCommand::If(command)) = visit.command else {
            continue;
        };

        for (condition, _) in &command.elif_branches {
            for condition_visit in query::iter_commands(
                condition,
                CommandWalkOptions {
                    descend_nested_word_commands: true,
                },
            ) {
                if let Some(id) =
                    command_id_for_command(condition_visit.command, command_ids_by_span)
                {
                    ids.insert(id);
                }
            }
        }
    }

    ids
}

fn build_condition_status_capture_spans(commands: &StmtSeq, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    for visit in query::iter_commands(
        commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
    ) {
        match visit.command {
            Command::Compound(CompoundCommand::If(command)) => {
                collect_condition_status_capture_from_body(
                    &command.condition,
                    &command.then_branch,
                    source,
                    &mut spans,
                );

                for (condition, branch) in &command.elif_branches {
                    collect_condition_status_capture_from_body(
                        condition, branch, source, &mut spans,
                    );
                }

                if let Some(else_branch) = &command.else_branch {
                    let fallback_condition = command
                        .elif_branches
                        .last()
                        .map(|(condition, _)| condition)
                        .unwrap_or(&command.condition);
                    collect_condition_status_capture_from_body(
                        fallback_condition,
                        else_branch,
                        source,
                        &mut spans,
                    );
                }
            }
            Command::Compound(CompoundCommand::While(command)) => {
                collect_condition_status_capture_from_body(
                    &command.condition,
                    &command.body,
                    source,
                    &mut spans,
                );
            }
            Command::Compound(CompoundCommand::Until(command)) => {
                collect_condition_status_capture_from_body(
                    &command.condition,
                    &command.body,
                    source,
                    &mut spans,
                );
            }
            Command::Binary(command) if matches!(command.op, BinaryOp::And | BinaryOp::Or) => {
                if stmt_terminals_are_test_commands(&command.left, source) {
                    collect_status_parameter_spans_in_stmt(&command.right, source, &mut spans);
                }
            }
            _ => {}
        }
    }

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
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
            | CompoundCommand::Case(_)
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
            expression,
            expression_ast,
            ..
        } => {
            if let Some(expression) = expression_ast {
                query::visit_arithmetic_words(expression, &mut |word| {
                    collect_status_parameter_spans_in_word(word, source, spans);
                });
            } else {
                collect_status_parameter_spans_in_source_text(expression, source, spans);
            }
        }
        WordPart::Parameter(parameter) => {
            collect_status_parameter_spans_in_parameter_expansion(parameter, source, spans);
        }
        WordPart::ParameterExpansion {
            reference, operand, ..
        }
        | WordPart::IndirectExpansion {
            reference, operand, ..
        } => {
            if reference.name.as_str() == "?" {
                spans.push(part.span);
            }
            collect_status_parameter_spans_in_var_ref(reference, source, spans);
            if let Some(operand) = operand {
                collect_status_parameter_spans_in_source_text(operand, source, spans);
            }
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
            offset,
            offset_ast,
            length,
            length_ast,
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
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
                collect_status_parameter_spans_in_source_text(offset, source, spans);
            }
            match (length_ast.as_ref(), length.as_ref()) {
                (Some(length_ast), _) => {
                    query::visit_arithmetic_words(length_ast, &mut |word| {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    });
                }
                (None, Some(length)) => {
                    collect_status_parameter_spans_in_source_text(length, source, spans);
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
                reference, operand, ..
            }
            | BourneParameterExpansion::Operation {
                reference, operand, ..
            } => {
                collect_status_parameter_spans_in_var_ref(reference, source, spans);
                if let Some(operand) = operand {
                    collect_status_parameter_spans_in_source_text(operand, source, spans);
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset,
                offset_ast,
                length,
                length_ast,
            } => {
                collect_status_parameter_spans_in_var_ref(reference, source, spans);
                if let Some(offset_ast) = offset_ast {
                    query::visit_arithmetic_words(offset_ast, &mut |word| {
                        collect_status_parameter_spans_in_word(word, source, spans);
                    });
                } else {
                    collect_status_parameter_spans_in_source_text(offset, source, spans);
                }

                match (length_ast.as_ref(), length.as_ref()) {
                    (Some(length_ast), _) => {
                        query::visit_arithmetic_words(length_ast, &mut |word| {
                            collect_status_parameter_spans_in_word(word, source, spans);
                        });
                    }
                    (None, Some(length)) => {
                        collect_status_parameter_spans_in_source_text(length, source, spans);
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
                        collect_status_parameter_spans_in_source_text(operand, source, spans);
                    }
                    shuck_ast::ZshExpansionOperation::ReplacementOperation {
                        pattern,
                        replacement,
                        ..
                    } => {
                        collect_status_parameter_spans_in_source_text(pattern, source, spans);
                        if let Some(replacement) = replacement {
                            collect_status_parameter_spans_in_source_text(
                                replacement,
                                source,
                                spans,
                            );
                        }
                    }
                    shuck_ast::ZshExpansionOperation::Slice { offset, length } => {
                        collect_status_parameter_spans_in_source_text(offset, source, spans);
                        if let Some(length) = length {
                            collect_status_parameter_spans_in_source_text(length, source, spans);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::Unknown(text) => {
                        collect_status_parameter_spans_in_source_text(text, source, spans);
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

fn collect_status_parameter_spans_in_source_text(
    text: &SourceText,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let snippet = text.slice(source);
    if !snippet.contains("$?") {
        return;
    }
    let word = Parser::parse_word_fragment(source, snippet, text.span());
    collect_status_parameter_spans_in_word(&word, source, spans);
}

fn build_redirect_facts<'a>(redirects: &'a [Redirect], source: &str) -> Box<[RedirectFact<'a>]> {
    redirects
        .iter()
        .map(|redirect| RedirectFact {
            redirect,
            target_span: redirect.word_target().map(|word| word.span),
            analysis: analyze_redirect_target(redirect, source),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn build_word_facts_for_command<'a>(
    visit: CommandVisit<'a>,
    source: &'a str,
    command_id: CommandId,
    nested_word_command: bool,
) -> (Vec<WordFact<'a>>, Vec<Span>, Vec<Span>) {
    let mut collector = WordFactCollector::new(source, command_id, nested_word_command);
    collector.collect_command(visit.command, visit.redirects);
    collector.finish()
}

struct WordFactCollector<'a> {
    source: &'a str,
    command_id: CommandId,
    nested_word_command: bool,
    facts: Vec<WordFact<'a>>,
    seen: FxHashSet<(FactSpan, WordFactContext, WordFactHostKind)>,
    pattern_literal_spans: Vec<Span>,
    pattern_charclass_spans: Vec<Span>,
}

impl<'a> WordFactCollector<'a> {
    fn new(source: &'a str, command_id: CommandId, nested_word_command: bool) -> Self {
        Self {
            source,
            command_id,
            nested_word_command,
            facts: Vec::new(),
            seen: FxHashSet::default(),
            pattern_literal_spans: Vec::new(),
            pattern_charclass_spans: Vec::new(),
        }
    }

    fn finish(self) -> (Vec<WordFact<'a>>, Vec<Span>, Vec<Span>) {
        (
            self.facts,
            self.pattern_literal_spans,
            self.pattern_charclass_spans,
        )
    }

    fn collect_command(&mut self, command: &'a Command, redirects: &'a [Redirect]) {
        self.collect_command_name_context_word(command);
        self.collect_argument_context_words(command);
        self.collect_expansion_assignment_value_words(command);

        if let Command::Compound(command) = command {
            match command {
                CompoundCommand::For(command) => {
                    if let Some(words) = &command.words {
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
                    self.push_word(
                        &command.count,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        WordFactHostKind::Direct,
                    );
                }
                CompoundCommand::Foreach(command) => {
                    for word in &command.words {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::ForList),
                            WordFactHostKind::Direct,
                        );
                    }
                }
                CompoundCommand::Select(command) => {
                    for word in &command.words {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::SelectList),
                            WordFactHostKind::Direct,
                        );
                    }
                }
                CompoundCommand::Case(command) => {
                    self.push_word(
                        &command.word,
                        WordFactContext::CaseSubject,
                        WordFactHostKind::Direct,
                    );
                    for case in &command.cases {
                        for pattern in &case.patterns {
                            self.collect_pattern_context_words(
                                pattern,
                                WordFactContext::Expansion(ExpansionContext::CasePattern),
                                WordFactHostKind::Direct,
                            );
                        }
                    }
                }
                CompoundCommand::Conditional(command) => {
                    self.collect_conditional_expansion_words(&command.expression);
                }
                CompoundCommand::If(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::Subshell(_)
                | CompoundCommand::BraceGroup(_)
                | CompoundCommand::Always(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Coproc(_)
                | CompoundCommand::Time(_) => {}
            }
        }

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
        match command {
            Command::Simple(command) => {
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
                if static_word_text(&command.name, self.source).as_deref() == Some("trap") {
                    return;
                }
                for word in &command.args {
                    self.push_word(
                        word,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        WordFactHostKind::Direct,
                    );
                }
            }
            Command::Builtin(command) => match command {
                BuiltinCommand::Break(command) => {
                    if let Some(word) = &command.depth {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
                BuiltinCommand::Continue(command) => {
                    if let Some(word) = &command.depth {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
                BuiltinCommand::Return(command) => {
                    if let Some(word) = &command.code {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
                BuiltinCommand::Exit(command) => {
                    if let Some(word) = &command.code {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    );
                }
            },
            Command::Decl(command) => {
                for operand in &command.operands {
                    if let DeclOperand::Dynamic(word) = operand {
                        self.push_word(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                        );
                    }
                }
            }
            Command::Binary(_) | Command::Compound(_) | Command::Function(_) => {}
            Command::AnonymousFunction(function) => {
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
                    query::visit_var_ref_subscript_words_with_source(
                        reference,
                        self.source,
                        &mut |word| {
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
        query::visit_var_ref_subscript_words_with_source(
            &assignment.target,
            self.source,
            &mut |word| {
                self.push_owned_word(
                    word.clone(),
                    context,
                    WordFactHostKind::AssignmentTargetSubscript,
                );
            },
        );

        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.push_word(word, context, WordFactHostKind::Direct)
            }
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => {
                            self.push_word(word, context, WordFactHostKind::Direct);
                        }
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            query::visit_subscript_words(Some(key), self.source, &mut |word| {
                                self.push_owned_word(
                                    word.clone(),
                                    context,
                                    WordFactHostKind::ArrayKeySubscript,
                                );
                            });
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
                PatternPart::Word(word) => self.push_owned_word(word.clone(), context, host_kind),
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

    fn collect_conditional_expansion_words(&mut self, expression: &'a ConditionalExpr) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.collect_conditional_expansion_words(&expr.left);
                self.collect_conditional_expansion_words(&expr.right);
            }
            ConditionalExpr::Unary(expr) => self.collect_conditional_expansion_words(&expr.expr),
            ConditionalExpr::Parenthesized(expr) => {
                self.collect_conditional_expansion_words(&expr.expr)
            }
            ConditionalExpr::Word(word) => self.push_word(
                word,
                WordFactContext::Expansion(ExpansionContext::StringTestOperand),
                WordFactHostKind::Direct,
            ),
            ConditionalExpr::Regex(word) => self.push_word(
                word,
                WordFactContext::Expansion(ExpansionContext::RegexOperand),
                WordFactHostKind::Direct,
            ),
            ConditionalExpr::Pattern(_) => {}
            ConditionalExpr::VarRef(reference) => {
                query::visit_var_ref_subscript_words_with_source(
                    reference,
                    self.source,
                    &mut |word| {
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

    fn push_word(&mut self, word: &'a Word, context: WordFactContext, host_kind: WordFactHostKind) {
        self.push_cow_word(Cow::Borrowed(word), context, host_kind);
    }

    fn push_owned_word(
        &mut self,
        word: Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        self.push_cow_word(Cow::Owned(word), context, host_kind);
    }

    fn push_cow_word(
        &mut self,
        word: Cow<'a, Word>,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        let word_ref = word.as_ref();
        let key = FactSpan::new(word_ref.span);
        if !self.seen.insert((key, context, host_kind)) {
            return;
        }

        self.collect_word_parameter_patterns(&word_ref.parts, host_kind);

        let analysis = analyze_word(word_ref, self.source);
        let operand_class = match context {
            WordFactContext::Expansion(context) if word_context_supports_operand_class(context) => {
                Some(
                    if analysis.literalness == WordLiteralness::Expanded
                        || analyze_literal_runtime(word_ref, self.source, context)
                            .is_runtime_sensitive()
                    {
                        TestOperandClass::RuntimeSensitive
                    } else {
                        TestOperandClass::FixedLiteral
                    },
                )
            }
            WordFactContext::Expansion(_) | WordFactContext::CaseSubject => None,
        };

        self.facts.push(WordFact {
            key,
            static_text: static_word_text(word_ref, self.source).map(String::into_boxed_str),
            has_literal_affixes: word_has_literal_affixes(word_ref),
            scalar_expansion_spans: span::scalar_expansion_part_spans(word_ref, self.source)
                .into_boxed_slice(),
            array_expansion_spans: span::array_expansion_part_spans(word_ref, self.source)
                .into_boxed_slice(),
            unquoted_array_expansion_spans: span::unquoted_array_expansion_part_spans(
                word_ref,
                self.source,
            )
            .into_boxed_slice(),
            command_substitution_spans: span::command_substitution_part_spans(word_ref)
                .into_boxed_slice(),
            unquoted_command_substitution_spans: span::unquoted_command_substitution_part_spans(
                word_ref,
            )
            .into_boxed_slice(),
            double_quoted_expansion_spans: double_quoted_expansion_part_spans(word_ref)
                .into_boxed_slice(),
            word,
            command_id: self.command_id,
            nested_word_command: self.nested_word_command,
            context,
            host_kind,
            analysis,
            operand_class,
        });
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
    let shape = match operands.len() {
        0 => SimpleTestShape::Empty,
        1 => SimpleTestShape::Truthy,
        2 => SimpleTestShape::Unary,
        3 => SimpleTestShape::Binary,
        _ => SimpleTestShape::Other,
    };
    let operator_family = match shape {
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
    };
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
        operand_classes,
        empty_test_suppressed: file_context
            .span_intersects_kind(ContextRegionKind::ShellSpecParametersBlock, command.span),
    })
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
    (!nodes.is_empty()).then_some(ConditionalFact {
        nodes: nodes.into_boxed_slice(),
    })
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
        ConditionalExpr::Binary(_)
        | ConditionalExpr::Unary(_)
        | ConditionalExpr::Parenthesized(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::VarRef(_) => None,
    };

    ConditionalOperandFact {
        expression,
        class: classify_conditional_operand(expression, source),
        word,
        word_classification: word.map(|word| classify_word(word, source)),
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

fn word_starts_with_literal_dash(word: &Word, source: &str) -> bool {
    matches!(
        word.parts_with_spans().next(),
        Some((WordPart::Literal(text), span)) if text.as_str(source, span).starts_with('-')
    )
}

fn parse_grep_command(args: &[&Word], source: &str) -> Option<GrepCommandFacts> {
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
        if text == "--only-matching" {
            return Some(GrepCommandFacts {
                uses_only_matching: true,
            });
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'o' {
                return Some(GrepCommandFacts {
                    uses_only_matching: true,
                });
            }

            if grep_option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        index += 1;
    }

    Some(GrepCommandFacts {
        uses_only_matching: false,
    })
}

fn grep_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'A' | 'B' | 'C' | 'D' | 'd' | 'e' | 'f' | 'm')
}

fn option_takes_argument(flag: char) -> bool {
    matches!(flag, 'a' | 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
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

fn parse_unset_command<'a>(args: &[&'a Word], source: &str) -> UnsetCommandFacts<'a> {
    let mut function_mode = false;
    let mut parsing_options = true;
    let mut options_parseable = true;
    let mut operands = Vec::new();

    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if parsing_options {
                options_parseable = false;
                break;
            }

            operands.push(*word);
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

        operands.push(*word);
    }

    UnsetCommandFacts {
        function_mode,
        operand_words: operands.into_boxed_slice(),
        options_parseable,
    }
}

fn parse_set_command(args: &[&Word], source: &str) -> SetCommandFacts {
    let mut errexit_change = None;
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        match text.as_str() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                let Some(name) = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                else {
                    break;
                };

                if name == "errexit" {
                    errexit_change = Some(enable);
                }
                index += 2;
                continue;
            }
            _ => {}
        }

        let Some(flags) = text.strip_prefix('-').or_else(|| text.strip_prefix('+')) else {
            break;
        };
        if flags.is_empty() {
            break;
        }

        if flags.chars().any(|flag| flag == 'e') {
            errexit_change = Some(text.starts_with('-'));
        }

        index += 1;
    }

    SetCommandFacts { errexit_change }
}

fn parse_expr_command(args: &[&Word], source: &str) -> Option<ExprCommandFacts> {
    if expr_uses_string_form(args, source) {
        return None;
    }

    Some(ExprCommandFacts {
        uses_arithmetic_operator: true,
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

fn parse_exit_command<'a>(command: &'a Command, source: &str) -> Option<ExitCommandFacts<'a>> {
    let Command::Builtin(BuiltinCommand::Exit(exit)) = command else {
        return None;
    };
    let Some(status_word) = exit.code.as_ref() else {
        return Some(ExitCommandFacts {
            status_word: None,
            is_numeric_literal: false,
            status_is_static: false,
        });
    };
    let status_text = static_word_text(status_word, source);

    Some(ExitCommandFacts {
        status_word: Some(status_word),
        is_numeric_literal: status_text
            .as_deref()
            .is_some_and(|text| text.chars().all(|character| character.is_ascii_digit())),
        status_is_static: status_text.is_some(),
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

fn collect_scalar_bindings<'a>(
    command: &'a Command,
    scalar_bindings: &mut FxHashMap<FactSpan, &'a Word>,
) {
    for assignment in query::command_assignments(command) {
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        scalar_bindings.insert(FactSpan::new(assignment.target.name_span), word);
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        scalar_bindings.insert(FactSpan::new(assignment.target.name_span), word);
    }
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

    use shuck_ast::BinaryOp;
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
    use shuck_semantic::SemanticModel;

    use super::{
        CommandId, ConditionalNodeFact, ConditionalOperatorFamily, LinterFacts,
        SimpleTestOperatorFamily, SimpleTestShape, SimpleTestSyntax, SubstitutionHostKind,
        SudoFamilyInvoker, WordFactHostKind,
    };
    use crate::rules::common::command::WrapperKind;
    use crate::rules::common::expansion::{ExpansionContext, SubstitutionOutputIntent};
    use crate::{ShellDialect, classify_file_context};

    fn with_facts_dialect(
        source: &str,
        path: Option<&Path>,
        parse_dialect: ParseShellDialect,
        shell: ShellDialect,
        visit: impl FnOnce(&shuck_parser::parser::ParseOutput, &LinterFacts<'_>),
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
        visit: impl FnOnce(&shuck_parser::parser::ParseOutput, &LinterFacts<'_>),
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
    fn indexes_scalar_bindings_from_assignments_and_declarations() {
        let source = "#!/bin/bash\nfoo=1 printf '%s\\n' \"$foo\"\nexport bar=2\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);

        let first_binding_span = match &output.file.body[0].command {
            shuck_ast::Command::Simple(command) => command.assignments[0].target.name_span,
            _ => panic!("expected simple command"),
        };
        assert_eq!(
            facts
                .scalar_binding_value(first_binding_span)
                .map(|word| word.span.slice(source)),
            Some("1")
        );

        let second_binding_span = match &output.file.body[1].command {
            shuck_ast::Command::Decl(command) => match &command.operands[0] {
                shuck_ast::DeclOperand::Assignment(assignment) => assignment.target.name_span,
                _ => panic!("expected declaration assignment"),
            },
            _ => panic!("expected declaration command"),
        };
        assert_eq!(
            facts
                .scalar_binding_value(second_binding_span)
                .map(|word| word.span.slice(source)),
            Some("2")
        );
    }

    #[test]
    fn summarizes_command_options_and_invokers() {
        let source = "#!/bin/bash\nread -r name\nprintf -v out \"$fmt\" value\nunset -f curl other\nfind . -print0 | xargs -0 rm\ngrep -o content file | wc -l\nexit foo\ndoas printf '%s\\n' hi\n";
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

        let xargs = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("xargs"))
            .and_then(|fact| fact.options().xargs())
            .expect("expected xargs facts");
        assert!(xargs.uses_null_input);

        let grep = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("grep"))
            .and_then(|fact| fact.options().grep())
            .expect("expected grep facts");
        assert!(grep.uses_only_matching);

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

        let doas = facts
            .commands()
            .iter()
            .find(|fact| fact.has_wrapper(WrapperKind::SudoFamily))
            .and_then(|fact| fact.options().sudo_family())
            .expect("expected sudo-family facts");
        assert_eq!(doas.invoker, SudoFamilyInvoker::Doas);
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
out=$(printf hi > out.txt)
drop=$(printf hi >/dev/null 2>&1)
mixed=$(jq -r . <<< \"$status\" || die >&2)
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
                    )
                })
                .collect::<Vec<_>>();

            assert!(substitutions.contains(&(
                "$(printf arg)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::CommandArgument,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(printf decl-assign)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::DeclarationAssignmentValue,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(printf quoted)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::CommandArgument,
                false,
            )));
            assert!(substitutions.contains(&(
                "$(printf assign)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::AssignmentTargetSubscript,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(printf decl-name)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::DeclarationNameSubscript,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(printf key)".to_owned(),
                SubstitutionOutputIntent::Captured,
                SubstitutionHostKind::ArrayKeySubscript,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(printf hi > out.txt)".to_owned(),
                SubstitutionOutputIntent::Rerouted,
                SubstitutionHostKind::Other,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(printf hi >/dev/null 2>&1)".to_owned(),
                SubstitutionOutputIntent::Discarded,
                SubstitutionHostKind::Other,
                true,
            )));
            assert!(substitutions.contains(&(
                "$(jq -r . <<< \"$status\" || die >&2)".to_owned(),
                SubstitutionOutputIntent::Mixed,
                SubstitutionHostKind::Other,
                true,
            )));
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
[[ foo && -n \"$bar\" && left == right && $value =~ ^\"foo\"bar$ && left == *.sh ]]
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

            let regex = logical.regex_nodes().next().expect("expected regex node");
            assert_eq!(regex.operator_family(), ConditionalOperatorFamily::Regex);
            assert_eq!(
                regex.right().word().map(|word| word.span.slice(source)),
                Some("^\"foo\"bar$")
            );
            assert!(
                regex
                    .right()
                    .quote()
                    .is_some_and(|quote| quote != crate::rules::common::word::WordQuote::Unquoted)
            );
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
                    .map(|fragment| fragment.span().slice(source))
                    .collect::<Vec<_>>(),
                vec!["$10", "$10"]
            );
            assert_eq!(
                facts
                    .open_double_quote_fragments()
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
                        fragment.command_name().map(str::to_owned),
                        fragment.assignment_target().map(str::to_owned),
                        fragment.variable_set_operand(),
                    )
                })
                .collect::<Vec<_>>();
            assert!(single_quoted.iter().any(
                |(text, _, assignment_target, variable_set_operand)| {
                    text == "'$prompt'"
                        && assignment_target.as_deref() == Some("PS4")
                        && !variable_set_operand
                }
            ));
            assert!(single_quoted.contains(&(
                "'$__loc__'".to_owned(),
                Some("jq".to_owned()),
                None,
                false,
            )));
            assert!(single_quoted.contains(&(
                "'$name'".to_owned(),
                Some("test".to_owned()),
                None,
                true,
            )));

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
}
