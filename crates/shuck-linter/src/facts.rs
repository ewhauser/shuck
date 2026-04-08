//! Linter-owned structural facts built once per file.
//!
//! `SemanticModel` remains the source of truth for bindings, references, scopes,
//! source references, the call graph, and flow-sensitive facts.
//! `LinterFacts` owns reusable linter-side summaries that are cheaper to build
//! once than to recompute in every rule: normalized commands, wrapper chains,
//! declaration summaries, option-shape summaries, and later word/expansion
//! facts.

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem,
    Assignment, AssignmentValue, BinaryCommand, BinaryOp, BourneParameterExpansion, BuiltinCommand,
    Command, CommandSubstitutionSyntax, CompoundCommand, ConditionalBinaryOp, ConditionalExpr,
    ConditionalUnaryOp, DeclClause, DeclOperand, File, ForCommand, Name, ParameterExpansionSyntax,
    ParameterOp, Pattern, PatternPart, Redirect, RedirectKind, SelectCommand, SimpleCommand, Span,
    Stmt, StmtSeq, Word, WordPart, WordPartNode, ZshGlobSegment, ZshQualifiedGlob,
};
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;
use std::borrow::Cow;

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
    command_key: FactSpan,
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

    pub fn command_key(&self) -> FactSpan {
        self.command_key
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
    command_key: FactSpan,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact<'a>]>,
}

impl<'a> ForHeaderFact<'a> {
    pub fn command(&self) -> &'a ForCommand {
        self.command
    }

    pub fn command_key(&self) -> FactSpan {
        self.command_key
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
    command_key: FactSpan,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact<'a>]>,
}

