use shuck_ast::{
    ArithmeticForCommand, Assignment, AssignmentValue, BourneParameterExpansion, BuiltinCommand,
    Command, CommandSubstitutionSyntax, CompoundCommand, ConditionalExpr, DeclClause, DeclOperand,
    File, FunctionDef, Heredoc, HeredocBody, HeredocBodyPart, HeredocBodyPartNode,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart,
    PatternPartNode, Redirect, RedirectKind, Stmt, StmtSeq, Subscript, TextRange, TextSize, VarRef,
    Word, WordPart, WordPartNode, ZshExpansionOperation, ZshExpansionTarget, ZshGlobSegment,
    ZshParameterExpansion,
};
/// A syntactic region that affects source interpretation.
///
/// Region kinds describe contexts where downstream tools should interpret bytes
/// differently from ordinary shell source. For example, formatters and lint
/// rules often need to suppress word-splitting assumptions inside quotes or
/// avoid scanning heredoc bodies as command syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Text inside single quotes.
    SingleQuoted,
    /// Text inside double quotes.
    DoubleQuoted,
    /// A heredoc body.
    Heredoc,
    /// A command substitution such as `$(...)` or backticks.
    CommandSubstitution,
    /// An arithmetic context such as `$((...))` or `((...))`.
    Arithmetic,
    /// A conditional context such as `[[ ... ]]`.
    Conditional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexedRegion {
    kind: RegionKind,
    range: TextRange,
}

/// Indexed metadata for one heredoc body and its source closer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedHeredoc {
    /// Byte range of the parsed heredoc body.
    pub body_range: TextRange,
    /// Byte range of the original closing marker line, excluding the line ending.
    pub closing_marker_range: Option<TextRange>,
}

/// Byte ranges of syntactic regions where special rules apply.
///
/// `RegionIndex` stores one sorted range list per commonly queried region type
/// plus a combined list for queries that need the innermost containing region.
/// Ranges are byte ranges in the original source and are half-open: the start
/// byte is included and the end byte is excluded.
///
/// Region ranges generally include the syntactic delimiters represented by the
/// parser span, such as quote characters or substitution delimiters. Callers
/// that need only the interior text should trim delimiters using source-aware
/// logic at the call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionIndex {
    // Disjoint union for boolean quote queries; original ranges retain nesting.
    quoted_coverage: Vec<TextRange>,
    single_quoted: Vec<TextRange>,
    double_quoted: Vec<TextRange>,
    heredocs: Vec<TextRange>,
    command_substitutions: Vec<TextRange>,
    backtick_command_substitutions: Vec<TextRange>,
    arithmetic: Vec<TextRange>,
    conditionals: Vec<TextRange>,
    quoted_heredocs: Vec<TextRange>,
    indexed_heredocs: Vec<IndexedHeredoc>,
    regions: Vec<IndexedRegion>,
    expansion_brace_edges: Vec<TextSize>,
    dollar_brace_pairs: Vec<TextRange>,
    structural_folding_ranges: Vec<TextRange>,
    selection_ranges: Vec<TextRange>,
}

impl RegionIndex {
    /// Build a region index from source text and a parsed file.
    ///
    /// `source` is accepted for API symmetry with the other indexes and future
    /// source-sensitive region collection, but current region construction is
    /// driven by AST spans from `file`. `source` and `file` should describe the
    /// same parsed text.
    pub fn new(source: &str, file: &File) -> Self {
        Self::new_with_options(source, file, false, false, false)
    }

    /// Build a region index that also retains source-layout metadata.
    ///
    /// Most callers should use [`Self::new`]. Formatter-style consumers that
    /// need heredoc closing marker ranges can use this constructor directly, or
    /// build an [`crate::Indexer`] with
    /// [`crate::IndexerOptions::with_source_layout_indexes`].
    pub fn with_source_layout_indexes(source: &str, file: &File) -> Self {
        Self::new_with_options(source, file, true, false, false)
    }

    pub(crate) fn new_with_options(
        source: &str,
        file: &File,
        source_layout_indexes: bool,
        folding_ranges: bool,
        selection_ranges: bool,
    ) -> Self {
        let mut collector = RegionCollector::new(
            source,
            source_layout_indexes,
            folding_ranges,
            selection_ranges,
        );
        collector.visit_file(file);
        collector.finish()
    }

    /// Return the innermost region kind containing `offset`, if any.
    ///
    /// If multiple regions contain the same byte, the smallest containing range
    /// wins. Ties prefer the region that starts later, matching the same
    /// innermost rule used by [`Self::region_with_range_at`].
    pub fn region_at(&self, offset: TextSize) -> Option<RegionKind> {
        self.region_with_range_at(offset).map(|(kind, _)| kind)
    }

    /// Return the innermost region kind and range containing `offset`, if any.
    ///
    /// The returned range is the original half-open byte range stored for that
    /// region. This is useful when a caller needs both the classification and
    /// the bounds of the syntactic context that produced it.
    pub fn region_with_range_at(&self, offset: TextSize) -> Option<(RegionKind, TextRange)> {
        let mut best: Option<IndexedRegion> = None;
        let end = self
            .regions
            .partition_point(|region| region.range.start() <= offset);

        for region in self.regions[..end].iter().copied() {
            if !contains(region.range, offset) {
                continue;
            }

            best = match best {
                None => Some(region),
                Some(current) if is_innermost(region.range, current.range) => Some(region),
                Some(current) => Some(current),
            };
        }

        best.map(|region| (region.kind, region.range))
    }

    /// Return whether `offset` falls inside any quoted region.
    ///
    /// This includes single quotes, double quotes, and bodies of heredocs whose
    /// delimiter was quoted.
    pub fn is_quoted(&self, offset: TextSize) -> bool {
        let index = self
            .quoted_coverage
            .partition_point(|range| range.start() <= offset);
        index > 0 && contains(self.quoted_coverage[index - 1], offset)
    }

    /// Return whether `offset` falls inside any heredoc body.
    ///
    /// Both quoted and unquoted heredoc bodies count as heredoc regions.
    pub fn is_heredoc(&self, offset: TextSize) -> bool {
        contains_any(&self.heredocs, offset)
    }

    /// Return whether `offset` falls inside a command substitution.
    ///
    /// This covers command substitutions represented by parser spans, including
    /// nested substitutions inside words and heredoc bodies.
    pub fn is_command_substitution(&self, offset: TextSize) -> bool {
        contains_any(&self.command_substitutions, offset)
    }

    /// Return all backtick command-substitution ranges in source order.
    ///
    /// The ranges are half-open byte ranges for legacy `` `...` `` command
    /// substitutions only. `$(...)` ranges are excluded.
    pub fn backtick_command_substitution_ranges(&self) -> &[TextRange] {
        &self.backtick_command_substitutions
    }

    /// Return whether `offset` falls inside an arithmetic context.
    ///
    /// Arithmetic contexts include arithmetic commands, arithmetic `for`
    /// headers, and arithmetic expansions represented by parser spans.
    pub fn is_arithmetic(&self, offset: TextSize) -> bool {
        contains_any(&self.arithmetic, offset)
    }

    /// Return all heredoc body ranges in source order.
    ///
    /// The ranges are half-open byte ranges and include both quoted and unquoted
    /// heredoc bodies. The returned slice borrows from the index and does not
    /// allocate.
    pub fn heredoc_ranges(&self) -> &[TextRange] {
        &self.heredocs
    }

    /// Return indexed heredoc metadata in source order.
    ///
    /// This formatter-oriented metadata is only retained when source-layout
    /// indexes are enabled.
    pub fn heredocs(&self) -> &[IndexedHeredoc] {
        &self.indexed_heredocs
    }

