use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BourneParameterExpansion, BuiltinCommand, Command, CompoundCommand, ConditionalExpr,
    DeclOperand, File, ForSyntax, ForeachSyntax, HeredocBody, HeredocBodyPart, IfSyntax,
    ParameterExpansion, ParameterExpansionSyntax, Pattern, PatternPart, Redirect, RedirectTarget,
    RepeatSyntax, Span, Stmt, StmtSeq, TextRange, TextSize, VarRef, Word, WordPart,
};

/// The source delimiter that closes a parsed shell compound command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CloseDelimiterKind {
    /// The `fi` delimiter for `if` commands.
    Fi,
    /// The `done` delimiter for loop bodies.
    Done,
    /// The `esac` delimiter for `case` commands.
    Esac,
    /// The `}` delimiter for brace-backed compound bodies.
    RightBrace,
}

impl CloseDelimiterKind {
    fn text(self) -> &'static str {
        match self {
            Self::Fi => "fi",
            Self::Done => "done",
            Self::Esac => "esac",
            Self::RightBrace => "}",
        }
    }
}

/// One indexed close delimiter and the compound command span it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedCloseDelimiter {
    command_range: TextRange,
    delimiter_range: TextRange,
    kind: CloseDelimiterKind,
}

impl IndexedCloseDelimiter {
    /// Return the parsed compound command range that owns this delimiter.
    pub fn command_range(self) -> TextRange {
        self.command_range
    }

    /// Return the source byte range for the close delimiter itself.
    pub fn delimiter_range(self) -> TextRange {
        self.delimiter_range
    }

    /// Return the delimiter kind.
    pub fn kind(self) -> CloseDelimiterKind {
        self.kind
    }
}

/// Lookup table for structural close delimiters in parsed shell source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloseDelimiterIndex {
    delimiters: Vec<IndexedCloseDelimiter>,
}

impl CloseDelimiterIndex {
    /// Build close-delimiter metadata from parser output.
    ///
    /// The index uses parser-owned syntax spans and ignores compound forms
    /// without a concrete close delimiter in source.
    pub fn new(source: &str, file: &File) -> Self {
        let mut collector = CloseDelimiterCollector::new(source);
        collector.visit_file(file);
        collector.finish()
    }

    /// Return an empty close-delimiter index.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Return all indexed close delimiters in source order.
    pub fn delimiters(&self) -> &[IndexedCloseDelimiter] {
        &self.delimiters
    }

    /// Return the close delimiter range for `command_range` and `kind`.
    pub fn close_for_command(
        &self,
        command_range: TextRange,
        kind: CloseDelimiterKind,
    ) -> Option<TextRange> {
        let start_index = self
            .delimiters
            .partition_point(|entry| entry.command_range.start() < command_range.start());
        for entry in &self.delimiters[start_index..] {
            if entry.command_range.start() != command_range.start() {
                break;
            }
            if entry.command_range == command_range && entry.kind == kind {
                return Some(entry.delimiter_range);
            }
        }
        None
    }
}

struct CloseDelimiterCollector<'source> {
    source: &'source str,
    delimiters: Vec<IndexedCloseDelimiter>,
}

impl<'source> CloseDelimiterCollector<'source> {
    fn new(source: &'source str) -> Self {
        Self {
            source,
            delimiters: Vec::new(),
        }
    }

    fn finish(mut self) -> CloseDelimiterIndex {
        self.delimiters.sort_unstable_by_key(|entry| {
            (
                entry.command_range.start().to_u32(),
                entry.command_range.end().to_u32(),
                entry.kind,
                entry.delimiter_range.start().to_u32(),
            )
        });
        self.delimiters
            .dedup_by_key(|entry| (entry.command_range, entry.kind, entry.delimiter_range));
        CloseDelimiterIndex {
            delimiters: self.delimiters,
        }
    }

    fn visit_file(&mut self, file: &File) {
        self.visit_stmt_seq(&file.body);
    }

