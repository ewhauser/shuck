use crate::{
    AlwaysCommand, AnonymousFunctionCommand, Assignment, AssignmentValue, BinaryCommand,
    BuiltinCommand, CaseCommand, Command, Comment, CompoundCommand, CoprocCommand, DeclOperand,
    File, ForCommand, FunctionDef, HeredocBodyPart, IdRange, Idx, ListArena, Pattern, PatternPart,
    Redirect, RedirectTarget, RepeatCommand, SelectCommand, Span, Stmt, StmtSeq, StmtTerminator,
    TimeCommand, UntilCommand, WhileCommand, Word, WordPart, WordPartNode,
};

/// Stable typed identifier for a parsed file node inside an [`AstStore`].
pub type FileId = Idx<FileNode>;
/// Stable typed identifier for a statement sequence node inside an [`AstStore`].
pub type StmtSeqId = Idx<StmtSeqNode>;
/// Stable typed identifier for a statement node inside an [`AstStore`].
pub type StmtId = Idx<StmtNode>;
/// Stable typed identifier for a command node inside an [`AstStore`].
pub type CommandId = Idx<CommandNode>;
/// Stable typed identifier for a word node inside an [`AstStore`].
pub type WordId = Idx<WordNode>;

/// An ID-backed parsed file representation.
#[derive(Debug, Clone)]
pub struct ArenaFile {
    /// Root file node.
    pub root: FileId,
    /// Arena storage for the parsed file.
    pub store: AstStore,
}

impl ArenaFile {
    /// Builds an arena representation from the existing recursive AST.
    pub fn from_file(file: &File) -> Self {
        let mut builder = AstStoreBuilder::default();
        let root = builder.lower_file(file);
        Self {
            root,
            store: builder.finish(),
        }
    }

    /// Returns a borrowed view of the root file node.
    pub fn view(&self) -> FileView<'_> {
        self.store.file(self.root)
    }

    /// Materializes the arena representation back into the legacy recursive AST.
    pub fn to_file(&self) -> File {
        self.store.materialize_file(self.root)
    }
}

/// Compact typed AST storage for one parsed file.
#[derive(Debug, Clone)]
pub struct AstStore {
    files: Vec<FileNode>,
    stmt_seqs: Vec<StmtSeqNode>,
    stmts: Vec<StmtNode>,
    commands: Vec<CommandNode>,
    words: Vec<WordNode>,
    stmt_id_lists: ListArena<StmtId>,
    stmt_seq_id_lists: ListArena<StmtSeqId>,
    comment_lists: ListArena<Comment>,
    redirect_lists: ListArena<Redirect>,
    word_id_lists: ListArena<WordId>,
    word_part_lists: ListArena<WordPartNode>,
    brace_syntax_lists: ListArena<crate::BraceSyntax>,
}

impl Default for AstStore {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            stmt_seqs: Vec::new(),
            stmts: Vec::new(),
            commands: Vec::new(),
            words: Vec::new(),
            stmt_id_lists: ListArena::new(),
            stmt_seq_id_lists: ListArena::new(),
            comment_lists: ListArena::new(),
            redirect_lists: ListArena::new(),
            word_id_lists: ListArena::new(),
            word_part_lists: ListArena::new(),
            brace_syntax_lists: ListArena::new(),
        }
    }
}