    /// Return the original closing marker range for a heredoc body range.
    ///
    /// Returns `None` unless source-layout indexes were enabled during
    /// construction.
    pub fn heredoc_closing_marker_range(&self, body_range: TextRange) -> Option<TextRange> {
        let index = self.indexed_heredocs.partition_point(|heredoc| {
            (heredoc.body_range.start(), heredoc.body_range.end())
                < (body_range.start(), body_range.end())
        });
        self.indexed_heredocs
            .get(index)
            .filter(|heredoc| heredoc.body_range == body_range)
            .and_then(|heredoc| heredoc.closing_marker_range)
    }

    /// Return whether `offset` is the byte position of an `{` or `}` that the
    /// parser classified as part of an expansion construct.
    ///
    /// Expansion brace edges include both delimiters of `${...}`-shaped
    /// parameter expansions and the delimiters of unquoted active brace
    /// expansions such as `{a,b}` or `{1..5}`. This lets downstream tools skip
    /// those braces when reporting on stray literal `{` or `}` characters.
    pub fn is_expansion_brace_edge(&self, offset: TextSize) -> bool {
        self.expansion_brace_edges.binary_search(&offset).is_ok()
    }

    /// Return the first indexed `${...}` parameter expansion whose `$` byte
    /// position falls within `range`, in source order.
    ///
    /// The returned range is the full `${...}` syntactic span, so its end is
    /// the byte position one past the matching `}`. Active brace expansions
    /// such as `{a,b}` are not included; only `${...}`-shaped expansions are.
    pub fn first_dollar_brace_pair_in(&self, range: TextRange) -> Option<TextRange> {
        let start = range.start();
        let idx = self
            .dollar_brace_pairs
            .partition_point(|pair| pair.start() < start);
        self.dollar_brace_pairs
            .get(idx)
            .copied()
            .filter(|pair| pair.start() < range.end())
    }
    pub(crate) fn structural_folding_ranges(&self) -> &[TextRange] {
        &self.structural_folding_ranges
    }

    pub(crate) fn selection_ranges(&self) -> &[TextRange] {
        &self.selection_ranges
    }
}

struct RegionCollector<'a> {
    _source: &'a str,
    source_layout_indexes: bool,
    collect_folding_ranges: bool,
    collect_selection_ranges: bool,
    single_quoted: Vec<TextRange>,
    double_quoted: Vec<TextRange>,
    heredocs: Vec<TextRange>,
    command_substitutions: Vec<TextRange>,
    backtick_command_substitutions: Vec<TextRange>,
    arithmetic: Vec<TextRange>,
    conditionals: Vec<TextRange>,
    quoted_heredocs: Vec<TextRange>,
    indexed_heredocs: Vec<IndexedHeredoc>,
    expansion_brace_edges: Vec<TextSize>,
    dollar_brace_pairs: Vec<TextRange>,
    structural_folding_ranges: Vec<TextRange>,
    selection_ranges: Vec<TextRange>,
}

impl<'a> RegionCollector<'a> {
    fn new(
        source: &'a str,
        source_layout_indexes: bool,
        collect_folding_ranges: bool,
        collect_selection_ranges: bool,
    ) -> Self {
        Self {
            _source: source,
            source_layout_indexes,
            collect_folding_ranges,
            collect_selection_ranges,
            single_quoted: Vec::new(),
            double_quoted: Vec::new(),
            heredocs: Vec::new(),
            command_substitutions: Vec::new(),
            backtick_command_substitutions: Vec::new(),
            arithmetic: Vec::new(),
            conditionals: Vec::new(),
            quoted_heredocs: Vec::new(),
            indexed_heredocs: Vec::new(),
            expansion_brace_edges: Vec::new(),
            dollar_brace_pairs: Vec::new(),
            structural_folding_ranges: Vec::new(),
            selection_ranges: Vec::new(),
        }
    }

