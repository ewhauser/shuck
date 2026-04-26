#[derive(Debug, Clone)]
struct ArenaCommandNameFacts {
    literal_name: Option<Box<str>>,
    effective_name: Option<Box<str>>,
    wrappers: Box<[WrapperKind]>,
    body_span: Span,
    body_word_span: Option<Span>,
    body_words: Box<[WordId]>,
    single_declaration_assignment: Option<CommandSingleAssignmentInfo>,
}

impl ArenaCommandNameFacts {
    fn from_normalized(normalized: command::ArenaNormalizedCommand<'_>) -> Self {
        let single_declaration_assignment = normalized
            .declaration
            .as_ref()
            .and_then(arena_single_declaration_assignment_info);
        Self {
            literal_name: normalized
                .literal_name
                .map(|name| name.into_owned().into_boxed_str()),
            effective_name: normalized
                .effective_name
                .map(|name| name.into_owned().into_boxed_str()),
            wrappers: normalized.wrappers.into_boxed_slice(),
            body_span: normalized.body_span,
            body_word_span: normalized.body_word_span,
            body_words: normalized.body_words.into_boxed_slice(),
            single_declaration_assignment,
        }
    }

    fn effective_or_literal_name(&self) -> Option<&str> {
        self.effective_name
            .as_deref()
            .or(self.literal_name.as_deref())
    }

    fn body_name_word_id(&self) -> Option<WordId> {
        self.body_words.first().copied()
    }

    fn body_args(&self) -> &[WordId] {
        self.body_words.split_first().map_or(&[], |(_, rest)| rest)
    }
}

#[derive(Debug, Clone)]
struct CommandSingleAssignmentInfo {
    target: Box<str>,
    span: Span,
}

