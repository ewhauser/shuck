use std::collections::{HashMap, HashSet};

use shuck_ast::{
    AnonymousFunctionCommand, ArrayElem, Assignment, AssignmentValue, BinaryCommand, BinaryOp,
    BuiltinCommand, CaseCommand, CaseItem, Command, CommandSubstitutionSyntax, CompoundCommand,
    ConditionalCommand, ConditionalExpr, DeclClause, DeclOperand, File, ForCommand, FunctionDef,
    IfCommand, Pattern, PatternPart, Redirect, RepeatCommand, SelectCommand, Span, Stmt, StmtSeq,
    StmtTerminator, TimeCommand, UntilCommand, WhileCommand, Word, WordPart,
};

use crate::ast_format::flatten_comments;
use crate::command::{
    case_item_was_inline_in_source, group_attachment_span, group_open_suffix,
    group_was_inline_in_source, rendered_stmt_end_line, should_render_verbatim,
    stmt_attachment_span, stmt_format_span, stmt_has_trailing_comment, stmt_span,
    stmt_verbatim_span,
};
use crate::comments::{SourceComment, SourceMap, inspect_sequence_comments_in_window};
use crate::options::ResolvedShellFormatOptions;

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
}

impl<'source> FormatterFacts<'source> {
    pub(crate) fn build(
        source: &'source str,
        file: &File,
        options: &ResolvedShellFormatOptions,
    ) -> Self {
        FormatterFactsBuilder::new(source, options).build(file)
    }

