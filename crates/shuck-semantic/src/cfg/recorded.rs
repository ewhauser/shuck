//! Compact command IR recorded during semantic traversal.
//!
//! Semantic traversal walks the parsed shell syntax once and records the pieces
//! CFG construction needs later. For example:
//!
//! ```sh
//! prepare && deploy | tee log
//! ```
//!
//! is stored as a logical-list command with ordered list items, and the pipeline
//! is stored with ordered segment records and per-segment scopes. Keeping that
//! compact representation here lets CFG construction, command topology, and
//! zsh option analysis share the same command identities and spans instead of
//! each reinterpreting parser nodes.

use rustc_hash::FxHashMap;
use shuck_ast::{BuiltinCommand, CaseTerminator, Command, CompoundCommand, Span};
use shuck_parser::ZshEmulationMode;
use std::marker::PhantomData;

use crate::cfg::FlowContext;
use crate::source_closure::SourcePathTemplate;
use crate::{BindingId, ScopeId, SpanKey};

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordedProgram {
    file_commands: RecordedCommandRange,
    function_bodies: FxHashMap<ScopeId, RecordedCommandRange>,
    commands: Vec<RecordedCommand>,
    command_sequence_items: Vec<CommandId>,
    statement_sequence_commands: Vec<StatementSequenceCommand>,
    isolated_regions: Vec<IsolatedRegion>,
    case_arms: Vec<RecordedCaseArm>,
    pipeline_segments: Vec<RecordedPipelineSegment>,
    list_items: Vec<RecordedListItem>,
    elif_branches: Vec<RecordedElifBranch>,
    command_info_records: Vec<RecordedCommandInfo>,
    pub(crate) command_infos: FxHashMap<SpanKey, RecordedCommandInfoId>,
    pub(crate) function_body_scopes: FxHashMap<BindingId, ScopeId>,
    pub(crate) call_command_spans: FxHashMap<SpanKey, Span>,
}

/// A statement-sequence item flattened out of structured syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementSequenceCommand {
    body_span: Span,
    stmt_span: Span,
}

impl StatementSequenceCommand {
    /// Returns the span of the command body without trailing statement punctuation.
    pub fn body_span(&self) -> Span {
        self.body_span
    }

    /// Returns the span of the full statement item.
    pub fn stmt_span(&self) -> Span {
        self.stmt_span
    }
}

/// Stable identifier for a semantic command in the recorded command stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub(crate) u32);

impl CommandId {
    /// Returns the zero-based index used by internal command-storage vectors.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RecordedCommandInfoId(u32);

impl RecordedCommandInfoId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// High-level command category recorded by semantic traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    /// A simple command.
    Simple,
    /// A builtin command with one of the tracked control-flow forms.
    Builtin(BuiltinCommandKind),
    /// A declaration builtin command.
    Decl,
    /// A binary command such as `[[ ... ]]` or `(( ... ))`.
    Binary,
    /// A compound command.
    Compound(CompoundCommandKind),
    /// A named function definition.
    Function,
    /// An anonymous function-like definition.
    AnonymousFunction,
}

impl CommandKind {
    /// Classifies a parsed AST command into a semantic command kind.
    pub fn from_command(command: &Command) -> Self {
        match command {
            Command::Simple(_) => Self::Simple,
            Command::Builtin(command) => Self::Builtin(BuiltinCommandKind::from_builtin(command)),
            Command::Decl(_) => Self::Decl,
            Command::Binary(_) => Self::Binary,
            Command::Compound(command) => {
                Self::Compound(CompoundCommandKind::from_compound(command))
            }
            Command::Function(_) => Self::Function,
            Command::AnonymousFunction(_) => Self::AnonymousFunction,
        }
    }
}

/// Builtin command kinds that affect control flow directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinCommandKind {
    /// `break`
    Break,
    /// `continue`
    Continue,
    /// `return`
    Return,
    /// `exit`
    Exit,
}

