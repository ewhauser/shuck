use crate::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticForCommand, ArithmeticLvalue, Assignment, AssignmentValue, BinaryCommand,
    BuiltinCommand, CaseCommand, Command, Comment, CompoundCommand, ConditionalCommand,
    ConditionalExpr, CoprocCommand, DeclOperand, File, ForCommand, FunctionDef, HeredocBodyPart,
    IdRange, Idx, ListArena, Pattern, PatternPart, Redirect, RedirectTarget, RepeatCommand,
    SelectCommand, Span, Stmt, StmtSeq, StmtTerminator, Subscript, TimeCommand, UntilCommand,
    WhileCommand, Word, WordPart, WordPartNode,
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

    /// Builds an arena representation from an owned parsed root body.
    pub fn from_body(body: StmtSeq, span: Span) -> Self {
        let mut builder = AstStoreBuilder::default();
        let root = builder.lower_file_body(body, span);
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
    assignment_lists: ListArena<Assignment>,
    decl_operand_lists: ListArena<DeclOperand>,
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
            assignment_lists: ListArena::new(),
            decl_operand_lists: ListArena::new(),
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
            command: self.materialize_command(node.command),
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

    fn materialize_command(&self, id: CommandId) -> Command {
        let node = &self.commands[id.index()];
        match &node.payload {
            CommandNodePayload::Simple(command) => Command::Simple(crate::SimpleCommand {
                name: self.materialize_word(command.name),
                args: self
                    .word_id_lists
                    .get(command.args)
                    .iter()
                    .copied()
                    .map(|id| self.materialize_word(id))
                    .collect(),
                assignments: self
                    .assignment_lists
                    .get(command.assignments)
                    .to_vec()
                    .into_boxed_slice(),
                span: node.span,
            }),
            CommandNodePayload::Builtin(command) => {
                let primary = command.primary.map(|id| self.materialize_word(id));
                let extra_args = self
                    .word_id_lists
                    .get(command.extra_args)
                    .iter()
                    .copied()
                    .map(|id| self.materialize_word(id))
                    .collect();
                let assignments = self
                    .assignment_lists
                    .get(command.assignments)
                    .to_vec()
                    .into_boxed_slice();
                let command = match command.kind {
                    BuiltinCommandNodeKind::Break => BuiltinCommand::Break(crate::BreakCommand {
                        depth: primary,
                        extra_args,
                        assignments,
                        span: node.span,
                    }),
                    BuiltinCommandNodeKind::Continue => {
                        BuiltinCommand::Continue(crate::ContinueCommand {
                            depth: primary,
                            extra_args,
                            assignments,
                            span: node.span,
                        })
                    }
                    BuiltinCommandNodeKind::Return => {
                        BuiltinCommand::Return(crate::ReturnCommand {
                            code: primary,
                            extra_args,
                            assignments,
                            span: node.span,
                        })
                    }
                    BuiltinCommandNodeKind::Exit => BuiltinCommand::Exit(crate::ExitCommand {
                        code: primary,
                        extra_args,
                        assignments,
                        span: node.span,
                    }),
                };
                Command::Builtin(command)
            }
            CommandNodePayload::Decl(command) => Command::Decl(crate::DeclClause {
                variant: command.variant.clone(),
                variant_span: command.variant_span,
                operands: self.decl_operand_lists.get(command.operands).to_vec(),
                assignments: self
                    .assignment_lists
                    .get(command.assignments)
                    .to_vec()
                    .into_boxed_slice(),
                span: node.span,
            }),
            CommandNodePayload::Legacy => node.legacy.clone(),
        }
    }

    fn materialize_word(&self, id: WordId) -> Word {
        let node = &self.words[id.index()];
        Word {
            parts: self.word_part_lists.get(node.parts).to_vec(),
            span: node.span,
            brace_syntax: self.brace_syntax_lists.get(node.brace_syntax).to_vec(),
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
    /// Words found under statement-level redirections.
    pub redirect_words: IdRange<WordId>,
    /// Nested statement sequences found under statement-level redirections.
    pub redirect_child_sequences: IdRange<StmtSeqId>,
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
    /// Native arena payload for command families that have been migrated.
    pub payload: CommandNodePayload,
    legacy: Command,
}

/// Arena-native command payloads.
#[derive(Debug, Clone)]
pub enum CommandNodePayload {
    /// Simple command payload.
    Simple(SimpleCommandNode),
    /// Typed builtin command payload.
    Builtin(BuiltinCommandNode),
    /// Declaration builtin command payload.
    Decl(DeclCommandNode),
    /// Compatibility payload for command families that still materialize from `legacy`.
    Legacy,
}

/// Arena-native simple command payload.
#[derive(Debug, Clone)]
pub struct SimpleCommandNode {
    /// Command name word.
    pub name: WordId,
    /// Command argument words.
    pub args: IdRange<WordId>,
    /// Prefix assignments.
    pub assignments: IdRange<Assignment>,
}

/// Arena-native typed builtin command payload.
#[derive(Debug, Clone)]
pub struct BuiltinCommandNode {
    /// Builtin command kind.
    pub kind: BuiltinCommandNodeKind,
    /// Optional primary operand (`depth` or `code` depending on the builtin).
    pub primary: Option<WordId>,
    /// Additional operands preserved for fidelity.
    pub extra_args: IdRange<WordId>,
    /// Prefix assignments.
    pub assignments: IdRange<Assignment>,
}

/// Arena-native typed builtin command kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinCommandNodeKind {
    /// `break [N]`.
    Break,
    /// `continue [N]`.
    Continue,
    /// `return [N]`.
    Return,
    /// `exit [N]`.
    Exit,
}

/// Arena-native declaration command payload.
#[derive(Debug, Clone)]
pub struct DeclCommandNode {
    /// Declaration builtin variant.
    pub variant: crate::Name,
    /// Source span of the declaration builtin name.
    pub variant_span: Span,
    /// Parsed declaration operands.
    pub operands: IdRange<DeclOperand>,
    /// Prefix assignments.
    pub assignments: IdRange<Assignment>,
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

    /// Returns IDs for words found under statement redirections.
    pub fn redirect_word_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().redirect_words)
    }

    /// Returns IDs for nested statement sequences found under statement redirections.
    pub fn redirect_child_sequence_ids(self) -> &'a [StmtSeqId] {
        self.store
            .stmt_seq_id_lists
            .get(self.node().redirect_child_sequences)
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

    /// Returns the native simple command payload when this command is simple.
    pub fn simple(self) -> Option<SimpleCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Simple(_) => Some(SimpleCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Legacy => None,
        }
    }

    /// Returns the native typed builtin payload when this command is a typed builtin.
    pub fn builtin(self) -> Option<BuiltinCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Builtin(_) => Some(BuiltinCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Legacy => None,
        }
    }

    /// Returns the native declaration payload when this command is a declaration builtin.
    pub fn decl(self) -> Option<DeclCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Decl(_) => Some(DeclCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Legacy => None,
        }
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

