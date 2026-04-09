use super::*;

#[derive(Debug, Default)]
pub(super) struct SurfaceFragmentFacts {
    pub(super) single_quoted: Vec<SingleQuotedFragmentFact>,
    pub(super) open_double_quotes: Vec<OpenDoubleQuoteFragmentFact>,
    pub(super) backticks: Vec<BacktickFragmentFact>,
    pub(super) legacy_arithmetic: Vec<LegacyArithmeticFragmentFact>,
    pub(super) positional_parameters: Vec<PositionalParameterFragmentFact>,
    pub(super) positional_parameter_operator_spans: Vec<Span>,
    pub(super) unicode_smart_quote_spans: Vec<Span>,
    pub(super) nested_parameter_expansions: Vec<NestedParameterExpansionFragmentFact>,
    pub(super) indirect_expansions: Vec<IndirectExpansionFragmentFact>,
    pub(super) indexed_array_references: Vec<IndexedArrayReferenceFragmentFact>,
    pub(super) substring_expansions: Vec<SubstringExpansionFragmentFact>,
    pub(super) case_modifications: Vec<CaseModificationFragmentFact>,
    pub(super) replacement_expansions: Vec<ReplacementExpansionFragmentFact>,
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

    fn record_array_reference(&mut self, span: Span) {
        let Some(span) = plain_array_reference_span(span, self.source) else {
            return;
        };
        self.facts
            .indexed_array_references
            .push(IndexedArrayReferenceFragmentFact { span });
    }

    fn record_substring_expansion(&mut self, span: Span) {
        let Some(span) = plain_substring_expansion_span(span, self.source) else {
            return;
        };
        if self
            .facts
            .substring_expansions
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .substring_expansions
            .push(SubstringExpansionFragmentFact { span });
    }

    fn record_case_modification(&mut self, span: Span) {
        let Some(span) = plain_case_modification_span(span, self.source) else {
            return;
        };
        if self
            .facts
            .case_modifications
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .case_modifications
            .push(CaseModificationFragmentFact { span });
    }

