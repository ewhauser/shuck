use rustc_hash::{FxHashMap, FxHashSet};
use shuck_parser::{OptionValue, ShellProfile, ZshEmulationMode, ZshOptionState};
use smallvec::SmallVec;
use std::sync::Mutex;

use crate::cfg::{
    CommandId, RecordedCaseArmRange, RecordedCommand, RecordedCommandKind, RecordedCommandRange,
    RecordedElifBranchRange, RecordedPipelineSegmentRange, RecordedProgram,
    RecordedZshCommandEffect, RecordedZshOptionUpdate,
};
use crate::{
    Binding, BindingId, BindingKind, IndirectTargetHint, Reference, ReferenceId, Scope, ScopeId,
    ScopeKind, Span, SpanKey,
};

#[derive(Debug, Clone)]
pub(crate) struct ZshOptionAnalysis {
    scope_entries: FxHashMap<ScopeId, ZshOptionEntry>,
    snapshots: FxHashMap<ScopeId, Vec<ZshOptionSnapshot>>,
    /// Scopes sorted by `(span.start.offset ASC, span.end.offset DESC)` so a binary search by
    /// start offset followed by a backward walk yields the deepest containing scope under
    /// proper scope nesting. Built once per analysis to keep `options_at` off the
    /// O(commands × scopes) path.
    scope_index: Vec<ScopeIndexEntry>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DynamicCallAnalysisContext<'a> {
    pub(crate) references: &'a [Reference],
    pub(crate) resolved: &'a FxHashMap<ReferenceId, BindingId>,
    pub(crate) indirect_target_hints: &'a FxHashMap<BindingId, IndirectTargetHint>,
    pub(crate) indirect_targets_by_binding: &'a FxHashMap<BindingId, Vec<BindingId>>,
    pub(crate) command_references: &'a FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
}

#[derive(Debug, Clone, Copy)]
struct ScopeIndexEntry {
    start: usize,
    end: usize,
    scope: ScopeId,
}

#[derive(Debug, Clone)]
struct ZshOptionSnapshot {
    offset: usize,
    entry: ZshOptionEntry,
}