impl<'a> SelectHeaderFact<'a> {
    pub fn command(&self) -> &'a SelectCommand {
        self.command
    }

    pub fn command_key(&self) -> FactSpan {
        self.command_key
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
pub struct PipelineSegmentFact<'a> {
    stmt: &'a Stmt,
    command_key: FactSpan,
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

    pub fn command_key(&self) -> FactSpan {
        self.command_key
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

#[derive(Debug, Clone)]
pub struct PipelineFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    segments: Box<[PipelineSegmentFact<'a>]>,
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

    pub fn body_args(&self) -> &[&'a Word] {
        self.normalized.body_args()
    }
}

#[derive(Debug, Clone)]
pub struct LinterFacts<'a> {
    commands: Vec<CommandFact<'a>>,
    structural_command_indices: Vec<usize>,
    command_index: FxHashMap<*const Command, usize>,
    scalar_bindings: FxHashMap<FactSpan, &'a Word>,
    presence_tested_names: FxHashSet<Name>,
    words: Vec<WordFact<'a>>,
    word_index: FxHashMap<FactSpan, Vec<usize>>,
    for_headers: Vec<ForHeaderFact<'a>>,
    select_headers: Vec<SelectHeaderFact<'a>>,
    pipelines: Vec<PipelineFact<'a>>,
    lists: Vec<ListFact<'a>>,
    single_quoted_fragments: Vec<SingleQuotedFragmentFact>,
    backtick_fragments: Vec<BacktickFragmentFact>,
    legacy_arithmetic_fragments: Vec<LegacyArithmeticFragmentFact>,
    positional_parameter_fragments: Vec<PositionalParameterFragmentFact>,
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
        self.structural_command_indices
            .iter()
            .map(|&index| &self.commands[index])
    }

    pub fn command(&self, span: Span) -> Option<&CommandFact<'a>> {
        self.commands.iter().find(|fact| fact.span() == span)
    }

    pub fn command_for_stmt(&self, stmt: &Stmt) -> Option<&CommandFact<'a>> {
        self.command_for_command(&stmt.command)
    }

    pub fn command_for_command(&self, command: &Command) -> Option<&CommandFact<'a>> {
        self.command_index
            .get(&command_ptr(command))
            .map(|&index| &self.commands[index])
    }

    pub fn scalar_binding_value(&self, span: Span) -> Option<&'a Word> {
        self.scalar_bindings.get(&FactSpan::new(span)).copied()
    }

    pub(crate) fn scalar_binding_values(&self) -> &FxHashMap<FactSpan, &'a Word> {
        &self.scalar_bindings
    }

    pub fn presence_tested_names(&self) -> &FxHashSet<Name> {
        &self.presence_tested_names
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

    pub fn for_headers(&self) -> &[ForHeaderFact<'a>] {
        &self.for_headers
    }

    pub fn select_headers(&self) -> &[SelectHeaderFact<'a>] {
        &self.select_headers
    }

    pub fn pipelines(&self) -> &[PipelineFact<'a>] {
        &self.pipelines
    }

    pub fn lists(&self) -> &[ListFact<'a>] {
        &self.lists
    }

    pub fn single_quoted_fragments(&self) -> &[SingleQuotedFragmentFact] {
        &self.single_quoted_fragments
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
        let mut structural_command_indices = Vec::new();
        let mut command_index = FxHashMap::default();
        let mut scalar_bindings = FxHashMap::default();
        let mut words = Vec::new();

        for visit in query::iter_commands(
            &self.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        ) {
            let key = FactSpan::new(command_span(visit.command));
            let index = commands.len();
            let previous = command_index.insert(command_ptr(visit.command), index);
            debug_assert!(previous.is_none(), "duplicate command pointer");

            collect_scalar_bindings(visit.command, &mut scalar_bindings);
            let normalized = command::normalize_command(visit.command, self.source);
            let nested_word_command = !structural_commands.contains(&key);
            if !nested_word_command {
                structural_command_indices.push(index);
            }
            words.extend(build_word_facts_for_command(
                visit,
                self.source,
                key,
                nested_word_command,
            ));
            let redirect_facts = build_redirect_facts(visit.redirects, self.source);
            let options = CommandOptionFacts::build(visit.command, &normalized, self.source);
            let simple_test =
                build_simple_test_fact(visit.command, self.source, self._file_context);
            let conditional = build_conditional_fact(visit.command, self.source);
            commands.push(CommandFact {
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

        let substitution_facts = build_substitution_facts(&commands, &command_index, self.source);
        for (fact, substitutions) in commands.iter_mut().zip(substitution_facts) {
            fact.substitution_facts = substitutions;
        }

        let presence_tested_names = build_presence_tested_names(&commands, self.source);
        let for_headers = build_for_header_facts(&commands, &command_index, self.source);
        let select_headers = build_select_header_facts(&commands, &command_index, self.source);
        let pipelines = build_pipeline_facts(&commands, &command_index);
        let lists = build_list_facts(&commands);
        let surface_fragments =
            build_surface_fragment_facts(self.file, &commands, &command_index, self.source);
        let mut word_index = FxHashMap::<FactSpan, Vec<usize>>::default();
        for (index, fact) in words.iter().enumerate() {
            word_index.entry(fact.key()).or_default().push(index);
        }

        LinterFacts {
            commands,
            structural_command_indices,
            command_index,
            scalar_bindings,
            presence_tested_names,
            words,
            word_index,
            for_headers,
            select_headers,
            pipelines,
            lists,
            single_quoted_fragments: surface_fragments.single_quoted,
            backtick_fragments: surface_fragments.backticks,
            legacy_arithmetic_fragments: surface_fragments.legacy_arithmetic,
            positional_parameter_fragments: surface_fragments.positional_parameters,
        }
    }
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

fn build_presence_tested_names(commands: &[CommandFact<'_>], source: &str) -> FxHashSet<Name> {
    let mut names = FxHashSet::default();

    for command in commands {
        if let Some(simple_test) = command.simple_test() {
            collect_presence_tested_names_from_simple_test_operands(
                simple_test.operands(),
                source,
                &mut names,
            );
        }

        if let Some(conditional) = command.conditional() {
            collect_presence_tested_names_from_conditional_expr(
                conditional.root().expression(),
                &mut names,
            );
        }
    }

    names
}

fn collect_presence_tested_names_from_simple_test_operands(
    operands: &[&Word],
    source: &str,
    names: &mut FxHashSet<Name>,
) {
    let mut index = 0;
    while index < operands.len() {
        if is_simple_test_logical_operator(operands[index], source) {
            index += 1;
            continue;
        }

        let consumed =
            collect_presence_tested_names_from_simple_test_leaf(&operands[index..], source, names);
        if consumed == 0 {
            break;
        }
        index += consumed;
    }
}

fn collect_presence_tested_names_from_simple_test_leaf(
    operands: &[&Word],
    source: &str,
    names: &mut FxHashSet<Name>,
) -> usize {
    let Some(first) = operands.first().copied() else {
        return 0;
    };

    if static_word_text(first, source).as_deref() == Some("!") {
        return 1 + collect_presence_tested_names_from_simple_test_leaf(
            &operands[1..],
            source,
            names,
        );
    }

    if static_word_text(first, source)
        .as_deref()
        .is_some_and(|operator| {
            simple_test_unary_operator_family(operator) == SimpleTestOperatorFamily::StringUnary
        })
    {
        if let Some(word) = operands.get(1).copied() {
            collect_presence_tested_names_from_word(word, names);
            return 2;
        }
        return 1;
    }

    if operands.len() == 1
        || operands
            .get(1)
            .copied()
            .is_some_and(|word| is_simple_test_logical_operator(word, source))
    {
        collect_presence_tested_names_from_word(first, names);
        return 1;
    }

    operands
        .iter()
        .skip(1)
        .position(|word| is_simple_test_logical_operator(word, source))
        .map_or(operands.len(), |offset| offset + 1)
}

fn is_simple_test_logical_operator(word: &Word, source: &str) -> bool {
    matches!(static_word_text(word, source).as_deref(), Some("-a" | "-o"))
}

fn collect_presence_tested_names_from_conditional_expr(
    expression: &ConditionalExpr,
    names: &mut FxHashSet<Name>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    match expression {
        ConditionalExpr::Word(word) => collect_presence_tested_names_from_word(word, names),
        ConditionalExpr::Unary(unary)
            if conditional_unary_operator_family(unary.op)
                == ConditionalOperatorFamily::StringUnary =>
        {
            collect_presence_tested_names_from_conditional_operand(&unary.expr, names);
        }
        ConditionalExpr::Binary(binary)
            if conditional_binary_operator_family(binary.op)
                == ConditionalOperatorFamily::Logical =>
        {
            collect_presence_tested_names_from_conditional_expr(&binary.left, names);
            collect_presence_tested_names_from_conditional_expr(&binary.right, names);
        }
        ConditionalExpr::Unary(_)
        | ConditionalExpr::Binary(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => {}
        ConditionalExpr::Parenthesized(_) => {
            unreachable!("parentheses should be stripped before collecting presence tests")
        }
    }
}

fn collect_presence_tested_names_from_conditional_operand(
    expression: &ConditionalExpr,
    names: &mut FxHashSet<Name>,
) {
    let expression = strip_parenthesized_conditionals(expression);

    if let ConditionalExpr::Word(word) = expression {
        collect_presence_tested_names_from_word(word, names);
    }
}

fn collect_presence_tested_names_from_word(word: &Word, names: &mut FxHashSet<Name>) {
    collect_presence_tested_names_from_word_parts(&word.parts, names);
}

fn collect_presence_tested_names_from_word_parts(
    parts: &[WordPartNode],
    names: &mut FxHashSet<Name>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_presence_tested_names_from_word_parts(parts, names);
            }
            WordPart::Variable(name)
            | WordPart::IndirectExpansion { name, .. }
            | WordPart::PrefixMatch { prefix: name, .. } => {
                names.insert(name.clone());
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Substring { reference, .. }
            | WordPart::ArraySlice { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                names.insert(reference.name.clone());
            }
            WordPart::Parameter(parameter) => {
                collect_presence_tested_names_from_parameter_expansion(parameter, names);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_presence_tested_names_from_parameter_expansion(
    parameter: &shuck_ast::ParameterExpansion,
    names: &mut FxHashSet<Name>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                names.insert(reference.name.clone());
            }
            BourneParameterExpansion::Indirect { name, .. }
            | BourneParameterExpansion::PrefixMatch { prefix: name, .. } => {
                names.insert(name.clone());
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            shuck_ast::ZshExpansionTarget::Reference(reference) => {
                names.insert(reference.name.clone());
            }
            shuck_ast::ZshExpansionTarget::Nested(parameter) => {
                collect_presence_tested_names_from_parameter_expansion(parameter, names);
            }
            shuck_ast::ZshExpansionTarget::Empty => {}
        },
    }
}

fn build_word_facts_for_command<'a>(
    visit: CommandVisit<'a>,
    source: &'a str,
    command_key: FactSpan,
    nested_word_command: bool,
) -> Vec<WordFact<'a>> {
    let mut collector = WordFactCollector::new(source, command_key, nested_word_command);
    collector.collect_command(visit.command, visit.redirects);
    collector.finish()
}

struct WordFactCollector<'a> {
    source: &'a str,
    command_key: FactSpan,
    nested_word_command: bool,
    facts: Vec<WordFact<'a>>,
    seen: FxHashSet<(FactSpan, WordFactContext, WordFactHostKind)>,
}

impl<'a> WordFactCollector<'a> {
    fn new(source: &'a str, command_key: FactSpan, nested_word_command: bool) -> Self {
        Self {
            source,
            command_key,
            nested_word_command,
            facts: Vec::new(),
            seen: FxHashSet::default(),
        }
    }

    fn finish(self) -> Vec<WordFact<'a>> {
        self.facts
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
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_pattern_context_words(pattern, context, host_kind);
                    }
                }
                PatternPart::Word(word) => self.push_owned_word(word.clone(), context, host_kind),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
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
            command_key: self.command_key,
            nested_word_command: self.nested_word_command,
            context,
            host_kind,
            analysis,
            operand_class,
        });
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

