use std::collections::{HashMap, HashSet};

use shuck_ast::TextSize;
use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, Assignment,
    AssignmentValue, BinaryCommand, BinaryOp, BuiltinCommand, CaseCommand, CaseItem, Command,
    CommandSubstitutionSyntax, CompoundCommand, ConditionalCommand, ConditionalExpr, DeclClause,
    DeclOperand, File, ForCommand, ForeachCommand, FunctionDef, HeredocBody, HeredocBodyPart,
    IfCommand, Pattern, PatternPart, Redirect, RepeatCommand, SelectCommand, Span, Stmt, StmtSeq,
    StmtTerminator, TimeCommand, UntilCommand, WhileCommand, Word, WordPart,
};
use shuck_indexer::Indexer;

use crate::command::{
    array_elem_parts, branch_open_keyword_start, builtin_like_parts, case_item_body_upper_bound,
    case_item_was_inline_in_source, collect_binary_list_first as collect_binary_list_first_with,
    collect_pipeline_parts, command_group_commands, done_close_span, group_attachment_span,
    group_open_suffix, group_was_inline_in_source, if_close_span,
    if_next_branch_region_with_body_end, matching_group_close, rendered_stmt_end_line,
    should_render_verbatim, stmt_attachment_span, stmt_format_span, stmt_has_trailing_comment,
    stmt_render_start_line, stmt_span, stmt_start_after_operator,
    stmt_verbatim_span_with_source_map,
};
use crate::comments::{SourceComment, SourceMap, inspect_sequence_comments_in_window};
use crate::options::ResolvedShellFormatOptions;
use crate::scan::{
    branch_prefix_first_comment_offset, has_newline_between_offsets as has_newline_between,
    last_shell_keyword_start, operator_starts_or_ends_line,
};

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

#[derive(Debug, Clone)]
pub(crate) struct StmtFacts {
    attachment_span: Span,
    render_span: Span,
    rendered_end_line: usize,
    has_trailing_comment: bool,
    preserve_verbatim: bool,
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
}

#[derive(Debug, Clone)]
pub(crate) struct SequenceFacts<'source> {
    leading: Vec<Vec<SourceComment<'source>>>,
    trailing: Vec<Vec<SourceComment<'source>>>,
    dangling: Vec<SourceComment<'source>>,
    ambiguous: bool,
    first_rendered_lines: Vec<usize>,
    group_open_suffix_span: Option<Span>,
}

impl<'source> SequenceFacts<'source> {
    fn new(child_count: usize) -> Self {
        Self {
            leading: vec![Vec::new(); child_count],
            trailing: vec![Vec::new(); child_count],
            dangling: Vec::new(),
            ambiguous: false,
            first_rendered_lines: vec![0; child_count],
            group_open_suffix_span: None,
        }
    }