    pub(crate) fn source_map(&self) -> &SourceMap<'source> {
        &self.source_map
    }

    pub(crate) fn stmt(&self, stmt: &Stmt) -> &StmtFacts {
        self.stmt_facts
            .get(&FactSpan::from(stmt_span(stmt)))
            .expect("missing statement facts")
    }

    pub(crate) fn sequence(
        &self,
        sequence: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> &SequenceFacts<'source> {
        let key = SequenceSiteKey::new(sequence, upper_bound);
        self.sequence_facts.get(&key).unwrap_or_else(|| {
            self.sequence_facts
                .iter()
                .find_map(|(candidate, facts)| (candidate.span == key.span).then_some(facts))
                .expect("missing sequence facts")
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

    #[cfg(feature = "benchmarking")]
    pub(crate) fn len(&self) -> usize {
        self.stmt_facts.len()
            + self.sequence_facts.len()
            + self.pipeline_breaks.len()
            + self.list_item_breaks.len()
            + self.background_breaks.len()
            + self.inline_group_sequences.len()
            + self.inline_case_item_bodies.len()
    }
}

struct FormatterFactsBuilder<'source, 'options> {
    source: &'source str,
    options: &'options ResolvedShellFormatOptions,
    facts: FormatterFacts<'source>,
    source_comments: Box<[SourceComment<'source>]>,
}

impl<'source, 'options> FormatterFactsBuilder<'source, 'options> {
    fn new(source: &'source str, options: &'options ResolvedShellFormatOptions) -> Self {
        Self {
            source,
            options,
            facts: FormatterFacts {
                source_map: SourceMap::new(source),
                stmt_facts: HashMap::new(),
                sequence_facts: HashMap::new(),
                pipeline_breaks: HashSet::new(),
                list_item_breaks: HashSet::new(),
                background_breaks: HashSet::new(),
                inline_group_sequences: HashSet::new(),
                inline_case_item_bodies: HashSet::new(),
            },
            source_comments: Box::from([]),
        }
    }

    fn build(mut self, file: &File) -> FormatterFacts<'source> {
        let mut source_comments = flatten_comments(file)
            .into_iter()
            .filter_map(|comment| self.source_map().source_comment(comment))
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
        for stmt in sequence.iter() {
            self.visit_stmt(stmt);
        }

        let key = SequenceSiteKey::new(sequence, upper_bound);
        if self.facts.sequence_facts.contains_key(&key) {
            return;
        }

        let mut facts = SequenceFacts::new(sequence.len());
        facts.group_open_suffix_span = group_open_char.and_then(|open| {
            group_open_suffix(sequence.as_slice(), self.source_map(), open).map(|(span, _)| span)
        });
        let sequence_limit = group_open_char
            .and_then(|open| {
                let close = match open {
                    '{' => '}',
                    '(' => ')',
                    other => other,
                };
                group_attachment_span(sequence.as_slice(), self.source_map(), open, close)
                    .map(|span| span.end.offset)
            })
            .or(upper_bound);

        let lower_bound = sequence_comment_lower_bound(sequence);
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
                    .unwrap_or(self.facts.stmt(stmt).attachment_span().start.line);
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
                stmt_verbatim_span(stmt, self.source)
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

        match &stmt.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                if group_was_inline_in_source(commands.as_slice(), self.source_map(), '{', '}') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(commands.span));
                }
                self.visit_sequence(commands, Some(stmt_span(stmt).end.offset), Some('{'));
            }
            Command::Compound(CompoundCommand::Subshell(commands)) => {
                if group_was_inline_in_source(commands.as_slice(), self.source_map(), '(', ')') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(commands.span));
                }
                self.visit_sequence(commands, Some(stmt_span(stmt).end.offset), Some('('));
            }
            _ => {}
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
        match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
                if let Some(depth) = &command.depth {
                    self.visit_word(depth);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
                if let Some(depth) = &command.depth {
                    self.visit_word(depth);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
                if let Some(code) = &command.code {
                    self.visit_word(code);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
                if let Some(code) = &command.code {
                    self.visit_word(code);
                }
                for word in &command.extra_args {
                    self.visit_word(word);
                }
            }
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
            && pipeline_has_explicit_line_break(command, self.source)
        {
            self.facts
                .pipeline_breaks
                .insert(FactSpan::from(command.span));
        }

        if matches!(command.op, BinaryOp::And | BinaryOp::Or) {
            let mut rest = Vec::new();
            collect_command_list_first(command, &mut rest);
            for item in rest {
                let next_start = self.facts.stmt(item.stmt).attachment_span().start.offset;
                if has_newline_between(self.source, item.operator_span.end.offset, next_start) {
                    self.facts
                        .list_item_breaks
                        .insert(FactSpan::from(item.operator_span));
                }
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
                if group_open_char.is_some()
                    && group_was_inline_in_source(
                        command.body.as_slice(),
                        self.source_map(),
                        '{',
                        '}',
                    )
                {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(command.body.span));
                }
                self.visit_sequence(
                    &command.body,
                    Some(command.span.end.offset),
                    group_open_char,
                );
            }
            CompoundCommand::ArithmeticFor(command) => {
                self.visit_sequence(&command.body, Some(command.span.end.offset), None);
            }
            CompoundCommand::While(command) => self.visit_while(command),
            CompoundCommand::Until(command) => self.visit_until(command),
            CompoundCommand::Case(command) => self.visit_case(command),
            CompoundCommand::Select(command) => self.visit_select(command),
            CompoundCommand::Subshell(body) => {
                if group_was_inline_in_source(body.as_slice(), self.source_map(), '(', ')') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(body.span));
                }
                self.visit_sequence(body, None, Some('('));
            }
            CompoundCommand::BraceGroup(body) => {
                if group_was_inline_in_source(body.as_slice(), self.source_map(), '{', '}') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(body.span));
                }
                self.visit_sequence(body, None, Some('{'));
            }
            CompoundCommand::Arithmetic(_) => {}
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
                if group_was_inline_in_source(command.body.as_slice(), self.source_map(), '{', '}')
                {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(command.body.span));
                }
                if group_was_inline_in_source(
                    command.always_body.as_slice(),
                    self.source_map(),
                    '{',
                    '}',
                ) {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(command.always_body.span));
                }
            }
        }
    }

    fn visit_if(&mut self, command: &IfCommand) {
        self.visit_sequence(&command.condition, None, None);
        let brace_syntax = matches!(command.syntax, shuck_ast::IfSyntax::Brace { .. });
        let group_open_char = brace_syntax.then_some('{');
        if brace_syntax
            && group_was_inline_in_source(
                command.then_branch.as_slice(),
                self.source_map(),
                '{',
                '}',
            )
        {
            self.facts
                .inline_group_sequences
                .insert(FactSpan::from(command.then_branch.span));
        }
        self.visit_sequence(
            &command.then_branch,
            Some(if_branch_upper_bound(command, 0, self.source)),
            group_open_char,
        );
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            self.visit_sequence(condition, None, None);
            if brace_syntax
                && group_was_inline_in_source(body.as_slice(), self.source_map(), '{', '}')
            {
                self.facts
                    .inline_group_sequences
                    .insert(FactSpan::from(body.span));
            }
            self.visit_sequence(
                body,
                Some(if_branch_upper_bound(command, index + 1, self.source)),
                group_open_char,
            );
        }
        if let Some(else_branch) = &command.else_branch {
            if brace_syntax
                && group_was_inline_in_source(else_branch.as_slice(), self.source_map(), '{', '}')
            {
                self.facts
                    .inline_group_sequences
                    .insert(FactSpan::from(else_branch.span));
            }
            let upper_bound = match command.syntax {
                shuck_ast::IfSyntax::ThenFi { .. } | shuck_ast::IfSyntax::Brace { .. } => {
                    Some(command.span.end.offset)
                }
            };
            self.visit_sequence(else_branch, upper_bound, group_open_char);
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
        if group_open_char.is_some()
            && group_was_inline_in_source(command.body.as_slice(), self.source_map(), '{', '}')
        {
            self.facts
                .inline_group_sequences
                .insert(FactSpan::from(command.body.span));
        }
        self.visit_sequence(
            &command.body,
            Some(command.span.end.offset),
            group_open_char,
        );
    }

    fn visit_repeat(&mut self, command: &RepeatCommand) {
        self.visit_word(&command.count);
        let group_open_char =
            matches!(command.syntax, shuck_ast::RepeatSyntax::Brace { .. }).then_some('{');
        if group_open_char.is_some()
            && group_was_inline_in_source(command.body.as_slice(), self.source_map(), '{', '}')
        {
            self.facts
                .inline_group_sequences
                .insert(FactSpan::from(command.body.span));
        }
        self.visit_sequence(
            &command.body,
            Some(command.span.end.offset),
            group_open_char,
        );
    }

    fn visit_while(&mut self, command: &WhileCommand) {
        self.visit_sequence(&command.condition, None, None);
        self.visit_sequence(&command.body, Some(command.span.end.offset), None);
    }

    fn visit_until(&mut self, command: &UntilCommand) {
        self.visit_sequence(&command.condition, None, None);
        self.visit_sequence(&command.body, Some(command.span.end.offset), None);
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
            self.visit_sequence(&item.body, Some(command.span.end.offset), None);
        }
    }

    fn visit_select(&mut self, command: &SelectCommand) {
        for word in &command.words {
            self.visit_word(word);
        }
        self.visit_sequence(&command.body, Some(command.span.end.offset), None);
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

        match function.body.as_ref() {
            Stmt {
                command: Command::Compound(CompoundCommand::BraceGroup(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                if group_was_inline_in_source(commands.as_slice(), self.source_map(), '{', '}') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(commands.span));
                }
                self.visit_sequence(commands, Some(function.span.end.offset), Some('{'));
            }
            Stmt {
                command: Command::Compound(CompoundCommand::Subshell(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                if group_was_inline_in_source(commands.as_slice(), self.source_map(), '(', ')') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(commands.span));
                }
                self.visit_sequence(commands, Some(function.span.end.offset), Some('('));
            }
            body => self.visit_stmt(body),
        }
    }

    fn visit_anonymous_function(&mut self, function: &AnonymousFunctionCommand) {
        for argument in &function.args {
            self.visit_word(argument);
        }

        match function.body.as_ref() {
            Stmt {
                command: Command::Compound(CompoundCommand::BraceGroup(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                if group_was_inline_in_source(commands.as_slice(), self.source_map(), '{', '}') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(commands.span));
                }
                self.visit_sequence(commands, Some(function.span.end.offset), Some('{'));
            }
            Stmt {
                command: Command::Compound(CompoundCommand::Subshell(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                if group_was_inline_in_source(commands.as_slice(), self.source_map(), '(', ')') {
                    self.facts
                        .inline_group_sequences
                        .insert(FactSpan::from(commands.span));
                }
                self.visit_sequence(commands, Some(function.span.end.offset), Some('('));
            }
            body => self.visit_stmt(body),
        }
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        if let Some(word) = redirect.word_target() {
            self.visit_word(word);
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.visit_word(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word)
                        | ArrayElem::Keyed { value: word, .. }
                        | ArrayElem::KeyedAppend { value: word, .. } => self.visit_word(word),
                    }
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
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::DoubleQuoted { parts, .. } => {
                for part in parts {
                    self.visit_word_part(&part.kind, part.span);
                }
            }
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
    if let Command::Binary(left_binary) = &command.left.command
        && command.left.redirects.is_empty()
        && !command.left.negated
        && command.left.terminator.is_none()
        && matches!(left_binary.op, BinaryOp::And | BinaryOp::Or)
    {
        let first = collect_command_list_first(left_binary, rest);
        rest.push(BinaryListItemFact {
            operator_span: command.op_span,
            stmt: command.right.as_ref(),
        });
        return first;
    }

    let first = command.left.as_ref();
    rest.push(BinaryListItemFact {
        operator_span: command.op_span,
        stmt: command.right.as_ref(),
    });
    first
}

fn pipeline_has_explicit_line_break(pipeline: &BinaryCommand, source: &str) -> bool {
    let mut statements = Vec::new();
    collect_pipeline(pipeline, &mut statements);

    let mut previous_end = match statements.first() {
        Some(stmt) => stmt_span(stmt).end.offset,
        None => return false,
    };

    for stmt in statements.iter().skip(1) {
        let next_start = stmt_span(stmt).start.offset;
        if has_newline_between(source, previous_end, next_start) {
            return true;
        }
        previous_end = stmt_span(stmt).end.offset;
    }

    false
}

fn collect_pipeline<'a>(command: &'a BinaryCommand, statements: &mut Vec<&'a Stmt>) {
    collect_pipeline_stmt(command.left.as_ref(), statements);
    collect_pipeline_stmt(command.right.as_ref(), statements);
}

fn collect_pipeline_stmt<'a>(stmt: &'a Stmt, statements: &mut Vec<&'a Stmt>) {
    if let Command::Binary(binary) = &stmt.command
        && stmt.redirects.is_empty()
        && !stmt.negated
        && stmt.terminator.is_none()
        && matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline(binary, statements);
    } else {
        statements.push(stmt);
    }
}