fn build_substitution_facts<'a>(
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
) -> Vec<Box<[SubstitutionFact]>> {
    commands
        .iter()
        .map(|fact| build_command_substitution_facts(fact, commands, command_index, source))
        .collect()
}

fn build_command_substitution_facts<'a>(
    fact: &CommandFact<'a>,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
) -> Box<[SubstitutionFact]> {
    let mut substitutions = Vec::new();
    let mut substitution_index = FxHashMap::default();

    visit_command_words_for_substitutions(fact.command(), fact.redirects(), source, &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::Other,
            commands,
            command_index,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_command_argument_words_for_substitutions(fact.command(), source, &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::CommandArgument,
            commands,
            command_index,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_declaration_assignment_words_for_substitutions(fact.command(), &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::DeclarationAssignmentValue,
            commands,
            command_index,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_command_subscript_words_for_substitutions(fact.command(), source, &mut |kind, word| {
        collect_or_update_word_substitution_facts(
            word,
            kind,
            commands,
            command_index,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    substitutions.into_boxed_slice()
}

fn collect_or_update_word_substitution_facts<'a>(
    word: &Word,
    host_kind: SubstitutionHostKind,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    let mut occurrences = Vec::new();
    collect_word_substitution_occurrences(&word.parts, false, &mut occurrences);

    for occurrence in occurrences {
        let key = FactSpan::new(occurrence.span);
        if let Some(&index) = substitution_index.get(&key) {
            substitutions[index].host_word_span = word.span;
            substitutions[index].host_kind = host_kind;
            substitutions[index].unquoted_in_host = occurrence.unquoted_in_host;
            continue;
        }

        let (stdout_intent, has_stdout_redirect) =
            classify_substitution_body(occurrence.body, commands, command_index, source);
        substitution_index.insert(key, substitutions.len());
        substitutions.push(SubstitutionFact {
            span: occurrence.span,
            kind: occurrence.kind,
            stdout_intent,
            has_stdout_redirect,
            host_word_span: word.span,
            host_kind,
            unquoted_in_host: occurrence.unquoted_in_host,
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct SubstitutionOccurrence<'a> {
    body: &'a StmtSeq,
    span: Span,
    kind: CommandSubstitutionKind,
    unquoted_in_host: bool,
}

fn collect_word_substitution_occurrences<'a>(
    parts: &'a [WordPartNode],
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_substitution_occurrences(parts, true, occurrences);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                visit_arithmetic_words_in_expression(expression_ast.as_ref(), quoted, occurrences);
            }
            WordPart::CommandSubstitution { body, .. } => {
                occurrences.push(SubstitutionOccurrence {
                    body,
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                    unquoted_in_host: !quoted,
                });
            }
            WordPart::ProcessSubstitution { body, is_input } => {
                occurrences.push(SubstitutionOccurrence {
                    body,
                    span: part.span,
                    kind: if *is_input {
                        CommandSubstitutionKind::ProcessInput
                    } else {
                        CommandSubstitutionKind::ProcessOutput
                    },
                    unquoted_in_host: !quoted,
                });
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
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
        }
    }
}

fn visit_arithmetic_words_in_expression<'a>(
    expression: Option<&'a ArithmeticExprNode>,
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    let Some(expression) = expression else {
        return;
    };

    collect_arithmetic_word_substitution_occurrences(expression, quoted, occurrences);
}

fn collect_arithmetic_word_substitution_occurrences<'a>(
    expression: &'a ArithmeticExprNode,
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => {
            collect_arithmetic_word_substitution_occurrences(index, quoted, occurrences);
        }
        ArithmeticExpr::ShellWord(word) => {
            collect_word_substitution_occurrences(&word.parts, quoted, occurrences);
        }
        ArithmeticExpr::Parenthesized { expression } => {
            collect_arithmetic_word_substitution_occurrences(expression, quoted, occurrences);
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            collect_arithmetic_word_substitution_occurrences(expr, quoted, occurrences);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_word_substitution_occurrences(left, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(right, quoted, occurrences);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_word_substitution_occurrences(condition, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(then_expr, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(else_expr, quoted, occurrences);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_arithmetic_lvalue_substitution_occurrences(target, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(value, quoted, occurrences);
        }
    }
}

fn collect_arithmetic_lvalue_substitution_occurrences<'a>(
    target: &'a ArithmeticLvalue,
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            collect_arithmetic_word_substitution_occurrences(index, quoted, occurrences);
        }
    }
}

fn classify_substitution_body<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
) -> (SubstitutionOutputIntent, bool) {
    let mut stdout_intent: Option<SubstitutionOutputIntent> = None;
    let mut has_stdout_redirect = false;

    for visit in query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
    ) {
        let state = if let Some(&index) = command_index.get(&command_ptr(visit.command)) {
            classify_redirect_facts(commands[index].redirect_facts())
        } else {
            let redirect_facts = build_redirect_facts(visit.redirects, source);
            classify_redirect_facts(&redirect_facts)
        };

        has_stdout_redirect |= state.has_stdout_redirect;
        stdout_intent = Some(match stdout_intent {
            Some(current) if current == state.stdout_intent => current,
            Some(_) => SubstitutionOutputIntent::Mixed,
            None => state.stdout_intent,
        });
    }

    (
        stdout_intent.unwrap_or(SubstitutionOutputIntent::Captured),
        has_stdout_redirect,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputSink {
    Captured,
    DevNull,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RedirectState {
    stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
}

fn classify_redirect_facts(redirects: &[RedirectFact<'_>]) -> RedirectState {
    let mut fds = FxHashMap::from_iter([(1, OutputSink::Captured), (2, OutputSink::Other)]);
    let mut has_stdout_redirect = false;

    for redirect in redirects {
        match redirect.redirect().kind {
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
                let sink = redirect_file_sink(redirect);
                let fd = redirect.redirect().fd.unwrap_or(1);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::OutputBoth => {
                let sink = redirect_file_sink(redirect);
                has_stdout_redirect = true;
                fds.insert(1, sink);
                fds.insert(2, sink);
            }
            RedirectKind::DupOutput => {
                let fd = redirect.redirect().fd.unwrap_or(1);
                let sink = redirect_dup_output_sink(redirect, &fds);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupInput => {}
        }
    }

    let stdout_sink = *fds.get(&1).unwrap_or(&OutputSink::Other);
    let stderr_sink = *fds.get(&2).unwrap_or(&OutputSink::Other);
    let stdout_intent = if matches!(stdout_sink, OutputSink::Captured)
        || matches!(stderr_sink, OutputSink::Captured)
    {
        SubstitutionOutputIntent::Captured
    } else if matches!(stdout_sink, OutputSink::DevNull) {
        SubstitutionOutputIntent::Discarded
    } else {
        SubstitutionOutputIntent::Rerouted
    };

    RedirectState {
        stdout_intent,
        has_stdout_redirect,
    }
}

fn redirect_file_sink(redirect: &RedirectFact<'_>) -> OutputSink {
    match redirect.analysis() {
        Some(analysis) if analysis.is_definitely_dev_null() => OutputSink::DevNull,
        Some(_) => OutputSink::Other,
        None => OutputSink::Other,
    }
}

fn redirect_dup_output_sink(
    redirect: &RedirectFact<'_>,
    fds: &FxHashMap<i32, OutputSink>,
) -> OutputSink {
    let Some(fd) = redirect
        .analysis()
        .and_then(|analysis| analysis.numeric_descriptor_target)
    else {
        return OutputSink::Other;
    };

    *fds.get(&fd).unwrap_or(&OutputSink::Other)
}

fn visit_command_words_for_substitutions(
    command: &Command,
    redirects: &[Redirect],
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match command {
        Command::Simple(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            visitor(&command.name);
            visit_words_for_substitutions(&command.args, visitor);
        }
        Command::Builtin(command) => {
            visit_builtin_words_for_substitutions(command, source, visitor)
        }
        Command::Decl(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            for operand in &command.operands {
                visit_decl_operand_words_for_substitutions(operand, source, visitor);
            }
        }
        Command::Binary(_) => {}
        Command::Function(function) => {
            for entry in &function.header.entries {
                visitor(&entry.word);
            }
        }
        Command::AnonymousFunction(function) => {
            visit_words_for_substitutions(&function.args, visitor);
        }
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    visit_words_for_substitutions(words, visitor);
                }
            }
            CompoundCommand::Repeat(command) => visitor(&command.count),
            CompoundCommand::Foreach(command) => {
                visit_words_for_substitutions(&command.words, visitor)
            }
            CompoundCommand::Case(command) => {
                visitor(&command.word);
                for case in &command.cases {
                    visit_patterns_for_substitutions(&case.patterns, visitor);
                }
            }
            CompoundCommand::Select(command) => {
                visit_words_for_substitutions(&command.words, visitor)
            }
            CompoundCommand::Conditional(command) => {
                visit_conditional_words_for_substitutions(&command.expression, source, visitor);
            }
            CompoundCommand::If(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Always(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Time(_)
            | CompoundCommand::Coproc(_) => {}
        },
    }

    for redirect in redirects {
        visitor(redirect_scan_word(redirect));
    }
}

fn visit_command_argument_words_for_substitutions(
    command: &Command,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match command {
        Command::Simple(command) => {
            if static_word_text(&command.name, source).as_deref() == Some("trap") {
                return;
            }
            visit_words_for_substitutions(&command.args, visitor);
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
            BuiltinCommand::Continue(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
        },
        Command::Decl(command) => {
            for operand in &command.operands {
                if let DeclOperand::Dynamic(word) = operand {
                    visitor(word);
                }
            }
        }
        Command::Binary(_) | Command::Compound(_) => {}
        Command::Function(function) => {
            for entry in &function.header.entries {
                visitor(&entry.word);
            }
        }
        Command::AnonymousFunction(function) => {
            visit_words_for_substitutions(&function.args, visitor);
        }
    }
}

fn visit_declaration_assignment_words_for_substitutions(
    command: &Command,
    visitor: &mut impl FnMut(&Word),
) {
    let Command::Decl(command) = command else {
        return;
    };

    for operand in &command.operands {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };

        if let AssignmentValue::Scalar(word) = &assignment.value {
            visitor(word);
        }
    }
}

fn visit_command_subscript_words_for_substitutions(
    command: &Command,
    source: &str,
    visitor: &mut impl FnMut(SubstitutionHostKind, &Word),
) {
    for assignment in query::command_assignments(command) {
        query::visit_var_ref_subscript_words_with_source(&assignment.target, source, &mut |word| {
            visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
        });

        if let AssignmentValue::Compound(array) = &assignment.value {
            for element in &array.elements {
                if let shuck_ast::ArrayElem::Keyed { key, .. }
                | shuck_ast::ArrayElem::KeyedAppend { key, .. } = element
                {
                    query::visit_subscript_words(Some(key), source, &mut |word| {
                        visitor(SubstitutionHostKind::ArrayKeySubscript, word);
                    });
                }
            }
        }
    }

    for operand in query::declaration_operands(command) {
        match operand {
            DeclOperand::Name(reference) => {
                query::visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
                    visitor(SubstitutionHostKind::DeclarationNameSubscript, word);
                });
            }
            DeclOperand::Assignment(assignment) => {
                query::visit_var_ref_subscript_words_with_source(
                    &assignment.target,
                    source,
                    &mut |word| {
                        visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
                    },
                );

                if let AssignmentValue::Compound(array) = &assignment.value {
                    for element in &array.elements {
                        if let shuck_ast::ArrayElem::Keyed { key, .. }
                        | shuck_ast::ArrayElem::KeyedAppend { key, .. } = element
                        {
                            query::visit_subscript_words(Some(key), source, &mut |word| {
                                visitor(SubstitutionHostKind::ArrayKeySubscript, word);
                            });
                        }
                    }
                }
            }
            DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => {}
        }
    }
}

fn visit_assignments_for_substitutions(
    assignments: &[shuck_ast::Assignment],
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    for assignment in assignments {
        query::visit_var_ref_subscript_words_with_source(&assignment.target, source, visitor);

        match &assignment.value {
            AssignmentValue::Scalar(word) => visitor(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        shuck_ast::ArrayElem::Sequential(word) => visitor(word),
                        shuck_ast::ArrayElem::Keyed { key, value }
                        | shuck_ast::ArrayElem::KeyedAppend { key, value } => {
                            query::visit_subscript_words(Some(key), source, visitor);
                            visitor(value);
                        }
                    }
                }
            }
        }
    }
}

fn visit_builtin_words_for_substitutions(
    command: &BuiltinCommand,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match command {
        BuiltinCommand::Break(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.depth {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
        BuiltinCommand::Continue(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.depth {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
        BuiltinCommand::Return(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.code {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
        BuiltinCommand::Exit(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.code {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
    }
}

fn visit_decl_operand_words_for_substitutions(
    operand: &DeclOperand,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => visitor(word),
        DeclOperand::Name(reference) => {
            query::visit_var_ref_subscript_words_with_source(reference, source, visitor);
        }
        DeclOperand::Assignment(assignment) => {
            visit_assignments_for_substitutions(std::slice::from_ref(assignment), source, visitor);
        }
    }
}

fn visit_words_for_substitutions(words: &[Word], visitor: &mut impl FnMut(&Word)) {
    for word in words {
        visitor(word);
    }
}

fn visit_patterns_for_substitutions(patterns: &[Pattern], visitor: &mut impl FnMut(&Word)) {
    for pattern in patterns {
        visit_pattern_for_substitutions(pattern, visitor);
    }
}

fn visit_pattern_for_substitutions(pattern: &Pattern, visitor: &mut impl FnMut(&Word)) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                visit_patterns_for_substitutions(patterns, visitor)
            }
            PatternPart::Word(word) => visitor(word),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn visit_conditional_words_for_substitutions(
    expression: &ConditionalExpr,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            visit_conditional_words_for_substitutions(&expr.left, source, visitor);
            visit_conditional_words_for_substitutions(&expr.right, source, visitor);
        }
        ConditionalExpr::Unary(expr) => {
            visit_conditional_words_for_substitutions(&expr.expr, source, visitor);
        }
        ConditionalExpr::Parenthesized(expr) => {
            visit_conditional_words_for_substitutions(&expr.expr, source, visitor);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => visitor(word),
        ConditionalExpr::Pattern(pattern) => visit_pattern_for_substitutions(pattern, visitor),
        ConditionalExpr::VarRef(reference) => {
            query::visit_var_ref_subscript_words_with_source(reference, source, visitor);
        }
    }
}

fn redirect_scan_word(redirect: &Redirect) -> &Word {
    match redirect.word_target() {
        Some(word) => word,
        None => &redirect.heredoc().expect("expected heredoc redirect").body,
    }
}

fn build_for_header_facts<'a>(
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
) -> Vec<ForHeaderFact<'a>> {
    commands
        .iter()
        .filter_map(|fact| {
            let Command::Compound(CompoundCommand::For(command)) = fact.command() else {
                return None;
            };

            Some(ForHeaderFact {
                command,
                command_key: fact.key(),
                nested_word_command: fact.is_nested_word_command(),
                words: build_loop_header_word_facts(
                    command.words.iter().flat_map(|words| words.iter()),
                    commands,
                    command_index,
                    source,
                ),
            })
        })
        .collect()
}

fn build_select_header_facts<'a>(
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
) -> Vec<SelectHeaderFact<'a>> {
    commands
        .iter()
        .filter_map(|fact| {
            let Command::Compound(CompoundCommand::Select(command)) = fact.command() else {
                return None;
            };

            Some(SelectHeaderFact {
                command,
                command_key: fact.key(),
                nested_word_command: fact.is_nested_word_command(),
                words: build_loop_header_word_facts(
                    command.words.iter(),
                    commands,
                    command_index,
                    source,
                ),
            })
        })
        .collect()
}

fn build_loop_header_word_facts<'a>(
    words: impl IntoIterator<Item = &'a Word>,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
    source: &str,
) -> Box<[LoopHeaderWordFact<'a>]> {
    words
        .into_iter()
        .map(|word| {
            let classification = classify_word(word, source);
            LoopHeaderWordFact {
                word,
                classification,
                has_unquoted_command_substitution: classification.has_command_substitution()
                    && !span::unquoted_command_substitution_part_spans(word).is_empty(),
                contains_ls_substitution: word_contains_command_substitution_named(
                    word,
                    "ls",
                    commands,
                    command_index,
                ),
                contains_find_substitution: word_contains_find_substitution(
                    word,
                    commands,
                    command_index,
                ),
            }
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn build_pipeline_facts<'a>(
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> Vec<PipelineFact<'a>> {
    let mut nested_pipeline_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
            continue;
        }

        if matches!(
            &command.left.command,
            Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        ) {
            nested_pipeline_commands.insert(command_ptr(&command.left.command));
        }
        if matches!(
            &command.right.command,
            Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        ) {
            nested_pipeline_commands.insert(command_ptr(&command.right.command));
        }
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll)
                || nested_pipeline_commands.contains(&command_ptr(fact.command()))
            {
                return None;
            }

            let segments = query::pipeline_segments(fact.command())?;
            Some(PipelineFact {
                key: fact.key(),
                command,
                segments: segments
                    .into_iter()
                    .map(|stmt| build_pipeline_segment_fact(stmt, commands, command_index))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
        })
        .collect()
}

fn build_pipeline_segment_fact<'a>(
    stmt: &'a Stmt,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> PipelineSegmentFact<'a> {
    let fact = command_index
        .get(&command_ptr(&stmt.command))
        .map(|&index| &commands[index])
        .expect("pipeline segment should have a corresponding command fact");

    PipelineSegmentFact {
        stmt,
        command_key: fact.key(),
        literal_name: fact
            .literal_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
        effective_name: fact
            .effective_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
    }
}

fn build_list_facts<'a>(commands: &[CommandFact<'a>]) -> Vec<ListFact<'a>> {
    let mut nested_list_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::And | BinaryOp::Or) {
            continue;
        }

        if matches!(&command.left.command, Command::Binary(left) if matches!(left.op, BinaryOp::And | BinaryOp::Or))
        {
            nested_list_commands.insert(command_ptr(&command.left.command));
        }
        if matches!(&command.right.command, Command::Binary(right) if matches!(right.op, BinaryOp::And | BinaryOp::Or))
        {
            nested_list_commands.insert(command_ptr(&command.right.command));
        }
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::And | BinaryOp::Or)
                || nested_list_commands.contains(&command_ptr(fact.command()))
            {
                return None;
            }

            let mut operators = Vec::new();
            collect_short_circuit_operators(command, &mut operators);
            let mixed_short_circuit_span = mixed_short_circuit_operator_span(&operators);

            Some(ListFact {
                key: fact.key(),
                command,
                operators: operators.into_boxed_slice(),
                mixed_short_circuit_span,
            })
        })
        .collect()
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
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    word.parts
        .iter()
        .any(|part| part_contains_find_substitution(&part.kind, commands, command_index))
}