    fn record_replacement_expansion(&mut self, span: Span) {
        let Some(span) = plain_replacement_expansion_span(span, self.source) else {
            return;
        };
        if self
            .facts
            .replacement_expansions
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .replacement_expansions
            .push(ReplacementExpansionFragmentFact { span });
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
        collect_unicode_smart_quote_spans_in_word_parts(
            &word.parts,
            self.source,
            false,
            &mut self.facts.unicode_smart_quote_spans,
        );
        if context.collect_open_double_quotes && context.assignment_target.is_none() {
            self.collect_open_double_quote_fragments(word);
        }
        self.collect_raw_substring_expansions_in_span(word.span);
        self.collect_raw_replacement_expansions_in_span(word.span);
        self.collect_raw_case_modifications_in_span(word.span);
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
                WordPart::SingleQuoted { dollar, .. } => {
                    self.facts.single_quoted.push(SingleQuotedFragmentFact {
                        span: part.span,
                        dollar_quoted: *dollar,
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
                    expression,
                    syntax: ArithmeticExpansionSyntax::LegacyBracket,
                    expression_ast,
                    ..
                } => {
                    self.facts
                        .legacy_arithmetic
                        .push(LegacyArithmeticFragmentFact { span: part.span });
                    collect_positional_parameter_operator_spans_in_arithmetic(
                        part.span,
                        expression,
                        self.source,
                        &mut self.facts.positional_parameter_operator_spans,
                    );
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context);
                        });
                    }
                }
                WordPart::ArithmeticExpansion {
                    expression,
                    expression_ast,
                    ..
                } => {
                    collect_positional_parameter_operator_spans_in_arithmetic(
                        part.span,
                        expression,
                        self.source,
                        &mut self.facts.positional_parameter_operator_spans,
                    );
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
                    if is_nested_parameter_expansion(parameter, self.source) {
                        self.facts
                            .nested_parameter_expansions
                            .push(NestedParameterExpansionFragmentFact { span: part.span });
                    }
                    if parameter_has_array_reference(parameter) {
                        self.record_array_reference(part.span);
                    }
                    if parameter_has_substring_expansion(parameter) {
                        self.record_substring_expansion(parameter.span);
                    }
                    if parameter_has_case_modification(parameter) {
                        self.record_case_modification(parameter.span);
                    }
                    if parameter_has_replacement_expansion(parameter) {
                        self.record_replacement_expansion(parameter.span);
                    }
                    self.record_parameter_subscripts(parameter);
                    if let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax {
                        if matches!(
                            syntax,
                            BourneParameterExpansion::Indirect { .. }
                                | BourneParameterExpansion::PrefixMatch { .. }
                                | BourneParameterExpansion::Indices { .. }
                        ) {
                            self.facts
                                .indirect_expansions
                                .push(IndirectExpansionFragmentFact {
                                    span: part.span,
                                    array_keys: matches!(
                                        syntax,
                                        BourneParameterExpansion::Indices { .. }
                                    ),
                                });
                        }
                        match syntax {
                            BourneParameterExpansion::Operation {
                                operator, operand, ..
                            }
                            | BourneParameterExpansion::Indirect {
                                operator: Some(operator),
                                operand,
                                ..
                            } => {
                                self.collect_parameter_operator_patterns(
                                    operator,
                                    operand.as_ref(),
                                    context,
                                );
                            }
                            BourneParameterExpansion::Access { .. }
                            | BourneParameterExpansion::Length { .. }
                            | BourneParameterExpansion::Indices { .. }
                            | BourneParameterExpansion::Indirect { operator: None, .. }
                            | BourneParameterExpansion::PrefixMatch { .. }
                            | BourneParameterExpansion::Slice { .. }
                            | BourneParameterExpansion::Transformation { .. } => {}
                        }
                    }
                }
                WordPart::Variable(name)
                    if name.as_str() == "$"
                        && contains_nested_parameter_marker(part.span.slice(self.source)) =>
                {
                    self.facts
                        .nested_parameter_expansions
                        .push(NestedParameterExpansionFragmentFact { span: part.span });
                }
                WordPart::ParameterExpansion {
                    operator, operand, ..
                } => {
                    if matches!(
                        operator,
                        ParameterOp::UpperFirst
                            | ParameterOp::UpperAll
                            | ParameterOp::LowerFirst
                            | ParameterOp::LowerAll
                    ) {
                        self.record_case_modification(part.span);
                    }
                    if matches!(
                        operator,
                        ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
                    ) {
                        self.record_replacement_expansion(part.span);
                    }
                    if let WordPart::ParameterExpansion { reference, .. } = &part.kind {
                        self.record_var_ref_subscript(reference);
                    }
                    self.collect_parameter_operator_patterns(operator, operand.as_ref(), context);
                }
                WordPart::Length(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::Transformation { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::ArrayAccess(reference) => {
                    if reference_has_array_subscript(reference) {
                        self.record_array_reference(part.span);
                        let case_modification_span = parts
                            .get(index + 1)
                            .filter(|next_part| {
                                matches!(&next_part.kind, WordPart::Literal(text) if {
                                    let text = text.as_str(self.source, next_part.span);
                                    text.starts_with('^') || text.starts_with(',')
                                })
                            })
                            .map_or(part.span, |next_part| part.span.merge(next_part.span));
                        self.record_case_modification(case_modification_span);
                    }
                    self.record_var_ref_subscript(reference);
                }
                WordPart::ArrayIndices(reference) => {
                    self.record_var_ref_subscript(reference);
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: true,
                        });
                }
                WordPart::Substring { reference, .. } => {
                    self.record_substring_expansion(part.span);
                    self.record_var_ref_subscript(reference);
                }
                WordPart::ArraySlice { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator: Some(operator),
                    operand,
                    ..
                } => {
                    self.record_var_ref_subscript(reference);
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: false,
                        });
                    self.collect_parameter_operator_patterns(operator, operand.as_ref(), context);
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator: None,
                    ..
                } => {
                    self.record_var_ref_subscript(reference);
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: false,
                        });
                }
                WordPart::PrefixMatch { .. } => {
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: false,
                        });
                }
                WordPart::Literal(_) | WordPart::Variable(_) => {}
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

    fn collect_source_text_word(&mut self, text: &SourceText, context: SurfaceScanContext<'_>) {
        let snippet = text.slice(self.source);
        if snippet.is_empty() {
            return;
        }

        self.collect_raw_substring_expansions_in_span(text.span());
        self.collect_raw_case_modifications_in_span(text.span());
        self.collect_raw_replacement_expansions_in_span(text.span());
        let word = Parser::parse_word_fragment(self.source, snippet, text.span());
        self.collect_word(&word, context.without_open_double_quote_scan());
    }

    fn collect_raw_substring_expansions_in_span(&mut self, span: Span) {
        let snippet = span.slice(self.source);
        let mut search_start = 0;

        while let Some(relative_start) = snippet[search_start..].find("${") {
            let start = search_start + relative_start;
            let Some(relative_end) = snippet[start..].find('}') else {
                break;
            };
            let end = start + relative_end + '}'.len_utf8();
            let candidate = &snippet[start..end];
            if is_plain_substring_expansion_text(candidate) {
                let span = Span::from_positions(
                    span.start.advanced_by(&snippet[..start]),
                    span.start.advanced_by(&snippet[..end]),
                );
                self.record_substring_expansion(span);
            }
            search_start = end;
        }
    }

    fn collect_raw_case_modifications_in_span(&mut self, span: Span) {
        let snippet = span.slice(self.source);
        let mut search_start = 0;

        while let Some(relative_start) = snippet[search_start..].find("${") {
            let start = search_start + relative_start;
            let Some(relative_end) = snippet[start..].find('}') else {
                break;
            };
            let end = start + relative_end + '}'.len_utf8();
            let candidate = &snippet[start..end];
            if is_plain_case_modification_text(candidate) {
                let span = Span::from_positions(
                    span.start.advanced_by(&snippet[..start]),
                    span.start.advanced_by(&snippet[..end]),
                );
                self.record_case_modification(span);
            }
            search_start = end;
        }
    }

    fn collect_raw_replacement_expansions_in_span(&mut self, span: Span) {
        let snippet = span.slice(self.source);
        let mut search_start = 0;

        while let Some((start, end)) = next_parameter_expansion_candidate(snippet, search_start) {
            let candidate = &snippet[start..end];
            if is_plain_replacement_expansion_text(candidate) {
                let span = Span::from_positions(
                    span.start.advanced_by(&snippet[..start]),
                    span.start.advanced_by(&snippet[..end]),
                );
                self.record_replacement_expansion(span);
            }
            search_start = end;
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
                        self.collect_raw_substring_expansions_in_span(heredoc.body.span);
                        self.collect_raw_case_modifications_in_span(heredoc.body.span);
                        self.collect_raw_replacement_expansions_in_span(heredoc.body.span);
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
        operand: Option<&SourceText>,
        context: SurfaceScanContext<'_>,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => self.collect_pattern(pattern, context),
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
            } => {
                self.collect_pattern(pattern, context);
                self.collect_source_text_word(replacement, context);
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error => {
                if let Some(operand) = operand {
                    self.collect_source_text_word(operand, context);
                }
            }
            ParameterOp::UpperFirst
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

fn parameter_has_array_reference(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => {
                reference_has_array_subscript(reference)
            }
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Transformation { .. } => false,
            BourneParameterExpansion::PrefixMatch { .. } => false,
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => reference_has_array_subscript(reference),
            ZshExpansionTarget::Nested(parameter) => parameter_has_array_reference(parameter),
            ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_has_substring_expansion(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice { reference, .. }) => {
            reference.subscript.is_none()
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => parameter_has_substring_expansion(parameter),
            ZshExpansionTarget::Reference(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_has_case_modification(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            operator, ..
        }) => {
            matches!(
                operator,
                ParameterOp::UpperFirst
                    | ParameterOp::UpperAll
                    | ParameterOp::LowerFirst
                    | ParameterOp::LowerAll
            )
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => parameter_has_case_modification(parameter),
            ZshExpansionTarget::Reference(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_has_replacement_expansion(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            operator, ..
        }) => {
            matches!(
                operator,
                ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
            )
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => parameter_has_replacement_expansion(parameter),
            ZshExpansionTarget::Reference(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => false,
        },
    }
}

fn reference_has_array_subscript(reference: &VarRef) -> bool {
    reference.subscript.is_some()
}

fn plain_array_reference_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let inner = text.strip_prefix("${")?.strip_suffix('}')?;
    if inner.starts_with('#') || inner.starts_with('!') || !inner.ends_with(']') {
        return None;
    }

    let open = inner.find('[')?;
    let close = inner.rfind(']')?;
    if close != inner.len() - 1 || close <= open {
        return None;
    }

    Some(span)
}

fn plain_substring_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let relative_start = text.find("${")?;
    let start = span.start.advanced_by(&text[..relative_start]);
    let after_start = &source[start.offset..];
    let relative_end = after_start.find('}')?;
    let end = start.advanced_by(&after_start[..relative_end + '}'.len_utf8()]);
    let candidate = &after_start[..relative_end + '}'.len_utf8()];

    is_plain_substring_expansion_text(candidate).then_some(Span::from_positions(start, end))
}

fn is_plain_substring_expansion_text(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };
    if inner.starts_with('#') || inner.starts_with('!') {
        return false;
    }

    let Some(colon_index) = inner.find(':') else {
        return false;
    };
    let name = &inner[..colon_index];
    if name.is_empty() {
        return false;
    }
    if name.contains('[') || name.contains(']') {
        return false;
    }

    let suffix = &inner[colon_index + 1..];
    if suffix.is_empty() {
        return false;
    }
    if matches!(suffix.chars().next(), Some('-' | '=' | '+' | '?')) {
        return false;
    }

    true
}

fn plain_case_modification_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let relative_start = text.find("${")?;
    let start = span.start.advanced_by(&text[..relative_start]);
    let after_start = &source[start.offset..];
    let relative_end = after_start.find('}')?;
    let end = start.advanced_by(&after_start[..relative_end + '}'.len_utf8()]);
    let candidate = &after_start[..relative_end + '}'.len_utf8()];

    is_plain_case_modification_text(candidate).then_some(Span::from_positions(start, end))
}

