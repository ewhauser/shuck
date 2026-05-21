use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, Assignment,
    AssignmentValue, BinaryCommand, BinaryOp, BourneParameterExpansion, CaseCommand, CaseItem,
    Command, CommandSubstitutionSyntax, CompoundCommand, ConditionalCommand, ConditionalExpr,
    DeclOperand, File, ForCommand, FunctionDef, Heredoc, HeredocBody, HeredocBodyPart,
    HeredocBodyPartNode, IfCommand, ParameterExpansion, ParameterExpansionSyntax, ParameterOp,
    Pattern, PatternPart, Redirect, RedirectKind, RedirectTarget, RepeatCommand, SelectCommand,
    Span, Stmt, StmtSeq, StmtTerminator, Subscript, TimeCommand, UntilCommand, VarRef,
    WhileCommand, Word, WordPart, WordPartNode, ZshExpansionOperation, ZshExpansionTarget,
    ZshGlobSegment,
};
use shuck_ast::{TextRange, TextSize};
use shuck_indexer::{CommentIndex, IndexedComment, Indexer, IndexerOptions, LineIndex};

use crate::command::{
    CompoundBodySite, array_elem_parts, builtin_like_parts, case_item_body_upper_bound,
    case_item_was_inline_in_source, case_terminator,
    collect_binary_list_first as collect_binary_list_first_with, collect_pipeline_parts,
    command_group_commands, group_attachment_span_with_heredoc, group_open_suffix,
    group_was_inline_in_source, if_close_span, if_next_branch_region_with_body_end,
    matching_group_close, rendered_stmt_end_line_with_heredoc, should_render_verbatim_with_heredoc,
    stmt_attachment_span_with_heredoc, stmt_format_span,
    stmt_group_attachment_or_verbatim_span_with_heredoc, stmt_has_trailing_comment,
    stmt_render_start_line, stmt_span, stmt_start_after_operator,
    stmt_verbatim_span_with_source_map, trim_unescaped_trailing_whitespace,
};
use crate::comments::{
    CommentAttachmentModel, SequenceCommentAttachment, SourceComment, SourceMap,
};
use crate::options::{LineEnding, ResolvedShellFormatOptions};
use crate::scan::{
    BranchPrefixComment, last_shell_keyword_end, last_shell_keyword_start, source_between_offsets,
};
use crate::visit::{self, AstVisitor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FactSpan {
    start: usize,
    end: usize,
}

impl FactSpan {
    fn new(span: Span) -> Self {
        Self {
            start: span.start.offset,
            end: span.end.offset,
        }
    }

    fn from_offsets(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

impl From<Span> for FactSpan {
    fn from(span: Span) -> Self {
        Self::new(span)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SequenceSiteKey {
    span: FactSpan,
    upper_bound: Option<usize>,
}

impl SequenceSiteKey {
    fn new(sequence: &StmtSeq, upper_bound: Option<usize>) -> Self {
        Self {
            span: FactSpan::from(sequence.span),
            upper_bound,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SequenceSite<'a> {
    sequence: &'a StmtSeq,
    upper_bound: Option<usize>,
    group_open_char: Option<char>,
    open_suffix_span: Option<Span>,
    open_end_offset: Option<usize>,
}

impl<'a> SequenceSite<'a> {
    fn new(
        sequence: &'a StmtSeq,
        upper_bound: Option<usize>,
        group_open_char: Option<char>,
        open_suffix_span: Option<Span>,
        open_end_offset: Option<usize>,
    ) -> Self {
        Self {
            sequence,
            upper_bound,
            group_open_char,
            open_suffix_span,
            open_end_offset,
        }
    }

    fn key(self) -> SequenceSiteKey {
        SequenceSiteKey::new(self.sequence, self.upper_bound)
    }
}

#[derive(Debug, Clone, Copy)]
struct StmtSite<'a> {
    stmt: &'a Stmt,
    key: FactSpan,
}

impl<'a> StmtSite<'a> {
    fn new(stmt: &'a Stmt) -> Self {
        Self {
            stmt,
            key: FactSpan::from(stmt_span(stmt)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OffsetRegionKey {
    start: usize,
    end: usize,
}

impl OffsetRegionKey {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BranchPrefixFacts {
    comments: Vec<BranchPrefixComment>,
    first_comment_offset: Option<usize>,
    has_blank_line_before_keyword: bool,
    has_blank_line_after_comments: bool,
}

impl BranchPrefixFacts {
    fn new(source: &str, start: usize, end: usize, comments: Vec<BranchPrefixComment>) -> Self {
        let first_comment_offset = comments.first().map(|comment| comment.offset);
        let has_blank_line_before_keyword =
            gap_has_empty_physical_line(source, start, first_comment_offset.unwrap_or(end));
        let has_blank_line_after_comments = comments.last().is_some_and(|last| {
            line_end_for_offset(source, last.offset)
                .filter(|line_end| *line_end < end)
                .is_some_and(|line_end| gap_has_empty_physical_line(source, line_end, end))
        });

        Self {
            comments,
            first_comment_offset,
            has_blank_line_before_keyword,
            has_blank_line_after_comments,
        }
    }

    pub(crate) fn comments(&self) -> &[BranchPrefixComment] {
        &self.comments
    }

    pub(crate) fn first_comment_offset(&self) -> Option<usize> {
        self.first_comment_offset
    }

    pub(crate) fn has_blank_line_before_keyword(&self) -> bool {
        self.has_blank_line_before_keyword
    }

    pub(crate) fn has_blank_line_after_comments(&self) -> bool {
        self.has_blank_line_after_comments
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CaseCommandFacts {
    esac_span: Option<Span>,
    body_fallback_upper_bound: usize,
    has_blank_line_after_in: bool,
    has_blank_line_before_esac: bool,
    suffix_comments_before_esac: Vec<BranchPrefixComment>,
}

impl CaseCommandFacts {
    pub(crate) fn esac_span(&self) -> Option<Span> {
        self.esac_span
    }

    pub(crate) fn body_fallback_upper_bound(&self) -> usize {
        self.body_fallback_upper_bound
    }

    pub(crate) fn has_blank_line_after_in(&self) -> bool {
        self.has_blank_line_after_in
    }

    pub(crate) fn has_blank_line_before_esac(&self) -> bool {
        self.has_blank_line_before_esac
    }

    pub(crate) fn suffix_comments_before_esac(&self) -> &[BranchPrefixComment] {
        &self.suffix_comments_before_esac
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CaseItemFacts<'source> {
    suffix_comment_start_line: Option<usize>,
    has_blank_line_before: bool,
    has_blank_line_after_pattern: bool,
    has_blank_line_before_terminator: bool,
    prefix_comments: Vec<SourceComment<'source>>,
    pattern_suffix_comment: Option<SourceComment<'source>>,
    terminator_suffix_comment: Option<SourceComment<'source>>,
}

impl<'source> CaseItemFacts<'source> {
    pub(crate) fn suffix_comment_start_line(&self) -> Option<usize> {
        self.suffix_comment_start_line
    }

    pub(crate) fn has_blank_line_before(&self) -> bool {
        self.has_blank_line_before
    }

    pub(crate) fn has_blank_line_after_pattern(&self) -> bool {
        self.has_blank_line_after_pattern
    }

    pub(crate) fn has_blank_line_before_terminator(&self) -> bool {
        self.has_blank_line_before_terminator
    }

    pub(crate) fn prefix_comments(&self) -> &[SourceComment<'source>] {
        &self.prefix_comments
    }

    pub(crate) fn pattern_suffix_comment(&self) -> Option<SourceComment<'source>> {
        self.pattern_suffix_comment
    }

    pub(crate) fn terminator_suffix_comment(&self) -> Option<SourceComment<'source>> {
        self.terminator_suffix_comment
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StmtFacts {
    attachment_span: Span,
    render_span: Span,
    rendered_end_line: usize,
    has_trailing_comment: bool,
    preserve_verbatim: bool,
    contains_heredoc: bool,
}

impl StmtFacts {
    pub(crate) fn attachment_span(&self) -> Span {
        self.attachment_span
    }

    pub(crate) fn render_span(&self) -> Span {
        self.render_span
    }

    pub(crate) fn rendered_end_line(&self) -> usize {
        self.rendered_end_line
    }

    pub(crate) fn has_trailing_comment(&self) -> bool {
        self.has_trailing_comment
    }

    pub(crate) fn preserve_verbatim(&self) -> bool {
        self.preserve_verbatim
    }

    pub(crate) fn contains_heredoc(&self) -> bool {
        self.contains_heredoc
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SequenceFacts<'source> {
    comments: SequenceCommentAttachment<'source>,
    first_rendered_lines: Vec<usize>,
    group_open_suffix_span: Option<Span>,
    group_attachment_span: Option<Span>,
    open_end_offset: Option<usize>,
    has_blank_line_after_open: bool,
    has_blank_line_before_close: bool,
    body_content_end: usize,
    close_gap_start: usize,
    contains_comments: bool,
    contains_heredoc: bool,
    contains_multiline_literal_source: bool,
    contains_multistatement_pipeline_brace_group: bool,
}

impl<'source> SequenceFacts<'source> {
    fn new(child_count: usize) -> Self {
        Self {
            comments: SequenceCommentAttachment::new(child_count),
            first_rendered_lines: vec![0; child_count],
            group_open_suffix_span: None,
            group_attachment_span: None,
            open_end_offset: None,
            has_blank_line_after_open: false,
            has_blank_line_before_close: false,
            body_content_end: 0,
            close_gap_start: 0,
            contains_comments: false,
            contains_heredoc: false,
            contains_multiline_literal_source: false,
            contains_multistatement_pipeline_brace_group: false,
        }
    }

    pub(crate) fn leading_for(&self, index: usize) -> &[SourceComment<'source>] {
        self.comments.leading_for(index)
    }

    pub(crate) fn trailing_for(&self, index: usize) -> &[SourceComment<'source>] {
        self.comments.trailing_for(index)
    }

    pub(crate) fn dangling(&self) -> &[SourceComment<'source>] {
        self.comments.dangling()
    }

    pub(crate) fn is_ambiguous(&self) -> bool {
        self.comments.is_ambiguous()
    }

    pub(crate) fn has_comments(&self) -> bool {
        self.comments.has_comments()
    }

    pub(crate) fn contains_comments(&self) -> bool {
        self.contains_comments
    }

    pub(crate) fn contains_heredoc(&self) -> bool {
        self.contains_heredoc
    }

    pub(crate) fn contains_multiline_literal_source(&self) -> bool {
        self.contains_multiline_literal_source
    }

    pub(crate) fn contains_multistatement_pipeline_brace_group(&self) -> bool {
        self.contains_multistatement_pipeline_brace_group
    }

    pub(crate) fn first_rendered_line_for(&self, index: usize) -> usize {
        self.first_rendered_lines
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn group_open_suffix_span(&self) -> Option<Span> {
        self.group_open_suffix_span
    }

    pub(crate) fn group_attachment_span(&self) -> Option<Span> {
        self.group_attachment_span
    }

    pub(crate) fn open_end_offset(&self) -> Option<usize> {
        self.open_end_offset
    }

    pub(crate) fn has_blank_line_after_open(&self) -> bool {
        self.has_blank_line_after_open
    }

    pub(crate) fn has_blank_line_before_close(&self) -> bool {
        self.has_blank_line_before_close
    }

    pub(crate) fn body_content_end(&self) -> usize {
        self.body_content_end
    }

    pub(crate) fn close_gap_start(&self) -> usize {
        self.close_gap_start
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WordFacts {
    has_multiline_literal_source: bool,
}

impl WordFacts {
    pub(crate) fn has_multiline_literal_source(&self) -> bool {
        self.has_multiline_literal_source
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LayoutSummary {
    contains_comments: bool,
    contains_heredoc: bool,
    contains_multiline_literal_source: bool,
    contains_multistatement_pipeline_brace_group: bool,
}

impl LayoutSummary {
    fn merge(&mut self, other: Self) {
        self.contains_comments |= other.contains_comments;
        self.contains_heredoc |= other.contains_heredoc;
        self.contains_multiline_literal_source |= other.contains_multiline_literal_source;
        self.contains_multistatement_pipeline_brace_group |=
            other.contains_multistatement_pipeline_brace_group;
    }

    fn with_comments(mut self, contains_comments: bool) -> Self {
        self.contains_comments |= contains_comments;
        self
    }
}

#[derive(Debug, Default)]
struct LayoutAnnotations {
    sequences: HashMap<FactSpan, LayoutSummary>,
    statements: HashMap<FactSpan, LayoutSummary>,
    words: HashMap<FactSpan, LayoutSummary>,
}

impl LayoutAnnotations {
    fn build_for_sequence(source: &str, sequence: &StmtSeq) -> Self {
        let mut annotations = Self::default();
        {
            let mut pass = LayoutAnnotationPass::new(source, &mut annotations);
            pass.visit_stmt_seq(sequence);
        }
        annotations
    }

    fn build_for_stmt(source: &str, stmt: &Stmt) -> Self {
        let mut annotations = Self::default();
        {
            let mut pass = LayoutAnnotationPass::new(source, &mut annotations);
            pass.visit_stmt(stmt);
        }
        annotations
    }

    fn build_for_word(source: &str, word: &Word) -> Self {
        let mut annotations = Self::default();
        {
            let mut pass = LayoutAnnotationPass::new(source, &mut annotations);
            pass.visit_word(word);
        }
        annotations
    }

    fn sequence(&self, sequence: &StmtSeq) -> LayoutSummary {
        self.sequences
            .get(&FactSpan::from(sequence.span))
            .copied()
            .unwrap_or_default()
    }

    fn stmt(&self, stmt: &Stmt) -> LayoutSummary {
        self.statements
            .get(&FactSpan::from(stmt_span(stmt)))
            .copied()
            .unwrap_or_default()
    }

    fn word_summary(&self, word: &Word) -> LayoutSummary {
        self.words
            .get(&FactSpan::from(word.span))
            .copied()
            .unwrap_or_default()
    }

    fn word_facts(&self, word: &Word) -> WordFacts {
        WordFacts {
            has_multiline_literal_source: self.word_summary(word).contains_multiline_literal_source,
        }
    }
}

struct LayoutAnnotationPass<'source, 'annotations> {
    source: &'source str,
    annotations: &'annotations mut LayoutAnnotations,
}

impl<'source, 'annotations> LayoutAnnotationPass<'source, 'annotations> {
    fn new(source: &'source str, annotations: &'annotations mut LayoutAnnotations) -> Self {
        Self {
            source,
            annotations,
        }
    }

    fn command_layout(&self, command: &Command) -> LayoutSummary {
        match command {
            Command::Simple(command) => {
                let mut summary = self.assignments_layout(&command.assignments);
                summary.merge(self.annotations.word_summary(&command.name));
                for word in &command.args {
                    summary.merge(self.annotations.word_summary(word));
                }
                summary
            }
            Command::Builtin(command) => {
                let (_, _, assignments, primary, extra_args) = builtin_like_parts(command);
                let mut summary = self.assignments_layout(assignments);
                if let Some(primary) = primary {
                    summary.merge(self.annotations.word_summary(primary));
                }
                for word in extra_args {
                    summary.merge(self.annotations.word_summary(word));
                }
                summary
            }
            Command::Decl(command) => {
                let mut summary = self.assignments_layout(&command.assignments);
                for operand in &command.operands {
                    summary.merge(self.decl_operand_layout(operand));
                }
                summary
            }
            Command::Binary(command) => {
                let mut summary = self.annotations.stmt(&command.left);
                summary.merge(self.annotations.stmt(&command.right));
                summary.contains_multistatement_pipeline_brace_group =
                    self.command_contains_multistatement_pipeline_brace_group(command, false);
                summary
            }
            Command::Compound(command) => self.compound_command_layout(command),
            Command::Function(function) => {
                let mut summary = self.annotations.stmt(function.body.as_ref());
                for entry in &function.header.entries {
                    summary.merge(self.annotations.word_summary(&entry.word));
                }
                summary
            }
            Command::AnonymousFunction(function) => {
                let mut summary = self.annotations.stmt(function.body.as_ref());
                for argument in &function.args {
                    summary.merge(self.annotations.word_summary(argument));
                }
                summary
            }
        }
    }

    fn compound_command_layout(&self, command: &CompoundCommand) -> LayoutSummary {
        match command {
            CompoundCommand::If(command) => {
                let mut summary = self.annotations.sequence(&command.condition);
                summary.merge(self.annotations.sequence(&command.then_branch));
                for (condition, body) in &command.elif_branches {
                    summary.merge(self.annotations.sequence(condition));
                    summary.merge(self.annotations.sequence(body));
                }
                if let Some(body) = &command.else_branch {
                    summary.merge(self.annotations.sequence(body));
                }
                summary
            }
            CompoundCommand::For(command) => {
                let mut summary = LayoutSummary::default();
                for target in &command.targets {
                    summary.merge(self.annotations.word_summary(&target.word));
                }
                if let Some(words) = &command.words {
                    for word in words {
                        summary.merge(self.annotations.word_summary(word));
                    }
                }
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::Repeat(command) => {
                let mut summary = self.annotations.word_summary(&command.count);
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::Foreach(command) => {
                let mut summary = LayoutSummary::default();
                for word in &command.words {
                    summary.merge(self.annotations.word_summary(word));
                }
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::ArithmeticFor(command) => {
                let mut summary = LayoutSummary::default();
                if let Some(expr) = &command.init_ast {
                    summary.merge(self.arithmetic_expr_layout(expr));
                }
                if let Some(expr) = &command.condition_ast {
                    summary.merge(self.arithmetic_expr_layout(expr));
                }
                if let Some(expr) = &command.step_ast {
                    summary.merge(self.arithmetic_expr_layout(expr));
                }
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::While(command) => {
                let mut summary = self.annotations.sequence(&command.condition);
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::Until(command) => {
                let mut summary = self.annotations.sequence(&command.condition);
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::Case(command) => {
                let mut summary = self.annotations.word_summary(&command.word);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        summary.merge(self.pattern_layout(pattern));
                    }
                    summary.merge(self.annotations.sequence(&item.body));
                }
                summary
            }
            CompoundCommand::Select(command) => {
                let mut summary = LayoutSummary::default();
                for word in &command.words {
                    summary.merge(self.annotations.word_summary(word));
                }
                summary.merge(self.annotations.sequence(&command.body));
                summary
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                self.annotations.sequence(body)
            }
            CompoundCommand::Arithmetic(command) => command
                .expr_ast
                .as_ref()
                .map_or_else(LayoutSummary::default, |expr| {
                    self.arithmetic_expr_layout(expr)
                }),
            CompoundCommand::Time(command) => command
                .command
                .as_ref()
                .map_or_else(LayoutSummary::default, |command| {
                    self.annotations.stmt(command)
                }),
            CompoundCommand::Conditional(command) => {
                self.conditional_expr_layout(&command.expression)
            }
            CompoundCommand::Coproc(command) => self.annotations.stmt(&command.body),
            CompoundCommand::Always(command) => {
                let mut summary = self.annotations.sequence(&command.body);
                summary.merge(self.annotations.sequence(&command.always_body));
                summary
            }
        }
    }

    fn decl_operand_layout(&self, operand: &DeclOperand) -> LayoutSummary {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                self.annotations.word_summary(word)
            }
            DeclOperand::Name(reference) => self.var_ref_layout(reference),
            DeclOperand::Assignment(assignment) => self.assignment_layout(assignment),
        }
    }

    fn assignments_layout(&self, assignments: &[Assignment]) -> LayoutSummary {
        let mut summary = LayoutSummary::default();
        for assignment in assignments {
            summary.merge(self.assignment_layout(assignment));
        }
        summary
    }

    fn assignment_layout(&self, assignment: &Assignment) -> LayoutSummary {
        let mut summary = self.var_ref_layout(&assignment.target);
        match &assignment.value {
            AssignmentValue::Scalar(word) => summary.merge(self.annotations.word_summary(word)),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    if let Some(key) = array_elem_parts(element).0 {
                        summary.merge(self.subscript_layout(key));
                    }
                    summary.merge(self.annotations.word_summary(array_elem_parts(element).1));
                }
            }
        }
        summary.contains_multiline_literal_source =
            self.assignment_has_multiline_literal_source(assignment);
        summary
    }

    fn redirect_layout(&self, redirect: &Redirect) -> LayoutSummary {
        let mut summary = match &redirect.target {
            RedirectTarget::Word(word) => self.annotations.word_summary(word),
            RedirectTarget::Heredoc(heredoc) => {
                let mut summary = self.annotations.word_summary(&heredoc.delimiter.raw);
                summary.merge(self.heredoc_body_layout(&heredoc.body));
                summary
            }
        };
        summary.contains_heredoc |= matches!(
            redirect.kind,
            RedirectKind::HereDoc | RedirectKind::HereDocStrip
        );
        summary.contains_multiline_literal_source =
            self.redirect_has_multiline_literal_source(redirect);
        summary
    }

    fn heredoc_body_layout(&self, body: &HeredocBody) -> LayoutSummary {
        let mut summary = LayoutSummary::default();
        for part in &body.parts {
            summary.merge(self.heredoc_body_part_layout(&part.kind));
        }
        summary
    }

    fn heredoc_body_part_layout(&self, part: &HeredocBodyPart) -> LayoutSummary {
        match part {
            HeredocBodyPart::CommandSubstitution { body, .. } => self.annotations.sequence(body),
            HeredocBodyPart::ArithmeticExpansion {
                expression_ast: Some(expr),
                ..
            } => self.arithmetic_expr_layout(expr),
            HeredocBodyPart::ArithmeticExpansion {
                expression_ast: None,
                expression_word_ast,
                ..
            } => self.annotations.word_summary(expression_word_ast),
            HeredocBodyPart::Parameter(parameter) => self.parameter_expansion_layout(parameter),
            HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => LayoutSummary::default(),
        }
    }

    fn word_layout(&self, word: &Word) -> LayoutSummary {
        let mut summary = LayoutSummary::default();
        for part in &word.parts {
            summary.merge(self.word_part_layout(&part.kind));
        }
        summary.contains_multiline_literal_source = self.word_has_multiline_literal_source(word);
        summary.contains_heredoc = false;
        summary.contains_multistatement_pipeline_brace_group = false;
        summary
    }

    fn word_part_layout(&self, part: &WordPart) -> LayoutSummary {
        match part {
            WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {
                LayoutSummary::default()
            }
            WordPart::ZshQualifiedGlob(glob) => {
                let mut summary = LayoutSummary::default();
                for segment in &glob.segments {
                    if let ZshGlobSegment::Pattern(pattern) = segment {
                        summary.merge(self.pattern_layout(pattern));
                    }
                }
                summary
            }
            WordPart::SingleQuoted { .. } => LayoutSummary::default(),
            WordPart::DoubleQuoted { parts, .. } => {
                let mut summary = LayoutSummary::default();
                for part in parts {
                    summary.merge(self.word_part_layout(&part.kind));
                }
                summary
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => self.annotations.sequence(body),
            WordPart::ArithmeticExpansion {
                expression_ast: Some(expr),
                ..
            } => self.arithmetic_expr_layout(expr),
            WordPart::ArithmeticExpansion {
                expression_ast: None,
                expression_word_ast,
                ..
            } => self.annotations.word_summary(expression_word_ast),
            WordPart::Parameter(parameter) => self.parameter_expansion_layout(parameter),
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                let mut summary = self.var_ref_layout(reference);
                summary.merge(self.parameter_op_layout(operator));
                if let Some(operand) = operand_word_ast {
                    summary.merge(self.annotations.word_summary(operand));
                }
                summary
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => self.var_ref_layout(reference),
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
                let mut summary = self.var_ref_layout(reference);
                if let Some(expression) = offset_ast {
                    summary.merge(self.arithmetic_expr_layout(expression));
                } else {
                    summary.merge(self.annotations.word_summary(offset_word_ast));
                }
                if let Some(expression) = length_ast {
                    summary.merge(self.arithmetic_expr_layout(expression));
                } else if let Some(word) = length_word_ast {
                    summary.merge(self.annotations.word_summary(word));
                }
                summary
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                let mut summary = self.var_ref_layout(reference);
                if let Some(operator) = operator {
                    summary.merge(self.parameter_op_layout(operator));
                }
                if let Some(operand) = operand_word_ast {
                    summary.merge(self.annotations.word_summary(operand));
                }
                summary
            }
        }
    }

    fn parameter_expansion_layout(&self, parameter: &ParameterExpansion) -> LayoutSummary {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => {
                self.bourne_parameter_expansion_layout(syntax)
            }
            ParameterExpansionSyntax::Zsh(syntax) => {
                let mut summary = match &syntax.target {
                    ZshExpansionTarget::Reference(reference) => self.var_ref_layout(reference),
                    ZshExpansionTarget::Nested(parameter) => {
                        self.parameter_expansion_layout(parameter)
                    }
                    ZshExpansionTarget::Word(word) => self.annotations.word_summary(word),
                    ZshExpansionTarget::Empty => LayoutSummary::default(),
                };
                for modifier in &syntax.modifiers {
                    if let Some(word) = modifier.argument_word_ast() {
                        summary.merge(self.annotations.word_summary(word));
                    }
                }
                if let Some(operation) = &syntax.operation {
                    summary.merge(self.zsh_expansion_operation_layout(operation));
                }
                summary
            }
        }
    }

    fn bourne_parameter_expansion_layout(
        &self,
        syntax: &BourneParameterExpansion,
    ) -> LayoutSummary {
        match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                self.var_ref_layout(reference)
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                let mut summary = self.var_ref_layout(reference);
                if let Some(operator) = operator {
                    summary.merge(self.parameter_op_layout(operator));
                }
                if let Some(operand) = operand_word_ast {
                    summary.merge(self.annotations.word_summary(operand));
                }
                summary
            }
            BourneParameterExpansion::PrefixMatch { .. } => LayoutSummary::default(),
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                let mut summary = self.var_ref_layout(reference);
                if let Some(expression) = offset_ast {
                    summary.merge(self.arithmetic_expr_layout(expression));
                } else {
                    summary.merge(self.annotations.word_summary(offset_word_ast));
                }
                if let Some(expression) = length_ast {
                    summary.merge(self.arithmetic_expr_layout(expression));
                } else if let Some(word) = length_word_ast {
                    summary.merge(self.annotations.word_summary(word));
                }
                summary
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                let mut summary = self.var_ref_layout(reference);
                summary.merge(self.parameter_op_layout(operator));
                if let Some(operand) = operand_word_ast {
                    summary.merge(self.annotations.word_summary(operand));
                }
                summary
            }
        }
    }

    fn parameter_op_layout(&self, operator: &ParameterOp) -> LayoutSummary {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => self.pattern_layout(pattern),
            ParameterOp::ReplaceFirst {
                pattern,
                replacement_word_ast,
                ..
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement_word_ast,
                ..
            } => {
                let mut summary = self.pattern_layout(pattern);
                summary.merge(self.annotations.word_summary(replacement_word_ast));
                summary
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => LayoutSummary::default(),
        }
    }

    fn zsh_expansion_operation_layout(&self, operation: &ZshExpansionOperation) -> LayoutSummary {
        match operation {
            ZshExpansionOperation::PatternOperation {
                operand_word_ast, ..
            }
            | ZshExpansionOperation::Defaulting {
                operand_word_ast, ..
            }
            | ZshExpansionOperation::TrimOperation {
                operand_word_ast, ..
            } => self.annotations.word_summary(operand_word_ast),
            ZshExpansionOperation::ReplacementOperation {
                pattern_word_ast,
                replacement_word_ast,
                ..
            } => {
                let mut summary = self.annotations.word_summary(pattern_word_ast);
                if let Some(replacement) = replacement_word_ast {
                    summary.merge(self.annotations.word_summary(replacement));
                }
                summary
            }
            ZshExpansionOperation::Slice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                let mut summary = self.annotations.word_summary(offset_word_ast);
                if let Some(length) = length_word_ast {
                    summary.merge(self.annotations.word_summary(length));
                }
                summary
            }
            ZshExpansionOperation::Unknown { word_ast, .. } => {
                self.annotations.word_summary(word_ast)
            }
        }
    }

    fn conditional_expr_layout(&self, expression: &ConditionalExpr) -> LayoutSummary {
        match expression {
            ConditionalExpr::Binary(expression) => {
                let mut summary = self.conditional_expr_layout(&expression.left);
                summary.merge(self.conditional_expr_layout(&expression.right));
                summary
            }
            ConditionalExpr::Unary(expression) => self.conditional_expr_layout(&expression.expr),
            ConditionalExpr::Parenthesized(expression) => {
                self.conditional_expr_layout(&expression.expr)
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.annotations.word_summary(word)
            }
            ConditionalExpr::Pattern(pattern) => self.pattern_layout(pattern),
            ConditionalExpr::VarRef(reference) => self.var_ref_layout(reference),
        }
    }

    fn pattern_layout(&self, pattern: &Pattern) -> LayoutSummary {
        let mut summary = LayoutSummary::default();
        for part in &pattern.parts {
            summary.merge(self.pattern_part_layout(&part.kind));
        }
        summary
    }

    fn pattern_part_layout(&self, part: &PatternPart) -> LayoutSummary {
        match part {
            PatternPart::Group { patterns, .. } => {
                let mut summary = LayoutSummary::default();
                for pattern in patterns {
                    summary.merge(self.pattern_layout(pattern));
                }
                summary
            }
            PatternPart::Word(word) => self.annotations.word_summary(word),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => LayoutSummary::default(),
        }
    }

    fn arithmetic_expr_layout(&self, expression: &ArithmeticExprNode) -> LayoutSummary {
        match &expression.kind {
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => LayoutSummary::default(),
            ArithmeticExpr::Indexed { index, .. } => self.arithmetic_expr_layout(index),
            ArithmeticExpr::ShellWord(word) => self.annotations.word_summary(word),
            ArithmeticExpr::Parenthesized { expression } => self.arithmetic_expr_layout(expression),
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                self.arithmetic_expr_layout(expr)
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                let mut summary = self.arithmetic_expr_layout(left);
                summary.merge(self.arithmetic_expr_layout(right));
                summary
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let mut summary = self.arithmetic_expr_layout(condition);
                summary.merge(self.arithmetic_expr_layout(then_expr));
                summary.merge(self.arithmetic_expr_layout(else_expr));
                summary
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                let mut summary = self.arithmetic_lvalue_layout(target);
                summary.merge(self.arithmetic_expr_layout(value));
                summary
            }
        }
    }

    fn arithmetic_lvalue_layout(&self, target: &ArithmeticLvalue) -> LayoutSummary {
        match target {
            ArithmeticLvalue::Variable(_) => LayoutSummary::default(),
            ArithmeticLvalue::Indexed { index, .. } => self.arithmetic_expr_layout(index),
        }
    }

    fn var_ref_layout(&self, reference: &VarRef) -> LayoutSummary {
        reference
            .subscript
            .as_deref()
            .map_or_else(LayoutSummary::default, |subscript| {
                self.subscript_layout(subscript)
            })
    }

    fn subscript_layout(&self, subscript: &Subscript) -> LayoutSummary {
        let mut summary = subscript
            .word_ast
            .as_ref()
            .map_or_else(LayoutSummary::default, |word| {
                self.annotations.word_summary(word)
            });
        if let Some(expression) = &subscript.arithmetic_ast {
            summary.merge(self.arithmetic_expr_layout(expression));
        }
        summary
    }

    fn word_has_multiline_literal_source(&self, word: &Word) -> bool {
        if raw_word_source_slice(word, self.source).is_some_and(|raw| {
            raw.contains("\\\n")
                && word_has_multiline_double_quoted_source(word, self.source)
                && !word_is_quoted_command_substitution_only(word)
        }) {
            return true;
        }

        word_part_nodes_any(&word.parts, &mut |part| {
            self.word_part_has_multiline_literal_source(&part.kind, part.span)
        })
    }

    fn word_part_has_multiline_literal_source(&self, part: &WordPart, span: Span) -> bool {
        match part {
            WordPart::Literal(text) => text.as_str(self.source, span).contains('\n'),
            WordPart::SingleQuoted { value, dollar } => {
                if *dollar {
                    raw_source_slice(span, self.source).is_some_and(|raw| raw.contains('\n'))
                } else {
                    value.slice(self.source).contains('\n')
                }
            }
            WordPart::CommandSubstitution { body, .. } => {
                self.annotations
                    .sequence(body)
                    .contains_multiline_literal_source
                    || (self.annotations.sequence(body).contains_comments
                        && raw_source_slice(span, self.source).is_some_and(|raw| {
                            raw.contains('\n')
                                && !command_substitution_source_starts_with_body_line(raw)
                        }))
            }
            WordPart::ProcessSubstitution { body, .. } => {
                self.annotations
                    .sequence(body)
                    .contains_multiline_literal_source
                    || (self.annotations.sequence(body).contains_comments
                        && raw_source_slice(span, self.source)
                            .is_some_and(|raw| raw.contains('\n')))
            }
            _ => false,
        }
    }

    fn redirect_has_multiline_literal_source(&self, redirect: &Redirect) -> bool {
        redirect.word_target().is_some_and(|word| {
            self.annotations
                .word_facts(word)
                .has_multiline_literal_source()
        }) || redirect.heredoc().is_some_and(|heredoc| {
            self.annotations
                .word_facts(&heredoc.delimiter.raw)
                .has_multiline_literal_source()
        })
    }

    fn assignment_has_multiline_literal_source(&self, assignment: &Assignment) -> bool {
        self.assignment_value_has_multiline_literal_source(assignment)
            || matches!(&assignment.value, AssignmentValue::Scalar(_))
                && assignment_has_raw_backslash_continuation_literal(assignment, self.source)
    }

    fn assignment_value_has_multiline_literal_source(&self, assignment: &Assignment) -> bool {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self
                .annotations
                .word_facts(word)
                .has_multiline_literal_source(),
            AssignmentValue::Compound(array) => array.elements.iter().any(|element| {
                self.annotations
                    .word_facts(array_elem_parts(element).1)
                    .has_multiline_literal_source()
            }),
        }
    }

    fn command_contains_multistatement_pipeline_brace_group(
        &self,
        command: &BinaryCommand,
        in_pipeline: bool,
    ) -> bool {
        let in_pipeline = in_pipeline || matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll);
        self.stmt_contains_multistatement_pipeline_brace_group(&command.left, in_pipeline)
            || self.stmt_contains_multistatement_pipeline_brace_group(&command.right, in_pipeline)
    }

    fn stmt_contains_multistatement_pipeline_brace_group(
        &self,
        stmt: &Stmt,
        in_pipeline: bool,
    ) -> bool {
        match &stmt.command {
            Command::Binary(command)
                if matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) =>
            {
                self.command_contains_multistatement_pipeline_brace_group(command, in_pipeline)
            }
            Command::Compound(CompoundCommand::BraceGroup(body)) if in_pipeline => body.len() > 1,
            _ => false,
        }
    }
}

impl AstVisitor for LayoutAnnotationPass<'_, '_> {
    fn visit_stmt_seq(&mut self, sequence: &StmtSeq) {
        let key = FactSpan::from(sequence.span);
        if self.annotations.sequences.contains_key(&key) {
            return;
        }

        visit::walk_stmt_seq(self, sequence);

        let mut summary = LayoutSummary::default().with_comments(
            !sequence.leading_comments.is_empty() || !sequence.trailing_comments.is_empty(),
        );
        for stmt in sequence.iter() {
            summary.merge(self.annotations.stmt(stmt));
        }
        self.annotations.sequences.insert(key, summary);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        let key = FactSpan::from(stmt_span(stmt));
        if self.annotations.statements.contains_key(&key) {
            return;
        }

        visit::walk_stmt(self, stmt);

        let mut summary = self
            .command_layout(&stmt.command)
            .with_comments(!stmt.leading_comments.is_empty() || stmt.inline_comment.is_some());
        for redirect in &stmt.redirects {
            summary.merge(self.redirect_layout(redirect));
        }
        self.annotations.statements.insert(key, summary);
    }

    fn visit_word(&mut self, word: &Word) {
        let key = FactSpan::from(word.span);
        if self.annotations.words.contains_key(&key) {
            return;
        }

        visit::walk_word(self, word);

        let summary = self.word_layout(word);
        self.annotations.words.insert(key, summary);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FormatterFacts<'source> {
    source_map: SourceMap<'source>,
    stmt_facts: HashMap<FactSpan, StmtFacts>,
    sequence_facts: HashMap<SequenceSiteKey, SequenceFacts<'source>>,
    word_facts: HashMap<FactSpan, WordFacts>,
    pipeline_breaks: HashSet<FactSpan>,
    list_item_breaks: HashSet<FactSpan>,
    background_breaks: HashSet<FactSpan>,
    inline_group_sequences: HashSet<FactSpan>,
    inline_case_item_bodies: HashSet<FactSpan>,
    branch_prefix_facts: HashMap<OffsetRegionKey, BranchPrefixFacts>,
    close_suffix_comments: HashMap<FactSpan, SourceComment<'source>>,
    case_facts: HashMap<FactSpan, CaseCommandFacts>,
    case_item_facts: HashMap<FactSpan, CaseItemFacts<'source>>,
    indexer: Indexer,
}

impl<'source> FormatterFacts<'source> {
    pub(crate) fn build(
        source: &'source str,
        file: &File,
        options: &ResolvedShellFormatOptions,
    ) -> Self {
        let indexer = Indexer::for_file_with_options(
            source,
            file,
            IndexerOptions::new().with_source_layout_indexes(true),
        );
        FormatterFactsBuilder::new(source, options, indexer).build(file)
    }

    pub(crate) fn source_map(&self) -> &SourceMap<'source> {
        &self.source_map
    }

    pub(crate) fn stmt(&self, stmt: &Stmt) -> &StmtFacts {
        let Some(facts) = self.stmt_facts.get(&FactSpan::from(stmt_span(stmt))) else {
            unreachable!("missing statement facts");
        };
        facts
    }

    pub(crate) fn sequence(
        &self,
        sequence: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> &SequenceFacts<'source> {
        let key = SequenceSiteKey::new(sequence, upper_bound);
        self.sequence_facts
            .get(&key)
            .unwrap_or_else(|| self.sequence_by_span(key.span))
    }

    fn sequence_by_span(&self, span: FactSpan) -> &SequenceFacts<'source> {
        let Some(facts) = self
            .sequence_facts
            .iter()
            .find_map(|(candidate, facts)| (candidate.span == span).then_some(facts))
        else {
            unreachable!("missing sequence facts");
        };
        facts
    }

    pub(crate) fn word_has_multiline_literal_source(&self, word: &Word) -> bool {
        self.word_facts.get(&FactSpan::from(word.span)).map_or_else(
            || classify_word_has_multiline_literal_source(word, self.source_map.source()),
            WordFacts::has_multiline_literal_source,
        )
    }

    pub(crate) fn assignment_value_has_multiline_literal_source(
        &self,
        assignment: &Assignment,
    ) -> bool {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.word_has_multiline_literal_source(word),
            AssignmentValue::Compound(array) => array
                .elements
                .iter()
                .any(|element| self.word_has_multiline_literal_source(array_elem_parts(element).1)),
        }
    }

    pub(crate) fn assignment_has_multiline_literal_source(
        &self,
        assignment: &Assignment,
        source: &str,
    ) -> bool {
        self.assignment_value_has_multiline_literal_source(assignment)
            || matches!(&assignment.value, AssignmentValue::Scalar(_))
                && assignment_has_raw_backslash_continuation_literal(assignment, source)
    }

    pub(crate) fn sequence_contains_comments(&self, sequence: &StmtSeq) -> bool {
        self.sequence_by_span(FactSpan::from(sequence.span))
            .contains_comments()
    }

    pub(crate) fn sequence_contains_heredoc(&self, sequence: &StmtSeq) -> bool {
        self.sequence_by_span(FactSpan::from(sequence.span))
            .contains_heredoc()
    }

    pub(crate) fn sequence_contains_multiline_literal_source(&self, sequence: &StmtSeq) -> bool {
        self.sequence_by_span(FactSpan::from(sequence.span))
            .contains_multiline_literal_source()
    }

    pub(crate) fn sequence_contains_multistatement_pipeline_brace_group(
        &self,
        sequence: &StmtSeq,
    ) -> bool {
        self.sequence_by_span(FactSpan::from(sequence.span))
            .contains_multistatement_pipeline_brace_group()
    }

    pub(crate) fn pipeline_has_explicit_line_break(&self, pipeline: &BinaryCommand) -> bool {
        self.pipeline_breaks
            .contains(&FactSpan::from(pipeline.span))
    }

    pub(crate) fn list_item_has_explicit_line_break(&self, operator_span: Span) -> bool {
        self.list_item_breaks
            .contains(&FactSpan::from(operator_span))
    }

    pub(crate) fn background_has_explicit_line_break(&self, stmt: &Stmt) -> bool {
        stmt.terminator_span
            .map(FactSpan::from)
            .or_else(|| {
                matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
                    .then_some(FactSpan::from(stmt_span(stmt)))
            })
            .is_some_and(|key| self.background_breaks.contains(&key))
    }

    pub(crate) fn stmt_contains_heredoc(&self, stmt: &Stmt) -> bool {
        self.stmt(stmt).contains_heredoc()
    }

    pub(crate) fn group_was_inline_in_source(&self, commands: &StmtSeq) -> bool {
        self.inline_group_sequences
            .contains(&FactSpan::from(commands.span))
    }

    pub(crate) fn case_item_was_inline_in_source(&self, item: &CaseItem) -> bool {
        self.inline_case_item_bodies
            .contains(&FactSpan::from(item.body.span))
    }

    pub(crate) fn close_suffix_comment_after_span(
        &self,
        span: Span,
    ) -> Option<SourceComment<'source>> {
        self.close_suffix_comments
            .get(&FactSpan::from(span))
            .copied()
            .or_else(|| self.source_map.suffix_comment_after_span(span))
    }

    pub(crate) fn branch_prefix_facts(&self, start: usize, end: usize) -> BranchPrefixFacts {
        let key = OffsetRegionKey::new(start, end);
        self.branch_prefix_facts
            .get(&key)
            .cloned()
            .unwrap_or_else(|| {
                BranchPrefixFacts::new(
                    self.source_map.source(),
                    start,
                    end,
                    self.branch_prefix_comments_from_source(start, end),
                )
            })
    }

    pub(crate) fn if_next_branch_region(
        &self,
        command: &IfCommand,
        branch_index: usize,
    ) -> Option<(usize, usize)> {
        if_next_branch_region_with_body_end(
            command,
            branch_index,
            self.source_map.source(),
            |body| self.sequence(body, None).body_content_end(),
        )
    }

    pub(crate) fn if_branch_upper_bound(&self, command: &IfCommand, branch_index: usize) -> usize {
        if let Some((start, end)) = self.if_next_branch_region(command, branch_index) {
            self.branch_prefix_facts(start, end)
                .first_comment_offset()
                .unwrap_or(end)
        } else {
            if_close_span(command, self.source_map.source(), &self.source_map)
                .start
                .offset
        }
    }

    pub(crate) fn case_command(&self, command: &CaseCommand) -> &CaseCommandFacts {
        self.case_facts
            .get(&FactSpan::from(command.span))
            .unwrap_or_else(|| unreachable!("missing case command facts"))
    }

    pub(crate) fn case_item(&self, item: &CaseItem) -> &CaseItemFacts<'source> {
        self.case_item_facts
            .get(&case_item_key(item))
            .unwrap_or_else(|| unreachable!("missing case item facts"))
    }

    pub(crate) fn offset_is_in_heredoc_body(&self, offset: usize) -> bool {
        self.indexer
            .region_index()
            .is_heredoc(TextSize::new(offset as u32))
    }

    pub(crate) fn line_ending(&self) -> LineEnding {
        match self.indexer.line_index().line_ending() {
            shuck_indexer::LineEndingStyle::Lf => LineEnding::Lf,
            shuck_indexer::LineEndingStyle::CrLf => LineEnding::CrLf,
        }
    }

    pub(crate) fn contains_newline_between(&self, start: usize, end: usize) -> bool {
        self.source_map.contains_newline_between(start, end)
    }

    pub(crate) fn has_continuation_line_start_between(&self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        let start = TextSize::new(start as u32);
        let end = TextSize::new(end as u32);
        self.indexer
            .continuation_line_starts()
            .iter()
            .copied()
            .any(|line_start| start < line_start && line_start <= end)
    }

    pub(crate) fn has_raw_continuation_backslash_between(&self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        let start = TextSize::new(start as u32);
        let end = TextSize::new(end as u32);
        self.indexer
            .line_index()
            .raw_continuation_backslashes()
            .iter()
            .copied()
            .any(|backslash| start <= backslash && backslash < end)
    }

    pub(crate) fn branch_prefix_first_comment_offset(
        &self,
        start: usize,
        end: usize,
    ) -> Option<usize> {
        self.branch_prefix_facts(start, end).first_comment_offset()
    }

    fn branch_prefix_comments_from_source(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<BranchPrefixComment> {
        branch_prefix_comments_from_index(
            self.source_map.source(),
            self.indexer.line_index(),
            self.indexer.comment_index(),
            start,
            end,
        )
        .into_iter()
        .filter(|comment| !self.offset_is_in_heredoc_body(comment.offset))
        .collect()
    }

    pub(crate) fn own_line_comments_in_region(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<BranchPrefixComment> {
        own_line_comments_in_region_from_index(
            self.source_map.source(),
            self.indexer.line_index(),
            self.indexer.comment_index(),
            start,
            end,
        )
        .into_iter()
        .filter(|comment| !self.offset_is_in_heredoc_body(comment.offset))
        .collect()
    }

    pub(crate) fn heredoc_closing_marker_bounds(
        &self,
        heredoc: &Heredoc,
    ) -> Option<(usize, usize)> {
        self.indexer
            .region_index()
            .heredoc_closing_marker_range(heredoc.body.span.to_range())
            .map(|range| (usize::from(range.start()), usize::from(range.end())))
    }

    #[cfg(feature = "benchmarking")]
    pub(crate) fn len(&self) -> usize {
        self.stmt_facts.len()
            + self.sequence_facts.len()
            + self.word_facts.len()
            + self.pipeline_breaks.len()
            + self.list_item_breaks.len()
            + self.background_breaks.len()
            + self.inline_group_sequences.len()
            + self.inline_case_item_bodies.len()
            + self.branch_prefix_facts.len()
            + self.close_suffix_comments.len()
            + self.case_facts.len()
            + self.case_item_facts.len()
            + self.indexer.region_index().heredoc_ranges().len()
    }
}

pub(crate) fn classify_word_has_multiline_literal_source(word: &Word, source: &str) -> bool {
    LayoutAnnotations::build_for_word(source, word)
        .word_facts(word)
        .has_multiline_literal_source()
}

pub(crate) fn classify_sequence_contains_multiline_literal_source(
    sequence: &StmtSeq,
    source: &str,
) -> bool {
    LayoutAnnotations::build_for_sequence(source, sequence)
        .sequence(sequence)
        .contains_multiline_literal_source
}

pub(crate) fn classify_stmt_contains_heredoc(stmt: &Stmt) -> bool {
    LayoutAnnotations::build_for_stmt("", stmt)
        .stmt(stmt)
        .contains_heredoc
}

pub(crate) fn classify_sequence_contains_heredoc(sequence: &StmtSeq) -> bool {
    LayoutAnnotations::build_for_sequence("", sequence)
        .sequence(sequence)
        .contains_heredoc
}

fn word_part_nodes_any(
    parts: &[WordPartNode],
    predicate: &mut impl FnMut(&WordPartNode) -> bool,
) -> bool {
    parts.iter().any(|part| {
        predicate(part)
            || matches!(
                &part.kind,
                WordPart::DoubleQuoted { parts, .. }
                    if word_part_nodes_any(parts.as_slice(), predicate)
            )
    })
}

fn assignment_has_raw_backslash_continuation_literal(
    assignment: &Assignment,
    source: &str,
) -> bool {
    let raw = assignment.span.slice(source);
    raw.contains("\\\n")
        && !raw.contains("$(")
        && !raw.contains('`')
        && !raw.contains("<(")
        && !raw.contains(">(")
}

fn word_has_multiline_double_quoted_source(word: &Word, source: &str) -> bool {
    word_part_nodes_any(&word.parts, &mut |part| {
        matches!(&part.kind, WordPart::DoubleQuoted { .. })
            && raw_source_slice(part.span, source).is_some_and(|raw| raw.contains('\n'))
    })
}

fn word_is_quoted_command_substitution_only(word: &Word) -> bool {
    quoted_command_substitution_only_body(word).is_some()
}

fn quoted_command_substitution_only_body(word: &Word) -> Option<&StmtSeq> {
    let [
        shuck_ast::WordPartNode {
            kind:
                WordPart::DoubleQuoted {
                    parts,
                    dollar: false,
                },
            ..
        },
    ] = word.parts.as_slice()
    else {
        return None;
    };

    let mut substitution_body = None;
    for part in parts {
        match &part.kind {
            WordPart::CommandSubstitution { body, .. } if substitution_body.is_none() => {
                substitution_body = Some(body);
            }
            WordPart::Literal(text) if text.is_empty() => {}
            _ => return None,
        }
    }

    substitution_body
}

fn command_substitution_source_starts_with_body_line(raw: &str) -> bool {
    if raw.starts_with(['\n', '\r']) {
        return true;
    }
    raw.strip_prefix("$(")
        .is_some_and(|after_open| after_open.starts_with(['\n', '\r']))
}

fn raw_word_source_slice<'a>(word: &Word, source: &'a str) -> Option<&'a str> {
    raw_source_slice(word.span, source)
}

fn raw_source_slice(span: Span, source: &str) -> Option<&str> {
    if span.start.offset >= span.end.offset || span.end.offset > source.len() {
        return None;
    }

    let slice = span.slice(source);
    if slice.contains('\n') {
        Some(slice)
    } else {
        Some(trim_unescaped_trailing_whitespace(slice))
    }
}

fn line_indent_before_offset<'source>(
    source: &'source str,
    line_index: &LineIndex,
    offset: usize,
) -> Option<&'source str> {
    let offset = offset.min(source.len());
    let line = line_index.line_number(TextSize::new(offset as u32));
    let line_start = usize::from(line_index.line_start(line)?);
    let prefix = source.get(line_start..offset)?;
    let indent_end = prefix
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
        .map_or(prefix.len(), |(index, _)| index);
    prefix.get(..indent_end)
}

fn branch_prefix_comments_from_index(
    source: &str,
    line_index: &LineIndex,
    comment_index: &CommentIndex,
    start: usize,
    end: usize,
) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    if start >= end {
        return Vec::new();
    }

    let keyword_indent = line_indent_before_offset(source, line_index, end).unwrap_or("");
    let mut comments = Vec::new();
    let mut in_branch_prefix_run = false;
    let first_line = line_index.line_number(TextSize::new(start as u32));
    let last_line = line_index.line_number(TextSize::new(end.saturating_sub(1) as u32));

    for line in first_line..=last_line {
        let Some((line_start, line_end, text)) =
            clamped_line_text(source, line_index, line, start, end)
        else {
            continue;
        };
        let trimmed = text.trim_start_matches([' ', '\t']);
        let indent = text.len().saturating_sub(trimmed.len());
        let own_line_comment =
            own_line_comment_in_bounds(comment_index, line, line_start, line_end).is_some();
        if own_line_comment
            && trimmed.starts_with('#')
            && (in_branch_prefix_run || text.get(..indent) == Some(keyword_indent))
        {
            comments.push(BranchPrefixComment {
                offset: line_start + indent,
                text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
                source_indent: indent,
            });
            in_branch_prefix_run = true;
        } else if !trimmed.is_empty() {
            in_branch_prefix_run = false;
        }
    }

    comments
}

fn own_line_comments_in_region_from_index(
    source: &str,
    line_index: &LineIndex,
    comment_index: &CommentIndex,
    start: usize,
    end: usize,
) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let start_line = line_index.line_number(TextSize::new(start as u32));
    let Some(next_line_start) = line_index.line_start(start_line + 1).map(usize::from) else {
        return Vec::new();
    };
    if next_line_start >= end {
        return Vec::new();
    }

    let mut comments = Vec::new();
    let first_line = start_line + 1;
    let last_line = line_index.line_number(TextSize::new(end.saturating_sub(1) as u32));
    for line in first_line..=last_line {
        let Some((line_start, line_end, text)) =
            clamped_line_text(source, line_index, line, next_line_start, end)
        else {
            continue;
        };
        if own_line_comment_in_bounds(comment_index, line, line_start, line_end).is_none() {
            continue;
        }
        let trimmed = text.trim_start_matches([' ', '\t']);
        if !trimmed.starts_with('#') {
            continue;
        }
        let indent = text.len().saturating_sub(trimmed.len());
        comments.push(BranchPrefixComment {
            offset: line_start + indent,
            text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
            source_indent: indent,
        });
    }

    comments
}

fn clamped_line_text<'source>(
    source: &'source str,
    line_index: &LineIndex,
    line: usize,
    start: usize,
    end: usize,
) -> Option<(usize, usize, &'source str)> {
    let range: TextRange = line_index.line_range(line, source)?;
    let line_start = usize::from(range.start()).max(start);
    let line_end = usize::from(range.end()).min(end);
    (line_start <= line_end)
        .then(|| {
            source
                .get(line_start..line_end)
                .map(|text| (line_start, line_end, text))
        })
        .flatten()
}

fn own_line_comment_in_bounds(
    comment_index: &CommentIndex,
    line: usize,
    line_start: usize,
    line_end: usize,
) -> Option<&IndexedComment> {
    comment_index.comments_on_line(line).iter().find(|comment| {
        comment.is_own_line && {
            let start = usize::from(comment.range.start());
            line_start <= start && start < line_end
        }
    })
}

fn line_end_for_offset(source: &str, offset: usize) -> Option<usize> {
    let offset = offset.min(source.len());
    source.get(offset..).map(|suffix| {
        suffix
            .find('\n')
            .map_or(source.len(), |index| offset + index)
    })
}

struct FormatterFactsBuilder<'source, 'options> {
    source: &'source str,
    options: &'options ResolvedShellFormatOptions,
    facts: FormatterFacts<'source>,
    layout: LayoutAnnotations,
    comment_attachments: CommentAttachmentModel<'source>,
}

impl<'source, 'options> FormatterFactsBuilder<'source, 'options> {
    fn new(
        source: &'source str,
        options: &'options ResolvedShellFormatOptions,
        indexer: Indexer,
    ) -> Self {
        let source_map = SourceMap::from_indexer(source, &indexer, options.keep_padding());
        let comment_attachments = CommentAttachmentModel::from_indexer(&source_map, &indexer);

        Self {
            source,
            options,
            facts: FormatterFacts {
                source_map,
                stmt_facts: HashMap::default(),
                sequence_facts: HashMap::default(),
                word_facts: HashMap::default(),
                pipeline_breaks: HashSet::default(),
                list_item_breaks: HashSet::default(),
                background_breaks: HashSet::default(),
                inline_group_sequences: HashSet::default(),
                inline_case_item_bodies: HashSet::default(),
                branch_prefix_facts: HashMap::default(),
                close_suffix_comments: HashMap::default(),
                case_facts: HashMap::default(),
                case_item_facts: HashMap::default(),
                indexer,
            },
            layout: LayoutAnnotations::default(),
            comment_attachments,
        }
    }

    fn build(mut self, file: &File) -> FormatterFacts<'source> {
        self.visit_sequence(&file.body, None, None);
        self.facts
    }

    fn record_sequence_layout(&mut self, sequence: &StmtSeq) -> LayoutSummary {
        let key = FactSpan::from(sequence.span);
        if let Some(summary) = self.layout.sequences.get(&key).copied() {
            return summary;
        }

        let mut summary = LayoutSummary::default().with_comments(
            !sequence.leading_comments.is_empty() || !sequence.trailing_comments.is_empty(),
        );
        for stmt in sequence.iter() {
            summary.merge(self.layout.stmt(stmt));
        }
        self.layout.sequences.insert(key, summary);
        summary
    }

    fn record_stmt_layout(&mut self, stmt: &Stmt) -> LayoutSummary {
        let key = FactSpan::from(stmt_span(stmt));
        if let Some(summary) = self.layout.statements.get(&key).copied() {
            return summary;
        }

        let mut summary = {
            let reader = LayoutAnnotationPass::new(self.source, &mut self.layout);
            reader.command_layout(&stmt.command)
        }
        .with_comments(!stmt.leading_comments.is_empty() || stmt.inline_comment.is_some());
        for redirect in &stmt.redirects {
            let reader = LayoutAnnotationPass::new(self.source, &mut self.layout);
            summary.merge(reader.redirect_layout(redirect));
        }
        self.layout.statements.insert(key, summary);
        summary
    }

    fn record_word_layout(&mut self, word: &Word) -> LayoutSummary {
        let key = FactSpan::from(word.span);
        if let Some(summary) = self.layout.words.get(&key).copied() {
            return summary;
        }

        let reader = LayoutAnnotationPass::new(self.source, &mut self.layout);
        let summary = reader.word_layout(word);
        self.layout.words.insert(key, summary);
        summary
    }

    fn visit_sequence(
        &mut self,
        sequence: &StmtSeq,
        upper_bound: Option<usize>,
        group_open_char: Option<char>,
    ) {
        self.visit_sequence_with_suffix(sequence, upper_bound, group_open_char, None, None);
    }

    fn visit_compound_body_site(&mut self, site: CompoundBodySite<'_>) {
        if let Some(open) = site.group_open_char() {
            self.record_inline_group_sequence(site.body(), open, matching_group_close(open));
        }
        self.visit_sequence_with_suffix(
            site.body(),
            Some(site.facts_upper_bound()),
            site.group_open_char(),
            site.open_suffix_span(self.source_map()),
            site.open_end_offset(self.source),
        );
        self.record_close_suffix(site.close_span());
    }

    fn record_inline_group_sequence(&mut self, body: &StmtSeq, open: char, close: char) {
        if group_was_inline_in_source(body.as_slice(), self.source_map(), open, close) {
            self.facts
                .inline_group_sequences
                .insert(FactSpan::from(body.span));
        }
    }

    fn visit_sequence_with_suffix(
        &mut self,
        sequence: &StmtSeq,
        upper_bound: Option<usize>,
        group_open_char: Option<char>,
        open_suffix_span: Option<Span>,
        open_end_offset: Option<usize>,
    ) {
        let site = SequenceSite::new(
            sequence,
            upper_bound,
            group_open_char,
            open_suffix_span,
            open_end_offset,
        );
        for stmt in sequence.iter() {
            self.visit_stmt(stmt);
        }

        let key = site.key();
        if self.facts.sequence_facts.contains_key(&key) {
            return;
        }

        let mut facts = SequenceFacts::new(sequence.len());
        facts.group_open_suffix_span = site.open_suffix_span.or_else(|| {
            site.group_open_char.and_then(|open| {
                group_open_suffix(sequence.as_slice(), self.source_map(), open)
                    .map(|(span, _)| span)
            })
        });
        let layout = self.record_sequence_layout(sequence);
        facts.contains_comments = layout.contains_comments;
        facts.contains_heredoc = layout.contains_heredoc;
        facts.contains_multiline_literal_source = layout.contains_multiline_literal_source;
        facts.contains_multistatement_pipeline_brace_group =
            layout.contains_multistatement_pipeline_brace_group;
        let group_attachment_span = site.group_open_char.and_then(|open| {
            let close = match open {
                '{' => '}',
                '(' => ')',
                other => other,
            };
            group_attachment_span_with_heredoc(
                sequence.as_slice(),
                self.source_map(),
                open,
                close,
                |stmt| self.layout.stmt(stmt).contains_heredoc,
            )
        });
        facts.group_attachment_span = group_attachment_span;
        facts.body_content_end =
            sequence_body_content_end(sequence, self.source, &self.facts.indexer);
        facts.close_gap_start = sequence
            .trailing_comments
            .iter()
            .map(|comment| usize::from(comment.range.end()))
            .max()
            .unwrap_or(facts.body_content_end);
        facts.open_end_offset = if let Some(open) = site.group_open_char {
            facts
                .group_open_suffix_span
                .map(|span| span.end.offset)
                .or_else(|| {
                    facts
                        .group_attachment_span
                        .map(|span| span.start.offset.saturating_add(open.len_utf8()))
                })
        } else {
            site.open_end_offset
        };
        facts.has_blank_line_after_open = facts.open_end_offset.is_some_and(|offset| {
            body_has_blank_line_after_open(
                self.source,
                self.source_map(),
                offset,
                sequence,
                &self.layout,
            )
        });
        facts.has_blank_line_before_close = if let (Some(open), Some(span)) =
            (site.group_open_char, facts.group_attachment_span)
        {
            let close = matching_group_close(open);
            let close_offset =
                group_close_offset(self.source, span, site.upper_bound, close, close.len_utf8());
            self.source_map()
                .has_blank_line_immediately_before_offset(close_offset)
        } else {
            site.upper_bound.is_some_and(|offset| {
                self.source_map()
                    .has_blank_line_immediately_before_offset(offset)
            })
        };
        let sequence_limit = group_attachment_span
            .map(|span| span.end.offset)
            .or(site.upper_bound);

        let comment_lower_bound = sequence_comment_lower_bound(sequence, self.source_map());
        let lower_bound = group_attachment_span
            .map(|span| span.start.offset.min(comment_lower_bound))
            .unwrap_or(comment_lower_bound);

        if sequence.is_empty() {
            facts.comments = self.comment_attachments.attach_sequence(
                lower_bound,
                sequence_limit,
                facts.group_open_suffix_span,
                &[],
            );
        } else {
            let child_spans = sequence
                .iter()
                .map(|stmt| self.facts.stmt(stmt).attachment_span())
                .collect::<Vec<_>>();
            facts.comments = self.comment_attachments.attach_sequence(
                lower_bound,
                sequence_limit,
                facts.group_open_suffix_span,
                &child_spans,
            );

            for (index, stmt) in sequence.iter().enumerate() {
                facts.first_rendered_lines[index] = facts
                    .comments
                    .leading_for(index)
                    .first()
                    .map(SourceComment::line)
                    .unwrap_or(stmt_render_start_line(
                        stmt,
                        self.source,
                        self.source_map(),
                        self.options,
                    ));
            }
        }

        for window in sequence.as_slice().windows(2) {
            let [current, next] = window else {
                continue;
            };
            if !matches!(current.terminator, Some(StmtTerminator::Background(_))) {
                continue;
            }
            let break_key = current
                .terminator_span
                .map(FactSpan::from)
                .unwrap_or_else(|| FactSpan::from(stmt_span(current)));
            let break_start = current
                .terminator_span
                .map(|span| span.end.offset)
                .unwrap_or_else(|| stmt_span(current).end.offset);
            let next_start = self.facts.stmt(next).attachment_span().start.offset;
            if self.facts.contains_newline_between(break_start, next_start) {
                self.facts.background_breaks.insert(break_key);
            }
        }

        self.facts.sequence_facts.insert(key, facts);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        let site = StmtSite::new(stmt);
        let stmt = site.stmt;
        let already_recorded = self.facts.stmt_facts.contains_key(&site.key);

        for redirect in &stmt.redirects {
            self.visit_redirect(redirect);
        }

        if let Some((commands, open)) = command_group_commands(&stmt.command) {
            if group_was_inline_in_source(
                commands.as_slice(),
                self.source_map(),
                open,
                matching_group_close(open),
            ) {
                self.facts
                    .inline_group_sequences
                    .insert(FactSpan::from(commands.span));
            }
            self.visit_sequence(commands, Some(stmt_span(stmt).end.offset), Some(open));
        }

        self.visit_command(&stmt.command);

        if already_recorded {
            return;
        }

        let layout = self.record_stmt_layout(stmt);
        let contains_heredoc = layout.contains_heredoc;
        let preserve_verbatim = should_render_verbatim_with_heredoc(
            stmt,
            self.source_map(),
            self.options,
            contains_heredoc,
        );
        let render_span = if preserve_verbatim {
            stmt_verbatim_span_with_source_map(stmt, self.source_map())
        } else {
            stmt_format_span(stmt)
        };
        let stmt_contains_heredoc = |stmt: &Stmt| self.layout.stmt(stmt).contains_heredoc;
        let attachment_span = stmt_attachment_span_with_heredoc(
            stmt,
            self.source,
            self.source_map(),
            self.options,
            stmt_contains_heredoc,
        );
        let rendered_end_line = rendered_stmt_end_line_with_heredoc(
            stmt,
            self.source,
            self.source_map(),
            stmt_contains_heredoc,
        );
        self.facts.stmt_facts.insert(
            site.key,
            StmtFacts {
                attachment_span,
                render_span,
                rendered_end_line,
                has_trailing_comment: stmt_has_trailing_comment(stmt, self.source_map()),
                preserve_verbatim,
                contains_heredoc,
            },
        );
    }

    fn visit_command(&mut self, command: &Command) {
        if let Command::Binary(command) = command {
            self.visit_binary_command(command);
        } else {
            visit::walk_command(self, command);
        }
    }

    fn visit_binary_command(&mut self, command: &BinaryCommand) {
        self.visit_stmt(command.left.as_ref());
        self.visit_stmt(command.right.as_ref());

        if matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll)
            && pipeline_has_explicit_line_break(command, self.source, self.source_map())
        {
            self.facts
                .pipeline_breaks
                .insert(FactSpan::from(command.span));
        }

        if matches!(command.op, BinaryOp::And | BinaryOp::Or) {
            let mut rest = Vec::new();
            let mut previous = collect_command_list_first(command, &mut rest);
            for item in rest {
                let next_start = stmt_start_after_operator(
                    item.stmt,
                    item.operator_span.end.offset,
                    self.source,
                    self.source_map(),
                );
                let next_start_line = self.source_map().line_number_for_offset(next_start);
                let previous_span = stmt_span(previous);
                if self
                    .source_map()
                    .operator_starts_or_ends_line(item.operator_span)
                    || self
                        .facts
                        .contains_newline_between(item.operator_span.end.offset, next_start)
                    || (stmt_is_multiline_conditional(previous)
                        && previous_span.start.line < item.operator_span.start.line
                        && item.operator_span.end.line == next_start_line
                        && !stmt_can_follow_multiline_conditional_inline(item.stmt))
                {
                    self.facts
                        .list_item_breaks
                        .insert(FactSpan::from(item.operator_span));
                }
                previous = item.stmt;
            }
        }
    }

    fn visit_compound_command(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => self.visit_if(command),
            CompoundCommand::For(command) => self.visit_for(command),
            CompoundCommand::Repeat(command) => self.visit_repeat(command),
            CompoundCommand::Foreach(command) => {
                for word in &command.words {
                    self.visit_word(word);
                }
                let site = CompoundBodySite::foreach_command(command, self.source_map());
                self.visit_compound_body_site(site);
            }
            CompoundCommand::ArithmeticFor(command) => {
                if let Some(expr) = &command.init_ast {
                    self.visit_arithmetic_expr(expr);
                }
                if let Some(expr) = &command.condition_ast {
                    self.visit_arithmetic_expr(expr);
                }
                if let Some(expr) = &command.step_ast {
                    self.visit_arithmetic_expr(expr);
                }
                let site = CompoundBodySite::arithmetic_for_command(command, self.source_map());
                self.visit_compound_body_site(site);
            }
            CompoundCommand::While(command) => self.visit_while(command),
            CompoundCommand::Until(command) => self.visit_until(command),
            CompoundCommand::Case(command) => self.visit_case(command),
            CompoundCommand::Select(command) => self.visit_select(command),
            CompoundCommand::Subshell(body) => {
                self.record_inline_group_sequence(body, '(', ')');
                self.visit_sequence(body, None, Some('('));
            }
            CompoundCommand::BraceGroup(body) => {
                self.record_inline_group_sequence(body, '{', '}');
                self.visit_sequence(body, None, Some('{'));
            }
            CompoundCommand::Arithmetic(command) => {
                if let Some(expr) = &command.expr_ast {
                    self.visit_arithmetic_expr(expr);
                }
            }
            CompoundCommand::Time(command) => self.visit_time(command),
            CompoundCommand::Conditional(command) => self.visit_conditional(command),
            CompoundCommand::Coproc(command) => self.visit_stmt(command.body.as_ref()),
            CompoundCommand::Always(command) => {
                self.visit_sequence(&command.body, Some(command.span.end.offset), Some('{'));
                self.visit_sequence(
                    &command.always_body,
                    Some(command.span.end.offset),
                    Some('{'),
                );
                self.record_inline_group_sequence(&command.body, '{', '}');
                self.record_inline_group_sequence(&command.always_body, '{', '}');
            }
        }
    }

    fn visit_if(&mut self, command: &IfCommand) {
        let condition_upper_bound = match command.syntax {
            shuck_ast::IfSyntax::ThenFi { then_span, .. } => Some(then_span.start.offset),
            shuck_ast::IfSyntax::Brace {
                left_brace_span, ..
            } => Some(left_brace_span.start.offset),
        };
        self.visit_sequence(&command.condition, condition_upper_bound, None);
        let brace_syntax = matches!(command.syntax, shuck_ast::IfSyntax::Brace { .. });
        let then_upper_bound =
            if_branch_upper_bound(command, 0, self.source, self.source_map(), &self.facts);
        let then_site =
            CompoundBodySite::if_then_branch(command, &command.then_branch, then_upper_bound);
        self.visit_compound_body_site(then_site);
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            let body_upper_bound = if_branch_upper_bound(
                command,
                index + 1,
                self.source,
                self.source_map(),
                &self.facts,
            );
            let body_site = CompoundBodySite::if_then_branch(command, body, body_upper_bound);
            if brace_syntax {
                self.visit_compound_body_site(body_site);
            }
            let condition_upper_bound = if brace_syntax {
                group_attachment_span_with_heredoc(
                    body.as_slice(),
                    self.source_map(),
                    '{',
                    '}',
                    |stmt| self.layout.stmt(stmt).contains_heredoc,
                )
                .map(|span| span.start.offset)
            } else {
                body_site.open_keyword_start(self.source)
            };
            self.visit_sequence(condition, condition_upper_bound, None);
            if !brace_syntax {
                self.visit_compound_body_site(body_site);
            }
        }
        if let Some(else_branch) = &command.else_branch {
            let upper_bound = if_close_start(command, self.source_map());
            let site = CompoundBodySite::if_else_branch(command, else_branch, upper_bound);
            self.visit_compound_body_site(site);
        }
        self.record_if_branch_prefix_facts(command);
        self.record_close_suffix(Some(if_close_span(command, self.source, self.source_map())));
    }

    fn visit_for(&mut self, command: &ForCommand) {
        for target in &command.targets {
            self.visit_word(&target.word);
        }
        if let Some(words) = &command.words {
            for word in words {
                self.visit_word(word);
            }
        }
        let site = CompoundBodySite::for_command(command, self.source_map());
        self.visit_compound_body_site(site);
    }

    fn visit_repeat(&mut self, command: &RepeatCommand) {
        self.visit_word(&command.count);
        let site = CompoundBodySite::repeat_command(command, self.source_map());
        self.visit_compound_body_site(site);
    }

    fn visit_while(&mut self, command: &WhileCommand) {
        let site = CompoundBodySite::while_command(command, self.source_map());
        let condition_upper_bound = site.open_keyword_start(self.source);
        self.visit_sequence(&command.condition, condition_upper_bound, None);
        self.visit_compound_body_site(site);
    }

    fn visit_until(&mut self, command: &UntilCommand) {
        let site = CompoundBodySite::until_command(command, self.source_map());
        let condition_upper_bound = site.open_keyword_start(self.source);
        self.visit_sequence(&command.condition, condition_upper_bound, None);
        self.visit_compound_body_site(site);
    }

    fn visit_case(&mut self, command: &CaseCommand) {
        self.visit_word(&command.word);
        let case_command_facts = self.build_case_command_facts(command);
        self.record_close_suffix(case_command_facts.esac_span());
        let mut previous_item: Option<&CaseItem> = None;
        for item in &command.cases {
            for pattern in &item.patterns {
                self.visit_pattern(pattern);
            }
            if case_item_was_inline_in_source(item) {
                self.facts
                    .inline_case_item_bodies
                    .insert(FactSpan::from(item.body.span));
            }
            let upper_bound =
                case_item_body_upper_bound(item, case_command_facts.body_fallback_upper_bound());
            self.visit_sequence(&item.body, upper_bound, None);
            let item_facts = self.build_case_item_facts(item, previous_item, upper_bound);
            self.facts
                .case_item_facts
                .insert(case_item_key(item), item_facts);
            previous_item = Some(item);
        }
        self.facts
            .case_facts
            .insert(FactSpan::from(command.span), case_command_facts);
    }

    fn visit_select(&mut self, command: &SelectCommand) {
        for word in &command.words {
            self.visit_word(word);
        }
        let site = CompoundBodySite::select_command(command, self.source_map());
        self.visit_compound_body_site(site);
    }

    fn visit_time(&mut self, command: &TimeCommand) {
        if let Some(inner) = &command.command {
            self.visit_stmt(inner.as_ref());
            self.record_close_suffix(Some(stmt_format_span(inner.as_ref())));
        }
    }

    fn visit_conditional(&mut self, command: &ConditionalCommand) {
        self.visit_conditional_expr(&command.expression);
    }

    fn visit_function(&mut self, function: &FunctionDef) {
        for entry in &function.header.entries {
            self.visit_word(&entry.word);
        }

        self.visit_function_body(function.body.as_ref(), function.span.end.offset);
    }

    fn visit_anonymous_function(&mut self, function: &AnonymousFunctionCommand) {
        for argument in &function.args {
            self.visit_word(argument);
        }

        self.visit_function_body(function.body.as_ref(), function.span.end.offset);
    }

    fn visit_function_body(&mut self, body: &Stmt, function_end_offset: usize) {
        if let Some(site) = CompoundBodySite::function_group_body(body, function_end_offset) {
            self.visit_compound_body_site(site);
            self.record_stmt_layout(body);
        } else {
            self.visit_stmt(body);
        }
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        if let Some(word) = redirect.word_target() {
            self.visit_word(word);
        }
        if let Some(heredoc) = redirect.heredoc() {
            self.visit_word(&heredoc.delimiter.raw);
            self.visit_heredoc_body(&heredoc.body);
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        visit::walk_assignment(self, assignment);
    }

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr) {
        visit::walk_conditional_expr(self, expression);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        visit::walk_pattern(self, pattern);
    }

    fn visit_word(&mut self, word: &Word) {
        let word_key = FactSpan::from(word.span);
        if self.facts.word_facts.contains_key(&word_key) {
            return;
        }

        visit::walk_word(self, word);
        let layout = self.record_word_layout(word);
        self.facts.word_facts.insert(
            word_key,
            WordFacts {
                has_multiline_literal_source: layout.contains_multiline_literal_source,
            },
        );
    }

    fn visit_word_part(&mut self, part: &WordPart, span: Span) {
        match part {
            WordPart::CommandSubstitution { body, syntax }
                if matches!(
                    *syntax,
                    CommandSubstitutionSyntax::DollarParen | CommandSubstitutionSyntax::Backtick
                ) =>
            {
                self.visit_sequence(body, Some(span.end.offset), None);
            }
            WordPart::ProcessSubstitution { body, .. } => {
                self.visit_sequence(body, span.end.offset.checked_sub(1), None);
            }
            WordPart::ZshQualifiedGlob(glob) => {
                for segment in &glob.segments {
                    if let ZshGlobSegment::Pattern(pattern) = segment {
                        self.visit_pattern(pattern);
                    }
                }
            }
            WordPart::ArithmeticExpansion {
                expression_ast: Some(expr),
                ..
            } => self.visit_arithmetic_expr(expr),
            WordPart::ArithmeticExpansion {
                expression_ast: None,
                expression_word_ast,
                ..
            } => self.visit_word(expression_word_ast),
            WordPart::Parameter(parameter) => self.visit_parameter_expansion(parameter),
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.visit_var_ref(reference);
                self.visit_parameter_op(operator);
                if let Some(operand) = operand_word_ast {
                    self.visit_word(operand);
                }
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => self.visit_var_ref(reference),
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
                self.visit_var_ref(reference);
                if let Some(expr) = offset_ast {
                    self.visit_arithmetic_expr(expr);
                } else {
                    self.visit_word(offset_word_ast);
                }
                if let Some(expr) = length_ast {
                    self.visit_arithmetic_expr(expr);
                } else if let Some(word) = length_word_ast {
                    self.visit_word(word);
                }
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.visit_var_ref(reference);
                if let Some(operator) = operator {
                    self.visit_parameter_op(operator);
                }
                if let Some(operand) = operand_word_ast {
                    self.visit_word(operand);
                }
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                for part in parts {
                    self.visit_word_part(&part.kind, part.span);
                }
            }
        }
    }

    fn visit_heredoc_body(&mut self, body: &HeredocBody) {
        for part in &body.parts {
            self.visit_heredoc_body_part(&part.kind, part.span);
        }
    }

    fn visit_heredoc_body_part(&mut self, part: &HeredocBodyPart, span: Span) {
        match part {
            HeredocBodyPart::CommandSubstitution { body, syntax }
                if matches!(
                    *syntax,
                    CommandSubstitutionSyntax::DollarParen | CommandSubstitutionSyntax::Backtick
                ) =>
            {
                self.visit_sequence(body, Some(span.end.offset), None);
            }
            HeredocBodyPart::ArithmeticExpansion {
                expression_ast: Some(expr),
                ..
            } => self.visit_arithmetic_expr(expr),
            HeredocBodyPart::ArithmeticExpansion {
                expression_ast: None,
                expression_word_ast,
                ..
            } => self.visit_word(expression_word_ast),
            HeredocBodyPart::Parameter(parameter) => self.visit_parameter_expansion(parameter),
            HeredocBodyPart::Literal(_)
            | HeredocBodyPart::Variable(_)
            | HeredocBodyPart::CommandSubstitution { .. } => {}
        }
    }

    fn visit_arithmetic_expr(&mut self, expr: &ArithmeticExprNode) {
        visit::walk_arithmetic_expr(self, expr);
    }

    fn record_close_suffix(&mut self, span: Option<Span>) {
        let Some(span) = span else {
            return;
        };
        if let Some(comment) = self.source_map().suffix_comment_after_span(span) {
            self.facts
                .close_suffix_comments
                .insert(FactSpan::from(span), comment);
        }
    }

    fn record_if_branch_prefix_facts(&mut self, command: &IfCommand) {
        for branch_index in 0..=command.elif_branches.len() {
            let Some((start, end)) =
                if_next_branch_region_with_body_end(command, branch_index, self.source, |body| {
                    self.facts.sequence(body, None).body_content_end()
                })
            else {
                continue;
            };
            self.record_branch_prefix_facts(start, end);
        }
    }

    fn record_branch_prefix_facts(&mut self, start: usize, end: usize) {
        let comments = branch_prefix_comments_from_index(
            self.source,
            self.facts.indexer.line_index(),
            self.facts.indexer.comment_index(),
            start,
            end,
        )
        .into_iter()
        .filter(|comment| !self.facts.offset_is_in_heredoc_body(comment.offset))
        .collect();
        self.facts.branch_prefix_facts.insert(
            OffsetRegionKey::new(start, end),
            BranchPrefixFacts::new(self.source, start, end, comments),
        );
    }

    fn build_case_command_facts(&self, command: &CaseCommand) -> CaseCommandFacts {
        let esac_span = case_close_span(command, self.source_map());
        let body_fallback_upper_bound = esac_span
            .map(|span| span.start.offset)
            .unwrap_or(command.span.end.offset);
        let suffix_comments_before_esac = command
            .cases
            .last()
            .and_then(|last_item| case_item_source_end_offset(last_item, self.source))
            .map(|start| {
                own_line_comments_in_region_from_index(
                    self.source,
                    self.facts.indexer.line_index(),
                    self.facts.indexer.comment_index(),
                    start,
                    body_fallback_upper_bound,
                )
                .into_iter()
                .filter(|comment| !self.facts.offset_is_in_heredoc_body(comment.offset))
                .collect()
            })
            .unwrap_or_default();

        CaseCommandFacts {
            esac_span,
            body_fallback_upper_bound,
            has_blank_line_after_in: case_has_blank_line_after_in(command, self.source),
            has_blank_line_before_esac: case_has_blank_line_before_esac(
                command,
                self.source,
                esac_span,
            ),
            suffix_comments_before_esac,
        }
    }

    fn build_case_item_facts(
        &self,
        item: &CaseItem,
        previous_item: Option<&CaseItem>,
        upper_bound: Option<usize>,
    ) -> CaseItemFacts<'source> {
        let sequence = self.facts.sequence(&item.body, upper_bound);
        let first_body_line = sequence.first_rendered_line_for(0);
        let first_body_stmt_line = item
            .body
            .first()
            .map(|stmt| stmt_render_start_line(stmt, self.source, self.source_map(), self.options))
            .unwrap_or(first_body_line);
        let first_pattern_start = item
            .patterns
            .first()
            .map(|pattern| pattern.span.start.offset);
        let mut prefix_comments = first_pattern_start
            .map(|start| {
                let mut comments = sequence
                    .leading_for(0)
                    .iter()
                    .copied()
                    .filter(|comment| comment.span().start.offset < start)
                    .collect::<Vec<_>>();
                for comment in
                    case_item_source_prefix_comments(self.source, self.source_map(), start)
                {
                    if !comments
                        .iter()
                        .any(|existing| existing.span().start.offset == comment.span().start.offset)
                    {
                        comments.push(comment);
                    }
                }
                comments
            })
            .unwrap_or_default();
        prefix_comments.sort_by_key(|comment| comment.span().start.offset);

        CaseItemFacts {
            suffix_comment_start_line: case_suffix_comment_start_line(item),
            has_blank_line_before: previous_item.is_some_and(|previous| {
                case_item_has_blank_line_before(previous, item, self.source)
            }),
            has_blank_line_after_pattern: case_item_has_blank_line_after_pattern(
                item,
                self.source,
                first_body_line,
                first_body_stmt_line,
            ),
            has_blank_line_before_terminator: case_item_has_blank_line_before_terminator(
                item,
                self.source,
                sequence.close_gap_start(),
            ),
            prefix_comments,
            pattern_suffix_comment: case_item_pattern_suffix_comment(
                item,
                upper_bound,
                self.source,
                self.source_map(),
            ),
            terminator_suffix_comment: case_item_terminator_suffix_comment(
                item,
                self.source,
                self.source_map(),
            ),
        }
    }

    fn source_map(&self) -> &SourceMap<'source> {
        &self.facts.source_map
    }
}