fn arena_single_declaration_assignment_info(
    declaration: &command::ArenaNormalizedDeclaration<'_>,
) -> Option<CommandSingleAssignmentInfo> {
    if !declaration.assignments.is_empty() {
        return None;
    }

    let mut assignment = None;
    for operand in declaration.operands {
        match operand {
            DeclOperandNode::Flag(_) => {}
            DeclOperandNode::Assignment(candidate) => {
                if assignment.replace(candidate).is_some() {
                    return None;
                }
            }
            DeclOperandNode::Name(_) | DeclOperandNode::Dynamic(_) => return None,
        }
    }

    let assignment = assignment?;
    Some(CommandSingleAssignmentInfo {
        target: assignment.target.name.as_str().to_owned().into_boxed_str(),
        span: assignment.span,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandFactCompoundKind {
    If,
    ArithmeticFor,
    For,
    Repeat,
    While,
    Until,
    Subshell,
    BraceGroup,
    Always,
    Case,
    Select,
    Foreach,
    Arithmetic,
    Time,
    Conditional,
    Coproc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandFactShape {
    kind: ArenaFileCommandKind,
    binary_op: Option<BinaryOp>,
    binary_op_span: Option<Span>,
    compound_kind: Option<CommandFactCompoundKind>,
}

impl CommandFactShape {
    fn from_arena(command: CommandView<'_>) -> Self {
        let kind = command.kind();
        let (binary_op, binary_op_span) = command
            .binary()
            .map_or((None, None), |binary| (Some(binary.op()), Some(binary.op_span())));
        let compound_kind = command
            .compound()
            .map(|compound| CommandFactCompoundKind::from_arena(compound.node()));
        Self {
            kind,
            binary_op,
            binary_op_span,
            compound_kind,
        }
    }

    fn from_recursive(command: &Command) -> Self {
        match command {
            Command::Simple(_) => Self {
                kind: ArenaFileCommandKind::Simple,
                binary_op: None,
                binary_op_span: None,
                compound_kind: None,
            },
            Command::Builtin(_) => Self {
                kind: ArenaFileCommandKind::Builtin,
                binary_op: None,
                binary_op_span: None,
                compound_kind: None,
            },
            Command::Decl(_) => Self {
                kind: ArenaFileCommandKind::Decl,
                binary_op: None,
                binary_op_span: None,
                compound_kind: None,
            },
            Command::Binary(command) => Self {
                kind: ArenaFileCommandKind::Binary,
                binary_op: Some(command.op),
                binary_op_span: Some(command.op_span),
                compound_kind: None,
            },
            Command::Compound(command) => Self {
                kind: ArenaFileCommandKind::Compound,
                binary_op: None,
                binary_op_span: None,
                compound_kind: Some(CommandFactCompoundKind::from_recursive(command)),
            },
            Command::Function(_) => Self {
                kind: ArenaFileCommandKind::Function,
                binary_op: None,
                binary_op_span: None,
                compound_kind: None,
            },
            Command::AnonymousFunction(_) => Self {
                kind: ArenaFileCommandKind::AnonymousFunction,
                binary_op: None,
                binary_op_span: None,
                compound_kind: None,
            },
        }
    }

    fn is_short_circuit_binary(self) -> bool {
        matches!(self.binary_op, Some(BinaryOp::And | BinaryOp::Or))
    }
}

impl CommandFactCompoundKind {
    fn from_recursive(command: &CompoundCommand) -> Self {
        match command {
            CompoundCommand::If(_) => Self::If,
            CompoundCommand::ArithmeticFor(_) => Self::ArithmeticFor,
            CompoundCommand::For(_) => Self::For,
            CompoundCommand::Repeat(_) => Self::Repeat,
            CompoundCommand::While(_) => Self::While,
            CompoundCommand::Until(_) => Self::Until,
            CompoundCommand::Subshell(_) => Self::Subshell,
            CompoundCommand::BraceGroup(_) => Self::BraceGroup,
            CompoundCommand::Always(_) => Self::Always,
            CompoundCommand::Case(_) => Self::Case,
            CompoundCommand::Select(_) => Self::Select,
            CompoundCommand::Foreach(_) => Self::Foreach,
            CompoundCommand::Arithmetic(_) => Self::Arithmetic,
            CompoundCommand::Time(_) => Self::Time,
            CompoundCommand::Conditional(_) => Self::Conditional,
            CompoundCommand::Coproc(_) => Self::Coproc,
        }
    }

    fn from_arena(command: &CompoundCommandNode) -> Self {
        match command {
            CompoundCommandNode::If { .. } => Self::If,
            CompoundCommandNode::ArithmeticFor { .. } => Self::ArithmeticFor,
            CompoundCommandNode::For { .. } => Self::For,
            CompoundCommandNode::Repeat { .. } => Self::Repeat,
            CompoundCommandNode::While { .. } => Self::While,
            CompoundCommandNode::Until { .. } => Self::Until,
            CompoundCommandNode::Subshell(_) => Self::Subshell,
            CompoundCommandNode::BraceGroup(_) => Self::BraceGroup,
            CompoundCommandNode::Always { .. } => Self::Always,
            CompoundCommandNode::Case { .. } => Self::Case,
            CompoundCommandNode::Select { .. } => Self::Select,
            CompoundCommandNode::Foreach { .. } => Self::Foreach,
            CompoundCommandNode::Arithmetic(_) => Self::Arithmetic,
            CompoundCommandNode::Time { .. } => Self::Time,
            CompoundCommandNode::Conditional(_) => Self::Conditional,
            CompoundCommandNode::Coproc { .. } => Self::Coproc,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandFact<'a> {
    id: CommandId,
    key: FactSpan,
    span: Span,
    arena_stmt_id: Option<AstStmtId>,
    arena_command_id: Option<AstCommandId>,
    shape: CommandFactShape,
    stmt_span: Span,
    stmt_negated: bool,
    stmt_terminator: Option<StmtTerminator>,
    stmt_terminator_span: Option<Span>,
    has_redirects: bool,
    has_assignments: bool,
    nested_word_command: bool,
    scope: Option<ScopeId>,
    normalized: NormalizedCommand<'a>,
    arena_normalized: Option<ArenaCommandNameFacts>,
    zsh_options: Option<ZshOptionState>,
    redirect_facts: IdRange<RedirectFact>,
    substitution_facts: IdRange<SubstitutionFact>,
    options: CommandOptionFacts,
    scope_read_source_words: IdRange<PathWordFact>,
    scope_name_read_uses: IdRange<ComparableNameUse>,
    scope_heredoc_name_read_uses: IdRange<ComparableNameUse>,
    scope_name_write_uses: IdRange<ComparableNameUse>,
    declaration_assignment_probes: IdRange<DeclarationAssignmentProbe>,
    glued_closing_bracket_operand_span: Option<Span>,
    glued_closing_bracket_insert_offset: Option<usize>,
    linebreak_in_test_anchor_span: Option<Span>,
    linebreak_in_test_insert_offset: Option<usize>,
    simple_test: Option<SimpleTestFact<'a>>,
    conditional: Option<ConditionalFact<'a>>,
}

impl<'a> CommandFact<'a> {
    pub fn id(&self) -> CommandId {
        self.id
    }

    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn arena_stmt_id(&self) -> Option<AstStmtId> {
        self.arena_stmt_id
    }

    pub fn arena_command_id(&self) -> Option<AstCommandId> {
        self.arena_command_id
    }

    fn shape(&self) -> CommandFactShape {
        self.shape
    }

    pub(crate) fn command_kind(&self) -> ArenaFileCommandKind {
        self.shape.kind
    }

    pub(crate) fn compound_kind(&self) -> Option<CommandFactCompoundKind> {
        self.shape.compound_kind
    }

    pub(crate) fn binary_op(&self) -> Option<BinaryOp> {
        self.shape.binary_op
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn scope(&self) -> Option<ScopeId> {
        self.scope
    }

    pub fn stmt_span(&self) -> Span {
        self.stmt_span
    }

    pub fn stmt_negated(&self) -> bool {
        self.stmt_negated
    }

    pub fn stmt_terminator(&self) -> Option<StmtTerminator> {
        self.stmt_terminator
    }

    pub fn stmt_terminator_span(&self) -> Option<Span> {
        self.stmt_terminator_span
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.span(), source)
    }

    pub fn has_redirects(&self) -> bool {
        self.has_redirects
    }

    pub fn zsh_options(&self) -> Option<&ZshOptionState> {
        self.zsh_options.as_ref()
    }

    pub fn normalized(&self) -> &NormalizedCommand<'a> {
        &self.normalized
    }

    pub fn options(&self) -> &CommandOptionFacts {
        &self.options
    }

    pub fn glued_closing_bracket_operand_span(&self) -> Option<Span> {
        self.glued_closing_bracket_operand_span
    }

    pub fn glued_closing_bracket_insert_offset(&self) -> Option<usize> {
        self.glued_closing_bracket_insert_offset
    }

    pub fn linebreak_in_test_anchor_span(&self) -> Option<Span> {
        self.linebreak_in_test_anchor_span
    }

    pub fn linebreak_in_test_insert_offset(&self) -> Option<usize> {
        self.linebreak_in_test_insert_offset
    }

    pub fn simple_test(&self) -> Option<&SimpleTestFact<'a>> {
        self.simple_test.as_ref()
    }

    pub fn conditional(&self) -> Option<&ConditionalFact<'a>> {
        self.conditional.as_ref()
    }

    pub fn literal_name(&self) -> Option<&str> {
        self.arena_normalized
            .as_ref()
            .and_then(|normalized| normalized.literal_name.as_deref())
            .or(self.normalized.literal_name.as_deref())
    }

    pub fn effective_name(&self) -> Option<&str> {
        self.arena_normalized
            .as_ref()
            .and_then(|normalized| normalized.effective_name.as_deref())
            .or(self.normalized.effective_name.as_deref())
    }

    pub fn effective_or_literal_name(&self) -> Option<&str> {
        self.arena_normalized
            .as_ref()
            .and_then(ArenaCommandNameFacts::effective_or_literal_name)
            .or_else(|| self.normalized.effective_or_literal_name())
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.effective_name() == Some(name)
    }

    pub fn static_utility_name(&self) -> Option<&str> {
        self.effective_or_literal_name()
    }

    pub fn static_utility_name_is(&self, name: &str) -> bool {
        self.static_utility_name() == Some(name)
    }

    pub fn wrappers(&self) -> &[WrapperKind] {
        self.arena_normalized
            .as_ref()
            .map_or(&self.normalized.wrappers, |normalized| &normalized.wrappers)
    }

    pub fn has_wrapper(&self, wrapper: WrapperKind) -> bool {
        self.wrappers().contains(&wrapper)
    }

    pub fn declaration(&self) -> Option<&NormalizedDeclaration<'a>> {
        self.normalized.declaration.as_ref()
    }

    fn single_declaration_assignment_info(&self) -> Option<(&str, Span)> {
        self.arena_normalized
            .as_ref()
            .and_then(|normalized| normalized.single_declaration_assignment.as_ref())
            .map(|assignment| (assignment.target.as_ref(), assignment.span))
    }

    pub fn body_span(&self) -> Span {
        self.arena_normalized
            .as_ref()
            .map_or(self.normalized.body_span, |normalized| normalized.body_span)
    }

    pub fn body_name_word(&self) -> Option<&'a Word> {
        self.normalized.body_name_word()
    }

    pub fn body_word_span(&self) -> Option<Span> {
        self.arena_normalized
            .as_ref()
            .and_then(|normalized| normalized.body_word_span)
            .or_else(|| self.normalized.body_word_span())
    }

    pub fn body_word_contains_template_placeholder(&self, source: &str) -> bool {
        let Some(span) = self.body_word_span() else {
            return false;
        };
        contains_template_placeholder_text(span.slice(source))
    }

    pub fn body_word_has_suspicious_quoted_command_trailer(
        &self,
        source: &str,
        trailing_literal_char: Option<char>,
    ) -> bool {
        let Some(span) = self.body_word_span() else {
            return false;
        };
        quoted_command_name_has_suspicious_ending(span.slice(source), trailing_literal_char)
    }

    pub fn body_word_has_hash_suffix(&self, source: &str) -> bool {
        let Some(span) = self.body_word_span() else {
            return false;
        };
        let text = span.slice(source);
        text != "#" && text.ends_with('#')
    }

    pub fn bracket_command_name_needs_separator(&self, source: &str) -> bool {
        if self.literal_name() != Some("[") {
            return false;
        }

        let Some(span) = self.body_word_span() else {
            return false;
        };
        let raw = span.slice(source);
        raw != "[" || self.has_assignments
    }

    pub fn body_args(&self) -> &[&'a Word] {
        self.normalized.body_args()
    }

    pub fn arena_body_name_word<'facts>(
        &self,
        arena_file: &'facts ArenaFile,
        source: &'facts str,
    ) -> Option<FactWordRef<'facts>> {
        let command_id = self.arena_command_id?;
        self.arena_normalized
            .as_ref()
            .and_then(ArenaCommandNameFacts::body_name_word_id)
            .or_else(|| {
                let command = arena_file.store.command(command_id);
                command::normalize_arena_command(command, source).body_name_word_id()
            })
            .map(|id| FactWordRef::new(arena_file, id))
    }

    pub fn arena_body_args<'facts>(
        &self,
        arena_file: &'facts ArenaFile,
        source: &'facts str,
    ) -> Vec<FactWordRef<'facts>> {
        let Some(command_id) = self.arena_command_id else {
            return Vec::new();
        };
        let owned_args;
        let args = if let Some(normalized) = self.arena_normalized.as_ref() {
            normalized.body_args()
        } else {
            let command = arena_file.store.command(command_id);
            owned_args = command::normalize_arena_command(command, source)
                .body_args()
                .to_vec();
            &owned_args
        };
        args.iter()
            .copied()
            .map(|id| FactWordRef::new(arena_file, id))
            .collect()
    }

}