    fn finish(mut self) -> RegionIndex {
        sort_ranges(&mut self.single_quoted);
        sort_ranges(&mut self.double_quoted);
        sort_ranges(&mut self.heredocs);
        sort_ranges(&mut self.command_substitutions);
        sort_ranges(&mut self.backtick_command_substitutions);
        sort_ranges(&mut self.arithmetic);
        sort_ranges(&mut self.conditionals);
        sort_ranges(&mut self.quoted_heredocs);
        self.indexed_heredocs.sort_unstable_by_key(|heredoc| {
            (
                heredoc.body_range.start().to_u32(),
                heredoc.body_range.end().to_u32(),
            )
        });
        self.expansion_brace_edges.sort_unstable();
        self.expansion_brace_edges.dedup();
        self.dollar_brace_pairs
            .sort_unstable_by_key(|range| (range.start(), range.end()));
        self.dollar_brace_pairs
            .dedup_by_key(|range| (range.start(), range.end()));
        sort_ranges(&mut self.structural_folding_ranges);
        self.structural_folding_ranges.dedup();
        sort_ranges(&mut self.selection_ranges);
        self.selection_ranges.dedup();

        let mut regions = Vec::with_capacity(
            self.single_quoted.len()
                + self.double_quoted.len()
                + self.heredocs.len()
                + self.command_substitutions.len()
                + self.arithmetic.len()
                + self.conditionals.len(),
        );
        regions.extend(
            self.single_quoted
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::SingleQuoted,
                    range,
                }),
        );
        regions.extend(
            self.double_quoted
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::DoubleQuoted,
                    range,
                }),
        );
        regions.extend(self.heredocs.iter().copied().map(|range| IndexedRegion {
            kind: RegionKind::Heredoc,
            range,
        }));
        regions.extend(
            self.command_substitutions
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::CommandSubstitution,
                    range,
                }),
        );
        regions.extend(self.arithmetic.iter().copied().map(|range| IndexedRegion {
            kind: RegionKind::Arithmetic,
            range,
        }));
        regions.extend(
            self.conditionals
                .iter()
                .copied()
                .map(|range| IndexedRegion {
                    kind: RegionKind::Conditional,
                    range,
                }),
        );
        regions.sort_unstable_by_key(|region| {
            (region.range.start().to_u32(), region.range.end().to_u32())
        });

        let mut quoted_coverage = self
            .single_quoted
            .iter()
            .chain(&self.double_quoted)
            .chain(&self.quoted_heredocs)
            .copied()
            .collect::<Vec<_>>();
        sort_ranges(&mut quoted_coverage);
        let mut merged = 0usize;
        for index in 0..quoted_coverage.len() {
            let range = quoted_coverage[index];
            if merged > 0 && range.start() <= quoted_coverage[merged - 1].end() {
                let previous = &mut quoted_coverage[merged - 1];
                *previous = TextRange::new(previous.start(), previous.end().max(range.end()));
            } else {
                quoted_coverage[merged] = range;
                merged += 1;
            }
        }
        quoted_coverage.truncate(merged);

        RegionIndex {
            quoted_coverage,
            single_quoted: self.single_quoted,
            double_quoted: self.double_quoted,
            heredocs: self.heredocs,
            command_substitutions: self.command_substitutions,
            backtick_command_substitutions: self.backtick_command_substitutions,
            arithmetic: self.arithmetic,
            conditionals: self.conditionals,
            quoted_heredocs: self.quoted_heredocs,
            indexed_heredocs: self.indexed_heredocs,
            regions,
            expansion_brace_edges: self.expansion_brace_edges,
            dollar_brace_pairs: self.dollar_brace_pairs,
            structural_folding_ranges: self.structural_folding_ranges,
            selection_ranges: self.selection_ranges,
        }
    }

    fn visit_file(&mut self, file: &File) {
        self.push_selection_range(file.span.to_range());
        self.visit_stmt_seq(&file.body);
    }

    fn visit_stmt_seq(&mut self, commands: &StmtSeq) {
        self.push_selection_range(commands.span.to_range());
        for stmt in commands.iter() {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        self.visit_stmt_with_folding(stmt, true);
    }

    fn visit_stmt_with_folding(&mut self, stmt: &Stmt, record_folding_range: bool) {
        self.push_selection_range(stmt.span.to_range());
        for redirect in &stmt.redirects {
            self.visit_redirect(redirect);
        }
        self.visit_command(&stmt.command, stmt.span.to_range(), record_folding_range);
    }

    fn visit_command(
        &mut self,
        command: &Command,
        statement_range: TextRange,
        record_folding_range: bool,
    ) {
        self.push_selection_range(command_range(command));
        match command {
            Command::Simple(command) => {
                self.visit_word(&command.name);
                for argument in &command.args {
                    self.visit_word(argument);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            Command::Builtin(command) => self.visit_builtin(command),
            Command::Decl(command) => self.visit_decl(command),
            Command::Binary(command) => {
                self.visit_stmt(&command.left);
                self.visit_stmt(&command.right);
            }
            Command::Compound(command) => {
                if self.collect_folding_ranges
                    && record_folding_range
                    && let Some(end) = compound_folding_end(command, self._source)
                {
                    push_bounded_range(
                        &mut self.structural_folding_ranges,
                        compound_folding_start(command, statement_range.start(), self._source),
                        end,
                    );
                }
                self.visit_compound(command);
            }
            Command::Function(function @ FunctionDef { header, body, .. }) => {
                if self.collect_folding_ranges
                    && let Some(end) = statement_folding_end(body, self._source)
                {
                    push_bounded_range(
                        &mut self.structural_folding_ranges,
                        function.span.to_range().start(),
                        end,
                    );
                }
                for entry in &header.entries {
                    self.visit_word(&entry.word);
                }
                self.visit_stmt_with_folding(body, false);
            }
            Command::AnonymousFunction(function) => {
                if self.collect_folding_ranges
                    && let Some(end) = statement_folding_end(&function.body, self._source)
                {
                    push_bounded_range(
                        &mut self.structural_folding_ranges,
                        function.span.to_range().start(),
                        end,
                    );
                }
                self.visit_stmt_with_folding(&function.body, false);
                for argument in &function.args {
                    self.visit_word(argument);
                }
            }
        }
    }

    fn visit_builtin(&mut self, command: &BuiltinCommand) {
        match command {
            BuiltinCommand::Break(command) => {
                if let Some(depth) = &command.depth {
                    self.visit_word(depth);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(depth) = &command.depth {
                    self.visit_word(depth);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(code) = &command.code {
                    self.visit_word(code);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(code) = &command.code {
                    self.visit_word(code);
                }
                for argument in &command.extra_args {
                    self.visit_word(argument);
                }
                for assignment in &command.assignments {
                    self.visit_assignment(assignment);
                }
            }
        }
    }

    fn visit_decl(&mut self, command: &DeclClause) {
        for operand in &command.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.visit_word(word),
                DeclOperand::Name(reference) => self.visit_var_ref_subscript(reference),
                DeclOperand::Assignment(assignment) => self.visit_assignment(assignment),
            }
        }
        for assignment in &command.assignments {
            self.visit_assignment(assignment);
        }
    }

    fn visit_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
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
                if let Some(words) = &command.words {
                    for word in words {
                        self.visit_word(word);
                    }
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Repeat(command) => {
                self.visit_word(&command.count);
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Foreach(command) => {
                for word in &command.words {
                    self.visit_word(word);
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::ArithmeticFor(command) => {
                self.push_arithmetic_range(command);
                for expression in [
                    command.init_ast.as_ref(),
                    command.condition_ast.as_ref(),
                    command.step_ast.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    self.visit_arithmetic_shell_words(expression);
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::While(command) => {
                self.visit_stmt_seq(&command.condition);
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Until(command) => {
                self.visit_stmt_seq(&command.condition);
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Case(command) => {
                self.visit_word(&command.word);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        self.visit_pattern(pattern);
                    }
                    self.visit_stmt_seq(&item.body);
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    self.visit_word(word);
                }
                self.visit_stmt_seq(&command.body);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.visit_stmt_seq(commands);
            }
            CompoundCommand::Always(command) => {
                self.visit_stmt_seq(&command.body);
                self.visit_stmt_seq(&command.always_body);
            }
            CompoundCommand::Arithmetic(command) => {
                push_range(&mut self.arithmetic, command.span.to_range());
                if let Some(expression) = command.expr_ast.as_ref() {
                    self.visit_arithmetic_shell_words(expression);
                }
            }
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.visit_stmt(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                push_range(&mut self.conditionals, command.span.to_range());
                self.visit_conditional_expr(&command.expression);
            }
            CompoundCommand::Coproc(command) => self.visit_stmt(&command.body),
        }
    }

    fn push_arithmetic_range(&mut self, command: &ArithmeticForCommand) {
        let range = command
            .left_paren_span
            .merge(command.right_paren_span)
            .to_range();
        push_range(&mut self.arithmetic, range);
    }

    fn push_dollar_brace_pair(&mut self, range: TextRange) {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        if !self
            ._source
            .get(start..end)
            .is_some_and(|text| text.starts_with("${") && text.ends_with('}'))
        {
            return;
        }
        if find_parameter_expansion_end(self._source, start) != Some(end) {
            return;
        }

        let dollar_len = TextSize::new('$'.len_utf8() as u32);
        let close_len = TextSize::new('}'.len_utf8() as u32);
        if range.len() < dollar_len + close_len {
            return;
        }
        let open = range.start() + dollar_len;
        let close = range.end() - close_len;
        if open >= close {
            return;
        }
        self.expansion_brace_edges.push(open);
        self.expansion_brace_edges.push(close);
        self.dollar_brace_pairs.push(range);
    }

    fn push_dollar_brace_pairs_in_source_range(&mut self, range: TextRange) {
        let source_start = usize::from(range.start());
        let source_end = usize::from(range.end());
        let Some(text) = self._source.get(source_start..source_end) else {
            return;
        };

        let mut index = 0usize;
        while index + 1 < text.len() {
            let Some(relative_dollar) = text[index..].find("${") else {
                break;
            };
            let dollar_offset = index + relative_dollar;
            if has_odd_backslash_run_before(text, dollar_offset) {
                index = dollar_offset + '$'.len_utf8();
                continue;
            }

            let Some(end_offset) = find_parameter_expansion_end(text, dollar_offset) else {
                index = dollar_offset + "${".len();
                continue;
            };
            let open = source_start + dollar_offset + '$'.len_utf8();
            let close = source_start + end_offset - '}'.len_utf8();
            let pair = TextRange::new(
                TextSize::new((source_start + dollar_offset) as u32),
                TextSize::new((source_start + end_offset) as u32),
            );
            self.expansion_brace_edges.push(TextSize::new(open as u32));
            self.expansion_brace_edges.push(TextSize::new(close as u32));
            self.dollar_brace_pairs.push(pair);
            index = end_offset;
        }
    }

    fn push_expansion_brace_pair(&mut self, range: TextRange) {
        let close_len = TextSize::new('}'.len_utf8() as u32);
        if range.len() < close_len {
            return;
        }
        let open = range.start();
        let close = range.end() - close_len;
        if open >= close {
            return;
        }
        self.expansion_brace_edges.push(open);
        self.expansion_brace_edges.push(close);
    }

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr) {
        self.push_selection_range(expression.span().to_range());
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
            ConditionalExpr::VarRef(reference) => self.visit_var_ref_subscript(reference),
        }
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        self.push_selection_range(redirect.span.to_range());
        match redirect.kind {
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => {
                let Some(heredoc) = redirect.heredoc() else {
                    unreachable!("expected heredoc redirect");
                };
                let range = heredoc.body.span.to_range();
                push_range(&mut self.heredocs, range);
                if self.collect_folding_ranges {
                    push_bounded_range(
                        &mut self.structural_folding_ranges,
                        redirect.span.to_range().start(),
                        range.end(),
                    );
                }
                if self.source_layout_indexes {
                    self.indexed_heredocs.push(IndexedHeredoc {
                        body_range: range,
                        closing_marker_range: heredoc_closing_marker_range(heredoc, self._source),
                    });
                }
                if heredoc.delimiter.quoted {
                    push_range(&mut self.quoted_heredocs, range);
                }
                self.visit_heredoc_body(&heredoc.body);
            }
            _ => {
                let Some(word) = redirect.word_target() else {
                    unreachable!("expected non-heredoc redirect target");
                };
                self.visit_word(word);
            }
        }
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        self.push_selection_range(assignment.span.to_range());
        self.visit_var_ref_subscript(&assignment.target);
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.visit_word(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        shuck_ast::ArrayElem::Sequential(word) => self.visit_word(word),
                        shuck_ast::ArrayElem::Keyed { key, value }
                        | shuck_ast::ArrayElem::KeyedAppend { key, value } => {
                            self.visit_subscript(Some(key));
                            self.visit_word(value);
                        }
                    }
                }
            }
        }
    }

    fn visit_word(&mut self, word: &Word) {
        self.push_selection_range(word.span.to_range());
        for brace in word.brace_syntax.iter().copied() {
            self.push_selection_range(brace.span.to_range());
            if brace.expands() {
                self.push_expansion_brace_pair(brace.span.to_range());
            }
        }
        self.visit_word_parts(&word.parts);
    }

    fn visit_parameter_expansion(&mut self, parameter: &ParameterExpansion) {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => {
                self.visit_bourne_parameter_expansion(syntax);
            }
            ParameterExpansionSyntax::Zsh(syntax) => {
                self.visit_zsh_parameter_expansion(syntax);
            }
        }
    }

    fn visit_bourne_parameter_expansion(&mut self, syntax: &BourneParameterExpansion) {
        match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                self.visit_var_ref_subscript(reference);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.visit_var_ref_subscript(reference);
                if let Some(operator) = operator.as_deref() {
                    self.visit_parameter_operator(operator);
                }
                if let Some(operand_word) = operand_word_ast.as_deref() {
                    self.visit_word(operand_word);
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                self.visit_var_ref_subscript(reference);
                self.visit_arithmetic_operand_word(offset_ast.as_deref(), offset_word_ast);
                if let Some(length_ast) = length_ast.as_deref() {
                    self.visit_arithmetic_shell_words(length_ast);
                } else if let Some(length_word) = length_word_ast.as_deref() {
                    self.visit_word(length_word);
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.visit_var_ref_subscript(reference);
                self.visit_parameter_operator(operator);
                if let Some(operand_word) = operand_word_ast.as_deref() {
                    self.visit_word(operand_word);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        }
    }

    fn visit_parameter_operator(&mut self, operator: &ParameterOp) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                self.visit_pattern(pattern);
            }
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
                self.visit_pattern(pattern);
                self.visit_word(replacement_word_ast);
            }
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

    fn visit_zsh_parameter_expansion(&mut self, syntax: &ZshParameterExpansion) {
        match &syntax.target {
            ZshExpansionTarget::Reference(reference) => self.visit_var_ref_subscript(reference),
            ZshExpansionTarget::Nested(parameter) => self.visit_parameter_expansion(parameter),
            ZshExpansionTarget::Word(word) => self.visit_word(word),
            ZshExpansionTarget::Empty => {}
        }

        for modifier in &syntax.modifiers {
            if let Some(word) = modifier.argument_word_ast() {
                self.visit_word(word);
            }
        }

        if let Some(operation) = syntax.operation.as_ref() {
            self.visit_zsh_expansion_operation(operation);
        }
    }

    fn visit_zsh_expansion_operation(&mut self, operation: &ZshExpansionOperation) {
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

    fn visit_arithmetic_operand_word(
        &mut self,
        expression_ast: Option<&shuck_ast::ArithmeticExprNode>,
        word: &Word,
    ) {
        if let Some(expression_ast) = expression_ast {
            self.visit_arithmetic_shell_words(expression_ast);
        } else {
            self.visit_word(word);
        }
    }

    fn visit_heredoc_body(&mut self, body: &HeredocBody) {
        self.push_selection_range(body.span.to_range());
        self.visit_heredoc_body_parts(&body.parts);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        self.push_selection_range(pattern.span.to_range());
        self.visit_pattern_parts(&pattern.parts);
    }

    fn visit_pattern_parts(&mut self, parts: &[PatternPartNode]) {
        for part in parts {
            self.push_selection_range(part.span.to_range());
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

    fn visit_word_parts(&mut self, parts: &[WordPartNode]) {
        for part in parts {
            let range = part.span.to_range();
            self.push_selection_range(range);
            match &part.kind {
                WordPart::ZshQualifiedGlob(glob) => {
                    for segment in &glob.segments {
                        if let ZshGlobSegment::Pattern(pattern) = segment {
                            self.visit_pattern(pattern);
                        }
                    }
                }
                WordPart::SingleQuoted { .. } => {
                    push_range(&mut self.single_quoted, range);
                }
                WordPart::DoubleQuoted { parts, .. } => {
                    push_range(&mut self.double_quoted, range);
                    self.push_dollar_brace_pairs_in_source_range(range);
                    self.visit_word_parts(parts);
                }
                WordPart::CommandSubstitution { body, syntax } => {
                    push_range(&mut self.command_substitutions, range);
                    if *syntax == CommandSubstitutionSyntax::Backtick {
                        push_range(&mut self.backtick_command_substitutions, range);
                    }
                    self.visit_stmt_seq(body);
                }
                WordPart::ArithmeticExpansion { expression_ast, .. } => {
                    push_range(&mut self.arithmetic, range);
                    self.push_dollar_brace_pairs_in_source_range(range);
                    if let Some(expression_ast) = expression_ast.as_deref() {
                        self.visit_arithmetic_shell_words(expression_ast);
                    }
                }
                WordPart::ProcessSubstitution { body, .. } => self.visit_stmt_seq(body),
                WordPart::Parameter(parameter) => {
                    self.push_dollar_brace_pair(range);
                    self.visit_parameter_expansion(parameter);
                }
                WordPart::ParameterExpansion {
                    reference,
                    operator,
                    operand_word_ast,
                    ..
                } => {
                    self.push_dollar_brace_pair(range);
                    self.visit_var_ref_subscript(reference);
                    self.visit_parameter_operator(operator);
                    if let Some(operand_word) = operand_word_ast.as_deref() {
                        self.visit_word(operand_word);
                    }
                }
                WordPart::Length(reference)
                | WordPart::ArrayAccess(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::ArrayIndices(reference)
                | WordPart::Transformation { reference, .. } => {
                    self.push_dollar_brace_pair(range);
                    self.visit_var_ref_subscript(reference);
                }
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
                    self.push_dollar_brace_pair(range);
                    self.visit_var_ref_subscript(reference);
                    self.visit_arithmetic_operand_word(offset_ast.as_deref(), offset_word_ast);
                    if let Some(length_ast) = length_ast.as_deref() {
                        self.visit_arithmetic_shell_words(length_ast);
                    } else if let Some(length_word) = length_word_ast.as_deref() {
                        self.visit_word(length_word);
                    }
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator,
                    operand_word_ast,
                    ..
                } => {
                    self.push_dollar_brace_pair(range);
                    self.visit_var_ref_subscript(reference);
                    if let Some(operator) = operator.as_deref() {
                        self.visit_parameter_operator(operator);
                    }
                    if let Some(operand_word) = operand_word_ast.as_deref() {
                        self.visit_word(operand_word);
                    }
                }
                WordPart::PrefixMatch { .. } => {
                    self.push_dollar_brace_pair(range);
                }
                WordPart::Variable(_) => {
                    self.push_dollar_brace_pair(range);
                }
                WordPart::Literal(_) => {}
            }
        }
    }

    fn visit_heredoc_body_parts(&mut self, parts: &[HeredocBodyPartNode]) {
        for part in parts {
            let range = part.span.to_range();
            self.push_selection_range(range);
            match &part.kind {
                HeredocBodyPart::CommandSubstitution { body, syntax } => {
                    push_range(&mut self.command_substitutions, range);
                    if *syntax == CommandSubstitutionSyntax::Backtick {
                        push_range(&mut self.backtick_command_substitutions, range);
                    }
                    self.visit_stmt_seq(body);
                }
                HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                    push_range(&mut self.arithmetic, range);
                    self.push_dollar_brace_pairs_in_source_range(range);
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        self.visit_arithmetic_shell_words(expression_ast);
                    }
                }
                HeredocBodyPart::Parameter(parameter) => {
                    self.push_dollar_brace_pair(range);
                    self.visit_parameter_expansion(parameter);
                }
                HeredocBodyPart::Variable(_) => {
                    self.push_dollar_brace_pair(range);
                }
                HeredocBodyPart::Literal(_) => {}
            }
        }
    }

    fn visit_var_ref_subscript(&mut self, reference: &VarRef) {
        self.push_selection_range(reference.name_span.to_range());
        self.push_selection_range(reference.span.to_range());
        self.visit_subscript(reference.subscript.as_deref());
    }

    fn visit_subscript(&mut self, subscript: Option<&Subscript>) {
        let Some(subscript) = subscript else {
            return;
        };
        self.push_selection_range(subscript.span().to_range());
        if subscript.selector().is_some() {
            return;
        }
        if let Some(expression_ast) = subscript.arithmetic_ast.as_ref() {
            self.visit_arithmetic_shell_words(expression_ast);
            return;
        }

        if let Some(word) = subscript.word_ast() {
            self.visit_word(word);
            return;
        }

        debug_assert!(
            subscript.word_ast().is_some(),
            "ordinary subscripts should always carry a word AST"
        );
    }

    fn visit_arithmetic_shell_words(&mut self, expression: &shuck_ast::ArithmeticExprNode) {
        self.push_selection_range(expression.span.to_range());
        match &expression.kind {
            shuck_ast::ArithmeticExpr::Number(_) | shuck_ast::ArithmeticExpr::Variable(_) => {}
            shuck_ast::ArithmeticExpr::Indexed { index, .. } => {
                self.visit_arithmetic_shell_words(index)
            }
            shuck_ast::ArithmeticExpr::ShellWord(word) => self.visit_word(word),
            shuck_ast::ArithmeticExpr::Parenthesized { expression } => {
                self.visit_arithmetic_shell_words(expression)
            }
            shuck_ast::ArithmeticExpr::Unary { expr, .. }
            | shuck_ast::ArithmeticExpr::Postfix { expr, .. } => {
                self.visit_arithmetic_shell_words(expr)
            }
            shuck_ast::ArithmeticExpr::Binary { left, right, .. } => {
                self.visit_arithmetic_shell_words(left);
                self.visit_arithmetic_shell_words(right);
            }
            shuck_ast::ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_arithmetic_shell_words(condition);
                self.visit_arithmetic_shell_words(then_expr);
                self.visit_arithmetic_shell_words(else_expr);
            }
            shuck_ast::ArithmeticExpr::Assignment { target, value, .. } => {
                if let shuck_ast::ArithmeticLvalue::Indexed { index, .. } = target {
                    self.visit_arithmetic_shell_words(index);
                }
                self.visit_arithmetic_shell_words(value);
            }
        }
    }

    fn push_selection_range(&mut self, range: TextRange) {
        if self.collect_selection_ranges {
            push_range(&mut self.selection_ranges, range);
        }
    }
}

fn command_range(command: &Command) -> TextRange {
    match command {
        Command::Simple(command) => command.span.to_range(),
        Command::Builtin(command) => builtin_range(command),
        Command::Decl(command) => command.span.to_range(),
        Command::Binary(command) => command.span.to_range(),
        Command::Compound(command) => compound_range(command),
        Command::Function(command) => command.span.to_range(),
        Command::AnonymousFunction(command) => command.span.to_range(),
    }
}

fn builtin_range(command: &BuiltinCommand) -> TextRange {
    match command {
        BuiltinCommand::Break(command) => command.span.to_range(),
        BuiltinCommand::Continue(command) => command.span.to_range(),
        BuiltinCommand::Return(command) => command.span.to_range(),
        BuiltinCommand::Exit(command) => command.span.to_range(),
    }
}

fn compound_range(command: &CompoundCommand) -> TextRange {
    match command {
        CompoundCommand::If(command) => command.span.to_range(),
        CompoundCommand::For(command) => command.span.to_range(),
        CompoundCommand::Repeat(command) => command.span.to_range(),
        CompoundCommand::Foreach(command) => command.span.to_range(),
        CompoundCommand::ArithmeticFor(command) => command.span.to_range(),
        CompoundCommand::While(command) => command.span.to_range(),
        CompoundCommand::Until(command) => command.span.to_range(),
        CompoundCommand::Case(command) => command.span.to_range(),
        CompoundCommand::Select(command) => command.span.to_range(),
        CompoundCommand::Subshell(sequence) | CompoundCommand::BraceGroup(sequence) => {
            sequence.span.to_range()
        }
        CompoundCommand::Arithmetic(command) => command.span.to_range(),
        CompoundCommand::Time(command) => command.span.to_range(),
        CompoundCommand::Conditional(command) => command.span.to_range(),
        CompoundCommand::Coproc(command) => command.span.to_range(),
        CompoundCommand::Always(command) => command.span.to_range(),
    }
}

fn sort_ranges(ranges: &mut [TextRange]) {
    ranges.sort_unstable_by_key(|range| (range.start().to_u32(), range.end().to_u32()));
}

fn push_range(ranges: &mut Vec<TextRange>, range: TextRange) {
    if !range.is_empty() {
        ranges.push(range);
    }
}

fn push_bounded_range(ranges: &mut Vec<TextRange>, start: TextSize, end: TextSize) {
    if start < end {
        ranges.push(TextRange::new(start, end));
    }
}

fn statement_folding_end(statement: &Stmt, source: &str) -> Option<TextSize> {
    match &statement.command {
        Command::Compound(command) => compound_folding_end(command, source),
        Command::Function(function) => statement_folding_end(&function.body, source),
        Command::AnonymousFunction(function) => statement_folding_end(&function.body, source),
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) | Command::Binary(_) => {
            Some(statement.span.to_range().end())
        }
    }
}