impl<'source, 'options> AstVisitor for FormatterFactsBuilder<'source, 'options> {
    fn visit_stmt_seq(&mut self, sequence: &StmtSeq) {
        self.visit_sequence(sequence, None, None);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        FormatterFactsBuilder::visit_stmt(self, stmt);
    }

    fn visit_command(&mut self, command: &Command) {
        FormatterFactsBuilder::visit_command(self, command);
    }

    fn visit_compound_command(&mut self, command: &CompoundCommand) {
        FormatterFactsBuilder::visit_compound_command(self, command);
    }

    fn visit_function(&mut self, function: &FunctionDef) {
        FormatterFactsBuilder::visit_function(self, function);
    }

    fn visit_anonymous_function(&mut self, function: &AnonymousFunctionCommand) {
        FormatterFactsBuilder::visit_anonymous_function(self, function);
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        FormatterFactsBuilder::visit_redirect(self, redirect);
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        FormatterFactsBuilder::visit_assignment(self, assignment);
    }

    fn visit_word(&mut self, word: &Word) {
        FormatterFactsBuilder::visit_word(self, word);
    }

    fn visit_word_part(&mut self, part: &WordPartNode) {
        FormatterFactsBuilder::visit_word_part(self, &part.kind, part.span);
    }

    fn visit_heredoc_body_part(&mut self, part: &HeredocBodyPartNode) {
        FormatterFactsBuilder::visit_heredoc_body_part(self, &part.kind, part.span);
    }
}

