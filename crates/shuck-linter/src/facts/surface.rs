use super::*;

#[derive(Debug, Default)]
pub(super) struct SurfaceFragmentFacts {
    pub(super) single_quoted: Vec<SingleQuotedFragmentFact>,
    pub(super) open_double_quotes: Vec<OpenDoubleQuoteFragmentFact>,
    pub(super) backticks: Vec<BacktickFragmentFact>,
    pub(super) legacy_arithmetic: Vec<LegacyArithmeticFragmentFact>,
    pub(super) positional_parameters: Vec<PositionalParameterFragmentFact>,
    pub(super) subscript_spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SurfaceScanContext<'a> {
    command_name: Option<&'a str>,
    assignment_target: Option<&'a str>,
    variable_set_operand: bool,
    collect_open_double_quotes: bool,
}

impl<'a> SurfaceScanContext<'a> {
    fn new() -> Self {
        Self {
            collect_open_double_quotes: true,
            ..Self::default()
        }
    }

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

    fn without_open_double_quote_scan(self) -> Self {
        Self {
            collect_open_double_quotes: false,
            ..self
        }
    }
}

struct SurfaceFragmentCollector<'a> {
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &'a CommandLookupIndex,
    source: &'a str,
    facts: SurfaceFragmentFacts,
}

