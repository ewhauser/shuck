use rustc_hash::FxHashMap;
use shuck_ast::{CaseTerminator, ListOperator, Span};

#[cfg(test)]
use shuck_ast::{
    Assignment, BuiltinCommand, CaseItem, Command, CommandList, CompoundCommand, ConditionalExpr,
    ContinueCommand, FunctionDef, Pattern, PatternPart, Redirect, Script, SelectCommand,
    SimpleCommand, WhileCommand, Word, WordPart,
};

use crate::{BindingId, ReferenceId, ScopeId, SpanKey};

#[cfg(test)]
use crate::Scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub(crate) u32);

impl BlockId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub commands: Vec<Span>,
    pub bindings: Vec<BindingId>,
    pub references: Vec<ReferenceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Sequential,
    ConditionalTrue,
    ConditionalFalse,
    LoopBack,
    LoopExit,
    CaseArm,
    CaseFallthrough,
    CaseContinue,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlowContext {
    pub in_function: bool,
    pub loop_depth: u32,
    pub in_subshell: bool,
    pub in_block: bool,
    pub exit_status_checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlowGraph {
    blocks: Vec<BasicBlock>,
    successors: FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
    predecessors: FxHashMap<BlockId, Vec<BlockId>>,
    entry: BlockId,
    exits: Vec<BlockId>,
    unreachable: Vec<BlockId>,
    pub(crate) scope_entries: FxHashMap<ScopeId, BlockId>,
    pub(crate) command_blocks: FxHashMap<SpanKey, Vec<BlockId>>,
    pub(crate) unreachable_causes: FxHashMap<BlockId, Span>,
}

impl ControlFlowGraph {
    pub fn blocks(&self) -> &[BasicBlock] {
        &self.blocks
    }

    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.index()]
    }

    pub fn successors(&self, id: BlockId) -> &[(BlockId, EdgeKind)] {
        self.successors.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn predecessors(&self, id: BlockId) -> &[BlockId] {
        self.predecessors.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn entry(&self) -> BlockId {
        self.entry
    }

    pub fn exits(&self) -> &[BlockId] {
        &self.exits
    }

    pub fn unreachable(&self) -> &[BlockId] {
        &self.unreachable
    }

    pub(crate) fn block_ids_for_span(&self, span: Span) -> &[BlockId] {
        self.command_blocks
            .get(&SpanKey::new(span))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn unreachable_cause(&self, id: BlockId) -> Option<Span> {
        self.unreachable_causes.get(&id).copied()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedProgram {
    pub(crate) file_commands: Vec<RecordedCommand>,
    pub(crate) function_bodies: FxHashMap<ScopeId, Vec<RecordedCommand>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedCommand {
    pub(crate) span: Span,
    pub(crate) nested_regions: Vec<IsolatedRegion>,
    pub(crate) kind: RecordedCommandKind,
}

#[derive(Debug, Clone)]
pub(crate) struct IsolatedRegion {
    pub(crate) scope: ScopeId,
    pub(crate) commands: Vec<RecordedCommand>,
}

#[derive(Debug, Clone)]
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
        first: Box<RecordedCommand>,
        rest: Vec<(ListOperator, RecordedCommand)>,
    },
    If {
        condition: Vec<RecordedCommand>,
        then_branch: Vec<RecordedCommand>,
        elif_branches: Vec<(Vec<RecordedCommand>, Vec<RecordedCommand>)>,
        else_branch: Vec<RecordedCommand>,
    },
    While {
        condition: Vec<RecordedCommand>,
        body: Vec<RecordedCommand>,
    },
    Until {
        condition: Vec<RecordedCommand>,
        body: Vec<RecordedCommand>,
    },
    For {
        body: Vec<RecordedCommand>,
    },
    Select {
        body: Vec<RecordedCommand>,
    },
    ArithmeticFor {
        body: Vec<RecordedCommand>,
    },
    Case {
        arms: Vec<RecordedCaseArm>,
    },
    BraceGroup {
        body: Vec<RecordedCommand>,
    },
    Subshell {
        body: Vec<RecordedCommand>,
    },
    Pipeline {
        segments: Vec<RecordedPipelineSegment>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedCaseArm {
    pub(crate) terminator: CaseTerminator,
    pub(crate) commands: Vec<RecordedCommand>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedPipelineSegment {
    pub(crate) scope: ScopeId,
    pub(crate) command: RecordedCommand,
}

impl RecordedProgram {
    #[cfg(test)]
    pub(crate) fn from_script(script: &Script, scopes: &[Scope]) -> Self {
        let mut function_bodies = FxHashMap::default();
        let file_commands = convert_commands(&script.commands, scopes, &mut function_bodies);
        Self {
            file_commands,
            function_bodies,
        }
    }
}

#[cfg(test)]
fn convert_commands(
    commands: &[Command],
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<RecordedCommand> {
    commands
        .iter()
        .map(|command| convert_command(command, scopes, function_bodies))
        .collect()
}

#[cfg(test)]
fn convert_command(
    command: &Command,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> RecordedCommand {
    match command {
        Command::Simple(command) => RecordedCommand {
            span: command.span,
            nested_regions: collect_regions_from_simple(command, scopes, function_bodies),
            kind: RecordedCommandKind::Linear,
        },
        Command::Decl(command) => RecordedCommand {
            span: command.span,
            nested_regions: collect_regions_from_decl(command, scopes, function_bodies),
            kind: RecordedCommandKind::Linear,
        },
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_builtin(command, scopes, function_bodies),
                kind: RecordedCommandKind::Break {
                    depth: depth_from_word(command.depth.as_ref()),
                },
            },
            BuiltinCommand::Continue(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_builtin(command, scopes, function_bodies),
                kind: RecordedCommandKind::Continue {
                    depth: depth_from_word(command.depth.as_ref()),
                },
            },
            BuiltinCommand::Return(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_builtin(command, scopes, function_bodies),
                kind: RecordedCommandKind::Return,
            },
            BuiltinCommand::Exit(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_builtin(command, scopes, function_bodies),
                kind: RecordedCommandKind::Exit,
            },
        },
        Command::Pipeline(command) => RecordedCommand {
            span: command.span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::Pipeline {
                segments: command
                    .commands
                    .iter()
                    .map(|segment| RecordedPipelineSegment {
                        scope: scope_for_span(scopes, command_span(segment)),
                        command: convert_command(segment, scopes, function_bodies),
                    })
                    .collect(),
            },
        },
        Command::List(CommandList { first, rest, span }) => RecordedCommand {
            span: *span,
            nested_regions: Vec::new(),
            kind: RecordedCommandKind::List {
                first: Box::new(convert_command(first, scopes, function_bodies)),
                rest: rest
                    .iter()
                    .map(|item| {
                        (
                            item.operator,
                            convert_command(&item.command, scopes, function_bodies),
                        )
                    })
                    .collect(),
            },
        },
        Command::Compound(command, redirects) => match command {
            CompoundCommand::If(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::If {
                    condition: convert_commands(&command.condition, scopes, function_bodies),
                    then_branch: convert_commands(&command.then_branch, scopes, function_bodies),
                    elif_branches: command
                        .elif_branches
                        .iter()
                        .map(|(condition, body)| {
                            (
                                convert_commands(condition, scopes, function_bodies),
                                convert_commands(body, scopes, function_bodies),
                            )
                        })
                        .collect(),
                    else_branch: command
                        .else_branch
                        .as_deref()
                        .map(|branch| convert_commands(branch, scopes, function_bodies))
                        .unwrap_or_default(),
                },
            },
            CompoundCommand::While(WhileCommand {
                condition,
                body,
                span,
            }) => RecordedCommand {
                span: *span,
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::While {
                    condition: convert_commands(condition, scopes, function_bodies),
                    body: convert_commands(body, scopes, function_bodies),
                },
            },
            CompoundCommand::Until(shuck_ast::UntilCommand {
                condition,
                body,
                span,
            }) => RecordedCommand {
                span: *span,
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::Until {
                    condition: convert_commands(condition, scopes, function_bodies),
                    body: convert_commands(body, scopes, function_bodies),
                },
            },
            CompoundCommand::For(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_for(
                    command,
                    redirects,
                    scopes,
                    function_bodies,
                ),
                kind: RecordedCommandKind::For {
                    body: convert_commands(&command.body, scopes, function_bodies),
                },
            },
            CompoundCommand::Select(SelectCommand {
                words, body, span, ..
            }) => {
                let mut nested = collect_regions_from_words(words, scopes, function_bodies);
                nested.extend(collect_regions_from_redirects(
                    redirects,
                    scopes,
                    function_bodies,
                ));
                RecordedCommand {
                    span: *span,
                    nested_regions: nested,
                    kind: RecordedCommandKind::Select {
                        body: convert_commands(body, scopes, function_bodies),
                    },
                }
            }
            CompoundCommand::ArithmeticFor(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::ArithmeticFor {
                    body: convert_commands(&command.body, scopes, function_bodies),
                },
            },
            CompoundCommand::Case(command) => {
                let mut nested = collect_regions_from_word(&command.word, scopes, function_bodies);
                nested.extend(collect_regions_from_redirects(
                    redirects,
                    scopes,
                    function_bodies,
                ));
                RecordedCommand {
                    span: command.span,
                    nested_regions: nested,
                    kind: RecordedCommandKind::Case {
                        arms: command
                            .cases
                            .iter()
                            .map(
                                |CaseItem {
                                     patterns,
                                     commands,
                                     terminator,
                                 }| {
                                    let pattern_regions = collect_regions_from_patterns(
                                        patterns,
                                        scopes,
                                        function_bodies,
                                    );
                                    if !pattern_regions.is_empty() {
                                        // Patterns execute as part of the case command.
                                        // Fold them into the first body command by attaching to
                                        // an empty leading command if needed.
                                        let mut recorded =
                                            convert_commands(commands, scopes, function_bodies);
                                        if let Some(first) = recorded.first_mut() {
                                            first.nested_regions.splice(0..0, pattern_regions);
                                        } else {
                                            recorded.push(RecordedCommand {
                                                span: command.span,
                                                nested_regions: pattern_regions,
                                                kind: RecordedCommandKind::Linear,
                                            });
                                        }
                                        RecordedCaseArm {
                                            terminator: *terminator,
                                            commands: recorded,
                                        }
                                    } else {
                                        RecordedCaseArm {
                                            terminator: *terminator,
                                            commands: convert_commands(
                                                commands,
                                                scopes,
                                                function_bodies,
                                            ),
                                        }
                                    }
                                },
                            )
                            .collect(),
                    },
                }
            }
            CompoundCommand::BraceGroup(commands) => RecordedCommand {
                span: command_span_from_compound(command),
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::BraceGroup {
                    body: convert_commands(commands, scopes, function_bodies),
                },
            },
            CompoundCommand::Subshell(commands) => RecordedCommand {
                span: command_span_from_compound(command),
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::Subshell {
                    body: convert_commands(commands, scopes, function_bodies),
                },
            },
            CompoundCommand::Arithmetic(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_redirects(redirects, scopes, function_bodies),
                kind: RecordedCommandKind::Linear,
            },
            CompoundCommand::Conditional(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_conditional(
                    &command.expression,
                    scopes,
                    function_bodies,
                ),
                kind: RecordedCommandKind::Linear,
            },
            CompoundCommand::Time(command) => RecordedCommand {
                span: command.span,
                nested_regions: command
                    .command
                    .as_deref()
                    .map(|command| collect_regions_from_command(command, scopes, function_bodies))
                    .unwrap_or_default(),
                kind: RecordedCommandKind::Linear,
            },
            CompoundCommand::Coproc(command) => RecordedCommand {
                span: command.span,
                nested_regions: collect_regions_from_command(
                    &command.body,
                    scopes,
                    function_bodies,
                ),
                kind: RecordedCommandKind::Linear,
            },
        },
        Command::Function(FunctionDef { body, span, .. }) => {
            let function_scope = scope_for_span(scopes, body_span(body));
            let commands = function_body_commands(body, scopes, function_bodies);
            function_bodies.insert(function_scope, commands);
            RecordedCommand {
                span: *span,
                nested_regions: Vec::new(),
                kind: RecordedCommandKind::Linear,
            }
        }
    }
}

#[cfg(test)]
fn function_body_commands(
    body: &Command,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<RecordedCommand> {
    match body {
        Command::Compound(CompoundCommand::BraceGroup(commands), _) => {
            convert_commands(commands, scopes, function_bodies)
        }
        _ => vec![convert_command(body, scopes, function_bodies)],
    }
}

#[cfg(test)]
fn collect_regions_from_command(
    command: &Command,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    match command {
        Command::Simple(command) => collect_regions_from_simple(command, scopes, function_bodies),
        Command::Decl(command) => collect_regions_from_decl(command, scopes, function_bodies),
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                collect_regions_from_builtin(command, scopes, function_bodies)
            }
            BuiltinCommand::Continue(command) => {
                collect_regions_from_builtin(command, scopes, function_bodies)
            }
            BuiltinCommand::Return(command) => {
                collect_regions_from_builtin(command, scopes, function_bodies)
            }
            BuiltinCommand::Exit(command) => {
                collect_regions_from_builtin(command, scopes, function_bodies)
            }
        },
        Command::Pipeline(command) => command
            .commands
            .iter()
            .flat_map(|command| collect_regions_from_command(command, scopes, function_bodies))
            .collect(),
        Command::List(CommandList { first, rest, .. }) => {
            let mut regions = collect_regions_from_command(first, scopes, function_bodies);
            for item in rest {
                regions.extend(collect_regions_from_command(
                    &item.command,
                    scopes,
                    function_bodies,
                ));
            }
            regions
        }
        Command::Compound(command, redirects) => {
            let mut regions = collect_regions_from_redirects(redirects, scopes, function_bodies);
            match command {
                CompoundCommand::If(command) => {
                    for condition in &command.condition {
                        regions.extend(collect_regions_from_command(
                            condition,
                            scopes,
                            function_bodies,
                        ));
                    }
                    for command in &command.then_branch {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                    for (condition, body) in &command.elif_branches {
                        for command in condition {
                            regions.extend(collect_regions_from_command(
                                command,
                                scopes,
                                function_bodies,
                            ));
                        }
                        for command in body {
                            regions.extend(collect_regions_from_command(
                                command,
                                scopes,
                                function_bodies,
                            ));
                        }
                    }
                    if let Some(body) = &command.else_branch {
                        for command in body {
                            regions.extend(collect_regions_from_command(
                                command,
                                scopes,
                                function_bodies,
                            ));
                        }
                    }
                }
                CompoundCommand::For(command) => {
                    regions.extend(collect_regions_from_words(
                        &command.words.clone().unwrap_or_default(),
                        scopes,
                        function_bodies,
                    ));
                    for command in &command.body {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::ArithmeticFor(command) => {
                    for command in &command.body {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::While(command) => {
                    for condition in &command.condition {
                        regions.extend(collect_regions_from_command(
                            condition,
                            scopes,
                            function_bodies,
                        ));
                    }
                    for command in &command.body {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::Until(command) => {
                    for condition in &command.condition {
                        regions.extend(collect_regions_from_command(
                            condition,
                            scopes,
                            function_bodies,
                        ));
                    }
                    for command in &command.body {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::Case(command) => {
                    regions.extend(collect_regions_from_word(
                        &command.word,
                        scopes,
                        function_bodies,
                    ));
                    for case in &command.cases {
                        regions.extend(collect_regions_from_patterns(
                            &case.patterns,
                            scopes,
                            function_bodies,
                        ));
                        for command in &case.commands {
                            regions.extend(collect_regions_from_command(
                                command,
                                scopes,
                                function_bodies,
                            ));
                        }
                    }
                }
                CompoundCommand::Select(command) => {
                    regions.extend(collect_regions_from_words(
                        &command.words,
                        scopes,
                        function_bodies,
                    ));
                    for command in &command.body {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                    for command in commands {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::Arithmetic(_) => {}
                CompoundCommand::Time(command) => {
                    if let Some(command) = &command.command {
                        regions.extend(collect_regions_from_command(
                            command,
                            scopes,
                            function_bodies,
                        ));
                    }
                }
                CompoundCommand::Conditional(command) => {
                    regions.extend(collect_regions_from_conditional(
                        &command.expression,
                        scopes,
                        function_bodies,
                    ));
                }
                CompoundCommand::Coproc(command) => {
                    regions.extend(collect_regions_from_command(
                        &command.body,
                        scopes,
                        function_bodies,
                    ));
                }
            }
            regions
        }
        Command::Function(FunctionDef { body, .. }) => {
            collect_regions_from_command(body, scopes, function_bodies)
        }
    }
}

#[cfg(test)]
fn collect_regions_from_simple(
    command: &SimpleCommand,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions = collect_regions_from_word(&command.name, scopes, function_bodies);
    regions.extend(collect_regions_from_words(
        &command.args,
        scopes,
        function_bodies,
    ));
    regions.extend(collect_regions_from_redirects(
        &command.redirects,
        scopes,
        function_bodies,
    ));
    regions.extend(collect_regions_from_assignments(
        &command.assignments,
        scopes,
        function_bodies,
    ));
    regions
}

#[cfg(test)]
fn collect_regions_from_decl(
    command: &shuck_ast::DeclClause,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions = collect_regions_from_redirects(&command.redirects, scopes, function_bodies);
    regions.extend(collect_regions_from_assignments(
        &command.assignments,
        scopes,
        function_bodies,
    ));
    for operand in &command.operands {
        match operand {
            shuck_ast::DeclOperand::Flag(word) | shuck_ast::DeclOperand::Dynamic(word) => {
                regions.extend(collect_regions_from_word(word, scopes, function_bodies));
            }
            shuck_ast::DeclOperand::Name(_) => {}
            shuck_ast::DeclOperand::Assignment(assignment) => {
                regions.extend(collect_regions_from_assignments(
                    std::slice::from_ref(assignment),
                    scopes,
                    function_bodies,
                ));
            }
        }
    }
    regions
}

#[cfg(test)]
fn collect_regions_from_builtin(
    command: &impl BuiltinRegionSource,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions =
        collect_regions_from_assignments(command.assignments(), scopes, function_bodies);
    regions.extend(collect_regions_from_redirects(
        command.redirects(),
        scopes,
        function_bodies,
    ));
    if let Some(word) = command.primary_word() {
        regions.extend(collect_regions_from_word(word, scopes, function_bodies));
    }
    regions.extend(collect_regions_from_words(
        command.extra_words(),
        scopes,
        function_bodies,
    ));
    regions
}

#[cfg(test)]
trait BuiltinRegionSource {
    fn assignments(&self) -> &[Assignment];
    fn redirects(&self) -> &[Redirect];
    fn primary_word(&self) -> Option<&Word>;
    fn extra_words(&self) -> &[Word];
}

#[cfg(test)]
impl BuiltinRegionSource for shuck_ast::BreakCommand {
    fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
    fn redirects(&self) -> &[Redirect] {
        &self.redirects
    }
    fn primary_word(&self) -> Option<&Word> {
        self.depth.as_ref()
    }
    fn extra_words(&self) -> &[Word] {
        &self.extra_args
    }
}

#[cfg(test)]
impl BuiltinRegionSource for ContinueCommand {
    fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
    fn redirects(&self) -> &[Redirect] {
        &self.redirects
    }
    fn primary_word(&self) -> Option<&Word> {
        self.depth.as_ref()
    }
    fn extra_words(&self) -> &[Word] {
        &self.extra_args
    }
}

#[cfg(test)]
impl BuiltinRegionSource for shuck_ast::ReturnCommand {
    fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
    fn redirects(&self) -> &[Redirect] {
        &self.redirects
    }
    fn primary_word(&self) -> Option<&Word> {
        self.code.as_ref()
    }
    fn extra_words(&self) -> &[Word] {
        &self.extra_args
    }
}

#[cfg(test)]
impl BuiltinRegionSource for shuck_ast::ExitCommand {
    fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
    fn redirects(&self) -> &[Redirect] {
        &self.redirects
    }
    fn primary_word(&self) -> Option<&Word> {
        self.code.as_ref()
    }
    fn extra_words(&self) -> &[Word] {
        &self.extra_args
    }
}

#[cfg(test)]
fn collect_regions_from_for(
    command: &shuck_ast::ForCommand,
    redirects: &[Redirect],
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions = command
        .words
        .as_deref()
        .map(|words| collect_regions_from_words(words, scopes, function_bodies))
        .unwrap_or_default();
    regions.extend(collect_regions_from_redirects(
        redirects,
        scopes,
        function_bodies,
    ));
    regions
}

#[cfg(test)]
fn collect_regions_from_assignments(
    assignments: &[Assignment],
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions = Vec::new();
    for assignment in assignments {
        match &assignment.value {
            shuck_ast::AssignmentValue::Scalar(word) => {
                regions.extend(collect_regions_from_word(word, scopes, function_bodies));
            }
            shuck_ast::AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        shuck_ast::ArrayElem::Sequential(word) => {
                            regions.extend(collect_regions_from_word(
                                word,
                                scopes,
                                function_bodies,
                            ));
                        }
                        shuck_ast::ArrayElem::Keyed { value, .. }
                        | shuck_ast::ArrayElem::KeyedAppend { value, .. } => {
                            regions.extend(collect_regions_from_word(
                                value,
                                scopes,
                                function_bodies,
                            ));
                        }
                    }
                }
            }
        }
    }
    regions
}

#[cfg(test)]
fn collect_regions_from_redirects(
    redirects: &[Redirect],
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    redirects
        .iter()
        .flat_map(|redirect| {
            collect_regions_from_word(
                match redirect.word_target() {
                    Some(word) => word,
                    None => &redirect.heredoc().expect("expected heredoc redirect").body,
                },
                scopes,
                function_bodies,
            )
        })
        .collect()
}

#[cfg(test)]
fn collect_regions_from_words(
    words: &[Word],
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    words
        .iter()
        .flat_map(|word| collect_regions_from_word(word, scopes, function_bodies))
        .collect()
}

#[cfg(test)]
fn collect_regions_from_patterns(
    patterns: &[Pattern],
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    patterns
        .iter()
        .flat_map(|pattern| collect_regions_from_pattern(pattern, scopes, function_bodies))
        .collect()
}

#[cfg(test)]
fn collect_regions_from_conditional(
    expression: &ConditionalExpr,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    match expression {
        ConditionalExpr::Binary(expr) => {
            let mut regions = collect_regions_from_conditional(&expr.left, scopes, function_bodies);
            regions.extend(collect_regions_from_conditional(
                &expr.right,
                scopes,
                function_bodies,
            ));
            regions
        }
        ConditionalExpr::Unary(expr) => {
            collect_regions_from_conditional(&expr.expr, scopes, function_bodies)
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_regions_from_conditional(&expr.expr, scopes, function_bodies)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_regions_from_word(word, scopes, function_bodies)
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_regions_from_pattern(pattern, scopes, function_bodies)
        }
        ConditionalExpr::VarRef(_) => Vec::new(),
    }
}

#[cfg(test)]
fn collect_regions_from_pattern(
    pattern: &Pattern,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions = Vec::new();
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                regions.extend(collect_regions_from_patterns(
                    patterns,
                    scopes,
                    function_bodies,
                ));
            }
            PatternPart::Word(word) => {
                regions.extend(collect_regions_from_word(word, scopes, function_bodies));
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
    regions
}

#[cfg(test)]
fn collect_regions_from_word(
    word: &Word,
    scopes: &[Scope],
    function_bodies: &mut FxHashMap<ScopeId, Vec<RecordedCommand>>,
) -> Vec<IsolatedRegion> {
    let mut regions = Vec::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::CommandSubstitution { commands, .. }
            | WordPart::ProcessSubstitution { commands, .. } => {
                regions.push(IsolatedRegion {
                    scope: scope_for_span(scopes, span),
                    commands: convert_commands(commands, scopes, function_bodies),
                });
            }
            _ => {}
        }
    }
    regions
}

#[cfg(test)]
fn scope_for_span(scopes: &[Scope], span: Span) -> ScopeId {
    scopes
        .iter()
        .filter(|scope| {
            span.start.offset >= scope.span.start.offset && span.end.offset <= scope.span.end.offset
        })
        .min_by_key(|scope| scope.span.end.offset - scope.span.start.offset)
        .map(|scope| scope.id)
        .unwrap_or(ScopeId(0))
}

#[cfg(test)]
fn depth_from_word(word: Option<&Word>) -> usize {
    word.and_then(single_literal_word)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|depth| *depth > 0)
        .unwrap_or(1)
}

#[cfg(test)]
fn single_literal_word(word: &Word) -> Option<&str> {
    match word.parts.as_slice() {
        [part] => match &part.kind {
            WordPart::Literal(shuck_ast::LiteralText::Owned(text)) => Some(text.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(BuiltinCommand::Break(command)) => command.span,
        Command::Builtin(BuiltinCommand::Continue(command)) => command.span,
        Command::Builtin(BuiltinCommand::Return(command)) => command.span,
        Command::Builtin(BuiltinCommand::Exit(command)) => command.span,
        Command::Decl(command) => command.span,
        Command::Pipeline(command) => command.span,
        Command::List(command) => command.span,
        Command::Compound(command, _) => command_span_from_compound(command),
        Command::Function(command) => command.span,
    }
}

#[cfg(test)]
fn body_span(command: &Command) -> Span {
    match command {
        Command::Compound(CompoundCommand::BraceGroup(commands), _) => commands
            .first()
            .map(command_span)
            .zip(commands.last().map(command_span))
            .map(|(start, end)| start.merge(end))
            .unwrap_or(command_span(command)),
        _ => command_span(command),
    }
}

#[cfg(test)]
fn command_span_from_compound(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .first()
            .map(command_span)
            .zip(commands.last().map(command_span))
            .map(|(start, end)| start.merge(end))
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
    }
}

struct SequenceResult {
    entry: Option<BlockId>,
    exits: Vec<BlockId>,
}

#[derive(Clone, Copy)]
struct LoopTarget {
    continue_target: BlockId,
    break_target: BlockId,
}

struct GraphBuilder<'a> {
    command_bindings: &'a FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: &'a FxHashMap<SpanKey, Vec<ReferenceId>>,
    blocks: Vec<BasicBlock>,
    successors: FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
    command_blocks: FxHashMap<SpanKey, Vec<BlockId>>,
    unreachable_causes: FxHashMap<BlockId, Span>,
    scope_entries: FxHashMap<ScopeId, BlockId>,
}

pub(crate) fn build_control_flow_graph(
    program: &RecordedProgram,
    command_bindings: &FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: &FxHashMap<SpanKey, Vec<ReferenceId>>,
) -> ControlFlowGraph {
    let mut builder = GraphBuilder {
        command_bindings,
        command_references,
        blocks: Vec::new(),
        successors: FxHashMap::default(),
        command_blocks: FxHashMap::default(),
        unreachable_causes: FxHashMap::default(),
        scope_entries: FxHashMap::default(),
    };

    let file = builder.build_sequence(&program.file_commands, &[]);
    let entry = file.entry.unwrap_or_else(|| builder.empty_block());
    builder.scope_entries.insert(ScopeId(0), entry);

    let mut exits = if file.exits.is_empty() {
        vec![entry]
    } else {
        file.exits
    };

    for (scope, commands) in &program.function_bodies {
        let function = builder.build_sequence(commands, &[]);
        let function_entry = function.entry.unwrap_or_else(|| builder.empty_block());
        builder.scope_entries.insert(*scope, function_entry);
        if function.exits.is_empty() {
            exits.push(function_entry);
        } else {
            exits.extend(function.exits);
        }
    }

    let predecessors = derive_predecessors(&builder.successors);
    let unreachable =
        compute_unreachable(&builder.blocks, &builder.scope_entries, &builder.successors);

    ControlFlowGraph {
        blocks: builder.blocks,
        successors: builder.successors,
        predecessors,
        entry,
        exits,
        unreachable,
        scope_entries: builder.scope_entries,
        command_blocks: builder.command_blocks,
        unreachable_causes: builder.unreachable_causes,
    }
}

impl<'a> GraphBuilder<'a> {
    fn build_sequence(
        &mut self,
        commands: &[RecordedCommand],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let mut entry = None;
        let mut pending = Vec::new();
        let mut unreachable_cause = None;

        for command in commands {
            let start = self.blocks.len();
            let sequence = self.build_command(command, loops);
            if entry.is_none() {
                entry = sequence.entry;
            }
            if let Some(command_entry) = sequence.entry {
                if let Some(cause) = unreachable_cause {
                    for block in &self.blocks[start..] {
                        self.unreachable_causes.insert(block.id, cause);
                    }
                } else {
                    for block in &pending {
                        self.add_edge(*block, command_entry, EdgeKind::Sequential);
                    }
                }
            }

            if sequence.exits.is_empty() {
                pending.clear();
                unreachable_cause = Some(command.span);
            } else {
                pending = sequence.exits;
                unreachable_cause = None;
            }
        }

        SequenceResult {
            entry,
            exits: pending,
        }
    }

    fn build_command(&mut self, command: &RecordedCommand, loops: &[LoopTarget]) -> SequenceResult {
        match &command.kind {
            RecordedCommandKind::Linear => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, &command.nested_regions, loops);
                SequenceResult {
                    entry: Some(block),
                    exits: vec![block],
                }
            }
            RecordedCommandKind::Break { depth } => {
                let block = self.command_block(command.span);
                if let Some(target) = resolve_break_target(loops, *depth) {
                    self.add_edge(block, target.break_target, EdgeKind::LoopExit);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: Vec::new(),
                }
            }
            RecordedCommandKind::Continue { depth } => {
                let block = self.command_block(command.span);
                if let Some(target) = resolve_break_target(loops, *depth) {
                    self.add_edge(block, target.continue_target, EdgeKind::LoopBack);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: Vec::new(),
                }
            }
            RecordedCommandKind::Return | RecordedCommandKind::Exit => {
                let block = self.command_block(command.span);
                SequenceResult {
                    entry: Some(block),
                    exits: Vec::new(),
                }
            }
            RecordedCommandKind::List { first, rest } => {
                self.build_list(command, first, rest, loops)
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => self.build_if(
                command,
                condition,
                then_branch,
                elif_branches,
                else_branch,
                loops,
            ),
            RecordedCommandKind::While { condition, body } => {
                self.build_while_like(command, condition, body, loops, true)
            }
            RecordedCommandKind::Until { condition, body } => {
                self.build_while_like(command, condition, body, loops, false)
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body } => {
                self.build_loop_command(command, body, loops)
            }
            RecordedCommandKind::Case { arms } => self.build_case(command, arms, loops),
            RecordedCommandKind::BraceGroup { body } => self.build_sequence(body, loops),
            RecordedCommandKind::Subshell { body, .. } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, &command.nested_regions, loops);
                let body_sequence = self.build_sequence(body, loops);
                if let Some(body_entry) = body_sequence.entry {
                    self.add_edge(block, body_entry, EdgeKind::Sequential);
                }
                SequenceResult {
                    entry: Some(block),
                    exits: vec![block],
                }
            }
            RecordedCommandKind::Pipeline { segments } => {
                let block = self.command_block(command.span);
                self.attach_nested_regions(block, &command.nested_regions, loops);
                for segment in segments {
                    let sequence = self.build_command(&segment.command, loops);
                    if let Some(segment_entry) = sequence.entry {
                        self.scope_entries
                            .entry(segment.scope)
                            .or_insert(segment_entry);
                        self.add_edge(block, segment_entry, EdgeKind::Sequential);
                    }
                }
                SequenceResult {
                    entry: Some(block),
                    exits: vec![block],
                }
            }
        }
    }

    fn build_list(
        &mut self,
        command: &RecordedCommand,
        first: &RecordedCommand,
        rest: &[(ListOperator, RecordedCommand)],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let mut current = self.build_command(first, loops);
        let entry = current.entry;
        let mut shortcut_exits = Vec::new();

        for (op, command) in rest {
            let next = self.build_command(command, loops);
            if let Some(next_entry) = next.entry {
                for exit in &current.exits {
                    let edge = match op {
                        ListOperator::And => EdgeKind::ConditionalTrue,
                        ListOperator::Or => EdgeKind::ConditionalFalse,
                        ListOperator::Semicolon | ListOperator::Background => EdgeKind::Sequential,
                    };
                    self.add_edge(*exit, next_entry, edge);
                }
            }

            if matches!(op, ListOperator::And | ListOperator::Or) {
                shortcut_exits.extend(current.exits.clone());
            }

            current = if matches!(op, ListOperator::Semicolon | ListOperator::Background) {
                next
            } else {
                SequenceResult {
                    entry,
                    exits: next.exits,
                }
            };
        }

        let mut exits = current.exits;
        exits.extend(shortcut_exits);
        self.attach_nested_regions_from_command(command);
        SequenceResult { entry, exits }
    }

    fn build_if(
        &mut self,
        command: &RecordedCommand,
        condition: &[RecordedCommand],
        then_branch: &[RecordedCommand],
        elif_branches: &[(Vec<RecordedCommand>, Vec<RecordedCommand>)],
        else_branch: &[RecordedCommand],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let condition_seq = self.build_sequence(condition, loops);
        let entry = condition_seq.entry.or_else(|| Some(self.empty_block()));
        let mut false_exits = condition_seq.exits.clone();

        let then_seq = self.build_sequence(then_branch, loops);
        if let (Some(cond_entry), Some(then_entry)) = (entry, then_seq.entry) {
            for exit in &condition_seq.exits {
                self.add_edge(*exit, then_entry, EdgeKind::ConditionalTrue);
            }
            if condition_seq.exits.is_empty() {
                self.add_edge(cond_entry, then_entry, EdgeKind::ConditionalTrue);
            }
        }

        let mut branch_exits = then_seq.exits;

        for (elif_condition, elif_body) in elif_branches {
            let elif_cond = self.build_sequence(elif_condition, loops);
            if let Some(elif_entry) = elif_cond.entry {
                for exit in &false_exits {
                    self.add_edge(*exit, elif_entry, EdgeKind::ConditionalFalse);
                }
            }

            let elif_body_seq = self.build_sequence(elif_body, loops);
            if let Some(body_entry) = elif_body_seq.entry {
                for exit in &elif_cond.exits {
                    self.add_edge(*exit, body_entry, EdgeKind::ConditionalTrue);
                }
            }

            false_exits = elif_cond.exits;
            branch_exits.extend(elif_body_seq.exits);
        }

        let else_seq = self.build_sequence(else_branch, loops);
        if let Some(else_entry) = else_seq.entry {
            for exit in &false_exits {
                self.add_edge(*exit, else_entry, EdgeKind::ConditionalFalse);
            }
            branch_exits.extend(else_seq.exits);
        } else {
            branch_exits.extend(false_exits);
        }

        self.attach_nested_regions_from_command(command);
        SequenceResult {
            entry,
            exits: branch_exits,
        }
    }

    fn build_while_like(
        &mut self,
        command: &RecordedCommand,
        condition: &[RecordedCommand],
        body: &[RecordedCommand],
        loops: &[LoopTarget],
        while_sense: bool,
    ) -> SequenceResult {
        let exit_block = self.empty_block();
        let condition_seq = self.build_sequence(condition, loops);
        let entry = condition_seq.entry.or_else(|| Some(self.empty_block()));
        let continue_target = condition_seq.entry.unwrap_or(exit_block);
        let mut next_loops = loops.to_vec();
        next_loops.push(LoopTarget {
            continue_target,
            break_target: exit_block,
        });
        let body_seq = self.build_sequence(body, &next_loops);

        if let Some(body_entry) = body_seq.entry {
            for exit in &condition_seq.exits {
                self.add_edge(
                    *exit,
                    body_entry,
                    if while_sense {
                        EdgeKind::ConditionalTrue
                    } else {
                        EdgeKind::ConditionalFalse
                    },
                );
                self.add_edge(
                    *exit,
                    exit_block,
                    if while_sense {
                        EdgeKind::ConditionalFalse
                    } else {
                        EdgeKind::ConditionalTrue
                    },
                );
            }
            for exit in &body_seq.exits {
                self.add_edge(*exit, continue_target, EdgeKind::LoopBack);
            }
        } else {
            for exit in &condition_seq.exits {
                self.add_edge(*exit, exit_block, EdgeKind::LoopExit);
            }
        }

        self.attach_nested_regions_from_command(command);
        SequenceResult {
            entry,
            exits: vec![exit_block],
        }
    }

    fn build_loop_command(
        &mut self,
        command: &RecordedCommand,
        body: &[RecordedCommand],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let header = self.command_block(command.span);
        self.attach_nested_regions(header, &command.nested_regions, loops);
        let exit_block = self.empty_block();
        let mut next_loops = loops.to_vec();
        next_loops.push(LoopTarget {
            continue_target: header,
            break_target: exit_block,
        });
        let body_seq = self.build_sequence(body, &next_loops);
        if let Some(body_entry) = body_seq.entry {
            self.add_edge(header, body_entry, EdgeKind::ConditionalTrue);
            self.add_edge(header, exit_block, EdgeKind::ConditionalFalse);
            for exit in &body_seq.exits {
                self.add_edge(*exit, header, EdgeKind::LoopBack);
            }
        } else {
            self.add_edge(header, exit_block, EdgeKind::LoopExit);
        }
        SequenceResult {
            entry: Some(header),
            exits: vec![exit_block],
        }
    }

    fn build_case(
        &mut self,
        command: &RecordedCommand,
        arms: &[RecordedCaseArm],
        loops: &[LoopTarget],
    ) -> SequenceResult {
        let head = self.command_block(command.span);
        self.attach_nested_regions(head, &command.nested_regions, loops);
        let exit_block = self.empty_block();
        let mut fallthrough_from = Vec::new();

        for arm in arms {
            let arm_seq = self.build_sequence(&arm.commands, loops);
            if let Some(arm_entry) = arm_seq.entry {
                self.add_edge(head, arm_entry, EdgeKind::CaseArm);
                for block in &fallthrough_from {
                    self.add_edge(*block, arm_entry, EdgeKind::CaseFallthrough);
                }
            }

            match arm.terminator {
                CaseTerminator::Break => {
                    for exit in &arm_seq.exits {
                        self.add_edge(*exit, exit_block, EdgeKind::LoopExit);
                    }
                    fallthrough_from.clear();
                }
                CaseTerminator::FallThrough => {
                    fallthrough_from = arm_seq.exits.clone();
                }
                CaseTerminator::Continue => {
                    fallthrough_from = arm_seq.exits.clone();
                    for block in &fallthrough_from {
                        self.successors
                            .entry(*block)
                            .or_default()
                            .push((head, EdgeKind::CaseContinue));
                    }
                }
            }
        }

        if arms.is_empty() {
            self.add_edge(head, exit_block, EdgeKind::Sequential);
        }

        SequenceResult {
            entry: Some(head),
            exits: vec![exit_block],
        }
    }

    fn attach_nested_regions(
        &mut self,
        block: BlockId,
        regions: &[IsolatedRegion],
        loops: &[LoopTarget],
    ) {
        for region in regions {
            let sequence = self.build_sequence(&region.commands, loops);
            if let Some(entry) = sequence.entry {
                self.scope_entries.entry(region.scope).or_insert(entry);
                self.add_edge(block, entry, EdgeKind::Sequential);
            }
        }
    }

    fn attach_nested_regions_from_command(&mut self, command: &RecordedCommand) {
        if command.nested_regions.is_empty() {
            return;
        }
        if let Some(blocks) = self
            .command_blocks
            .get(&SpanKey::new(command.span))
            .cloned()
        {
            for block in blocks {
                self.attach_nested_regions(block, &command.nested_regions, &[]);
            }
        }
    }

    fn command_block(&mut self, span: Span) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        let key = SpanKey::new(span);
        self.blocks.push(BasicBlock {
            id,
            commands: vec![span],
            bindings: self.command_bindings.get(&key).cloned().unwrap_or_default(),
            references: self
                .command_references
                .get(&key)
                .cloned()
                .unwrap_or_default(),
        });
        self.command_blocks.entry(key).or_default().push(id);
        id
    }

    fn empty_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            commands: Vec::new(),
            bindings: Vec::new(),
            references: Vec::new(),
        });
        id
    }

    fn add_edge(&mut self, from: BlockId, to: BlockId, kind: EdgeKind) {
        self.successors.entry(from).or_default().push((to, kind));
    }
}

fn resolve_break_target(loops: &[LoopTarget], depth: usize) -> Option<&LoopTarget> {
    loops.iter().rev().nth(depth.saturating_sub(1))
}

fn derive_predecessors(
    successors: &FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
) -> FxHashMap<BlockId, Vec<BlockId>> {
    let mut predecessors: FxHashMap<BlockId, Vec<BlockId>> = FxHashMap::default();
    for (block, edges) in successors {
        for (target, _) in edges {
            predecessors.entry(*target).or_default().push(*block);
        }
    }
    predecessors
}

fn compute_unreachable(
    blocks: &[BasicBlock],
    roots: &FxHashMap<ScopeId, BlockId>,
    successors: &FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
) -> Vec<BlockId> {
    let mut visited = FxHashMap::default();
    let mut stack: Vec<BlockId> = roots.values().copied().collect();
    while let Some(block) = stack.pop() {
        if visited.insert(block, ()).is_some() {
            continue;
        }
        if let Some(edges) = successors.get(&block) {
            for (target, _) in edges {
                stack.push(*target);
            }
        }
    }

    blocks
        .iter()
        .filter_map(|block| (!visited.contains_key(&block.id)).then_some(block.id))
        .collect()
}