#[derive(Debug, Clone, Copy)]
struct BinaryListItemFact<'a> {
    operator_span: Span,
    stmt: &'a Stmt,
}

fn collect_command_list_first<'a>(
    command: &'a BinaryCommand,
    rest: &mut Vec<BinaryListItemFact<'a>>,
) -> &'a Stmt {
    collect_binary_list_first_with(command, rest, &|command| BinaryListItemFact {
        operator_span: command.op_span,
        stmt: command.right.as_ref(),
    })
}

fn stmt_is_multiline_conditional(stmt: &Stmt) -> bool {
    matches!(
        stmt.command,
        Command::Compound(CompoundCommand::Conditional(_))
    )
}

fn stmt_can_follow_multiline_conditional_inline(stmt: &Stmt) -> bool {
    matches!(
        stmt.command,
        Command::Simple(_)
            | Command::Builtin(_)
            | Command::Compound(CompoundCommand::BraceGroup(_) | CompoundCommand::Subshell(_))
    )
}

fn pipeline_has_explicit_line_break(
    pipeline: &BinaryCommand,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    let mut statements = Vec::new();
    let mut operators = Vec::new();
    collect_pipeline_parts(pipeline, &mut statements, &mut operators, &|command| {
        command.op_span
    });

    for (statement, operator_span) in statements.iter().skip(1).zip(operators.iter()) {
        let next_start =
            stmt_start_after_operator(statement, operator_span.end.offset, source, source_map);
        if source_map.operator_starts_or_ends_line(*operator_span)
            || source_map.contains_newline_between(operator_span.end.offset, next_start)
        {
            return true;
        }
    }

    false
}