/// Borrowed view of an arena-native simple command payload.
#[derive(Debug, Clone, Copy)]
pub struct SimpleCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> SimpleCommandView<'a> {
    /// Returns the command name word.
    pub fn name(self) -> WordView<'a> {
        self.store.word(self.node().name)
    }

    /// Returns the command name word ID.
    pub fn name_id(self) -> WordId {
        self.node().name
    }

    /// Returns command argument word IDs.
    pub fn arg_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().args)
    }

    /// Returns command argument words.
    pub fn args(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.arg_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    /// Returns prefix assignments.
    pub fn assignments(self) -> &'a [Assignment] {
        self.store.assignment_lists.get(self.node().assignments)
    }

    fn node(self) -> &'a SimpleCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Simple(command) => command,
            CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Legacy => {
                unreachable!("simple view requires simple payload")
            }
        }
    }
}

/// Borrowed view of an arena-native typed builtin payload.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> BuiltinCommandView<'a> {
    /// Returns the builtin command kind.
    pub fn kind(self) -> BuiltinCommandNodeKind {
        self.node().kind
    }

    /// Returns the primary operand word, if present.
    pub fn primary(self) -> Option<WordView<'a>> {
        self.node().primary.map(|id| self.store.word(id))
    }

    /// Returns the primary operand word ID, if present.
    pub fn primary_id(self) -> Option<WordId> {
        self.node().primary
    }

    /// Returns additional operand word IDs.
    pub fn extra_arg_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().extra_args)
    }

    /// Returns additional operand words.
    pub fn extra_args(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.extra_arg_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    /// Returns prefix assignments.
    pub fn assignments(self) -> &'a [Assignment] {
        self.store.assignment_lists.get(self.node().assignments)
    }

    fn node(self) -> &'a BuiltinCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Builtin(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Legacy => {
                unreachable!("builtin view requires builtin payload")
            }
        }
    }
}