fn compound_folding_start(command: &CompoundCommand, fallback: TextSize, source: &str) -> TextSize {
    match command {
        CompoundCommand::Subshell(sequence) => {
            delimiter_before(source, sequence.span.to_range().start(), '(').unwrap_or(fallback)
        }
        CompoundCommand::BraceGroup(sequence) => {
            delimiter_before(source, sequence.span.to_range().start(), '{').unwrap_or(fallback)
        }
        _ => fallback,
    }
}

fn compound_folding_end(command: &CompoundCommand, source: &str) -> Option<TextSize> {
    let close_start = |span: shuck_ast::Span| span.to_range().start();
    match command {
        CompoundCommand::If(command) => Some(match command.syntax {
            shuck_ast::IfSyntax::ThenFi { fi_span, .. } => close_start(fi_span),
            shuck_ast::IfSyntax::Brace {
                right_brace_span, ..
            } => close_start(right_brace_span),
        }),
        CompoundCommand::For(command) => Some(match command.syntax {
            shuck_ast::ForSyntax::InDoDone { done_span, .. }
            | shuck_ast::ForSyntax::ParenDoDone { done_span, .. } => close_start(done_span),
            shuck_ast::ForSyntax::InBrace {
                right_brace_span, ..
            }
            | shuck_ast::ForSyntax::ParenBrace {
                right_brace_span, ..
            } => close_start(right_brace_span),
            shuck_ast::ForSyntax::InDirect { .. } | shuck_ast::ForSyntax::ParenDirect { .. } => {
                command.span.to_range().end()
            }
        }),
        CompoundCommand::Repeat(command) => Some(match command.syntax {
            shuck_ast::RepeatSyntax::DoDone { done_span, .. } => close_start(done_span),
            shuck_ast::RepeatSyntax::Brace {
                right_brace_span, ..
            } => close_start(right_brace_span),
            shuck_ast::RepeatSyntax::Direct => command.span.to_range().end(),
        }),
        CompoundCommand::Foreach(command) => Some(match command.syntax {
            shuck_ast::ForeachSyntax::ParenBrace {
                right_brace_span, ..
            } => close_start(right_brace_span),
            shuck_ast::ForeachSyntax::InDoDone { done_span, .. } => close_start(done_span),
        }),
        CompoundCommand::ArithmeticFor(command) => command.done_span.map(close_start),
        CompoundCommand::While(command) => command.done_span.map(close_start),
        CompoundCommand::Until(command) => command.done_span.map(close_start),
        CompoundCommand::Case(command) => Some(close_start(command.esac_span)),
        CompoundCommand::Select(command) => Some(close_start(command.done_span)),
        CompoundCommand::Subshell(sequence) => Some(
            delimiter_before(source, sequence.span.to_range().end(), ')')
                .unwrap_or_else(|| sequence.span.to_range().end()),
        ),
        CompoundCommand::BraceGroup(sequence) => Some(
            delimiter_before(source, sequence.span.to_range().end(), '}')
                .unwrap_or_else(|| sequence.span.to_range().end()),
        ),
        CompoundCommand::Always(command) => Some(command.always_body.span.to_range().end()),
        CompoundCommand::Time(command) => command
            .command
            .as_deref()
            .and_then(|statement| statement_folding_end(statement, source)),
        CompoundCommand::Coproc(command) => statement_folding_end(&command.body, source),
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => None,
    }
}

