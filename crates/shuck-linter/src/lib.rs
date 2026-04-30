#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! Linting and fix application for shell scripts parsed by the Shuck toolchain.
//!
//! This crate combines parser output, semantic analysis, suppressions, and rule metadata into a
//! diagnostics pipeline used by `shuck check`.
mod ambient_contracts;
#[allow(missing_docs)]
mod checker;
#[allow(missing_docs)]
mod diagnostic;
#[allow(missing_docs)]
mod facts;
#[allow(missing_docs)]
mod fix;
#[allow(missing_docs)]
mod fix_helpers;
#[allow(missing_docs)]
mod parse_diagnostics;
#[allow(missing_docs)]
mod registry;
#[allow(missing_docs)]
mod rule_metadata;
#[allow(missing_docs)]
mod rule_selector;
#[allow(missing_docs)]
mod rule_set;
#[allow(missing_docs)]
/// Rule implementations and rule-oriented helper modules.
pub mod rules;
#[allow(missing_docs)]
mod settings;
#[allow(missing_docs)]
mod shell;
#[allow(missing_docs)]
mod suppression;
#[allow(missing_docs)]
mod violation;

#[cfg(test)]
/// Test helpers for rule and fix assertions.
#[allow(missing_docs)]
pub mod test;

/// Primary checker API for walking facts and emitting diagnostics.
pub use checker::Checker;
/// Rule diagnostics and severity levels.
pub use diagnostic::{Diagnostic, Severity};
/// Command-substitution classification exposed by fact APIs.
pub use facts::CommandSubstitutionKind;
pub use facts::words::{
    ExpansionAnalysis, ExpansionContext, ExpansionHazards, ExpansionValueShape,
    RuntimeLiteralAnalysis, TestOperandClass, WordClassification, WordExpansionKind,
    WordFactContext, WordFactHostKind, WordLiteralness, WordOccurrence, WordOccurrenceIter,
    WordOccurrenceRef, WordQuote, WordSubstitutionShape, leading_literal_word_prefix,
};
/// Extracted structural facts available to rules and callers.
pub use facts::{
    BacktickFragmentFact, CommandFact, CommandFactRef, CommandFacts, ConditionalBareWordFact,
    ConditionalBinaryFact, ConditionalFact, ConditionalMixedLogicalOperatorFact,
    ConditionalNodeFact, ConditionalOperandFact, ConditionalOperatorFamily,
    ConditionalPortabilityFacts, ConditionalUnaryFact, ForHeaderFact, FunctionCallArityFacts,
    FunctionHeaderFact, LegacyArithmeticFragmentFact, ListFact, ListOperatorFact,
    LoopHeaderWordFact, PipelineFact, PipelineOperatorFact, PipelineSegmentFact,
    PositionalParameterFragmentFact, RedirectDevNullStatus, RedirectFact, RedirectTargetAnalysis,
    RedirectTargetKind, SelectHeaderFact, SimpleTestFact, SimpleTestOperatorFamily,
    SimpleTestShape, SimpleTestSyntax, SingleQuotedFragmentFact, StatementFact, SubstitutionFact,
    SubstitutionHostKind, SubstitutionOutputIntent, SudoFamilyInvoker,
};
/// Fact collection types and stable identifiers into those collections.
pub use facts::{
    CommandId, DeclarationKind, FactSpan, LinterFacts, NormalizedCommand, NormalizedDeclaration,
    WrapperKind,
};
pub(crate) use facts::{
    ComparableNameKey, ComparableNameUseKind, ComparablePathKey, ComparablePathMatchKey,
};
/// Autofix types and fix application helpers.
pub use fix::{Applicability, AppliedFixes, Edit, Fix, FixAvailability, apply_fixes};
pub(crate) use fix_helpers::leading_static_word_prefix_fix_in_source;
/// Rule identifiers, categories, and registry lookup helpers.
pub use registry::{Category, Rule, code_to_rule};
/// Rule metadata lookup utilities.
pub use rule_metadata::{RuleMetadata, ShellCheckLevel, rule_metadata, rule_metadata_by_code};
/// Rule selector parsing types.
pub use rule_selector::{RuleSelector, SelectorParseError};
/// Sets of enabled or disabled rules.
pub use rule_set::RuleSet;
#[allow(unused_imports)]
pub(crate) use rules::common::word::conditional_binary_op_is_string_match;
/// Linter configuration and per-file ignore types.
pub use settings::{
    AmbientShellOptions, C001RuleOptions, C063RuleOptions, CompiledPerFileIgnoreList,
    LinterRuleOptions, LinterSettings, PerFileIgnore,
};
/// Shell dialect selection used by the linter.
pub use shell::ShellDialect;
pub(crate) use suppression::parse_directives;
/// Suppression directives, shellcheck mappings, and rewrite helpers.
pub use suppression::{
    AddIgnoreParseError, AddIgnoreResult, ShellCheckCodeMap, SuppressionAction,
    SuppressionDirective, SuppressionIndex, SuppressionSource, add_ignores_to_path,
};
/// Trait implemented by rule-specific diagnostic payloads.
pub use violation::Violation;

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, CompoundCommand, File, Position, Span, Stmt, StmtSeq, TextSize};
use shuck_indexer::{Indexer, LineIndex};
use shuck_parser::parser::{ParseResult, Parser};
use shuck_semantic::{
    CommandKind, CompoundCommandKind, FlowContext, ScopeId, SemanticBuildOptions, SemanticModel,
    SourcePathResolver, TraversalObserver, build_with_observer_with_options,
};
use std::ops::Deref;
use std::path::Path;

use crate::suppression::{
    DirectiveAttachmentFacts, DirectiveCommandVisit, filter_attached_directives,
    first_statement_line, sort_command_spans_for_lookup, statement_suppression_span,
};

/// Combined semantic model and diagnostic output for a file analysis pass.
pub struct AnalysisResult {
    /// Semantic model built for the analyzed file.
    pub semantic: SemanticModel,
    /// Diagnostics emitted by linter rules and parse checks.
    pub diagnostics: Vec<Diagnostic>,
}

struct LinterAnalysisResult<'a> {
    semantic: LinterSemanticArtifacts<'a>,
    diagnostics: Vec<Diagnostic>,
}

/// Semantic model plus linter-private traversal artifacts needed to build facts.
pub struct LinterSemanticArtifacts<'a> {
    semantic: SemanticModel,
    command_visits_by_id: Vec<Option<facts::CommandVisit<'a>>>,
    conditional_expression_visits_by_command_span:
        FxHashMap<facts::FactSpan, Vec<facts::ConditionalExpressionVisit<'a>>>,
    direct_command_ids_by_body_span: FxHashMap<facts::FactSpan, Vec<CommandId>>,
    suppression_command_spans: Vec<Span>,
    directive_attachment_facts: DirectiveAttachmentFacts,
}

impl<'a> LinterSemanticArtifacts<'a> {
    /// Builds semantic analysis artifacts for linter fact construction.
    pub fn build(file: &'a File, source: &'a str, indexer: &'a Indexer) -> Self {
        Self::build_with_options(file, source, indexer, SemanticBuildOptions::default())
    }

    /// Builds semantic analysis artifacts for linter fact construction with custom options.
    pub fn build_with_options(
        file: &'a File,
        source: &'a str,
        indexer: &'a Indexer,
        options: SemanticBuildOptions<'_>,
    ) -> Self {
        let mut observer = LintTraversalObserver::default();
        let semantic =
            build_with_observer_with_options(file, source, indexer, &mut observer, options);
        let mut suppression_command_spans = observer.collect_suppression_command_spans(&semantic);
        sort_command_spans_for_lookup(&mut suppression_command_spans);
        let directive_attachment_facts = DirectiveAttachmentFacts::from_command_visits(
            source,
            indexer.comment_index(),
            observer.command_visits_by_id.iter().filter_map(|visit| {
                visit.map(|visit| DirectiveCommandVisit {
                    stmt: visit.stmt,
                    command: visit.command,
                })
            }),
        );
        Self {
            semantic,
            command_visits_by_id: observer.command_visits_by_id,
            conditional_expression_visits_by_command_span: observer
                .conditional_expression_visits_by_command_span,
            direct_command_ids_by_body_span: observer.direct_command_ids_by_body_span,
            suppression_command_spans,
            directive_attachment_facts,
        }
    }

    /// Returns the semantic model built for this file.
    pub fn semantic(&self) -> &SemanticModel {
        &self.semantic
    }

    /// Converts this linter semantic model into the underlying semantic model.
    pub fn into_semantic(self) -> SemanticModel {
        self.semantic
    }

    pub(crate) fn command_visits_by_id(&self) -> &[Option<facts::CommandVisit<'a>>] {
        &self.command_visits_by_id
    }

    pub(crate) fn conditional_expression_visits(
        &self,
        command_span: Span,
    ) -> &[facts::ConditionalExpressionVisit<'a>] {
        self.conditional_expression_visits_by_command_span
            .get(&facts::FactSpan::new(command_span))
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn suppression_command_spans(&self) -> &[Span] {
        &self.suppression_command_spans
    }

    pub(crate) fn directive_attachment_facts(&self) -> &DirectiveAttachmentFacts {
        &self.directive_attachment_facts
    }

    pub(crate) fn missing_done_trailing_loop_is_for(
        &self,
        body: &StmtSeq,
        eof_offset: usize,
    ) -> Option<bool> {
        let mut trailing_loop_kind = None;
        self.for_each_command_visit_in_body(body, false, |visit| {
            if visit.stmt.span.end.offset != eof_offset {
                return;
            }

            let is_for_loop = match visit.command {
                Command::Compound(CompoundCommand::For(_)) => true,
                Command::Compound(CompoundCommand::While(_) | CompoundCommand::Until(_)) => false,
                _ => return,
            };

            let start_offset = visit.stmt.span.start.offset;
            if trailing_loop_kind
                .as_ref()
                .is_none_or(|(best_start, _)| start_offset >= *best_start)
            {
                trailing_loop_kind = Some((start_offset, is_for_loop));
            }
        });

        trailing_loop_kind.map(|(_, is_for_loop)| is_for_loop)
    }

    #[cfg(test)]
    pub(crate) fn command_visits_in_body(
        &self,
        body: &StmtSeq,
        descend_nested_word_commands: bool,
    ) -> Vec<facts::CommandVisit<'a>> {
        let mut visits = Vec::new();
        self.for_each_command_visit_in_body(body, descend_nested_word_commands, |visit| {
            visits.push(visit);
        });
        visits
    }

    pub(crate) fn for_each_command_visit_in_body(
        &self,
        body: &StmtSeq,
        descend_nested_word_commands: bool,
        mut visitor: impl FnMut(facts::CommandVisit<'a>),
    ) {
        let body_span = facts::FactSpan::new(body.span);
        let Some(root_ids) = self.direct_command_ids_by_body_span.get(&body_span) else {
            return;
        };
        let baseline_depth = root_ids
            .iter()
            .filter_map(|id| self.semantic.command_context(*id))
            .map(|context| context.nested_word_command_depth())
            .min()
            .unwrap_or(0);
        let mut stack = root_ids.iter().rev().copied().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            let Some(context) = self.semantic.command_context(id) else {
                continue;
            };
            if !descend_nested_word_commands && context.nested_word_command_depth() > baseline_depth
            {
                continue;
            }
            if let Some(visit) = self
                .command_visits_by_id
                .get(id.index())
                .and_then(|visit| *visit)
            {
                visitor(visit);
            }
            for child in self
                .semantic
                .syntax_backed_command_children(id)
                .iter()
                .rev()
            {
                stack.push(*child);
            }
        }
    }
}

impl Deref for LinterSemanticArtifacts<'_> {
    type Target = SemanticModel;

    fn deref(&self) -> &Self::Target {
        self.semantic()
    }
}

#[derive(Default)]
struct LintTraversalObserver<'a> {
    command_visits_by_id: Vec<Option<facts::CommandVisit<'a>>>,
    conditional_expression_visits_by_command_span:
        FxHashMap<facts::FactSpan, Vec<facts::ConditionalExpressionVisit<'a>>>,
    direct_command_ids_by_body_span: FxHashMap<facts::FactSpan, Vec<CommandId>>,
    suppression_command_spans_by_id: Vec<Option<Span>>,
}

impl LintTraversalObserver<'_> {
    fn collect_suppression_command_spans(&self, semantic: &SemanticModel) -> Vec<Span> {
        let mut spans = Vec::new();
        for id in semantic.commands().iter().copied() {
            let Some(Some(span)) = self.suppression_command_spans_by_id.get(id.index()) else {
                continue;
            };
            if suppression_span_is_function_body_wrapper(semantic, id) {
                continue;
            }
            spans.push(*span);
        }
        spans
    }
}

impl<'a> TraversalObserver<'a> for LintTraversalObserver<'a> {
    fn conditional_expression(
        &mut self,
        command_span: Span,
        expression: &'a shuck_ast::ConditionalExpr,
        parent_in_same_logical_group: bool,
    ) {
        self.conditional_expression_visits_by_command_span
            .entry(facts::FactSpan::new(command_span))
            .or_default()
            .push(facts::ConditionalExpressionVisit::new(
                expression,
                parent_in_same_logical_group,
            ));
    }

    fn recorded_command(
        &mut self,
        id: CommandId,
        stmt: &'a Stmt,
        _scope: ScopeId,
        _flow: FlowContext,
    ) {
        if self.command_visits_by_id.len() <= id.index() {
            self.command_visits_by_id.resize(id.index() + 1, None);
        }
        self.command_visits_by_id[id.index()] = Some(facts::CommandVisit::new(stmt));
        if self.suppression_command_spans_by_id.len() <= id.index() {
            self.suppression_command_spans_by_id
                .resize(id.index() + 1, None);
        }
        let span = statement_suppression_span(stmt);
        if span.start.line != 0 && span.end.line != 0 {
            self.suppression_command_spans_by_id[id.index()] = Some(span);
        }
    }

    fn recorded_statement_sequence_command(
        &mut self,
        body_span: Span,
        _stmt_span: Span,
        id: CommandId,
    ) {
        self.direct_command_ids_by_body_span
            .entry(facts::FactSpan::new(body_span))
            .or_default()
            .push(id);
    }
}