fn sequence_comment_lower_bound(sequence: &StmtSeq, source_map: &SourceMap<'_>) -> usize {
    let mut lower_bound = sequence.span.start.offset;
    for comment in &sequence.leading_comments {
        if source_map
            .source_comment(*comment)
            .is_some_and(|comment| !comment.inline())
        {
            lower_bound = lower_bound.min(usize::from(comment.range.start()));
        }
    }
    for stmt in sequence.iter() {
        for comment in &stmt.leading_comments {
            if source_map
                .source_comment(*comment)
                .is_some_and(|comment| !comment.inline())
            {
                lower_bound = lower_bound.min(usize::from(comment.range.start()));
            }
        }
    }
    lower_bound
}

fn case_close_span(command: &CaseCommand, source_map: &SourceMap<'_>) -> Option<Span> {
    let start = last_shell_keyword_start(source_map.source(), command.span, "esac")?;
    Some(source_map.span_for_offsets(start, start + "esac".len()))
}

fn group_close_offset(
    source: &str,
    span: Span,
    upper_bound: Option<usize>,
    close_char: char,
    close_len: usize,
) -> usize {
    let fallback = span.end.offset.saturating_sub(close_len);
    let search_end = upper_bound
        .map(|offset| offset.saturating_add(close_len))
        .unwrap_or(span.end.offset)
        .min(source.len())
        .max(span.start.offset);
    source
        .get(span.start.offset..search_end)
        .and_then(|text| text.rfind(close_char))
        .map_or(fallback, |offset| span.start.offset + offset)
}