fn word_contains_command_substitution_named<'a>(
    word: &'a Word,
    name: &str,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    word.parts.iter().any(|part| {
        part_contains_command_substitution_named(&part.kind, name, commands, command_index)
    })
}

fn part_contains_command_substitution_named<'a>(
    part: &WordPart,
    name: &str,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts.iter().any(|part| {
            part_contains_command_substitution_named(&part.kind, name, commands, command_index)
        }),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            substitution_body_is_simple_command_named(body, name, commands, command_index)
        }
        _ => false,
    }
}

fn part_contains_find_substitution<'a>(
    part: &WordPart,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_find_substitution(&part.kind, commands, command_index)),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            substitution_body_is_find(body, commands, command_index)
        }
        _ => false,
    }
}

fn substitution_body_is_find<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    matches!(body.as_slice(), [stmt] if stmt_effective_name_is(stmt, "find", commands, command_index))
}

fn substitution_body_is_simple_command_named<'a>(
    body: &'a StmtSeq,
    name: &str,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    matches!(body.as_slice(), [stmt] if stmt_literal_name_is(stmt, name, commands, command_index))
}

fn stmt_effective_name_is<'a>(
    stmt: &'a Stmt,
    name: &str,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    command_index
        .get(&command_ptr(&stmt.command))
        .map(|&index| commands[index].effective_name_is(name))
        .unwrap_or(false)
}