fn suppression_span_is_function_body_wrapper(semantic: &SemanticModel, id: CommandId) -> bool {
    matches!(
        semantic.command_kind(id),
        CommandKind::Compound(CompoundCommandKind::BraceGroup)
            | CommandKind::Compound(CompoundCommandKind::Subshell)
    ) && semantic.command_parent_id(id).is_some_and(|parent| {
        matches!(
            semantic.command_kind(parent),
            CommandKind::Function | CommandKind::AnonymousFunction
        )
    })
}

fn build_suppression_index_from_semantic(
    directives: &[SuppressionDirective],
    file: &File,
    source: &str,
    semantic: &LinterSemanticArtifacts<'_>,
) -> Option<SuppressionIndex> {
    if directives.is_empty() {
        return None;
    }

    let directives =
        filter_attached_directives(source, directives, semantic.directive_attachment_facts());
    if directives.is_empty() {
        return None;
    }

    Some(SuppressionIndex::from_sorted_command_spans(
        &directives,
        semantic.suppression_command_spans(),
        first_statement_line(file).unwrap_or(u32::MAX),
    ))
}

/// Builds semantic facts and linter diagnostics for a parsed file.
pub fn analyze_file(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> AnalysisResult {
    analyze_file_at_path(file, source, indexer, settings, suppression_index, None)
}

#[cfg(feature = "benchmarking")]
#[doc(hidden)]
pub use facts::benchmark::CasePatternMatcher as BenchmarkCasePatternMatcher;

/// Builds semantic facts and linter diagnostics for a parsed file at an optional source path.
pub fn analyze_file_at_path(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
) -> AnalysisResult {
    analyze_file_at_path_with_resolver(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        None,
    )
}

/// Builds semantic facts and linter diagnostics with a custom source-path resolver.
pub fn analyze_file_at_path_with_resolver(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> AnalysisResult {
    let shell = resolve_shell(settings, source, source_path);
    let first_parse_error = parse_error_position(&parse_for_lint(source, shell));

    analyze_file_at_path_with_resolver_and_shell(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        source_path_resolver,
        shell,
        first_parse_error,
    )
}

#[allow(clippy::too_many_arguments)]
fn analyze_file_at_path_with_resolver_and_shell(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    shell: ShellDialect,
    first_parse_error: Option<(usize, usize)>,
) -> AnalysisResult {
    let result = analyze_linter_file_at_path_with_resolver_and_shell(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        source_path_resolver,
        shell,
        first_parse_error,
    );
    AnalysisResult {
        semantic: result.semantic.into_semantic(),
        diagnostics: result.diagnostics,
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze_linter_file_at_path_with_resolver_and_shell<'a>(
    file: &'a File,
    source: &'a str,
    indexer: &'a Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    shell: ShellDialect,
    first_parse_error: Option<(usize, usize)>,
) -> LinterAnalysisResult<'a> {
    let mut file_entry_contract_collector =
        ambient_contracts::AmbientContractCollector::new(source, source_path, shell);
    let analyzed_paths_fallback =
        source_path.map(|path| FxHashSet::from_iter([path.to_path_buf()]));
    let analyzed_paths = settings
        .analyzed_paths
        .as_deref()
        .or(analyzed_paths_fallback.as_ref());

    let shell_profile = shell.shell_profile();
    let linter_semantic_artifacts = LinterSemanticArtifacts::build_with_options(
        file,
        source,
        indexer,
        SemanticBuildOptions {
            source_path,
            source_path_resolver,
            file_entry_contract: None,
            file_entry_contract_collector: Some(&mut file_entry_contract_collector),
            analyzed_paths,
            shell_profile: Some(shell_profile),
            resolve_source_closure: settings.resolve_source_closure,
        },
    );
    let checker_diagnostics = {
        let checker = Checker::new(
            file,
            source,
            &linter_semantic_artifacts,
            indexer,
            &settings.rules,
            shell,
            settings.ambient_shell_options,
            settings.report_environment_style_names,
            settings.rule_options.clone(),
            suppression_index,
            first_parse_error,
        );
        checker.check()
    };
    let mut diagnostics = checker_diagnostics;
    for diagnostic in &mut diagnostics {
        if let Some(&severity) = settings.severity_overrides.get(&diagnostic.rule) {
            diagnostic.severity = severity;
        }
    }

    if let Some(suppression_index) = suppression_index {
        filter_suppressed_diagnostics(&mut diagnostics, indexer, suppression_index);
    }
    filter_per_file_ignored_diagnostics(&mut diagnostics, settings, source_path);

    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.start.offset, diagnostic.span.end.offset));
    LinterAnalysisResult {
        semantic: linter_semantic_artifacts,
        diagnostics,
    }
}

fn resolve_shell(
    settings: &LinterSettings,
    source: &str,
    source_path: Option<&Path>,
) -> ShellDialect {
    if settings.shell == ShellDialect::Unknown {
        ShellDialect::infer(source, source_path)
    } else {
        settings.shell
    }
}

fn parse_for_lint(source: &str, shell: ShellDialect) -> ParseResult {
    Parser::with_profile(source, shell.shell_profile()).parse()
}

fn parse_error_position(parse_result: &ParseResult) -> Option<(usize, usize)> {
    if !parse_result.is_err() {
        return None;
    }

    let shuck_parser::Error::Parse { line, column, .. } = parse_result.strict_error();
    if line > 0 && column > 0 {
        return Some((line, column));
    }

    parse_result
        .diagnostics
        .first()
        .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
}

/// Lints a parsed file located at an optional source path.
pub fn lint_file_at_path(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
) -> Vec<Diagnostic> {
    lint_file_at_path_with_resolver(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        None,
    )
}

/// Lints a parsed file with a custom source-path resolver.
pub fn lint_file_at_path_with_resolver(
    file: &File,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<Diagnostic> {
    let shell = resolve_shell(settings, source, source_path);
    let parse_result = parse_for_lint(source, shell);

    let analysis = analyze_linter_file_at_path_with_resolver_and_shell(
        file,
        source,
        indexer,
        settings,
        suppression_index,
        source_path,
        source_path_resolver,
        shell,
        parse_error_position(&parse_result),
    );
    let mut diagnostics = analysis.diagnostics;

    diagnostics.extend(parse_diagnostics::collect_parse_rule_diagnostics(
        file,
        source,
        Some(&parse_result),
        &analysis.semantic,
        &settings.rules,
        shell,
    ));

    for diagnostic in &mut diagnostics {
        if let Some(&severity) = settings.severity_overrides.get(&diagnostic.rule) {
            diagnostic.severity = severity;
        }
    }

    if let Some(suppression_index) = suppression_index {
        filter_suppressed_diagnostics(&mut diagnostics, indexer, suppression_index);
    }
    filter_per_file_ignored_diagnostics(&mut diagnostics, settings, source_path);

    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.start.offset, diagnostic.span.end.offset));

    diagnostics
}

/// Lints an existing parse result while preserving parse-aware diagnostics.
#[allow(clippy::too_many_arguments)]
pub fn lint_file_at_path_with_resolver_and_parse_result(
    parse_result: &ParseResult,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<Diagnostic> {
    let directives = parse_directives(source, indexer.comment_index(), shellcheck_map);
    lint_file_at_path_with_resolver_and_parse_result_and_directives(
        parse_result,
        source,
        indexer,
        settings,
        &directives,
        source_path,
        source_path_resolver,
    )
}

/// Lints an existing parse result while deriving suppressions from parsed directives
/// and semantic-collected command spans.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lint_file_at_path_with_resolver_and_parse_result_and_directives(
    parse_result: &ParseResult,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    directives: &[SuppressionDirective],
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<Diagnostic> {
    let shell = resolve_shell(settings, source, source_path);
    let mut file_entry_contract_collector =
        ambient_contracts::AmbientContractCollector::new(source, source_path, shell);
    let analyzed_paths_fallback =
        source_path.map(|path| FxHashSet::from_iter([path.to_path_buf()]));
    let analyzed_paths = settings
        .analyzed_paths
        .as_deref()
        .or(analyzed_paths_fallback.as_ref());
    let linter_semantic_artifacts = LinterSemanticArtifacts::build_with_options(
        &parse_result.file,
        source,
        indexer,
        SemanticBuildOptions {
            source_path,
            source_path_resolver,
            file_entry_contract: None,
            file_entry_contract_collector: Some(&mut file_entry_contract_collector),
            analyzed_paths,
            shell_profile: Some(shell.shell_profile()),
            resolve_source_closure: settings.resolve_source_closure,
        },
    );
    let suppression_index = build_suppression_index_from_semantic(
        directives,
        &parse_result.file,
        source,
        &linter_semantic_artifacts,
    );
    let checker = Checker::new(
        &parse_result.file,
        source,
        &linter_semantic_artifacts,
        indexer,
        &settings.rules,
        shell,
        settings.ambient_shell_options,
        settings.report_environment_style_names,
        settings.rule_options.clone(),
        suppression_index.as_ref(),
        parse_error_position(parse_result),
    );
    let mut diagnostics = checker.check();

    diagnostics.extend(parse_diagnostics::collect_parse_rule_diagnostics(
        &parse_result.file,
        source,
        Some(parse_result),
        &linter_semantic_artifacts,
        &settings.rules,
        shell,
    ));
    if parse_result.is_err() {
        sanitize_diagnostic_spans_cold(&mut diagnostics, source, indexer);
    }

    for diagnostic in &mut diagnostics {
        if let Some(&severity) = settings.severity_overrides.get(&diagnostic.rule) {
            diagnostic.severity = severity;
        }
    }

    if let Some(suppression_index) = suppression_index.as_ref() {
        filter_suppressed_diagnostics(&mut diagnostics, indexer, suppression_index);
    }
    filter_per_file_ignored_diagnostics(&mut diagnostics, settings, source_path);

    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.start.offset, diagnostic.span.end.offset));

    diagnostics
}

/// Lints an existing parse result while parsing suppression directives inside the linter.
///
/// This keeps directive attachment internal while we migrate it to semantic command visits.
#[allow(clippy::too_many_arguments)]
pub fn lint_file_at_path_with_resolver_and_parse_result_with_comment_directives(
    parse_result: &ParseResult,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<Diagnostic> {
    let directives = parse_directives(source, indexer.comment_index(), shellcheck_map);
    lint_file_at_path_with_resolver_and_parse_result_and_directives(
        parse_result,
        source,
        indexer,
        settings,
        &directives,
        source_path,
        source_path_resolver,
    )
}

/// Lints an existing parse result located at an optional source path.
pub fn lint_file(
    parse_result: &ParseResult,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    source_path: Option<&Path>,
) -> Vec<Diagnostic> {
    lint_file_at_path_with_resolver_and_parse_result(
        parse_result,
        source,
        indexer,
        settings,
        shellcheck_map,
        source_path,
        None,
    )
}

/// Lints an existing parse result while deriving suppressions from parsed directives
/// and semantic-collected command spans.
pub(crate) fn lint_file_with_directives(
    parse_result: &ParseResult,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    directives: &[SuppressionDirective],
    source_path: Option<&Path>,
) -> Vec<Diagnostic> {
    lint_file_at_path_with_resolver_and_parse_result_and_directives(
        parse_result,
        source,
        indexer,
        settings,
        directives,
        source_path,
        None,
    )
}

/// Lints an existing parse result while parsing suppression directives inside the linter.
pub fn lint_file_with_comment_directives(
    parse_result: &ParseResult,
    source: &str,
    indexer: &Indexer,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    source_path: Option<&Path>,
) -> Vec<Diagnostic> {
    lint_file_at_path_with_resolver_and_parse_result_with_comment_directives(
        parse_result,
        source,
        indexer,
        settings,
        shellcheck_map,
        source_path,
        None,
    )
}

fn filter_suppressed_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    indexer: &Indexer,
    suppression_index: &SuppressionIndex,
) {
    diagnostics.retain(|diagnostic| {
        let line = indexer
            .line_index()
            .line_number(TextSize::new(diagnostic.span.start.offset as u32));
        let Ok(line) = u32::try_from(line) else {
            return true;
        };

        !suppression_index.is_suppressed(diagnostic.rule, line)
    });
}

fn filter_per_file_ignored_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    settings: &LinterSettings,
    source_path: Option<&Path>,
) {
    let ignored_rules = settings.per_file_ignored_rules(source_path);
    if ignored_rules.is_empty() {
        return;
    }

    diagnostics.retain(|diagnostic| !ignored_rules.contains(diagnostic.rule));
}

#[cold]
#[inline(never)]
fn sanitize_diagnostic_spans_cold(diagnostics: &mut [Diagnostic], source: &str, indexer: &Indexer) {
    for diagnostic in diagnostics {
        diagnostic.span = sanitize_span(diagnostic.span, source, indexer.line_index());
    }
}

