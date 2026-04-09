use std::fmt::Write as _;
use std::mem;

use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticForCommand, Assignment,
    BinaryCommand, BinaryOp, BuiltinCommand, CaseCommand, CaseItem, Command, CompoundCommand,
    ConditionalBinaryExpr, ConditionalCommand, ConditionalExpr, ConditionalParenExpr,
    ConditionalUnaryExpr, CoprocCommand, DeclClause, DeclOperand, File, ForCommand, ForSyntax,
    ForeachCommand, ForeachSyntax, FunctionDef, IfCommand, IfSyntax, Pattern, Redirect,
    RedirectKind, RepeatCommand, RepeatSyntax, SelectCommand, SimpleCommand, Span, Stmt, StmtSeq,
    StmtTerminator, TimeCommand, UntilCommand, VarRef, WhileCommand, Word,
};
use shuck_format::{IndentStyle, LineEnding};

use crate::Result;
use crate::command::{
    binary_operator, case_terminator, command_format_span, line_gap_break_count,
    multiline_compound_assignment_lines, render_assignment_head_to_buf, render_assignment_to_buf,
    render_background_operator, render_var_ref_to_buf, slice_span, stmt_span, stmt_verbatim_span,
};
use crate::comments::{SourceComment, SourceMap};
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::word::{render_pattern_syntax_to_buf, render_word_syntax_with_facts_to_buf};

pub(crate) fn format_file_streaming(
    source: &str,
    file: &File,
    options: &ResolvedShellFormatOptions,
) -> Result<String> {
    let facts = FormatterFacts::build(source, file, options);
    let mut formatter = ShellStreamFormatter::new(source, options, &facts);
    formatter.format_stmt_sequence(&file.body, None)?;

    Ok(formatter.finish())
}

pub(crate) fn format_stmt_sequence_streaming_to_buf(
    source: &str,
    statements: &StmtSeq,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
    output: &mut String,
) -> Result<()> {
    let mut nested_output = mem::take(output);
    nested_output.clear();

    let mut formatter =
        ShellStreamFormatter::with_output_buffer(source, options, facts, nested_output);
    formatter.format_stmt_sequence(statements, None)?;
    *output = formatter.finish();
    Ok(())
}

#[derive(Debug, Clone)]
struct PendingHeredoc {
    body_span: Span,
    delimiter: String,
}

#[derive(Debug, Clone, Copy)]
struct BinaryListItem<'a> {
    operator: BinaryOp,
    operator_span: Span,
    stmt: &'a Stmt,
}

struct ShellStreamFormatter<'source, 'facts> {
    source: &'source str,
    options: ResolvedShellFormatOptions,
    facts: &'facts FormatterFacts<'source>,
    output: String,
    scratch: String,
    indent_level: usize,
    line_start: bool,
    pending_heredocs: Vec<PendingHeredoc>,
}

impl<'source, 'facts> ShellStreamFormatter<'source, 'facts> {
    fn new(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
    ) -> Self {
        Self::with_output_buffer(source, options, facts, String::with_capacity(source.len()))
    }

    fn with_output_buffer(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
        output: String,
    ) -> Self {
        Self {
            source,
            options: options.clone(),
            facts,
            output,
            scratch: String::new(),
            indent_level: 0,
            line_start: true,
            pending_heredocs: Vec::new(),
        }
    }

    fn finish(mut self) -> String {
        self.flush_pending_heredocs();
        self.output
    }