#[derive(Debug, Clone)]
struct FunctionSummary {
    final_outward: InternalState,
    outward_touched: ZshOptionMask,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FunctionSummaryKey {
    scope: ScopeId,
    entry: InternalState,
    active_function_scopes: SmallVec<[ScopeId; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeakBehavior {
    Always,
    Function,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum EmulationState {
    Zsh,
    Sh,
    Ksh,
    Csh,
    Unknown,
}

impl EmulationState {
    fn from_mode(mode: ZshEmulationMode) -> Self {
        match mode {
            ZshEmulationMode::Zsh => Self::Zsh,
            ZshEmulationMode::Sh => Self::Sh,
            ZshEmulationMode::Ksh => Self::Ksh,
            ZshEmulationMode::Csh => Self::Csh,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self == other { self } else { Self::Unknown }
    }

    fn is_definitely_sh(self) -> bool {
        matches!(self, Self::Sh)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InternalState {
    public: ZshOptionState,
    local_options: OptionValue,
    emulation: EmulationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ZshOptionEntry {
    public: ZshOptionState,
    emulation: EmulationState,
}

impl ZshOptionEntry {
    fn from_internal(state: &InternalState) -> Self {
        Self {
            public: state.public,
            emulation: state.emulation,
        }
    }

    fn merge(&self, other: &Self) -> Self {
        Self {
            public: self.public.merge(&other.public),
            emulation: self.emulation.merge(other.emulation),
        }
    }
}

impl InternalState {
    fn from_profile(profile: &ShellProfile) -> Option<Self> {
        Some(Self::from_public(*profile.zsh_options()?))
    }

    fn from_public(public: ZshOptionState) -> Self {
        let emulation = if public == ZshOptionState::for_emulate(ZshEmulationMode::Sh) {
            EmulationState::Sh
        } else if public == ZshOptionState::for_emulate(ZshEmulationMode::Ksh) {
            EmulationState::Ksh
        } else if public == ZshOptionState::for_emulate(ZshEmulationMode::Csh) {
            EmulationState::Csh
        } else {
            EmulationState::Zsh
        };
        Self {
            public,
            local_options: OptionValue::Off,
            emulation,
        }
    }

    fn merge(&self, other: &Self) -> Self {
        Self {
            public: self.public.merge(&other.public),
            local_options: self.local_options.merge(other.local_options),
            emulation: self.emulation.merge(other.emulation),
        }
    }
}

#[derive(Debug, Clone)]
struct EvalState {
    current: InternalState,
    outward: InternalState,
    outward_touched: ZshOptionMask,
}

impl EvalState {
    fn new(entry: InternalState) -> Self {
        Self {
            current: entry.clone(),
            outward: entry,
            outward_touched: ZshOptionMask::default(),
        }
    }

    fn merge(&self, other: &Self) -> Self {
        Self {
            current: self.current.merge(&other.current),
            outward: self.outward.merge(&other.outward),
            outward_touched: self.outward_touched.union(other.outward_touched),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum ZshOptionField {
    ShWordSplit,
    GlobSubst,
    RcExpandParam,
    Glob,
    Nomatch,
    NullGlob,
    CshNullGlob,
    ExtendedGlob,
    KshGlob,
    ShGlob,
    BareGlobQual,
    GlobDots,
    Equals,
    MagicEqualSubst,
    ShFileExpansion,
    GlobAssign,
    IgnoreBraces,
    IgnoreCloseBraces,
    BraceCcl,
    KshArrays,
    KshZeroSubscript,
    ShortLoops,
    ShortRepeat,
    RcQuotes,
    InteractiveComments,
    CBases,
    OctalZeroes,
}

impl ZshOptionField {
    const ALL: [Self; 27] = [
        Self::ShWordSplit,
        Self::GlobSubst,
        Self::RcExpandParam,
        Self::Glob,
        Self::Nomatch,
        Self::NullGlob,
        Self::CshNullGlob,
        Self::ExtendedGlob,
        Self::KshGlob,
        Self::ShGlob,
        Self::BareGlobQual,
        Self::GlobDots,
        Self::Equals,
        Self::MagicEqualSubst,
        Self::ShFileExpansion,
        Self::GlobAssign,
        Self::IgnoreBraces,
        Self::IgnoreCloseBraces,
        Self::BraceCcl,
        Self::KshArrays,
        Self::KshZeroSubscript,
        Self::ShortLoops,
        Self::ShortRepeat,
        Self::RcQuotes,
        Self::InteractiveComments,
        Self::CBases,
        Self::OctalZeroes,
    ];

    fn bit(self) -> u32 {
        1u32 << (self as u8)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct ZshOptionMask(u32);

impl ZshOptionMask {
    pub(crate) const ALL: Self = Self((1u32 << ZshOptionField::ALL.len()) - 1);

    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn contains(self, field: ZshOptionField) -> bool {
        self.0 & field.bit() != 0
    }

    pub(crate) fn insert(&mut self, field: ZshOptionField) {
        self.0 |= field.bit();
    }

    fn insert_all(&mut self, fields: impl IntoIterator<Item = ZshOptionField>) {
        for field in fields {
            self.insert(field);
        }
    }

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) fn iter(self) -> impl Iterator<Item = ZshOptionField> {
        ZshOptionField::ALL
            .into_iter()
            .filter(move |field| self.contains(*field))
    }
}

pub(crate) fn analyze(
    shell_profile: &ShellProfile,
    scopes: &[Scope],
    bindings: &[Binding],
    recorded_program: &RecordedProgram,
    dynamic_calls: DynamicCallAnalysisContext<'_>,
) -> Option<ZshOptionAnalysis> {
    let entry = InternalState::from_profile(shell_profile)?;
    let function_count = recorded_program.function_body_scopes.len();
    let mut analyzer = Analyzer {
        scopes,
        bindings,
        dynamic_calls,
        recorded_program,
        treat_unknown_dispatch_bindings_as_ambiguous_in_functions: false,
        scope_entries: FxHashMap::with_capacity_and_hasher(scopes.len(), Default::default()),
        snapshots: FxHashMap::with_capacity_and_hasher(scopes.len(), Default::default()),
        active_function_scopes: FxHashSet::with_capacity_and_hasher(
            function_count,
            Default::default(),
        ),
        function_summaries: FxHashMap::with_capacity_and_hasher(function_count, Default::default()),
        direct_function_callees: FxHashMap::with_capacity_and_hasher(
            function_count,
            Default::default(),
        ),
        shared_function_summaries: None,
    };

    analyzer.analyze_sequence(
        ScopeId(0),
        recorded_program.file_commands(),
        EvalState::new(entry),
        LeakBehavior::Always,
    );

    for snapshots in analyzer.snapshots.values_mut() {
        snapshots.sort_by_key(|snapshot| snapshot.offset);
    }

    let mut scope_index: Vec<ScopeIndexEntry> = scopes
        .iter()
        .map(|scope| ScopeIndexEntry {
            start: scope.span.start.offset,
            end: scope.span.end.offset,
            scope: scope.id,
        })
        .collect();
    scope_index.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));

    Some(ZshOptionAnalysis {
        scope_entries: analyzer.scope_entries,
        snapshots: analyzer.snapshots,
        scope_index,
    })
}

pub(crate) fn runtime_ambiguous_entry_mask(recorded_program: &RecordedProgram) -> ZshOptionMask {
    let mut mask = ZshOptionMask::default();
    for info_id in recorded_program.command_infos.values() {
        let info = recorded_program.command_info(*info_id);
        for effect in &info.zsh_effects {
            match effect {
                RecordedZshCommandEffect::Emulate { mode, .. } => {
                    if *mode == ZshEmulationMode::Ksh {
                        mask.insert(ZshOptionField::KshArrays);
                    }
                }
                RecordedZshCommandEffect::EmulateUnknown { .. } => {
                    return ZshOptionMask::ALL;
                }
                RecordedZshCommandEffect::SetOptions { updates } => {
                    for update in updates {
                        match update {
                            RecordedZshOptionUpdate::UnknownName => return ZshOptionMask::ALL,
                            RecordedZshOptionUpdate::Named { name, .. } => {
                                if let Some(field) = field_for_option_name(name) {
                                    mask.insert(field);
                                }
                            }
                            RecordedZshOptionUpdate::LocalOptions { .. } => {}
                        }
                    }
                }
            }
        }
    }
    mask
}

pub(crate) fn function_runtime_analysis_with_entry(
    scopes: &[Scope],
    bindings: &[Binding],
    recorded_program: &RecordedProgram,
    dynamic_calls: DynamicCallAnalysisContext<'_>,
    shared_summaries: Option<&SharedFunctionSummaryCache>,
    function_scope: ScopeId,
    entry: ZshOptionState,
) -> Option<ZshOptionAnalysis> {
    let function_span = scopes.get(function_scope.index())?.span;
    let mut scope_index: Vec<ScopeIndexEntry> = scopes
        .iter()
        .filter(|scope| {
            function_span.start.offset <= scope.span.start.offset
                && scope.span.end.offset <= function_span.end.offset
        })
        .map(|scope| ScopeIndexEntry {
            start: scope.span.start.offset,
            end: scope.span.end.offset,
            scope: scope.id,
        })
        .collect();
    scope_index.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));
    let scope_capacity = scope_index.len();

    let mut analyzer = Analyzer {
        scopes,
        bindings,
        dynamic_calls,
        recorded_program,
        treat_unknown_dispatch_bindings_as_ambiguous_in_functions: true,
        scope_entries: FxHashMap::with_capacity_and_hasher(scope_capacity, Default::default()),
        snapshots: FxHashMap::with_capacity_and_hasher(scope_capacity, Default::default()),
        active_function_scopes: FxHashSet::default(),
        function_summaries: FxHashMap::default(),
        direct_function_callees: FxHashMap::default(),
        shared_function_summaries: shared_summaries,
    };
    analyzer.analyze_function_scope_with_shared_summary_reuse(
        function_scope,
        EvalState::new(InternalState::from_public(entry)),
        false,
    );

    for snapshots in analyzer.snapshots.values_mut() {
        snapshots.sort_by_key(|snapshot| snapshot.offset);
    }

    Some(ZshOptionAnalysis {
        scope_entries: analyzer.scope_entries,
        snapshots: analyzer.snapshots,
        scope_index,
    })
}

pub(crate) fn set_public_option_field(
    state: &mut ZshOptionState,
    field: ZshOptionField,
    value: OptionValue,
) {
    set_option_field(state, field, value);
}

#[derive(Debug, Default)]
pub(crate) struct SharedFunctionSummaryCache {
    summaries: Mutex<FxHashMap<FunctionSummaryKey, FunctionSummary>>,
    direct_callees: Mutex<FxHashMap<ScopeId, SmallVec<[ScopeId; 8]>>>,
}

impl SharedFunctionSummaryCache {
    fn get(&self, key: &FunctionSummaryKey) -> Option<FunctionSummary> {
        let summaries = match self.summaries.lock() {
            Ok(summaries) => summaries,
            Err(poisoned) => poisoned.into_inner(),
        };
        summaries.get(key).cloned()
    }

    fn insert(&self, key: FunctionSummaryKey, summary: FunctionSummary) {
        let mut summaries = match self.summaries.lock() {
            Ok(summaries) => summaries,
            Err(poisoned) => poisoned.into_inner(),
        };
        summaries.insert(key, summary);
    }

    fn get_direct_callees(&self, scope: ScopeId) -> Option<SmallVec<[ScopeId; 8]>> {
        let callees = match self.direct_callees.lock() {
            Ok(callees) => callees,
            Err(poisoned) => poisoned.into_inner(),
        };
        callees.get(&scope).cloned()
    }

    fn insert_direct_callees(&self, scope: ScopeId, callees: SmallVec<[ScopeId; 8]>) {
        let mut cached = match self.direct_callees.lock() {
            Ok(cached) => cached,
            Err(poisoned) => poisoned.into_inner(),
        };
        cached.insert(scope, callees);
    }
}

impl ZshOptionAnalysis {
    fn entry_at<'a>(&'a self, scopes: &[Scope], offset: usize) -> Option<&'a ZshOptionEntry> {
        let upper = self
            .scope_index
            .partition_point(|entry| entry.start <= offset);
        let mut scope = self.scope_index[..upper]
            .iter()
            .rev()
            .find(|entry| entry.end >= offset)
            .map(|entry| entry.scope);

        while let Some(scope_id) = scope {
            if let Some(snapshots) = self.snapshots.get(&scope_id) {
                let upper = snapshots.partition_point(|snapshot| snapshot.offset <= offset);
                if upper > 0 {
                    return Some(&snapshots[upper - 1].entry);
                }
            }

            if let Some(entry) = self.scope_entries.get(&scope_id) {
                return Some(entry);
            }

            scope = scopes[scope_id.index()].parent;
        }

        None
    }

    pub(crate) fn options_at<'a>(
        &'a self,
        scopes: &[Scope],
        offset: usize,
    ) -> Option<&'a ZshOptionState> {
        self.entry_at(scopes, offset).map(|entry| &entry.public)
    }

    pub(crate) fn sh_emulation_at(&self, scopes: &[Scope], offset: usize) -> Option<bool> {
        self.entry_at(scopes, offset)
            .map(|entry| entry.emulation.is_definitely_sh())
    }
}

struct Analyzer<'a> {
    scopes: &'a [Scope],
    bindings: &'a [Binding],
    dynamic_calls: DynamicCallAnalysisContext<'a>,
    recorded_program: &'a RecordedProgram,
    treat_unknown_dispatch_bindings_as_ambiguous_in_functions: bool,
    scope_entries: FxHashMap<ScopeId, ZshOptionEntry>,
    snapshots: FxHashMap<ScopeId, Vec<ZshOptionSnapshot>>,
    active_function_scopes: FxHashSet<ScopeId>,
    function_summaries: FxHashMap<FunctionSummaryKey, FunctionSummary>,
    direct_function_callees: FxHashMap<ScopeId, SmallVec<[ScopeId; 8]>>,
    shared_function_summaries: Option<&'a SharedFunctionSummaryCache>,
}

impl<'a> Analyzer<'a> {
    fn scope_is_within_function(&self, start: ScopeId) -> bool {
        let mut current = Some(start);
        while let Some(scope) = current {
            if matches!(self.scopes[scope.index()].kind, ScopeKind::Function(_)) {
                return true;
            }
            current = self.scopes[scope.index()].parent;
        }
        false
    }

    fn should_treat_unknown_dispatch_binding_as_ambiguous(&self, scope: ScopeId) -> bool {
        self.treat_unknown_dispatch_bindings_as_ambiguous_in_functions
            || !self.scope_is_within_function(scope)
    }

    fn apply_function_summary(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        summary: &FunctionSummary,
    ) {
        for field in summary.outward_touched.iter() {
            let value = get_option_field(&summary.final_outward.public, field);
            set_option_field(&mut state.current.public, field, value);
            self.apply_explicit_public_field(state, leak, field, value);
        }
        self.apply_emulation_state(state, leak, summary.final_outward.emulation);
    }

    fn dynamic_function_summary(
        &mut self,
        scope: ScopeId,
        command_span: Span,
        name_span: Span,
        state: &EvalState,
    ) -> Option<FunctionSummary> {
        let mut merged: Option<FunctionSummary> = None;
        let mut seen_scopes = FxHashSet::default();
        let mut saw_unresolved_name = false;
        let reference_ids = self
            .dynamic_calls
            .command_references
            .get(&SpanKey::new(command_span))?;

        for &reference_id in reference_ids {
            let reference = &self.dynamic_calls.references[reference_id.index()];
            if !contains_span(name_span, reference.span) {
                continue;
            }
            let Some(binding_id) = self.dynamic_calls.resolved.get(&reference_id).copied() else {
                saw_unresolved_name = true;
                continue;
            };
            let Some(targets) = self
                .dynamic_calls
                .indirect_targets_by_binding
                .get(&binding_id)
            else {
                if !self
                    .dynamic_calls
                    .indirect_target_hints
                    .contains_key(&binding_id)
                    && self.should_treat_unknown_dispatch_binding_as_ambiguous(scope)
                {
                    saw_unresolved_name = true;
                }
                continue;
            };

            // Dynamic name patterns can still refer to multiple distinct function names, but a
            // shadowed redefinition of the same name should resolve to the latest visible body.
            let mut seen_names = FxHashSet::default();
            for &target_id in targets.iter().rev() {
                let binding = &self.bindings[target_id.index()];
                if binding.kind != BindingKind::FunctionDefinition
                    || !binding_visible_at(self.scopes, binding, scope, reference.span)
                {
                    continue;
                }
                if !seen_names.insert(binding.name.as_str()) {
                    continue;
                }
                let Some(function_scope) =
                    self.recorded_program.function_body_scopes.get(&target_id)
                else {
                    continue;
                };
                if !seen_scopes.insert(*function_scope) {
                    continue;
                }

                let summary = self
                    .analyze_function_scope(*function_scope, EvalState::new(state.current.clone()));
                merged = Some(match merged {
                    Some(accumulated) => accumulated.merge(&summary),
                    None => summary,
                });
            }
        }

        if saw_unresolved_name {
            let unchanged = FunctionSummary {
                final_outward: state.current.clone(),
                outward_touched: ZshOptionMask::default(),
            };
            merged = Some(match merged {
                Some(accumulated) => accumulated.merge(&unchanged),
                None => unchanged,
            });

            for function_scope in self.visible_function_scopes_at(scope, name_span) {
                if !seen_scopes.insert(function_scope) {
                    continue;
                }
                let summary = self
                    .analyze_function_scope(function_scope, EvalState::new(state.current.clone()));
                merged = Some(match merged {
                    Some(accumulated) => accumulated.merge(&summary),
                    None => summary,
                });
            }
        }

        merged
    }

    fn visible_function_scopes_at(&self, scope: ScopeId, at: Span) -> Vec<ScopeId> {
        let mut visible = Vec::new();
        let mut seen_names = FxHashSet::default();
        let mut current = Some(scope);

        while let Some(scope_id) = current {
            for bindings in self.scopes[scope_id.index()].bindings.values() {
                for binding_id in bindings.iter().rev().copied() {
                    let binding = &self.bindings[binding_id.index()];
                    if binding.span.start.offset > at.start.offset
                        || binding.kind != BindingKind::FunctionDefinition
                    {
                        continue;
                    }
                    if !seen_names.insert(binding.name.as_str()) {
                        break;
                    }
                    if let Some(function_scope) =
                        self.recorded_program.function_body_scopes.get(&binding_id)
                    {
                        visible.push(*function_scope);
                    }
                    break;
                }
            }
            current = self.scopes[scope_id.index()].parent;
        }

        visible
    }

    fn analyze_single_command_sequence(
        &mut self,
        scope: ScopeId,
        command: CommandId,
        state: EvalState,
        leak: LeakBehavior,
    ) -> EvalState {
        self.record_scope_entry(scope, &state.current);
        let recorded = self.recorded_program.command(command);
        self.record_snapshot(scope, recorded.span.start.offset, &state.current);
        self.analyze_command(scope, command, state, leak)
    }

    fn analyze_sequence(
        &mut self,
        scope: ScopeId,
        commands: RecordedCommandRange,
        mut state: EvalState,
        leak: LeakBehavior,
    ) -> EvalState {
        self.record_scope_entry(scope, &state.current);
        for &command in self.recorded_program.commands_in(commands) {
            let recorded = self.recorded_program.command(command);
            self.record_snapshot(scope, recorded.span.start.offset, &state.current);
            state = self.analyze_command(scope, command, state, leak);
        }
        state
    }

    fn analyze_command(
        &mut self,
        scope: ScopeId,
        command: CommandId,
        state: EvalState,
        leak: LeakBehavior,
    ) -> EvalState {
        let command = self.recorded_program.command(command);
        if !command.nested_regions.is_empty() {
            for region in self.recorded_program.nested_regions(command.nested_regions) {
                self.analyze_sequence(
                    region.scope,
                    region.commands,
                    EvalState::new(state.current.clone()),
                    LeakBehavior::Never,
                );
            }
        }

        match &command.kind {
            RecordedCommandKind::Linear
            | RecordedCommandKind::Break { .. }
            | RecordedCommandKind::Continue { .. }
            | RecordedCommandKind::Return
            | RecordedCommandKind::Exit => self.analyze_linear_command(scope, command, state, leak),
            RecordedCommandKind::List { first, rest } => {
                let mut list_state = self.analyze_command(scope, *first, state, leak);
                for item in self.recorded_program.list_items(*rest) {
                    let branch =
                        self.analyze_command(scope, item.command, list_state.clone(), leak);
                    list_state = list_state.merge(&branch);
                }
                list_state
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => self.analyze_if(
                scope,
                &state,
                *condition,
                *then_branch,
                *elif_branches,
                *else_branch,
                leak,
            ),
            RecordedCommandKind::While { condition, body }
            | RecordedCommandKind::Until { condition, body } => {
                let after_condition = self.analyze_sequence(scope, *condition, state.clone(), leak);
                let iterated = self.analyze_sequence(scope, *body, after_condition.clone(), leak);
                after_condition.merge(&iterated)
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body } => {
                let iterated = self.analyze_sequence(scope, *body, state.clone(), leak);
                state.merge(&iterated)
            }
            RecordedCommandKind::BraceGroup { body } => {
                self.analyze_sequence(scope, *body, state, leak)
            }
            RecordedCommandKind::Always { body, always_body } => {
                let after_body = self.analyze_sequence(scope, *body, state, leak);
                self.analyze_sequence(scope, *always_body, after_body, leak)
            }
            RecordedCommandKind::Case { arms } => self.analyze_case(scope, &state, *arms, leak),
            RecordedCommandKind::Subshell { body } => {
                self.analyze_sequence(
                    self.subshell_scope_for(command.span.start.offset)
                        .unwrap_or(scope),
                    *body,
                    EvalState::new(state.current.clone()),
                    LeakBehavior::Never,
                );
                state
            }
            RecordedCommandKind::Pipeline { segments } => {
                self.analyze_pipeline(scope, &state, *segments, leak)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn analyze_if(
        &mut self,
        scope: ScopeId,
        state: &EvalState,
        condition: RecordedCommandRange,
        then_branch: RecordedCommandRange,
        elif_branches: RecordedElifBranchRange,
        else_branch: RecordedCommandRange,
        leak: LeakBehavior,
    ) -> EvalState {
        let after_condition = self.analyze_sequence(scope, condition, state.clone(), leak);
        let mut merged = self.analyze_sequence(scope, then_branch, after_condition.clone(), leak);

        for elif_branch in self.recorded_program.elif_branches(elif_branches) {
            let after_elif_condition =
                self.analyze_sequence(scope, elif_branch.condition, after_condition.clone(), leak);
            let elif_result =
                self.analyze_sequence(scope, elif_branch.body, after_elif_condition, leak);
            merged = merged.merge(&elif_result);
        }

        if else_branch.is_empty() {
            merged.merge(&after_condition)
        } else {
            let else_result = self.analyze_sequence(scope, else_branch, after_condition, leak);
            merged.merge(&else_result)
        }
    }

    fn analyze_case(
        &mut self,
        scope: ScopeId,
        state: &EvalState,
        arms: RecordedCaseArmRange,
        leak: LeakBehavior,
    ) -> EvalState {
        let mut merged = state.clone();
        for arm in self.recorded_program.case_arms(arms) {
            let arm_result = self.analyze_sequence(scope, arm.commands, state.clone(), leak);
            merged = merged.merge(&arm_result);
            if arm.matches_anything {
                return merged;
            }
        }
        merged
    }

    fn analyze_pipeline(
        &mut self,
        _scope: ScopeId,
        state: &EvalState,
        segments: RecordedPipelineSegmentRange,
        leak: LeakBehavior,
    ) -> EvalState {
        let mut result = state.clone();
        let emulation = state.current.emulation;
        let segments = self.recorded_program.pipeline_segments(segments);

        if emulation == EmulationState::Unknown {
            let mut touched = ZshOptionMask::default();
            for segment in segments {
                let segment_result = self.analyze_single_command_sequence(
                    segment.scope,
                    segment.command,
                    EvalState::new(state.current.clone()),
                    LeakBehavior::Never,
                );
                touched = touched.union(segment_result.outward_touched);
            }
            for field in touched.iter() {
                self.apply_explicit_public_field(&mut result, leak, field, OptionValue::Unknown);
            }
            return result;
        }

        for (index, segment) in segments.iter().enumerate() {
            let segment_leak = if emulation.is_definitely_sh() || index + 1 != segments.len() {
                LeakBehavior::Never
            } else {
                leak
            };
            let segment_result = self.analyze_single_command_sequence(
                segment.scope,
                segment.command,
                EvalState::new(state.current.clone()),
                segment_leak,
            );
            if segment_leak != LeakBehavior::Never {
                result = segment_result;
            }
        }

        result
    }

    fn analyze_linear_command(
        &mut self,
        scope: ScopeId,
        command: &RecordedCommand,
        mut state: EvalState,
        leak: LeakBehavior,
    ) -> EvalState {
        let info = command
            .command_info
            .map(|info_id| self.recorded_program.command_info(info_id));

        if let Some(function_scope) = info
            .and_then(|info| info.static_callee.as_deref())
            .and_then(|name| {
                self.resolve_visible_function_scope(scope, command.span.start.offset, name)
            })
        {
            let summary =
                self.analyze_function_scope(function_scope, EvalState::new(state.current.clone()));
            self.apply_function_summary(&mut state, leak, &summary);
            return state;
        }

        if let Some(info) = info {
            if let Some(name_span) = info.dynamic_name_span
                && let Some(summary) =
                    self.dynamic_function_summary(scope, command.span, name_span, &state)
            {
                self.apply_function_summary(&mut state, leak, &summary);
            }
            for effect in &info.zsh_effects {
                match effect {
                    RecordedZshCommandEffect::Emulate { mode, local } => {
                        self.apply_emulate(
                            &mut state,
                            leak,
                            *mode,
                            *local,
                            is_function_scope(self.scopes, scope),
                        );
                    }
                    RecordedZshCommandEffect::EmulateUnknown { local } => {
                        self.apply_unknown_emulate(
                            &mut state,
                            leak,
                            *local,
                            is_function_scope(self.scopes, scope),
                        );
                    }
                    RecordedZshCommandEffect::SetOptions { updates } => {
                        for update in updates {
                            self.apply_option_update(
                                &mut state,
                                leak,
                                update,
                                is_function_scope(self.scopes, scope),
                            );
                        }
                    }
                }
            }
        }

        state
    }

    fn analyze_function_scope(&mut self, scope: ScopeId, entry: EvalState) -> FunctionSummary {
        self.analyze_function_scope_with_shared_summary_reuse(scope, entry, true)
    }

    fn analyze_function_scope_with_shared_summary_reuse(
        &mut self,
        scope: ScopeId,
        entry: EvalState,
        allow_shared_reuse: bool,
    ) -> FunctionSummary {
        if self.active_function_scopes.contains(&scope) {
            return FunctionSummary {
                final_outward: entry.outward,
                outward_touched: ZshOptionMask::default(),
            };
        }

        let cache_key = FunctionSummaryKey {
            scope,
            entry: entry.current.clone(),
            active_function_scopes: self.active_function_summary_context(scope),
        };
        let recursive_context = !cache_key.active_function_scopes.is_empty();
        if let Some(summary) = self.function_summaries.get(&cache_key) {
            return summary.clone();
        }
        if !recursive_context
            && allow_shared_reuse
            && let Some(summary) = self
                .shared_function_summaries
                .and_then(|summaries| summaries.get(&cache_key))
        {
            self.function_summaries.insert(cache_key, summary.clone());
            return summary;
        }

        self.active_function_scopes.insert(scope);
        let body = self.recorded_program.function_body(scope);
        let result = self.analyze_sequence(scope, body, entry, LeakBehavior::Function);
        self.active_function_scopes.remove(&scope);
        let summary = FunctionSummary {
            final_outward: result.outward,
            outward_touched: result.outward_touched,
        };
        self.function_summaries
            .insert(cache_key.clone(), summary.clone());
        // Recursive-context summaries are useful inside this analyzer, but they may have cut a
        // call through `active_function_scopes`, so do not publish them across root analyses.
        if !recursive_context && let Some(shared_summaries) = self.shared_function_summaries {
            shared_summaries.insert(cache_key, summary.clone());
        }
        summary
    }

    fn active_function_summary_context(&mut self, scope: ScopeId) -> SmallVec<[ScopeId; 4]> {
        if self.active_function_scopes.is_empty() {
            return SmallVec::new();
        }

        // A summary only depends on active functions that this function can reach before the
        // recursion guard cuts the call. Unrelated active ancestors should not split the cache.
        let mut active = SmallVec::<[ScopeId; 4]>::new();
        let mut visited = FxHashSet::default();
        let mut stack: Vec<ScopeId> = self.direct_function_callees(scope).into_iter().collect();

        while let Some(callee) = stack.pop() {
            if !visited.insert(callee) {
                continue;
            }
            if self.active_function_scopes.contains(&callee) {
                active.push(callee);
                continue;
            }
            stack.extend(self.direct_function_callees(callee));
        }

        active.sort_by_key(|scope| scope.0);
        active
    }

    fn direct_function_callees(&mut self, scope: ScopeId) -> SmallVec<[ScopeId; 8]> {
        if let Some(callees) = self.direct_function_callees.get(&scope) {
            return callees.clone();
        }
        if let Some(callees) = self
            .shared_function_summaries
            .and_then(|summaries| summaries.get_direct_callees(scope))
        {
            self.direct_function_callees.insert(scope, callees.clone());
            return callees;
        }

        let mut callees = SmallVec::<[ScopeId; 8]>::new();
        let mut seen = FxHashSet::default();
        self.collect_direct_function_callees_in_range(
            scope,
            self.recorded_program.function_body(scope),
            &mut seen,
            &mut callees,
        );
        self.direct_function_callees.insert(scope, callees.clone());
        if let Some(shared_summaries) = self.shared_function_summaries {
            shared_summaries.insert_direct_callees(scope, callees.clone());
        }
        callees
    }

    fn collect_direct_function_callees_in_range(
        &self,
        scope: ScopeId,
        commands: RecordedCommandRange,
        seen: &mut FxHashSet<ScopeId>,
        callees: &mut SmallVec<[ScopeId; 8]>,
    ) {
        for &command in self.recorded_program.commands_in(commands) {
            self.collect_direct_function_callees_in_command(scope, command, seen, callees);
        }
    }

    fn collect_direct_function_callees_in_command(
        &self,
        scope: ScopeId,
        command: CommandId,
        seen: &mut FxHashSet<ScopeId>,
        callees: &mut SmallVec<[ScopeId; 8]>,
    ) {
        let command = self.recorded_program.command(command);
        for region in self.recorded_program.nested_regions(command.nested_regions) {
            self.collect_direct_function_callees_in_range(
                region.scope,
                region.commands,
                seen,
                callees,
            );
        }

        let command_scope = command.scope.unwrap_or(scope);
        if let Some(info) = command
            .command_info
            .map(|info_id| self.recorded_program.command_info(info_id))
        {
            if let Some(function_scope) = info.static_callee.as_deref().and_then(|name| {
                self.resolve_visible_function_scope(command_scope, command.span.start.offset, name)
            }) {
                push_unique_scope(seen, callees, function_scope);
            } else if let Some(name_span) = info.dynamic_name_span {
                for function_scope in self.visible_function_scopes_at(command_scope, name_span) {
                    push_unique_scope(seen, callees, function_scope);
                }
            }
        }

        match &command.kind {
            RecordedCommandKind::Linear
            | RecordedCommandKind::Break { .. }
            | RecordedCommandKind::Continue { .. }
            | RecordedCommandKind::Return
            | RecordedCommandKind::Exit => {}
            RecordedCommandKind::List { first, rest } => {
                self.collect_direct_function_callees_in_command(
                    command_scope,
                    *first,
                    seen,
                    callees,
                );
                for item in self.recorded_program.list_items(*rest) {
                    self.collect_direct_function_callees_in_command(
                        command_scope,
                        item.command,
                        seen,
                        callees,
                    );
                }
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                self.collect_direct_function_callees_in_range(
                    command_scope,
                    *condition,
                    seen,
                    callees,
                );
                self.collect_direct_function_callees_in_range(
                    command_scope,
                    *then_branch,
                    seen,
                    callees,
                );
                for branch in self.recorded_program.elif_branches(*elif_branches) {
                    self.collect_direct_function_callees_in_range(
                        command_scope,
                        branch.condition,
                        seen,
                        callees,
                    );
                    self.collect_direct_function_callees_in_range(
                        command_scope,
                        branch.body,
                        seen,
                        callees,
                    );
                }
                self.collect_direct_function_callees_in_range(
                    command_scope,
                    *else_branch,
                    seen,
                    callees,
                );
            }
            RecordedCommandKind::While { condition, body }
            | RecordedCommandKind::Until { condition, body } => {
                self.collect_direct_function_callees_in_range(
                    command_scope,
                    *condition,
                    seen,
                    callees,
                );
                self.collect_direct_function_callees_in_range(command_scope, *body, seen, callees);
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body }
            | RecordedCommandKind::BraceGroup { body } => {
                self.collect_direct_function_callees_in_range(command_scope, *body, seen, callees);
            }
            RecordedCommandKind::Always { body, always_body } => {
                self.collect_direct_function_callees_in_range(command_scope, *body, seen, callees);
                self.collect_direct_function_callees_in_range(
                    command_scope,
                    *always_body,
                    seen,
                    callees,
                );
            }
            RecordedCommandKind::Case { arms } => {
                for arm in self.recorded_program.case_arms(*arms) {
                    self.collect_direct_function_callees_in_range(
                        command_scope,
                        arm.commands,
                        seen,
                        callees,
                    );
                }
            }
            RecordedCommandKind::Subshell { body } => {
                let subshell_scope = self
                    .subshell_scope_for(command.span.start.offset)
                    .unwrap_or(command_scope);
                self.collect_direct_function_callees_in_range(subshell_scope, *body, seen, callees);
            }
            RecordedCommandKind::Pipeline { segments } => {
                for segment in self.recorded_program.pipeline_segments(*segments) {
                    self.collect_direct_function_callees_in_command(
                        segment.scope,
                        segment.command,
                        seen,
                        callees,
                    );
                }
            }
        }
    }

    fn resolve_visible_function_scope(
        &self,
        scope: ScopeId,
        offset: usize,
        name: &str,
    ) -> Option<ScopeId> {
        let mut current = Some(scope);
        while let Some(scope_id) = current {
            if let Some(bindings) = self.scopes[scope_id.index()].bindings.get(name) {
                for binding_id in bindings.iter().rev().copied() {
                    let binding = &self.bindings[binding_id.index()];
                    if binding.span.start.offset > offset
                        || binding.kind != BindingKind::FunctionDefinition
                    {
                        continue;
                    }
                    if let Some(body_scope) =
                        self.recorded_program.function_body_scopes.get(&binding_id)
                    {
                        return Some(*body_scope);
                    }
                }
            }
            current = self.scopes[scope_id.index()].parent;
        }
        None
    }

    fn subshell_scope_for(&self, offset: usize) -> Option<ScopeId> {
        self.scopes
            .iter()
            .filter(|scope| {
                scope.span.start.offset <= offset
                    && offset <= scope.span.end.offset
                    && matches!(
                        scope.kind,
                        ScopeKind::Subshell | ScopeKind::CommandSubstitution
                    )
            })
            .min_by_key(|scope| scope.span.end.offset - scope.span.start.offset)
            .map(|scope| scope.id)
    }

    fn apply_emulate(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        mode: ZshEmulationMode,
        local: bool,
        in_function: bool,
    ) {
        let localize = local && in_function;
        let fields = ZshOptionField::ALL;
        let next_public = ZshOptionState::for_emulate(mode);
        state.current.public = next_public;
        state.current.emulation = EmulationState::from_mode(mode);
        if localize {
            state.current.local_options = OptionValue::On;
            return;
        }

        self.apply_explicit_public_state(state, leak, &fields, &next_public);
        state.outward.emulation = state
            .outward
            .emulation
            .merge(EmulationState::from_mode(mode));
        if leak == LeakBehavior::Always
            || (leak == LeakBehavior::Function && state.current.local_options.is_definitely_off())
        {
            state.outward.emulation = EmulationState::from_mode(mode);
        }
    }

    fn apply_unknown_emulate(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        local: bool,
        in_function: bool,
    ) {
        let localize = local && in_function;
        for field in ZshOptionField::ALL {
            set_option_field(&mut state.current.public, field, OptionValue::Unknown);
        }
        state.current.emulation = EmulationState::Unknown;
        if localize {
            state.current.local_options = OptionValue::On;
            return;
        }

        for field in ZshOptionField::ALL {
            self.apply_explicit_public_field(state, leak, field, OptionValue::Unknown);
        }
        self.apply_emulation_state(state, leak, EmulationState::Unknown);
    }

    fn apply_option_update(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        update: &RecordedZshOptionUpdate,
        _in_function: bool,
    ) {
        match update {
            RecordedZshOptionUpdate::LocalOptions { enable } => {
                state.current.local_options = if *enable {
                    OptionValue::On
                } else {
                    OptionValue::Off
                };
            }
            RecordedZshOptionUpdate::UnknownName => {
                state.current.local_options = OptionValue::Unknown;
                for field in ZshOptionField::ALL {
                    set_option_field(&mut state.current.public, field, OptionValue::Unknown);
                    self.apply_explicit_public_field(state, leak, field, OptionValue::Unknown);
                }
            }
            RecordedZshOptionUpdate::Named { name, enable } => {
                let Some(field) = field_for_option_name(name) else {
                    return;
                };
                let value = if *enable {
                    OptionValue::On
                } else {
                    OptionValue::Off
                };
                set_option_field(&mut state.current.public, field, value);
                self.apply_explicit_public_field(state, leak, field, value);
            }
        }
    }

    fn apply_explicit_public_state(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        fields: &[ZshOptionField],
        next: &ZshOptionState,
    ) {
        match leak {
            LeakBehavior::Never => {}
            LeakBehavior::Always => {
                state.outward.public = *next;
                state.outward_touched.insert_all(fields.iter().copied());
            }
            LeakBehavior::Function => match state.current.local_options {
                OptionValue::On => {}
                OptionValue::Off => {
                    state.outward.public = *next;
                    state.outward_touched.insert_all(fields.iter().copied());
                }
                OptionValue::Unknown => {
                    let merged = state.outward.public.merge(next);
                    state.outward.public = merged;
                    state.outward_touched.insert_all(fields.iter().copied());
                }
            },
        }
    }

    fn apply_explicit_public_field(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        field: ZshOptionField,
        value: OptionValue,
    ) {
        match leak {
            LeakBehavior::Never => {}
            LeakBehavior::Always => {
                set_option_field(&mut state.outward.public, field, value);
                state.outward_touched.insert(field);
            }
            LeakBehavior::Function => match state.current.local_options {
                OptionValue::On => {}
                OptionValue::Off => {
                    set_option_field(&mut state.outward.public, field, value);
                    state.outward_touched.insert(field);
                }
                OptionValue::Unknown => {
                    let merged = get_option_field(&state.outward.public, field).merge(value);
                    set_option_field(&mut state.outward.public, field, merged);
                    state.outward_touched.insert(field);
                }
            },
        }
    }

    fn apply_emulation_state(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        emulation: EmulationState,
    ) {
        state.current.emulation = emulation;
        match leak {
            LeakBehavior::Never => {}
            LeakBehavior::Always => {
                state.outward.emulation = emulation;
            }
            LeakBehavior::Function => match state.current.local_options {
                OptionValue::On => {}
                OptionValue::Off => {
                    state.outward.emulation = emulation;
                }
                OptionValue::Unknown => {
                    state.outward.emulation = state.outward.emulation.merge(emulation);
                }
            },
        }
    }

    fn record_scope_entry(&mut self, scope: ScopeId, state: &InternalState) {
        let entry = ZshOptionEntry::from_internal(state);
        self.scope_entries
            .entry(scope)
            .and_modify(|current| *current = current.merge(&entry))
            .or_insert(entry);
    }

    fn record_snapshot(&mut self, scope: ScopeId, offset: usize, state: &InternalState) {
        let entry = ZshOptionEntry::from_internal(state);
        let snapshots = self.snapshots.entry(scope).or_default();
        if let Some(last) = snapshots.last()
            && last.offset <= offset
            && last.entry == entry
        {
            return;
        }
        if let Some(existing) = snapshots
            .iter_mut()
            .find(|snapshot| snapshot.offset == offset)
        {
            existing.entry = existing.entry.merge(&entry);
            return;
        }

        snapshots.push(ZshOptionSnapshot { offset, entry });
    }
}

impl FunctionSummary {
    fn merge(&self, other: &Self) -> Self {
        Self {
            final_outward: self.final_outward.merge(&other.final_outward),
            outward_touched: self.outward_touched.union(other.outward_touched),
        }
    }
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn push_unique_scope(
    seen: &mut FxHashSet<ScopeId>,
    scopes: &mut SmallVec<[ScopeId; 8]>,
    scope: ScopeId,
) {
    if seen.insert(scope) {
        scopes.push(scope);
    }
}

fn binding_visible_at(scopes: &[Scope], binding: &Binding, scope: ScopeId, at: Span) -> bool {
    binding.span.start.offset <= at.start.offset
        && ancestor_scopes(scopes, scope).any(|ancestor| ancestor == binding.scope)
}

fn ancestor_scopes(scopes: &[Scope], scope: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
    std::iter::successors(Some(scope), move |scope_id| scopes[scope_id.index()].parent)
}

fn is_function_scope(scopes: &[Scope], scope: ScopeId) -> bool {
    let mut current = Some(scope);
    while let Some(scope_id) = current {
        if matches!(scopes[scope_id.index()].kind, ScopeKind::Function(_)) {
            return true;
        }
        current = scopes[scope_id.index()].parent;
    }
    false
}

fn field_for_option_name(name: &str) -> Option<ZshOptionField> {
    match name {
        "shwordsplit" => Some(ZshOptionField::ShWordSplit),
        "globsubst" => Some(ZshOptionField::GlobSubst),
        "rcexpandparam" => Some(ZshOptionField::RcExpandParam),
        "glob" => Some(ZshOptionField::Glob),
        "nomatch" => Some(ZshOptionField::Nomatch),
        "nullglob" => Some(ZshOptionField::NullGlob),
        "cshnullglob" => Some(ZshOptionField::CshNullGlob),
        "extendedglob" => Some(ZshOptionField::ExtendedGlob),
        "kshglob" => Some(ZshOptionField::KshGlob),
        "shglob" => Some(ZshOptionField::ShGlob),
        "bareglobqual" => Some(ZshOptionField::BareGlobQual),
        "globdots" => Some(ZshOptionField::GlobDots),
        "equals" => Some(ZshOptionField::Equals),
        "magicequalsubst" => Some(ZshOptionField::MagicEqualSubst),
        "shfileexpansion" => Some(ZshOptionField::ShFileExpansion),
        "globassign" => Some(ZshOptionField::GlobAssign),
        "ignorebraces" => Some(ZshOptionField::IgnoreBraces),
        "ignoreclosebraces" => Some(ZshOptionField::IgnoreCloseBraces),
        "braceccl" => Some(ZshOptionField::BraceCcl),
        "ksharrays" => Some(ZshOptionField::KshArrays),
        "kshzerosubscript" => Some(ZshOptionField::KshZeroSubscript),
        "shortloops" => Some(ZshOptionField::ShortLoops),
        "shortrepeat" => Some(ZshOptionField::ShortRepeat),
        "rcquotes" => Some(ZshOptionField::RcQuotes),
        "interactivecomments" => Some(ZshOptionField::InteractiveComments),
        "cbases" => Some(ZshOptionField::CBases),
        "octalzeroes" => Some(ZshOptionField::OctalZeroes),
        _ => None,
    }
}

fn get_option_field(state: &ZshOptionState, field: ZshOptionField) -> OptionValue {
    match field {
        ZshOptionField::ShWordSplit => state.sh_word_split,
        ZshOptionField::GlobSubst => state.glob_subst,
        ZshOptionField::RcExpandParam => state.rc_expand_param,
        ZshOptionField::Glob => state.glob,
        ZshOptionField::Nomatch => state.nomatch,
        ZshOptionField::NullGlob => state.null_glob,
        ZshOptionField::CshNullGlob => state.csh_null_glob,
        ZshOptionField::ExtendedGlob => state.extended_glob,
        ZshOptionField::KshGlob => state.ksh_glob,
        ZshOptionField::ShGlob => state.sh_glob,
        ZshOptionField::BareGlobQual => state.bare_glob_qual,
        ZshOptionField::GlobDots => state.glob_dots,
        ZshOptionField::Equals => state.equals,
        ZshOptionField::MagicEqualSubst => state.magic_equal_subst,
        ZshOptionField::ShFileExpansion => state.sh_file_expansion,
        ZshOptionField::GlobAssign => state.glob_assign,
        ZshOptionField::IgnoreBraces => state.ignore_braces,
        ZshOptionField::IgnoreCloseBraces => state.ignore_close_braces,
        ZshOptionField::BraceCcl => state.brace_ccl,
        ZshOptionField::KshArrays => state.ksh_arrays,
        ZshOptionField::KshZeroSubscript => state.ksh_zero_subscript,
        ZshOptionField::ShortLoops => state.short_loops,
        ZshOptionField::ShortRepeat => state.short_repeat,
        ZshOptionField::RcQuotes => state.rc_quotes,
        ZshOptionField::InteractiveComments => state.interactive_comments,
        ZshOptionField::CBases => state.c_bases,
        ZshOptionField::OctalZeroes => state.octal_zeroes,
    }
}

fn set_option_field(state: &mut ZshOptionState, field: ZshOptionField, value: OptionValue) {
    match field {
        ZshOptionField::ShWordSplit => state.sh_word_split = value,
        ZshOptionField::GlobSubst => state.glob_subst = value,
        ZshOptionField::RcExpandParam => state.rc_expand_param = value,
        ZshOptionField::Glob => state.glob = value,
        ZshOptionField::Nomatch => state.nomatch = value,
        ZshOptionField::NullGlob => state.null_glob = value,
        ZshOptionField::CshNullGlob => state.csh_null_glob = value,
        ZshOptionField::ExtendedGlob => state.extended_glob = value,
        ZshOptionField::KshGlob => state.ksh_glob = value,
        ZshOptionField::ShGlob => state.sh_glob = value,
        ZshOptionField::BareGlobQual => state.bare_glob_qual = value,
        ZshOptionField::GlobDots => state.glob_dots = value,
        ZshOptionField::Equals => state.equals = value,
        ZshOptionField::MagicEqualSubst => state.magic_equal_subst = value,
        ZshOptionField::ShFileExpansion => state.sh_file_expansion = value,
        ZshOptionField::GlobAssign => state.glob_assign = value,
        ZshOptionField::IgnoreBraces => state.ignore_braces = value,
        ZshOptionField::IgnoreCloseBraces => state.ignore_close_braces = value,
        ZshOptionField::BraceCcl => state.brace_ccl = value,
        ZshOptionField::KshArrays => state.ksh_arrays = value,
        ZshOptionField::KshZeroSubscript => state.ksh_zero_subscript = value,
        ZshOptionField::ShortLoops => state.short_loops = value,
        ZshOptionField::ShortRepeat => state.short_repeat = value,
        ZshOptionField::RcQuotes => state.rc_quotes = value,
        ZshOptionField::InteractiveComments => state.interactive_comments = value,
        ZshOptionField::CBases => state.c_bases = value,
        ZshOptionField::OctalZeroes => state.octal_zeroes = value,
    }
}