fn delimiter_before(source: &str, offset: TextSize, delimiter: char) -> Option<TextSize> {
    let prefix = source.get(..usize::from(offset).min(source.len()))?;
    let trimmed = prefix.trim_end_matches(char::is_whitespace);
    trimmed
        .ends_with(delimiter)
        .then(|| TextSize::new(trimmed.len().saturating_sub(delimiter.len_utf8()) as u32))
}

fn heredoc_closing_marker_range(heredoc: &Heredoc, source: &str) -> Option<TextRange> {
    let mut start = heredoc.body.span.end.offset().min(source.len());
    if source
        .as_bytes()
        .get(start)
        .is_some_and(|byte| *byte == b'\n')
    {
        start += 1;
    }
    let line_end = source[start..]
        .find(['\n', '\r'])
        .map_or(source.len(), |offset| start + offset);
    let line = source.get(start..line_end)?;
    (line.trim_start_matches('\t') == heredoc.delimiter.cooked.as_str()).then_some(TextRange::new(
        TextSize::new(start as u32),
        TextSize::new(line_end as u32),
    ))
}

fn contains(range: TextRange, offset: TextSize) -> bool {
    range.start() <= offset && offset < range.end()
}

fn contains_any(ranges: &[TextRange], offset: TextSize) -> bool {
    containing_range(ranges, offset).is_some()
}