impl AstStore {
    /// Returns the file view for `id`.
    pub fn file(&self, id: FileId) -> FileView<'_> {
        FileView { store: self, id }
    }

    /// Returns the statement sequence view for `id`.
    pub fn stmt_seq(&self, id: StmtSeqId) -> StmtSeqView<'_> {
        StmtSeqView { store: self, id }
    }

    /// Returns the statement view for `id`.
    pub fn stmt(&self, id: StmtId) -> StmtView<'_> {
        StmtView { store: self, id }
    }

    /// Returns the command view for `id`.
    pub fn command(&self, id: CommandId) -> CommandView<'_> {
        CommandView { store: self, id }
    }

    /// Returns the word view for `id`.
    pub fn word(&self, id: WordId) -> WordView<'_> {
        WordView { store: self, id }
    }

    /// Number of file nodes in this store.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Number of statement sequence nodes in this store.
    pub fn stmt_seq_count(&self) -> usize {
        self.stmt_seqs.len()
    }

    /// Number of statement nodes in this store.
    pub fn stmt_count(&self) -> usize {
        self.stmts.len()
    }

    /// Number of command nodes in this store.
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// Number of word nodes in this store.
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    fn materialize_file(&self, id: FileId) -> File {
        let node = &self.files[id.index()];
        File {
            body: self.materialize_stmt_seq(node.body),
            span: node.span,
        }
    }

    fn materialize_stmt_seq(&self, id: StmtSeqId) -> StmtSeq {
        let node = &self.stmt_seqs[id.index()];
        StmtSeq {
            leading_comments: self.comment_lists.get(node.leading_comments).to_vec(),
            stmts: self
                .stmt_id_lists
                .get(node.stmts)
                .iter()
                .copied()
                .map(|stmt| self.materialize_stmt(stmt))
                .collect(),
            trailing_comments: self.comment_lists.get(node.trailing_comments).to_vec(),
            span: node.span,
        }
    }

    fn materialize_stmt(&self, id: StmtId) -> Stmt {
        let node = &self.stmts[id.index()];
        Stmt {
            leading_comments: self.comment_lists.get(node.leading_comments).to_vec(),
            command: self.commands[node.command.index()].legacy.clone(),
            negated: node.negated,
            redirects: self
                .redirect_lists
                .get(node.redirects)
                .to_vec()
                .into_boxed_slice(),
            terminator: node.terminator,
            terminator_span: node.terminator_span,
            inline_comment: node.inline_comment,
            span: node.span,
        }
    }
}

/// File-level arena node.
#[derive(Debug, Clone)]
pub struct FileNode {
    /// Root statement sequence.
    pub body: StmtSeqId,
    /// Source span of the file.
    pub span: Span,
}

/// Statement-sequence arena node.
#[derive(Debug, Clone)]
pub struct StmtSeqNode {
    /// Comments before the first statement in this sequence.
    pub leading_comments: IdRange<Comment>,
    /// Statements in source order.
    pub stmts: IdRange<StmtId>,
    /// Comments after the final statement and before the enclosing terminator.
    pub trailing_comments: IdRange<Comment>,
    /// Source span covering the full sequence.
    pub span: Span,
}

/// Statement arena node.
#[derive(Debug, Clone)]
pub struct StmtNode {
    /// Own-line comments immediately preceding this statement.
    pub leading_comments: IdRange<Comment>,
    /// Statement command payload.
    pub command: CommandId,
    /// Whether this statement was prefixed with `!`.
    pub negated: bool,
    /// Statement redirections.
    pub redirects: IdRange<Redirect>,
    /// Optional statement terminator.
    pub terminator: Option<StmtTerminator>,
    /// Source span of the terminator token when present.
    pub terminator_span: Option<Span>,
    /// Trailing inline comment on the statement line.
    pub inline_comment: Option<Comment>,
    /// Source span of the full statement.
    pub span: Span,
}

/// Command arena node.
#[derive(Debug, Clone)]
pub struct CommandNode {
    /// Coarse command kind for cheap filtering.
    pub kind: ArenaFileCommandKind,
    /// Source span of the command.
    pub span: Span,
    /// Words found under this command.
    pub words: IdRange<WordId>,
    /// Nested statement sequences found under this command.
    pub child_sequences: IdRange<StmtSeqId>,
    legacy: Command,
}

/// Coarse command family stored with command arena nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArenaFileCommandKind {
    /// Simple command.
    Simple,
    /// Builtin command with a dedicated node.
    Builtin,
    /// Declaration builtin clause.
    Decl,
    /// Binary shell operator.
    Binary,
    /// Compound command.
    Compound,
    /// Function definition.
    Function,
    /// Anonymous zsh function.
    AnonymousFunction,
}

/// Word arena node.
#[derive(Debug, Clone)]
pub struct WordNode {
    /// Word parts in source order.
    pub parts: IdRange<WordPartNode>,
    /// Source span of this word.
    pub span: Span,
    /// Brace-syntax facts attached by the parser.
    pub brace_syntax: IdRange<crate::BraceSyntax>,
}