impl<'facts, 'a> CommandFactRef<'facts, 'a> {
    pub fn id(self) -> CommandId {
        self.fact.id()
    }

    pub fn key(self) -> FactSpan {
        self.fact.key()
    }

    pub fn is_nested_word_command(self) -> bool {
        self.fact.is_nested_word_command()
    }

    pub fn scope(self) -> Option<ScopeId> {
        self.fact.scope()
    }

    pub fn stmt_span(self) -> Span {
        self.arena_stmt()
            .map_or_else(|| self.fact.stmt_span(), |stmt| stmt.span())
    }

    pub fn stmt_negated(self) -> bool {
        self.arena_stmt()
            .map_or_else(|| self.fact.stmt_negated(), |stmt| stmt.negated())
    }

    pub fn stmt_terminator(self) -> Option<StmtTerminator> {
        self.arena_stmt()
            .map_or_else(|| self.fact.stmt_terminator(), |stmt| stmt.terminator())
    }

    pub fn stmt_terminator_span(self) -> Option<Span> {
        self.arena_stmt()
            .map_or_else(|| self.fact.stmt_terminator_span(), |stmt| stmt.terminator_span())
    }

    pub fn span(self) -> Span {
        self.arena_command()
            .map_or_else(|| self.fact.span(), |command| command.span())
    }

    pub fn span_in_source(self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.span(), source)
    }

    pub fn has_redirects(self) -> bool {
        self.arena_stmt()
            .map_or_else(|| self.fact.has_redirects(), |stmt| !stmt.redirects().is_empty())
    }

    pub fn arena_stmt(self) -> Option<StmtView<'facts>> {
        self.fact
            .arena_stmt_id
            .map(|id| self.arena_file.store.stmt(id))
    }

    pub fn arena_command(self) -> Option<CommandView<'facts>> {
        self.fact
            .arena_command_id
            .map(|id| self.arena_file.store.command(id))
    }

    pub(crate) fn command_kind(self) -> ArenaFileCommandKind {
        self.fact.command_kind()
    }

    pub(crate) fn compound_kind(self) -> Option<CommandFactCompoundKind> {
        self.fact.compound_kind()
    }

    pub(crate) fn binary_op(self) -> Option<BinaryOp> {
        self.fact.binary_op()
    }

    pub fn arena_redirects(self) -> Option<&'facts [RedirectNode]> {
        self.arena_stmt().map(|stmt| stmt.redirects())
    }

    pub fn arena_assignments(self) -> &'facts [AssignmentNode] {
        self.arena_command()
            .map(arena_command_assignments)
            .unwrap_or(&[])
    }

    pub fn arena_declaration_operands(self) -> &'facts [DeclOperandNode] {
        self.arena_command()
            .map(arena_declaration_operands)
            .unwrap_or(&[])
    }

    pub fn zsh_options(self) -> Option<&'facts ZshOptionState> {
        self.fact.zsh_options.as_ref()
    }

    pub fn redirect_facts(self) -> &'facts [RedirectFact] {
        self.store.redirect_facts(self.fact.redirect_facts)
    }

    pub fn substitution_facts(self) -> &'facts [SubstitutionFact] {
        self.store.substitution_facts(self.fact.substitution_facts)
    }

    pub fn scope_read_source_words(self) -> &'facts [PathWordFact] {
        self.store
            .scope_read_source_words(self.fact.scope_read_source_words)
    }

    pub(crate) fn scope_name_read_uses(self) -> &'facts [ComparableNameUse] {
        self.store.scope_name_read_uses(self.fact.scope_name_read_uses)
    }

    pub(crate) fn scope_heredoc_name_read_uses(self) -> &'facts [ComparableNameUse] {
        self.store
            .scope_heredoc_name_read_uses(self.fact.scope_heredoc_name_read_uses)
    }

    pub(crate) fn scope_name_write_uses(self) -> &'facts [ComparableNameUse] {
        self.store
            .scope_name_write_uses(self.fact.scope_name_write_uses)
    }

    pub fn declaration_assignment_probes(self) -> &'facts [DeclarationAssignmentProbe] {
        self.store
            .declaration_assignment_probes(self.fact.declaration_assignment_probes)
    }

    pub fn normalized(self) -> &'facts NormalizedCommand<'a> {
        &self.fact.normalized
    }

    pub fn arena_normalized(
        self,
        source: &'facts str,
    ) -> Option<command::ArenaNormalizedCommand<'facts>> {
        self.arena_command()
            .map(|command| command::normalize_arena_command(command, source))
    }

    pub fn arena_literal_name(self, source: &'facts str) -> Option<Cow<'facts, str>> {
        self.arena_normalized(source)?.literal_name
    }

    pub fn arena_effective_name(self, source: &'facts str) -> Option<Cow<'facts, str>> {
        self.arena_normalized(source)?.effective_name
    }

    pub fn arena_effective_or_literal_name(self, source: &'facts str) -> Option<Cow<'facts, str>> {
        let normalized = self.arena_normalized(source)?;
        normalized.effective_name.or(normalized.literal_name)
    }

    pub fn arena_body_name_word_id(self, source: &'facts str) -> Option<WordId> {
        self.arena_normalized(source)?.body_name_word_id()
    }

    pub fn arena_body_name_word(self, source: &'facts str) -> Option<FactWordRef<'facts>> {
        self.arena_body_name_word_id(source)
            .map(|id| FactWordRef::new(self.arena_file, id))
    }

    pub fn arena_body_word_ids(self, source: &'facts str) -> Vec<WordId> {
        self.arena_normalized(source)
            .map_or_else(Vec::new, |normalized| normalized.body_words)
    }

    pub fn arena_body_args(self, source: &'facts str) -> Vec<FactWordRef<'facts>> {
        self.arena_normalized(source)
            .map_or_else(Vec::new, |normalized| {
                normalized
                    .body_args()
                    .iter()
                    .copied()
                    .map(|id| FactWordRef::new(self.arena_file, id))
                    .collect()
            })
    }

    pub fn options(self) -> &'facts CommandOptionFacts {
        &self.fact.options
    }

    pub fn glued_closing_bracket_operand_span(self) -> Option<Span> {
        self.fact.glued_closing_bracket_operand_span()
    }

    pub fn glued_closing_bracket_insert_offset(self) -> Option<usize> {
        self.fact.glued_closing_bracket_insert_offset()
    }

    pub fn linebreak_in_test_anchor_span(self) -> Option<Span> {
        self.fact.linebreak_in_test_anchor_span()
    }

    pub fn linebreak_in_test_insert_offset(self) -> Option<usize> {
        self.fact.linebreak_in_test_insert_offset()
    }

    pub fn simple_test(self) -> Option<&'facts SimpleTestFact<'a>> {
        self.fact.simple_test.as_ref()
    }

    pub fn conditional(self) -> Option<&'facts ConditionalFact<'a>> {
        self.fact.conditional.as_ref()
    }

    pub fn literal_name(self) -> Option<&'facts str> {
        self.fact.literal_name()
    }

    pub fn effective_name(self) -> Option<&'facts str> {
        self.fact.effective_name()
    }

    pub fn effective_or_literal_name(self) -> Option<&'facts str> {
        self.fact.effective_or_literal_name()
    }

    pub fn effective_name_is(self, name: &str) -> bool {
        self.fact.effective_name_is(name)
    }

    pub fn static_utility_name(self) -> Option<&'facts str> {
        self.effective_or_literal_name()
    }

    pub fn static_utility_name_is(self, name: &str) -> bool {
        self.static_utility_name() == Some(name)
    }

    pub fn wrappers(self) -> &'facts [WrapperKind] {
        self.fact.wrappers()
    }

    pub fn has_wrapper(self, wrapper: WrapperKind) -> bool {
        self.fact.has_wrapper(wrapper)
    }

    pub fn declaration(self) -> Option<&'facts NormalizedDeclaration<'a>> {
        self.fact.normalized.declaration.as_ref()
    }

    pub fn body_span(self) -> Span {
        self.fact.body_span()
    }

    pub fn body_name_word(self) -> Option<&'a Word> {
        self.fact.body_name_word()
    }

    pub fn body_word_span(self) -> Option<Span> {
        self.fact.body_word_span()
    }

    pub fn body_word_contains_template_placeholder(self, source: &str) -> bool {
        self.fact.body_word_contains_template_placeholder(source)
    }

    pub fn body_word_has_suspicious_quoted_command_trailer(
        self,
        source: &str,
        trailing_literal_char: Option<char>,
    ) -> bool {
        self.fact
            .body_word_has_suspicious_quoted_command_trailer(source, trailing_literal_char)
    }

    pub fn body_word_has_hash_suffix(self, source: &str) -> bool {
        self.fact.body_word_has_hash_suffix(source)
    }

    pub fn bracket_command_name_needs_separator(self, source: &str) -> bool {
        self.fact.bracket_command_name_needs_separator(source)
    }

    pub fn body_args(self) -> &'facts [&'a Word] {
        self.fact.normalized.body_args()
    }

    pub fn file_operand_words(self) -> Vec<FactWordSpan> {
        self.fact
            .options
            .file_operand_path_facts()
            .iter()
            .map(|fact| FactWordSpan { span: fact.span() })
            .collect()
    }

    pub fn shellcheck_command_span(self, source: &str) -> Option<Span> {
        command_span_with_redirects_and_shellcheck_tail(self, source)
            .map(|span| trim_trailing_whitespace_span(span, source))
    }
}