fn stmt_literal_name_is<'a>(
    stmt: &'a Stmt,
    name: &str,
    commands: &[CommandFact<'a>],
    command_index: &FxHashMap<*const Command, usize>,
) -> bool {
    command_index
        .get(&command_ptr(&stmt.command))
        .and_then(|&index| commands[index].literal_name())
        == Some(name)
}

#[derive(Debug, Default)]
struct SurfaceFragmentFacts {
    single_quoted: Vec<SingleQuotedFragmentFact>,
    backticks: Vec<BacktickFragmentFact>,
    legacy_arithmetic: Vec<LegacyArithmeticFragmentFact>,
    positional_parameters: Vec<PositionalParameterFragmentFact>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SurfaceScanContext<'a> {
    command_name: Option<&'a str>,
    assignment_target: Option<&'a str>,
    variable_set_operand: bool,
}

impl<'a> SurfaceScanContext<'a> {
    fn with_assignment_target(self, assignment_target: &'a str) -> Self {
        Self {
            assignment_target: Some(assignment_target),
            ..self
        }
    }

    fn variable_set_operand(self) -> Self {
        Self {
            variable_set_operand: true,
            ..self
        }
    }
}

struct SurfaceFragmentCollector<'a> {
    commands: &'a [CommandFact<'a>],
    command_index: &'a FxHashMap<*const Command, usize>,
    source: &'a str,
    facts: SurfaceFragmentFacts,
}