/// Borrowed view of a file node.
#[derive(Debug, Clone, Copy)]
pub struct FileView<'a> {
    store: &'a AstStore,
    id: FileId,
}

impl<'a> FileView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> FileId {
        self.id
    }

    /// Returns the root statement sequence.
    pub fn body(self) -> StmtSeqView<'a> {
        self.store.stmt_seq(self.store.files[self.id.index()].body)
    }

    /// Returns the file source span.
    pub fn span(self) -> Span {
        self.store.files[self.id.index()].span
    }
}

/// Borrowed view of a statement sequence node.
#[derive(Debug, Clone, Copy)]
pub struct StmtSeqView<'a> {
    store: &'a AstStore,
    id: StmtSeqId,
}

impl<'a> StmtSeqView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> StmtSeqId {
        self.id
    }

    /// Returns the raw statement IDs in this sequence.
    pub fn stmt_ids(self) -> &'a [StmtId] {
        self.store.stmt_id_lists.get(self.node().stmts)
    }

    /// Returns the statements in this sequence.
    pub fn stmts(self) -> impl ExactSizeIterator<Item = StmtView<'a>> + 'a {
        self.stmt_ids()
            .iter()
            .copied()
            .map(move |id| self.store.stmt(id))
    }

    /// Returns leading comments for this sequence.
    pub fn leading_comments(self) -> &'a [Comment] {
        self.store.comment_lists.get(self.node().leading_comments)
    }

    /// Returns trailing comments for this sequence.
    pub fn trailing_comments(self) -> &'a [Comment] {
        self.store.comment_lists.get(self.node().trailing_comments)
    }

    /// Returns this sequence's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    fn node(self) -> &'a StmtSeqNode {
        &self.store.stmt_seqs[self.id.index()]
    }
}

/// Borrowed view of a statement node.
#[derive(Debug, Clone, Copy)]
pub struct StmtView<'a> {
    store: &'a AstStore,
    id: StmtId,
}

impl<'a> StmtView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> StmtId {
        self.id
    }

    /// Returns this statement's command.
    pub fn command(self) -> CommandView<'a> {
        self.store.command(self.node().command)
    }

    /// Returns comments immediately preceding this statement.
    pub fn leading_comments(self) -> &'a [Comment] {
        self.store.comment_lists.get(self.node().leading_comments)
    }

    /// Returns this statement's redirections.
    pub fn redirects(self) -> &'a [Redirect] {
        self.store.redirect_lists.get(self.node().redirects)
    }

    /// Returns whether this statement is negated.
    pub fn negated(self) -> bool {
        self.node().negated
    }

    /// Returns this statement's terminator.
    pub fn terminator(self) -> Option<StmtTerminator> {
        self.node().terminator
    }

    /// Returns this statement's inline comment.
    pub fn inline_comment(self) -> Option<Comment> {
        self.node().inline_comment
    }

    /// Returns this statement's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    fn node(self) -> &'a StmtNode {
        &self.store.stmts[self.id.index()]
    }
}

/// Borrowed view of a command node.
#[derive(Debug, Clone, Copy)]
pub struct CommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> CommandView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> CommandId {
        self.id
    }

    /// Returns this command's coarse kind.
    pub fn kind(self) -> ArenaFileCommandKind {
        self.node().kind
    }

    /// Returns this command's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    /// Returns IDs for words found under this command.
    pub fn word_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().words)
    }

    /// Returns words found under this command.
    pub fn words(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.word_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    /// Returns IDs for nested statement sequences found under this command.
    pub fn child_sequence_ids(self) -> &'a [StmtSeqId] {
        self.store
            .stmt_seq_id_lists
            .get(self.node().child_sequences)
    }

    /// Returns the legacy recursive command payload.
    pub fn legacy(self) -> &'a Command {
        &self.node().legacy
    }

    fn node(self) -> &'a CommandNode {
        &self.store.commands[self.id.index()]
    }
}

/// Borrowed view of a word node.
#[derive(Debug, Clone, Copy)]
pub struct WordView<'a> {
    store: &'a AstStore,
    id: WordId,
}