    fn visit_stmt_seq(&mut self, sequence: &StmtSeq) {
        for stmt in sequence.iter() {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        for redirect in &stmt.redirects {
            self.visit_redirect(redirect);
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
            Command::Builtin(command) => {
                self.visit_builtin(command);
            }
            Command::Decl(command) => {
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            self.visit_word(word);
                        }
                        DeclOperand::Name(reference) => self.visit_var_ref(reference),
                        DeclOperand::Assignment(assignment) => self.visit_assignment(assignment),
                    }
                }
            }
            Command::Binary(command) => {
                self.visit_stmt(&command.left);
                self.visit_stmt(&command.right);
            }
            Command::Compound(command) => self.visit_compound(command),
            Command::Function(function) => {
                for entry in &function.header.entries {
                    self.visit_word(&entry.word);
                }
                self.visit_stmt(&function.body);
            }
            Command::AnonymousFunction(function) => {
                self.visit_stmt(&function.body);
                for word in &function.args {
                    self.visit_word(word);
                }
            }
        }
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand) {
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

    fn visit_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
                match command.syntax {
                    IfSyntax::ThenFi { fi_span, .. } => {
                        self.push_span(command.span, CloseDelimiterKind::Fi, fi_span);
                    }
                    IfSyntax::Brace {
                        right_brace_span, ..
                    } => {
                        self.push_span(
                            command.span,
                            CloseDelimiterKind::RightBrace,
                            right_brace_span,
                        );
                    }
                }
                self.visit_stmt_seq(&command.condition);
                self.visit_stmt_seq(&command.then_branch);
                for (condition, branch) in &command.elif_branches {
                    self.visit_stmt_seq(condition);
                    self.visit_stmt_seq(branch);
                }
                if let Some(branch) = &command.else_branch {
                    self.visit_stmt_seq(branch);
                }
            }
            CompoundCommand::For(command) => {
                match command.syntax {
                    ForSyntax::InDoDone { done_span, .. }
                    | ForSyntax::ParenDoDone { done_span, .. } => {
                        self.push_span(command.span, CloseDelimiterKind::Done, done_span);
                    }
                    ForSyntax::InBrace {
                        right_brace_span, ..
                    }
                    | ForSyntax::ParenBrace {
                        right_brace_span, ..
                    } => {
                        self.push_span(
                            command.span,
                            CloseDelimiterKind::RightBrace,
                            right_brace_span,
                        );
                    }
                    ForSyntax::InDirect { .. } | ForSyntax::ParenDirect { .. } => {}
                }
                if let Some(words) = &command.words {
                    for word in words {
                        self.visit_word(word);
                    }
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Repeat(command) => {
                match command.syntax {
                    RepeatSyntax::DoDone { done_span, .. } => {
                        self.push_span(command.span, CloseDelimiterKind::Done, done_span);
                    }
                    RepeatSyntax::Brace {
                        right_brace_span, ..
                    } => {
                        self.push_span(
                            command.span,
                            CloseDelimiterKind::RightBrace,
                            right_brace_span,
                        );
                    }
                    RepeatSyntax::Direct => {}
                }
                self.visit_word(&command.count);
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Foreach(command) => {
                match command.syntax {
                    ForeachSyntax::InDoDone { done_span, .. } => {
                        self.push_span(command.span, CloseDelimiterKind::Done, done_span);
                    }
                    ForeachSyntax::ParenBrace {
                        right_brace_span, ..
                    } => {
                        self.push_span(
                            command.span,
                            CloseDelimiterKind::RightBrace,
                            right_brace_span,
                        );
                    }
                }
                for word in &command.words {
                    self.visit_word(word);
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::ArithmeticFor(command) => {
                if let Some(done_span) = command.done_span {
                    self.push_span(command.span, CloseDelimiterKind::Done, done_span);
                }
                if let Some(expr) = &command.init_ast {
                    self.visit_arithmetic_expr(expr);
                }
                if let Some(expr) = &command.condition_ast {
                    self.visit_arithmetic_expr(expr);
                }
                if let Some(expr) = &command.step_ast {
                    self.visit_arithmetic_expr(expr);
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::While(command) => {
                if let Some(done_span) = command.done_span {
                    self.push_span(command.span, CloseDelimiterKind::Done, done_span);
                }
                self.visit_stmt_seq(&command.condition);
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Until(command) => {
                if let Some(done_span) = command.done_span {
                    self.push_span(command.span, CloseDelimiterKind::Done, done_span);
                }
                self.visit_stmt_seq(&command.condition);
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Case(command) => {
                self.push_span(command.span, CloseDelimiterKind::Esac, command.esac_span);
                self.visit_word(&command.word);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        self.visit_pattern(pattern);
                    }
                    self.visit_stmt_seq(&item.body);
                }
            }
            CompoundCommand::Select(command) => {
                self.push_span(command.span, CloseDelimiterKind::Done, command.done_span);
                for word in &command.words {
                    self.visit_word(word);
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Subshell(sequence) | CompoundCommand::BraceGroup(sequence) => {
                self.visit_stmt_seq(sequence);
            }
            CompoundCommand::Always(command) => {
                self.visit_stmt_seq(&command.body);
                self.visit_stmt_seq(&command.always_body);
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.visit_stmt(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.visit_conditional_expr(&command.expression);
            }
            CompoundCommand::Coproc(command) => self.visit_stmt(&command.body),
            CompoundCommand::Arithmetic(command) => {
                if let Some(expr) = &command.expr_ast {
                    self.visit_arithmetic_expr(expr);
                }
            }
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        self.visit_var_ref(&assignment.target);
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.visit_word(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.visit_word(word),
                        ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                            self.visit_word(value);
                        }
                    }
                }
            }
        }
    }

    fn visit_var_ref(&mut self, reference: &VarRef) {
        if let Some(subscript) = reference.subscript.as_deref()
            && let Some(word) = subscript.word_ast()
        {
            self.visit_word(word);
        }
        if let Some(subscript) = reference.subscript.as_deref()
            && let Some(expr) = &subscript.arithmetic_ast
        {
            self.visit_arithmetic_expr(expr);
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

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr) {
        match expression {
            ConditionalExpr::Binary(expression) => {
                self.visit_conditional_expr(&expression.left);
                self.visit_conditional_expr(&expression.right);
            }
            ConditionalExpr::Unary(expression) => self.visit_conditional_expr(&expression.expr),
            ConditionalExpr::Parenthesized(expression) => {
                self.visit_conditional_expr(&expression.expr);
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => self.visit_word(word),
            ConditionalExpr::Pattern(pattern) => self.visit_pattern(pattern),
            ConditionalExpr::VarRef(reference) => self.visit_var_ref(reference),
        }
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        match &redirect.target {
            RedirectTarget::Word(word) => self.visit_word(word),
            RedirectTarget::Heredoc(heredoc) => {
                self.visit_word(&heredoc.delimiter.raw);
                self.visit_heredoc_body(&heredoc.body);
            }
        }
    }

    fn visit_heredoc_body(&mut self, body: &HeredocBody) {
        for part in &body.parts {
            match &part.kind {
                HeredocBodyPart::CommandSubstitution { body, .. } => self.visit_stmt_seq(body),
                HeredocBodyPart::ArithmeticExpansion {
                    expression_ast,
                    expression_word_ast,
                    ..
                } => {
                    if let Some(expr) = expression_ast {
                        self.visit_arithmetic_expr(expr);
                    }
                    self.visit_word(expression_word_ast);
                }
                HeredocBodyPart::Parameter(parameter) => self.visit_parameter(parameter),
                HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
            }
        }
    }

    fn visit_word(&mut self, word: &Word) {
        for part in &word.parts {
            self.visit_word_part(&part.kind);
        }
    }

    fn visit_word_part(&mut self, part: &WordPart) {
        match part {
            WordPart::DoubleQuoted { parts, .. } => {
                for nested in parts {
                    self.visit_word_part(&nested.kind);
                }
            }
            WordPart::CommandSubstitution { body, .. } => self.visit_stmt_seq(body),
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expr) = expression_ast.as_deref() {
                    self.visit_arithmetic_expr(expr);
                }
                self.visit_word(expression_word_ast);
            }
            WordPart::Parameter(parameter) => self.visit_parameter(parameter),
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.visit_var_ref(reference);
                self.visit_parameter_operator(operator);
                if let Some(operand) = operand_word_ast.as_deref() {
                    self.visit_word(operand);
                }
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.visit_var_ref(reference);
                if let Some(operator) = operator.as_deref() {
                    self.visit_parameter_operator(operator);
                }
                if let Some(operand) = operand_word_ast.as_deref() {
                    self.visit_word(operand);
                }
            }
            WordPart::Substring {
                reference,
                offset_word_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.visit_var_ref(reference);
                self.visit_word(offset_word_ast);
                if let Some(length) = length_word_ast.as_deref() {
                    self.visit_word(length);
                }
            }
            WordPart::ProcessSubstitution { body, .. } => self.visit_stmt_seq(body),
            WordPart::Transformation { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference) => self.visit_var_ref(reference),
            WordPart::Variable(_)
            | WordPart::Literal(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::PrefixMatch { .. } => {}
        }
    }

    fn visit_parameter(&mut self, parameter: &ParameterExpansion) {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => self.visit_bourne_parameter(syntax),
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &syntax.target {
                    shuck_ast::ZshExpansionTarget::Reference(reference) => {
                        self.visit_var_ref(reference);
                    }
                    shuck_ast::ZshExpansionTarget::Nested(parameter) => {
                        self.visit_parameter(parameter);
                    }
                    shuck_ast::ZshExpansionTarget::Word(word) => self.visit_word(word),
                    shuck_ast::ZshExpansionTarget::Empty => {}
                }
                for modifier in &syntax.modifiers {
                    if let Some(word) = modifier.argument_word_ast() {
                        self.visit_word(word);
                    }
                }
                if let Some(operation) = &syntax.operation {
                    if let Some(word) = operation.operand_word_ast() {
                        self.visit_word(word);
                    }
                    if let Some(word) = operation.pattern_word_ast() {
                        self.visit_word(word);
                    }
                    if let Some(word) = operation.replacement_word_ast() {
                        self.visit_word(word);
                    }
                    if let Some(word) = operation.offset_word_ast() {
                        self.visit_word(word);
                    }
                    if let Some(word) = operation.length_word_ast() {
                        self.visit_word(word);
                    }
                }
            }
        }
    }

    fn visit_bourne_parameter(&mut self, syntax: &BourneParameterExpansion) {
        if let Some(word) = syntax.operand_word_ast() {
            self.visit_word(word);
        }
        if let Some(word) = syntax.offset_word_ast() {
            self.visit_word(word);
        }
        if let Some(word) = syntax.length_word_ast() {
            self.visit_word(word);
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

    fn visit_parameter_operator(&mut self, operator: &shuck_ast::ParameterOp) {
        match operator {
            shuck_ast::ParameterOp::RemovePrefixShort { pattern }
            | shuck_ast::ParameterOp::RemovePrefixLong { pattern }
            | shuck_ast::ParameterOp::RemoveSuffixShort { pattern }
            | shuck_ast::ParameterOp::RemoveSuffixLong { pattern } => self.visit_pattern(pattern),
            shuck_ast::ParameterOp::ReplaceFirst { pattern, .. }
            | shuck_ast::ParameterOp::ReplaceAll { pattern, .. } => self.visit_pattern(pattern),
            shuck_ast::ParameterOp::UseDefault
            | shuck_ast::ParameterOp::AssignDefault
            | shuck_ast::ParameterOp::UseReplacement
            | shuck_ast::ParameterOp::Error
            | shuck_ast::ParameterOp::UpperFirst
            | shuck_ast::ParameterOp::UpperAll
            | shuck_ast::ParameterOp::LowerFirst
            | shuck_ast::ParameterOp::LowerAll => {}
        }
        if let Some(word) = operator.replacement_word_ast() {
            self.visit_word(word);
        }
    }

    fn push_span(&mut self, command_span: Span, kind: CloseDelimiterKind, delimiter_span: Span) {
        let Some(delimiter_range) = self.delimiter_range_from_span(delimiter_span, kind.text())
        else {
            return;
        };
        self.push_range(command_span, kind, delimiter_range);
    }

    fn push_range(
        &mut self,
        command_span: Span,
        kind: CloseDelimiterKind,
        delimiter_range: TextRange,
    ) {
        let command_range = command_span.to_range();
        if command_range.is_empty() || !self.range_is_valid(command_range) {
            return;
        }
        self.delimiters.push(IndexedCloseDelimiter {
            command_range,
            delimiter_range,
            kind,
        });
    }

    fn delimiter_range_from_span(&self, span: Span, text: &str) -> Option<TextRange> {
        let start = span.start.offset;
        let start_end = start.checked_add(text.len())?;
        if self.source.get(start..start_end) == Some(text) {
            return Some(text_range(start, start_end));
        }

        let end = span.end.offset;
        let end_start = end.checked_sub(text.len())?;
        if self.source.get(end_start..end) == Some(text) {
            return Some(text_range(end_start, end));
        }

        None
    }

    fn range_is_valid(&self, range: TextRange) -> bool {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        start < end && end <= self.source.len()
    }
}

fn text_range(start: usize, end: usize) -> TextRange {
    TextRange::new(TextSize::new(start as u32), TextSize::new(end as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Indexer, IndexerOptions};
    use shuck_parser::parser::Parser;

    fn parse_with_close_index(source: &str) -> (File, CloseDelimiterIndex) {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new_with_options(
            source,
            &output,
            IndexerOptions::new().with_source_layout_indexes(true),
        );
        (output.file, indexer.close_delimiter_index().clone())
    }

    fn compound_range(stmt: &Stmt) -> TextRange {
        let Command::Compound(command) = &stmt.command else {
            panic!("expected compound command");
        };
        match command {
            CompoundCommand::If(command) => command.span.to_range(),
            CompoundCommand::While(command) => command.span.to_range(),
            CompoundCommand::Case(command) => command.span.to_range(),
            other => panic!("unexpected compound command: {other:?}"),
        }
    }

    fn keyword_range(source: &str, marker: &str, keyword: &str) -> TextRange {
        let start = source.find(marker).unwrap();
        text_range(start, start + keyword.len())
    }

    #[test]
    fn indexes_close_keywords_without_following_command_bleed() {
        let source = "\
if true; then
  echo fi
fi # close
echo after
while true; do
  echo done
done # loop
case $x in
  a) echo esac ;;
esac # case
";
        let (file, index) = parse_with_close_index(source);

        assert_eq!(
            index.close_for_command(compound_range(&file.body[0]), CloseDelimiterKind::Fi),
            Some(keyword_range(source, "fi # close", "fi"))
        );
        assert_eq!(
            index.close_for_command(compound_range(&file.body[2]), CloseDelimiterKind::Done),
            Some(keyword_range(source, "done # loop", "done"))
        );
        assert_eq!(
            index.close_for_command(compound_range(&file.body[3]), CloseDelimiterKind::Esac),
            Some(keyword_range(source, "esac # case", "esac"))
        );
    }

    #[test]
    fn close_delimiters_are_source_layout_metadata() {
        let source = "if true; then echo ok; fi\n";
        let output = Parser::new(source).parse().unwrap();
        let default_indexer = Indexer::new(source, &output);
        let layout_indexer = Indexer::new_with_options(
            source,
            &output,
            IndexerOptions::new().with_source_layout_indexes(true),
        );

        assert!(
            default_indexer
                .close_delimiter_index()
                .delimiters()
                .is_empty()
        );
        assert_eq!(layout_indexer.close_delimiter_index().delimiters().len(), 1);
    }
}