fn has_newline_between(source: &str, start: usize, end: usize) -> bool {
    source
        .get(start.min(end)..end.min(source.len()))
        .is_some_and(|between| between.contains('\n'))
}

fn sequence_comment_lower_bound(sequence: &StmtSeq) -> usize {
    let mut lower_bound = sequence.span.start.offset;
    for comment in &sequence.leading_comments {
        lower_bound = lower_bound.min(usize::from(comment.range.start()));
    }
    for stmt in sequence.iter() {
        for comment in &stmt.leading_comments {
            lower_bound = lower_bound.min(usize::from(comment.range.start()));
        }
    }
    lower_bound
}

fn span_contains_comment(span: Span, comment: SourceComment<'_>) -> bool {
    span.start.offset <= comment.span().start.offset && comment.span().end.offset <= span.end.offset
}

fn if_branch_upper_bound(command: &IfCommand, branch_index: usize, source: &str) -> usize {
    let current_branch_end = if branch_index == 0 {
        command.then_branch.span.end.offset
    } else {
        command
            .elif_branches
            .get(branch_index - 1)
            .map(|(_, body)| body.span.end.offset)
            .unwrap_or(command.then_branch.span.end.offset)
    };

    if let Some((condition, _)) = command.elif_branches.get(branch_index) {
        branch_keyword_offset(
            source,
            current_branch_end,
            condition.span.start.offset,
            "elif",
        )
        .unwrap_or(condition.span.start.offset)
    } else if let Some(body) = &command.else_branch {
        branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
            .unwrap_or(body.span.start.offset)
    } else {
        command.span.end.offset
    }
}