fn sequence_body_content_end(body: &StmtSeq, source: &str, indexer: &Indexer) -> usize {
    let mut end = body
        .last()
        .map(|stmt| stmt_span(stmt).end.offset)
        .unwrap_or(body.span.end.offset);
    if let Some(stmt) = body.last() {
        for redirect in &stmt.redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let heredoc_end = indexer
                .region_index()
                .heredoc_closing_marker_range(heredoc.body.span.to_range())
                .map(|range| usize::from(range.end()))
                .unwrap_or(heredoc.body.span.end.offset);
            end = end.max(heredoc_end);
        }
    }
    trim_trailing_gap_before_offset(source, end.min(source.len()))
}

fn trim_trailing_gap_before_offset(source: &str, mut offset: usize) -> usize {
    let bytes = source.as_bytes();
    while offset > 0 && matches!(bytes[offset - 1], b' ' | b'\t' | b'\r' | b'\n') {
        offset -= 1;
    }
    offset
}

fn body_has_blank_line_after_open(
    source: &str,
    source_map: &SourceMap<'_>,
    open_end_offset: usize,
    commands: &StmtSeq,
    layout: &LayoutAnnotations,
) -> bool {
    let Some(mut first_start) = sequence_first_content_offset(commands, source_map, layout) else {
        return false;
    };
    if first_start <= open_end_offset
        && let Some(stmt) = commands.first()
    {
        first_start = stmt_first_content_offset(stmt, source_map, layout);
    }
    if source_map.line_number_for_offset(first_start)
        == source_map.line_number_for_offset(open_end_offset)
        && let Some(stmt) = commands.first()
    {
        first_start = stmt_first_content_offset(stmt, source_map, layout);
    }
    let open_line = source_map.line_number_for_offset(open_end_offset);
    let mut comment_search = open_end_offset;
    while let Some(comment_start) = source_map.first_comment_between(comment_search, first_start) {
        if source_map.line_number_for_offset(comment_start) != open_line {
            first_start = comment_start;
            break;
        }
        comment_search = comment_start.saturating_add(1);
    }
    gap_has_blank_line(source, open_end_offset, first_start)
        || (source
            .get(..open_end_offset.min(source.len()))
            .is_some_and(|prefix| prefix.ends_with('\n'))
            && gap_starts_with_empty_physical_line(source, open_end_offset, first_start))
}

