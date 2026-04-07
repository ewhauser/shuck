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
    AssignmentValue, BinaryCommand, BinaryOp, BuiltinCommand, Command, CompoundCommand,
    ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, DeclOperand, File, ForCommand,
    Redirect, SelectCommand, Span, Stmt, StmtSeq, Word, WordPart,
};
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;

use crate::FileContext;
use crate::context::ContextRegionKind;
use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::{
    command::{self, NormalizedCommand, WrapperKind},
    query::{self, CommandVisit, CommandWalkOptions},
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

#[derive(Debug, Clone, Copy)]
pub struct LoopHeaderWordFact<'a> {
    word: &'a Word,
    classification: WordClassification,
    has_unquoted_command_substitution: bool,
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
    options: CommandOptionFacts<'a>,
    simple_test: Option<SimpleTestFact<'a>>,
    conditional: Option<ConditionalFact<'a>>,
}

impl<'a> CommandFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn visit(&self) -> CommandVisit<'a> {
        self.visit
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
    for_headers: Vec<ForHeaderFact<'a>>,
    select_headers: Vec<SelectHeaderFact<'a>>,
    pipelines: Vec<PipelineFact<'a>>,
    lists: Vec<ListFact<'a>>,
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
            let options = CommandOptionFacts::build(visit.command, &normalized, self.source);
            let simple_test =
                build_simple_test_fact(visit.command, self.source, self._file_context);
            let conditional = build_conditional_fact(visit.command, self.source);
            commands.push(CommandFact {
                key,
                visit,
                nested_word_command,
                normalized,
                options,
                simple_test,
                conditional,
            });
        }

        let for_headers = build_for_header_facts(&commands, &command_index, self.source);
        let select_headers = build_select_header_facts(&commands, &command_index, self.source);
        let pipelines = build_pipeline_facts(&commands, &command_index);
        let lists = build_list_facts(&commands);

        LinterFacts {
            commands,
            structural_command_indices,
            command_index,
            scalar_bindings,
            for_headers,
            select_headers,
            pipelines,
            lists,
        }
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
    if let Command::Binary(left) = &command.left.command {
        if matches!(left.op, BinaryOp::And | BinaryOp::Or) {
            collect_short_circuit_operators(left, operators);
        }
    }

    if matches!(command.op, BinaryOp::And | BinaryOp::Or) {
        operators.push(ListOperatorFact {
            op: command.op,
            span: command.op_span,
        });
    }

    if let Command::Binary(right) = &command.right.command {
        if matches!(right.op, BinaryOp::And | BinaryOp::Or) {
            collect_short_circuit_operators(right, operators);
        }
    }
}

fn mixed_short_circuit_operator_span(operators: &[ListOperatorFact]) -> Option<Span> {
    let mut current = None;

    for operator in operators {
        match current {
            None => current = Some(operator.op()),
            Some(previous) if previous == operator.op() => {}
            Some(_) => return Some(operator.span()),
        }
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

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

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

    std::iter::once(&command.name)
        .chain(command.args.iter())
        .take_while(|word| word.span.start.offset < body_start)
        .filter_map(|word| static_word_text(word, source))
        .filter_map(|word| match word.as_str() {
            "sudo" => Some(SudoFamilyInvoker::Sudo),
            "doas" => Some(SudoFamilyInvoker::Doas),
            "run0" => Some(SudoFamilyInvoker::Run0),
            _ => None,
        })
        .last()
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
    use shuck_parser::parser::Parser;
    use shuck_semantic::SemanticModel;

    use super::{
        ConditionalNodeFact, ConditionalOperatorFamily, LinterFacts, SimpleTestOperatorFamily,
        SimpleTestShape, SimpleTestSyntax, SudoFamilyInvoker,
    };
    use crate::rules::common::command::WrapperKind;
    use crate::{ShellDialect, classify_file_context};

    fn with_facts(
        source: &str,
        path: Option<&Path>,
        visit: impl FnOnce(&shuck_parser::parser::ParseOutput, &LinterFacts<'_>),
    ) {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, path, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        visit(&output, &facts);
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
                Some("||")
            );
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
}