    fn source(&self) -> &'source str {
        self.source
    }

    fn options(&self) -> &ResolvedShellFormatOptions {
        &self.options
    }

    fn facts(&self) -> &FormatterFacts<'source> {
        self.facts
    }

    fn source_map(&self) -> &SourceMap<'source> {
        self.facts.source_map()
    }

    fn line_ending(&self) -> &'static str {
        match self.options.line_ending() {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    fn with_indent<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.indent_level += 1;
        let result = f(self);
        self.indent_level = self.indent_level.saturating_sub(1);
        result
    }

    fn take_scratch_buffer(&mut self) -> String {
        let mut scratch = mem::take(&mut self.scratch);
        scratch.clear();
        scratch
    }

    fn restore_scratch_buffer(&mut self, scratch: String) {
        self.scratch = scratch;
    }

    fn write_rendered(
        &mut self,
        render: impl FnOnce(&mut String, &'source str, &ResolvedShellFormatOptions),
    ) {
        let mut scratch = self.take_scratch_buffer();
        render(&mut scratch, self.source, &self.options);
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    fn write_display(&mut self, value: impl std::fmt::Display) {
        self.write_rendered(|scratch, _, _| {
            let _ = write!(scratch, "{value}");
        });
    }

    fn write_indent_units(&mut self, levels: usize) {
        if levels == 0 {
            return;
        }

        if self.line_start {
            self.write_indent();
        }

        match self.options.indent_style() {
            IndentStyle::Tab => {
                for _ in 0..levels {
                    self.output.push('\t');
                }
            }
            IndentStyle::Space => {
                for _ in 0..(levels * usize::from(self.options.indent_width())) {
                    self.output.push(' ');
                }
            }
        }

        self.line_start = false;
    }

    fn write_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let mut remaining = text;
        while !remaining.is_empty() {
            if self.line_start && !remaining.starts_with('\n') {
                self.write_indent();
            }

            match remaining.find('\n') {
                Some(index) => {
                    let end = index + 1;
                    self.output.push_str(&remaining[..end]);
                    self.line_start = true;
                    remaining = &remaining[end..];
                }
                None => {
                    self.output.push_str(remaining);
                    self.line_start = false;
                    break;
                }
            }
        }
    }

    fn write_verbatim(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.output.push_str(text);
        self.line_start = text.ends_with('\n');
    }

    fn write_indent(&mut self) {
        if !self.line_start || self.indent_level == 0 || self.options.minify() {
            return;
        }

        match self.options.indent_style() {
            IndentStyle::Tab => {
                for _ in 0..self.indent_level {
                    self.output.push('\t');
                }
            }
            IndentStyle::Space => {
                for _ in 0..(self.indent_level * usize::from(self.options.indent_width())) {
                    self.output.push(' ');
                }
            }
        }

        self.line_start = false;
    }

    fn write_space(&mut self) {
        if self.line_start {
            return;
        }
        self.output.push(' ');
    }

    fn flush_pending_heredocs(&mut self) {
        let pending = mem::take(&mut self.pending_heredocs);
        for heredoc in pending {
            self.output.push_str(self.line_ending());
            self.line_start = true;
            self.write_verbatim(heredoc.body_span.slice(self.source));
            self.write_verbatim(&heredoc.delimiter);
        }
    }

    fn newline(&mut self) {
        self.flush_pending_heredocs();
        self.output.push_str(self.line_ending());
        self.line_start = true;
    }

    fn write_line_breaks(&mut self, count: usize) {
        for _ in 0..count {
            self.newline();
        }
    }

    fn write_word(&mut self, word: &Word) {
        let source_map = self.source_map().clone();
        let mut scratch = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_word_syntax_with_facts_to_buf(
                word,
                self.source(),
                self.options(),
                &source_map,
                facts,
                &mut scratch,
            );
        }
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    fn write_pattern(&mut self, pattern: &Pattern) {
        self.write_rendered(|scratch, source, options| {
            render_pattern_syntax_to_buf(pattern, source, options, scratch);
        });
    }

    fn write_var_ref(&mut self, reference: &VarRef) {
        self.write_rendered(|scratch, source, _| {
            render_var_ref_to_buf(reference, source, scratch);
        });
    }

    fn write_assignment(&mut self, assignment: &Assignment) {
        self.write_rendered(|scratch, source, options| {
            render_assignment_to_buf(assignment, source, options, scratch);
        });
    }

    fn write_assignment_head(&mut self, assignment: &Assignment) {
        self.write_rendered(|scratch, source, _| {
            render_assignment_head_to_buf(assignment, source, scratch);
        });
    }

    fn write_rendered_name_if_nonempty(
        &mut self,
        rendered_name: &str,
        previous_end: Option<usize>,
        name_span: Span,
    ) -> Option<usize> {
        if rendered_name.is_empty() {
            previous_end
        } else {
            self.write_command_gap(previous_end, name_span.start.offset);
            self.write_text(rendered_name);
            Some(name_span.end.offset)
        }
    }

    fn write_comment(&mut self, comment: &SourceComment<'_>) {
        self.write_text(comment.text());
    }

    fn emit_leading_comments(&mut self, comments: &[SourceComment<'_>], next_line: usize) {
        for (index, comment) in comments.iter().enumerate() {
            self.write_comment(comment);
            let target_line = comments
                .get(index + 1)
                .map(SourceComment::line)
                .unwrap_or(next_line);
            self.write_line_breaks(line_gap_break_count(comment.line(), target_line));
        }
    }

    fn emit_trailing_comments(&mut self, comments: &[SourceComment<'_>]) {
        for comment in comments {
            self.write_space();
            self.write_comment(comment);
        }
    }

    fn emit_dangling_comments(&mut self, comments: &[SourceComment<'_>]) {
        for (index, comment) in comments.iter().enumerate() {
            self.newline();
            self.write_comment(comment);
            if let Some(next) = comments.get(index + 1) {
                self.write_line_breaks(line_gap_break_count(comment.line(), next.line()));
            }
        }
    }

    fn format_stmt_sequence(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        let source = self.source();
        let compact_layout = self.options().compact_layout();
        let minify = self.options().minify();
        let attachments = (!minify).then(|| self.facts().sequence(statements, upper_bound).clone());
        let compact = compact_layout
            && attachments
                .as_ref()
                .is_none_or(|sequence| !sequence.has_comments());

        if statements.is_empty() {
            if let Some(attachment) = attachments.as_ref() {
                self.emit_dangling_comments(attachment.dangling());
            }
            return Ok(());
        }

        if attachments
            .as_ref()
            .is_some_and(|value| value.is_ambiguous())
            && let Some(span) = sequence_verbatim_span(statements, source)
        {
            if let Some(attachment) = attachments.as_ref()
                && let Some(first) = statements.first()
            {
                let leading = attachment
                    .leading_for(0)
                    .iter()
                    .copied()
                    .filter(|comment| comment.span().end.offset <= span.start.offset)
                    .collect::<Vec<_>>();
                self.emit_leading_comments(
                    &leading,
                    self.facts().stmt(first).render_span().start.line,
                );
            }
            self.write_verbatim(span.slice(source));
            if let Some(attachment) = attachments.as_ref() {
                self.emit_dangling_comments(attachment.dangling());
            }
            return Ok(());
        }

        for (index, stmt) in statements.iter().enumerate() {
            if let Some(attachment) = attachments.as_ref() {
                let next_line = self.facts().stmt(stmt).attachment_span().start.line;
                self.emit_leading_comments(attachment.leading_for(index), next_line);
            }

            self.format_stmt(stmt)?;

            if let Some(attachment) = attachments.as_ref() {
                self.emit_trailing_comments(attachment.trailing_for(index));
            }

            if index + 1 < statements.len() {
                if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
                    if self.facts().background_has_explicit_line_break(stmt) {
                        let current_end = self.facts().stmt(stmt).rendered_end_line();
                        let next_start = attachments
                            .as_ref()
                            .map(|attachment| attachment.first_rendered_line_for(index + 1))
                            .unwrap_or(
                                self.facts()
                                    .stmt(&statements[index + 1])
                                    .attachment_span()
                                    .start
                                    .line,
                            );
                        self.write_line_breaks(line_gap_break_count(current_end, next_start));
                    } else {
                        self.write_space();
                    }
                } else if compact {
                    self.write_text("; ");
                } else {
                    let current_end = self.facts().stmt(stmt).rendered_end_line();
                    let next_start = attachments
                        .as_ref()
                        .map(|attachment| attachment.first_rendered_line_for(index + 1))
                        .unwrap_or(
                            self.facts()
                                .stmt(&statements[index + 1])
                                .attachment_span()
                                .start
                                .line,
                        );
                    self.write_line_breaks(line_gap_break_count(current_end, next_start));
                }
            }
        }

        if let Some(attachment) = attachments.as_ref() {
            self.emit_dangling_comments(attachment.dangling());
        }
        Ok(())
    }

    fn format_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        let source = self.source();
        let stmt_facts = self.facts().stmt(stmt);
        if stmt_facts.preserve_verbatim() {
            self.write_verbatim(stmt_facts.render_span().slice(source));
            return Ok(());
        }

        if stmt.negated {
            self.write_text("! ");
        }

        let command_span = command_format_span(&stmt.command);
        let emit_redirects_first = !stmt.redirects.is_empty()
            && command_span != Span::new()
            && stmt
                .redirects
                .iter()
                .all(|redirect| redirect.span.start.offset < command_span.start.offset);

        if emit_redirects_first {
            self.format_redirect_list(&stmt.redirects);
            if command_span != Span::new() {
                self.write_space();
            }
        }

        match &stmt.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                self.format_brace_group(commands, Some(stmt_span(stmt).end.offset))?;
            }
            Command::Compound(CompoundCommand::Subshell(commands)) => {
                self.format_subshell(commands, Some(stmt_span(stmt).end.offset))?;
            }
            _ => self.format_command(&stmt.command)?,
        }

        if !stmt.redirects.is_empty() && !emit_redirects_first {
            self.write_space();
            self.format_redirect_list(&stmt.redirects);
        }

        self.queue_heredocs(&stmt.redirects);

        if let Some(StmtTerminator::Background(operator)) = stmt.terminator {
            self.write_space();
            self.write_text(render_background_operator(operator));
        }

        Ok(())
    }

    fn format_command(&mut self, command: &Command) -> Result<()> {
        match command {
            Command::Simple(command) => self.format_simple_command(command),
            Command::Builtin(command) => self.format_builtin_command(command),
            Command::Decl(command) => self.format_decl_clause(command),
            Command::Binary(command) => self.format_binary_command(command),
            Command::Compound(compound) => self.format_compound_command(compound),
            Command::Function(function) => self.format_function(function),
            Command::AnonymousFunction(function) => self.format_anonymous_function(function),
        }
    }

    fn format_compound_command(&mut self, command: &CompoundCommand) -> Result<()> {
        match command {
            CompoundCommand::If(command) => self.format_if(command),
            CompoundCommand::For(command) => self.format_for(command),
            CompoundCommand::Repeat(command) => self.format_repeat(command),
            CompoundCommand::Foreach(command) => self.format_foreach(command),
            CompoundCommand::ArithmeticFor(command) => self.format_arithmetic_for(command),
            CompoundCommand::While(command) => self.format_while(command),
            CompoundCommand::Until(command) => self.format_until(command),
            CompoundCommand::Case(command) => self.format_case(command),
            CompoundCommand::Select(command) => self.format_select(command),
            CompoundCommand::Subshell(commands) => self.format_subshell(commands, None),
            CompoundCommand::BraceGroup(commands) => self.format_brace_group(commands, None),
            CompoundCommand::Arithmetic(command) => self.format_arithmetic(command),
            CompoundCommand::Time(command) => self.format_time(command),
            CompoundCommand::Conditional(command) => self.format_conditional(command),
            CompoundCommand::Coproc(command) => self.format_coproc(command),
            CompoundCommand::Always(command) => self.format_always(command),
        }
    }

    fn format_simple_command(&mut self, command: &SimpleCommand) -> Result<()> {
        let source = self.source();
        let source_map = self.source_map().clone();
        let mut rendered_name = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_word_syntax_with_facts_to_buf(
                &command.name,
                source,
                self.options(),
                &source_map,
                facts,
                &mut rendered_name,
            );
        }
        if command.args.is_empty()
            && command.assignments.len() == 1
            && rendered_name.is_empty()
            && multiline_compound_assignment_lines(&command.assignments[0], source).is_some()
        {
            self.restore_scratch_buffer(rendered_name);
            return self.format_standalone_multiline_compound_assignment(&command.assignments[0]);
        }

        let mut previous_end = None;
        for assignment in &command.assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        previous_end =
            self.write_rendered_name_if_nonempty(&rendered_name, previous_end, command.name.span);
        self.restore_scratch_buffer(rendered_name);
        for argument in &command.args {
            self.write_command_gap(previous_end, argument.span.start.offset);
            self.write_word(argument);
            previous_end = Some(argument.span.end.offset);
        }
        Ok(())
    }

    fn format_builtin_command(&mut self, command: &BuiltinCommand) -> Result<()> {
        match command {
            BuiltinCommand::Break(command) => self.format_builtin_like(
                "break",
                command.span.start,
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Continue(command) => self.format_builtin_like(
                "continue",
                command.span.start,
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Return(command) => self.format_builtin_like(
                "return",
                command.span.start,
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Exit(command) => self.format_builtin_like(
                "exit",
                command.span.start,
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
        }
    }

    fn format_builtin_like(
        &mut self,
        name: &str,
        start: shuck_ast::Position,
        assignments: &[shuck_ast::Assignment],
        primary: Option<&Word>,
        extra_args: &[Word],
    ) -> Result<()> {
        let mut previous_end = None;
        for assignment in assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        let name_span = Span::from_positions(start, start.advanced_by(name));
        self.write_command_gap(previous_end, name_span.start.offset);
        self.write_text(name);
        previous_end = Some(name_span.end.offset);
        if let Some(primary) = primary {
            self.write_command_gap(previous_end, primary.span.start.offset);
            self.write_word(primary);
            previous_end = Some(primary.span.end.offset);
        }
        for argument in extra_args {
            self.write_command_gap(previous_end, argument.span.start.offset);
            self.write_word(argument);
            previous_end = Some(argument.span.end.offset);
        }
        Ok(())
    }

    fn format_decl_clause(&mut self, command: &DeclClause) -> Result<()> {
        let mut previous_end = None;
        for assignment in &command.assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        self.write_command_gap(previous_end, command.variant_span.start.offset);
        self.write_text(command.variant.as_ref());
        previous_end = Some(command.variant_span.end.offset);
        for operand in &command.operands {
            let span = decl_operand_span(operand);
            self.write_command_gap(previous_end, span.start.offset);
            self.write_decl_operand(operand);
            previous_end = Some(span.end.offset);
        }
        Ok(())
    }

    fn write_command_gap(&mut self, previous_end: Option<usize>, next_start: usize) {
        let Some(previous_end) = previous_end else {
            return;
        };
        if self
            .source()
            .get(previous_end..next_start)
            .is_some_and(|between| between.contains('\n'))
        {
            self.write_text(" \\");
            self.newline();
            self.write_indent_units(1);
        } else {
            self.write_space();
        }
    }

    fn write_decl_operand(&mut self, operand: &DeclOperand) {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.write_word(word),
            DeclOperand::Name(name) => self.write_var_ref(name),
            DeclOperand::Assignment(assignment) => self.write_assignment(assignment),
        }
    }
    fn format_binary_command(&mut self, command: &BinaryCommand) -> Result<()> {
        match command.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => self.format_pipeline(command),
            BinaryOp::And | BinaryOp::Or => self.format_command_list(command),
        }
    }

    fn format_pipeline(&mut self, pipeline: &BinaryCommand) -> Result<()> {
        let mut statements = Vec::new();
        let mut operators = Vec::new();
        collect_pipeline(pipeline, &mut statements, &mut operators);

        let multiline = self.options().binary_next_line()
            && statements.len() > 1
            && self.facts().pipeline_has_explicit_line_break(pipeline);

        for (index, stmt) in statements.iter().enumerate() {
            if index > 0 {
                let operator = operators
                    .get(index - 1)
                    .map(|(operator, _)| binary_operator(operator))
                    .unwrap_or("|");
                if multiline {
                    self.write_text(" \\");
                    self.newline();
                    self.with_indent(|formatter| {
                        formatter.write_text(operator);
                        formatter.write_space();
                        formatter.format_stmt(stmt)
                    })?;
                    continue;
                }
                self.write_space();
                self.write_text(operator);
                self.write_space();
            }
            if !multiline || index == 0 {
                self.format_stmt(stmt)?;
            }
        }

        Ok(())
    }

    fn format_command_list(&mut self, list: &BinaryCommand) -> Result<()> {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.format_stmt(first)?;
        for item in &rest {
            self.format_list_item(item)?;
        }
        Ok(())
    }

    fn format_list_item(&mut self, item: &BinaryListItem<'_>) -> Result<()> {
        if self
            .facts()
            .list_item_has_explicit_line_break(item.operator_span)
        {
            self.write_text(list_item_multiline_separator(item.operator));
            self.newline();
            self.with_indent(|formatter| formatter.format_stmt(item.stmt))?;
            return Ok(());
        }

        self.write_text(list_item_inline_separator(item.operator));
        self.format_stmt(item.stmt)
    }

    fn format_if(&mut self, command: &IfCommand) -> Result<()> {
        match command.syntax {
            IfSyntax::ThenFi { .. } => self.format_then_fi_if(command),
            IfSyntax::Brace { .. } => self.format_brace_if(command),
        }
    }

    fn format_then_fi_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        if command.elif_branches.is_empty()
            && command.else_branch.is_none()
            && self.can_inline_body(&command.then_branch, command.span)
        {
            self.write_text("; then ");
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; fi");
            return Ok(());
        }

        self.write_text("; then");
        self.format_body_with_upper_bound(
            &command.then_branch,
            Some(if_branch_upper_bound(command, 0, source)),
        )?;
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            if self.options().compact_layout() {
                self.write_text("; elif ");
                self.format_inline_stmts(condition)?;
                self.write_text("; then");
            } else {
                self.newline();
                self.write_text("elif ");
                self.format_inline_stmts(condition)?;
                self.write_text("; then");
            }
            self.format_body_with_upper_bound(
                body,
                Some(if_branch_upper_bound(command, index + 1, source)),
            )?;
        }
        if let Some(body) = &command.else_branch {
            if self.options().compact_layout() {
                self.write_text("; else");
            } else {
                self.newline();
                self.write_text("else");
            }
            self.format_body_with_upper_bound(body, Some(command.span.end.offset))?;
        }
        if self.options().compact_layout() {
            self.write_text("; fi");
        } else {
            self.newline();
            self.write_text("fi");
        }
        Ok(())
    }

    fn format_brace_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        self.write_space();
        self.format_brace_group(
            &command.then_branch,
            Some(if_branch_upper_bound(command, 0, source)),
        )?;
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            self.write_text(" elif ");
            self.format_inline_stmts(condition)?;
            self.write_space();
            self.format_brace_group(
                body,
                Some(if_branch_upper_bound(command, index + 1, source)),
            )?;
        }
        if let Some(body) = &command.else_branch {
            self.write_text(" else ");
            self.format_brace_group(body, Some(command.span.end.offset))?;
        }
        Ok(())
    }

    fn format_for(&mut self, command: &ForCommand) -> Result<()> {
        self.write_text("for ");
        for (index, target) in command.targets.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(&target.word);
        }

        match command.syntax {
            ForSyntax::InDoDone { .. } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    for word in words {
                        self.write_space();
                        self.write_word(word);
                    }
                }
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("; do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("; do");
                    self.format_body_with_upper_bound(
                        &command.body,
                        Some(command.span.end.offset),
                    )?;
                    self.finish_block("done");
                }
            }
            ForSyntax::ParenDoDone { .. } => {
                self.write_text(" (");
                for (index, word) in command
                    .words
                    .iter()
                    .flat_map(|words| words.iter())
                    .enumerate()
                {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("); do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("); do");
                    self.format_body_with_upper_bound(
                        &command.body,
                        Some(command.span.end.offset),
                    )?;
                    self.finish_block("done");
                }
            }
            ForSyntax::InBrace { .. } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    for word in words {
                        self.write_space();
                        self.write_word(word);
                    }
                }
                self.write_text("; ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
            ForSyntax::ParenBrace { .. } => {
                self.write_text(" (");
                for (index, word) in command
                    .words
                    .iter()
                    .flat_map(|words| words.iter())
                    .enumerate()
                {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                self.write_text("); ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
        }
        Ok(())
    }

    fn format_repeat(&mut self, command: &RepeatCommand) -> Result<()> {
        self.write_text("repeat ");
        self.write_word(&command.count);
        match command.syntax {
            RepeatSyntax::DoDone { .. } => {
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("; do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("; do");
                    self.format_body_with_upper_bound(
                        &command.body,
                        Some(command.span.end.offset),
                    )?;
                    self.finish_block("done");
                }
            }
            RepeatSyntax::Direct => {
                self.write_space();
                self.format_inline_stmts(&command.body)?;
            }
            RepeatSyntax::Brace { .. } => {
                self.write_space();
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
        }
        Ok(())
    }

    fn format_foreach(&mut self, command: &ForeachCommand) -> Result<()> {
        self.write_text("foreach ");
        self.write_text(command.variable.as_ref());
        match command.syntax {
            ForeachSyntax::ParenBrace { .. } => {
                self.write_text(" (");
                for (index, word) in command.words.iter().enumerate() {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                self.write_text(") ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
            ForeachSyntax::InDoDone { .. } => {
                self.write_text(" in ");
                for (index, word) in command.words.iter().enumerate() {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("; do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("; do");
                    self.format_body_with_upper_bound(
                        &command.body,
                        Some(command.span.end.offset),
                    )?;
                    self.finish_block("done");
                }
            }
        }
        Ok(())
    }

    fn format_select(&mut self, command: &SelectCommand) -> Result<()> {
        self.write_text("select ");
        self.write_text(command.variable.as_ref());
        self.write_text(" in ");
        for (index, word) in command.words.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(word);
        }
        if self.can_inline_body(&command.body, command.span) {
            self.write_text("; do ");
            self.format_inline_stmts(&command.body)?;
            self.write_text("; done");
            return Ok(());
        }
        self.write_text("; do");
        self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset))?;
        self.finish_block("done");
        Ok(())
    }

    fn format_while(&mut self, command: &WhileCommand) -> Result<()> {
        self.write_text("while ");
        self.format_inline_stmts(&command.condition)?;
        if self.can_inline_body(&command.body, command.span) {
            self.write_text("; do ");
            self.format_inline_stmts(&command.body)?;
            self.write_text("; done");
            return Ok(());
        }
        self.write_text("; do");
        self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset))?;
        self.finish_block("done");
        Ok(())
    }

    fn format_until(&mut self, command: &UntilCommand) -> Result<()> {
        self.write_text("until ");
        self.format_inline_stmts(&command.condition)?;
        if self.can_inline_body(&command.body, command.span) {
            self.write_text("; do ");
            self.format_inline_stmts(&command.body)?;
            self.write_text("; done");
            return Ok(());
        }
        self.write_text("; do");
        self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset))?;
        self.finish_block("done");
        Ok(())
    }

    fn format_case(&mut self, command: &CaseCommand) -> Result<()> {
        self.write_text("case ");
        self.write_word(&command.word);
        self.write_text(" in");
        if self.options().compact_layout() {
            for item in &command.cases {
                self.write_space();
                self.format_case_item(item, Some(command.span.end.offset))?;
            }
            self.write_text(" esac");
        } else {
            for item in &command.cases {
                self.newline();
                self.format_case_item(item, Some(command.span.end.offset))?;
            }
            self.newline();
            self.write_text("esac");
        }
        Ok(())
    }

    fn format_case_item(&mut self, item: &CaseItem, upper_bound: Option<usize>) -> Result<()> {
        let base_indent =
            usize::from(!self.options().compact_layout() && self.options().switch_case_indent());

        if base_indent > 0 {
            self.write_case_prefix(base_indent);
        }
        for (index, word) in item.patterns.iter().enumerate() {
            if index > 0 {
                self.write_text(" | ");
            }
            self.write_pattern(word);
        }
        self.write_text(")");

        if item.body.is_empty() {
            self.write_space();
            self.write_text(case_terminator(item.terminator));
        } else if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence(&item.body, upper_bound)?;
            self.write_text("; ");
            self.write_text(case_terminator(item.terminator));
        } else {
            if base_indent == 0
                && item.body.len() == 1
                && self.facts().case_item_was_inline_in_source(item)
            {
                self.write_space();
                self.format_stmt(&item.body[0])?;
                self.write_space();
                self.write_text(case_terminator(item.terminator));
                return Ok(());
            }

            self.newline();
            self.with_extra_prefix_indent(base_indent + 1, |formatter| {
                formatter.format_stmt_sequence(&item.body, upper_bound)
            })?;
            self.newline();
            self.write_case_prefix(base_indent + 1);
            self.write_text(case_terminator(item.terminator));
        }
        Ok(())
    }

    fn with_extra_prefix_indent<T>(&mut self, levels: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        self.indent_level += levels;
        let result = f(self);
        self.indent_level = self.indent_level.saturating_sub(levels);
        result
    }

    fn format_brace_group(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && self.facts().group_was_inline_in_source(commands)
            && self.can_inline_group(commands);
        if should_inline {
            self.write_text("{ ");
            self.format_inline_stmts(commands)?;
            self.write_text("; }");
            return Ok(());
        }
        self.format_group_with_upper_bound("{", "}", '{', commands, false, upper_bound)
    }

    fn format_subshell(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && self.facts().group_was_inline_in_source(commands)
            && self.can_inline_group(commands);
        if should_inline {
            self.write_text("(");
            self.format_inline_stmts(commands)?;
            self.write_text(")");
            return Ok(());
        }
        self.format_group_with_upper_bound("(", ")", '(', commands, false, upper_bound)
    }

    fn format_arithmetic(&mut self, command: &ArithmeticCommand) -> Result<()> {
        let rendered = self
            .source()
            .get(command.span.start.offset..command.span.end.offset)
            .unwrap_or_default();
        self.write_text(rendered);
        Ok(())
    }

    fn format_arithmetic_for(&mut self, command: &ArithmeticForCommand) -> Result<()> {
        let source = self.source();
        let init = slice_span(source, command.init_span);
        let condition = command
            .condition_span
            .map(|span| span.slice(source))
            .unwrap_or("");
        let step = command
            .step_span
            .map(|span| span.slice(source))
            .unwrap_or("");
        self.write_text("for ((");
        self.write_text(init);
        self.write_text(";");
        self.write_text(condition);
        self.write_text(";");
        self.write_text(step);
        self.write_text(")); do");
        self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset))?;
        self.finish_block("done");
        Ok(())
    }

    fn format_time(&mut self, command: &TimeCommand) -> Result<()> {
        if command.posix_format {
            self.write_text("time -p");
        } else {
            self.write_text("time");
        }
        if let Some(command) = &command.command {
            self.write_space();
            self.format_stmt(command)?;
        }
        Ok(())
    }

    fn format_conditional(&mut self, command: &ConditionalCommand) -> Result<()> {
        self.write_text("[[ ");
        self.format_conditional_expr(&command.expression)?;
        self.write_text(" ]]");
        Ok(())
    }

    fn format_coproc(&mut self, command: &CoprocCommand) -> Result<()> {
        self.write_text("coproc");
        if command.name.as_str() != "COPROC" || command.name_span.is_some() {
            self.write_space();
            self.write_text(command.name.as_str());
        }
        self.write_space();
        self.format_stmt(&command.body)
    }

    fn format_always(&mut self, command: &AlwaysCommand) -> Result<()> {
        self.format_brace_group(&command.body, Some(command.span.end.offset))?;
        self.write_text(" always ");
        self.format_brace_group(&command.always_body, Some(command.span.end.offset))
    }

    fn format_function(&mut self, function: &FunctionDef) -> Result<()> {
        self.format_named_function_header(function);
        if self.options().function_next_line() {
            self.newline();
        } else {
            self.write_space();
        }
        self.format_function_body(function.body.as_ref(), function.span.end.offset)
    }

    fn format_anonymous_function(&mut self, function: &AnonymousFunctionCommand) -> Result<()> {
        self.write_text(match function.surface {
            shuck_ast::AnonymousFunctionSurface::FunctionKeyword { .. } => "function",
            shuck_ast::AnonymousFunctionSurface::Parens { .. } => "()",
        });
        if self.options().function_next_line() {
            self.newline();
        } else {
            self.write_space();
        }
        self.format_function_body(function.body.as_ref(), function.span.end.offset)?;
        if !function.args.is_empty() {
            for argument in &function.args {
                self.write_space();
                self.write_word(argument);
            }
        }
        Ok(())
    }

    fn format_named_function_header(&mut self, function: &FunctionDef) {
        if function.header.entries.len() == 1
            && let Some(name) = function.header.entries[0].static_name.as_ref()
        {
            let source_map = self.source_map().clone();
            let mut rendered_entry = self.take_scratch_buffer();
            {
                let facts = self.facts();
                render_word_syntax_with_facts_to_buf(
                    &function.header.entries[0].word,
                    self.source(),
                    self.options(),
                    &source_map,
                    facts,
                    &mut rendered_entry,
                );
            }
            let classic_single_name = name.as_str() == rendered_entry;
            self.restore_scratch_buffer(rendered_entry);

            if classic_single_name {
                if function.uses_function_keyword() {
                    self.write_text("function ");
                }
                self.write_text(name.as_str());
                if function.has_trailing_parens() {
                    self.write_text("()");
                }
                return;
            }
        }

        if function.uses_function_keyword() {
            self.write_text("function");
            if !function.header.entries.is_empty() {
                self.write_space();
            }
        }
        for (index, entry) in function.header.entries.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(&entry.word);
        }
        if function.has_trailing_parens() {
            self.write_text("()");
        }
    }

    fn format_function_body(&mut self, body: &Stmt, upper_bound: usize) -> Result<()> {
        match body {
            Stmt {
                command: Command::Compound(CompoundCommand::BraceGroup(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                let should_inline = !self.options().function_next_line()
                    && self.facts().group_was_inline_in_source(commands)
                    && self.can_inline_group(commands);
                if should_inline {
                    self.write_text("{ ");
                    self.format_inline_stmts(commands)?;
                    self.write_text("; }");
                    Ok(())
                } else {
                    self.format_brace_group(commands, Some(upper_bound))
                }
            }
            Stmt {
                command: Command::Compound(CompoundCommand::Subshell(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                let should_inline = !self.options().function_next_line()
                    && self.facts().group_was_inline_in_source(commands)
                    && self.can_inline_group(commands);
                if should_inline {
                    self.write_text("(");
                    self.format_inline_stmts(commands)?;
                    self.write_text(")");
                    Ok(())
                } else {
                    self.format_subshell(commands, Some(upper_bound))
                }
            }
            _ => self.format_stmt(body),
        }
    }

    fn format_inline_stmts(&mut self, commands: &StmtSeq) -> Result<()> {
        for (index, stmt) in commands.iter().enumerate() {
            if index > 0 {
                if matches!(
                    commands[index - 1].terminator,
                    Some(StmtTerminator::Background(_))
                ) {
                    self.write_space();
                } else {
                    self.write_text("; ");
                }
            }
            self.format_stmt(stmt)?;
        }
        Ok(())
    }

    fn format_body_with_upper_bound(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }

        if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence(commands, upper_bound)
        } else {
            self.newline();
            self.with_indent(|formatter| formatter.format_stmt_sequence(commands, upper_bound))
        }
    }

    fn finish_block(&mut self, close: &'static str) {
        if self.options().compact_layout() {
            self.write_text("; ");
            self.write_text(close);
        } else {
            self.newline();
            self.write_text(close);
        }
    }

    fn format_group_with_upper_bound(
        &mut self,
        open: &'static str,
        close: &'static str,
        _open_char: char,
        commands: &StmtSeq,
        leading_space: bool,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        if leading_space {
            self.write_space();
        }
        self.write_text(open);
        let open_suffix_span = self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span();
        if let Some(span) = open_suffix_span {
            self.write_text(span.slice(self.source()));
        }
        self.format_body_with_upper_bound(commands, upper_bound)?;
        self.finish_block(close);
        Ok(())
    }

    fn format_redirect_list(&mut self, redirects: &[Redirect]) {
        for (index, redirect) in redirects.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.format_redirect(redirect);
        }
    }

    fn format_redirect(&mut self, redirect: &Redirect) {
        let source = self.source();
        let options = self.options().clone();
        if !options.simplify()
            && !options.minify()
            && let Some(raw) = raw_redirect_source_slice(redirect, source)
            && should_preserve_raw_redirect(raw)
        {
            self.write_text(raw);
            return;
        }

        if let Some(name) = &redirect.fd_var {
            self.write_text("{");
            self.write_text(name.as_str());
            self.write_text("}");
        } else if let Some(fd) = redirect
            .fd
            .filter(|fd| should_render_explicit_fd(*fd, redirect.kind))
        {
            self.write_display(fd);
        }

        self.write_text(match redirect.kind {
            RedirectKind::Output => ">",
            RedirectKind::Clobber => ">|",
            RedirectKind::Append => ">>",
            RedirectKind::Input => "<",
            RedirectKind::ReadWrite => "<>",
            RedirectKind::HereDoc => "<<",
            RedirectKind::HereDocStrip => "<<-",
            RedirectKind::HereString => "<<<",
            RedirectKind::DupOutput => ">&",
            RedirectKind::DupInput => "<&",
            RedirectKind::OutputBoth => "&>",
        });

        let mut target = self.take_scratch_buffer();
        let source_map = self.source_map().clone();
        {
            let facts = self.facts();
            match (redirect.word_target(), redirect.heredoc()) {
                (Some(word), None) => render_word_syntax_with_facts_to_buf(
                    word,
                    source,
                    &options,
                    &source_map,
                    facts,
                    &mut target,
                ),
                (None, Some(heredoc)) => render_word_syntax_with_facts_to_buf(
                    &heredoc.delimiter.raw,
                    source,
                    &options,
                    &source_map,
                    facts,
                    &mut target,
                ),
                (None, None) => {}
                (Some(_), Some(_)) => {
                    unreachable!("redirect target cannot be both word and heredoc")
                }
            }
        }
        if needs_space_before_target(redirect.kind, &target, options.space_redirects()) {
            self.write_space();
        }
        self.write_text(&target);
        self.restore_scratch_buffer(target);
    }

    fn queue_heredocs(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        for redirect in redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let mut delimiter = String::new();
            heredoc
                .delimiter
                .raw
                .render_syntax_to_buf(source, &mut delimiter);
            self.pending_heredocs.push(PendingHeredoc {
                body_span: heredoc.body.span,
                delimiter,
            });
        }
    }

    fn format_standalone_multiline_compound_assignment(
        &mut self,
        assignment: &shuck_ast::Assignment,
    ) -> Result<()> {
        let source = self.source();
        let Some(lines) = multiline_compound_assignment_lines(assignment, source) else {
            self.write_assignment(assignment);
            return Ok(());
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        self.newline();
        self.with_indent(|formatter| {
            for (index, line) in lines.iter().enumerate() {
                if index > 0 {
                    formatter.newline();
                }
                formatter.write_text(line);
            }
        });
        self.newline();
        self.write_text(")");
        Ok(())
    }

    fn can_inline_body(&self, commands: &StmtSeq, enclosing_span: Span) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };
        if matches!(command.terminator, Some(StmtTerminator::Background(_)))
            || !self.can_inline_stmt(command)
        {
            return false;
        }

        if self
            .facts()
            .sequence(commands, Some(enclosing_span.end.offset))
            .has_comments()
        {
            return false;
        }

        self.options().compact_layout()
            || stmt_span(command).start.line == enclosing_span.start.line
    }

    fn can_inline_group(&self, commands: &StmtSeq) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };

        self.can_inline_stmt(command)
            && stmt_span(command).start.line == stmt_span(command).end.line
            && self.can_inline_body(commands, stmt_span(command))
    }

    fn can_inline_stmt(&self, stmt: &Stmt) -> bool {
        let stmt_facts = self.facts().stmt(stmt);
        if stmt_facts.preserve_verbatim() || stmt_facts.has_trailing_comment() {
            return false;
        }

        matches!(
            &stmt.command,
            Command::Simple(_)
                | Command::Builtin(_)
                | Command::Decl(_)
                | Command::Binary(_)
                | Command::Compound(
                    CompoundCommand::Conditional(_)
                        | CompoundCommand::Arithmetic(_)
                        | CompoundCommand::Time(_)
                )
        )
    }

    fn format_conditional_expr(&mut self, expression: &ConditionalExpr) -> Result<()> {
        match expression {
            ConditionalExpr::Binary(expr) => self.format_conditional_binary(expr),
            ConditionalExpr::Unary(expr) => self.format_conditional_unary(expr),
            ConditionalExpr::Parenthesized(expr) => self.format_conditional_paren(expr),
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.write_word(word);
                Ok(())
            }
            ConditionalExpr::Pattern(pattern) => {
                self.write_pattern(pattern);
                Ok(())
            }
            ConditionalExpr::VarRef(reference) => {
                self.write_var_ref(reference);
                Ok(())
            }
        }
    }

    fn format_conditional_binary(&mut self, expression: &ConditionalBinaryExpr) -> Result<()> {
        self.format_conditional_expr(&expression.left)?;
        self.write_space();
        self.write_text(expression.op.as_str());
        self.write_space();
        self.format_conditional_expr(&expression.right)
    }

    fn format_conditional_unary(&mut self, expression: &ConditionalUnaryExpr) -> Result<()> {
        self.write_text(expression.op.as_str());
        self.write_space();
        self.format_conditional_expr(&expression.expr)
    }

    fn format_conditional_paren(&mut self, expression: &ConditionalParenExpr) -> Result<()> {
        self.write_text("(");
        self.format_conditional_expr(&expression.expr)?;
        self.write_text(")");
        Ok(())
    }

    fn write_case_prefix(&mut self, levels: usize) {
        if levels == 0 {
            return;
        }
        self.write_indent_units(levels);
    }
}