impl BuiltinCommandKind {
    fn from_builtin(command: &BuiltinCommand) -> Self {
        match command {
            BuiltinCommand::Break(_) => Self::Break,
            BuiltinCommand::Continue(_) => Self::Continue,
            BuiltinCommand::Return(_) => Self::Return,
            BuiltinCommand::Exit(_) => Self::Exit,
        }
    }
}

/// Compound command kinds tracked by semantic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompoundCommandKind {
    /// `if`
    If,
    /// `for`
    For,
    /// `repeat`
    Repeat,
    /// `foreach`
    Foreach,
    /// Arithmetic `for (( ...; ...; ... ))`
    ArithmeticFor,
    /// `while`
    While,
    /// `until`
    Until,
    /// `case`
    Case,
    /// `select`
    Select,
    /// Subshell `( ... )`
    Subshell,
    /// Brace group `{ ...; }`
    BraceGroup,
    /// Arithmetic command.
    Arithmetic,
    /// `time`
    Time,
    /// Conditional command such as `[[ ... ]]`.
    Conditional,
    /// `coproc`
    Coproc,
    /// `always`
    Always,
}

impl CompoundCommandKind {
    fn from_compound(command: &CompoundCommand) -> Self {
        match command {
            CompoundCommand::If(_) => Self::If,
            CompoundCommand::For(_) => Self::For,
            CompoundCommand::Repeat(_) => Self::Repeat,
            CompoundCommand::Foreach(_) => Self::Foreach,
            CompoundCommand::ArithmeticFor(_) => Self::ArithmeticFor,
            CompoundCommand::While(_) => Self::While,
            CompoundCommand::Until(_) => Self::Until,
            CompoundCommand::Case(_) => Self::Case,
            CompoundCommand::Select(_) => Self::Select,
            CompoundCommand::Subshell(_) => Self::Subshell,
            CompoundCommand::BraceGroup(_) => Self::BraceGroup,
            CompoundCommand::Arithmetic(_) => Self::Arithmetic,
            CompoundCommand::Time(_) => Self::Time,
            CompoundCommand::Conditional(_) => Self::Conditional,
            CompoundCommand::Coproc(_) => Self::Coproc,
            CompoundCommand::Always(_) => Self::Always,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordedRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> RecordedRange<T> {
    pub(crate) const fn empty() -> Self {
        Self {
            start: 0,
            len: 0,
            marker: PhantomData,
        }
    }

    pub(crate) fn new(start: usize, len: usize) -> Self {
        Self {
            start: match u32::try_from(start) {
                Ok(start) => start,
                Err(err) => panic!("recorded IR start fits in u32: {err}"),
            },
            len: match u32::try_from(len) {
                Ok(len) => len,
                Err(err) => panic!("recorded IR length fits in u32: {err}"),
            },
            marker: PhantomData,
        }
    }

    pub(crate) fn len(self) -> usize {
        self.len as usize
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn slice(self, store: &[T]) -> &[T] {
        let start = self.start as usize;
        &store[start..start + self.len()]
    }
}

impl<T> Default for RecordedRange<T> {
    fn default() -> Self {
        Self::empty()
    }
}

pub(crate) type RecordedCommandRange = RecordedRange<CommandId>;
pub(crate) type RecordedRegionRange = RecordedRange<IsolatedRegion>;
pub(crate) type RecordedCaseArmRange = RecordedRange<RecordedCaseArm>;
pub(crate) type RecordedPipelineSegmentRange = RecordedRange<RecordedPipelineSegment>;
pub(crate) type RecordedListItemRange = RecordedRange<RecordedListItem>;
pub(crate) type RecordedElifBranchRange = RecordedRange<RecordedElifBranch>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedCommand {
    pub(crate) span: Span,
    pub(crate) syntax_span: Span,
    pub(crate) syntax_kind: Option<CommandKind>,
    pub(crate) scope: Option<ScopeId>,
    pub(crate) flow_context: Option<FlowContext>,
    pub(crate) nested_regions: RecordedRegionRange,
    pub(crate) command_info: Option<RecordedCommandInfoId>,
    pub(crate) kind: RecordedCommandKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IsolatedRegion {
    pub(crate) scope: ScopeId,
    pub(crate) commands: RecordedCommandRange,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RecordedCommandKind {
    Linear,
    Break {
        depth: usize,
    },
    Continue {
        depth: usize,
    },
    Return,
    Exit,
    List {
        first: CommandId,
        rest: RecordedListItemRange,
    },
    If {
        condition: RecordedCommandRange,
        then_branch: RecordedCommandRange,
        elif_branches: RecordedElifBranchRange,
        else_branch: RecordedCommandRange,
    },
    While {
        condition: RecordedCommandRange,
        body: RecordedCommandRange,
    },
    Until {
        condition: RecordedCommandRange,
        body: RecordedCommandRange,
    },
    For {
        body: RecordedCommandRange,
    },
    Select {
        body: RecordedCommandRange,
    },
    ArithmeticFor {
        body: RecordedCommandRange,
    },
    Case {
        arms: RecordedCaseArmRange,
    },
    BraceGroup {
        body: RecordedCommandRange,
    },
    Always {
        body: RecordedCommandRange,
        always_body: RecordedCommandRange,
    },
    Subshell {
        body: RecordedCommandRange,
    },
    Pipeline {
        segments: RecordedPipelineSegmentRange,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedCaseArm {
    pub(crate) terminator: CaseTerminator,
    pub(crate) matches_anything: bool,
    pub(crate) commands: RecordedCommandRange,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedPipelineSegment {
    pub(crate) operator_before: Option<RecordedPipelineOperator>,
    pub(crate) scope: ScopeId,
    pub(crate) command: CommandId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedListItem {
    pub(crate) operator: RecordedListOperator,
    pub(crate) operator_span: Span,
    pub(crate) command: CommandId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedPipelineOperator {
    pub(crate) operator: RecordedPipelineOperatorKind,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordedPipelineOperatorKind {
    Pipe,
    PipeAll,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordedElifBranch {
    pub(crate) condition: RecordedCommandRange,
    pub(crate) body: RecordedCommandRange,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordedCommandInfo {
    pub(crate) static_callee: Option<compact_str::CompactString>,
    pub(crate) dynamic_name_span: Option<Span>,
    pub(crate) static_args: Box<[Option<String>]>,
    pub(crate) source_path_template: Option<SourcePathTemplate>,
    pub(crate) zsh_effects: Vec<RecordedZshCommandEffect>,
}

impl RecordedCommandInfo {
    pub(crate) fn is_empty(&self) -> bool {
        self.static_callee.is_none()
            && self.dynamic_name_span.is_none()
            && self.static_args.is_empty()
            && self.source_path_template.is_none()
            && self.zsh_effects.is_empty()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RecordedZshCommandEffect {
    Emulate {
        mode: ZshEmulationMode,
        local: bool,
    },
    EmulateUnknown {
        local: bool,
    },
    SetOptions {
        updates: Vec<RecordedZshOptionUpdate>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedZshOptionUpdate {
    Named { name: Box<str>, enable: bool },
    UnknownName,
    LocalOptions { enable: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordedListOperator {
    And,
    Or,
}

impl RecordedProgram {
    pub(crate) fn file_commands(&self) -> RecordedCommandRange {
        self.file_commands
    }

    pub(crate) fn set_file_commands(&mut self, commands: RecordedCommandRange) {
        self.file_commands = commands;
    }

    pub(crate) fn function_body(&self, scope: ScopeId) -> RecordedCommandRange {
        self.function_bodies
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn set_function_body(&mut self, scope: ScopeId, commands: RecordedCommandRange) {
        self.function_bodies.insert(scope, commands);
    }

    pub(crate) fn function_bodies(&self) -> &FxHashMap<ScopeId, RecordedCommandRange> {
        &self.function_bodies
    }

    pub(crate) fn command(&self, id: CommandId) -> &RecordedCommand {
        &self.commands[id.index()]
    }

    pub(crate) fn command_mut(&mut self, id: CommandId) -> &mut RecordedCommand {
        &mut self.commands[id.index()]
    }

    pub(crate) fn command_info(&self, id: RecordedCommandInfoId) -> &RecordedCommandInfo {
        &self.command_info_records[id.index()]
    }

    pub(crate) fn command_info_for_span(&self, span: Span) -> Option<&RecordedCommandInfo> {
        self.command_infos
            .get(&SpanKey::new(span))
            .map(|id| self.command_info(*id))
    }

    pub(crate) fn commands(&self) -> &[RecordedCommand] {
        &self.commands
    }

    pub(crate) fn commands_in(&self, range: RecordedCommandRange) -> &[CommandId] {
        range.slice(&self.command_sequence_items)
    }

    pub fn statement_sequence_commands(&self) -> &[StatementSequenceCommand] {
        &self.statement_sequence_commands
    }

    pub(crate) fn push_statement_sequence_command(&mut self, body_span: Span, stmt_span: Span) {
        self.statement_sequence_commands
            .push(StatementSequenceCommand {
                body_span,
                stmt_span,
            });
    }

    pub(crate) fn nested_regions(&self, range: RecordedRegionRange) -> &[IsolatedRegion] {
        range.slice(&self.isolated_regions)
    }

    pub(crate) fn case_arms(&self, range: RecordedCaseArmRange) -> &[RecordedCaseArm] {
        range.slice(&self.case_arms)
    }

    pub(crate) fn pipeline_segments(
        &self,
        range: RecordedPipelineSegmentRange,
    ) -> &[RecordedPipelineSegment] {
        range.slice(&self.pipeline_segments)
    }

    pub(crate) fn list_items(&self, range: RecordedListItemRange) -> &[RecordedListItem] {
        range.slice(&self.list_items)
    }

    pub(crate) fn elif_branches(&self, range: RecordedElifBranchRange) -> &[RecordedElifBranch] {
        range.slice(&self.elif_branches)
    }

    pub(crate) fn push_command(&mut self, command: RecordedCommand) -> CommandId {
        let id = CommandId(match u32::try_from(self.commands.len()) {
            Ok(id) => id,
            Err(err) => panic!("recorded command count fits in u32: {err}"),
        });
        self.commands.push(command);
        id
    }

    pub(crate) fn push_command_info(&mut self, info: RecordedCommandInfo) -> RecordedCommandInfoId {
        let id = RecordedCommandInfoId(match u32::try_from(self.command_info_records.len()) {
            Ok(id) => id,
            Err(err) => panic!("recorded command info count fits in u32: {err}"),
        });
        self.command_info_records.push(info);
        id
    }

    pub(crate) fn push_command_ids(&mut self, command_ids: Vec<CommandId>) -> RecordedCommandRange {
        push_range(&mut self.command_sequence_items, command_ids)
    }

    pub(crate) fn push_regions(&mut self, regions: Vec<IsolatedRegion>) -> RecordedRegionRange {
        push_range(&mut self.isolated_regions, regions)
    }

    pub(crate) fn push_case_arms(&mut self, arms: Vec<RecordedCaseArm>) -> RecordedCaseArmRange {
        push_range(&mut self.case_arms, arms)
    }

    pub(crate) fn push_pipeline_segments(
        &mut self,
        segments: Vec<RecordedPipelineSegment>,
    ) -> RecordedPipelineSegmentRange {
        push_range(&mut self.pipeline_segments, segments)
    }

    pub(crate) fn push_list_items(
        &mut self,
        list_items: Vec<RecordedListItem>,
    ) -> RecordedListItemRange {
        push_range(&mut self.list_items, list_items)
    }

    pub(crate) fn push_elif_branches(
        &mut self,
        elif_branches: Vec<RecordedElifBranch>,
    ) -> RecordedElifBranchRange {
        push_range(&mut self.elif_branches, elif_branches)
    }
}

fn push_range<T>(store: &mut Vec<T>, mut items: Vec<T>) -> RecordedRange<T> {
    if items.is_empty() {
        return RecordedRange::empty();
    }

    let start = store.len();
    store.append(&mut items);
    RecordedRange::new(start, store.len() - start)
}