#[cold]
fn sanitize_span(span: Span, source: &str, line_index: &LineIndex) -> Span {
    if span.start.offset <= span.end.offset
        && span.end.offset <= source.len()
        && source.is_char_boundary(span.start.offset)
        && source.is_char_boundary(span.end.offset)
    {
        return span;
    }

    let offsets_are_bounded = span.start.offset <= source.len() && span.end.offset <= source.len();
    let offsets_are_aligned =
        source.is_char_boundary(span.start.offset) && source.is_char_boundary(span.end.offset);
    if offsets_are_bounded && offsets_are_aligned && span.start.offset > span.end.offset {
        return Span::from_positions(span.end, span.start);
    }

    let len = source.len();
    let raw_start = span.start.offset.min(len);
    let raw_end = span.end.offset.min(len);
    let (start_offset, end_offset) = if raw_start <= raw_end {
        (
            floor_char_boundary(source, raw_start),
            ceil_char_boundary(source, raw_end),
        )
    } else {
        (
            floor_char_boundary(source, raw_end),
            ceil_char_boundary(source, raw_start),
        )
    };

    Span::from_positions(
        position_at_offset(source, line_index, start_offset),
        position_at_offset(source, line_index, end_offset),
    )
}

#[cold]
fn position_at_offset(source: &str, line_index: &LineIndex, target_offset: usize) -> Position {
    let line = line_index.line_number(TextSize::new(target_offset as u32));
    let line_start = line_index
        .line_start(line)
        .map(usize::from)
        .unwrap_or_default();

    Position {
        line,
        column: source[line_start..target_offset].chars().count() + 1,
        offset: target_offset,
    }
}

#[cold]
fn floor_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cold]
fn ceil_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset < source.len() && !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_ast::{Command, Position, Span, StmtSeq, WordPart, WordPartNode};
    use shuck_parser::Error as ParseError;
    use shuck_parser::parser::{
        ParseDiagnostic, ParseStatus, Parser, ShellDialect as ParseDialect, SyntaxFacts,
    };
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn lint(source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        lint_file_with_directives(&output, source, &indexer, settings, &directives, None)
    }

    fn lint_path(path: &Path, settings: &LinterSettings) -> Vec<Diagnostic> {
        let source = fs::read_to_string(path).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let directives = parse_directives(
            &source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        lint_file_with_directives(
            &output,
            &source,
            &indexer,
            settings,
            &directives,
            Some(path),
        )
    }

    fn lint_for_rule(source: &str, rule: Rule) -> Vec<Diagnostic> {
        lint(source, &LinterSettings::for_rule(rule))
    }

    fn lint_path_for_rule(path: &Path, rule: Rule) -> Vec<Diagnostic> {
        lint_path(path, &LinterSettings::for_rule(rule))
    }

    fn lint_path_for_rule_with_resolver(
        path: &Path,
        rule: Rule,
        source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    ) -> Vec<Diagnostic> {
        let source = fs::read_to_string(path).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let directives = parse_directives(
            &source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        lint_file_at_path_with_resolver_and_parse_result_and_directives(
            &output,
            &source,
            &indexer,
            &LinterSettings::for_rule(rule),
            &directives,
            Some(path),
            source_path_resolver,
        )
    }

    fn lint_named_source(path: &Path, source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        lint_file_with_directives(&output, source, &indexer, settings, &directives, Some(path))
    }

    fn lint_named_source_with_parse_dialect(
        path: &Path,
        source: &str,
        parse_dialect: ParseDialect,
        settings: &LinterSettings,
    ) -> Vec<Diagnostic> {
        let output = Parser::with_dialect(source, parse_dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        lint_file_with_directives(&output, source, &indexer, settings, &directives, Some(path))
    }

    fn runtime_prelude_source(shebang: &str) -> String {
        format!(
            "{shebang}\nprintf '%s\\n' \"$IFS\" \"$USER\" \"$HOME\" \"$SHELL\" \"$PWD\" \"$TERM\" \"$PATH\" \"$CDPATH\" \"$LANG\" \"$LC_ALL\" \"$LC_TIME\" \"$SUDO_USER\" \"$DOAS_USER\"\nprintf '%s\\n' \"$LINENO\" \"$FUNCNAME\" \"${{BASH_SOURCE[0]}}\" \"${{BASH_LINENO[0]}}\" \"$RANDOM\" \"${{BASH_REMATCH[0]}}\" \"$READLINE_LINE\" \"$BASH_VERSION\" \"${{BASH_VERSINFO[0]}}\" \"$OSTYPE\" \"$HISTCONTROL\" \"$HISTSIZE\"\n"
        )
    }

    fn first_command_substitution_body(parts: &[WordPartNode]) -> Option<&StmtSeq> {
        for part in parts {
            match &part.kind {
                WordPart::CommandSubstitution { body, .. }
                | WordPart::ProcessSubstitution { body, .. } => return Some(body),
                WordPart::DoubleQuoted { parts, .. } => {
                    if let Some(body) = first_command_substitution_body(parts) {
                        return Some(body);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn simple_command_names(visits: Vec<facts::CommandVisit<'_>>, source: &str) -> Vec<String> {
        visits
            .into_iter()
            .filter_map(|visit| {
                let Command::Simple(command) = visit.command else {
                    return None;
                };
                let span = command.name.span;
                Some(source[span.start.offset..span.end.offset].to_owned())
            })
            .collect()
    }

    #[test]
    fn linter_semantic_artifacts_iterate_body_commands_without_deeper_nested_words() {
        let source = "echo \"$(if probe; then printf \"$(inner)\"; fi)\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let Command::Simple(command) = &output.file.body.stmts[0].command else {
            panic!("expected simple command");
        };
        let body = first_command_substitution_body(&command.args[0].parts)
            .expect("expected command substitution body");

        let same_body_names =
            simple_command_names(semantic.command_visits_in_body(body, false), source);
        let nested_names =
            simple_command_names(semantic.command_visits_in_body(body, true), source);

        assert_eq!(same_body_names, vec!["probe", "printf"]);
        assert_eq!(nested_names, vec!["probe", "printf", "inner"]);
    }

    #[test]
    fn linter_semantic_artifacts_keep_function_body_nested_depths() {
        let source = "echo \"$(f() { printf \"$(inner)\"; }; f)\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let Command::Simple(command) = &output.file.body.stmts[0].command else {
            panic!("expected simple command");
        };
        let body = first_command_substitution_body(&command.args[0].parts)
            .expect("expected command substitution body");

        let same_body_names =
            simple_command_names(semantic.command_visits_in_body(body, false), source);
        let nested_names =
            simple_command_names(semantic.command_visits_in_body(body, true), source);

        assert_eq!(same_body_names, vec!["printf", "f"]);
        assert_eq!(nested_names, vec!["printf", "inner", "f"]);
    }

    #[test]
    fn default_settings_run_without_emitting_noop_diagnostics() {
        let diagnostics = lint("#!/bin/bash\necho ok\n", &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn lint_file_preserves_parse_rule_diagnostics() {
        let source = "#!/bin/sh\n{ :; } always { :; }\n";
        let parse_result = Parser::new(source).parse();
        let indexer = Indexer::new(source, &parse_result);
        let diagnostics = lint_file(
            &parse_result,
            source,
            &indexer,
            &LinterSettings::for_rule(Rule::ZshAlwaysBlock),
            &ShellCheckCodeMap::default(),
            None,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshAlwaysBlock);
        assert_eq!(diagnostics[0].span.slice(source), "always");
    }

    #[test]
    fn analyze_file_returns_semantic_model_and_diagnostics() {
        let source = "#!/bin/bash\nvalue=ok\necho \"$value\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let result = analyze_file(
            &output.file,
            source,
            &indexer,
            &LinterSettings::default(),
            None,
        );

        assert!(result.diagnostics.is_empty());
        assert!(!result.semantic.scopes().is_empty());
        assert!(!result.semantic.bindings().is_empty());
    }

    #[test]
    fn parse_error_position_falls_back_to_first_diagnostic_span() {
        let file = Parser::new("#!/bin/bash\n").parse().unwrap().file;
        let diagnostic_start = Position {
            line: 3,
            column: 2,
            offset: 14,
        };
        let parse_result = ParseResult {
            file,
            diagnostics: vec![ParseDiagnostic {
                message: "expected command".to_owned(),
                span: Span::at(diagnostic_start),
            }],
            status: ParseStatus::Recovered,
            terminal_error: Some(ParseError::parse("expected command")),
            syntax_facts: SyntaxFacts::default(),
        };

        assert_eq!(parse_error_position(&parse_result), Some((3, 2)));
    }

    #[test]
    fn empty_rule_set_is_a_noop() {
        let diagnostics = lint(
            "#!/bin/bash\necho ok\n",
            &LinterSettings {
                rules: RuleSet::EMPTY,
                ..LinterSettings::default()
            },
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn shell_inference_uses_path_when_shebang_is_missing() {
        let source = "local value=ok\n";
        let diagnostics = lint_named_source(
            Path::new("/tmp/example.bash"),
            source,
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalTopLevel);
    }

    #[test]
    fn project_specific_paths_do_not_suppress_undefined_variables() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/void-packages/common/build-style/void-cross.sh"),
            "\
build() {
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$configure_args\" \"$wrksrc\"
}
build
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("configure_args"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("wrksrc"))
        );
    }

    #[test]
    fn flattened_corpus_paths_do_not_suppress_undefined_variables() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/scripts/void-linux__void-packages__common__build-style__void-cross.sh"),
            "\
build() {
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$configure_args\" \"$wrksrc\"
}
build
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("configure_args"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("wrksrc"))
        );
    }

    #[test]
    fn sourced_theme_contract_does_not_suppress_runtime_color_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-it/themes/minimal/minimal.theme.bash"),
            "\
prompt_command() {
  PS1=\"$green $reset_color\"
}
PROMPT_COMMAND=prompt_command
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("green"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("reset_color"))
        );
    }

    #[test]
    fn generic_theme_directory_does_not_suppress_palette_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/themes/minimal.theme.bash"),
            "\
render_prompt() {
  printf '%s\\n' \"$green\" \"$reset_color\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("green"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("reset_color"))
        );
    }

    #[test]
    fn generic_completion_directory_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/completions/example.sh"),
            "\
complete_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn generic_completion_directory_with_compreply_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/completions/example.sh"),
            "\
complete_example() {
  COMPREPLY=()
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn bash_completion_directory_without_initializer_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
complete_example() {
  COMPREPLY=()
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn bash_completion_directory_with_initializer_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
complete_example() {
  _init_completion || return
  printf '%s\\n' \"$cur\" \"$cword\" \"$comp_args\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("comp_args"))
        );
    }

    #[test]
    fn bash_completion_directory_with_commented_initializer_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