fn pipeline_span_with_shellcheck_tail(
    commands: CommandFacts<'_, '_>,
    pipeline: &PipelineFact,
    source: &str,
) -> Span {
    let Some(first_segment) = pipeline.first_segment() else {
        unreachable!("pipeline has segments");
    };
    let Some(last_segment) = pipeline.last_segment() else {
        unreachable!("pipeline has segments");
    };
    let first = command_fact_ref(commands, first_segment.command_id());
    let last = command_fact_ref(commands, last_segment.command_id());
    let last_end = last.span_in_source(source).end;
    let end = extend_over_shellcheck_trailing_inline_space(last_end, source);

    let Some(body_name_word) = first.arena_body_name_word(source) else {
        unreachable!("plain echo command should have a body name");
    };
    Span::from_positions(body_name_word.span().start, end)
}

fn command_span_with_redirects_and_shellcheck_tail(
    command: CommandFactRef<'_, '_>,
    source: &str,
) -> Option<Span> {
    let body_name = command.arena_body_name_word(source)?;
    let mut end = body_name.span().end;

    for word in command.arena_body_args(source) {
        if word.span().end.offset > end.offset {
            end = word.span().end;
        }
    }

    for redirect in command.arena_redirects().into_iter().flatten() {
        let redirect_end = redirect.span.end;
        if redirect_end.offset > end.offset {
            end = redirect_end;
        }
    }

    Some(Span::from_positions(
        body_name.span().start,
        extend_over_shellcheck_trailing_inline_space(end, source),
    ))
}

fn effective_command_zsh_options(
    semantic: &SemanticModel,
    offset: usize,
    normalized: &NormalizedCommand<'_>,
) -> Option<ZshOptionState> {
    let mut options = semantic.zsh_options_at(offset).cloned();
    if normalized.has_wrapper(WrapperKind::Noglob)
        && let Some(options) = options.as_mut()
    {
        options.glob = shuck_semantic::OptionValue::Off;
    }
    options
}

fn contains_template_placeholder_text(text: &str) -> bool {
    let Some(start) = text.find("{{") else {
        return false;
    };
    text[start + 2..].contains("}}")
}

fn quoted_command_name_has_suspicious_ending(
    text: &str,
    trailing_literal_char: Option<char>,
) -> bool {
    let Some(inner) = strip_matching_quotes(text) else {
        return false;
    };

    let Some(ch) = trailing_literal_char.or_else(|| inner.chars().next_back()) else {
        return false;
    };
    if !is_suspicious_command_trailer(ch) {
        return false;
    }
    if trailing_literal_char.is_some() {
        return true;
    }

    match ch {
        '}' => !inner_ends_with_parameter_expansion(inner),
        ')' => !inner_ends_with_command_substitution(inner),
        _ => true,
    }
}

fn strip_matching_quotes(text: &str) -> Option<&str> {
    if text.len() < 2 {
        return None;
    }

    match (
        text.as_bytes().first().copied(),
        text.as_bytes().last().copied(),
    ) {
        (Some(b'"'), Some(b'"')) | (Some(b'\''), Some(b'\'')) => Some(&text[1..text.len() - 1]),
        _ => None,
    }
}

fn is_suspicious_command_trailer(ch: char) -> bool {
    matches!(
        ch,
        '.' | ',' | '#' | '[' | ']' | '(' | ')' | '{' | '}' | '\''
    )
}

fn inner_ends_with_parameter_expansion(inner: &str) -> bool {
    matching_shell_delimiter_start(inner, b'{', b'}')
        .is_some_and(|index| index > 0 && inner.as_bytes()[index - 1] == b'$')
}

fn inner_ends_with_command_substitution(inner: &str) -> bool {
    matching_shell_delimiter_start(inner, b'(', b')')
        .is_some_and(|index| index > 0 && inner.as_bytes()[index - 1] == b'$')
}