fn sequence_first_content_offset(
    commands: &StmtSeq,
    source_map: &SourceMap<'_>,
    layout: &LayoutAnnotations,
) -> Option<usize> {
    let mut first = commands
        .leading_comments
        .iter()
        .map(|comment| usize::from(comment.range.start()))
        .min();
    if let Some(stmt) = commands.first() {
        first = first
            .into_iter()
            .chain(
                stmt.leading_comments
                    .iter()
                    .map(|comment| usize::from(comment.range.start())),
            )
            .chain(std::iter::once(stmt_first_content_offset(
                stmt, source_map, layout,
            )))
            .min();
    }
    first
}

fn stmt_first_content_offset(
    stmt: &Stmt,
    source_map: &SourceMap<'_>,
    layout: &LayoutAnnotations,
) -> usize {
    match &stmt.command {
        Command::Binary(command) => stmt_first_content_offset(&command.left, source_map, layout),
        _ => {
            stmt_group_attachment_or_verbatim_span_with_heredoc(stmt, source_map, |stmt| {
                layout.stmt(stmt).contains_heredoc
            })
            .unwrap_or_else(|| stmt_verbatim_span_with_source_map(stmt, source_map))
            .start
            .offset
        }
    }
}

fn gap_has_blank_line(source: &str, start: usize, end: usize) -> bool {
    source_between_offsets(source, start, end)
        .is_some_and(|gap| gap.bytes().filter(|byte| *byte == b'\n').count() >= 2)
}

