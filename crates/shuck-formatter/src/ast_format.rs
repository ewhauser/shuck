use shuck_ast::{
    ArrayElem, Assignment, AssignmentValue, BuiltinCommand, CaseItem, CaseTerminator, Command,
    Comment, CompoundCommand, DeclOperand, File, FunctionDef, Pattern, Redirect, RedirectKind,
    SourceText, Stmt, StmtSeq, StmtTerminator, Subscript, VarRef, Word,
};
use shuck_format::{FormatResult, IndentStyle, text, write};

use crate::comments::Comments;
use crate::options::ResolvedShellFormatOptions;
use crate::shared_traits::FormatRefWithRule;
use crate::{FormatNodeRule, ShellFormatter};

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatFile;

impl FormatNodeRule<File> for FormatFile {
    fn fmt(&self, file: &File, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let rendered = Renderer::new(formatter.context().source(), formatter.context().options())
            .render_file(file);
        write!(formatter, [text(rendered)])
    }
}

impl<'a> crate::shared_traits::AsFormat<'a> for File {
    type Format = FormatRefWithRule<'a, File, FormatFile>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatFile)
    }
}

pub(crate) fn flatten_comments(file: &File) -> Vec<Comment> {
    let mut comments = Vec::new();
    collect_stmt_seq_comments(&file.body, &mut comments);
    comments
}

fn collect_stmt_seq_comments(sequence: &StmtSeq, comments: &mut Vec<Comment>) {
    comments.extend(sequence.leading_comments.iter().copied());
    for stmt in sequence.iter() {
        collect_stmt_comments(stmt, comments);
    }
    comments.extend(sequence.trailing_comments.iter().copied());
}