impl<'a> SurfaceFragmentCollector<'a> {
    fn new(
        commands: &'a [CommandFact<'a>],
        command_index: &'a FxHashMap<*const Command, usize>,
        source: &'a str,
    ) -> Self {
        Self {
            commands,
            command_index,
            source,
            facts: SurfaceFragmentFacts::default(),
        }
    }

    fn finish(self) -> SurfaceFragmentFacts {
        self.facts
    }

    fn collect_commands(&mut self, commands: &StmtSeq) {
        for command in commands.iter() {
            self.collect_command(command);
        }
    }

    fn collect_command(&mut self, stmt: &Stmt) {
        let command_name_storage = self
            .command_fact_for_command(&stmt.command)
            .and_then(CommandFact::effective_or_literal_name)
            .map(str::to_owned)
            .map(String::into_boxed_str);
        let context = SurfaceScanContext {
            command_name: command_name_storage.as_deref(),
            ..SurfaceScanContext::default()
        };

        match &stmt.command {
            Command::Simple(command) => self.collect_simple_command(command, context),
            Command::Builtin(command) => self.collect_builtin(command),
            Command::Decl(command) => self.collect_decl_command(command),
            Command::Binary(command) => {
                self.collect_command(&command.left);
                self.collect_command(&command.right);
            }
            Command::Compound(command) => self.collect_compound(command),
            Command::Function(function) => {
                for entry in &function.header.entries {
                    self.collect_word(&entry.word, context.clone());
                }
                self.collect_command(&function.body);
            }
            Command::AnonymousFunction(function) => {
                for word in &function.args {
                    self.collect_word(word, context.clone());
                }
                self.collect_command(&function.body);
            }
        }

        self.collect_redirects(&stmt.redirects, SurfaceScanContext::default());
    }

    fn collect_simple_command(&mut self, command: &SimpleCommand, context: SurfaceScanContext<'_>) {
        self.collect_assignments(&command.assignments, context.clone());
        self.collect_word(&command.name, context.clone());

        let variable_set_operand = simple_command_variable_set_operand(command, self.source);
        for word in &command.args {
            let word_context =
                if variable_set_operand.is_some_and(|operand| std::ptr::eq(word, operand)) {
                    context.clone().variable_set_operand()
                } else {
                    context.clone()
                };
            self.collect_word(word, word_context);
        }
    }

    fn collect_builtin(&mut self, command: &BuiltinCommand) {
        let context = SurfaceScanContext::default();
        match command {
            BuiltinCommand::Break(command) => {
                self.collect_assignments(&command.assignments, context.clone());
                if let Some(word) = &command.depth {
                    self.collect_word(word, context.clone());
                }
                self.collect_words(&command.extra_args, context);
            }
            BuiltinCommand::Continue(command) => {
                self.collect_assignments(&command.assignments, context.clone());
                if let Some(word) = &command.depth {
                    self.collect_word(word, context.clone());
                }
                self.collect_words(&command.extra_args, context);
            }
            BuiltinCommand::Return(command) => {
                self.collect_assignments(&command.assignments, context.clone());
                if let Some(word) = &command.code {
                    self.collect_word(word, context.clone());
                }
                self.collect_words(&command.extra_args, context);
            }
            BuiltinCommand::Exit(command) => {
                self.collect_assignments(&command.assignments, context.clone());
                if let Some(word) = &command.code {
                    self.collect_word(word, context.clone());
                }
                self.collect_words(&command.extra_args, context);
            }
        }
    }

    fn collect_decl_command(&mut self, command: &DeclClause) {
        let context = SurfaceScanContext::default();
        self.collect_assignments(&command.assignments, context.clone());
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                    self.collect_word(word, context.clone());
                }
                DeclOperand::Name(reference) => {
                    query::visit_var_ref_subscript_words_with_source(
                        reference,
                        self.source,
                        &mut |word| self.collect_word(word, context.clone()),
                    );
                }
                DeclOperand::Assignment(assignment) => {
                    self.collect_assignment(assignment, context.clone())
                }
            }
        }
    }

    fn collect_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
                self.collect_commands(&command.condition);
                self.collect_commands(&command.then_branch);
                for (condition, body) in &command.elif_branches {
                    self.collect_commands(condition);
                    self.collect_commands(body);
                }
                if let Some(body) = &command.else_branch {
                    self.collect_commands(body);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    self.collect_words(words, SurfaceScanContext::default());
                }
                self.collect_commands(&command.body);
            }
            CompoundCommand::Repeat(command) => {
                self.collect_word(&command.count, SurfaceScanContext::default());
                self.collect_commands(&command.body);
            }
            CompoundCommand::Foreach(command) => {
                self.collect_words(&command.words, SurfaceScanContext::default());
                self.collect_commands(&command.body);
            }
            CompoundCommand::ArithmeticFor(command) => self.collect_commands(&command.body),
            CompoundCommand::While(command) => {
                self.collect_commands(&command.condition);
                self.collect_commands(&command.body);
            }
            CompoundCommand::Until(command) => {
                self.collect_commands(&command.condition);
                self.collect_commands(&command.body);
            }
            CompoundCommand::Case(command) => {
                self.collect_word(&command.word, SurfaceScanContext::default());
                for case in &command.cases {
                    self.collect_patterns(&case.patterns, SurfaceScanContext::default());
                    self.collect_commands(&case.body);
                }
            }
            CompoundCommand::Select(command) => {
                self.collect_words(&command.words, SurfaceScanContext::default());
                self.collect_commands(&command.body);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.collect_commands(commands);
            }
            CompoundCommand::Always(command) => {
                self.collect_commands(&command.body);
                self.collect_commands(&command.always_body);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.collect_command(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.collect_conditional_expr(&command.expression, SurfaceScanContext::default());
            }
            CompoundCommand::Coproc(command) => self.collect_command(&command.body),
        }
    }

    fn collect_assignments(&mut self, assignments: &[Assignment], context: SurfaceScanContext<'_>) {
        for assignment in assignments {
            self.collect_assignment(assignment, context.clone());
        }
    }

    fn collect_assignment(&mut self, assignment: &Assignment, context: SurfaceScanContext<'_>) {
        let context = context.with_assignment_target(assignment.target.name.as_str());
        query::visit_var_ref_subscript_words_with_source(
            &assignment.target,
            self.source,
            &mut |word| self.collect_word(word, context.clone()),
        );
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.collect_word(word, context.clone()),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.collect_word(word, context.clone()),
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            query::visit_subscript_words(Some(key), self.source, &mut |word| {
                                self.collect_word(word, context.clone());
                            });
                            self.collect_word(value, context.clone());
                        }
                    }
                }
            }
        }
    }

    fn collect_words(&mut self, words: &[Word], context: SurfaceScanContext<'_>) {
        for word in words {
            self.collect_word(word, context.clone());
        }
    }

    fn collect_patterns(&mut self, patterns: &[Pattern], context: SurfaceScanContext<'_>) {
        for pattern in patterns {
            self.collect_pattern(pattern, context.clone());
        }
    }

    fn collect_word(&mut self, word: &Word, context: SurfaceScanContext<'_>) {
        self.collect_word_parts(&word.parts, context);
    }

    fn collect_word_parts(&mut self, parts: &[WordPartNode], context: SurfaceScanContext<'_>) {
        for (index, part) in parts.iter().enumerate() {
            if let WordPart::Variable(name) = &part.kind
                && matches!(
                    name.as_str(),
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                )
                && let Some(next_part) = parts.get(index + 1)
                && let WordPart::Literal(text) = &next_part.kind
                && text
                    .as_str(self.source, next_part.span)
                    .starts_with(|char: char| char.is_ascii_digit())
            {
                self.facts
                    .positional_parameters
                    .push(PositionalParameterFragmentFact {
                        span: part.span.merge(next_part.span),
                    });
            }

            match &part.kind {
                WordPart::SingleQuoted { .. } => {
                    self.facts.single_quoted.push(SingleQuotedFragmentFact {
                        span: part.span,
                        command_name: context
                            .command_name
                            .map(str::to_owned)
                            .map(String::into_boxed_str),
                        assignment_target: context
                            .assignment_target
                            .map(str::to_owned)
                            .map(String::into_boxed_str),
                        variable_set_operand: context.variable_set_operand,
                    });
                }
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_word_parts(parts, context.clone())
                }
                WordPart::ZshQualifiedGlob(glob) => {
                    self.collect_zsh_qualified_glob(glob, context.clone())
                }
                WordPart::ArithmeticExpansion {
                    syntax: ArithmeticExpansionSyntax::LegacyBracket,
                    expression_ast,
                    ..
                } => {
                    self.facts
                        .legacy_arithmetic
                        .push(LegacyArithmeticFragmentFact { span: part.span });
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context.clone());
                        });
                    }
                }
                WordPart::ArithmeticExpansion { expression_ast, .. } => {
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context.clone());
                        });
                    }
                }
                WordPart::CommandSubstitution {
                    syntax: CommandSubstitutionSyntax::Backtick,
                    body,
                    ..
                } => {
                    self.facts
                        .backticks
                        .push(BacktickFragmentFact { span: part.span });
                    self.collect_commands(body);
                }
                WordPart::CommandSubstitution { body, .. }
                | WordPart::ProcessSubstitution { body, .. } => self.collect_commands(body),
                WordPart::Parameter(parameter) => {
                    if let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                        operator,
                        ..
                    }) = &parameter.syntax
                    {
                        self.collect_parameter_operator_patterns(operator, context.clone());
                    }
                }
                WordPart::ParameterExpansion { operator, .. } => {
                    self.collect_parameter_operator_patterns(operator, context.clone());
                }
                WordPart::IndirectExpansion {
                    operator: Some(operator),
                    ..
                } => self.collect_parameter_operator_patterns(operator, context.clone()),
                WordPart::Literal(_)
                | WordPart::Variable(_)
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { operator: None, .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::Transformation { .. } => {}
            }
        }
    }

    fn collect_pattern(&mut self, pattern: &Pattern, context: SurfaceScanContext<'_>) {
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => {
                    self.collect_patterns(patterns, context.clone())
                }
                PatternPart::Word(word) => self.collect_word(word, context.clone()),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn collect_zsh_qualified_glob(
        &mut self,
        glob: &ZshQualifiedGlob,
        context: SurfaceScanContext<'_>,
    ) {
        for segment in &glob.segments {
            if let ZshGlobSegment::Pattern(pattern) = segment {
                self.collect_pattern(pattern, context.clone());
            }
        }
    }

    fn collect_redirects(&mut self, redirects: &[Redirect], context: SurfaceScanContext<'_>) {
        for redirect in redirects {
            match redirect.word_target() {
                Some(word) => self.collect_word(word, context.clone()),
                None => {
                    let heredoc = redirect.heredoc().expect("expected heredoc redirect");
                    if heredoc.delimiter.expands_body {
                        self.collect_word(&heredoc.body, context.clone());
                    }
                }
            }
        }
    }

    fn collect_conditional_expr(
        &mut self,
        expression: &ConditionalExpr,
        context: SurfaceScanContext<'_>,
    ) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.collect_conditional_expr(&expr.left, context.clone());
                self.collect_conditional_expr(&expr.right, context);
            }
            ConditionalExpr::Unary(expr) => {
                let context = if expr.op == ConditionalUnaryOp::VariableSet {
                    context.variable_set_operand()
                } else {
                    context
                };
                self.collect_conditional_expr(&expr.expr, context);
            }
            ConditionalExpr::Parenthesized(expr) => {
                self.collect_conditional_expr(&expr.expr, context);
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.collect_word(word, context)
            }
            ConditionalExpr::Pattern(pattern) => self.collect_pattern(pattern, context),
            ConditionalExpr::VarRef(reference) => {
                query::visit_var_ref_subscript_words_with_source(
                    reference,
                    self.source,
                    &mut |word| self.collect_word(word, context.clone()),
                );
            }
        }
    }

    fn collect_parameter_operator_patterns(
        &mut self,
        operator: &ParameterOp,
        context: SurfaceScanContext<'_>,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern }
            | ParameterOp::ReplaceFirst { pattern, .. }
            | ParameterOp::ReplaceAll { pattern, .. } => self.collect_pattern(pattern, context),
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

    fn command_fact_for_command(&self, command: &Command) -> Option<&CommandFact<'a>> {
        self.command_index
            .get(&command_ptr(command))
            .map(|&index| &self.commands[index])
    }
}