/// Borrowed view of an arena-native declaration command payload.
#[derive(Debug, Clone, Copy)]
pub struct DeclCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> DeclCommandView<'a> {
    /// Returns the declaration builtin variant.
    pub fn variant(self) -> &'a crate::Name {
        &self.node().variant
    }

    /// Returns the source span of the declaration builtin name.
    pub fn variant_span(self) -> Span {
        self.node().variant_span
    }

    /// Returns parsed declaration operands.
    pub fn operands(self) -> &'a [DeclOperand] {
        self.store.decl_operand_lists.get(self.node().operands)
    }

    /// Returns prefix assignments.
    pub fn assignments(self) -> &'a [Assignment] {
        self.store.assignment_lists.get(self.node().assignments)
    }

    fn node(self) -> &'a DeclCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Decl(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Legacy => unreachable!("decl view requires decl payload"),
        }
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

    fn lower_file_body(&mut self, body: StmtSeq, span: Span) -> FileId {
        let body = self.lower_stmt_seq_owned(body);
        let id = Idx::new(self.store.files.len());
        self.store.files.push(FileNode { body, span });
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

    fn lower_stmt_seq_owned(&mut self, sequence: StmtSeq) -> StmtSeqId {
        let leading_comments = self
            .store
            .comment_lists
            .push_many(sequence.leading_comments);
        let stmts = sequence
            .stmts
            .into_iter()
            .map(|stmt| self.lower_stmt_owned(stmt))
            .collect::<Vec<_>>();
        let stmts = self.store.stmt_id_lists.push_many(stmts);
        let trailing_comments = self
            .store
            .comment_lists
            .push_many(sequence.trailing_comments);

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
        let mut redirect_words = Vec::new();
        let mut redirect_child_sequences = Vec::new();
        for redirect in stmt.redirects.iter() {
            self.collect_redirect_children(
                redirect,
                &mut redirect_words,
                &mut redirect_child_sequences,
            );
        }
        let redirect_words = self.store.word_id_lists.push_many(redirect_words);
        let redirect_child_sequences = self
            .store
            .stmt_seq_id_lists
            .push_many(redirect_child_sequences);
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
            redirect_words,
            redirect_child_sequences,
            terminator: stmt.terminator,
            terminator_span: stmt.terminator_span,
            inline_comment: stmt.inline_comment,
            span: stmt.span,
        });
        id
    }

    fn lower_stmt_owned(&mut self, stmt: Stmt) -> StmtId {
        let leading_comments = self.store.comment_lists.push_many(stmt.leading_comments);
        let command = self.lower_command_owned(stmt.command);
        let mut redirect_words = Vec::new();
        let mut redirect_child_sequences = Vec::new();
        for redirect in stmt.redirects.iter() {
            self.collect_redirect_children(
                redirect,
                &mut redirect_words,
                &mut redirect_child_sequences,
            );
        }
        let redirect_words = self.store.word_id_lists.push_many(redirect_words);
        let redirect_child_sequences = self
            .store
            .stmt_seq_id_lists
            .push_many(redirect_child_sequences);
        let redirects = self
            .store
            .redirect_lists
            .push_many(stmt.redirects.into_vec());

        let id = Idx::new(self.store.stmts.len());
        self.store.stmts.push(StmtNode {
            leading_comments,
            command,
            negated: stmt.negated,
            redirects,
            redirect_words,
            redirect_child_sequences,
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
        let payload = self.collect_command_children(command, &mut words, &mut child_sequences);

        let words = self.store.word_id_lists.push_many(words);
        let child_sequences = self.store.stmt_seq_id_lists.push_many(child_sequences);
        let id = Idx::new(self.store.commands.len());
        self.store.commands.push(CommandNode {
            kind: command_kind(command),
            span: command_span(command),
            words,
            child_sequences,
            payload,
            legacy: command.clone(),
        });
        id
    }

    fn lower_command_owned(&mut self, command: Command) -> CommandId {
        let mut words = Vec::new();
        let mut child_sequences = Vec::new();
        let payload = self.collect_command_children(&command, &mut words, &mut child_sequences);

        let words = self.store.word_id_lists.push_many(words);
        let child_sequences = self.store.stmt_seq_id_lists.push_many(child_sequences);
        let id = Idx::new(self.store.commands.len());
        self.store.commands.push(CommandNode {
            kind: command_kind(&command),
            span: command_span(&command),
            words,
            child_sequences,
            payload,
            legacy: command,
        });
        id
    }

    fn lower_word(&mut self, word: &Word) -> WordId {
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

    fn collect_word(
        &mut self,
        word: &Word,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        self.collect_word_part_children(word.parts.as_slice(), words, child_sequences);
        words.push(self.lower_word(word));
    }

    fn collect_word_id(
        &mut self,
        word: &Word,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> WordId {
        self.collect_word_part_children(word.parts.as_slice(), words, child_sequences);
        let id = self.lower_word(word);
        words.push(id);
        id
    }

    fn collect_command_children(
        &mut self,
        command: &Command,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CommandNodePayload {
        match command {
            Command::Simple(command) => {
                let name = self.collect_word_id(&command.name, words, child_sequences);
                let args = command
                    .args
                    .iter()
                    .map(|word| self.collect_word_id(word, words, child_sequences))
                    .collect::<Vec<_>>();
                let args = self.store.word_id_lists.push_many(args);
                self.collect_assignments(command.assignments.iter(), words, child_sequences);
                let assignments = self
                    .store
                    .assignment_lists
                    .push_many(command.assignments.iter().cloned());
                CommandNodePayload::Simple(SimpleCommandNode {
                    name,
                    args,
                    assignments,
                })
            }
            Command::Builtin(command) => {
                self.collect_builtin_children(command, words, child_sequences)
            }
            Command::Decl(command) => {
                for operand in &command.operands {
                    self.collect_decl_operand(operand, words, child_sequences);
                }
                self.collect_assignments(command.assignments.iter(), words, child_sequences);
                let operands = self
                    .store
                    .decl_operand_lists
                    .push_many(command.operands.iter().cloned());
                let assignments = self
                    .store
                    .assignment_lists
                    .push_many(command.assignments.iter().cloned());
                CommandNodePayload::Decl(DeclCommandNode {
                    variant: command.variant.clone(),
                    variant_span: command.variant_span,
                    operands,
                    assignments,
                })
            }
            Command::Binary(command) => {
                self.collect_binary_children(command, child_sequences);
                CommandNodePayload::Legacy
            }
            Command::Compound(command) => {
                self.collect_compound_children(command, words, child_sequences);
                CommandNodePayload::Legacy
            }
            Command::Function(function) => {
                self.collect_function_children(function, words, child_sequences);
                CommandNodePayload::Legacy
            }
            Command::AnonymousFunction(function) => {
                self.collect_anonymous_function_children(function, words, child_sequences);
                CommandNodePayload::Legacy
            }
        }
    }

    fn collect_builtin_children(
        &mut self,
        command: &BuiltinCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CommandNodePayload {
        match command {
            BuiltinCommand::Break(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Break,
                command.depth.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
            BuiltinCommand::Continue(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Continue,
                command.depth.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
            BuiltinCommand::Return(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Return,
                command.code.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
            BuiltinCommand::Exit(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Exit,
                command.code.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
        }
    }

    fn collect_builtin_payload<'a>(
        &mut self,
        kind: BuiltinCommandNodeKind,
        primary: Option<&Word>,
        extra_args: &[Word],
        assignments: impl Iterator<Item = &'a Assignment> + Clone,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CommandNodePayload {
        let primary = primary.map(|word| self.collect_word_id(word, words, child_sequences));
        let extra_args = extra_args
            .iter()
            .map(|word| self.collect_word_id(word, words, child_sequences))
            .collect::<Vec<_>>();
        let extra_args = self.store.word_id_lists.push_many(extra_args);
        self.collect_assignments(assignments.clone(), words, child_sequences);
        let assignments = self.store.assignment_lists.push_many(assignments.cloned());
        CommandNodePayload::Builtin(BuiltinCommandNode {
            kind,
            primary,
            extra_args,
            assignments,
        })
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
                for word in &command.words {
                    self.collect_word(word, words, child_sequences);
                }
                child_sequences.push(self.lower_stmt_seq(&command.body));
            }
            CompoundCommand::ArithmeticFor(command) => {
                self.collect_arithmetic_for_children(command, words, child_sequences);
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
            CompoundCommand::Arithmetic(command) => {
                self.collect_arithmetic_command_children(command, words, child_sequences);
            }
            CompoundCommand::Conditional(command) => {
                self.collect_conditional_command_children(command, words, child_sequences);
            }
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
        for target in &command.targets {
            self.collect_word(&target.word, words, child_sequences);
        }
        if let Some(header_words) = &command.words {
            for word in header_words {
                self.collect_word(word, words, child_sequences);
            }
        }
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_repeat_children(
        &mut self,
        command: &RepeatCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        self.collect_word(&command.count, words, child_sequences);
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_arithmetic_for_children(
        &mut self,
        command: &ArithmeticForCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        self.collect_arithmetic_expr_option(command.init_ast.as_ref(), words, child_sequences);
        self.collect_arithmetic_expr_option(command.condition_ast.as_ref(), words, child_sequences);
        self.collect_arithmetic_expr_option(command.step_ast.as_ref(), words, child_sequences);
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
        self.collect_word(&command.word, words, child_sequences);
        for case in &command.cases {
            for pattern in &case.patterns {
                self.collect_pattern_words(pattern, words, child_sequences);
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
        for word in &command.words {
            self.collect_word(word, words, child_sequences);
        }
        child_sequences.push(self.lower_stmt_seq(&command.body));
    }

    fn collect_arithmetic_command_children(
        &mut self,
        command: &ArithmeticCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        self.collect_arithmetic_expr_option(command.expr_ast.as_ref(), words, child_sequences);
    }

    fn collect_conditional_command_children(
        &mut self,
        command: &ConditionalCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        self.collect_conditional_expr(&command.expression, words, child_sequences);
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
        for entry in &function.header.entries {
            self.collect_word(&entry.word, words, child_sequences);
        }
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
        for word in &function.args {
            self.collect_word(word, words, child_sequences);
        }
    }

    fn collect_decl_operand(
        &mut self,
        operand: &DeclOperand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                self.collect_word(word, words, child_sequences);
            }
            DeclOperand::Assignment(assignment) => {
                self.collect_assignment(assignment, words, child_sequences)
            }
            DeclOperand::Name(reference) => {
                self.collect_var_ref_words(reference, words, child_sequences);
            }
        }
    }

    fn collect_assignments<'a>(
        &mut self,
        assignments: impl Iterator<Item = &'a Assignment>,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        for assignment in assignments {
            self.collect_assignment(assignment, words, child_sequences);
        }
    }

    fn collect_assignment(
        &mut self,
        assignment: &Assignment,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        self.collect_var_ref_words(&assignment.target, words, child_sequences);
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.collect_word(word, words, child_sequences),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        crate::ArrayElem::Sequential(value) => {
                            self.collect_word(&value.word, words, child_sequences);
                        }
                        crate::ArrayElem::Keyed { key, value }
                        | crate::ArrayElem::KeyedAppend { key, value } => {
                            self.collect_subscript_words(key, words, child_sequences);
                            self.collect_word(&value.word, words, child_sequences);
                        }
                    }
                }
            }
        }
    }

    fn collect_redirect_children(
        &mut self,
        redirect: &Redirect,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match &redirect.target {
            RedirectTarget::Word(word) => {
                self.collect_word(word, words, child_sequences);
            }
            RedirectTarget::Heredoc(heredoc) => {
                self.collect_word(&heredoc.delimiter.raw, words, child_sequences);
                for part in &heredoc.body.parts {
                    match &part.kind {
                        HeredocBodyPart::CommandSubstitution { body, .. } => {
                            child_sequences.push(self.lower_stmt_seq(body));
                        }
                        HeredocBodyPart::ArithmeticExpansion {
                            expression_word_ast,
                            ..
                        } => {
                            self.collect_word(expression_word_ast, words, child_sequences);
                        }
                        HeredocBodyPart::Parameter(expansion) => {
                            self.collect_parameter_expansion_words(
                                expansion,
                                words,
                                child_sequences,
                            );
                        }
                        HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
                    }
                }
            }
        }
    }

    fn collect_word_part_children(
        &mut self,
        parts: &[WordPartNode],
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_word_part_children(parts, words, child_sequences)
                }
                WordPart::CommandSubstitution { body, .. }
                | WordPart::ProcessSubstitution { body, .. } => {
                    child_sequences.push(self.lower_stmt_seq(body));
                }
                WordPart::ArithmeticExpansion {
                    expression_ast,
                    expression_word_ast,
                    ..
                } => {
                    self.collect_arithmetic_expr_option(
                        expression_ast.as_ref(),
                        words,
                        child_sequences,
                    );
                    self.collect_word(expression_word_ast, words, child_sequences);
                }
                WordPart::Parameter(expansion) => {
                    self.collect_parameter_expansion_words(expansion, words, child_sequences);
                }
                WordPart::ParameterExpansion {
                    reference,
                    operand_word_ast,
                    ..
                }
                | WordPart::IndirectExpansion {
                    reference,
                    operand_word_ast,
                    ..
                } => {
                    self.collect_var_ref_words(reference, words, child_sequences);
                    if let Some(word) = operand_word_ast {
                        self.collect_word(word, words, child_sequences);
                    }
                }
                WordPart::Length(reference)
                | WordPart::ArrayAccess(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::ArrayIndices(reference) => {
                    self.collect_var_ref_words(reference, words, child_sequences);
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
                    self.collect_var_ref_words(reference, words, child_sequences);
                    self.collect_arithmetic_expr_option(
                        offset_ast.as_ref(),
                        words,
                        child_sequences,
                    );
                    self.collect_word(offset_word_ast, words, child_sequences);
                    self.collect_arithmetic_expr_option(
                        length_ast.as_ref(),
                        words,
                        child_sequences,
                    );
                    if let Some(word) = length_word_ast {
                        self.collect_word(word, words, child_sequences);
                    }
                }
                WordPart::ZshQualifiedGlob(glob) => {
                    for segment in &glob.segments {
                        if let crate::ZshGlobSegment::Pattern(pattern) = segment {
                            self.collect_pattern_words(pattern, words, child_sequences);
                        }
                    }
                }
                WordPart::Literal(_)
                | WordPart::SingleQuoted { .. }
                | WordPart::Variable(_)
                | WordPart::PrefixMatch { .. } => {}
                WordPart::Transformation { reference, .. } => {
                    self.collect_var_ref_words(reference, words, child_sequences);
                }
            }
        }
    }

    fn collect_parameter_expansion_words(
        &mut self,
        expansion: &crate::ParameterExpansion,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match &expansion.syntax {
            crate::ParameterExpansionSyntax::Bourne(expansion) => match expansion {
                crate::BourneParameterExpansion::Access { reference }
                | crate::BourneParameterExpansion::Length { reference }
                | crate::BourneParameterExpansion::Indices { reference }
                | crate::BourneParameterExpansion::Transformation { reference, .. } => {
                    self.collect_var_ref_words(reference, words, child_sequences);
                }
                crate::BourneParameterExpansion::Indirect {
                    reference,
                    operand_word_ast,
                    ..
                }
                | crate::BourneParameterExpansion::Operation {
                    reference,
                    operand_word_ast,
                    ..
                } => {
                    self.collect_var_ref_words(reference, words, child_sequences);
                    if let Some(word) = operand_word_ast {
                        self.collect_word(word, words, child_sequences);
                    }
                }
                crate::BourneParameterExpansion::Slice {
                    reference,
                    offset_ast,
                    offset_word_ast,
                    length_ast,
                    length_word_ast,
                    ..
                } => {
                    self.collect_var_ref_words(reference, words, child_sequences);
                    self.collect_arithmetic_expr_option(
                        offset_ast.as_ref(),
                        words,
                        child_sequences,
                    );
                    self.collect_word(offset_word_ast, words, child_sequences);
                    self.collect_arithmetic_expr_option(
                        length_ast.as_ref(),
                        words,
                        child_sequences,
                    );
                    if let Some(word) = length_word_ast {
                        self.collect_word(word, words, child_sequences);
                    }
                }
                crate::BourneParameterExpansion::PrefixMatch { .. } => {}
            },
            crate::ParameterExpansionSyntax::Zsh(expansion) => {
                match &expansion.target {
                    crate::ZshExpansionTarget::Nested(nested) => {
                        self.collect_parameter_expansion_words(nested, words, child_sequences);
                    }
                    crate::ZshExpansionTarget::Word(word) => {
                        self.collect_word(word, words, child_sequences);
                    }
                    crate::ZshExpansionTarget::Reference(reference) => {
                        self.collect_var_ref_words(reference, words, child_sequences);
                    }
                    crate::ZshExpansionTarget::Empty => {}
                }
                for modifier in &expansion.modifiers {
                    if let Some(word) = &modifier.argument_word_ast {
                        self.collect_word(word, words, child_sequences);
                    }
                }
                if let Some(operation) = &expansion.operation {
                    self.collect_zsh_operation_words(operation, words, child_sequences);
                }
            }
        }
    }

    fn collect_zsh_operation_words(
        &mut self,
        operation: &crate::ZshExpansionOperation,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
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
                self.collect_word(operand_word_ast, words, child_sequences);
            }
            crate::ZshExpansionOperation::ReplacementOperation {
                pattern_word_ast,
                replacement_word_ast,
                ..
            } => {
                self.collect_word(pattern_word_ast, words, child_sequences);
                if let Some(word) = replacement_word_ast {
                    self.collect_word(word, words, child_sequences);
                }
            }
            crate::ZshExpansionOperation::Slice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.collect_word(offset_word_ast, words, child_sequences);
                if let Some(word) = length_word_ast {
                    self.collect_word(word, words, child_sequences);
                }
            }
            crate::ZshExpansionOperation::Unknown { word_ast, .. } => {
                self.collect_word(word_ast, words, child_sequences);
            }
        }
    }

    fn collect_pattern_words(
        &mut self,
        pattern: &Pattern,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        for part in &pattern.parts {
            match &part.kind {
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_pattern_words(pattern, words, child_sequences);
                    }
                }
                PatternPart::Word(word) => self.collect_word(word, words, child_sequences),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn collect_conditional_expr(
        &mut self,
        expression: &ConditionalExpr,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.collect_conditional_expr(&expr.left, words, child_sequences);
                self.collect_conditional_expr(&expr.right, words, child_sequences);
            }
            ConditionalExpr::Unary(expr) => {
                self.collect_conditional_expr(&expr.expr, words, child_sequences);
            }
            ConditionalExpr::Parenthesized(expr) => {
                self.collect_conditional_expr(&expr.expr, words, child_sequences);
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.collect_word(word, words, child_sequences);
            }
            ConditionalExpr::Pattern(pattern) => {
                self.collect_pattern_words(pattern, words, child_sequences);
            }
            ConditionalExpr::VarRef(var_ref) => {
                self.collect_var_ref_words(var_ref, words, child_sequences);
            }
        }
    }

    fn collect_arithmetic_expr_option(
        &mut self,
        expression: Option<&ArithmeticExprNode>,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        if let Some(expression) = expression {
            self.collect_arithmetic_expr(expression, words, child_sequences);
        }
    }

    fn collect_arithmetic_expr(
        &mut self,
        expression: &ArithmeticExprNode,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match &expression.kind {
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => {
                self.collect_arithmetic_expr(index, words, child_sequences);
            }
            ArithmeticExpr::ShellWord(word) => {
                self.collect_word(word, words, child_sequences);
            }
            ArithmeticExpr::Parenthesized { expression } => {
                self.collect_arithmetic_expr(expression, words, child_sequences);
            }
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                self.collect_arithmetic_expr(expr, words, child_sequences);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                self.collect_arithmetic_expr(left, words, child_sequences);
                self.collect_arithmetic_expr(right, words, child_sequences);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_arithmetic_expr(condition, words, child_sequences);
                self.collect_arithmetic_expr(then_expr, words, child_sequences);
                self.collect_arithmetic_expr(else_expr, words, child_sequences);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                self.collect_arithmetic_lvalue(target, words, child_sequences);
                self.collect_arithmetic_expr(value, words, child_sequences);
            }
        }
    }

    fn collect_arithmetic_lvalue(
        &mut self,
        lvalue: &ArithmeticLvalue,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        match lvalue {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => {
                self.collect_arithmetic_expr(index, words, child_sequences);
            }
        }
    }

    fn collect_subscript_words(
        &mut self,
        subscript: &Subscript,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        if let Some(word) = subscript.word_ast() {
            self.collect_word(word, words, child_sequences);
        }
        self.collect_arithmetic_expr_option(
            subscript.arithmetic_ast.as_ref(),
            words,
            child_sequences,
        );
    }

    fn collect_var_ref_words(
        &mut self,
        var_ref: &crate::VarRef,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) {
        if let Some(subscript) = var_ref.subscript.as_deref() {
            self.collect_subscript_words(subscript, words, child_sequences);
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
    use crate::{
        ArenaFile, ArithmeticCommand, ArithmeticExpr, ArithmeticExprNode, ArrayElem, ArrayExpr,
        ArrayKind, ArrayValueWord, Assignment, AssignmentValue, BourneParameterExpansion,
        BuiltinCommand, BuiltinCommandNodeKind, Command, CommandSubstitutionSyntax,
        CompoundCommand, ConditionalCommand, ConditionalExpr, DeclClause, DeclOperand, Heredoc,
        HeredocBody, HeredocBodyMode, HeredocBodyPart, HeredocBodyPartNode, HeredocDelimiter, Name,
        ParameterExpansion, ParameterExpansionSyntax, Pattern, PatternPart, PatternPartNode,
        Redirect, RedirectKind, RedirectTarget, SimpleCommand, Span, Stmt, StmtSeq, Subscript,
        SubscriptInterpretation, SubscriptKind, Word, WordPart, WordPartNode, ZshExpansionTarget,
        ZshGlobSegment, ZshParameterExpansion, ZshQualifiedGlob,
    };

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

    #[test]
    fn arena_file_builds_from_owned_body() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![Word::literal("hello")],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_body(file.body, file.span);
        let materialized = arena.to_file();

        assert_eq!(arena.store.file_count(), 1);
        assert_eq!(arena.store.stmt_seq_count(), 1);
        assert_eq!(materialized.body.len(), 1);
    }

    #[test]
    fn command_substitution_body_is_reachable_from_command() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word_with_command_substitution()],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 2);
    }

    #[test]
    fn simple_command_payload_is_arena_native() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("printf"),
            args: vec![Word::literal("%s"), Word::literal("value")],
            assignments: Box::new([Assignment {
                target: crate::VarRef {
                    name: Name::new("LC_ALL"),
                    name_span: Span::new(),
                    subscript: None,
                    span: Span::new(),
                },
                value: AssignmentValue::Scalar(Word::literal("C")),
                append: false,
                span: Span::new(),
            }]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let simple = command.simple().expect("expected native simple payload");

        assert_eq!(simple.name().parts().len(), 1);
        assert_eq!(simple.arg_ids().len(), 2);
        assert_eq!(simple.args().len(), 2);
        assert_eq!(simple.assignments().len(), 1);

        let materialized = arena.to_file();
        let Command::Simple(command) = &materialized.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.assignments.len(), 1);
    }

    #[test]
    fn builtin_command_payload_is_arena_native() {
        let file = file_with_command(Command::Builtin(BuiltinCommand::Return(
            crate::ReturnCommand {
                code: Some(Word::literal("7")),
                extra_args: vec![Word::literal("extra")],
                assignments: Box::new([Assignment {
                    target: crate::VarRef {
                        name: Name::new("status"),
                        name_span: Span::new(),
                        subscript: None,
                        span: Span::new(),
                    },
                    value: AssignmentValue::Scalar(Word::literal("set")),
                    append: false,
                    span: Span::new(),
                }]),
                span: Span::new(),
            },
        )));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let builtin = command.builtin().expect("expected native builtin payload");

        assert_eq!(builtin.kind(), BuiltinCommandNodeKind::Return);
        assert!(builtin.primary().is_some());
        assert_eq!(builtin.extra_arg_ids().len(), 1);
        assert_eq!(builtin.extra_args().len(), 1);
        assert_eq!(builtin.assignments().len(), 1);

        let materialized = arena.to_file();
        let Command::Builtin(BuiltinCommand::Return(command)) = &materialized.body[0].command
        else {
            panic!("expected return command");
        };
        assert!(command.code.is_some());
        assert_eq!(command.extra_args.len(), 1);
        assert_eq!(command.assignments.len(), 1);
    }

    #[test]
    fn declaration_command_payload_is_arena_native() {
        let file = file_with_command(Command::Decl(DeclClause {
            variant: Name::new("declare"),
            variant_span: Span::new(),
            operands: vec![
                DeclOperand::Flag(Word::literal("-a")),
                DeclOperand::Name(var_ref_with_dynamic_subscript("arr")),
            ],
            assignments: Box::new([Assignment {
                target: crate::VarRef {
                    name: Name::new("prefix"),
                    name_span: Span::new(),
                    subscript: None,
                    span: Span::new(),
                },
                value: AssignmentValue::Scalar(Word::literal("value")),
                append: false,
                span: Span::new(),
            }]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let decl = command.decl().expect("expected native declaration payload");

        assert_eq!(decl.variant(), "declare");
        assert_eq!(decl.operands().len(), 2);
        assert_eq!(decl.assignments().len(), 1);
        assert_eq!(command.child_sequence_ids().len(), 1);

        let materialized = arena.to_file();
        let Command::Decl(command) = &materialized.body[0].command else {
            panic!("expected declaration command");
        };
        assert_eq!(command.operands.len(), 2);
        assert_eq!(command.assignments.len(), 1);
    }

    #[test]
    fn heredoc_substitution_body_is_reachable_from_statement_redirects() {
        let redirect = Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::HereDoc,
            span: Span::new(),
            target: RedirectTarget::Heredoc(Heredoc {
                delimiter: HeredocDelimiter {
                    raw: Word::literal("EOF"),
                    cooked: "EOF".to_string(),
                    span: Span::new(),
                    quoted: false,
                    expands_body: true,
                    strip_tabs: false,
                },
                body: HeredocBody {
                    mode: HeredocBodyMode::Expanding,
                    source_backed: false,
                    parts: vec![HeredocBodyPartNode::new(
                        HeredocBodyPart::CommandSubstitution {
                            body: simple_sequence("inner"),
                            syntax: CommandSubstitutionSyntax::DollarParen,
                        },
                        Span::new(),
                    )],
                    span: Span::new(),
                },
            }),
        };
        let file = file_with_stmt(Stmt {
            leading_comments: Vec::new(),
            command: Command::Simple(SimpleCommand {
                name: Word::literal("cat"),
                args: Vec::new(),
                assignments: Box::new([]),
                span: Span::new(),
            }),
            negated: false,
            redirects: Box::new([redirect]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        });

        let arena = ArenaFile::from_file(&file);
        let stmt = arena.view().body().stmts().next().unwrap();

        assert_eq!(stmt.redirect_child_sequence_ids().len(), 1);
        assert!(!stmt.redirect_word_ids().is_empty());
    }

    #[test]
    fn zsh_qualified_glob_pattern_words_are_command_words() {
        let pattern = Pattern {
            parts: vec![PatternPartNode::new(
                PatternPart::Word(Word::literal("nested")),
                Span::new(),
            )],
            span: Span::new(),
        };
        let glob_word = Word {
            parts: vec![WordPartNode::new(
                WordPart::ZshQualifiedGlob(ZshQualifiedGlob {
                    span: Span::new(),
                    segments: vec![ZshGlobSegment::Pattern(pattern)],
                    qualifiers: None,
                }),
                Span::new(),
            )],
            span: Span::new(),
            brace_syntax: Vec::new(),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("print"),
            args: vec![glob_word],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.word_ids().len(), 3);
    }

    #[test]
    fn conditional_and_arithmetic_words_link_nested_substitutions() {
        let conditional_file = file_with_command(Command::Compound(CompoundCommand::Conditional(
            ConditionalCommand {
                expression: ConditionalExpr::Word(word_with_command_substitution()),
                span: Span::new(),
                left_bracket_span: Span::new(),
                right_bracket_span: Span::new(),
            },
        )));
        let arithmetic_file = file_with_command(Command::Compound(CompoundCommand::Arithmetic(
            ArithmeticCommand {
                span: Span::new(),
                left_paren_span: Span::new(),
                expr_span: Some(Span::new()),
                expr_ast: Some(ArithmeticExprNode::new(
                    ArithmeticExpr::ShellWord(word_with_command_substitution()),
                    Span::new(),
                )),
                right_paren_span: Span::new(),
            },
        )));

        for file in [conditional_file, arithmetic_file] {
            let arena = ArenaFile::from_file(&file);
            let command = arena.view().body().stmts().next().unwrap().command();

            assert_eq!(command.child_sequence_ids().len(), 1);
            assert!(!command.word_ids().is_empty());
        }
    }

    #[test]
    fn keyed_array_subscript_words_are_assignment_words() {
        let assignment = Assignment {
            target: crate::VarRef {
                name: Name::new("arr"),
                name_span: Span::new(),
                subscript: None,
                span: Span::new(),
            },
            value: AssignmentValue::Compound(ArrayExpr {
                kind: ArrayKind::Associative,
                elements: vec![ArrayElem::Keyed {
                    key: dynamic_subscript(),
                    value: ArrayValueWord::from(Word::literal("value")),
                }],
                span: Span::new(),
            }),
            append: false,
            span: Span::new(),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("printf"),
            args: Vec::new(),
            assignments: Box::new([assignment]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 3);
    }

    #[test]
    fn assignment_target_subscript_words_are_assignment_words() {
        let assignment = Assignment {
            target: crate::VarRef {
                name: Name::new("arr"),
                name_span: Span::new(),
                subscript: Some(Box::new(dynamic_subscript())),
                span: Span::new(),
            },
            value: AssignmentValue::Scalar(Word::literal("value")),
            append: false,
            span: Span::new(),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("printf"),
            args: Vec::new(),
            assignments: Box::new([assignment]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 3);
    }

    #[test]
    fn bourne_parameter_subscript_words_are_command_words() {
        let expansion = ParameterExpansion {
            syntax: ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: var_ref_with_dynamic_subscript("arr"),
            }),
            span: Span::new(),
            raw_body: crate::SourceText::cooked(Span::new(), "arr[$(idx)]"),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word(vec![WordPart::Parameter(expansion)])],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 3);
    }

    #[test]
    fn declaration_name_subscript_words_are_command_words() {
        let file = file_with_command(Command::Decl(DeclClause {
            variant: Name::new("declare"),
            variant_span: Span::new(),
            operands: vec![DeclOperand::Name(var_ref_with_dynamic_subscript("arr"))],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(!command.word_ids().is_empty());
    }

    #[test]
    fn zsh_reference_target_subscript_words_are_command_words() {
        let expansion = ParameterExpansion {
            syntax: ParameterExpansionSyntax::Zsh(ZshParameterExpansion {
                target: ZshExpansionTarget::Reference(var_ref_with_dynamic_subscript("arr")),
                modifiers: Vec::new(),
                length_prefix: None,
                operation: None,
            }),
            span: Span::new(),
            raw_body: crate::SourceText::cooked(Span::new(), "arr[$(idx)]"),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word(vec![WordPart::Parameter(expansion)])],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 2);
    }

    #[test]
    fn transformation_subscript_words_are_command_words() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word(vec![WordPart::Transformation {
                reference: var_ref_with_dynamic_subscript("arr"),
                operator: 'Q',
            }])],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 2);
    }

    fn file_with_command(command: Command) -> crate::File {
        file_with_stmt(Stmt {
            leading_comments: Vec::new(),
            command,
            negated: false,
            redirects: Box::new([]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        })
    }

    fn file_with_stmt(stmt: Stmt) -> crate::File {
        crate::File {
            body: StmtSeq {
                leading_comments: Vec::new(),
                stmts: vec![stmt],
                trailing_comments: Vec::new(),
                span: Span::new(),
            },
            span: Span::new(),
        }
    }

    fn simple_sequence(name: &str) -> StmtSeq {
        StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![Stmt {
                leading_comments: Vec::new(),
                command: Command::Simple(SimpleCommand {
                    name: Word::literal(name),
                    args: Vec::new(),
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
        }
    }

    fn word_with_command_substitution() -> Word {
        word(vec![WordPart::CommandSubstitution {
            body: simple_sequence("subcmd"),
            syntax: CommandSubstitutionSyntax::DollarParen,
        }])
    }

    fn word(parts: Vec<WordPart>) -> Word {
        Word {
            parts: parts
                .into_iter()
                .map(|part| WordPartNode::new(part, Span::new()))
                .collect(),
            span: Span::new(),
            brace_syntax: Vec::new(),
        }
    }

    fn var_ref_with_dynamic_subscript(name: &str) -> crate::VarRef {
        crate::VarRef {
            name: Name::new(name),
            name_span: Span::new(),
            subscript: Some(Box::new(dynamic_subscript())),
            span: Span::new(),
        }
    }

    fn dynamic_subscript() -> Subscript {
        Subscript {
            text: crate::SourceText::cooked(Span::new(), "$(key)"),
            raw: None,
            kind: SubscriptKind::Ordinary,
            interpretation: SubscriptInterpretation::Indexed,
            word_ast: Some(word_with_command_substitution()),
            arithmetic_ast: None,
        }
    }
}