fn collect_stmt_comments(stmt: &Stmt, comments: &mut Vec<Comment>) {
    comments.extend(stmt.leading_comments.iter().copied());
    if let Some(comment) = stmt.inline_comment {
        comments.push(comment);
    }
    match &stmt.command {
        Command::Binary(command) => {
            collect_stmt_comments(&command.left, comments);
            collect_stmt_comments(&command.right, comments);
        }
        Command::Compound(command) => collect_compound_comments(command, comments),
        Command::Function(function) => collect_stmt_comments(&function.body, comments),
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_compound_comments(command: &CompoundCommand, comments: &mut Vec<Comment>) {
    match command {
        CompoundCommand::If(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.then_branch, comments);
            for (condition, body) in &command.elif_branches {
                collect_stmt_seq_comments(condition, comments);
                collect_stmt_seq_comments(body, comments);
            }
            if let Some(body) = &command.else_branch {
                collect_stmt_seq_comments(body, comments);
            }
        }
        CompoundCommand::For(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::ArithmeticFor(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::While(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Until(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Case(command) => {
            for case in &command.cases {
                collect_stmt_seq_comments(&case.body, comments);
            }
        }
        CompoundCommand::Select(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            collect_stmt_seq_comments(body, comments);
        }
        CompoundCommand::Time(command) => {
            if let Some(inner) = &command.command {
                collect_stmt_comments(inner, comments);
            }
        }
        CompoundCommand::Coproc(command) => collect_stmt_comments(&command.body, comments),
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
    }
}

struct Renderer<'a> {
    source: &'a str,
    options: &'a ResolvedShellFormatOptions,
}

impl<'a> Renderer<'a> {
    fn new(source: &'a str, options: &'a ResolvedShellFormatOptions) -> Self {
        Self { source, options }
    }

    fn render_file(&self, file: &File) -> String {
        self.render_stmt_seq(file.body.as_slice(), &file.body.leading_comments, &file.body.trailing_comments, 0)
    }

    fn indent(&self, level: usize) -> String {
        match self.options.indent_style() {
            IndentStyle::Tab => "\t".repeat(level),
            IndentStyle::Space => " ".repeat(level * usize::from(self.options.indent_width())),
        }
    }

    fn comment_text(&self, comment: Comment) -> &str {
        comment.range.slice(self.source)
    }

    fn render_stmt_seq(
        &self,
        stmts: &[Stmt],
        leading_comments: &[Comment],
        trailing_comments: &[Comment],
        level: usize,
    ) -> String {
        let mut lines = Vec::new();

        if !self.options.minify() {
            for comment in leading_comments {
                lines.push(format!("{}{}", self.indent(level), self.comment_text(*comment)));
            }
        }

        for stmt in stmts {
            if !self.options.minify() {
                for comment in &stmt.leading_comments {
                    lines.push(format!("{}{}", self.indent(level), self.comment_text(*comment)));
                }
            }

            let mut rendered = format!("{}{}", self.indent(level), self.render_stmt(stmt, level));
            if !self.options.minify()
                && let Some(comment) = stmt.inline_comment
            {
                rendered.push_str("  ");
                rendered.push_str(self.comment_text(comment));
            }
            lines.push(rendered);
        }

        if !self.options.minify() {
            for comment in trailing_comments {
                lines.push(format!("{}{}", self.indent(level), self.comment_text(*comment)));
            }
        }

        lines.join("\n")
    }

    fn render_stmt_seq_inline(&self, sequence: &StmtSeq, level: usize) -> String {
        let mut rendered = String::new();
        for (index, stmt) in sequence.iter().enumerate() {
            if index > 0 {
                if rendered.ends_with('&') {
                    rendered.push(' ');
                } else if !rendered.ends_with(';') {
                    rendered.push_str("; ");
                } else {
                    rendered.push(' ');
                }
            }
            rendered.push_str(&self.render_stmt(stmt, level));
        }
        rendered
    }

    fn render_stmt(&self, stmt: &Stmt, level: usize) -> String {
        let mut rendered = String::new();
        if stmt.negated {
            rendered.push_str("! ");
        }
        rendered.push_str(&self.render_command(&stmt.command, level));
        if !stmt.redirects.is_empty() {
            if !rendered.is_empty() {
                rendered.push(' ');
            }
            rendered.push_str(&self.render_redirects(&stmt.redirects));
        }
        if let Some(terminator) = stmt.terminator {
            match terminator {
                StmtTerminator::Semicolon => rendered.push(';'),
                StmtTerminator::Background => rendered.push_str(" &"),
            }
        }
        rendered
    }

    fn render_command(&self, command: &Command, level: usize) -> String {
        match command {
            Command::Simple(command) => self.render_simple_like(
                command.assignments.iter().map(|assignment| self.render_assignment(assignment)),
                std::iter::once(command.name.render_syntax(self.source))
                    .filter(|name| !name.is_empty())
                    .chain(command.args.iter().map(|word| word.render_syntax(self.source))),
            ),
            Command::Builtin(command) => self.render_builtin(command),
            Command::Decl(command) => {
                let operands = command.operands.iter().map(|operand| match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        word.render_syntax(self.source)
                    }
                    DeclOperand::Name(name) => self.render_var_ref(name),
                    DeclOperand::Assignment(assignment) => self.render_assignment(assignment),
                });
                self.render_simple_like(
                    command.assignments.iter().map(|assignment| self.render_assignment(assignment)),
                    std::iter::once(command.variant.to_string()).chain(operands),
                )
            }
            Command::Binary(command) => {
                let separator = if self.options.binary_next_line() {
                    format!(" \\\n{}{}", self.indent(level + 1), self.render_binary_op(&command.op))
                } else {
                    format!(" {} ", self.render_binary_op(&command.op))
                };
                format!(
                    "{}{}{}",
                    self.render_stmt(&command.left, level),
                    separator,
                    self.render_stmt(&command.right, level)
                )
            }
            Command::Compound(command) => self.render_compound(command, level),
            Command::Function(function) => self.render_function(function, level),
        }
    }

    fn render_builtin(&self, command: &BuiltinCommand) -> String {
        match command {
            BuiltinCommand::Break(command) => self.render_simple_like(
                command.assignments.iter().map(|assignment| self.render_assignment(assignment)),
                std::iter::once("break".to_string())
                    .chain(command.depth.iter().map(|word| word.render_syntax(self.source)))
                    .chain(command.extra_args.iter().map(|word| word.render_syntax(self.source))),
            ),
            BuiltinCommand::Continue(command) => self.render_simple_like(
                command.assignments.iter().map(|assignment| self.render_assignment(assignment)),
                std::iter::once("continue".to_string())
                    .chain(command.depth.iter().map(|word| word.render_syntax(self.source)))
                    .chain(command.extra_args.iter().map(|word| word.render_syntax(self.source))),
            ),
            BuiltinCommand::Return(command) => self.render_simple_like(
                command.assignments.iter().map(|assignment| self.render_assignment(assignment)),
                std::iter::once("return".to_string())
                    .chain(command.code.iter().map(|word| word.render_syntax(self.source)))
                    .chain(command.extra_args.iter().map(|word| word.render_syntax(self.source))),
            ),
            BuiltinCommand::Exit(command) => self.render_simple_like(
                command.assignments.iter().map(|assignment| self.render_assignment(assignment)),
                std::iter::once("exit".to_string())
                    .chain(command.code.iter().map(|word| word.render_syntax(self.source)))
                    .chain(command.extra_args.iter().map(|word| word.render_syntax(self.source))),
            ),
        }
    }

    fn render_simple_like<I, J>(&self, assignments: I, words: J) -> String
    where
        I: IntoIterator<Item = String>,
        J: IntoIterator<Item = String>,
    {
        assignments
            .into_iter()
            .chain(words)
            .filter(|piece| !piece.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn render_binary_op(&self, op: &shuck_ast::BinaryOp) -> &'static str {
        match op {
            shuck_ast::BinaryOp::And => "&&",
            shuck_ast::BinaryOp::Or => "||",
            shuck_ast::BinaryOp::Pipe => "|",
            shuck_ast::BinaryOp::PipeAll => "|&",
        }
    }

    fn render_compound(&self, command: &CompoundCommand, level: usize) -> String {
        match command {
            CompoundCommand::If(command) => {
                let mut rendered = format!(
                    "if {}; then",
                    self.render_stmt_seq_inline(&command.condition, level)
                );
                let then_body = self.render_stmt_seq(
                    command.then_branch.as_slice(),
                    &command.then_branch.leading_comments,
                    &command.then_branch.trailing_comments,
                    level + 1,
                );
                if !then_body.is_empty() {
                    rendered.push('\n');
                    rendered.push_str(&then_body);
                }
                for (condition, body) in &command.elif_branches {
                    rendered.push('\n');
                    rendered.push_str(&self.indent(level));
                    rendered.push_str("elif ");
                    rendered.push_str(&self.render_stmt_seq_inline(condition, level));
                    rendered.push_str("; then");
                    let body_text = self.render_stmt_seq(
                        body.as_slice(),
                        &body.leading_comments,
                        &body.trailing_comments,
                        level + 1,
                    );
                    if !body_text.is_empty() {
                        rendered.push('\n');
                        rendered.push_str(&body_text);
                    }
                }
                if let Some(body) = &command.else_branch {
                    rendered.push('\n');
                    rendered.push_str(&self.indent(level));
                    rendered.push_str("else");
                    let body_text = self.render_stmt_seq(
                        body.as_slice(),
                        &body.leading_comments,
                        &body.trailing_comments,
                        level + 1,
                    );
                    if !body_text.is_empty() {
                        rendered.push('\n');
                        rendered.push_str(&body_text);
                    }
                }
                rendered.push('\n');
                rendered.push_str(&self.indent(level));
                rendered.push_str("fi");
                rendered
            }
            CompoundCommand::For(command) => {
                let mut rendered = format!("for {}", command.variable);
                if let Some(words) = &command.words {
                    rendered.push_str(" in");
                    for word in words {
                        rendered.push(' ');
                        rendered.push_str(&word.render_syntax(self.source));
                    }
                }
                rendered.push_str("; do");
                let body = self.render_stmt_seq(
                    command.body.as_slice(),
                    &command.body.leading_comments,
                    &command.body.trailing_comments,
                    level + 1,
                );
                if !body.is_empty() {
                    rendered.push('\n');
                    rendered.push_str(&body);
                }
                rendered.push('\n');
                rendered.push_str(&self.indent(level));
                rendered.push_str("done");
                rendered
            }
            CompoundCommand::ArithmeticFor(command) => {
                let mut rendered = format!(
                    "for {}; do",
                    command
                        .left_paren_span
                        .merge(command.right_paren_span)
                        .slice(self.source)
                );
                let body = self.render_stmt_seq(
                    command.body.as_slice(),
                    &command.body.leading_comments,
                    &command.body.trailing_comments,
                    level + 1,
                );
                if !body.is_empty() {
                    rendered.push('\n');
                    rendered.push_str(&body);
                }
                rendered.push('\n');
                rendered.push_str(&self.indent(level));
                rendered.push_str("done");
                rendered
            }
            CompoundCommand::While(command) => {
                self.render_loop_like("while", &command.condition, &command.body, level)
            }
            CompoundCommand::Until(command) => {
                self.render_loop_like("until", &command.condition, &command.body, level)
            }
            CompoundCommand::Case(command) => {
                let mut rendered = format!("case {} in", command.word.render_syntax(self.source));
                for item in &command.cases {
                    rendered.push('\n');
                    rendered.push_str(&self.render_case_item(item, level + 1));
                }
                rendered.push('\n');
                rendered.push_str(&self.indent(level));
                rendered.push_str("esac");
                rendered
            }
            CompoundCommand::Select(command) => {
                let mut rendered = format!("select {} in", command.variable);
                for word in &command.words {
                    rendered.push(' ');
                    rendered.push_str(&word.render_syntax(self.source));
                }
                rendered.push_str("; do");
                let body = self.render_stmt_seq(
                    command.body.as_slice(),
                    &command.body.leading_comments,
                    &command.body.trailing_comments,
                    level + 1,
                );
                if !body.is_empty() {
                    rendered.push('\n');
                    rendered.push_str(&body);
                }
                rendered.push('\n');
                rendered.push_str(&self.indent(level));
                rendered.push_str("done");
                rendered
            }
            CompoundCommand::Subshell(body) => self.render_group("(", ")", body, level),
            CompoundCommand::BraceGroup(body) => self.render_group("{", "}", body, level),
            CompoundCommand::Arithmetic(command) => command.span.slice(self.source).to_string(),
            CompoundCommand::Time(command) => {
                let mut rendered = String::from("time");
                if command.posix_format {
                    rendered.push_str(" -p");
                }
                if let Some(inner) = &command.command {
                    rendered.push(' ');
                    rendered.push_str(&self.render_stmt(inner, level));
                }
                rendered
            }
            CompoundCommand::Conditional(command) => command.span.slice(self.source).to_string(),
            CompoundCommand::Coproc(command) => {
                let mut rendered = String::from("coproc");
                if command.name_span.is_some() {
                    rendered.push(' ');
                    rendered.push_str(command.name.as_str());
                }
                rendered.push(' ');
                rendered.push_str(&self.render_stmt(&command.body, level));
                rendered
            }
        }
    }

    fn render_loop_like(
        &self,
        keyword: &str,
        condition: &StmtSeq,
        body: &StmtSeq,
        level: usize,
    ) -> String {
        let mut rendered = format!("{keyword} {}; do", self.render_stmt_seq_inline(condition, level));
        let body_text = self.render_stmt_seq(
            body.as_slice(),
            &body.leading_comments,
            &body.trailing_comments,
            level + 1,
        );
        if !body_text.is_empty() {
            rendered.push('\n');
            rendered.push_str(&body_text);
        }
        rendered.push('\n');
        rendered.push_str(&self.indent(level));
        rendered.push_str("done");
        rendered
    }

    fn render_case_item(&self, item: &CaseItem, level: usize) -> String {
        let patterns = item
            .patterns
            .iter()
            .map(|pattern| pattern.render_syntax(self.source))
            .collect::<Vec<_>>()
            .join(" | ");
        let mut rendered = format!("{}{})", self.indent(level), patterns);
        let body = self.render_stmt_seq(
            item.body.as_slice(),
            &item.body.leading_comments,
            &item.body.trailing_comments,
            level + 1 + usize::from(self.options.switch_case_indent()),
        );
        if !body.is_empty() {
            rendered.push('\n');
            rendered.push_str(&body);
            rendered.push('\n');
            rendered.push_str(&self.indent(level));
        }
        rendered.push_str(match item.terminator {
            CaseTerminator::Break => ";;",
            CaseTerminator::FallThrough => ";&",
            CaseTerminator::Continue => ";;&",
        });
        rendered
    }

    fn render_group(&self, open: &str, close: &str, body: &StmtSeq, level: usize) -> String {
        let body_text = self.render_stmt_seq(
            body.as_slice(),
            &body.leading_comments,
            &body.trailing_comments,
            level + 1,
        );
        if body_text.is_empty() {
            format!("{open} {close}")
        } else {
            format!(
                "{open}\n{body}\n{indent}{close}",
                body = body_text,
                indent = self.indent(level)
            )
        }
    }

    fn render_function(&self, function: &FunctionDef, level: usize) -> String {
        let mut rendered = String::new();
        if function.surface.uses_function_keyword() {
            rendered.push_str("function ");
        }
        rendered.push_str(function.name.as_str());
        if function.surface.has_name_parens() {
            rendered.push_str("()");
        }
        if self.options.function_next_line() {
            rendered.push('\n');
            rendered.push_str(&self.indent(level));
        } else {
            rendered.push(' ');
        }
        rendered.push_str(&self.render_stmt(&function.body, level));
        rendered
    }

    fn render_redirects(&self, redirects: &[Redirect]) -> String {
        redirects
            .iter()
            .map(|redirect| self.render_redirect(redirect))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn render_redirect(&self, redirect: &Redirect) -> String {
        let mut rendered = String::new();

        if let Some(name) = &redirect.fd_var {
            rendered.push('{');
            rendered.push_str(name.as_str());
            rendered.push('}');
        } else if let Some(fd) = redirect
            .fd
            .filter(|fd| self.should_render_explicit_fd(*fd, redirect.kind))
        {
            rendered.push_str(&fd.to_string());
        }

        rendered.push_str(match redirect.kind {
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

        if self.options.space_redirects()
            && !matches!(
                redirect.kind,
                RedirectKind::DupOutput | RedirectKind::DupInput
            )
        {
            rendered.push(' ');
        }

        match (redirect.word_target(), redirect.heredoc()) {
            (Some(word), None) => rendered.push_str(&word.render_syntax(self.source)),
            (None, Some(heredoc)) => rendered.push_str(&heredoc.delimiter.raw.render_syntax(self.source)),
            (None, None) => {}
            (Some(_), Some(_)) => unreachable!("redirect target cannot be both word and heredoc"),
        }

        rendered
    }

    fn should_render_explicit_fd(&self, fd: i32, kind: RedirectKind) -> bool {
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

    fn render_assignment(&self, assignment: &Assignment) -> String {
        let mut rendered = assignment.target.name.to_string();
        if let Some(index) = &assignment.target.subscript {
            rendered.push('[');
            rendered.push_str(&self.render_subscript(index));
            rendered.push(']');
        }
        if assignment.append {
            rendered.push_str("+=");
        } else {
            rendered.push('=');
        }
        match &assignment.value {
            AssignmentValue::Scalar(value) => rendered.push_str(&value.render_syntax(self.source)),
            AssignmentValue::Compound(array) => {
                rendered.push('(');
                for (index, value) in array.elements.iter().enumerate() {
                    if index > 0 {
                        rendered.push(' ');
                    }
                    rendered.push_str(&self.render_array_elem(value));
                }
                rendered.push(')');
            }
        }
        trim_unescaped_trailing_whitespace(&rendered).to_string()
    }

    fn render_array_elem(&self, element: &ArrayElem) -> String {
        match element {
            ArrayElem::Sequential(word) => word.render_syntax(self.source),
            ArrayElem::Keyed { key, value } => {
                format!("[{}]={}", self.render_subscript(key), value.render_syntax(self.source))
            }
            ArrayElem::KeyedAppend { key, value } => format!(
                "[{}]+={}",
                self.render_subscript(key),
                value.render_syntax(self.source)
            ),
        }
    }

    fn render_var_ref(&self, reference: &VarRef) -> String {
        let mut rendered = reference.name.to_string();
        if let Some(subscript) = &reference.subscript {
            rendered.push('[');
            rendered.push_str(&self.render_subscript(subscript));
            rendered.push(']');
        }
        rendered
    }

    fn render_subscript(&self, subscript: &Subscript) -> String {
        if let Some(selector) = subscript.selector() {
            return selector.as_char().to_string();
        }

        self.render_source_text(subscript.syntax_source_text())
    }

    fn render_source_text(&self, text: &SourceText) -> String {
        if text.is_source_backed() && text.span().end.offset > self.source.len() {
            String::new()
        } else {
            text.slice(self.source).to_string()
        }
    }
}

fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text[..whitespace_start]
            .as_bytes()
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}
