pub use shuck_semantic::CommandId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FactSpan {
    start: usize,
    end: usize,
}

impl FactSpan {
    pub fn new(span: Span) -> Self {
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
pub struct WordNodeId(u32);

impl WordNodeId {
    fn new(index: usize) -> Self {
        Self(fact_id_index_to_u32(index, "word node id"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WordOccurrenceId(u32);

impl WordOccurrenceId {
    fn new(index: usize) -> Self {
        Self(fact_id_index_to_u32(index, "word occurrence id"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[inline]
fn fact_id_index_to_u32(index: usize, kind: &'static str) -> u32 {
    if index > u32::MAX as usize {
        fact_id_index_overflow(kind);
    }
    index as u32
}

#[cold]
#[inline(never)]
fn fact_id_index_overflow(kind: &'static str) -> ! {
    panic!("{kind} must fit in u32");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CommandLookupKind {
    Simple,
    Builtin(BuiltinLookupKind),
    Decl,
    Binary,
    Compound(CompoundLookupKind),
    Function,
    AnonymousFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BuiltinLookupKind {
    Break,
    Continue,
    Return,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CompoundLookupKind {
    If,
    For,
    Repeat,
    Foreach,
    ArithmeticFor,
    While,
    Until,
    Case,
    Select,
    Subshell,
    BraceGroup,
    Arithmetic,
    Time,
    Conditional,
    Coproc,
    Always,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CommandLookupEntry {
    kind: CommandLookupKind,
    id: CommandId,
}

type CommandLookupIndex = FxHashMap<FactSpan, SmallVec<[CommandLookupEntry; 1]>>;

#[derive(Debug, Clone)]
struct FactStore<'a> {
    redirect_facts: ListArena<RedirectFact<'a>>,
    substitution_facts: ListArena<SubstitutionFact>,
    scope_read_source_words: ListArena<PathWordFact<'a>>,
    scope_name_read_uses: ListArena<ComparableNameUse>,
    scope_heredoc_name_read_uses: ListArena<ComparableNameUse>,
    scope_name_write_uses: ListArena<ComparableNameUse>,
    declaration_assignment_probes: ListArena<DeclarationAssignmentProbe>,
    word_occurrence_ids: ListArena<WordOccurrenceId>,
    word_occurrence_ids_by_command: Vec<IdRange<WordOccurrenceId>>,
    word_spans: ListArena<Span>,
}

#[derive(Debug, Clone)]
pub(crate) struct CommandChildIndex {
    ids: ListArena<CommandId>,
    by_parent: Vec<IdRange<CommandId>>,
}

impl CommandChildIndex {
    fn from_parent_lists(children_by_parent: Vec<Vec<CommandId>>) -> Self {
        let total_children = children_by_parent.iter().map(Vec::len).sum();
        let mut ids = ListArena::with_capacity(total_children);
        let mut by_parent = Vec::with_capacity(children_by_parent.len());

        for children in children_by_parent {
            by_parent.push(ids.push_many(children));
        }

        Self { ids, by_parent }
    }

    fn child_ids(&self, id: CommandId) -> &[CommandId] {
        self.by_parent
            .get(id.index())
            .copied()
            .map_or(&[], |range| self.ids.get(range))
    }
}

impl<'a> FactStore<'a> {
    fn empty() -> Self {
        Self {
            redirect_facts: ListArena::new(),
            substitution_facts: ListArena::new(),
            scope_read_source_words: ListArena::new(),
            scope_name_read_uses: ListArena::new(),
            scope_heredoc_name_read_uses: ListArena::new(),
            scope_name_write_uses: ListArena::new(),
            declaration_assignment_probes: ListArena::new(),
            word_occurrence_ids: ListArena::new(),
            word_occurrence_ids_by_command: Vec::new(),
            word_spans: ListArena::new(),
        }
    }

    fn redirect_facts(&self, range: IdRange<RedirectFact<'a>>) -> &[RedirectFact<'a>] {
        self.redirect_facts.get(range)
    }

    fn substitution_facts(&self, range: IdRange<SubstitutionFact>) -> &[SubstitutionFact] {
        self.substitution_facts.get(range)
    }

    fn scope_read_source_words(&self, range: IdRange<PathWordFact<'a>>) -> &[PathWordFact<'a>] {
        self.scope_read_source_words.get(range)
    }

    fn scope_name_read_uses(&self, range: IdRange<ComparableNameUse>) -> &[ComparableNameUse] {
        self.scope_name_read_uses.get(range)
    }

    fn scope_heredoc_name_read_uses(
        &self,
        range: IdRange<ComparableNameUse>,
    ) -> &[ComparableNameUse] {
        self.scope_heredoc_name_read_uses.get(range)
    }

    fn scope_name_write_uses(&self, range: IdRange<ComparableNameUse>) -> &[ComparableNameUse] {
        self.scope_name_write_uses.get(range)
    }

    fn declaration_assignment_probes(
        &self,
        range: IdRange<DeclarationAssignmentProbe>,
    ) -> &[DeclarationAssignmentProbe] {
        self.declaration_assignment_probes.get(range)
    }

    fn word_occurrence_ids_for_command(&self, id: CommandId) -> &[WordOccurrenceId] {
        self.word_occurrence_ids_by_command
            .get(id.index())
            .copied()
            .map_or(&[], |range| self.word_occurrence_ids.get(range))
    }

    fn word_spans(&self, range: IdRange<Span>) -> &[Span] {
        self.word_spans.get(range)
    }
}

#[derive(Clone, Copy)]
pub struct CommandFactRef<'facts, 'a> {
    fact: &'facts CommandFact<'a>,
    store: &'facts FactStore<'a>,
}

impl<'facts, 'a> CommandFactRef<'facts, 'a> {
    fn new(fact: &'facts CommandFact<'a>, store: &'facts FactStore<'a>) -> Self {
        Self { fact, store }
    }
}

impl<'facts, 'a> std::fmt::Debug for CommandFactRef<'facts, 'a> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fact.fmt(formatter)
    }
}

impl<'facts, 'a> std::ops::Deref for CommandFactRef<'facts, 'a> {
    type Target = CommandFact<'a>;

    fn deref(&self) -> &Self::Target {
        self.fact
    }
}

#[derive(Clone, Copy)]
pub struct CommandFacts<'facts, 'a> {
    commands: &'facts [CommandFact<'a>],
    store: &'facts FactStore<'a>,
}

impl<'facts, 'a> CommandFacts<'facts, 'a> {
    fn new(commands: &'facts [CommandFact<'a>], store: &'facts FactStore<'a>) -> Self {
        Self { commands, store }
    }

    pub fn len(self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(self) -> bool {
        self.commands.is_empty()
    }

    pub fn iter(self) -> CommandFactIter<'facts, 'a> {
        CommandFactIter {
            inner: self.commands.iter(),
            store: self.store,
        }
    }

    #[cfg(test)]
    pub(crate) fn raw(self) -> &'facts [CommandFact<'a>] {
        self.commands
    }

    pub fn get(self, index: usize) -> Option<CommandFactRef<'facts, 'a>> {
        self.commands
            .get(index)
            .map(|fact| CommandFactRef::new(fact, self.store))
    }

    pub fn find(self, id: CommandId) -> Option<CommandFactRef<'facts, 'a>> {
        self.commands
            .iter()
            .find(|fact| fact.id() == id)
            .map(|fact| CommandFactRef::new(fact, self.store))
    }

    pub fn first(self) -> Option<CommandFactRef<'facts, 'a>> {
        self.get(0)
    }

    pub fn last(self) -> Option<CommandFactRef<'facts, 'a>> {
        self.commands
            .last()
            .map(|fact| CommandFactRef::new(fact, self.store))
    }
}

impl<'facts, 'a> IntoIterator for CommandFacts<'facts, 'a> {
    type Item = CommandFactRef<'facts, 'a>;
    type IntoIter = CommandFactIter<'facts, 'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'facts, 'a> IntoIterator for &CommandFacts<'facts, 'a> {
    type Item = CommandFactRef<'facts, 'a>;
    type IntoIter = CommandFactIter<'facts, 'a>;

    fn into_iter(self) -> Self::IntoIter {
        (*self).iter()
    }
}

#[derive(Clone)]
pub struct CommandFactIter<'facts, 'a> {
    inner: std::slice::Iter<'facts, CommandFact<'a>>,
    store: &'facts FactStore<'a>,
}

impl<'facts, 'a> Iterator for CommandFactIter<'facts, 'a> {
    type Item = CommandFactRef<'facts, 'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|fact| CommandFactRef::new(fact, self.store))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'facts, 'a> DoubleEndedIterator for CommandFactIter<'facts, 'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|fact| CommandFactRef::new(fact, self.store))
    }
}

impl<'facts, 'a> ExactSizeIterator for CommandFactIter<'facts, 'a> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SudoFamilyInvoker {
    Sudo,
    Doas,
    Run0,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSpan {
    pub name: Name,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktickEscapedParameter {
    pub name: Option<Name>,
    pub diagnostic_span: Span,
    pub reference_span: Span,
    pub standalone_command_name: bool,
}