# TODO: call _init_completion later
complete_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn bash_completion_directory_with_wrapper_identifier_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
complete_example() {
  my_init_completion_wrapper || return
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn bash_completion_directory_with_initializer_definition_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
_init_completion() {
  :
}
complete_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn bash_completion_directory_with_separator_comment_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
noop;# _init_completion later
complete_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn bash_completion_directory_with_heredoc_initializer_does_not_suppress_helper_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/bash-completion/completions/example.bash"),
            "\
cat <<EOF
_init_completion
EOF
complete_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cur"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cword"))
        );
    }

    #[test]
    fn sourced_runtime_contract_does_not_mark_arbitrary_assignments_used() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/rvm/scripts/cleanup"),
            "\
rvm_base_except=\"selector\"
cleanup() { :; }
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
    }

    #[test]
    fn sourced_module_contract_does_not_suppress_arbitrary_runtime_state_reads() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/LinuxGSM/lgsm/modules/command_backup.sh"),
            "\
commandname=\"BACKUP\"
backup_run() {
  printf '%s\\n' \"$lockdir\" \"$commandname\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
    }

    #[test]
    fn prefix_name_expansions_do_not_trigger_c006() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/plain.sh"),
            "unset \"${!completion_prefix@}\"\n",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn project_closure_context_without_a_provider_still_reports_c006() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/scripts/helper.sh"),
            "\
# shellcheck source=helpers.sh
. ./helpers.sh
printf '%s\\n' \"$pkgname\"
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
    }

    #[test]
    fn void_packages_paths_without_required_source_anchors_still_report_c006() {
        let xbps_src_diagnostics = lint_named_source(
            Path::new("/tmp/void-packages/common/xbps-src/shutils/common.sh"),
            "printf '%s\\n' \"$build_style\"\n",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert_eq!(xbps_src_diagnostics.len(), 1);
        assert_eq!(xbps_src_diagnostics[0].rule, Rule::UndefinedVariable);

        let pycompile_diagnostics = lint_named_source(
            Path::new("/tmp/void-packages/srcpkgs/xbps-triggers/files/pycompile"),
            "printf '%s\\n' \"$pycompile_version\"\n",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert_eq!(pycompile_diagnostics.len(), 1);
        assert_eq!(pycompile_diagnostics[0].rule, Rule::UndefinedVariable);
    }

    #[test]
    fn project_closure_function_contract_suppresses_c006_when_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
set_flag
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
set_flag() {
  flag=1
}
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_zsh_helper_imports_bindings_after_zsh_only_syntax() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.zsh");
        let helper = temp.path().join("helper.zsh");
        fs::write(
            &main,
            "\
#!/bin/zsh
. ./helper.zsh
print \"$helper_value\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
#!/bin/zsh
repeat 1; do print loaded; done
helper_value=ready
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    fn obscure_zsh_linter_stress_source() -> &'static str {
        r#"#!/bin/zsh
() {
  emulate -L zsh
  setopt extendedglob
  local -a matches=(src/**/*.zsh(.N:t:r))
  print -r -- ${(j:,:)matches}
} one two

{
  if [[ -n ${commands[zsh]} ]] {
    print -r -- ok
  } elif (( ${+commands[false]} )) {
    print -r -- maybe
  } else {
    print -r -- missing
  }
} always {
  print -r -- cleanup
}

repeat 2 print -r -- tick

foreach item (alpha beta gamma) {
  print -r -- ${item:u}
}

typeset -A colors=(
  [normal]=black
  [warning]=yellow
  [error]=red
)
print -r -- ${colors[(i)warn*]} ${colors[(r)r*]}
colors[(I)e*]=brightred

local target=/tmp/archive.tar.gz
print -r -- ${${target:t}:r} ${${:-$target}:h}
print -r -- ${(Q)${:-one\ two}} ${(%)${:-%n@%m}}

local -a words=(one two three)
print -r -- ${(j:|:)words}
print -r -- ${(qqq)${(F)words}}

print -r -- **/*.zsh(.DN:t:r)
print -r -- *(^@N) *(.om[1,3])

diff =(print -r -- left) <(print -r -- right)
cat > >(sed 's/^/[out] /') <<< ${:-payload}

print -r -- message >out.log >audit.log
print -r -- quiet &>/dev/null &|

case $OSTYPE in
  (darwin|freebsd)<->)
    print -r -- bsd ;|
  linux(|-gnu))
    print -r -- linux ;&
  *)
    print -r -- other ;;
esac

if [[ $file == (#b)(*/)([^/]##).(zsh|plugin)(#e) ]]; then
  print -r -- $match[1] $match[2]
fi

zparseopts -D -E -F -- \
  h=help -help=help \
  v+:=verbose -verbose+:=verbose \
  o:=output -output:=output

noglob command print -r -- **/*(N)
whence -m 'z*' >/dev/null

PS1=$'%F{green}%n%f:%~ %# '
local rendered=${(%)PS1}
print -r -- $rendered

integer count=${#path}
(( count += ${+commands[git]} ? path[(I)*bin*] : 0 ))
print -r -- $count

coproc {
  print -r -- request
  read -r reply
  print -r -- $reply
}
"#
    }

    #[test]
    fn default_linter_handles_obscure_zsh_syntax_without_parse_rule_diagnostics() {
        let source = obscure_zsh_linter_stress_source();
        let path = Path::new("stress.zsh");
        let diagnostics =
            crate::test::test_snippet_at_path(path, source, &LinterSettings::default());

        let parse_rules = [
            Rule::MissingFi,
            Rule::IfMissingThen,
            Rule::LoopWithoutEnd,
            Rule::MissingDoneInForLoop,
            Rule::DanglingElse,
            Rule::UntilMissingDo,
            Rule::IfBracketGlued,
            Rule::CPrototypeFragment,
        ];
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !parse_rules.contains(&diagnostic.rule)),
            "unexpected parse-shaped diagnostics: {diagnostics:#?}"
        );
    }

    #[test]
    fn native_zsh_portability_rules_ignore_obscure_zsh_syntax() {
        let source = obscure_zsh_linter_stress_source();
        let path = Path::new("stress.zsh");
        let settings = LinterSettings::for_rules([
            Rule::ZshRedirPipe,
            Rule::ZshBraceIf,
            Rule::ZshAlwaysBlock,
            Rule::ZshFlagExpansion,
            Rule::NestedZshSubstitution,
            Rule::ZshNestedExpansion,
            Rule::ZshPromptBracket,
            Rule::ZshAssignmentToZero,
            Rule::ZshParameterFlag,
            Rule::ZshArraySubscriptInCase,
            Rule::ZshParameterIndexFlag,
            Rule::MultiVarForLoop,
            Rule::ProcessSubstitution,
            Rule::HereString,
            Rule::Coproc,
            Rule::ArrayAssignment,
            Rule::ArrayReference,
        ])
        .with_shell(ShellDialect::Zsh);
        let diagnostics = crate::test::test_snippet_at_path(path, source, &settings);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:#?}");
    }

    #[test]
    fn sourced_zsh_helper_imports_bindings_after_obscure_native_syntax() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.zsh");
        let helper = temp.path().join("helper.zsh");
        fs::write(
            &main,
            "\
#!/bin/zsh
. ./helper.zsh
print -r -- \"$helper_value\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            r#"#!/bin/zsh
() {
  emulate -L zsh
  local -a matches=(src/**/*.zsh(.N:t:r))
  print -r -- ${(j:,:)matches}
}

{
  if [[ -n ${commands[zsh]} ]] {
    print -r -- ok
  } else {
    print -r -- missing
  }
} always {
  print -r -- cleanup
}

typeset -A colors=(
  [normal]=black
  [warning]=yellow
  [error]=red
)
print -r -- ${colors[(i)warn*]} ${colors[(r)r*]}

foreach item (alpha beta gamma) {
  print -r -- ${item:u}
}

print -r -- **/*.zsh(.DN:t:r)
print -r -- *(^@N) *(.om[1,3])
print -r -- quiet &>/dev/null &|

case $OSTYPE in
  (darwin|freebsd)<->) print -r -- bsd ;|
  linux(|-gnu)) print -r -- linux ;&
  *) print -r -- other ;;
esac

helper_value=ready
"#,
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_env_split_bash_helper_prefers_shebang_over_sh_extension() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$helper_value\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
#!/usr/bin/env -S bash -e
for ((i=0; i<1; i++)); do :; done
helper_value=ready
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_env_split_bash_helper_normalizes_shebang_path() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$helper_value\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
#!/usr/bin/env -S /bin/bash -e
for ((i=0; i<1; i++)); do :; done
helper_value=ready
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_env_split_bash_helper_skips_env_assignments() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$helper_value\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
#!/usr/bin/env -S FOO=1 bash -e
for ((i=0; i<1; i++)); do :; done
helper_value=ready
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn sourced_helper_reads_keep_c150_live_for_subshell_assignments() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
(flag=1)
. ./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "printf '%s\\n' \"$flag\"\n").unwrap();

        let source = fs::read_to_string(&main).unwrap();
        let diagnostics = lint_path_for_rule(&main, Rule::SubshellLocalAssignment);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(&source))
                .collect::<Vec<_>>(),
            vec!["flag"]
        );
    }

    #[test]
    fn sourced_helper_reads_ignore_subshell_writes_after_same_command_resets() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
(flag=1)
flag=2 . ./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "printf '%s\\n' \"$flag\"\n").unwrap();

        let local_assignment_diagnostics = lint_path_for_rule(&main, Rule::SubshellLocalAssignment);
        assert!(
            local_assignment_diagnostics.is_empty(),
            "diagnostics: {local_assignment_diagnostics:?}"
        );

        let side_effect_diagnostics = lint_path_for_rule(&main, Rule::SubshellSideEffect);
        assert!(
            side_effect_diagnostics.is_empty(),
            "diagnostics: {side_effect_diagnostics:?}"
        );
    }

    #[test]
    fn quoted_heredoc_generated_shell_text_does_not_report_c006() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
build=\"$(command cat <<\\END
printf '%s\\n' \"$workdir\"
END
)\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn escaped_dollar_heredoc_generated_text_does_not_report_c006() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
cat <<EOF
\\${devtype} \\${devnum}
EOF
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn quoted_heredoc_generated_shell_text_does_not_report_c006_with_source_closure() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn posix_quoted_heredoc_generated_shell_text_does_not_report_c006_with_source_closure() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
";
        fs::write(&main, source).unwrap();

        let diagnostics = lint_named_source_with_parse_dialect(
            &main,
            source,
            ParseDialect::Posix,
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn posix_second_quoted_heredoc_generated_shell_text_does_not_report_c006_with_source_closure() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/sh
usage=\"$(command cat <<\\END
Usage
END
)\"

build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
";
        fs::write(&main, source).unwrap();

        let diagnostics = lint_named_source_with_parse_dialect(
            &main,
            source,
            ParseDialect::Posix,
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn escaped_dollar_heredoc_generated_text_does_not_report_c006_with_source_closure() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
cat <<EOF > ./postinst
if [ \"\\$1\" = \"configure\" ]; then
  for ver in 1 current; do
    for x in rewriteSystem rewriteURI; do
      xmlcatalog --noout --add \\$x http://example.test/xsl/\\$ver
    done
  done
fi
EOF
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn quoted_heredoc_generated_shell_text_with_nested_same_name_heredoc_does_not_report_c006() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
command cat >&2 <<-END
\tSUCCESS
\tEND
END
)\"
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn tab_stripped_escaped_dollar_heredoc_generated_text_does_not_report_c006() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
cat <<- EOF > ./postinst
\tif [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in 1 current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://example.test/xsl/\\$ver
\t\t\tdone
\t\tdone
\tfi
\tEOF
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UndefinedVariable);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn posix_tab_stripped_escaped_dollar_heredoc_generated_text_does_not_report_c006() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/sh
cat <<- EOF > ./postinst
\tif [ \"$TERMUX_PACKAGE_FORMAT\" = \"pacman\" ] || [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in $TERMUX_PKG_VERSION current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl-ns/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\t\t\tdone
\t\tdone
\tfi
\tEOF
";
        fs::write(&main, source).unwrap();

        let diagnostics = lint_named_source_with_parse_dialect(
            &main,
            source,
            ParseDialect::Posix,
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn posix_docbook_wrapper_does_not_report_c006_for_escaped_placeholders() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let source = "\
#!/bin/sh
termux_step_create_debscripts() {
\tcat <<- EOF > ./postinst
\t#!$TERMUX_PREFIX/bin/sh
\tif [ \"$TERMUX_PACKAGE_FORMAT\" = \"pacman\" ] || [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in $TERMUX_PKG_VERSION current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://cdn.docbook.org/release/xsl/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl-ns/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-${TERMUX_PKG_VERSION}-nons\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\t\t\tdone
\t\tdone
\tfi
\tEOF
}
";
        fs::write(&main, source).unwrap();

        let diagnostics = lint_named_source_with_parse_dialect(
            &main,
            source,
            ParseDialect::Posix,
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn bash_quoted_heredoc_case_arm_text_does_not_report_c006() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
build=\"$(command cat <<\\END
case \"$gitstatus_kernel\" in
  linux)
    for formula in libiconv cmake git wget; do
      if command brew ls --version \"$formula\" >/dev/null; then
        command brew upgrade \"$formula\"
      else
        command brew install \"$formula\"
      fi
    done
  ;;
esac
command cat >&2 <<-END
\tSUCCESS
\tEND
END
)\"
",
        )
        .unwrap();

        let diagnostics = lint_named_source_with_parse_dialect(
            &main,
            &fs::read_to_string(&main).unwrap(),
            ParseDialect::Bash,
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn recovered_lint_diagnostics_keep_valid_spans_for_fuzz_regression() {
        let source = concat!(
            "$~h) echo help; exit 0 ;;\n",
            "  esac\n",
            "done\n",
            "\n",
            "# Should not trigger: one arm can handle m   a) alg=le are correlated.\n",
            "while g$OPTARG ;;\n",
            "    d) domain=$eultiple options.\n",
            "while ge}opts ':ab' opt; do\n",
            "  case \"$opt\" in\n",
            "   | ) ba: ;;\n",
            "  esac\n",
            "doneJ\n",
            "# Shou#!/bin/sh\n",
            "\n",
            "# Should trigger: getopts declares -o, but the matching case never handles itld not trigger: only cases over the getopts variab.\n",
            "while getopts ':a:d:o:h' OPT; do\n",
            "  case \"$OPT\" in\n",
            "    a) alg=le are correlated.\n",
            "while g$OPTARG ;;\n",
            "    d) domain=$etoptA "
        );
        let cases = [
            (Some(Path::new("fuzz.sh")), ParseDialect::Posix),
            (Some(Path::new("fuzz.bash")), ParseDialect::Bash),
            (Some(Path::new("fuzz.mksh")), ParseDialect::Mksh),
            (Some(Path::new("fuzz.zsh")), ParseDialect::Zsh),
            (None, ParseDialect::Posix),
            (None, ParseDialect::Bash),
            (None, ParseDialect::Mksh),
            (None, ParseDialect::Zsh),
        ];

        for (path, dialect) in cases {
            let parse_result = Parser::with_dialect(source, dialect).parse();
            let indexer = Indexer::new(source, &parse_result);
            let diagnostics = lint_file(
                &parse_result,
                source,
                &indexer,
                &LinterSettings::default(),
                &ShellCheckCodeMap::default(),
                path,
            );

            for diagnostic in diagnostics {
                assert!(
                    diagnostic.span.start.offset <= diagnostic.span.end.offset,
                    "invalid span ordering for {} with path {:?} and dialect {:?}: {:?}",
                    diagnostic.code(),
                    path,
                    dialect,
                    diagnostic.span
                );
                assert!(
                    diagnostic.span.end.offset <= source.len(),
                    "span end out of bounds for {} with path {:?} and dialect {:?}: {:?}",
                    diagnostic.code(),
                    path,
                    dialect,
                    diagnostic.span
                );
                assert!(
                    source.is_char_boundary(diagnostic.span.start.offset),
                    "span start not on char boundary for {} with path {:?} and dialect {:?}: {:?}",
                    diagnostic.code(),
                    path,
                    dialect,
                    diagnostic.span
                );
                assert!(
                    source.is_char_boundary(diagnostic.span.end.offset),
                    "span end not on char boundary for {} with path {:?} and dialect {:?}: {:?}",
                    diagnostic.code(),
                    path,
                    dialect,
                    diagnostic.span
                );
            }
        }
    }

    #[test]
    fn helper_library_functions_still_report_c006_without_calls() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/lib/helper.sh"),
            "\
helper() {
  printf '%s\\n' \"$flag\"
}
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
    }

    #[test]
    fn helper_library_functions_still_report_c006_when_called() {
        let diagnostics = lint_named_source(
            Path::new("/tmp/project/lib/helper.sh"),
            "\
helper() {
  printf '%s\\n' \"$flag\"
}
helper
",
            &LinterSettings::for_rule(Rule::UndefinedVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
    }

    #[test]
    fn post_hoc_filtering_removes_only_suppressed_diagnostics() {
        let source = "\
echo ok
# shellcheck disable=SC2086
echo $foo
echo $bar
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let suppressions =
            build_suppression_index_from_semantic(&directives, &output.file, source, &semantic)
                .unwrap();

        let echo_foo = match &output.file.body[1].command {
            Command::Simple(command) => command.span,
            other => {
                debug_assert!(false, "expected simple command, got {other:?}");
                return;
            }
        };
        let echo_bar = match &output.file.body[2].command {
            Command::Simple(command) => command.span,
            other => {
                debug_assert!(false, "expected simple command, got {other:?}");
                return;
            }
        };

        let mut diagnostics = vec![
            Diagnostic {
                rule: Rule::UnquotedExpansion,
                message: "first".to_owned(),
                severity: Rule::UnquotedExpansion.default_severity(),
                span: echo_foo,
                fix: None,
                fix_title: None,
            },
            Diagnostic {
                rule: Rule::UnquotedExpansion,
                message: "second".to_owned(),
                severity: Rule::UnquotedExpansion.default_severity(),
                span: echo_bar,
                fix: None,
                fix_title: None,
            },
        ];

        filter_suppressed_diagnostics(&mut diagnostics, &indexer, &suppressions);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "second");
    }

    #[test]
    fn shellcheck_disable_inside_function_suppresses_heredoc_body_diagnostics() {
        let source = "\
#!/bin/bash
echo ready
emit_file() {
  # shellcheck disable=SC2154
  cat \"$path\" <<EOF
value=$body_value
other=${other_value}
EOF
  echo \"$later\"
}
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$later"]
        );
    }

    #[test]
    fn undefined_variable_reports_escaped_ps4_prompt_reference_at_assignment() {
        let source = "\
#!/bin/bash
export PS4=\"+ \\${BASH_SOURCE##\\${rvm_path:-}} > \"
p=\"$rvm_path\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["PS4"]
        );
    }

    #[test]
    fn undefined_variable_reports_trap_action_references_at_action_word() {
        let source = "\
#!/bin/sh
tmpdir=/tmp/example
trap 'ret=$?; rmdir \"$tmpdir/d\" \"$tmpdir\" 2>/dev/null; exit $ret' 0
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'ret=$?; rmdir \"$tmpdir/d\" \"$tmpdir\" 2>/dev/null; exit $ret'"]
        );
    }

    #[test]
    fn unused_assignment_flags_unread_variable() {
        let source = "#!/bin/sh\nfoo=1\n";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("foo"));
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn unused_assignment_flags_indirect_only_target_by_default() {
        let source = "\
#!/bin/bash
target=ok
name=target
printf '%s\\n' \"${!name}\"
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "target");
    }

    #[test]
    fn unused_assignment_can_keep_indirect_only_target_live_with_rule_option() {
        let diagnostics = lint(
            "\
#!/bin/bash
target=ok
name=target
printf '%s\\n' \"${!name}\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment)
                .with_c001_treat_indirect_expansion_targets_as_used(true),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_reports_declaration_only_targets_by_default() {
        let source = "\
#!/bin/bash
f(){
  local cur
  declare words
}
f
";
        let diagnostics = lint(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "cur");
        assert_eq!(diagnostics[1].span.slice(source), "words");
    }

    #[test]
    fn unused_assignment_keeps_dynamic_target_arrays_live() {
        let diagnostics = lint(
            "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_ignores_plain_underscore_bindings() {
        let diagnostics = lint_for_rule("#!/bin/bash\n_=1\n", Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_ignores_leading_underscore_bindings() {
        let diagnostics = lint_for_rule(
            "#!/bin/bash\n_unused=1\n__unused=2\n",
            Rule::UnusedAssignment,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_reports_plain_rest_bindings() {
        let diagnostics = lint_for_rule("#!/bin/bash\nrest=1\nREST=2\n", Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn unused_assignment_ignores_underscore_read_targets() {
        let diagnostics = lint(
            "\
#!/bin/bash
printf 'x y\n' | while read -r _ value; do
  printf '%s\n' \"$value\"
done
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_reports_read_target_name_span() {
        let source = "#!/bin/sh\nread -r foo\n";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn unused_assignment_reports_getopts_target_name_span() {
        let source = "\
#!/bin/sh
while getopts \"ab\" opt; do
  :
done
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "opt");
    }

    #[test]
    fn read_header_bindings_used_in_loop_body_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
printf '%s\n' 'service safe ok yes' | while read UNIT EXPOSURE PREDICATE HAPPY; do
  printf '%s %s %s %s\n' \"$UNIT\" \"$EXPOSURE\" \"$PREDICATE\" \"$HAPPY\"
done
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn command_prefix_environment_assignment_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
CFLAGS=\"$SLKCFLAGS\" make
DESTDIR=\"$pkgdir\" install
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn indirect_expansion_keeps_dynamic_target_arrays_live() {
        let diagnostics = lint(
            "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn array_append_used_by_later_expansion_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
arr=(--first)
arr+=(--second)
printf '%s\\n' \"${arr[@]}\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn assignments_used_in_process_substitution_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local opts
  case \"$1\" in
    a) opts=alpha ;;
    *) opts=beta ;;
  esac
  while IFS= read -r line; do :; done < <(printf '%s\\n' \"$opts\")
}
f a
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 8);
        assert_eq!(diagnostics[0].span.slice(
            "#!/bin/bash\nf() {\n  local opts\n  case \"$1\" in\n    a) opts=alpha ;;\n    *) opts=beta ;;\n  esac\n  while IFS= read -r line; do :; done < <(printf '%s\\n' \"$opts\")\n}\nf a\n"
        ), "line");
    }

    #[test]
    fn overwritten_empty_initializers_only_report_the_later_dead_assignment() {
        let source = "\
#!/bin/bash
f() {
  local foo=
  foo=bar
}
f
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn substring_offset_arithmetic_reads_do_not_trigger_unused_assignment() {
        let diagnostics = lint(
            "\
#!/bin/bash
spinner() {
  local chars=\"/-\\\\|\"
  local spin_i=0
  while true; do
    printf '%s\\n' \"${chars:spin_i++%${#chars}:1}\"
  done
}
spinner
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn self_referential_assignments_are_not_flagged_unused() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
foo=\"$foo\"
bar=\"${bar:-fallback}\"
",
            Rule::UnusedAssignment,
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_default_operand_followed_by_later_expansion_keeps_assignment_live() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
foo=bar
default=/tmp
cmd=\"${home:-\"${default}\"}'${foo}'\"
printf '%s\\n' \"$cmd\"
",
            Rule::UnusedAssignment,
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unused_append_assignment_is_not_flagged() {
        let diagnostics = lint_for_rule("#!/bin/bash\nfoo+=bar\n", Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn later_defined_helper_assignment_to_caller_local_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
main() {
  local status=''
  helper
  printf '%s\\n' \"$status\"
}
helper() {
  status=ok
}
main
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn later_defined_helper_array_append_to_caller_local_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
main() {
  local errors=()
  helper
  printf '%s\\n' \"${errors[@]}\"
}
helper() {
  errors+=(oops)
}
main
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn read_implicitly_consumes_ifs_but_still_flags_unrelated_local() {
        let source = "\
#!/bin/bash
f() {
  local IFS=$'\\n'
  local unused=1
  read -d '' -ra reply < <(printf 'alpha\\nbeta\\0')
  printf '%s\\n' \"${reply[@]}\"
}
f
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn getopts_runtime_state_assignments_are_not_flagged() {
        let source = "\
#!/bin/sh
f() {
  local flag OPTIND=1 OPTARG='' OPTERR=0
  while getopts 'a:' flag; do :; done
}
f
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "flag");
    }

    #[test]
    fn global_ifs_assignment_is_not_flagged_but_unrelated_assignment_is() {
        let source = "\
#!/bin/bash
IFS=$'\\n\\t'
unused=1
echo ok
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn shell_runtime_assignments_are_not_flagged_but_unrelated_assignment_is() {
        let source = "\
#!/bin/sh
PATH=$PATH:/opt/custom
CDPATH=/tmp
LANG=C
LC_ALL=C
LC_TIME=C
unused=1
echo ok
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn special_runtime_assignments_are_not_flagged_but_unrelated_assignment_is() {
        let source = "\
#!/bin/bash
HOME=/tmp/home
SHELL=/bin/bash
TERM=xterm-256color
USER=builder
PWD=/tmp/work
HISTFILE=/tmp/history
HISTFILESIZE=unlimited
HISTIGNORE='ls:bg:fg:history'
HISTSIZE=-1
HISTTIMEFORMAT='%F %T '
COMP_WORDBREAKS=\"${COMP_WORDBREAKS//:/}\"
PROMPT_COMMAND='history -a'
BASH_ENV=/dev/null
BASH_XTRACEFD=9
ENV=/dev/null
INPUTRC=/tmp/inputrc
MAIL=/tmp/mail
OLDPWD=/tmp/old
PROMPT_DIRTRIM=2
SECONDS=0
TIMEFORMAT='%R'
TMOUT=30
PS1='prompt> '
PS2='continuation> '
PS3=''
PS4='+ '
COLUMNS=1
READLINE_POINT=0
unused=1
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn unrelated_array_assignment_is_still_flagged_with_indirect_expansion() {
        let source = "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused_args");
    }

    #[test]
    fn used_variable_produces_no_diagnostic() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\necho \"$foo\"\n",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn parameter_default_operand_usage_is_not_flagged() {
        let source = "\
#!/bin/sh
repo_root=$(pwd)
cache_dir=${1:-\"$repo_root/.cache\"}
printf '%s\\n' \"$cache_dir\"
";
        let diagnostics = lint_named_source_with_parse_dialect(
            Path::new("/tmp/parameter-default.sh"),
            source,
            ParseDialect::Posix,
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_at_script_scope_is_flagged() {
        let diagnostics = lint(
            "#!/bin/bash\nlocal foo=bar\nprintf '%s\\n' \"$foo\"\n",
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalTopLevel);
    }

    #[test]
    fn local_at_script_scope_in_sh_is_not_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nlocal foo=bar\nprintf '%s\\n' \"$foo\"\n",
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_inside_function_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo=bar
  printf '%s\\n' \"$foo\"
}
f
",
            &LinterSettings::for_rule(Rule::LocalTopLevel),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_is_flagged_in_sh_scripts() {
        let diagnostics = lint(
            "\
#!/bin/sh
f() {
  local foo=bar
  printf '%s\\n' \"$foo\"
}
f
",
            &LinterSettings::for_rule(Rule::LocalVariableInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalVariableInSh);
    }

    #[test]
    fn local_in_bash_script_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo=bar
  printf '%s\\n' \"$foo\"
}
f
",
            &LinterSettings::for_rule(Rule::LocalVariableInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_keyword_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nfunction f { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeyword),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FunctionKeyword);
    }

    #[test]
    fn function_keyword_in_dash_is_flagged() {
        let diagnostics = lint(
            "#!/bin/dash\nfunction f { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeyword),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FunctionKeyword);
    }

    #[test]
    fn function_keyword_with_parens_is_not_flagged_by_x004() {
        let diagnostics = lint(
            "#!/bin/sh\nfunction f() { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeyword),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_keyword_in_bash_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "#!/bin/bash\nfunction f { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeyword),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_keyword_with_parens_in_sh_is_flagged_by_x052() {
        let diagnostics = lint(
            "#!/bin/sh\nfunction f() { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FunctionKeywordInSh);
    }

    #[test]
    fn function_keyword_without_parens_is_not_flagged_by_x052() {
        let diagnostics = lint(
            "#!/bin/sh\nfunction f { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_keyword_with_parens_in_dash_is_flagged_by_x052() {
        let diagnostics = lint(
            "#!/bin/dash\nfunction f() { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FunctionKeywordInSh);
    }

    #[test]
    fn function_keyword_with_parens_in_bash_is_not_flagged_by_x052() {
        let diagnostics = lint(
            "#!/bin/bash\nfunction f() { :; }\n",
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn source_inside_function_in_sh_is_not_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/sh\nf() {\n  source ./helpers.sh\n}\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn directive_pinned_source_inside_function_in_sh_is_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/sh\nf() {\n  # shellcheck source=/dev/null\n  source ./helpers.sh\n}\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceInsideFunctionInSh);
    }

    #[test]
    fn directive_pinned_source_inside_function_in_dash_is_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/dash\nf() {\n  # shellcheck source=/dev/null\n  source ./helpers.sh\n}\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceInsideFunctionInSh);
    }

    #[test]
    fn directive_pinned_guarded_source_inside_function_in_sh_is_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/sh\nf() {\n  # shellcheck source=/dev/null\n  [ -r ./helpers.sh ] && source ./helpers.sh\n}\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceInsideFunctionInSh);
    }

    #[test]
    fn directive_pinned_source_inside_function_command_substitution_in_sh_is_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/sh\nf() {\n  version=$(\n    # shellcheck source=/dev/null\n    source ./helpers.sh && echo \"$name\"\n  )\n}\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceInsideFunctionInSh);
    }

    #[test]
    fn top_level_source_is_not_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/sh\nsource ./helpers.sh\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn source_inside_function_in_bash_is_not_flagged_by_x080() {
        let diagnostics = lint(
            "#!/bin/bash\nf() {\n  source ./helpers.sh\n}\n",
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn let_command_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nlet x=1\n",
            &LinterSettings::for_rule(Rule::LetCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LetCommand);
    }

    #[test]
    fn let_command_in_dash_is_flagged() {
        let diagnostics = lint(
            "#!/bin/dash\nlet x=1\n",
            &LinterSettings::for_rule(Rule::LetCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LetCommand);
    }

    #[test]
    fn let_command_in_bash_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "#!/bin/bash\nlet x=1\n",
            &LinterSettings::for_rule(Rule::LetCommand),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn declare_command_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\ndeclare foo=bar\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
    }

    #[test]
    fn declare_command_in_dash_is_flagged() {
        let diagnostics = lint(
            "#!/bin/dash\ndeclare foo=bar\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
    }

    #[test]
    fn typeset_command_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\ntypeset foo=bar\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
        assert_eq!(
            diagnostics[0].message,
            "`typeset` is not portable in `sh` scripts"
        );
    }

    #[test]
    fn typeset_command_in_dash_is_flagged() {
        let diagnostics = lint(
            "#!/bin/dash\ntypeset foo=bar\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
        assert_eq!(
            diagnostics[0].message,
            "`typeset` is not portable in `sh` scripts"
        );
    }

    #[test]
    fn shopt_command_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nshopt -s nullglob\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
        assert_eq!(
            diagnostics[0].message,
            "`shopt` is not portable in `sh` scripts"
        );
    }

    #[test]
    fn pushd_command_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\npushd /tmp\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
        assert_eq!(
            diagnostics[0].message,
            "`pushd` is not portable in `sh` scripts"
        );
    }

    #[test]
    fn mapfile_command_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nmapfile entries\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::DeclareCommand);
        assert_eq!(
            diagnostics[0].message,
            "`mapfile` is not portable in `sh` scripts"
        );
    }

    #[test]
    fn declare_command_in_bash_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "#!/bin/bash\ndeclare foo=bar\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn typeset_command_in_bash_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "#!/bin/bash\ntypeset foo=bar\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn shopt_command_in_bash_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "#!/bin/bash\nshopt -s nullglob\n",
            &LinterSettings::for_rule(Rule::DeclareCommand),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn multiline_declare_command_is_clipped_to_opening_line() {
        let source = "#!/bin/sh\ndeclare -a values=(\n  one\n  two\n)\n";
        let diagnostics = lint(source, &LinterSettings::for_rule(Rule::DeclareCommand));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "declare -a values");
        assert_eq!(diagnostics[0].span.end.line, 2);
    }

    #[test]
    fn source_builtin_in_sh_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nsource ./helpers.sh\n",
            &LinterSettings::for_rule(Rule::SourceBuiltinInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceBuiltinInSh);
    }

    #[test]
    fn source_builtin_in_dash_is_flagged() {
        let diagnostics = lint(
            "#!/bin/dash\nsource ./helpers.sh\n",
            &LinterSettings::for_rule(Rule::SourceBuiltinInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceBuiltinInSh);
    }

    #[test]
    fn source_builtin_in_bash_is_not_flagged_for_portability_rule() {
        let diagnostics = lint(
            "#!/bin/bash\nsource ./helpers.sh\n",
            &LinterSettings::for_rule(Rule::SourceBuiltinInSh),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn source_builtin_in_command_substitution_is_flagged() {
        let diagnostics = lint(
            "#!/bin/sh\nversion=$(source ./helpers.sh)\n",
            &LinterSettings::for_rule(Rule::SourceBuiltinInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceBuiltinInSh);
    }

    #[test]
    fn source_builtin_inside_function_is_flagged_by_x031() {
        let diagnostics = lint(
            "#!/bin/sh\nload() {\n  source ./helpers.sh\n}\n",
            &LinterSettings::for_rule(Rule::SourceBuiltinInSh),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::SourceBuiltinInSh);
    }

    #[test]
    fn exported_variable_not_flagged() {
        let diagnostics = lint_for_rule("#!/bin/sh\nexport FOO=1\n", Rule::UnusedAssignment);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn branch_assignments_followed_by_a_read_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
${code_command} --version
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn mutually_exclusive_unused_branch_assignments_report_one_diagnostic() {
        let source = "\
#!/bin/sh
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 5);
    }

    #[test]
    fn branch_local_reads_suppress_unused_assignment_family() {
        let source = "\
#!/bin/sh
if a; then
  VAR=1
elif b; then
  VAR=2
else
  VAR=3
  echo \"$VAR\"
fi
";
        let diagnostics = lint_for_rule(source, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn case_branch_assignments_used_in_function_body_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
case \"$arch\" in
amd64 | x86_64)
  jq_arch=amd64
  core_arch=64
  ;;
arm64 | aarch64)
  jq_arch=arm64
  core_arch=arm64-v8a
  ;;
esac
download() {
  echo \"$jq_arch\"
  echo \"$core_arch\"
}
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn case_without_matching_arm_keeps_initializer_live() {
        let source = "\
#!/bin/sh
value=''
case \"$kind\" in
  one)
    value=1
    ;;
  two)
    value=2
    ;;
esac
printf '%s\\n' \"$value\"
";
        let diagnostics = lint_named_source_with_parse_dialect(
            Path::new("/tmp/case-no-match.sh"),
            source,
            ParseDialect::Posix,
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_global_assignments_read_later_by_caller_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
pass_args() {
  local_install=1
  proxy=$1
}
main() {
  pass_args \"$@\"
  printf '%s %s\\n' \"$local_install\" \"$proxy\"
}
main \"$@\"
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn recursive_function_state_assignment_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
check_status() {
  if [[ $is_wget ]]; then
    printf '%s\\n' ok
  else
    is_wget=1
    check_status
  fi
}
check_status
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_function_global_assignment_is_still_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
f() {
  foo=1
}
f
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(
            diagnostics[0]
                .span
                .slice("#!/bin/sh\nf() {\n  foo=1\n}\nf\n"),
            "foo"
        );
    }

    #[test]
    fn name_only_local_declaration_read_is_not_reported_as_uninitialized() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
f() {
  local foo
  printf '%s\\n' \"$foo\"
}
f
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn resolved_indirect_expansion_carrier_is_not_reported_as_uninitialized() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
f() {
  local foo
  printf '%s\\n' \"${!foo}\"
}
f
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn indirect_reads_do_not_report_missing_targets_for_indirect_or_nameref_access() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
name=missing
declare -n ref=missing
printf '%s %s\\n' \"${!name}\" \"$ref\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unresolved_indirect_expansion_carrier_is_still_reported() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${!foo}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("foo"));
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 16);
    }

    #[test]
    fn unresolved_indirect_expansion_with_subscript_reports_carrier_only() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${!tools[$target]}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("tools"));
        assert!(!diagnostics[0].message.contains("target"));
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 16);
    }

    #[test]
    fn unresolved_indirect_replacement_reports_carrier_only() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${!var//$'\\n'/' '}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("var"));
        assert!(!diagnostics[0].message.contains("!var//"));
    }

    #[test]
    fn indirect_special_parameter_carrier_is_not_reported() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
set -- last
printf '%s\\n' \"${!#}\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn special_hash_parameter_operations_are_not_reported() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
printf '%s\\n' \"${##*/}\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn special_zero_prefix_removal_inside_escaped_quotes_is_not_reported() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
usage=\"
Terraform:

    data \\\"external\\\" \\\"github_repos\\\" {
        program = [\\\"/path/to/${0##*/}\\\", \\\"github_repository\\\"]
    }