fn matching_shell_delimiter_start(inner: &str, open: u8, close: u8) -> Option<usize> {
    let bytes = inner.as_bytes();
    if bytes.last().copied() != Some(close) {
        return None;
    }

    let mut depth = 1usize;
    let mut quote_state = None;
    let mut index = bytes.len() - 1;

    while index > 0 {
        index -= 1;
        match quote_state {
            Some(QuoteState::Single) => {
                if bytes[index] == b'\'' {
                    quote_state = None;
                }
            }
            Some(QuoteState::Double) => {
                if bytes[index] == b'"' && !byte_is_shell_escaped(bytes, index) {
                    quote_state = None;
                }
            }
            Some(QuoteState::Backtick) => {
                if bytes[index] == b'`' && !byte_is_shell_escaped(bytes, index) {
                    quote_state = None;
                }
            }
            None => match bytes[index] {
                b'\'' if !byte_is_shell_escaped(bytes, index) => {
                    quote_state = Some(QuoteState::Single);
                }
                b'"' if !byte_is_shell_escaped(bytes, index) => {
                    quote_state = Some(QuoteState::Double);
                }
                b'`' if !byte_is_shell_escaped(bytes, index) => {
                    quote_state = Some(QuoteState::Backtick);
                }
                byte if byte == close && !byte_is_shell_escaped(bytes, index) => depth += 1,
                byte if byte == open && !byte_is_shell_escaped(bytes, index) => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            },
        }
    }

    None
}

fn byte_is_shell_escaped(bytes: &[u8], index: usize) -> bool {
    let mut slash_count = 0usize;
    let mut cursor = index;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        slash_count += 1;
        cursor -= 1;
    }

    slash_count % 2 == 1
}

#[derive(Clone, Copy)]
enum QuoteState {
    Single,
    Double,
    Backtick,
}

fn extend_over_shellcheck_trailing_inline_space(end: Position, source: &str) -> Position {
    let tail = &source[end.offset..];
    let spaces_len = tail
        .char_indices()
        .take_while(|(_, ch)| matches!(ch, ' ' | '\t'))
        .last()
        .map_or(0, |(index, ch)| index + ch.len_utf8());

    if spaces_len == 0 {
        return end;
    }

    let rest = &tail[spaces_len..];
    if rest.is_empty()
        || rest.starts_with('\n')
        || rest.starts_with('\r')
        || rest.starts_with(')')
        || rest.starts_with(']')
        || rest.starts_with('}')
    {
        end.advanced_by(&tail[..spaces_len])
    } else {
        end
    }
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

fn build_background_semicolon_spans(
    commands: &[CommandFact<'_>],
    case_items: &[CaseItemFact],
    source: &str,
) -> Vec<Span> {
    let case_terminator_starts = case_items
        .iter()
        .filter_map(CaseItemFact::terminator_span)
        .map(|span| span.start.offset)
        .collect::<FxHashSet<_>>();
    let mut spans = commands
        .iter()
        .filter_map(|command| background_semicolon_span(command, &case_terminator_starts, source))
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

fn background_semicolon_span(
    command: &CommandFact<'_>,
    case_terminator_starts: &FxHashSet<usize>,
    source: &str,
) -> Option<Span> {
    if command.stmt_terminator() != Some(StmtTerminator::Background(BackgroundOperator::Plain)) {
        return None;
    }

    let terminator_span = command.stmt_terminator_span()?;
    if terminator_span.slice(source) != "&" {
        return None;
    }

    let semicolon_offset = source[terminator_span.end.offset..]
        .char_indices()
        .find_map(|(relative, ch)| match ch {
            ' ' | '\t' | '\r' => None,
            '\n' | '#' => Some(None),
            ';' => Some(Some(terminator_span.end.offset + relative)),
            _ => Some(None),
        })??;

    if case_terminator_starts.contains(&semicolon_offset) {
        return None;
    }

    let start = position_at_offset(source, semicolon_offset)?;
    let end = position_at_offset(source, semicolon_offset + 1)?;
    Some(Span::from_positions(start, end))
}

fn populate_scope_fact_ranges<'a>(
    commands: &mut [CommandFact<'a>],
    fact_store: &mut FactStore<'a>,
    pipelines: &[PipelineFact],
    if_condition_command_ids: &FxHashSet<CommandId>,
    arena_file: &ArenaFile,
    source: &str,
) {
    let (pipeline_summaries, pipeline_summary_ids_by_writer) = {
        let command_facts = CommandFacts::new(commands, fact_store, arena_file);
        build_pipeline_scope_summaries(
            command_facts,
            pipelines,
            if_condition_command_ids,
            source,
        )
    };
    let mut source_words = Vec::new();
    let mut name_reads = Vec::new();
    let mut heredoc_name_reads = Vec::new();
    let mut name_writes = Vec::new();

    for index in 0..commands.len() {
        {
            let command_facts = CommandFacts::new(commands, fact_store, arena_file);
            let command = command_facts
                .get(index)
                .expect("command index should resolve while populating scope facts");
            collect_scope_read_source_words_for_command(
                command_facts,
                command,
                &pipeline_summaries,
                &pipeline_summary_ids_by_writer[index],
                if_condition_command_ids,
                source,
                &mut source_words,
            );
            collect_scope_name_read_uses_for_command(
                command_facts,
                command,
                &pipeline_summaries,
                &pipeline_summary_ids_by_writer[index],
                source,
                &mut name_reads,
            );
            collect_scope_heredoc_name_read_uses_for_command(
                command_facts,
                command,
                &pipeline_summaries,
                &pipeline_summary_ids_by_writer[index],
                source,
                &mut heredoc_name_reads,
            );
            collect_scope_name_write_uses_for_command(
                command_facts,
                command,
                source,
                &mut name_writes,
            );
        }

        commands[index].scope_read_source_words =
            fact_store.scope_read_source_words.push_many(source_words.drain(..));
        commands[index].scope_name_read_uses =
            fact_store.scope_name_read_uses.push_many(name_reads.drain(..));
        commands[index].scope_heredoc_name_read_uses = fact_store
            .scope_heredoc_name_read_uses
            .push_many(heredoc_name_reads.drain(..));
        commands[index].scope_name_write_uses =
            fact_store.scope_name_write_uses.push_many(name_writes.drain(..));
    }
}

struct PipelineScopeSummary {
    source_words: Vec<PathWordFact>,
    name_reads: Vec<ComparableNameUse>,
    heredoc_name_reads: Vec<ComparableNameUse>,
}

fn build_pipeline_scope_summaries<'a>(
    commands: CommandFacts<'_, 'a>,
    pipelines: &[PipelineFact],
    if_condition_command_ids: &FxHashSet<CommandId>,
    source: &str,
) -> (Vec<PipelineScopeSummary>, Vec<SmallVec<[usize; 1]>>) {
    let mut summaries = Vec::new();
    let mut summary_ids_by_writer = vec![SmallVec::<[usize; 1]>::new(); commands.len()];

    for pipeline in pipelines {
        let writer_ids = pipeline
            .segments()
            .iter()
            .map(|segment| segment.command_id())
            .filter(|id| {
                commands
                    .get(id.index())
                    .is_some_and(command_has_file_output_redirect)
            })
            .collect::<SmallVec<[_; 4]>>();
        if writer_ids.is_empty() {
            continue;
        }

        let mut source_words = Vec::new();
        let mut name_reads = Vec::new();
        let mut heredoc_name_reads = Vec::new();
        for command in commands
            .iter()
            .filter(|command| contains_span(pipeline.span(), command.span()))
        {
            collect_own_scope_read_source_words(
                command,
                if_condition_command_ids,
                source,
                &mut source_words,
            );
            collect_own_scope_name_read_uses(command, source, &mut name_reads);
            collect_own_scope_heredoc_name_read_uses(command, source, &mut heredoc_name_reads);
        }
        dedup_path_words(&mut source_words);
        dedup_name_uses(&mut name_reads);
        dedup_name_uses(&mut heredoc_name_reads);

        let summary_id = summaries.len();
        summaries.push(PipelineScopeSummary {
            source_words,
            name_reads,
            heredoc_name_reads,
        });
        for writer_id in writer_ids {
            summary_ids_by_writer[writer_id.index()].push(summary_id);
        }
    }

    (summaries, summary_ids_by_writer)
}

fn collect_scope_read_source_words_for_command<'a>(
    commands: CommandFacts<'_, 'a>,
    command: CommandFactRef<'_, 'a>,
    pipeline_summaries: &[PipelineScopeSummary],
    pipeline_summary_ids: &[usize],
    if_condition_command_ids: &FxHashSet<CommandId>,
    source: &str,
    words: &mut Vec<PathWordFact>,
) {
    collect_own_scope_read_source_words(command, if_condition_command_ids, source, words);
    if command_has_file_output_redirect(command) {
        collect_nested_scope_read_source_words(
            commands,
            command,
            if_condition_command_ids,
            source,
            words,
        );
        for summary_id in pipeline_summary_ids {
            words.extend(
                pipeline_summaries[*summary_id]
                    .source_words
                    .iter()
                    .cloned(),
            );
        }
    }
    dedup_path_words(words);
}

fn collect_scope_name_read_uses_for_command(
    commands: CommandFacts<'_, '_>,
    command: CommandFactRef<'_, '_>,
    pipeline_summaries: &[PipelineScopeSummary],
    pipeline_summary_ids: &[usize],
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    collect_own_scope_name_read_uses(command, source, uses);
    if command_has_file_output_redirect(command) {
        collect_nested_scope_name_read_uses(commands, command, source, uses);
        for summary_id in pipeline_summary_ids {
            uses.extend(pipeline_summaries[*summary_id].name_reads.iter().cloned());
        }
    }
    dedup_name_uses(uses);
}

fn collect_scope_heredoc_name_read_uses_for_command(
    commands: CommandFacts<'_, '_>,
    command: CommandFactRef<'_, '_>,
    pipeline_summaries: &[PipelineScopeSummary],
    pipeline_summary_ids: &[usize],
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    collect_own_scope_heredoc_name_read_uses(command, source, uses);
    if command_has_file_output_redirect(command) || command_has_file_input_redirect(command) {
        collect_nested_scope_heredoc_name_read_uses(commands, command, source, uses);
    }
    if command_has_file_output_redirect(command) {
        for summary_id in pipeline_summary_ids {
            uses.extend(
                pipeline_summaries[*summary_id]
                    .heredoc_name_reads
                    .iter()
                    .cloned(),
            );
        }
    }
    dedup_name_uses(uses);
}

fn collect_scope_name_write_uses_for_command(
    commands: CommandFacts<'_, '_>,
    command: CommandFactRef<'_, '_>,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    collect_own_scope_name_write_uses(command, source, uses);
    if command_has_file_input_redirect(command) {
        collect_nested_scope_name_write_uses(commands, command, source, uses);
    }
    dedup_name_uses(uses);
}

fn collect_own_scope_read_source_words<'a>(
    command: CommandFactRef<'_, 'a>,
    if_condition_command_ids: &FxHashSet<CommandId>,
    source: &str,
    words: &mut Vec<PathWordFact>,
) {
    words.extend(command.options().file_operand_path_facts().iter().cloned());
    collect_command_redirect_read_source_words(command, source, words);
    collect_command_simple_test_path_words(command, source, words);
    if !if_condition_command_ids.contains(&command.id()) {
        collect_command_conditional_path_words(command, source, words);
    }
}