impl<'a> WordView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> WordId {
        self.id
    }

    /// Returns this word's parts.
    pub fn parts(self) -> &'a [WordPartNode] {
        self.store.word_part_lists.get(self.node().parts)
    }

    /// Returns this word's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    /// Returns this word's brace-syntax facts.
    pub fn brace_syntax(self) -> &'a [crate::BraceSyntax] {
        self.store.brace_syntax_lists.get(self.node().brace_syntax)
    }

    fn node(self) -> &'a WordNode {
        &self.store.words[self.id.index()]
    }
}

#[derive(Default)]
struct AstStoreBuilder {
    store: AstStore,
}

impl AstStoreBuilder {
    fn finish(self) -> AstStore {
        self.store
    }

    fn lower_file(&mut self, file: &File) -> FileId {
        let body = self.lower_stmt_seq(&file.body);
        let id = Idx::new(self.store.files.len());
        self.store.files.push(FileNode {
            body,
            span: file.span,
        });
        id
    }

    fn lower_stmt_seq(&mut self, sequence: &StmtSeq) -> StmtSeqId {
        let leading_comments = self
            .store
            .comment_lists
            .push_many(sequence.leading_comments.iter().copied());
        let stmts = sequence
            .stmts
            .iter()
            .map(|stmt| self.lower_stmt(stmt))
            .collect::<Vec<_>>();
        let stmts = self.store.stmt_id_lists.push_many(stmts);
        let trailing_comments = self
            .store
            .comment_lists
            .push_many(sequence.trailing_comments.iter().copied());

        let id = Idx::new(self.store.stmt_seqs.len());
        self.store.stmt_seqs.push(StmtSeqNode {
            leading_comments,
            stmts,
            trailing_comments,
            span: sequence.span,
        });
        id
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> StmtId {
        let leading_comments = self
            .store
            .comment_lists
            .push_many(stmt.leading_comments.iter().copied());
        let command = self.lower_command(&stmt.command);
        for redirect in stmt.redirects.iter() {
            self.collect_redirect_words(redirect);
        }
        let redirects = self
            .store
            .redirect_lists
            .push_many(stmt.redirects.iter().cloned());

        let id = Idx::new(self.store.stmts.len());
        self.store.stmts.push(StmtNode {
            leading_comments,
            command,
            negated: stmt.negated,
            redirects,
            terminator: stmt.terminator,
            terminator_span: stmt.terminator_span,
            inline_comment: stmt.inline_comment,
            span: stmt.span,
        });
        id
    }

    fn lower_command(&mut self, command: &Command) -> CommandId {
        let mut words = Vec::new();
        let mut child_sequences = Vec::new();
        self.collect_command_children(command, &mut words, &mut child_sequences);

        let words = self.store.word_id_lists.push_many(words);
        let child_sequences = self.store.stmt_seq_id_lists.push_many(child_sequences);
        let id = Idx::new(self.store.commands.len());
        self.store.commands.push(CommandNode {
            kind: command_kind(command),
            span: command_span(command),
            words,
            child_sequences,
            legacy: command.clone(),
        });
        id
    }

    fn lower_word(&mut self, word: &Word) -> WordId {
        self.collect_word_part_children(word.parts.as_slice());
        let parts = self
            .store
            .word_part_lists
            .push_many(word.parts.iter().cloned());
        let brace_syntax = self
            .store
            .brace_syntax_lists
            .push_many(word.brace_syntax.iter().copied());
        let id = Idx::new(self.store.words.len());
        self.store.words.push(WordNode {
            parts,
            span: word.span,
            brace_syntax,
        });
        id
    }

    fn collect_command_children(
        &mut self,
        command: &Command,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match command {
            Command::Simple(command) => {
                words.push(self.lower_word(&command.name));
                words.extend(command.args.iter().map(|word| self.lower_word(word)));
                self.collect_assignments(command.assignments.iter(), words);
            }
            Command::Builtin(command) => self.collect_builtin_children(command, words),
            Command::Decl(command) => {
                for operand in &command.operands {
                    self.collect_decl_operand(operand, words);
                }
                self.collect_assignments(command.assignments.iter(), words);
            }
            Command::Binary(command) => self.collect_binary_children(command, child_sequences),
            Command::Compound(command) => {
                self.collect_compound_children(command, words, child_sequences)
            }
            Command::Function(function) => {
                self.collect_function_children(function, words, child_sequences)
            }
            Command::AnonymousFunction(function) => {
                self.collect_anonymous_function_children(function, words, child_sequences);
            }
        }
    }

    fn collect_builtin_children(&mut self, command: &BuiltinCommand, words: &mut Vec<WordId>) {
        match command {
            BuiltinCommand::Break(command) => {
                words.extend(command.depth.iter().map(|word| self.lower_word(word)));
                words.extend(command.extra_args.iter().map(|word| self.lower_word(word)));
                self.collect_assignments(command.assignments.iter(), words);
            }
            BuiltinCommand::Continue(command) => {
                words.extend(command.depth.iter().map(|word| self.lower_word(word)));
                words.extend(command.extra_args.iter().map(|word| self.lower_word(word)));
                self.collect_assignments(command.assignments.iter(), words);
            }
            BuiltinCommand::Return(command) => {
                words.extend(command.code.iter().map(|word| self.lower_word(word)));
                words.extend(command.extra_args.iter().map(|word| self.lower_word(word)));
                self.collect_assignments(command.assignments.iter(), words);
            }
            BuiltinCommand::Exit(command) => {
                words.extend(command.code.iter().map(|word| self.lower_word(word)));
                words.extend(command.extra_args.iter().map(|word| self.lower_word(word)));
                self.collect_assignments(command.assignments.iter(), words);
            }
        }
    }

    fn collect_binary_children(
        &mut self,
        command: &BinaryCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        child_sequences.push(self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*command.left).clone()],
            trailing_comments: Vec::new(),
            span: command.left.span,
        }));
        child_sequences.push(self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*command.right).clone()],
            trailing_comments: Vec::new(),
            span: command.right.span,
        }));
    }

    fn collect_compound_children(
        &mut self,
        command: &CompoundCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match command {
            CompoundCommand::If(command) => {
                child_sequences.push(self.lower_stmt_seq(&command.condition));
                child_sequences.push(self.lower_stmt_seq(&command.then_branch));
                for (condition, body) in &command.elif_branches {
                    child_sequences.push(self.lower_stmt_seq(condition));
                    child_sequences.push(self.lower_stmt_seq(body));
                }
                if let Some(else_branch) = &command.else_branch {
                    child_sequences.push(self.lower_stmt_seq(else_branch));
                }
            }
            CompoundCommand::For(command) => {
                self.collect_for_children(command, words, child_sequences)
            }
            CompoundCommand::Repeat(command) => {
                self.collect_repeat_children(command, words, child_sequences);
            }
            CompoundCommand::Foreach(command) => {
                words.extend(command.words.iter().map(|word| self.lower_word(word)));
                child_sequences.push(self.lower_stmt_seq(&command.body));
            }
            CompoundCommand::ArithmeticFor(command) => {
                child_sequences.push(self.lower_stmt_seq(&command.body));
            }
            CompoundCommand::While(command) => {
                self.collect_while_children(command, child_sequences)
            }
            CompoundCommand::Until(command) => {
                self.collect_until_children(command, child_sequences)
            }
            CompoundCommand::Case(command) => {
                self.collect_case_children(command, words, child_sequences)
            }
            CompoundCommand::Select(command) => {
                self.collect_select_children(command, words, child_sequences);
            }
            CompoundCommand::Subshell(sequence) | CompoundCommand::BraceGroup(sequence) => {
                child_sequences.push(self.lower_stmt_seq(sequence));
            }
            CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
            CompoundCommand::Time(command) => self.collect_time_children(command, child_sequences),
            CompoundCommand::Coproc(command) => {
                self.collect_coproc_children(command, child_sequences)
            }
            CompoundCommand::Always(command) => {
                self.collect_always_children(command, child_sequences)
            }
        }
    }

    fn collect_for_children(
        &mut self,
        command: &ForCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        words.extend(
            command
                .targets
                .iter()
                .map(|target| self.lower_word(&target.word)),
        );
        if let Some(header_words) = &command.words {
            words.extend(header_words.iter().map(|word| self.lower_word(word)));
        }
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_repeat_children(
        &mut self,
        command: &RepeatCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        words.push(self.lower_word(&command.count));
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_while_children(
        &mut self,
        command: &WhileCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        child_sequences.push(self.lower_stmt_seq(&command.condition));
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_until_children(
        &mut self,
        command: &UntilCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        child_sequences.push(self.lower_stmt_seq(&command.condition));
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_case_children(
        &mut self,
        command: &CaseCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        words.push(self.lower_word(&command.word));
        for case in &command.cases {
            for pattern in &case.patterns {
                self.collect_pattern_words(pattern, words);
            }
            child_sequences.push(self.lower_stmt_seq(&case.body));
        }
    }

    fn collect_select_children(
        &mut self,
        command: &SelectCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        words.extend(command.words.iter().map(|word| self.lower_word(word)));
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_time_children(
        &mut self,
        command: &TimeCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        if let Some(command) = &command.command {
            child_sequences.push(self.lower_stmt_seq(&StmtSeq {
                leading_comments: Vec::new(),
                stmts: vec![(**command).clone()],
                trailing_comments: Vec::new(),
                span: command.span,
            }));
        }
    }

    fn collect_coproc_children(
        &mut self,
        command: &CoprocCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        child_sequences.push(self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*command.body).clone()],
            trailing_comments: Vec::new(),
            span: command.body.span,
        }));
    }

    fn collect_always_children(
        &mut self,
        command: &AlwaysCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        child_sequences.push(self.lower_stmt_seq(&command.body));
        child_sequences.push(self.lower_stmt_seq(&command.always_body));
    }

    fn collect_function_children(
        &mut self,
        function: &FunctionDef,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        words.extend(
            function
                .header
                .entries
                .iter()
                .map(|entry| self.lower_word(&entry.word)),
        );
        child_sequences.push(self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*function.body).clone()],
            trailing_comments: Vec::new(),
            span: function.body.span,
        }));
    }

    fn collect_anonymous_function_children(
        &mut self,
        function: &AnonymousFunctionCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        child_sequences.push(self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*function.body).clone()],
            trailing_comments: Vec::new(),
            span: function.body.span,
        }));
        words.extend(function.args.iter().map(|word| self.lower_word(word)));
    }

    fn collect_decl_operand(&mut self, operand: &DeclOperand, words: &mut Vec<WordId>) {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                words.push(self.lower_word(word));
            }
            DeclOperand::Assignment(assignment) => self.collect_assignment(assignment, words),
            DeclOperand::Name(_) => {}
        }
    }

    fn collect_assignments<'a>(
        &mut self,
        assignments: impl Iterator<Item = &'a Assignment>,
        words: &mut Vec<WordId>,
    ) {
        for assignment in assignments {
            self.collect_assignment(assignment, words);
        }
    }

    fn collect_assignment(&mut self, assignment: &Assignment, words: &mut Vec<WordId>) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => words.push(self.lower_word(word)),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        crate::ArrayElem::Sequential(value)
                        | crate::ArrayElem::Keyed { value, .. }
                        | crate::ArrayElem::KeyedAppend { value, .. } => {
                            words.push(self.lower_word(&value.word));
                        }
                    }
                }
            }
        }
    }

    fn collect_redirect_words(&mut self, redirect: &Redirect) {
        match &redirect.target {
            RedirectTarget::Word(word) => {
                self.lower_word(word);
            }
            RedirectTarget::Heredoc(heredoc) => {
                self.lower_word(&heredoc.delimiter.raw);
                for part in &heredoc.body.parts {
                    match &part.kind {
                        HeredocBodyPart::CommandSubstitution { body, .. } => {
                            self.lower_stmt_seq(body);
                        }
                        HeredocBodyPart::ArithmeticExpansion {
                            expression_word_ast,
                            ..
                        } => {
                            self.lower_word(expression_word_ast);
                        }
                        HeredocBodyPart::Parameter(expansion) => {
                            self.collect_parameter_expansion_words(expansion);
                        }
                        HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
                    }
                }
            }
        }
    }

    fn collect_word_part_children(&mut self, parts: &[WordPartNode]) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => self.collect_word_part_children(parts),
                WordPart::CommandSubstitution { body, .. }
                | WordPart::ProcessSubstitution { body, .. } => {
                    self.lower_stmt_seq(body);
                }
                WordPart::ArithmeticExpansion {
                    expression_word_ast,
                    ..
                } => {
                    self.lower_word(expression_word_ast);
                }
                WordPart::Parameter(expansion) => self.collect_parameter_expansion_words(expansion),
                WordPart::ParameterExpansion {
                    operand_word_ast, ..
                }
                | WordPart::IndirectExpansion {
                    operand_word_ast, ..
                } => {
                    if let Some(word) = operand_word_ast {
                        self.lower_word(word);
                    }
                }
                WordPart::Substring {
                    offset_word_ast,
                    length_word_ast,
                    ..
                }
                | WordPart::ArraySlice {
                    offset_word_ast,
                    length_word_ast,
                    ..
                } => {
                    self.lower_word(offset_word_ast);
                    if let Some(word) = length_word_ast {
                        self.lower_word(word);
                    }
                }
                WordPart::ZshQualifiedGlob(glob) => {
                    for segment in &glob.segments {
                        if let crate::ZshGlobSegment::Pattern(pattern) = segment {
                            self.collect_pattern_words(pattern, &mut Vec::new());
                        }
                    }
                }
                WordPart::Literal(_)
                | WordPart::SingleQuoted { .. }
                | WordPart::Variable(_)
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::PrefixMatch { .. }
                | WordPart::Transformation { .. } => {}
            }
        }
    }

    fn collect_parameter_expansion_words(&mut self, expansion: &crate::ParameterExpansion) {
        match &expansion.syntax {
            crate::ParameterExpansionSyntax::Bourne(expansion) => match expansion {
                crate::BourneParameterExpansion::Indirect {
                    operand_word_ast, ..
                }
                | crate::BourneParameterExpansion::Operation {
                    operand_word_ast, ..
                } => {
                    if let Some(word) = operand_word_ast {
                        self.lower_word(word);
                    }
                }
                crate::BourneParameterExpansion::Slice {
                    offset_word_ast,
                    length_word_ast,
                    ..
                } => {
                    self.lower_word(offset_word_ast);
                    if let Some(word) = length_word_ast {
                        self.lower_word(word);
                    }
                }
                crate::BourneParameterExpansion::Access { .. }
                | crate::BourneParameterExpansion::Length { .. }
                | crate::BourneParameterExpansion::Indices { .. }
                | crate::BourneParameterExpansion::PrefixMatch { .. }
                | crate::BourneParameterExpansion::Transformation { .. } => {}
            },
            crate::ParameterExpansionSyntax::Zsh(expansion) => {
                match &expansion.target {
                    crate::ZshExpansionTarget::Nested(nested) => {
                        self.collect_parameter_expansion_words(nested);
                    }
                    crate::ZshExpansionTarget::Word(word) => {
                        self.lower_word(word);
                    }
                    crate::ZshExpansionTarget::Reference(_) | crate::ZshExpansionTarget::Empty => {}
                }
                for modifier in &expansion.modifiers {
                    if let Some(word) = &modifier.argument_word_ast {
                        self.lower_word(word);
                    }
                }
                if let Some(operation) = &expansion.operation {
                    self.collect_zsh_operation_words(operation);
                }
            }
        }
    }

    fn collect_zsh_operation_words(&mut self, operation: &crate::ZshExpansionOperation) {
        match operation {
            crate::ZshExpansionOperation::PatternOperation {
                operand_word_ast, ..
            }
            | crate::ZshExpansionOperation::Defaulting {
                operand_word_ast, ..
            }
            | crate::ZshExpansionOperation::TrimOperation {
                operand_word_ast, ..
            } => {
                self.lower_word(operand_word_ast);
            }
            crate::ZshExpansionOperation::ReplacementOperation {
                pattern_word_ast,
                replacement_word_ast,
                ..
            } => {
                self.lower_word(pattern_word_ast);
                if let Some(word) = replacement_word_ast {
                    self.lower_word(word);
                }
            }
            crate::ZshExpansionOperation::Slice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.lower_word(offset_word_ast);
                if let Some(word) = length_word_ast {
                    self.lower_word(word);
                }
            }
            crate::ZshExpansionOperation::Unknown { word_ast, .. } => {
                self.lower_word(word_ast);
            }
        }
    }

    fn collect_pattern_words(&mut self, pattern: &Pattern, words: &mut Vec<WordId>) {
        for part in &pattern.parts {
            match &part.kind {
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_pattern_words(pattern, words);
                    }
                }
                PatternPart::Word(word) => words.push(self.lower_word(word)),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }
}