fn gap_has_empty_physical_line(source: &str, start: usize, end: usize) -> bool {
    let Some(gap) = source_between_offsets(source, start, end) else {
        return false;
    };
    let bytes = gap.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut next = index + 1;
            while next < bytes.len() && matches!(bytes[next], b' ' | b'\t' | b'\r') {
                next += 1;
            }
            if next < bytes.len() && bytes[next] == b'\n' {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn gap_starts_with_empty_physical_line(source: &str, start: usize, end: usize) -> bool {
    let Some(gap) = source_between_offsets(source, start, end) else {
        return false;
    };
    for byte in gap.bytes() {
        match byte {
            b' ' | b'\t' | b'\r' => {}
            b'\n' => return true,
            _ => return false,
        }
    }
    false
}

fn case_has_blank_line_after_in(command: &CaseCommand, source: &str) -> bool {
    let Some(first_pattern_start) = command
        .cases
        .first()
        .and_then(|item| item.patterns.first())
        .map(|pattern| pattern.span.start.offset)
    else {
        return false;
    };
    let start = command.word.span.end.offset.min(source.len());
    let end = first_pattern_start.min(source.len());
    let Some(prefix) = source.get(start..end) else {
        return false;
    };
    let Some(in_end) = last_shell_keyword_end(prefix, "in") else {
        return false;
    };
    gap_has_empty_physical_line(source, start + in_end, end)
}

fn case_item_has_blank_line_before(previous: &CaseItem, item: &CaseItem, source: &str) -> bool {
    let Some(start) = case_item_source_end_offset(previous, source) else {
        return false;
    };
    let Some(end) = item
        .patterns
        .first()
        .map(|pattern| pattern.span.start.offset)
    else {
        return false;
    };
    gap_has_empty_physical_line(source, start, end)
}

fn case_item_source_end_offset(item: &CaseItem, source: &str) -> Option<usize> {
    let content_end = item
        .body
        .last()
        .map(|stmt| stmt_format_span(stmt).end.offset)
        .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.offset))?;
    if let Some(terminator_span) = item.terminator_span
        && terminator_span.end.offset >= content_end
        && terminator_span.end.offset <= source.len()
    {
        return Some(terminator_span.end.offset);
    }
    let stmt_end = content_end.min(source.len());
    let line_end = source[stmt_end..]
        .find(['\n', '\r'])
        .map_or(source.len(), |offset| stmt_end + offset);
    let terminator = case_terminator(item.terminator);
    let end = source
        .get(stmt_end..line_end)
        .and_then(|tail| {
            tail.find(terminator)
                .map(|offset| stmt_end + offset + terminator.len())
        })
        .unwrap_or(stmt_end);
    Some(end)
}

fn case_suffix_comment_start_line(item: &CaseItem) -> Option<usize> {
    item.terminator_span
        .map(|span| span.end.line)
        .or_else(|| item.body.last().map(|stmt| stmt_span(stmt).end.line))
        .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.line))
}

fn case_has_blank_line_before_esac(
    command: &CaseCommand,
    source: &str,
    esac_span: Option<Span>,
) -> bool {
    let Some(last_item) = command.cases.last() else {
        return false;
    };
    let Some(start) = case_item_source_end_offset(last_item, source) else {
        return false;
    };
    let Some(esac_start) = esac_span.map(|span| span.start.offset) else {
        return false;
    };
    gap_has_blank_line(source, start, esac_start)
}

fn case_item_has_blank_line_after_pattern(
    item: &CaseItem,
    source: &str,
    first_body_line: usize,
    first_body_stmt_line: usize,
) -> bool {
    let Some(pattern_line) = item.patterns.last().map(|pattern| pattern.span.end.line) else {
        return false;
    };
    let stmt_line = if first_body_line <= pattern_line {
        first_body_stmt_line
    } else {
        first_body_line
    };
    if stmt_line == 0 || stmt_line <= pattern_line.saturating_add(1) {
        return false;
    }
    let lines = source.lines().collect::<Vec<_>>();
    ((pattern_line + 1)..stmt_line).any(|line| {
        line.checked_sub(1)
            .and_then(|index| lines.get(index))
            .is_some_and(|text| text.trim_matches([' ', '\t', '\r']).is_empty())
    })
}

fn case_item_has_blank_line_before_terminator(
    item: &CaseItem,
    source: &str,
    content_end: usize,
) -> bool {
    let Some(terminator_start) = item.terminator_span.map(|span| span.start.offset) else {
        return false;
    };
    !item.body.is_empty() && gap_has_empty_physical_line(source, content_end, terminator_start)
}

fn case_item_pattern_suffix_comment<'source>(
    item: &CaseItem,
    upper_bound: Option<usize>,
    source: &'source str,
    source_map: &SourceMap<'source>,
) -> Option<SourceComment<'source>> {
    let start = item.patterns.last()?.span.end.offset.min(source.len());
    let end = item
        .body
        .first()
        .map(|stmt| stmt_span(stmt).start.offset)
        .or_else(|| item.terminator_span.map(|span| span.start.offset))
        .or(upper_bound)
        .unwrap_or(source.len())
        .min(source.len());
    if start >= end {
        return None;
    }
    let between = source.get(start..end)?;
    let line = between.split_once('\n').map_or(between, |(line, _)| line);
    let comment_start = line.find('#')?;
    let before = &line[..comment_start];
    if !before.contains(')') {
        return None;
    }
    let comment = line[comment_start..].trim_end_matches([' ', '\t', '\r']);
    let absolute_start = start + comment_start;
    let absolute_end = absolute_start + comment.len();
    source_map.source_comment_for_offsets(absolute_start, absolute_end)
}

fn case_item_terminator_suffix_comment<'source>(
    item: &CaseItem,
    source: &'source str,
    source_map: &SourceMap<'source>,
) -> Option<SourceComment<'source>> {
    let span = item.terminator_span?;
    if span.start.line != span.end.line {
        return None;
    }
    let start = span.end.offset.min(source.len());
    let suffix_source = source.get(start..)?;
    let line_end = suffix_source
        .find('\n')
        .map_or(source.len(), |offset| start + offset);
    let suffix = source.get(start..line_end)?;
    let leading_padding = suffix.len() - suffix.trim_start_matches([' ', '\t']).len();
    let comment = suffix[leading_padding..].trim_end_matches([' ', '\t', '\r']);
    if !comment.starts_with('#') {
        return None;
    }
    let absolute_start = start + leading_padding;
    let absolute_end = absolute_start + comment.len();
    source_map.source_comment_for_offsets(absolute_start, absolute_end)
}

fn case_item_source_prefix_comments<'source>(
    source: &'source str,
    source_map: &SourceMap<'source>,
    first_pattern_start: usize,
) -> Vec<SourceComment<'source>> {
    let Some((pattern_line_start, _)) = source_map.line_bounds_for_offset(first_pattern_start)
    else {
        return Vec::new();
    };
    if source
        .get(pattern_line_start..first_pattern_start)
        .is_some_and(|prefix| !prefix.trim_matches([' ', '\t', '\r']).is_empty())
    {
        return Vec::new();
    }
    let mut comments = Vec::new();
    let mut next_start = pattern_line_start;
    while let Some((start, end)) = source_map.previous_line_bounds(next_start) {
        let Some(line) = source.get(start..end) else {
            break;
        };
        let trimmed = line.trim_matches([' ', '\t', '\r']);
        if trimmed.is_empty() {
            next_start = start;
            continue;
        }
        let leading_padding = line.len() - line.trim_start_matches([' ', '\t']).len();
        let comment = &line[leading_padding..];
        if !comment.starts_with('#') {
            break;
        }
        let absolute_start = start + leading_padding;
        let absolute_end = absolute_start + comment.trim_end_matches([' ', '\t', '\r']).len();
        if let Some(comment) = source_map.source_comment_for_offsets(absolute_start, absolute_end) {
            comments.push(comment);
        }
        next_start = start;
    }
    comments.reverse();
    comments
}

fn case_item_key(item: &CaseItem) -> FactSpan {
    let start = item
        .patterns
        .first()
        .map(|pattern| pattern.span.start.offset)
        .unwrap_or(item.body.span.start.offset);
    let end = item
        .terminator_span
        .map(|span| span.end.offset)
        .or_else(|| item.body.last().map(|stmt| stmt_span(stmt).end.offset))
        .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.offset))
        .unwrap_or(item.body.span.end.offset);
    FactSpan::from_offsets(start, end)
}

fn if_close_start(command: &IfCommand, source_map: &SourceMap<'_>) -> usize {
    if_close_span(command, source_map.source(), source_map)
        .start
        .offset
}