fn collect_own_scope_name_read_uses(
    command: CommandFactRef<'_, '_>,
    _source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    for redirect in command.redirect_facts() {
        match redirect.kind() {
            RedirectKind::Input => {
                uses.extend(redirect.comparable_name_uses().iter().cloned());
            }
            RedirectKind::ReadWrite => {}
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => {}
            RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::HereString
            | RedirectKind::DupOutput
            | RedirectKind::DupInput
            | RedirectKind::OutputBoth => {}
        }
    }
}

fn collect_own_scope_heredoc_name_read_uses(
    command: CommandFactRef<'_, '_>,
    _source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    for redirect in command.redirect_facts() {
        uses.extend(redirect.comparable_heredoc_name_uses().iter().cloned());
    }
}

fn collect_own_scope_name_write_uses(
    command: CommandFactRef<'_, '_>,
    _source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    if let Some(read) = command.options().read() {
        uses.extend(read.target_name_uses().iter().cloned());
    }
}

fn collect_nested_scope_read_source_words<'a>(
    commands: CommandFacts<'_, 'a>,
    command: CommandFactRef<'_, 'a>,
    if_condition_command_ids: &FxHashSet<CommandId>,
    source: &str,
    words: &mut Vec<PathWordFact>,
) {
    for other in commands
        .iter()
        .filter(|other| other.id() != command.id() && contains_span(command.span(), other.span()))
    {
        collect_own_scope_read_source_words(other, if_condition_command_ids, source, words);
    }
}

fn collect_nested_scope_name_read_uses(
    commands: CommandFacts<'_, '_>,
    command: CommandFactRef<'_, '_>,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    for other in commands
        .iter()
        .filter(|other| other.id() != command.id() && contains_span(command.span(), other.span()))
    {
        collect_own_scope_name_read_uses(other, source, uses);
    }
}

fn collect_nested_scope_heredoc_name_read_uses(
    commands: CommandFacts<'_, '_>,
    command: CommandFactRef<'_, '_>,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    for other in commands
        .iter()
        .filter(|other| other.id() != command.id() && contains_span(command.span(), other.span()))
    {
        collect_own_scope_heredoc_name_read_uses(other, source, uses);
    }
}

fn collect_nested_scope_name_write_uses(
    commands: CommandFacts<'_, '_>,
    command: CommandFactRef<'_, '_>,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    for other in commands
        .iter()
        .filter(|other| other.id() != command.id() && contains_span(command.span(), other.span()))
    {
        collect_own_scope_name_write_uses(other, source, uses);
    }
}

fn dedup_path_words(words: &mut Vec<PathWordFact>) {
    let mut seen = FxHashSet::<(FactSpan, ExpansionContext)>::default();
    words.retain(|fact| seen.insert((FactSpan::new(fact.span()), fact.context())));
}

fn dedup_name_uses(uses: &mut Vec<ComparableNameUse>) {
    let mut seen = FxHashSet::<(ComparableNameKey, FactSpan)>::default();
    uses.retain(|name_use| seen.insert((name_use.key().clone(), FactSpan::new(name_use.span()))));
}