fn is_plain_case_modification_text(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };
    if inner.starts_with('#') || inner.starts_with('!') {
        return false;
    }

    let mut index = 0;
    let chars = inner.chars().collect::<Vec<_>>();
    while index < chars.len()
        && (chars[index].is_ascii_alphanumeric()
            || chars[index] == '_'
            || matches!(chars[index], '@' | '*'))
    {
        index += 1;
    }

    if index == 0 {
        return false;
    }

    if chars.get(index) == Some(&'[') {
        let mut close = index + 1;
        while close < chars.len() && chars[close] != ']' {
            close += 1;
        }
        if close == chars.len() {
            return false;
        }
        index = close + 1;
    }

    let Some(&operator) = chars.get(index) else {
        return false;
    };
    if !matches!(operator, '^' | ',') {
        return false;
    }

    true
}

fn plain_replacement_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let (relative_start, relative_end) = next_parameter_expansion_candidate(text, 0)?;
    let start = span.start.advanced_by(&text[..relative_start]);
    let end = span.start.advanced_by(&text[..relative_end]);
    let candidate = &text[relative_start..relative_end];

    is_plain_replacement_expansion_text(candidate).then_some(Span::from_positions(start, end))
}

fn is_plain_replacement_expansion_text(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };
    if inner.starts_with('#') || inner.starts_with('!') {
        return false;
    }

    let mut index = 0;
    let chars = inner.chars().collect::<Vec<_>>();
    while index < chars.len()
        && (chars[index].is_ascii_alphanumeric()
            || chars[index] == '_'
            || matches!(chars[index], '@' | '*'))
    {
        index += 1;
    }

    if index == 0 {
        return false;
    }

    if chars.get(index) == Some(&'[') {
        let mut close = index + 1;
        while close < chars.len() && chars[close] != ']' {
            close += 1;
        }
        if close == chars.len() {
            return false;
        }
        index = close + 1;
    }

    if chars.get(index) != Some(&'/') {
        return false;
    }

    index += 1;
    if chars.get(index) == Some(&'/') {
        index += 1;
    }

    index < chars.len()
}