fn containing_range(ranges: &[TextRange], offset: TextSize) -> Option<TextRange> {
    let index = ranges.partition_point(|range| range.start() <= offset);
    let mut best = None;

    for range in ranges[..index].iter().copied() {
        if !contains(range, offset) {
            continue;
        }

        best = match best {
            None => Some(range),
            Some(current) if is_innermost(range, current) => Some(range),
            Some(current) => Some(current),
        };
    }

    best
}

fn is_innermost(candidate: TextRange, current: TextRange) -> bool {
    candidate.len() < current.len()
        || (candidate.len() == current.len() && candidate.start() >= current.start())
}

fn has_odd_backslash_run_before(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    text[..offset]
        .chars()
        .rev()
        .take_while(|&ch| ch == '\\')
        .count()
        % 2
        == 1
}

fn find_command_substitution_end(text: &str, start_offset: usize) -> Option<usize> {
    if start_offset >= text.len() || !text[start_offset..].starts_with("$(") {
        return None;
    }

    let mut index = start_offset + "$(".len();
    let mut paren_depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;

    while index < text.len() {
        let ch = text[index..].chars().next()?;
        let ch_len = ch.len_utf8();

        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            index += ch_len;
            continue;
        }

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '\'' && !in_double {
            in_single = true;
            index += ch_len;
            continue;
        }

        if ch == '"' {
            in_double = !in_double;
            index += ch_len;
            continue;
        }

        if ch == '$'
            && text[index..].starts_with("$(")
            && !has_odd_backslash_run_before(text, index)
        {
            index = find_command_substitution_end(text, index)?;
            continue;
        }

        if ch == '$'
            && text[index..].starts_with("${")
            && !has_odd_backslash_run_before(text, index)
        {
            index = find_parameter_expansion_end(text, index)?;
            continue;
        }

        if ch == '`' {
            index = find_backtick_substitution_end(text, index)?;
            continue;
        }

        match ch {
            '(' => {
                paren_depth += 1;
                index += ch_len;
            }
            ')' if paren_depth == 0 => return Some(index + ch_len),
            ')' => {
                paren_depth -= 1;
                index += ch_len;
            }
            _ => index += ch_len,
        }
    }

    None
}

fn find_backtick_substitution_end(text: &str, start_offset: usize) -> Option<usize> {
    if start_offset >= text.len() || !text[start_offset..].starts_with('`') {
        return None;
    }

    let mut index = start_offset + '`'.len_utf8();
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '`' {
            return Some(index + ch_len);
        }

        index += ch_len;
    }

    None
}