fn build_surface_fragment_facts<'a>(
    file: &'a File,
    commands: &'a [CommandFact<'a>],
    command_index: &'a FxHashMap<*const Command, usize>,
    source: &'a str,
) -> SurfaceFragmentFacts {
    let mut collector = SurfaceFragmentCollector::new(commands, command_index, source);
    collector.collect_commands(&file.body);
    collector.finish()
}

fn simple_command_variable_set_operand<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
    let operands = simple_test_operands(command, source)?;
    (operands.len() == 2 && static_word_text(&operands[0], source).as_deref() == Some("-v"))
        .then(|| &operands[1])
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

fn command_ptr(command: &Command) -> *const Command {
    std::ptr::from_ref(command)
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

    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
    use shuck_semantic::SemanticModel;

    use super::{
        ConditionalNodeFact, ConditionalOperatorFamily, LinterFacts, SimpleTestOperatorFamily,
        SimpleTestShape, SimpleTestSyntax, SubstitutionHostKind, SudoFamilyInvoker,
        WordFactHostKind,
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
    }

    #[test]
    fn exposes_structural_commands_and_stable_lookups() {
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
        assert!(facts.command_for_stmt(&output.file.body[0]).is_some());
        assert!(
            facts
                .command_for_command(&output.file.body[0].command)
                .is_some()
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
        let source = "#!/bin/bash\nread -r name\nprintf -v out \"$fmt\" value\nunset -f curl other\nfind . -print0 | xargs -0 rm\nexit foo\ndoas printf '%s\\n' hi\n";
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
printf '%s\\n' 123 | command kill -9 | tee out.txt
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
PS4='$prompt'
command jq '$__loc__'
test -v '$name'
printf '%s\\n' 123 | command kill -9
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