fn if_branch_upper_bound(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
) -> usize {
    if let Some((start, end)) = if_next_branch_region(command, branch_index, source) {
        facts
            .branch_prefix_first_comment_offset(start, end)
            .unwrap_or(end)
    } else {
        if_close_start(command, source_map)
    }
}

fn if_next_branch_region(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
) -> Option<(usize, usize)> {
    if_next_branch_region_with_body_end(command, branch_index, source, branch_body_content_end)
}

fn branch_body_content_end(body: &StmtSeq) -> usize {
    body.last()
        .map(|stmt| stmt_span(stmt).end.offset)
        .unwrap_or(body.span.end.offset)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_parser::parser::Parser;

    use super::*;
    use crate::command::group_attachment_span;
    use crate::{ShellDialect, ShellFormatOptions};

    fn parse(source: &str) -> shuck_ast::File {
        Parser::new(source).parse().unwrap().file
    }

    fn build_facts<'source>(source: &'source str) -> (shuck_ast::File, FormatterFacts<'source>) {
        build_facts_with_options(source, ShellFormatOptions::default(), "test.sh")
    }

    fn build_facts_with_options<'source>(
        source: &'source str,
        options: ShellFormatOptions,
        path: &str,
    ) -> (shuck_ast::File, FormatterFacts<'source>) {
        let file = parse(source);
        let resolved = options.resolve(source, Some(Path::new(path)));
        let facts = FormatterFacts::build(source, &file, &resolved);
        (file, facts)
    }

    fn first_brace_group(file: &shuck_ast::File) -> &StmtSeq {
        match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        }
    }

    fn group_attachment_source<'source>(
        source: &'source str,
        facts: &FormatterFacts<'source>,
        commands: &StmtSeq,
        open: char,
        close: char,
    ) -> &'source str {
        group_attachment_span(commands.as_slice(), facts.source_map(), open, close)
            .expect("expected group attachment span")
            .slice(source)
    }

    #[test]
    fn builds_branch_comment_sequence_facts() {
        let source =
            "if foo; then\n  one\nelif bar; then\n  # note\n  two\nelse\n  # alt\n  three\nfi\n";
        let (file, facts) = build_facts_with_options(
            source,
            ShellFormatOptions::default().with_dialect(ShellDialect::Bash),
            "test.bash",
        );

        let (_, elif_body) = &match &file.body[0].command {
            Command::Compound(CompoundCommand::If(command)) => &command.elif_branches[0],
            _ => panic!("expected if command"),
        };
        let elif_facts = facts.sequence(
            elif_body,
            Some(if_branch_upper_bound(
                match &file.body[0].command {
                    Command::Compound(CompoundCommand::If(command)) => command,
                    _ => unreachable!(),
                },
                1,
                source,
                facts.source_map(),
                &facts,
            )),
        );
        assert_eq!(elif_facts.leading_for(0).len(), 1);
        assert!(!elif_facts.is_ambiguous());
    }

    #[test]
    fn captures_group_open_suffix_comments() {
        let source = "foo() {\n  # outer\n  { # note\n    echo hi\n  }\n}\n";
        let (file, facts) = build_facts(source);

        let body = match &file.body[0].command {
            Command::Function(function) => match function.body.as_ref() {
                Stmt {
                    command: Command::Compound(CompoundCommand::BraceGroup(commands)),
                    ..
                } => commands,
                _ => panic!("expected brace group"),
            },
            _ => panic!("expected function"),
        };
        let inner = match &body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected inner brace group"),
        };

        let sequence = facts.sequence(inner, Some(body[0].span.end.offset));
        assert!(sequence.group_open_suffix_span().is_some());
        assert!(sequence.leading_for(0).is_empty());
    }

    #[test]
    fn captures_group_attachment_and_blank_line_layout() {
        let source = "{\n\n  echo hi\n\n}\n";
        let (file, facts) = build_facts(source);
        let body = first_brace_group(&file);
        let sequence = facts.sequence(body, None);

        assert_eq!(
            sequence
                .group_attachment_span()
                .expect("expected group attachment")
                .slice(source),
            source.trim_end()
        );
        assert!(sequence.open_end_offset().is_some());
        assert!(sequence.has_blank_line_after_open());
        assert!(sequence.has_blank_line_before_close());
        assert_eq!(&source[..sequence.close_gap_start()], "{\n\n  echo hi");
    }

    #[test]
    fn captures_then_branch_open_suffix_comments() {
        let source = "if foo; then # note\n  bar\nfi\n";
        let (file, facts) = build_facts(source);

        let then_branch = match &file.body[0].command {
            Command::Compound(CompoundCommand::If(command)) => &command.then_branch,
            _ => panic!("expected if command"),
        };
        let sequence = facts.sequence(
            then_branch,
            Some(if_branch_upper_bound(
                match &file.body[0].command {
                    Command::Compound(CompoundCommand::If(command)) => command,
                    _ => unreachable!(),
                },
                0,
                source,
                facts.source_map(),
                &facts,
            )),
        );
        assert!(sequence.group_open_suffix_span().is_some());
        assert!(!sequence.is_ambiguous());
        assert!(sequence.leading_for(0).is_empty());
    }

    #[test]
    fn captures_branch_prefix_region_layout() {
        let source = "if foo; then\n  one\n\n# keep with branch\n\nelif bar; then\n  two\nfi\n";
        let (file, facts) = build_facts(source);
        let command = match &file.body[0].command {
            Command::Compound(CompoundCommand::If(command)) => command,
            _ => panic!("expected if command"),
        };
        let (start, end) = facts
            .if_next_branch_region(command, 0)
            .expect("expected elif branch region");
        let branch = facts.branch_prefix_facts(start, end);

        assert_eq!(branch.comments().len(), 1);
        assert_eq!(branch.comments()[0].text, "# keep with branch");
        assert!(branch.has_blank_line_before_keyword());
        assert!(branch.has_blank_line_after_comments());
        assert_eq!(
            facts.if_branch_upper_bound(command, 0),
            branch
                .first_comment_offset()
                .expect("expected prefix comment")
        );
    }

    #[test]
    fn records_explicit_break_layout_facts() {
        let list_source = "foo &&\n  bar\n";
        let (list_file, list_facts) = build_facts(list_source);

        let Command::Binary(list) = &list_file.body[0].command else {
            panic!("expected command list");
        };
        assert!(list_facts.list_item_has_explicit_line_break(list.op_span));

        let background_source = "background &\necho next\n";
        let (background_file, background_facts) = build_facts(background_source);
        assert!(background_facts.background_has_explicit_line_break(&background_file.body[0]));
    }

    #[test]
    fn records_padding_and_heredoc_verbatim_facts() {
        let source = "a=1  b=2\ncat <<EOF # note\nhi\nEOF\n";
        let (file, facts) = build_facts_with_options(
            source,
            ShellFormatOptions::default().with_keep_padding(true),
            "test.sh",
        );

        assert!(facts.stmt(&file.body[0]).preserve_verbatim());
        assert!(facts.stmt(&file.body[1]).preserve_verbatim());
    }

    #[test]
    fn captures_case_prefix_suffix_and_blank_line_regions() {
        let source = "case value in\n\n  # before pattern\n  one) # pattern note\n    echo one\n\n    ;; # done note\n\n  # before close\nesac # close note\n";
        let (file, facts) = build_facts(source);
        let command = match &file.body[0].command {
            Command::Compound(CompoundCommand::Case(command)) => command,
            _ => panic!("expected case command"),
        };
        let case_facts = facts.case_command(command);
        let item_facts = facts.case_item(&command.cases[0]);

        assert!(case_facts.has_blank_line_after_in());
        assert_eq!(case_facts.suffix_comments_before_esac().len(), 1);
        assert_eq!(
            case_facts.suffix_comments_before_esac()[0].text,
            "# before close"
        );
        assert!(case_facts.has_blank_line_before_esac());
        assert_eq!(item_facts.prefix_comments().len(), 1);
        assert_eq!(item_facts.prefix_comments()[0].text(), "# before pattern");
        assert_eq!(
            item_facts
                .pattern_suffix_comment()
                .expect("expected pattern suffix")
                .text(),
            "# pattern note"
        );
        assert_eq!(
            item_facts
                .terminator_suffix_comment()
                .expect("expected terminator suffix")
                .text(),
            "# done note"
        );
        assert!(
            facts
                .close_suffix_comment_after_span(case_facts.esac_span().unwrap())
                .is_some()
        );
    }

    #[test]
    fn records_non_layout_classification_facts() {
        let source = "value=\"one\ntwo\"\ncat <<EOF\nhi\nEOF\nfoo | { a; b; }\n";
        let (file, facts) = build_facts(source);

        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple assignment");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };

        assert!(facts.word_has_multiline_literal_source(word));
        assert!(facts.stmt(&file.body[1]).contains_heredoc());
        assert!(facts.sequence_contains_heredoc(&file.body));
        assert!(facts.sequence_contains_multistatement_pipeline_brace_group(&file.body));
    }

    #[test]
    fn records_nested_sequence_comment_classification_facts() {
        let source = "value=$(echo hi\n  # note\n)\n";
        let (file, facts) = build_facts(source);

        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple assignment");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };
        let [
            WordPartNode {
                kind: WordPart::CommandSubstitution { body, .. },
                ..
            },
        ] = word.parts.as_slice()
        else {
            panic!("expected command substitution word");
        };

        assert!(facts.sequence_contains_comments(body));
        assert!(facts.word_has_multiline_literal_source(word));
    }

    #[test]
    fn grouped_condition_sequences_do_not_capture_later_file_comments() {
        let source = "download() {\n  local url\n  url=https://github.com/junegunn/fzf/releases/download/v$version/${1}\n  set -o pipefail\n  if ! (try_curl $url || try_wget $url); then\n    set +o pipefail\n    binary_error=\"Failed to download with curl and wget\"\n    return\n  fi\n  set +o pipefail\n}\n\n# Try to download binary executable\narchi=$(uname -smo 2> /dev/null || uname -sm)\n";
        let (file, facts) = build_facts(source);

        let function = match &file.body[0].command {
            Command::Function(function) => function,
            _ => panic!("expected function"),
        };
        let function_body = match &function.body.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group function body"),
        };
        let if_command = match &function_body[3].command {
            Command::Compound(CompoundCommand::If(command)) => command,
            _ => panic!("expected if command"),
        };
        let condition_stmt = &if_command.condition[0];
        let subshell = match &condition_stmt.command {
            Command::Compound(CompoundCommand::Subshell(commands)) => commands,
            _ => panic!("expected subshell condition"),
        };

        let sequence = facts.sequence(subshell, Some(stmt_span(condition_stmt).end.offset));
        let attachment_span =
            group_attachment_span(subshell.as_slice(), facts.source_map(), '(', ')')
                .expect("expected subshell attachment span");
        assert!(!sequence.has_comments());
        assert!(facts.group_was_inline_in_source(subshell));
        assert_eq!(
            attachment_span.slice(source),
            "(try_curl $url || try_wget $url)"
        );
    }

    #[test]
    fn brace_group_attachment_span_reaches_wrapper_close_after_parameter_expansion() {
        let source = "{\n  echo ${value}\n}\n# outside\nprintf '%s\\n' done\n";
        let (file, facts) = build_facts(source);

        let attachment =
            group_attachment_source(source, &facts, first_brace_group(&file), '{', '}');

        assert_eq!(attachment, "{\n  echo ${value}\n}");
    }

    #[test]
    fn function_body_comments_with_parameter_syntax_attach_to_first_stmt() {
        let source = "function f() {\n  # parse all defined shortcuts ${BASH_IT_DIRS_BKS}\n  if [[ -s x ]]; then\n    echo ok\n  fi\n}\n";
        let (file, facts) = build_facts_with_options(
            source,
            ShellFormatOptions::default().with_dialect(ShellDialect::Bash),
            "test.bash",
        );

        let Command::Function(function) = &file.body[0].command else {
            panic!("expected function");
        };
        let Command::Compound(CompoundCommand::BraceGroup(body)) = &function.body.command else {
            panic!("expected brace group body");
        };
        let sequence = facts.sequence(body, Some(function.span.end.offset));
        let leading = sequence.leading_for(0);

        assert_eq!(leading.len(), 1);
        assert_eq!(
            leading[0].text(),
            "# parse all defined shortcuts ${BASH_IT_DIRS_BKS}"
        );
    }

    #[test]
    fn subshell_attachment_span_reaches_wrapper_close_after_command_substitution() {
        let source = "(\n  echo $(printf '%s' value)\n)\n# outside\nprintf '%s\\n' done\n";
        let (file, facts) = build_facts(source);

        let subshell = match &file.body[0].command {
            Command::Compound(CompoundCommand::Subshell(commands)) => commands,
            _ => panic!("expected subshell"),
        };
        let attachment_span =
            group_attachment_span(subshell.as_slice(), facts.source_map(), '(', ')')
                .expect("expected subshell attachment span");

        assert_eq!(
            attachment_span.slice(source),
            "(\n  echo $(printf '%s' value)\n)"
        );
    }

    #[test]
    fn brace_group_attachment_span_keeps_semicolon_terminated_trailing_comments() {
        let source = "{\n  echo ok; # inside\n}\n# outside\nprintf '%s\\n' done\n";
        let (file, facts) = build_facts(source);

        let attachment =
            group_attachment_source(source, &facts, first_brace_group(&file), '{', '}');

        assert_eq!(attachment, "{\n  echo ok; # inside\n}");
    }

    #[test]
    fn brace_group_attachment_span_reaches_wrapper_close_after_heredoc_body() {
        let source = "{\n  cat <<EOF\npayload\nEOF\n}\n# outside\nprintf '%s\\n' done\n";
        let (file, facts) = build_facts(source);

        let attachment =
            group_attachment_source(source, &facts, first_brace_group(&file), '{', '}');

        assert_eq!(attachment, "{\n  cat <<EOF\npayload\nEOF\n}");
    }

    #[test]
    fn brace_group_attachment_span_reaches_wrapper_close_after_line_continuation() {
        let source = "{ echo ok; \\\n}\n# outside\nprintf '%s\\n' done\n";
        let (file, facts) = build_facts(source);

        let brace_group = first_brace_group(&file);
        let attachment = group_attachment_source(source, &facts, brace_group, '{', '}');

        assert!(!facts.group_was_inline_in_source(brace_group));
        assert_eq!(attachment, "{ echo ok; \\\n}");
    }
}