fn next_parameter_expansion_candidate(text: &str, search_start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = search_start;

    while index + 1 < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 2;
            }
            b'$' if bytes[index + 1] == b'{' => {
                let start = index;
                index += 2;
                let mut depth = 1;

                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => {
                            index += 2;
                        }
                        b'$' if index + 1 < bytes.len() && bytes[index + 1] == b'{' => {
                            depth += 1;
                            index += 2;
                        }
                        b'}' => {
                            depth -= 1;
                            index += 1;
                            if depth == 0 {
                                return Some((start, index));
                            }
                        }
                        _ => {
                            index += 1;
                        }
                    }
                }

                return None;
            }
            _ => {
                index += 1;
            }
        }
    }

    None
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

fn collect_positional_parameter_operator_spans_in_arithmetic(
    expansion_span: Span,
    expression: &SourceText,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = expression.slice(source);
    let mut should_report = false;
    let mut state = ArithmeticScanState::default();
    let mut chars = text.char_indices();

    while let Some((index, char)) = chars.next() {
        match state {
            ArithmeticScanState::Normal => match char {
                '\'' => state = ArithmeticScanState::SingleQuoted,
                '"' => state = ArithmeticScanState::DoubleQuoted,
                '\\' => {
                    chars.next();
                }
                '$' => {
                    let Some(token_end) = positional_parameter_token_end(text, index) else {
                        continue;
                    };

                    let prev = text[..index].chars().rev().find(|ch| !ch.is_whitespace());
                    let next = text[token_end..].chars().find(|ch| !ch.is_whitespace());

                    if prev.is_some_and(is_left_operand_neighbor)
                        || next.is_some_and(is_right_operand_neighbor)
                    {
                        should_report = true;
                        break;
                    }
                }
                _ => {}
            },
            ArithmeticScanState::SingleQuoted => {
                if char == '\'' {
                    state = ArithmeticScanState::Normal;
                }
            }
            ArithmeticScanState::DoubleQuoted => match char {
                '"' => state = ArithmeticScanState::Normal,
                '\\' => {
                    chars.next();
                }
                _ => {}
            },
        }
    }

    if should_report {
        spans.push(Span::from_positions(
            expansion_span.start,
            expansion_span.start,
        ));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ArithmeticScanState {
    #[default]
    Normal,
    SingleQuoted,
    DoubleQuoted,
}

fn positional_parameter_token_end(text: &str, start: usize) -> Option<usize> {
    let rest = text.get(start..)?;
    if !rest.starts_with('$') {
        return None;
    }

    let bytes = rest.as_bytes();
    if bytes.get(1).is_some_and(u8::is_ascii_digit) {
        let mut idx = 2usize;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
        }
        return Some(start + idx);
    }

    if bytes.get(1) == Some(&b'{') {
        let mut idx = 2usize;
        let mut saw_digit = false;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            saw_digit = true;
            idx += 1;
        }
        if saw_digit && bytes.get(idx) == Some(&b'}') {
            return Some(start + idx + 1);
        }
    }

    None
}