fn find_parameter_expansion_end(text: &str, start_offset: usize) -> Option<usize> {
    if start_offset >= text.len() || !text[start_offset..].starts_with("${") {
        return None;
    }

    let mut index = start_offset + "${".len();
    let mut depth = 1usize;
    let mut in_single = false;
    let mut in_double = false;

    while index < text.len() {
        let ch = text[index..].chars().next()?;
        let ch_len = ch.len_utf8();

        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            index += ch_len;
            continue;
        }

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '\'' && !in_double {
            in_single = true;
            index += ch_len;
            continue;
        }

        if ch == '"' {
            in_double = !in_double;
            index += ch_len;
            continue;
        }

        if ch == '$'
            && text[index..].starts_with("$(")
            && !has_odd_backslash_run_before(text, index)
        {
            index = find_command_substitution_end(text, index)?;
            continue;
        }

        if ch == '$'
            && text[index..].starts_with("${")
            && !has_odd_backslash_run_before(text, index)
        {
            depth += 1;
            index += "${".len();
            continue;
        }

        if ch == '`' {
            index = find_backtick_substitution_end(text, index)?;
            continue;
        }

        if ch == '}' {
            depth -= 1;
            index += ch_len;
            if depth == 0 {
                return Some(index);
            }
            continue;
        }

        index += ch_len;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn regions(source: &str) -> RegionIndex {
        let output = Parser::new(source).parse().unwrap();
        RegionIndex::new(source, &output.file)
    }

    fn regions_with_source_layout_indexes(source: &str) -> RegionIndex {
        let output = Parser::new(source).parse().unwrap();
        RegionIndex::with_source_layout_indexes(source, &output.file)
    }

    #[test]
    fn quote_coverage_matches_nested_ranges_at_every_byte() {
        for source in [
            "echo 'plain' \"outer $(echo 'inner') end\" tail\n",
            "cat <<'EOF'\nquoted $text\nEOF\necho \"tail\"\n",
            "echo \"é $(printf \"%s\" \"nested\")\" '' outside\n",
        ] {
            let index = regions(source);
            for byte in 0..=source.len() {
                let offset = TextSize::new(byte as u32);
                let expected = index
                    .single_quoted
                    .iter()
                    .chain(&index.double_quoted)
                    .chain(&index.quoted_heredocs)
                    .any(|range| contains(*range, offset));
                assert_eq!(index.is_quoted(offset), expected, "{source:?} at {byte}");
            }
        }
    }

    #[test]
    fn finds_single_and_double_quoted_regions() {
        let source = "echo 'hello' \"world $name\"\n";
        let regions = regions(source);

        let single = TextSize::new(source.find("hello").unwrap() as u32);
        let double = TextSize::new(source.find("world").unwrap() as u32);

        assert_eq!(regions.region_at(single), Some(RegionKind::SingleQuoted));
        assert_eq!(regions.region_at(double), Some(RegionKind::DoubleQuoted));
    }

    #[test]
    fn finds_command_substitution_and_arithmetic_regions() {
        let source = "echo $(printf hi) $((1 + 2))\n";
        let regions = regions(source);

        let command = TextSize::new(source.find("printf").unwrap() as u32);
        let arithmetic = TextSize::new(source.find("1 + 2").unwrap() as u32);

        assert_eq!(
            regions.region_at(command),
            Some(RegionKind::CommandSubstitution)
        );
        assert_eq!(regions.region_at(arithmetic), Some(RegionKind::Arithmetic));
        assert!(regions.is_command_substitution(command));
        assert!(regions.is_arithmetic(arithmetic));
    }

    #[test]
    fn tracks_backtick_command_substitution_ranges() {
        let source = "echo `printf '%s' \"$name\"` $(pwd)\n";
        let regions = regions(source);
        let start = source.find('`').unwrap();
        let end = source[start + 1..].find('`').unwrap() + start + 2;

        assert_eq!(
            regions.backtick_command_substitution_ranges(),
            &[TextRange::new(
                TextSize::new(start as u32),
                TextSize::new(end as u32),
            )]
        );
    }

    #[test]
    fn tracks_backtick_command_substitution_ranges_in_parameter_operands() {
        let source = "echo ${x:-`printf '%s' \"$name\"`}\n";
        let regions = regions(source);
        let start = source.find('`').unwrap();
        let end = source[start + 1..].find('`').unwrap() + start + 2;

        assert_eq!(
            regions.backtick_command_substitution_ranges(),
            &[TextRange::new(
                TextSize::new(start as u32),
                TextSize::new(end as u32),
            )]
        );
    }

    #[test]
    fn tracks_heredoc_closing_marker_ranges() {
        let source = "cat <<-EOF\n\tbody\n\tEOF\n";
        let regions = regions_with_source_layout_indexes(source);
        let body_start = source.find("body").unwrap();
        let body_range = regions
            .heredocs()
            .iter()
            .find(|heredoc| {
                heredoc.body_range.start() <= TextSize::new(body_start as u32)
                    && TextSize::new(body_start as u32) < heredoc.body_range.end()
            })
            .unwrap()
            .body_range;
        let close_start = source.rfind("\tEOF").unwrap();
        let close_range = TextRange::new(
            TextSize::new(close_start as u32),
            TextSize::new((close_start + "\tEOF".len()) as u32),
        );

        assert_eq!(
            regions.heredoc_closing_marker_range(body_range),
            Some(close_range)
        );
    }

    #[test]
    fn skips_heredoc_closing_marker_ranges_by_default() {
        let source = "cat <<-EOF\n\tbody\n\tEOF\n";
        let regions = regions(source);

        assert!(regions.heredocs().is_empty());
        assert!(
            regions
                .heredoc_closing_marker_range(regions.heredoc_ranges()[0])
                .is_none()
        );
    }

    #[test]
    fn tracks_quoted_regions_inside_keyed_array_subscripts() {
        let source = "declare -A map=(['$HOME']=1)\n";
        let regions = regions(source);
        let offset = TextSize::new(source.find("$HOME").unwrap() as u32);

        assert_eq!(regions.region_at(offset), Some(RegionKind::SingleQuoted));
        assert!(regions.is_quoted(offset));
    }

    #[test]
    fn finds_heredoc_regions_and_tracks_quoted_heredocs() {
        let source = "cat <<'EOF'\nhello $name\nEOF\n";
        let regions = regions(source);
        let offset = TextSize::new(source.find("hello $name").unwrap() as u32);

        assert_eq!(regions.region_at(offset), Some(RegionKind::Heredoc));
        assert!(regions.is_heredoc(offset));
        assert!(regions.is_quoted(offset));
    }

    #[test]
    fn excludes_quoted_heredoc_backticks_from_backtick_ranges() {
        let source = "cat <<'EOF'\n`printf quoted`\nEOF\ncat <<EOF\n`printf live`\nEOF\n";
        let regions = regions(source);
        let [range] = regions.backtick_command_substitution_ranges() else {
            panic!("expected one live backtick substitution");
        };

        assert_eq!(
            &source[usize::from(range.start())..usize::from(range.end())],
            "`printf live`"
        );
    }

    #[test]
    fn returns_the_innermost_nested_region() {
        let source = "echo \"$(printf '%s' \"$name\")\"\n";
        let regions = regions(source);

        let name = TextSize::new(source.find("$name").unwrap() as u32);
        let printf = TextSize::new(source.find("printf").unwrap() as u32);

        assert_eq!(regions.region_at(name), Some(RegionKind::DoubleQuoted));
        assert_eq!(
            regions.region_at(printf),
            Some(RegionKind::CommandSubstitution)
        );
        assert_eq!(
            regions.region_with_range_at(printf),
            Some((
                RegionKind::CommandSubstitution,
                TextRange::new(
                    TextSize::new(source.find("$(printf").unwrap() as u32),
                    TextSize::new(source.rfind(')').unwrap() as u32 + 1),
                )
            ))
        );
    }

    #[test]
    fn tracks_conditional_ranges() {
        let source = "[[ \"$name\" =~ foo ]]\n";
        let regions = regions(source);
        let offset = TextSize::new(source.find("foo").unwrap() as u32);

        assert_eq!(regions.region_at(offset), Some(RegionKind::Conditional));
    }

    #[test]
    fn flags_parameter_expansion_brace_edges() {
        let source = "echo ${name}\n";
        let regions = regions(source);
        let open = TextSize::new(source.find('{').unwrap() as u32);
        let close = TextSize::new(source.find('}').unwrap() as u32);

        assert!(regions.is_expansion_brace_edge(open));
        assert!(regions.is_expansion_brace_edge(close));
        let interior = TextSize::new(source.find("name").unwrap() as u32);
        assert!(!regions.is_expansion_brace_edge(interior));
    }

    #[test]
    fn flags_active_brace_expansion_edges() {
        let source = "echo {a,b,c}\n";
        let regions = regions(source);
        let open = TextSize::new(source.find('{').unwrap() as u32);
        let close = TextSize::new(source.find('}').unwrap() as u32);

        assert!(regions.is_expansion_brace_edge(open));
        assert!(regions.is_expansion_brace_edge(close));
    }

    #[test]
    fn flags_parameter_expansion_brace_edges_in_heredocs() {
        let source = "cat <<EOF\nhello ${name}\nEOF\n";
        let regions = regions(source);
        let open = TextSize::new(source.find('{').unwrap() as u32);
        let close = TextSize::new(source.find('}').unwrap() as u32);

        assert!(regions.is_expansion_brace_edge(open));
        assert!(regions.is_expansion_brace_edge(close));
    }

    #[test]
    fn flags_braced_variable_edges_in_multiline_double_quoted_assignment() {
        let source = "payload=\"{\n  \\\"description\\\": \\\"${MESSAGE}\\\"\n}\"\n";
        let regions = regions(source);
        let parameter_start = source.find("${MESSAGE}").unwrap();
        let parameter_open = TextSize::new((parameter_start + 1) as u32);
        let parameter_close = TextSize::new((parameter_start + "${MESSAGE}".len() - 1) as u32);
        let parameter_pair = TextRange::new(
            TextSize::new(parameter_start as u32),
            TextSize::new((parameter_start + "${MESSAGE}".len()) as u32),
        );

        assert!(regions.is_expansion_brace_edge(parameter_open));
        assert!(regions.is_expansion_brace_edge(parameter_close));
        assert_eq!(
            regions.first_dollar_brace_pair_in(TextRange::new(
                TextSize::new((parameter_start - 3) as u32),
                TextSize::new((parameter_start + "MESSAGE".len()) as u32),
            )),
            Some(parameter_pair)
        );

        let object_open = TextSize::new(source.find("{\n").unwrap() as u32);
        let object_close = TextSize::new((source.rfind("\n}").unwrap() + 1) as u32);
        assert!(!regions.is_expansion_brace_edge(object_open));
        assert!(!regions.is_expansion_brace_edge(object_close));
    }

    #[test]
    fn ignores_incomplete_parameter_expansion_recovery_edges() {
        let source = "echo ${var:1\n";
        let regions = regions(source);
        let parameter_start = source.find("${").unwrap();
        let range = TextRange::new(
            TextSize::new(parameter_start as u32),
            TextSize::new(source.len() as u32),
        );

        assert_eq!(regions.first_dollar_brace_pair_in(range), None);
        for offset in parameter_start..source.len() {
            assert!(
                !regions.is_expansion_brace_edge(TextSize::new(offset as u32)),
                "unexpected expansion brace edge at byte {offset}"
            );
        }
    }

    #[test]
    fn skips_command_substitutions_when_indexing_parameter_expansion_end() {
        let source = "echo \"${x:-$(echo })}\"\n";
        let regions = regions(source);
        let parameter_start = source.find("${").unwrap();
        let parameter_end = source.rfind('}').unwrap() + '}'.len_utf8();
        let inner_literal_close = TextSize::new(source.find('}').unwrap() as u32);
        let outer_parameter_close = TextSize::new((parameter_end - 1) as u32);
        let parameter_pair = TextRange::new(
            TextSize::new(parameter_start as u32),
            TextSize::new(parameter_end as u32),
        );

        assert!(!regions.is_expansion_brace_edge(inner_literal_close));
        assert!(regions.is_expansion_brace_edge(outer_parameter_close));
        assert_eq!(
            regions.first_dollar_brace_pair_in(TextRange::new(
                TextSize::new(parameter_start as u32),
                TextSize::new((parameter_start + "${x".len()) as u32),
            )),
            Some(parameter_pair)
        );
    }

    #[test]
    fn skips_backticks_when_indexing_parameter_expansion_end() {
        assert_eq!(
            find_parameter_expansion_end("\"${x:-`echo }`}\"", 1),
            Some("\"${x:-`echo }`}".len())
        );
    }

    #[test]
    fn keeps_backslashes_literal_inside_single_quotes_when_indexing_parameter_expansion_end() {
        assert_eq!(
            find_parameter_expansion_end("${x:-'\\'}", 0),
            Some("${x:-'\\'}".len())
        );
    }

    #[test]
    fn skips_parameter_expansions_when_indexing_command_substitution_end() {
        let source = "echo \"${outer:-$(echo ${x//)/y})}\"\n";
        let regions = regions(source);
        let outer_start = source.find("${outer").unwrap();
        let outer_end = source.rfind('}').unwrap() + '}'.len_utf8();
        let inner_close_paren = TextSize::new(source.find(')').unwrap() as u32);
        let outer_parameter_close = TextSize::new((outer_end - 1) as u32);
        let outer_pair = TextRange::new(
            TextSize::new(outer_start as u32),
            TextSize::new(outer_end as u32),
        );

        assert!(!regions.is_expansion_brace_edge(inner_close_paren));
        assert!(regions.is_expansion_brace_edge(outer_parameter_close));
        assert_eq!(
            regions.first_dollar_brace_pair_in(TextRange::new(
                TextSize::new(outer_start as u32),
                TextSize::new((outer_start + "${outer".len()) as u32),
            )),
            Some(outer_pair)
        );
    }

    #[test]
    fn indexes_parameter_expansion_edges_inside_arithmetic_expansions() {
        let source = "echo $(( ${x:-1} + 1 ))\n";
        let regions = regions(source);
        let parameter_start = source.find("${x:-1}").unwrap();
        let parameter_open = TextSize::new((parameter_start + 1) as u32);
        let parameter_close = TextSize::new((parameter_start + "${x:-1}".len() - 1) as u32);
        let parameter_pair = TextRange::new(
            TextSize::new(parameter_start as u32),
            TextSize::new((parameter_start + "${x:-1}".len()) as u32),
        );

        assert!(regions.is_expansion_brace_edge(parameter_open));
        assert!(regions.is_expansion_brace_edge(parameter_close));
        assert_eq!(
            regions.first_dollar_brace_pair_in(TextRange::new(
                TextSize::new(parameter_start as u32),
                TextSize::new((parameter_start + "${x".len()) as u32),
            )),
            Some(parameter_pair)
        );
    }

    #[test]
    fn skips_backticks_when_indexing_command_substitution_end() {
        assert_eq!(
            find_command_substitution_end("$(echo `echo )`)", 0),
            Some("$(echo `echo )`)".len())
        );
    }

    #[test]
    fn does_not_flag_literal_braces() {
        let source = "echo {literal}\n";
        let regions = regions(source);
        let open = TextSize::new(source.find('{').unwrap() as u32);
        let close = TextSize::new(source.find('}').unwrap() as u32);

        assert!(!regions.is_expansion_brace_edge(open));
        assert!(!regions.is_expansion_brace_edge(close));
    }
}