    pub(crate) fn leading_for(&self, index: usize) -> &[SourceComment<'source>] {
        self.leading.get(index).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn trailing_for(&self, index: usize) -> &[SourceComment<'source>] {
        self.trailing.get(index).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn dangling(&self) -> &[SourceComment<'source>] {
        &self.dangling
    }

    pub(crate) fn is_ambiguous(&self) -> bool {
        self.ambiguous
    }

    pub(crate) fn has_comments(&self) -> bool {
        self.ambiguous
            || !self.dangling.is_empty()
            || self.leading.iter().any(|comments| !comments.is_empty())
            || self.trailing.iter().any(|comments| !comments.is_empty())
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
}

#[derive(Debug, Clone)]
pub(crate) struct FormatterFacts<'source> {
    source_map: SourceMap<'source>,
    stmt_facts: HashMap<FactSpan, StmtFacts>,
    sequence_facts: HashMap<SequenceSiteKey, SequenceFacts<'source>>,
    pipeline_breaks: HashSet<FactSpan>,
    list_item_breaks: HashSet<FactSpan>,
    background_breaks: HashSet<FactSpan>,
    inline_group_sequences: HashSet<FactSpan>,
    inline_case_item_bodies: HashSet<FactSpan>,
    indexer: Indexer,
}

impl<'source> FormatterFacts<'source> {
    pub(crate) fn build(
        source: &'source str,
        file: &File,
        options: &ResolvedShellFormatOptions,
    ) -> Self {
        let indexer = Indexer::for_file(source, file);
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
        self.sequence_facts.get(&key).unwrap_or_else(|| {
            let Some(facts) = self
                .sequence_facts
                .iter()
                .find_map(|(candidate, facts)| (candidate.span == key.span).then_some(facts))
            else {
                unreachable!("missing sequence facts");
            };
            facts
        })
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

    pub(crate) fn group_was_inline_in_source(&self, commands: &StmtSeq) -> bool {
        self.inline_group_sequences
            .contains(&FactSpan::from(commands.span))
    }

    pub(crate) fn case_item_was_inline_in_source(&self, item: &CaseItem) -> bool {
        self.inline_case_item_bodies
            .contains(&FactSpan::from(item.body.span))
    }

    pub(crate) fn offset_is_in_heredoc_body(&self, offset: usize) -> bool {
        self.indexer
            .region_index()
            .is_heredoc(TextSize::new(offset as u32))
    }

    #[cfg(feature = "benchmarking")]
    pub(crate) fn len(&self) -> usize {
        self.stmt_facts.len()
            + self.sequence_facts.len()
            + self.pipeline_breaks.len()
            + self.list_item_breaks.len()
            + self.background_breaks.len()
            + self.inline_group_sequences.len()
            + self.inline_case_item_bodies.len()
            + self.indexer.region_index().heredoc_ranges().len()
    }
}

struct FormatterFactsBuilder<'source, 'options> {
    source: &'source str,
    options: &'options ResolvedShellFormatOptions,
    facts: FormatterFacts<'source>,
    source_comments: Box<[SourceComment<'source>]>,
}

impl<'source, 'options> FormatterFactsBuilder<'source, 'options> {
    fn new(
        source: &'source str,
        options: &'options ResolvedShellFormatOptions,
        indexer: Indexer,
    ) -> Self {
        let source_map = SourceMap::from_indexer(source, &indexer, options.keep_padding());

        Self {
            source,
            options,
            facts: FormatterFacts {
                source_map,
                stmt_facts: HashMap::new(),
                sequence_facts: HashMap::new(),
                pipeline_breaks: HashSet::new(),
                list_item_breaks: HashSet::new(),
                background_breaks: HashSet::new(),
                inline_group_sequences: HashSet::new(),
                inline_case_item_bodies: HashSet::new(),
                indexer,
            },
            source_comments: Box::from([]),
        }
    }

    fn build(mut self, file: &File) -> FormatterFacts<'source> {
        let mut source_comments = self
            .facts
            .indexer
            .comment_index()
            .comments()
            .iter()
            .filter(|comment| {
                !self
                    .facts
                    .indexer
                    .region_index()
                    .is_heredoc(comment.range.start())
            })
            .filter_map(|comment| self.source_map().source_comment_for_indexed(comment))
            .collect::<Vec<_>>();
        source_comments.sort_by_key(|comment| comment.span().start.offset);
        self.source_comments = source_comments.into_boxed_slice();
        self.visit_sequence(&file.body, None, None);
        self.facts
    }

    fn visit_sequence(
        &mut self,
        sequence: &StmtSeq,
        upper_bound: Option<usize>,
        group_open_char: Option<char>,
    ) {
        self.visit_sequence_with_suffix(sequence, upper_bound, group_open_char, None);
    }

    fn visit_do_done_body(&mut self, body: &StmtSeq, span: Span) {
        self.visit_sequence_with_suffix(
            body,
            Some(done_body_upper_bound(self.source_map(), span)),
            None,
            branch_open_suffix_span(body, self.source_map(), "do"),
        );
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
    ) {
        for stmt in sequence.iter() {
            self.visit_stmt(stmt);
        }

        let key = SequenceSiteKey::new(sequence, upper_bound);
        if self.facts.sequence_facts.contains_key(&key) {
            return;
        }

        let mut facts = SequenceFacts::new(sequence.len());
        facts.group_open_suffix_span = open_suffix_span.or_else(|| {
            group_open_char.and_then(|open| {
                group_open_suffix(sequence.as_slice(), self.source_map(), open)
                    .map(|(span, _)| span)
            })
        });
        let group_attachment_span = group_open_char.and_then(|open| {
            let close = match open {
                '{' => '}',
                '(' => ')',
                other => other,
            };
            group_attachment_span(sequence.as_slice(), self.source_map(), open, close)
        });
        let sequence_limit = group_attachment_span
            .map(|span| span.end.offset)
            .or(upper_bound);

        let comment_lower_bound = sequence_comment_lower_bound(sequence, self.source_map());
        let lower_bound = group_attachment_span
            .map(|span| span.start.offset.min(comment_lower_bound))
            .unwrap_or(comment_lower_bound);
        let comment_window = self.comment_window(lower_bound, sequence_limit);

        if sequence.is_empty() {
            facts.dangling = comment_window
                .iter()
                .copied()
                .filter(|comment| {
                    sequence_limit.is_none_or(|limit| comment.span().end.offset <= limit)
                })
                .filter(|comment| {
                    facts
                        .group_open_suffix_span
                        .is_none_or(|span| !span_contains_comment(span, *comment))
                })
                .collect();
            facts.ambiguous = facts.dangling.iter().any(SourceComment::inline);
        } else {
            let child_spans = sequence
                .iter()
                .map(|stmt| self.facts.stmt(stmt).attachment_span())
                .collect::<Vec<_>>();
            let attachment = inspect_sequence_comments_in_window(
                comment_window,
                &child_spans,
                sequence_limit,
                facts.group_open_suffix_span,
            );
            let (leading, trailing, dangling, ambiguous) = attachment.into_parts();
            facts.leading = leading;
            facts.trailing = trailing;
            facts.dangling = dangling;
            facts.ambiguous = ambiguous;

            for (index, stmt) in sequence.iter().enumerate() {
                facts.first_rendered_lines[index] = facts.leading[index]
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
            if has_newline_between(self.source, break_start, next_start) {
                self.facts.background_breaks.insert(break_key);
            }
        }

        self.facts.sequence_facts.insert(key, facts);
    }

    fn comment_window(
        &self,
        lower_bound: usize,
        upper_bound: Option<usize>,
    ) -> &[SourceComment<'source>] {
        let start_index = self
            .source_comments
            .partition_point(|comment| comment.span().start.offset < lower_bound);
        let end_index = upper_bound.map_or(self.source_comments.len(), |limit| {
            self.source_comments
                .partition_point(|comment| comment.span().start.offset < limit)
        });
        &self.source_comments[start_index..end_index.max(start_index)]
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        let stmt_key = FactSpan::from(stmt_span(stmt));
        if !self.facts.stmt_facts.contains_key(&stmt_key) {
            let preserve_verbatim = should_render_verbatim(stmt, self.source_map(), self.options);
            let render_span = if preserve_verbatim {
                stmt_verbatim_span_with_source_map(stmt, self.source_map())
            } else {
                stmt_format_span(stmt)
            };
            self.facts.stmt_facts.insert(
                stmt_key,
                StmtFacts {
                    attachment_span: stmt_attachment_span(
                        stmt,
                        self.source,
                        self.source_map(),
                        self.options,
                    ),
                    render_span,
                    rendered_end_line: rendered_stmt_end_line(stmt, self.source, self.source_map()),
                    has_trailing_comment: stmt_has_trailing_comment(stmt, self.source_map()),
                    preserve_verbatim,
                },
            );
        }

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
    }

    fn visit_command(&mut self, command: &Command) {
        match command {
            Command::Simple(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
                self.visit_word(&command.name);
                for word in &command.args {
                    self.visit_word(word);
                }
            }
            Command::Builtin(command) => self.visit_builtin_command(command),
            Command::Decl(command) => self.visit_decl_clause(command),
            Command::Binary(command) => self.visit_binary_command(command),
            Command::Compound(command) => self.visit_compound_command(command),
            Command::Function(function) => self.visit_function(function),
            Command::AnonymousFunction(function) => self.visit_anonymous_function(function),
        }
    }

    fn visit_builtin_command(&mut self, command: &BuiltinCommand) {
        let (_, _, assignments, primary, extra_args) = builtin_like_parts(command);
        for assignment in assignments {
            self.visit_assignment(assignment);
        }
        if let Some(primary) = primary {
            self.visit_word(primary);
        }
        for word in extra_args {
            self.visit_word(word);
        }
    }

    fn visit_decl_clause(&mut self, command: &DeclClause) {
        for assignment in &command.assignments {
            self.visit_assignment(assignment);
        }
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.visit_word(word),
                DeclOperand::Name(_) => {}
                DeclOperand::Assignment(assignment) => self.visit_assignment(assignment),
            }
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
                if operator_starts_or_ends_line(self.source, item.operator_span)
                    || has_newline_between(self.source, item.operator_span.end.offset, next_start)
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
                let group_open_char =
                    matches!(command.syntax, shuck_ast::ForeachSyntax::ParenBrace { .. })
                        .then_some('{');
                if group_open_char.is_some() {
                    self.record_inline_group_sequence(&command.body, '{', '}');
                }
                self.visit_sequence_with_suffix(
                    &command.body,
                    Some(foreach_body_upper_bound(command, self.source_map())),
                    group_open_char,
                    group_open_char
                        .is_none()
                        .then(|| {
                            matches!(command.syntax, shuck_ast::ForeachSyntax::InDoDone { .. })
                                .then(|| {
                                    branch_open_suffix_span(&command.body, self.source_map(), "do")
                                })
                                .flatten()
                        })
                        .flatten(),
                );
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
                self.visit_do_done_body(&command.body, command.span);
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
        let group_open_char = brace_syntax.then_some('{');
        if brace_syntax {
            self.record_inline_group_sequence(&command.then_branch, '{', '}');
        }
        self.visit_sequence_with_suffix(
            &command.then_branch,
            Some(if_branch_upper_bound(
                command,
                0,
                self.source,
                self.source_map(),
            )),
            group_open_char,
            (!brace_syntax)
                .then(|| branch_open_suffix_span(&command.then_branch, self.source_map(), "then"))
                .flatten(),
        );
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            let condition_upper_bound = if brace_syntax {
                group_attachment_span(body.as_slice(), self.source_map(), '{', '}')
                    .map(|span| span.start.offset)
            } else {
                branch_open_keyword_start(body, self.source_map().source(), "then")
            };
            self.visit_sequence(condition, condition_upper_bound, None);
            if brace_syntax {
                self.record_inline_group_sequence(body, '{', '}');
            }
            self.visit_sequence_with_suffix(
                body,
                Some(if_branch_upper_bound(
                    command,
                    index + 1,
                    self.source,
                    self.source_map(),
                )),
                group_open_char,
                (!brace_syntax)
                    .then(|| branch_open_suffix_span(body, self.source_map(), "then"))
                    .flatten(),
            );
        }
        if let Some(else_branch) = &command.else_branch {
            if brace_syntax {
                self.record_inline_group_sequence(else_branch, '{', '}');
            }
            let upper_bound = Some(if_close_start(command, self.source_map()));
            self.visit_sequence_with_suffix(
                else_branch,
                upper_bound,
                group_open_char,
                (!brace_syntax)
                    .then(|| branch_open_suffix_span(else_branch, self.source_map(), "else"))
                    .flatten(),
            );
        }
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
        let group_open_char = matches!(
            command.syntax,
            shuck_ast::ForSyntax::InBrace { .. } | shuck_ast::ForSyntax::ParenBrace { .. }
        )
        .then_some('{');
        if group_open_char.is_some() {
            self.record_inline_group_sequence(&command.body, '{', '}');
        }
        self.visit_sequence_with_suffix(
            &command.body,
            Some(for_body_upper_bound(command, self.source_map())),
            group_open_char,
            group_open_char
                .is_none()
                .then(|| {
                    matches!(
                        command.syntax,
                        shuck_ast::ForSyntax::InDoDone { .. }
                            | shuck_ast::ForSyntax::ParenDoDone { .. }
                    )
                    .then(|| branch_open_suffix_span(&command.body, self.source_map(), "do"))
                    .flatten()
                })
                .flatten(),
        );
    }

    fn visit_repeat(&mut self, command: &RepeatCommand) {
        self.visit_word(&command.count);
        let group_open_char =
            matches!(command.syntax, shuck_ast::RepeatSyntax::Brace { .. }).then_some('{');
        if group_open_char.is_some() {
            self.record_inline_group_sequence(&command.body, '{', '}');
        }
        self.visit_sequence_with_suffix(
            &command.body,
            Some(repeat_body_upper_bound(command, self.source_map())),
            group_open_char,
            group_open_char
                .is_none()
                .then(|| {
                    matches!(command.syntax, shuck_ast::RepeatSyntax::DoDone { .. })
                        .then(|| branch_open_suffix_span(&command.body, self.source_map(), "do"))
                        .flatten()
                })
                .flatten(),
        );
    }

    fn visit_while(&mut self, command: &WhileCommand) {
        let condition_upper_bound =
            branch_open_keyword_start(&command.body, self.source_map().source(), "do");
        self.visit_sequence(&command.condition, condition_upper_bound, None);
        self.visit_do_done_body(&command.body, command.span);
    }

    fn visit_until(&mut self, command: &UntilCommand) {
        let condition_upper_bound =
            branch_open_keyword_start(&command.body, self.source_map().source(), "do");
        self.visit_sequence(&command.condition, condition_upper_bound, None);
        self.visit_do_done_body(&command.body, command.span);
    }

    fn visit_case(&mut self, command: &CaseCommand) {
        self.visit_word(&command.word);
        for item in &command.cases {
            for pattern in &item.patterns {
                self.visit_pattern(pattern);
            }
            if case_item_was_inline_in_source(item) {
                self.facts
                    .inline_case_item_bodies
                    .insert(FactSpan::from(item.body.span));
            }
            self.visit_sequence(
                &item.body,
                case_item_body_upper_bound(
                    item,
                    case_body_fallback_upper_bound(command, self.source_map()),
                ),
                None,
            );
        }
    }

    fn visit_select(&mut self, command: &SelectCommand) {
        for word in &command.words {
            self.visit_word(word);
        }
        self.visit_do_done_body(&command.body, command.span);
    }

    fn visit_time(&mut self, command: &TimeCommand) {
        if let Some(inner) = &command.command {
            self.visit_stmt(inner.as_ref());
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
        if !body.negated
            && body.redirects.is_empty()
            && body.terminator.is_none()
            && let Some((commands, open)) = command_group_commands(&body.command)
        {
            self.visit_function_group_body(
                commands,
                function_end_offset,
                open,
                matching_group_close(open),
            );
        } else {
            self.visit_stmt(body);
        }
    }

    fn visit_function_group_body(
        &mut self,
        commands: &StmtSeq,
        function_end_offset: usize,
        open: char,
        close: char,
    ) {
        if group_was_inline_in_source(commands.as_slice(), self.source_map(), open, close) {
            self.facts
                .inline_group_sequences
                .insert(FactSpan::from(commands.span));
        }
        self.visit_sequence(commands, Some(function_end_offset), Some(open));
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        if let Some(word) = redirect.word_target() {
            self.visit_word(word);
        }
        if let Some(heredoc) = redirect.heredoc() {
            self.visit_heredoc_body(&heredoc.body);
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.visit_word(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    self.visit_word(array_elem_parts(element).1);
                }
            }
        }
    }

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.visit_conditional_expr(expr.left.as_ref());
                self.visit_conditional_expr(expr.right.as_ref());
            }
            ConditionalExpr::Unary(expr) => self.visit_conditional_expr(expr.expr.as_ref()),
            ConditionalExpr::Parenthesized(expr) => self.visit_conditional_expr(expr.expr.as_ref()),
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => self.visit_word(word),
            ConditionalExpr::Pattern(pattern) => self.visit_pattern(pattern),
            ConditionalExpr::VarRef(_) => {}
        }
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        for part in &pattern.parts {
            match &part.kind {
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.visit_pattern(pattern);
                    }
                }
                PatternPart::Word(word) => self.visit_word(word),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn visit_word(&mut self, word: &Word) {
        for part in &word.parts {
            self.visit_word_part(&part.kind, part.span);
        }
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
            WordPart::ArithmeticExpansion {
                expression_ast: Some(expr),
                ..
            } => self.visit_arithmetic_expr(expr),
            WordPart::CommandSubstitution { .. }
            | WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
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
            HeredocBodyPart::Literal(_)
            | HeredocBodyPart::Variable(_)
            | HeredocBodyPart::CommandSubstitution { .. }
            | HeredocBodyPart::Parameter(_) => {}
        }
    }

    fn visit_arithmetic_expr(&mut self, expr: &ArithmeticExprNode) {
        match &expr.kind {
            ArithmeticExpr::ShellWord(word) => self.visit_word(word),
            ArithmeticExpr::Indexed { index, .. } => self.visit_arithmetic_expr(index),
            ArithmeticExpr::Parenthesized { expression } => self.visit_arithmetic_expr(expression),
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                self.visit_arithmetic_expr(expr);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                self.visit_arithmetic_expr(left);
                self.visit_arithmetic_expr(right);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_arithmetic_expr(condition);
                self.visit_arithmetic_expr(then_expr);
                self.visit_arithmetic_expr(else_expr);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                self.visit_arithmetic_lvalue(target);
                self.visit_arithmetic_expr(value);
            }
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        }
    }

    fn visit_arithmetic_lvalue(&mut self, target: &ArithmeticLvalue) {
        match target {
            ArithmeticLvalue::Indexed { index, .. } => self.visit_arithmetic_expr(index),
            ArithmeticLvalue::Variable(_) => {}
        }
    }

    fn source_map(&self) -> &SourceMap<'source> {
        &self.facts.source_map
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
        if operator_starts_or_ends_line(source, *operator_span)
            || has_newline_between(source, operator_span.end.offset, next_start)
        {
            return true;
        }
    }

    false
}