fn command_kind(command: &Command) -> ArenaFileCommandKind {
    match command {
        Command::Simple(_) => ArenaFileCommandKind::Simple,
        Command::Builtin(_) => ArenaFileCommandKind::Builtin,
        Command::Decl(_) => ArenaFileCommandKind::Decl,
        Command::Binary(_) => ArenaFileCommandKind::Binary,
        Command::Compound(_) => ArenaFileCommandKind::Compound,
        Command::Function(_) => ArenaFileCommandKind::Function,
        Command::AnonymousFunction(_) => ArenaFileCommandKind::AnonymousFunction,
    }
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(BuiltinCommand::Break(command)) => command.span,
        Command::Builtin(BuiltinCommand::Continue(command)) => command.span,
        Command::Builtin(BuiltinCommand::Return(command)) => command.span,
        Command::Builtin(BuiltinCommand::Exit(command)) => command.span,
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(CompoundCommand::If(command)) => command.span,
        Command::Compound(CompoundCommand::For(command)) => command.span,
        Command::Compound(CompoundCommand::Repeat(command)) => command.span,
        Command::Compound(CompoundCommand::Foreach(command)) => command.span,
        Command::Compound(CompoundCommand::ArithmeticFor(command)) => command.span,
        Command::Compound(CompoundCommand::While(command)) => command.span,
        Command::Compound(CompoundCommand::Until(command)) => command.span,
        Command::Compound(CompoundCommand::Case(command)) => command.span,
        Command::Compound(CompoundCommand::Select(command)) => command.span,
        Command::Compound(CompoundCommand::Subshell(sequence))
        | Command::Compound(CompoundCommand::BraceGroup(sequence)) => sequence.span,
        Command::Compound(CompoundCommand::Arithmetic(command)) => command.span,
        Command::Compound(CompoundCommand::Time(command)) => command.span,
        Command::Compound(CompoundCommand::Conditional(command)) => command.span,
        Command::Compound(CompoundCommand::Coproc(command)) => command.span,
        Command::Compound(CompoundCommand::Always(command)) => command.span,
        Command::Function(command) => command.span,
        Command::AnonymousFunction(command) => command.span,
    }
}