fn raw_redirect_source_slice<'a>(redirect: &Redirect, source: &'a str) -> Option<&'a str> {
    let span = redirect.span;
    (span.start.offset < span.end.offset && span.end.offset <= source.len())
        .then(|| span.slice(source))
}

fn should_preserve_raw_redirect(raw: &str) -> bool {
    raw.contains(">&$") || raw.contains("<&$")
}

fn should_render_explicit_fd(fd: i32, kind: RedirectKind) -> bool {
    match kind {
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::DupOutput
        | RedirectKind::OutputBoth => fd != 1,
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupInput => fd != 0,
    }
}

fn needs_space_before_target(kind: RedirectKind, target: &str, space_redirects: bool) -> bool {
    if target.is_empty() {
        return false;
    }
    if space_redirects && !matches!(kind, RedirectKind::DupOutput | RedirectKind::DupInput) {
        return true;
    }
    !matches!(kind, RedirectKind::DupOutput | RedirectKind::DupInput)
        && target
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b'<' | b'>' | b'&'))
}

fn decl_operand_span(operand: &DeclOperand) -> Span {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
        DeclOperand::Name(name) => name.span,
        DeclOperand::Assignment(assignment) => assignment.span,
    }
}

fn sequence_verbatim_span(statements: &StmtSeq, source: &str) -> Option<Span> {
    statements
        .iter()
        .map(|stmt| stmt_verbatim_span(stmt, source))
        .reduce(|left, right| left.merge(right))
}