fn command_has_file_output_redirect(command: CommandFactRef<'_, '_>) -> bool {
    command.redirect_facts().iter().any(|redirect| {
        matches!(
            redirect.kind(),
            RedirectKind::Output
                | RedirectKind::Clobber
                | RedirectKind::Append
                | RedirectKind::OutputBoth
        ) && redirect
            .analysis()
            .is_some_and(|analysis| analysis.is_file_target())
    })
}

fn command_has_file_input_redirect(command: CommandFactRef<'_, '_>) -> bool {
    command.redirect_facts().iter().any(|redirect| {
        matches!(
            redirect.kind(),
            RedirectKind::Input | RedirectKind::ReadWrite
        ) && redirect
            .analysis()
            .is_some_and(|analysis| analysis.is_file_target())
    })
}

fn collect_command_redirect_read_source_words<'a>(
    command: CommandFactRef<'_, 'a>,
    _source: &str,
    words: &mut Vec<PathWordFact>,
) {
    for redirect in command.redirect_facts() {
        if let Some(word) = redirect.read_source_word() {
            words.push(word.clone());
        }
    }
}

fn collect_command_simple_test_path_words<'a>(
    command: CommandFactRef<'_, 'a>,
    source: &str,
    words: &mut Vec<PathWordFact>,
) {
    let Some(simple_test) = command.simple_test() else {
        return;
    };

    words.extend(simple_test
        .operator_expression_operand_words(source)
        .into_iter()
        .map(|word| {
            PathWordFact::new(
                word,
                None,
                ExpansionContext::StringTestOperand,
                source,
                command.zsh_options(),
            )
        }));
}

fn collect_command_conditional_path_words<'a>(
    command: CommandFactRef<'_, 'a>,
    source: &str,
    words: &mut Vec<PathWordFact>,
) {
    if let Some(conditional) = command.conditional() {
        for node in conditional.nodes() {
            match node {
                ConditionalNodeFact::Binary(binary)
                    if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
                {
                    if let Some(word) = binary.left().word() {
                        words.push(PathWordFact::new(
                            word,
                            None,
                            ExpansionContext::StringTestOperand,
                            source,
                            command.zsh_options(),
                        ));
                    }
                    if let Some(word) = binary.right().word() {
                        words.push(PathWordFact::new(
                            word,
                            None,
                            ExpansionContext::StringTestOperand,
                            source,
                            command.zsh_options(),
                        ));
                    }
                }
                ConditionalNodeFact::Binary(_) => {}
                ConditionalNodeFact::BareWord(_) | ConditionalNodeFact::Other(_) => {}
                ConditionalNodeFact::Unary(_) => {}
            }
        }
    }
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn contains_span_strictly(outer: Span, inner: Span) -> bool {
    contains_span(outer, inner)
        && (outer.start.offset < inner.start.offset || inner.end.offset < outer.end.offset)
}

fn build_backtick_command_name_spans(commands: &[CommandFact<'_>], arena_file: &ArenaFile) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter_map(|fact| {
            let command = arena_file.store.command(fact.arena_command_id()?);
            let simple = command.simple()?;
            simple
                .arg_ids()
                .is_empty()
                .then(|| plain_backtick_arena_command_name_span(simple.name()))
                .flatten()
        })
        .collect::<Vec<_>>();

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans
}

fn plain_backtick_arena_command_name_span(word: WordView<'_>) -> Option<Span> {
    let [part] = word.parts() else {
        return None;
    };

    matches!(
        part.kind,
        WordPartArena::CommandSubstitution {
            syntax: CommandSubstitutionSyntax::Backtick,
            ..
        }
    )
    .then_some(part.span)
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => builtin_span(command),
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(command) => compound_span(command),
        Command::Function(command) => command.span,
        Command::AnonymousFunction(command) => command.span,
    }
}

fn command_lookup_kind(command: &Command) -> CommandLookupKind {
    match command {
        Command::Simple(_) => CommandLookupKind::Simple,
        Command::Builtin(command) => CommandLookupKind::Builtin(match command {
            BuiltinCommand::Break(_) => BuiltinLookupKind::Break,
            BuiltinCommand::Continue(_) => BuiltinLookupKind::Continue,
            BuiltinCommand::Return(_) => BuiltinLookupKind::Return,
            BuiltinCommand::Exit(_) => BuiltinLookupKind::Exit,
        }),
        Command::Decl(_) => CommandLookupKind::Decl,
        Command::Binary(_) => CommandLookupKind::Binary,
        Command::Compound(command) => CommandLookupKind::Compound(match command {
            CompoundCommand::If(_) => CompoundLookupKind::If,
            CompoundCommand::For(_) => CompoundLookupKind::For,
            CompoundCommand::Repeat(_) => CompoundLookupKind::Repeat,
            CompoundCommand::Foreach(_) => CompoundLookupKind::Foreach,
            CompoundCommand::ArithmeticFor(_) => CompoundLookupKind::ArithmeticFor,
            CompoundCommand::While(_) => CompoundLookupKind::While,
            CompoundCommand::Until(_) => CompoundLookupKind::Until,
            CompoundCommand::Case(_) => CompoundLookupKind::Case,
            CompoundCommand::Select(_) => CompoundLookupKind::Select,
            CompoundCommand::Subshell(_) => CompoundLookupKind::Subshell,
            CompoundCommand::BraceGroup(_) => CompoundLookupKind::BraceGroup,
            CompoundCommand::Arithmetic(_) => CompoundLookupKind::Arithmetic,
            CompoundCommand::Time(_) => CompoundLookupKind::Time,
            CompoundCommand::Conditional(_) => CompoundLookupKind::Conditional,
            CompoundCommand::Coproc(_) => CompoundLookupKind::Coproc,
            CompoundCommand::Always(_) => CompoundLookupKind::Always,
        }),
        Command::Function(_) => CommandLookupKind::Function,
        Command::AnonymousFunction(_) => CommandLookupKind::AnonymousFunction,
    }
}

fn command_id_for_command(
    command: &Command,
    command_ids_by_span: &CommandLookupIndex,
) -> Option<CommandId> {
    command_ids_by_span
        .get(&FactSpan::new(command_span(command)))
        .and_then(|entries| {
            let kind = command_lookup_kind(command);
            entries
                .iter()
                .find(|entry| entry.kind == kind)
                .map(|entry| entry.id)
        })
}

fn command_fact<'a>(commands: &'a [CommandFact<'a>], id: CommandId) -> &'a CommandFact<'a> {
    &commands[id.index()]
}

fn command_fact_ref<'facts, 'a>(
    commands: CommandFacts<'facts, 'a>,
    id: CommandId,
) -> CommandFactRef<'facts, 'a> {
    commands
        .get(id.index())
        .unwrap_or_else(|| panic!("command id {} must exist", id.index()))
}

#[derive(Clone, Copy)]
struct CommandRelationshipContext<'facts, 'a> {
    commands: &'facts [CommandFact<'a>],
    command_ids_by_span: &'facts CommandLookupIndex,
    command_child_index: &'facts CommandChildIndex,
}