\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_anchors_parameter_operator_reports_to_carrier_name() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${missing%%/*}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("missing"));
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 16);
    }

    #[test]
    fn undefined_variable_anchors_escaped_quote_parameter_expansions_to_the_parameter() {
        let source = "\
#!/bin/bash
rvm_info=\"
  uname: \\\"${_system_info}\\\"
\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("_system_info"));
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 12);
        assert_eq!(diagnostics[0].span.slice(source), "${_system_info}");
    }

    #[test]
    fn undefined_variable_anchors_multiline_escaped_quote_parameter_expansions_to_the_parameter() {
        let source = "\
#!/bin/bash
payload=\"{
\t\\\"client_id\\\": \\\"${uuidinstance}\\\",
\t\\\"events\\\": [
\t\t{
\t\t\\\"name\\\": \\\"LinuxGSM\\\",
\t\t\\\"params\\\": {
\t\t\t\\\"cpuusedmhzroundup\\\": \\\"${cpuusedmhzroundup}\\\",
\t\t\t\\\"diskused\\\": \\\"${serverfilesdu}\\\",
\t\t\t}
\t\t}
\t]
}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("serverfilesdu"))
            .unwrap();

        assert_eq!(diagnostic.span.start.line, 9);
        assert_eq!(diagnostic.span.start.column, 20);
        assert_eq!(diagnostic.span.slice(source), "${serverfilesdu}");
    }

    #[test]
    fn undefined_variable_anchors_unbraced_references_after_escaped_quotes() {
        let source = "\
#!/bin/bash
rvm_info=\"
  path:         \\\"$rvm_path\\\"
\"
addtimestamp=\"gawk '{ print strftime(\\\\\\\"[$logtimestampformat]\\\\\\\"), \\\\\\$0 }'\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        let spans = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(spans, vec!["$rvm_path", "$logtimestampformat"]);
    }

    #[test]
    fn undefined_variable_ignores_self_referential_assignments() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
foo=\"$foo\"
for flag in a b; do
  valid_flags=\"${valid_flags} $flag\"
done
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_ignores_escaped_declaration_dynamic_assignments() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
\\typeset ret=$?
echo \"$ret\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_reports_arithmetic_conditional_literal_operands() {
        let source = "\
#!/bin/bash
version=1
if [[ $version -eq \"latest\" ]]; then
  :
fi
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("latest"));
        assert_eq!(diagnostics[0].span.slice(source), "\"latest\"");
    }

    #[test]
    fn undefined_variable_ignores_let_arithmetic_assignment_targets() {
        let source = "\
#!/bin/bash
let line=\"$number\"+1
printf '%s\\n' \"$line\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert_eq!(diagnostics[0].span.slice(source), "$number");
    }

    #[test]
    fn undefined_variable_ignores_assignment_values_after_escaped_newlines() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
easyrsa_ksh=\\
'value'
[ \"${KSH_VERSION}\" = \"${easyrsa_ksh}\" ] && echo ok
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_ignores_backtick_double_escaped_echo_templates() {
        let source = "\
#!/bin/bash
XDGPATH=`echo \"foreach dir [split [::tcl::tm::path list]] {puts \\\\$dir}\" | tclsh | tail -n1`
printf '%s\\n' \"$missing\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$missing");
    }

    #[test]
    fn undefined_variable_reports_unparsed_indexed_subscript_prefixes() {
        let source = "\
#!/bin/bash
arr+=([docker:dind]=x [nats-streaming:nanoserver]=y)
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["docker", "nats", "streaming"]
        );
    }

    #[test]
    fn undefined_variable_skips_parameter_replacement_pattern_reads() {
        let source = "\
#!/bin/bash
dir=all/retroarch.cfg
echo \"${dir//$configdir\\/}\"
find \"$configdir\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$configdir"]
        );
    }

    #[test]
    fn undefined_variable_reports_plain_reads_before_parameter_patterns() {
        let source = "\
#!/bin/bash
dir=all/retroarch.cfg
find \"$configdir\"
echo \"${dir//$configdir\\/}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$configdir"]
        );
    }

    #[test]
    fn undefined_variable_reports_redirect_target_references() {
        let source = "\
#!/bin/bash
{ echo value; } >> \"${missing_target}/out\"
echo \"${ordinary_missing}/out\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${missing_target}", "${ordinary_missing}"]
        );
    }

    #[test]
    fn undefined_variable_reports_unreachable_references() {
        let source = "\
#!/bin/bash
load_value() {
  return 1
  printf '%s\\n' \"$after_return\"
}
load_value
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$after_return"]
        );
    }

    #[test]
    fn undefined_variable_ignores_bound_name_between_escaped_quote_literals() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/sh
archname=archive
echo Self-extractable archive \\\"$archname\\\" successfully created.
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unquoted_heredoc_generated_shell_text_reports_c006() {
        let diagnostics = lint_for_rule(
            "\
archname=archive
cat <<EOF > \"$archname\"
#!/bin/sh
ORIG_UMASK=`umask`
if test \"$KEEP_UMASK\" = n; then
    umask 077
fi

CRCsum=\"$CRCsum\"
archdirname=\"$archdirname\"
EOF
",
            Rule::UndefinedVariable,
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("CRCsum"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("archdirname"))
        );
    }

    #[test]
    fn escaped_heredoc_parameter_literals_report_nested_references() {
        let source = "\
#!/bin/bash
cat <<EOF
\\${OUTER:-$inner}
EOF
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("inner"));
        assert_eq!(diagnostics[0].span.slice(source), "$inner");
    }

    #[test]
    fn undefined_variable_reports_bash_fallback_after_zsh_split_branch() {
        let source = "\
#!/bin/bash
if [[ -n \"${ZSH_VERSION:-}\" ]]; then
  rvm_configure_flags=( ${=db_configure_flags} \"${rvm_configure_flags[@]}\" )
else
  rvm_configure_flags=( ${db_configure_flags} \"${rvm_configure_flags[@]}\" )
fi
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert_eq!(diagnostics[0].span.start.line, 5);
        assert_eq!(diagnostics[0].span.slice(source), "${db_configure_flags}");
    }

    #[test]
    fn undefined_variable_ignores_parameter_slice_arithmetic_operands() {
        let source = "\
#!/bin/bash
value=abcdef
printf '%s\\n' \"${value:offset}\" \"${value:1:$length}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn undefined_variable_ignores_names_bound_anywhere_in_the_file() {
        let source = "\
#!/bin/bash
echo \"$missing\"
if true; then
  maybe=1
fi
echo \"$maybe\"
echo \"$late\"
late=1
helper() {
  printf '%s\\n' \"$package\" \"$seeded_elsewhere\"
}
seed() {
  local seeded_elsewhere=1
}
package=readline
helper
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("missing"));
        assert!(
            diagnostics[0]
                .message
                .contains("referenced before assignment")
        );
    }

    #[test]
    fn undefined_variable_ignores_same_declaration_command_bindings() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
f() {
  local first=1 second=\"$first\"
  local later=\"$after\" after=1
}
f
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_reports_only_first_reportable_use_per_name() {
        let source = "\
#!/bin/bash
helper() {
  printf '%s %s\\n' \"$missing\" \"$also_missing\"
}
printf '%s\\n' \"$missing\"
printf '%s\\n' \"$also_missing\"
helper
printf '%s %s\\n' \"$missing\" \"$also_missing\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert!(diagnostics[0].message.contains("missing"));
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[1].rule, Rule::UndefinedVariable);
        assert!(diagnostics[1].message.contains("also_missing"));
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn undefined_variable_parameter_guard_flow_respects_same_command_order() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$before_default\" \"${before_default:-fallback}\"
printf '%s\\n' \"${guarded:-fallback}\" \"$guarded\"
printf '%s\\n' \"$plain_missing\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("before_default")
                    && diagnostic.span.start.line == 2)
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("plain_missing")
                    && diagnostic.span.start.line == 4)
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("guarded"))
        );
    }

    #[test]
    fn undefined_variable_ignores_declaration_names_and_special_parameters() {
        let diagnostics = lint_for_rule(
            "\
#!/bin/bash
readonly declared
export exported
printf '%s %s %s\\n' \"$1\" \"$@\" \"$#\"
printf '%s %s\\n' \"${#}\" \"${$}\"
",
            Rule::UndefinedVariable,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn undefined_variable_ignores_bash_runtime_vars_in_bash_scripts() {
        let source = runtime_prelude_source("#!/bin/bash");
        let diagnostics = lint_for_rule(&source, Rule::UndefinedVariable);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn undefined_variable_ignores_environment_style_names() {
        let source = "\
#!/bin/sh
printf '%s %s %s %s %s %s %s\\n' \
  \"$FOO\" \
  \"$PATH\" \
  \"$UID\" \
  \"$XDG_CONFIG_HOME\" \
  \"$OPTARG\" \
  \"$OPTIND\" \
  \"$__FOO\"
printf '%s %s\\n' \"$foo\" \"$Foo_BAR\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("foo"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("Foo_BAR"))
        );
    }

    #[test]
    fn undefined_variable_can_report_environment_style_names_when_requested() {
        let source = "\
#!/bin/sh
printf '%s %s\\n' \"$FOO\" \"$XDG_CONFIG_HOME\"
";
        let diagnostics = lint(
            source,
            &LinterSettings {
                rules: RuleSet::from_iter([Rule::UndefinedVariable]),
                report_environment_style_names: true,
                ..LinterSettings::default()
            },
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("FOO"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("XDG_CONFIG_HOME"))
        );
    }

    #[test]
    fn undefined_variable_ignores_guarded_parameter_expansions() {
        let source = "\
#!/bin/sh
printf '%s %s %s %s\\n' \
  \"${missing_default:-fallback}\" \
  \"${missing_assign:=value}\" \
  \"${missing_replace:+alt}\" \
  \"${missing_error:?missing}\"
printf '%s %s %s %s %s\\n' \
  \"${missing_default:-$fallback_name}\" \
  \"${missing_assign:=${seed_name:-value}}\" \
  \"${missing_replace:+$replacement_name}\" \
  \"${missing_error:?$hint_name}\" \
  \"$missing_assign\"
printf '%s\\n' \"$fallback_name\" \"$seed_name\" \"$replacement_name\" \"$hint_name\" \"$plain_missing\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.rule == Rule::UndefinedVariable)
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("plain_missing"))
        );
    }

    #[test]
    fn undefined_variable_ignores_associative_subscript_literals() {
        let source = "\
#!/bin/bash
declare -A map
map[swift-cmark]=1
printf '%s %s\\n' \"${map[swift-cmark]}\" \"${map[$dynamic_key]}\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_suppresses_later_subscript_uses_after_read_subscripts() {
        let source = "\
#!/bin/bash
declare -a args
declare -A tools
printf '%s\\n' \"${args[$__array_start]}\"
args[$__array_start]=ok
unset args[$unset_index]
printf '%s\\n' \"${tools[$target]}\"
tools[$target]=ok
printf '%s\\n' \"$__array_start\" \"$target\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn undefined_variable_reports_unseen_plain_uses_after_subscript_only_uses() {
        let source = "\
#!/bin/bash
declare -a args
declare -A tools
printf '%s %s\\n' \"${args[$idx]}\" \"${tools[$target]}\"
printf '%s %s %s\\n' \"$idx\" \"$target\" \"$unseen\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$unseen"]
        );
    }

    #[test]
    fn undefined_variable_ignores_presence_tested_names_in_supported_guards() {
        let source = "\
#!/bin/bash
[ -z \"$guarded\" ] && echo nope
[ \"$truthy\" ] && echo maybe
[ -v simple_v ] && echo set
test -v test_v && echo set
[ -z \"$chain_left\" -a -z \"$chain_right\" ] && echo both
[ \"$or_left\" -o \"$or_right\" ] && echo either
if [[ -n \"${nonempty:-}\" && \"$also_truthy\" ]]; then
  echo yes
fi
if [[ -v conditional_v ]]; then
  echo set
fi
if [[ ! -v conditional_not_v ]]; then
  echo unset
fi
if [ \"$eq_mix\" = x -a -z \"$guard_after_eq\" ]; then
  echo no
fi
if [[ \"$eq_only\" = x ]]; then
  echo no
fi
if [[ -s \"$file_only\" ]]; then
  echo no
fi
echo \"$guarded\" \"$truthy\" \"$simple_v\" \"$test_v\" \"$chain_left\" \"$chain_right\" \"$or_left\" \"$or_right\" \"$nonempty\" \"$also_truthy\" \"$conditional_v\" \"$conditional_not_v\" \"$eq_mix\" \"$guard_after_eq\" \"$eq_only\" \"$file_only\" \"$still_missing\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("guarded"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("truthy"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("simple_v"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("test_v"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("chain_left"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("chain_right"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("or_left"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("or_right"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("nonempty"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("also_truthy"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("conditional_v"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("conditional_not_v"))
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("guard_after_eq"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("eq_mix"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("eq_only"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("file_only"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("still_missing"))
        );
    }

    #[test]
    fn undefined_variable_reports_plain_test_command_presence_reads() {
        let source = "\
#!/bin/bash
test -n \"$plain_test\" && echo present
[ -n \"$bracket_test\" ] && echo present
if [[ -n \"$conditional_test\" ]]; then
  echo present
fi
echo \"$plain_test\" \"$bracket_test\" \"$conditional_test\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$plain_test"]
        );
    }

    #[test]
    fn undefined_variable_nested_word_guards_do_not_suppress_plain_uses() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${fallback:-$([ \"$missing\" ])}\"
printf '%s\\n' \"$missing\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn undefined_variable_keeps_nested_word_guard_suppression_inside_same_substitution() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$([ -n \"$missing\" ] && printf '%s' \"$missing\")\"
";
        let diagnostics = lint_for_rule(source, Rule::UndefinedVariable);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unread_name_only_declarations_are_flagged() {
        let source = "\
#!/bin/bash
f() {
  local foo
  declare bar
  typeset baz
}
f
";
        let diagnostics = lint(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
        assert_eq!(diagnostics[1].span.slice(source), "bar");
        assert_eq!(diagnostics[2].span.slice(source), "baz");
    }

    #[test]
    fn initialized_local_declaration_is_flagged_when_unused() {
        let diagnostics = lint(
            "\
#!/bin/bash
f() {
  local foo=1
}
f
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("foo"));
    }

    #[test]
    fn name_only_export_consumes_existing_assignment() {
        let diagnostics = lint_for_rule("#!/bin/sh\nfoo=1\nexport foo\n", Rule::UnusedAssignment);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn name_only_readonly_consumes_existing_assignment() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\nreadonly foo\n",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn corpus_false_negative_moduleselfname_is_now_flagged() {
        let diagnostics = lint(
            "#!/bin/bash\nmoduleselfname=\"$(basename \"$(readlink -f \"${BASH_SOURCE[0]}\")\")\"\n",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert!(diagnostics[0].message.contains("moduleselfname"));
    }

    #[test]
    fn global_assignment_used_in_a_function_body_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
red='\\e[31m'
print_red() { printf '%s\\n' \"$red\"; }
print_red
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn top_level_assignment_read_by_later_function_call_is_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/sh
show() { echo \"$flag\"; }
flag=1
show
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn callee_subshell_reads_keep_caller_assignments_live() {
        let diagnostics = lint(
            "\
#!/bin/bash
install_package() {
  (
    printf '%s\\n' \"$archive_format\" \"${configure[@]}\"
  )
}
install_readline() {
  archive_format='tar.gz'
  configure=( ./configure --disable-dependency-tracking )
  install_package
}
install_readline
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn completion_reply_assignments_are_not_flagged() {
        let diagnostics = lint(
            "\
#!/bin/bash
_pyenv() {
  COMPREPLY=()
  local word=\"${COMP_WORDS[COMP_CWORD]}\"
  COMPREPLY=( $(compgen -W \"$(printf 'a b')\" -- \"$word\") )
}
complete -F _pyenv pyenv
",
            &LinterSettings::for_rule(Rule::UnusedAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn sourced_helper_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
. ./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn disabled_source_closure_reports_assignment_only_read_by_sourced_helper() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("lib.sh");
        let source = "\
#!/bin/sh
foo=1
. ./lib.sh
";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "printf '%s\\n' \"$foo\"\n").unwrap();

        let diagnostics = lint_path(
            &main,
            &LinterSettings::for_rule(Rule::UnusedAssignment).with_resolve_source_closure(false),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn disabled_source_closure_reports_read_only_assigned_by_sourced_helper() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("lib.sh");
        let source = "\
#!/bin/sh
. ./lib.sh
printf '%s\\n' \"$foo\"
";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "foo=1\n").unwrap();

        let diagnostics = lint_path(
            &main,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_resolve_source_closure(false),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert_eq!(diagnostics[0].span.slice(source), "$foo");
    }

    #[test]
    fn sourced_helper_function_reads_do_not_keep_top_level_assignment_live_until_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/sh
flag=1
. ./helper.sh
";
        fs::write(&main, source).unwrap();
        fs::write(
            &helper,
            "\
use_flag() {
  printf '%s\\n' \"$flag\"
}
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "flag");
    }

    #[test]
    fn sourced_helper_function_reads_keep_top_level_assignment_live_when_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
. ./helper.sh
use_flag
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
use_flag() {
  printf '%s\\n' \"$flag\"
}
",
        )
        .unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn generic_dynamic_source_function_writes_do_not_initialize_c006_reads() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("tests/main.sh");
        let helper = temp.path().join("scripts/helper.sh");
        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let source = "\
#!/bin/sh
helper_root=/tmp
. \"$helper_root/scripts/helper.sh\"
set_flag
printf '%s\\n' \"$flag\"
";
        fs::write(&main, source).unwrap();
        fs::write(
            &helper,
            "\
set_flag() {
  flag=1
}
",
        )
        .unwrap();

        let main_path = main.clone();
        let helper_path = helper.clone();
        let resolver = move |source_path: &Path, candidate: &str| {
            if source_path == main_path.as_path() && candidate == "scripts/helper.sh" {
                vec![helper_path.clone()]
            } else {
                Vec::new()
            }
        };

        let diagnostics =
            lint_path_for_rule_with_resolver(&main, Rule::UndefinedVariable, Some(&resolver));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UndefinedVariable);
        assert_eq!(diagnostics[0].span.slice(source), "$flag");
    }

    #[test]
    fn source_builtin_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./helper.bash
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_scalar_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_index_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[0]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_double_zero_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[00]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_spaced_zero_suffix_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[ 0 ]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_nonzero_suffix_source_does_not_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[1]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn bash_source_scalar_dirname_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn bash_source_index_dirname_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"$(dirname \"${BASH_SOURCE[0]}\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn executed_helper_reads_keep_loop_variable_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
for queryip in 127.0.0.1; do
  helper.sh
done
",
        )
        .unwrap();
        fs::write(&helper, "printf '%s\\n' \"$queryip\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn executed_helper_without_read_still_flags_unused_assignment() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/sh
unused=1
helper.sh
";
        fs::write(&main, source).unwrap();
        fs::write(&helper, "printf '%s\\n' ok\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UnusedAssignment);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn loader_function_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
load() { . \"$ROOT/$1\"; }
flag=1
load helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let diagnostics = lint_path_for_rule(&main, Rule::UnusedAssignment);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_reports_each_unreachable_command() {
        let source = "\
#!/bin/bash
if [ -f /etc/hosts ]; then
  echo found
  exit 0
else
  echo missing
  exit 1
fi
echo unreachable
printf '%s\\n' never
f() {
  return 0
  echo also_unreachable
}
f
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 4);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule == Rule::UnreachableAfterExit)
        );
        assert_eq!(diagnostics[0].span.slice(source), "echo unreachable");
        assert_eq!(diagnostics[1].span.slice(source), "printf '%s\\n' never");
        assert_eq!(
            diagnostics[2].span.slice(source),
            "f() {\n  return 0\n  echo also_unreachable\n}"
        );
        assert_eq!(diagnostics[3].span.slice(source), "f");
    }

    #[test]
    fn unreachable_after_exit_prefers_outermost_compound_command_spans() {
        let source = "\
#!/bin/bash
return
if true; then
  echo one
fi
printf '%s\\n' two
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "if true; then\n  echo one\nfi"
        );
        assert_eq!(diagnostics[0].span.end.line, 5);
        assert_eq!(diagnostics[0].span.end.column, 3);
        assert_eq!(diagnostics[1].span.slice(source), "printf '%s\\n' two");
        assert_eq!(diagnostics[1].span.end.line, 6);
        assert_eq!(diagnostics[1].span.end.column, 18);
    }

    #[test]
    fn unreachable_after_exit_reports_after_script_terminating_function_calls() {
        let source = "\
#!/bin/bash
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' never
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "printf '%s\\n' never");
    }

    #[test]
    fn unreachable_after_exit_ignores_helper_exit_calls_in_sourceable_files() {
        let source = "\
#!/bin/sh
[ -n \"$loaded\" ] && return
loaded=1
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' still_reachable
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unreachable_after_exit_ignores_statements_inside_unreached_functions() {
        let source = "\
#!/bin/bash
helper() {
  return 0
  printf '%s\\n' unreachable_inside_unreached_function
}
exit 0
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unreachable_after_exit_ignores_dynamic_dispatch_only_functions_before_exit() {
        let source = "\
#!/bin/bash
dispatch() {
  \"$command\"
}
helper() {
  return 0
  printf '%s\\n' unreachable_inside_dynamic_target
}
dispatch
exit 0
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unreachable_after_exit_reports_inside_transitively_called_functions_before_exit() {
        let source = "\
#!/bin/bash
helper() {
  return 0
  printf '%s\\n' still_reported
}
main() {
  helper
}
main
exit 0
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "printf '%s\\n' still_reported"
        );
    }

    #[test]
    fn unreachable_after_exit_reports_inside_later_defined_transitive_functions() {
        let source = "\
#!/bin/bash
main() {
  helper
}
helper() {
  return 0
  printf '%s\\n' still_reported
}
main
exit 0
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "printf '%s\\n' still_reported"
        );
    }

    #[test]
    fn unreachable_after_exit_reports_inside_called_nested_functions_before_exit() {
        let source = "\
#!/bin/bash
outer() {
  helper() {
    return 0
    printf '%s\\n' still_reported_nested
  }
  helper
}
outer
exit 0
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "printf '%s\\n' still_reported_nested"
        );
    }

    #[test]
    fn unreachable_after_exit_reports_before_sourceable_footer_return() {
        let source = "\
#!/bin/bash
finish() {
  exit \"$1\"
}
terminal() {
  finish 34 && return 34
}
return 0
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "return 34");
    }

    #[test]
    fn unreachable_after_exit_reports_uncalled_function_when_exit_is_conditional() {
        let source = "\
#!/bin/bash
helper() {
  return 0
  printf '%s\\n' still_reported
}
if maybe; then
  exit 0
fi
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "printf '%s\\n' still_reported"
        );
    }

    #[test]
    fn unreachable_after_exit_reports_after_redirected_exit_helpers() {
        let source = "\
#!/bin/bash
exit_script() {
  exit 0
}
main() {
  exit_script >/dev/null
  printf '%s\\n' never
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "printf '%s\\n' never");
    }

    #[test]
    fn unreachable_after_exit_reports_condition_body_after_terminating_condition() {
        let source = "\
#!/bin/bash
if exit 0; then
  printf '%s\\n' never
fi
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "printf '%s\\n' never");
    }

    #[test]
    fn unreachable_after_exit_includes_redirects_but_not_statement_terminators() {
        let source = "\
#!/bin/bash
exit 0
while read -r item; do
  printf '%s\\n' \"$item\"
done < input.txt;
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "while read -r item; do\n  printf '%s\\n' \"$item\"\ndone < input.txt"
        );
    }

    #[test]
    fn unreachable_after_exit_ignores_loop_control_only_dead_code() {
        let source = "\
#!/bin/bash
while true; do
  break; printf '%s\\n' after_break
done
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_ignores_loop_control_if_branches_and_following_code() {
        let source = "\
#!/bin/bash
while true; do
  if break; then
    printf '%s\\n' after_true
  else
    printf '%s\\n' after_false
  fi
  printf '%s\\n' after_if
done
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_reports_after_brace_group_defined_exit_helpers() {
        let source = "\
#!/bin/bash
{
  exit_script() {
    exit 0
  }
}
main() {
  exit_script
  printf '%s\\n' never
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "printf '%s\\n' never");
    }

    #[test]
    fn unreachable_after_exit_reports_after_later_parent_scope_exit_helpers() {
        let source = "\
#!/bin/bash
main() {
  exit_script
  printf '%s\\n' never
}
exit_script() {
  exit 0
}
main
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "printf '%s\\n' never");
    }

    #[test]
    fn unreachable_after_exit_ignores_later_function_definitions_for_earlier_calls() {
        let source = "\
#!/bin/bash
main() {
  exit_script
  printf '%s\\n' still_reachable
}
main
exit_script() {
  exit 0
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_ignores_transitive_calls_before_parent_definitions() {
        let source = "\
#!/bin/bash
main() {
  helper
}
helper() {
  inner
}
inner() {
  exit_script
  printf '%s\\n' maybe
}
if should_run; then
  main
fi
exit_script() {
  exit 0
}
main
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_ignores_stale_terminating_helper_redefinitions() {
        let source = "\
#!/bin/bash
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' maybe
}
if should_run; then
  main
fi
exit_script() {
  :
}
main
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_ignores_conditionally_defined_exit_helpers() {
        let source = "\
#!/bin/bash
if false; then
  exit_script() {
    exit 0
  }
fi
exit_script
printf '%s\\n' still_reachable
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unreachable_after_exit_ignores_fallback_after_conditional_exit() {
        let source = "\
#!/bin/bash
run && exit 0 || sleep 15
";
        let diagnostics = lint(source, &LinterSettings::default());

        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.rule == Rule::ChainedTestBranches)
                .count(),
            1
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule != Rule::UnreachableAfterExit),
            "diagnostics: {diagnostics:?}"
        );
    }

    #[test]
    fn unreachable_after_exit_skips_dead_short_circuit_lists() {
        let source = "\
#!/bin/bash
exit 0
echo one && echo two
echo after
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "echo after");
    }

    #[test]
    fn unreachable_after_exit_skips_dead_short_circuit_exit_guards() {
        let source = "\
#!/bin/bash
exit 0
cleanup || exit 1
echo after
printf '%s\\n' later
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "echo after");
        assert_eq!(diagnostics[1].span.slice(source), "printf '%s\\n' later");
    }

    #[test]
    fn unreachable_after_exit_skips_dead_short_circuit_segments() {
        let source = "\
#!/bin/bash
usage() { exit 0; }
error() {
  [ $# -eq 0 ] && usage && exit 0
  echo after
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unreachable_after_exit_reports_nested_dead_code_in_skipped_short_circuit_segments() {
        let source = "\
#!/bin/bash
check() {
  [ \"$1\" = stop ] && { return 0; echo inner; } && echo tail
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "echo inner");
    }

    #[test]
    fn unreachable_after_exit_reports_shadowed_condition_names_in_short_circuit_lists() {
        let source = "\
#!/bin/bash
true() {
  exit 0
}
check() {
  true && echo a && echo b
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "echo a");
        assert_eq!(diagnostics[1].span.slice(source), "echo b");
    }

    #[test]
    fn unreachable_after_exit_reports_shadowed_condition_wrapper_names() {
        for wrapper in ["command", "builtin", "sudo", "doas", "run0"] {
            let source = format!(
                "\
#!/bin/bash
{wrapper}() {{
  exit 0
}}
check() {{
  {wrapper} true && echo a && echo b
}}
"
            );
            let diagnostics = lint_for_rule(&source, Rule::UnreachableAfterExit);

            assert_eq!(diagnostics.len(), 2, "{wrapper}: {diagnostics:?}");
            assert_eq!(diagnostics[0].span.slice(&source), "echo a", "{wrapper}");
            assert_eq!(diagnostics[1].span.slice(&source), "echo b", "{wrapper}");
        }
    }

    #[test]
    fn unreachable_after_exit_ignores_conditionally_defined_condition_names() {
        let source = "\
#!/bin/bash
die() {
  exit 1
}
check() {
  if maybe; then
    true() { exit 0; }
  fi
  true && die && exit 1
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn unreachable_after_exit_keeps_dead_two_segment_short_circuit_tail() {
        let source = "\
#!/bin/bash
finish() { exit \"$1\"; }
terminal() {
  finish 34 && return 34
}
";
        let diagnostics = lint_for_rule(source, Rule::UnreachableAfterExit);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "return 34");
    }

    #[test]
    fn unused_assignment_respects_disabled_rule() {
        let diagnostics = lint(
            "#!/bin/sh\nfoo=1\n",
            &LinterSettings {
                rules: RuleSet::EMPTY,
                ..LinterSettings::default()
            },
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2034
foo=1
";
        let diagnostics = lint(source, &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn parsed_result_linting_respects_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2034
foo=1
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        let diagnostics = lint_file_with_directives(
            &output,
            source,
            &indexer,
            &LinterSettings::default(),
            &directives,
            None,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unused_assignment_suppression_stays_on_matching_binding_line() {
        let source = "\
#!/bin/bash
:
# shellcheck disable=SC2034
foo=1
foo=2
";
        let diagnostics = lint(source, &LinterSettings::default());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 5);
    }

    #[test]
    fn redundant_return_status_suppressed_by_legacy_shuck_directive() {
        let source = "\
#!/bin/sh
# shuck: disable=SH-170
f() {
  false
  return $?
}
";
        let diagnostics = lint_for_rule(source, Rule::RedundantReturnStatus);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_top_level_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/bash
# shellcheck disable=SC2168
local foo=bar
printf '%s\\n' \"$foo\"
";
        let diagnostics = lint(source, &LinterSettings::default());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_declare_combined_suppressed_by_shellcheck_alias_directive() {
        let source = "\
#!/bin/bash
# shellcheck disable=SC2316
f() {
  local declare hard_list
  echo \"$hard_list\"
}
";
        let diagnostics = lint_for_rule(source, Rule::LocalDeclareCombined);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn backtick_in_command_position_suppressed_by_shellcheck_alias_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2316
`echo hello` | cat
";
        let diagnostics = lint_for_rule(source, Rule::BacktickInCommandPosition);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn compound_test_operator_suppressed_by_shellcheck_disable_all() {
        let source = "\
#!/bin/bash
# shellcheck disable=all
[ \"$a\" = 1 -a \"$b\" = 2 ]
";
        let diagnostics = lint_for_rule(source, Rule::CompoundTestOperator);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn local_in_sh_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC3043
f() {
  local foo=bar
  printf '%s\\n' \"$foo\"
}
f
";
        let diagnostics = lint_for_rule(source, Rule::LocalVariableInSh);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_keyword_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2113
function f { :; }
";
        let diagnostics = lint_for_rule(source, Rule::FunctionKeyword);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn backslash_before_command_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2268
\\command printf '%s\\n' hi
";
        let diagnostics = lint_for_rule(source, Rule::BackslashBeforeCommand);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn literal_control_escape_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC1012
echo \\n
";
        let diagnostics = lint_for_rule(source, Rule::LiteralControlEscape);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn let_command_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC3042
let x=1
";
        let diagnostics = lint_for_rule(source, Rule::LetCommand);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn declare_command_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC3044
declare foo=bar
";
        let diagnostics = lint_for_rule(source, Rule::DeclareCommand);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn source_builtin_in_sh_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC3046
source ./helpers.sh
";
        let diagnostics = lint_for_rule(source, Rule::SourceBuiltinInSh);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_keyword_with_parens_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2112
function f() { :; }
";
        let diagnostics = lint_for_rule(source, Rule::FunctionKeywordInSh);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn array_index_arithmetic_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/bash
# shellcheck disable=SC2321
arr[$((1+1))]=x
";
        let diagnostics = lint_for_rule(source, Rule::ArrayIndexArithmetic);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn source_inside_function_suppressed_by_shellcheck_directive() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC3084
f() {
  source ./helpers.sh
}
";
        let diagnostics = lint_for_rule(source, Rule::SourceInsideFunctionInSh);
        assert!(diagnostics.is_empty());
    }
}