fn collect_pipeline<'a>(
    command: &'a BinaryCommand,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<(BinaryOp, Span)>,
) {
    collect_pipeline_stmt(&command.left, statements, operators);
    operators.push((command.op, command.op_span));
    collect_pipeline_stmt(&command.right, statements, operators);
}

fn collect_pipeline_stmt<'a>(
    stmt: &'a Stmt,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<(BinaryOp, Span)>,
) {
    if let Command::Binary(binary) = &stmt.command
        && stmt.redirects.is_empty()
        && !stmt.negated
        && stmt.terminator.is_none()
        && matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline(binary, statements, operators);
    } else {
        statements.push(stmt);
    }
}

fn collect_command_list_first<'a>(
    command: &'a BinaryCommand,
    rest: &mut Vec<BinaryListItem<'a>>,
) -> &'a Stmt {
    if let Command::Binary(left_binary) = &command.left.command
        && command.left.redirects.is_empty()
        && !command.left.negated
        && command.left.terminator.is_none()
        && matches!(left_binary.op, BinaryOp::And | BinaryOp::Or)
    {
        let first = collect_command_list_first(left_binary, rest);
        rest.push(BinaryListItem {
            operator: command.op,
            operator_span: command.op_span,
            stmt: &command.right,
        });
        return first;
    }

    let first = command.left.as_ref();
    rest.push(BinaryListItem {
        operator: command.op,
        operator_span: command.op_span,
        stmt: &command.right,
    });
    first
}

fn list_item_inline_separator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::And => " && ",
        BinaryOp::Or => " || ",
        BinaryOp::Pipe | BinaryOp::PipeAll => "; ",
    }
}

fn list_item_multiline_separator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::And => " &&",
        BinaryOp::Or => " ||",
        BinaryOp::Pipe | BinaryOp::PipeAll => ";",
    }
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