fn branch_keyword_offset(source: &str, start: usize, end: usize, keyword: &str) -> Option<usize> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    source[start..end]
        .rfind(keyword)
        .map(|offset| start + offset)
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

    #[test]
    fn builds_branch_comment_sequence_facts() {
        let source =
            "if foo; then\n  one\nelif bar; then\n  # note\n  two\nelse\n  # alt\n  three\nfi\n";
        let file = parse(source);
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);
        let resolved = options.resolve(source, Some(Path::new("test.bash")));
        let facts = FormatterFacts::build(source, &file, &resolved);

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
            )),
        );
        assert_eq!(elif_facts.leading_for(0).len(), 1);
        assert!(!elif_facts.is_ambiguous());
    }

    #[test]
    fn captures_group_open_suffix_comments() {
        let source = "foo() {\n  # outer\n  { # note\n    echo hi\n  }\n}\n";
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

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
    fn marks_inline_leading_comments_as_ambiguous() {
        let source = "if foo; then # note\n  bar\nfi\n";
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

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
            )),
        );
        assert!(sequence.is_ambiguous());
    }

    #[test]
    fn records_explicit_break_layout_facts() {
        let list_source = "foo &&\n  bar\n";
        let list_file = parse(list_source);
        let options = ShellFormatOptions::default();
        let list_resolved = options.resolve(list_source, Some(Path::new("test.sh")));
        let list_facts = FormatterFacts::build(list_source, &list_file, &list_resolved);

        let Command::Binary(list) = &list_file.body[0].command else {
            panic!("expected command list");
        };
        assert!(list_facts.list_item_has_explicit_line_break(list.op_span));

        let background_source = "background &\necho next\n";
        let background_file = parse(background_source);
        let background_resolved = options.resolve(background_source, Some(Path::new("test.sh")));
        let background_facts =
            FormatterFacts::build(background_source, &background_file, &background_resolved);
        assert!(background_facts.background_has_explicit_line_break(&background_file.body[0]));
    }

    #[test]
    fn records_padding_and_heredoc_verbatim_facts() {
        let source = "a=1  b=2\ncat <<EOF # note\nhi\nEOF\n";
        let file = parse(source);
        let options = ShellFormatOptions::default().with_keep_padding(true);
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

        assert!(facts.stmt(&file.body[0]).preserve_verbatim());
        assert!(facts.stmt(&file.body[1]).preserve_verbatim());
    }

    #[test]
    fn grouped_condition_sequences_do_not_capture_later_file_comments() {
        let source = "download() {\n  local url\n  url=https://github.com/junegunn/fzf/releases/download/v$version/${1}\n  set -o pipefail\n  if ! (try_curl $url || try_wget $url); then\n    set +o pipefail\n    binary_error=\"Failed to download with curl and wget\"\n    return\n  fi\n  set +o pipefail\n}\n\n# Try to download binary executable\narchi=$(uname -smo 2> /dev/null || uname -sm)\n";
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

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
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };
        let attachment_span =
            group_attachment_span(brace_group.as_slice(), facts.source_map(), '{', '}')
                .expect("expected brace group attachment span");

        assert_eq!(attachment_span.slice(source), "{\n  echo ${value}\n}");
    }

    #[test]
    fn subshell_attachment_span_reaches_wrapper_close_after_command_substitution() {
        let source = "(\n  echo $(printf '%s' value)\n)\n# outside\nprintf '%s\\n' done\n";
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

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
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };
        let attachment_span =
            group_attachment_span(brace_group.as_slice(), facts.source_map(), '{', '}')
                .expect("expected brace group attachment span");

        assert_eq!(attachment_span.slice(source), "{\n  echo ok; # inside\n}");
    }

    #[test]
    fn brace_group_attachment_span_reaches_wrapper_close_after_heredoc_body() {
        let source = "{\n  cat <<EOF\npayload\nEOF\n}\n# outside\nprintf '%s\\n' done\n";
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };
        let attachment_span =
            group_attachment_span(brace_group.as_slice(), facts.source_map(), '{', '}')
                .expect("expected brace group attachment span");

        assert_eq!(
            attachment_span.slice(source),
            "{\n  cat <<EOF\npayload\nEOF\n}"
        );
    }

    #[test]
    fn brace_group_attachment_span_reaches_wrapper_close_after_line_continuation() {
        let source = "{ echo ok; \\\n}\n# outside\nprintf '%s\\n' done\n";
        let file = parse(source);
        let options = ShellFormatOptions::default();
        let resolved = options.resolve(source, Some(Path::new("test.sh")));
        let facts = FormatterFacts::build(source, &file, &resolved);

        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };
        let attachment_span =
            group_attachment_span(brace_group.as_slice(), facts.source_map(), '{', '}')
                .expect("expected brace group attachment span");

        assert!(!facts.group_was_inline_in_source(brace_group));
        assert_eq!(attachment_span.slice(source), "{ echo ok; \\\n}");
    }
}