#[cfg(test)]
mod tests {
    use crate::{ArenaFile, Command, Span, Stmt, StmtSeq, Word};

    #[test]
    fn arena_file_round_trips_simple_sequence() {
        let file = crate::File {
            body: StmtSeq {
                leading_comments: Vec::new(),
                stmts: vec![Stmt {
                    leading_comments: Vec::new(),
                    command: Command::Simple(crate::SimpleCommand {
                        name: Word::literal("echo"),
                        args: vec![Word::literal("hello")],
                        assignments: Box::new([]),
                        span: Span::new(),
                    }),
                    negated: false,
                    redirects: Box::new([]),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span: Span::new(),
                }],
                trailing_comments: Vec::new(),
                span: Span::new(),
            },
            span: Span::new(),
        };

        let arena = ArenaFile::from_file(&file);

        assert_eq!(arena.store.file_count(), 1);
        assert_eq!(arena.store.stmt_seq_count(), 1);
        assert_eq!(arena.store.stmt_count(), 1);
        assert_eq!(arena.store.command_count(), 1);
        assert_eq!(arena.view().body().stmt_ids().len(), 1);
        assert_eq!(arena.view().body().stmts().len(), 1);

        let materialized = arena.to_file();
        assert_eq!(materialized.body.len(), 1);
        let Command::Simple(command) = &materialized.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
    }
}