impl<'a> SurfaceFragmentCollector<'a> {
    fn new(
        commands: &'a [CommandFact<'a>],
        command_ids_by_span: &'a CommandLookupIndex,
        source: &'a str,
    ) -> Self {
        Self {
            commands,
            command_ids_by_span,
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
            ..SurfaceScanContext::new()
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
                    self.collect_word(&entry.word, context);
                }
                self.collect_command(&function.body);
            }
            Command::AnonymousFunction(function) => {
                for word in &function.args {
                    self.collect_word(word, context);
                }
                self.collect_command(&function.body);
            }
        }

        self.collect_redirects(&stmt.redirects, SurfaceScanContext::new());
    }

    fn collect_simple_command(&mut self, command: &SimpleCommand, context: SurfaceScanContext<'_>) {
        self.collect_assignments(&command.assignments, context);
        self.collect_word(&command.name, context);

        if context.command_name == Some("unset") {
            for word in &command.args {
                if word_looks_like_unset_array_target(word, self.source) {
                    self.facts.subscript_spans.push(word.span);
                }
            }
        }

        let variable_set_operand = simple_command_variable_set_operand(command, self.source);
        for word in &command.args {
            let word_context =
                if variable_set_operand.is_some_and(|operand| std::ptr::eq(word, operand)) {
                    context.variable_set_operand()
                } else {
                    context
                };
            self.collect_word(word, word_context);
        }
    }

    fn collect_builtin(&mut self, command: &BuiltinCommand) {
        let context = SurfaceScanContext::new();
        match command {
            BuiltinCommand::Break(command) => {
                self.collect_assignments(&command.assignments, context);
                if let Some(word) = &command.depth {
                    self.collect_word(word, context);
                }
                self.collect_words(&command.extra_args, context);
            }
            BuiltinCommand::Continue(command) => {
                self.collect_assignments(&command.assignments, context);
                if let Some(word) = &command.depth {
                    self.collect_word(word, context);
                }
                self.collect_words(&command.extra_args, context);
            }
            BuiltinCommand::Return(command) => {
                self.collect_assignments(&command.assignments, context);
                if let Some(word) = &command.code {
                    self.collect_word(word, context);
                }
                self.collect_words(&command.extra_args, context);
            }
            BuiltinCommand::Exit(command) => {
                self.collect_assignments(&command.assignments, context);
                if let Some(word) = &command.code {
                    self.collect_word(word, context);
                }
                self.collect_words(&command.extra_args, context);
            }
        }
    }

    fn collect_decl_command(&mut self, command: &DeclClause) {
        let context = SurfaceScanContext::new();
        self.collect_assignments(&command.assignments, context);
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                    self.collect_word(word, context);
                }
                DeclOperand::Name(reference) => {
                    self.record_var_ref_subscript(reference);
                    query::visit_var_ref_subscript_words_with_source(
                        reference,
                        self.source,
                        &mut |word| self.collect_word(word, context),
                    );
                }
                DeclOperand::Assignment(assignment) => self.collect_assignment(assignment, context),
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
                    self.collect_words(words, SurfaceScanContext::new());
                }
                self.collect_commands(&command.body);
            }
            CompoundCommand::Repeat(command) => {
                self.collect_word(&command.count, SurfaceScanContext::new());
                self.collect_commands(&command.body);
            }
            CompoundCommand::Foreach(command) => {
                self.collect_words(&command.words, SurfaceScanContext::new());
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
                self.collect_word(&command.word, SurfaceScanContext::new());
                for case in &command.cases {
                    self.collect_patterns(&case.patterns, SurfaceScanContext::new());
                    self.collect_commands(&case.body);
                }
            }
            CompoundCommand::Select(command) => {
                self.collect_words(&command.words, SurfaceScanContext::new());
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
                self.collect_conditional_expr(&command.expression, SurfaceScanContext::new());
            }
            CompoundCommand::Coproc(command) => self.collect_command(&command.body),
        }
    }

    fn collect_assignments(&mut self, assignments: &[Assignment], context: SurfaceScanContext<'_>) {
        for assignment in assignments {
            self.collect_assignment(assignment, context);
        }
    }

    fn collect_assignment(&mut self, assignment: &Assignment, context: SurfaceScanContext<'_>) {
        let context = context.with_assignment_target(assignment.target.name.as_str());
        self.record_var_ref_subscript(&assignment.target);
        query::visit_var_ref_subscript_words_with_source(
            &assignment.target,
            self.source,
            &mut |word| self.collect_word(word, context),
        );
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.collect_word(word, context),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.collect_word(word, context),
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            self.record_subscript(Some(key));
                            query::visit_subscript_words(Some(key), self.source, &mut |word| {
                                self.collect_word(word, context);
                            });
                            self.collect_word(value, context);
                        }
                    }
                }
            }
        }
    }

    fn collect_words(&mut self, words: &[Word], context: SurfaceScanContext<'_>) {
        for word in words {
            self.collect_word(word, context);
        }
    }

    fn collect_patterns(&mut self, patterns: &[Pattern], context: SurfaceScanContext<'_>) {
        for pattern in patterns {
            self.collect_pattern(pattern, context);
        }
    }

    fn collect_word(&mut self, word: &Word, context: SurfaceScanContext<'_>) {
        if context.collect_open_double_quotes && context.assignment_target.is_none() {
            self.collect_open_double_quote_fragments(word);
        }
        self.collect_word_parts(&word.parts, context);
    }

    fn collect_open_double_quote_fragments(&mut self, word: &Word) {
        for (index, part) in word.parts.iter().enumerate() {
            let WordPart::DoubleQuoted { .. } = &part.kind else {
                continue;
            };
            if !part.span.slice(self.source).contains('\n') {
                continue;
            }
            let Some(next_double_quoted_index) = word.parts[index + 1..]
                .iter()
                .position(|later| matches!(later.kind, WordPart::DoubleQuoted { .. }))
                .map(|relative_index| index + 1 + relative_index)
            else {
                continue;
            };
            if word.parts[index + 1..next_double_quoted_index]
                .iter()
                .any(|between| {
                    matches!(
                        between.kind,
                        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. }
                    )
                })
            {
                continue;
            }
            let Some(span) = opening_double_quote_span(part.span, self.source) else {
                continue;
            };
            self.facts
                .open_double_quotes
                .push(OpenDoubleQuoteFragmentFact { span });
        }
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
                WordPart::DoubleQuoted { parts, .. } => self.collect_word_parts(parts, context),
                WordPart::ZshQualifiedGlob(glob) => self.collect_zsh_qualified_glob(glob, context),
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
                            self.collect_word(word, context);
                        });
                    }
                }
                WordPart::ArithmeticExpansion { expression_ast, .. } => {
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context);
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
                    self.record_parameter_subscripts(parameter);
                    if let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                        operator,
                        ..
                    }) = &parameter.syntax
                    {
                        self.collect_parameter_operator_patterns(operator, context);
                    }
                }
                WordPart::ParameterExpansion { operator, .. } => {
                    if let WordPart::ParameterExpansion { reference, .. } = &part.kind {
                        self.record_var_ref_subscript(reference);
                    }
                    self.collect_parameter_operator_patterns(operator, context);
                }
                WordPart::Length(reference)
                | WordPart::ArrayAccess(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::ArrayIndices(reference)
                | WordPart::Transformation { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::Substring { reference, .. } | WordPart::ArraySlice { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator: Some(operator),
                    ..
                } => {
                    self.record_var_ref_subscript(reference);
                    self.collect_parameter_operator_patterns(operator, context);
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator: None,
                    ..
                } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {}
            }
        }
    }

    fn collect_pattern(&mut self, pattern: &Pattern, context: SurfaceScanContext<'_>) {
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => self.collect_patterns(patterns, context),
                PatternPart::Word(word) => self.collect_word(word, context),
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
                self.collect_pattern(pattern, context);
            }
        }
    }

    fn collect_redirects(&mut self, redirects: &[Redirect], context: SurfaceScanContext<'_>) {
        for redirect in redirects {
            match redirect.word_target() {
                Some(word) => self.collect_word(word, context),
                None => {
                    let heredoc = redirect.heredoc().expect("expected heredoc redirect");
                    if heredoc.delimiter.expands_body {
                        self.collect_word(&heredoc.body, context.without_open_double_quote_scan());
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
                self.collect_conditional_expr(&expr.left, context);
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
                self.record_var_ref_subscript(reference);
                query::visit_var_ref_subscript_words_with_source(
                    reference,
                    self.source,
                    &mut |word| self.collect_word(word, context),
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

    fn record_parameter_subscripts(&mut self, parameter: &shuck_ast::ParameterExpansion) {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Indirect { reference, .. }
                | BourneParameterExpansion::Slice { reference, .. }
                | BourneParameterExpansion::Operation { reference, .. }
                | BourneParameterExpansion::Transformation { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
            },
            ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
                ZshExpansionTarget::Reference(reference) => {
                    self.record_var_ref_subscript(reference)
                }
                ZshExpansionTarget::Nested(parameter) => {
                    self.record_parameter_subscripts(parameter)
                }
                ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => {}
            },
        }
    }

    fn record_var_ref_subscript(&mut self, reference: &VarRef) {
        self.record_subscript(reference.subscript.as_ref());
    }

    fn record_subscript(&mut self, subscript: Option<&Subscript>) {
        let Some(subscript) = subscript else {
            return;
        };
        if subscript.selector().is_some() {
            return;
        }
        self.facts.subscript_spans.push(subscript.span());
    }

    fn command_fact_for_command(&self, command: &Command) -> Option<&CommandFact<'a>> {
        command_fact_for_command(command, self.commands, self.command_ids_by_span)
    }
}

pub(super) fn build_surface_fragment_facts<'a>(
    file: &'a File,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &'a CommandLookupIndex,
    source: &'a str,
) -> SurfaceFragmentFacts {
    let mut collector = SurfaceFragmentCollector::new(commands, command_ids_by_span, source);
    collector.collect_commands(&file.body);
    collector.finish()
}

pub(super) fn build_subscript_index_reference_spans(
    semantic: &SemanticModel,
    subscript_spans: &[Span],
) -> FxHashSet<FactSpan> {
    if subscript_spans.is_empty() {
        return FxHashSet::default();
    }

    let references = semantic.references();
    if references.len().saturating_mul(subscript_spans.len()) <= 4_096 {
        return build_subscript_index_reference_spans_linear(references, subscript_spans);
    }

    let subscript_index = SubscriptSpanIndex::new(subscript_spans);
    references
        .iter()
        .filter(|reference| subscript_index.contains(reference.span))
        .map(|reference| FactSpan::new(reference.span))
        .collect()
}

fn build_subscript_index_reference_spans_linear(
    references: &[shuck_semantic::Reference],
    subscript_spans: &[Span],
) -> FxHashSet<FactSpan> {
    references
        .iter()
        .filter(|reference| {
            subscript_spans
                .iter()
                .any(|subscript| span_contains(*subscript, reference.span))
        })
        .map(|reference| FactSpan::new(reference.span))
        .collect()
}

#[derive(Debug, Default)]
struct SubscriptSpanIndex {
    starts: Vec<usize>,
    prefix_max_ends: Vec<usize>,
}

impl SubscriptSpanIndex {
    fn new(subscript_spans: &[Span]) -> Self {
        let mut bounds = subscript_spans
            .iter()
            .map(|span| (span.start.offset, span.end.offset))
            .collect::<Vec<_>>();
        bounds.sort_unstable();

        let mut starts = Vec::with_capacity(bounds.len());
        let mut prefix_max_ends = Vec::with_capacity(bounds.len());
        let mut max_end = 0usize;

        for (start, end) in bounds {
            starts.push(start);
            max_end = max_end.max(end);
            prefix_max_ends.push(max_end);
        }

        Self {
            starts,
            prefix_max_ends,
        }
    }

    fn contains(&self, span: Span) -> bool {
        let candidate_count = self
            .starts
            .partition_point(|start| *start <= span.start.offset);
        candidate_count > 0 && self.prefix_max_ends[candidate_count - 1] >= span.end.offset
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn word_looks_like_unset_array_target(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    let Some((name, _)) = text.split_once('[') else {
        return false;
    };
    text.ends_with(']') && is_shell_name(name)
}

fn is_shell_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|char| char == '_' || char.is_ascii_alphanumeric())
}

fn opening_double_quote_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let quote_offset = text.find('"')?;
    let start = span.start.advanced_by(&text[..quote_offset]);
    Some(Span::from_positions(start, start))
}

fn simple_command_variable_set_operand<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
    let operands = simple_test_operands(command, source)?;
    (operands.len() == 2 && static_word_text(&operands[0], source).as_deref() == Some("-v"))
        .then(|| &operands[1])
}

#[cfg(test)]
mod tests {
    use super::SubscriptSpanIndex;
    use shuck_ast::{Position, Span};

    fn span(start: usize, end: usize) -> Span {
        Span::from_positions(
            Position {
                line: 1,
                column: start + 1,
                offset: start,
            },
            Position {
                line: 1,
                column: end + 1,
                offset: end,
            },
        )
    }

    #[test]
    fn subscript_span_index_uses_prefix_max_for_containment() {
        let index = SubscriptSpanIndex::new(&[span(50, 60), span(0, 100), span(120, 130)]);

        assert!(index.contains(span(55, 56)));
        assert!(index.contains(span(80, 90)));
        assert!(index.contains(span(99, 100)));
        assert!(!index.contains(span(100, 101)));
        assert!(!index.contains(span(110, 115)));
    }
}