fn branch_open_suffix_span(
    sequence: &StmtSeq,
    source_map: &SourceMap<'_>,
    keyword: &str,
) -> Option<Span> {
    let source = source_map.source();
    let keyword_offset = branch_open_keyword_start(sequence, source, keyword)?;
    let line_end = source[keyword_offset..]
        .find('\n')
        .map(|offset| keyword_offset + offset)
        .unwrap_or(source.len());
    let suffix_start = keyword_offset + keyword.len();
    let suffix = source.get(suffix_start..line_end)?;
    suffix
        .trim_start_matches(char::is_whitespace)
        .starts_with('#')
        .then(|| source_map.span_for_offsets(suffix_start, line_end))
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

fn case_body_fallback_upper_bound(command: &CaseCommand, source_map: &SourceMap<'_>) -> usize {
    last_shell_keyword_start(source_map.source(), command.span, "esac")
        .unwrap_or(command.span.end.offset)
}

fn done_body_upper_bound(source_map: &SourceMap<'_>, span: Span) -> usize {
    done_close_span(source_map.source(), source_map, span, None)
        .map_or(span.end.offset, |close| close.start.offset)
}

fn for_body_upper_bound(command: &ForCommand, source_map: &SourceMap<'_>) -> usize {
    match command.syntax {
        shuck_ast::ForSyntax::InDoDone { done_span, .. }
        | shuck_ast::ForSyntax::ParenDoDone { done_span, .. } => done_close_span(
            source_map.source(),
            source_map,
            command.span,
            Some(done_span),
        )
        .map_or(done_span.start.offset, |span| span.start.offset),
        shuck_ast::ForSyntax::InBrace {
            right_brace_span, ..
        }
        | shuck_ast::ForSyntax::ParenBrace {
            right_brace_span, ..
        } => right_brace_span.start.offset,
        shuck_ast::ForSyntax::InDirect { .. } | shuck_ast::ForSyntax::ParenDirect { .. } => {
            command.span.end.offset
        }
    }
}

fn foreach_body_upper_bound(command: &ForeachCommand, source_map: &SourceMap<'_>) -> usize {
    match command.syntax {
        shuck_ast::ForeachSyntax::InDoDone { done_span, .. } => done_close_span(
            source_map.source(),
            source_map,
            command.span,
            Some(done_span),
        )
        .map_or(done_span.start.offset, |span| span.start.offset),
        shuck_ast::ForeachSyntax::ParenBrace {
            right_brace_span, ..
        } => right_brace_span.start.offset,
    }
}

fn repeat_body_upper_bound(command: &RepeatCommand, source_map: &SourceMap<'_>) -> usize {
    match command.syntax {
        shuck_ast::RepeatSyntax::DoDone { done_span, .. } => done_close_span(
            source_map.source(),
            source_map,
            command.span,
            Some(done_span),
        )
        .map_or(done_span.start.offset, |span| span.start.offset),
        shuck_ast::RepeatSyntax::Brace {
            right_brace_span, ..
        } => right_brace_span.start.offset,
        shuck_ast::RepeatSyntax::Direct => command.span.end.offset,
    }
}

fn if_close_start(command: &IfCommand, source_map: &SourceMap<'_>) -> usize {
    if_close_span(command, source_map.source(), source_map)
        .start
        .offset
}

fn span_contains_comment(span: Span, comment: SourceComment<'_>) -> bool {
    span.start.offset <= comment.span().start.offset && comment.span().end.offset <= span.end.offset
}

fn if_branch_upper_bound(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    source_map: &SourceMap<'_>,
) -> usize {
    if let Some((start, end)) = if_next_branch_region(command, branch_index, source) {
        branch_prefix_first_comment_offset(source, start, end).unwrap_or(end)
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
            )),
        );
        assert!(sequence.group_open_suffix_span().is_some());
        assert!(!sequence.is_ambiguous());
        assert!(sequence.leading_for(0).is_empty());
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