impl<'facts, 'a> CommandRelationshipContext<'facts, 'a> {
    fn new(
        commands: &'facts [CommandFact<'a>],
        command_ids_by_span: &'facts CommandLookupIndex,
        command_child_index: &'facts CommandChildIndex,
    ) -> Self {
        Self {
            commands,
            command_ids_by_span,
            command_child_index,
        }
    }

    fn fact(self, id: CommandId) -> &'facts CommandFact<'a> {
        &self.commands[id.index()]
    }

    fn id_for_command(self, command: &Command) -> Option<CommandId> {
        command_id_for_command(command, self.command_ids_by_span)
    }

    fn fact_for_command(self, command: &Command) -> Option<&'facts CommandFact<'a>> {
        self.id_for_command(command).map(|id| self.fact(id))
    }

    fn fact_for_stmt(self, stmt: &Stmt) -> Option<&'facts CommandFact<'a>> {
        self.fact_for_command(&stmt.command)
    }

    fn fact_for_arena_stmt(self, stmt: StmtView<'_>) -> Option<&'facts CommandFact<'a>> {
        let id = stmt.command().id();
        self.commands
            .iter()
            .find(|fact| fact.arena_command_id().is_some_and(|candidate| candidate.index() == id.index()))
    }

    fn child_id_for_command(self, parent_id: CommandId, command: &Command) -> Option<CommandId> {
        child_command_id_for_command(parent_id, command, self.commands, self.command_child_index)
    }

    fn child_fact_for_stmt(
        self,
        parent_id: CommandId,
        stmt: &Stmt,
    ) -> Option<&'facts CommandFact<'a>> {
        self.child_id_for_command(parent_id, &stmt.command)
            .map(|id| self.fact(id))
    }

    fn child_or_lookup_fact(
        self,
        parent_id: CommandId,
        stmt: &Stmt,
    ) -> Option<&'facts CommandFact<'a>> {
        self.child_fact_for_stmt(parent_id, stmt)
            .or_else(|| self.fact_for_stmt(stmt))
    }

    fn child_or_lookup_arena_fact(
        self,
        parent_id: CommandId,
        stmt: StmtView<'_>,
    ) -> Option<&'facts CommandFact<'a>> {
        let child_id = stmt.command().id();
        self.command_child_index
            .child_ids(parent_id)
            .iter()
            .copied()
            .find(|id| {
                self.fact(*id)
                    .arena_command_id()
                    .is_some_and(|candidate| candidate.index() == child_id.index())
            })
            .map(|id| self.fact(id))
            .or_else(|| self.fact_for_arena_stmt(stmt))
    }

}

fn build_command_parent_ids(
    commands: &[CommandFact<'_>],
    require_source_order: bool,
) -> Vec<Option<CommandId>> {
    let mut parent_ids = vec![None; commands.len()];
    let mut active_commands = Vec::<OpenParentCommand>::new();

    if !require_source_order {
        for command in commands {
            assign_command_parent(
                command.span(),
                command.id(),
                &mut active_commands,
                &mut parent_ids,
            );
        }
    } else {
        let mut command_spans = commands
            .iter()
            .map(|command| (command.span(), command.id()))
            .collect::<Vec<_>>();
        command_spans
            .sort_unstable_by(|left, right| compare_command_parent_entries(*left, *right));

        for (span, id) in command_spans {
            assign_command_parent(span, id, &mut active_commands, &mut parent_ids);
        }
    }

    parent_ids
}

fn assign_command_parent(
    span: Span,
    id: CommandId,
    active_commands: &mut Vec<OpenParentCommand>,
    parent_ids: &mut [Option<CommandId>],
) {
    while active_commands
        .last()
        .is_some_and(|candidate| candidate.end_offset < span.end.offset)
    {
        active_commands.pop();
    }

    parent_ids[id.index()] = active_commands.last().map(|command| command.id);
    active_commands.push(OpenParentCommand {
        id,
        end_offset: span.end.offset,
    });
}

fn command_facts_are_source_ordered(commands: &[CommandFact<'_>]) -> bool {
    commands
        .windows(2)
        .all(|window| compare_command_facts_by_offset(&window[0], &window[1]).is_le())
}

fn compare_command_facts_by_offset(
    left: &CommandFact<'_>,
    right: &CommandFact<'_>,
) -> std::cmp::Ordering {
    compare_command_parent_entries((left.span(), left.id()), (right.span(), right.id()))
}

fn compare_command_parent_entries(
    (left_span, left_id): (Span, CommandId),
    (right_span, right_id): (Span, CommandId),
) -> std::cmp::Ordering {
    left_span
        .start
        .offset
        .cmp(&right_span.start.offset)
        .then_with(|| right_span.end.offset.cmp(&left_span.end.offset))
        .then_with(|| right_id.index().cmp(&left_id.index()))
}

fn build_command_dominance_barrier_flags(commands: &[CommandFact<'_>]) -> Vec<bool> {
    commands
        .iter()
        .map(|fact| match fact.shape().kind {
            ArenaFileCommandKind::Binary => true,
            ArenaFileCommandKind::Compound => !matches!(
                fact.shape().compound_kind,
                Some(
                    CommandFactCompoundKind::BraceGroup
                        | CommandFactCompoundKind::Arithmetic
                        | CommandFactCompoundKind::Time
                )
            ),
            ArenaFileCommandKind::Simple
            | ArenaFileCommandKind::Builtin
            | ArenaFileCommandKind::Decl
            | ArenaFileCommandKind::Function
            | ArenaFileCommandKind::AnonymousFunction => false,
        })
        .collect()
}

fn sort_and_dedup_spans(spans: &mut Vec<Span>) {
    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
}

#[derive(Debug, Clone, Copy)]
struct OpenParentCommand {
    id: CommandId,
    end_offset: usize,
}

fn trim_trailing_whitespace_span(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let trimmed = text.trim_end_matches(char::is_whitespace);
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}

fn command_fact_for_command<'a>(
    command: &Command,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<&'a CommandFact<'a>> {
    command_id_for_command(command, command_ids_by_span).map(|id| command_fact(commands, id))
}

fn command_fact_for_stmt<'a>(
    stmt: &Stmt,
    commands: &'a [CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<&'a CommandFact<'a>> {
    command_fact_for_command(&stmt.command, commands, command_ids_by_span)
}

fn child_command_id_for_command(
    parent_id: CommandId,
    command: &Command,
    commands: &[CommandFact<'_>],
    command_child_index: &CommandChildIndex,
) -> Option<CommandId> {
    command_child_index
        .child_ids(parent_id)
        .iter()
        .copied()
        .find(|id| command_fact(commands, *id).span() == command_span(command))
}

fn command_fact_ref_for_stmt<'facts, 'a>(
    stmt: &Stmt,
    commands: CommandFacts<'facts, 'a>,
    command_ids_by_span: &CommandLookupIndex,
) -> Option<CommandFactRef<'facts, 'a>> {
    command_id_for_command(&stmt.command, command_ids_by_span).map(|id| command_fact_ref(commands, id))
}

fn builtin_span(command: &BuiltinCommand) -> Span {
    match command {
        BuiltinCommand::Break(command) => command.span,
        BuiltinCommand::Continue(command) => command.span,
        BuiltinCommand::Return(command) => command.span,
        BuiltinCommand::Exit(command) => command.span,
    }
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            commands.span
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}