fn is_left_operand_neighbor(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | ')' | ']' | '}' | '"' | '\'')
}

fn is_right_operand_neighbor(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '(' | '[' | '{' | '"' | '\'')
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

fn is_nested_parameter_expansion(parameter: &shuck_ast::ParameterExpansion, source: &str) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Zsh(syntax) => {
            matches!(syntax.target, ZshExpansionTarget::Nested(_))
        }
        ParameterExpansionSyntax::Bourne(_) => {
            let body = parameter.raw_body.slice(source).trim_start();
            contains_nested_parameter_marker(body)
        }
    }
}

fn contains_nested_parameter_marker(text: &str) -> bool {
    text.starts_with("${${") || text.starts_with("${#${") || text.starts_with("${!${")
}
fn simple_command_variable_set_operand<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
    let operands = simple_test_operands(command, source)?;
    (operands.len() == 2 && static_word_text(&operands[0], source).as_deref() == Some("-v"))
        .then(|| &operands[1])
}

fn collect_unicode_smart_quote_spans_in_word_parts(
    parts: &[WordPartNode],
    source: &str,
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) if !quoted => {
                let literal = text.as_str(source, part.span);
                for (offset, char) in literal.char_indices() {
                    if !is_unicode_smart_quote(char) {
                        continue;
                    }
                    let start = part.span.start.advanced_by(&literal[..offset]);
                    let end = start.advanced_by(char.encode_utf8(&mut [0; 4]));
                    spans.push(Span::from_positions(start, end));
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unicode_smart_quote_spans_in_word_parts(parts, source, true, spans)
            }
            _ => {}
        }
    }
}

fn is_unicode_smart_quote(char: char) -> bool {
    matches!(char, '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}')
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
