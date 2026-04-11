use rustc_hash::{FxHashMap, FxHashSet};
use shuck_parser::{OptionValue, ShellProfile, ZshEmulationMode, ZshOptionState};

use crate::cfg::{
    RecordedCaseArm, RecordedCommand, RecordedCommandKind, RecordedPipelineSegment,
    RecordedProgram, RecordedZshCommandEffect, RecordedZshOptionUpdate,
};
use crate::{Binding, BindingKind, Scope, ScopeId, ScopeKind, SpanKey};

#[derive(Debug, Clone)]
pub(crate) struct ZshOptionAnalysis {
    scope_entries: FxHashMap<ScopeId, ZshOptionState>,
    snapshots: FxHashMap<ScopeId, Vec<ZshOptionSnapshot>>,
}

#[derive(Debug, Clone)]
struct ZshOptionSnapshot {
    offset: usize,
    state: ZshOptionState,
}

#[derive(Debug, Clone)]
struct FunctionSummary {
    final_outward: InternalState,
    outward_touched: FxHashSet<ZshOptionField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeakBehavior {
    Always,
    Function,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
struct InternalState {
    public: ZshOptionState,
    local_options: OptionValue,
    emulation: EmulationState,
}

impl InternalState {
    fn from_profile(profile: &ShellProfile) -> Option<Self> {
        let public = profile.zsh_options()?.clone();
        let emulation = if public == ZshOptionState::for_emulate(ZshEmulationMode::Sh) {
            EmulationState::Sh
        } else if public == ZshOptionState::for_emulate(ZshEmulationMode::Ksh) {
            EmulationState::Ksh
        } else if public == ZshOptionState::for_emulate(ZshEmulationMode::Csh) {
            EmulationState::Csh
        } else {
            EmulationState::Zsh
        };
        Some(Self {
            public,
            local_options: OptionValue::Off,
            emulation,
        })
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
    outward_touched: FxHashSet<ZshOptionField>,
}

impl EvalState {
    fn new(entry: InternalState) -> Self {
        Self {
            current: entry.clone(),
            outward: entry,
            outward_touched: FxHashSet::default(),
        }
    }

    fn merge(&self, other: &Self) -> Self {
        let mut outward_touched = self.outward_touched.clone();
        outward_touched.extend(other.outward_touched.iter().copied());
        Self {
            current: self.current.merge(&other.current),
            outward: self.outward.merge(&other.outward),
            outward_touched,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ZshOptionField {
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
}

pub(crate) fn analyze(
    shell_profile: &ShellProfile,
    scopes: &[Scope],
    bindings: &[Binding],
    recorded_program: &RecordedProgram,
) -> Option<ZshOptionAnalysis> {
    let entry = InternalState::from_profile(shell_profile)?;
    let mut analyzer = Analyzer {
        scopes,
        bindings,
        recorded_program,
        scope_entries: FxHashMap::default(),
        snapshots: FxHashMap::default(),
        active_function_scopes: FxHashSet::default(),
    };

    analyzer.analyze_sequence(
        ScopeId(0),
        &recorded_program.file_commands,
        EvalState::new(entry),
        LeakBehavior::Always,
    );

    for snapshots in analyzer.snapshots.values_mut() {
        snapshots.sort_by_key(|snapshot| snapshot.offset);
    }

    Some(ZshOptionAnalysis {
        scope_entries: analyzer.scope_entries,
        snapshots: analyzer.snapshots,
    })
}

impl ZshOptionAnalysis {
    pub(crate) fn options_at<'a>(&'a self, scopes: &[Scope], offset: usize) -> Option<&'a ZshOptionState> {
        let scope = scopes
            .iter()
            .filter(|scope| contains_offset(scope.span, offset))
            .min_by_key(|scope| scope.span.end.offset - scope.span.start.offset)
            .map(|scope| scope.id)?;

        if let Some(snapshots) = self.snapshots.get(&scope)
            && let Some(snapshot) = snapshots
                .iter()
                .rev()
                .find(|snapshot| snapshot.offset <= offset)
        {
            return Some(&snapshot.state);
        }

        self.scope_entries.get(&scope)
    }
}

struct Analyzer<'a> {
    scopes: &'a [Scope],
    bindings: &'a [Binding],
    recorded_program: &'a RecordedProgram,
    scope_entries: FxHashMap<ScopeId, ZshOptionState>,
    snapshots: FxHashMap<ScopeId, Vec<ZshOptionSnapshot>>,
    active_function_scopes: FxHashSet<ScopeId>,
}

impl<'a> Analyzer<'a> {
    fn analyze_sequence(
        &mut self,
        scope: ScopeId,
        commands: &[RecordedCommand],
        mut state: EvalState,
        leak: LeakBehavior,
    ) -> EvalState {
        self.record_scope_entry(scope, &state.current.public);
        for command in commands {
            self.record_snapshot(scope, command.span.start.offset, &state.current.public);
            state = self.analyze_command(scope, command, state, leak);
        }
        state
    }

    fn analyze_command(
        &mut self,
        scope: ScopeId,
        command: &RecordedCommand,
        state: EvalState,
        leak: LeakBehavior,
    ) -> EvalState {
        for region in &command.nested_regions {
            self.analyze_sequence(
                region.scope,
                &region.commands,
                EvalState::new(state.current.clone()),
                LeakBehavior::Never,
            );
        }

        match &command.kind {
            RecordedCommandKind::Linear
            | RecordedCommandKind::Break { .. }
            | RecordedCommandKind::Continue { .. }
            | RecordedCommandKind::Return
            | RecordedCommandKind::Exit => {
                self.analyze_linear_command(scope, command, state, leak)
            }
            RecordedCommandKind::List { first, rest } => {
                let mut list_state = self.analyze_command(scope, first, state, leak);
                for (_operator, command) in rest {
                    let branch = self.analyze_command(scope, command, list_state.clone(), leak);
                    list_state = list_state.merge(&branch);
                }
                list_state
            }
            RecordedCommandKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => self.analyze_if(scope, &state, condition, then_branch, elif_branches, else_branch, leak),
            RecordedCommandKind::While { condition, body }
            | RecordedCommandKind::Until { condition, body } => {
                let after_condition =
                    self.analyze_sequence(scope, condition, state.clone(), leak);
                let iterated = self.analyze_sequence(scope, body, after_condition.clone(), leak);
                after_condition.merge(&iterated)
            }
            RecordedCommandKind::For { body }
            | RecordedCommandKind::Select { body }
            | RecordedCommandKind::ArithmeticFor { body }
            | RecordedCommandKind::BraceGroup { body } => {
                let iterated = self.analyze_sequence(scope, body, state.clone(), leak);
                state.merge(&iterated)
            }
            RecordedCommandKind::Case { arms } => self.analyze_case(scope, &state, arms, leak),
            RecordedCommandKind::Subshell { body } => {
                self.analyze_sequence(
                    self.subshell_scope_for(command.span.start.offset).unwrap_or(scope),
                    body,
                    EvalState::new(state.current.clone()),
                    LeakBehavior::Never,
                );
                state
            }
            RecordedCommandKind::Pipeline { segments } => self.analyze_pipeline(scope, &state, segments, leak),
        }
    }

    fn analyze_if(
        &mut self,
        scope: ScopeId,
        state: &EvalState,
        condition: &[RecordedCommand],
        then_branch: &[RecordedCommand],
        elif_branches: &[(Vec<RecordedCommand>, Vec<RecordedCommand>)],
        else_branch: &[RecordedCommand],
        leak: LeakBehavior,
    ) -> EvalState {
        let after_condition = self.analyze_sequence(scope, condition, state.clone(), leak);
        let mut merged = self.analyze_sequence(scope, then_branch, after_condition.clone(), leak);

        for (elif_condition, elif_body) in elif_branches {
            let after_elif_condition =
                self.analyze_sequence(scope, elif_condition, after_condition.clone(), leak);
            let elif_result =
                self.analyze_sequence(scope, elif_body, after_elif_condition, leak);
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
        arms: &[RecordedCaseArm],
        leak: LeakBehavior,
    ) -> EvalState {
        let mut merged = state.clone();
        for arm in arms {
            let arm_result = self.analyze_sequence(scope, &arm.commands, state.clone(), leak);
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
        segments: &[RecordedPipelineSegment],
        leak: LeakBehavior,
    ) -> EvalState {
        let mut result = state.clone();
        let emulation = state.current.emulation;

        if emulation == EmulationState::Unknown {
            let mut touched = FxHashSet::default();
            for segment in segments {
                let segment_result = self.analyze_sequence(
                    segment.scope,
                    std::slice::from_ref(&segment.command),
                    EvalState::new(state.current.clone()),
                    LeakBehavior::Never,
                );
                touched.extend(segment_result.outward_touched.iter().copied());
            }
            for field in touched {
                self.apply_explicit_public_field(
                    &mut result,
                    leak,
                    field,
                    OptionValue::Unknown,
                );
            }
            return result;
        }

        for (index, segment) in segments.iter().enumerate() {
            let segment_leak = if emulation.is_definitely_sh() || index + 1 != segments.len() {
                LeakBehavior::Never
            } else {
                leak
            };
            let segment_result = self.analyze_sequence(
                segment.scope,
                std::slice::from_ref(&segment.command),
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
        let info = self.recorded_program.command_infos.get(&SpanKey::new(command.span));

        if let Some(function_scope) = info
            .and_then(|info| info.static_callee.as_deref())
            .and_then(|name| self.resolve_visible_function_scope(scope, command.span.start.offset, name))
        {
            let summary = self.analyze_function_scope(
                function_scope,
                EvalState::new(state.current.clone()),
            );
            for field in &summary.outward_touched {
                let value = get_option_field(&summary.final_outward.public, *field);
                set_option_field(&mut state.current.public, *field, value);
                self.apply_explicit_public_field(&mut state, leak, *field, value);
            }
            self.apply_emulation_state(&mut state, leak, summary.final_outward.emulation);
            return state;
        }

        if let Some(info) = info {
            for effect in &info.zsh_effects {
                match effect {
                    RecordedZshCommandEffect::Emulate { mode, local } => {
                        self.apply_emulate(&mut state, leak, *mode, *local, is_function_scope(self.scopes, scope));
                    }
                    RecordedZshCommandEffect::SetOptions { updates } => {
                        for update in updates {
                            self.apply_option_update(&mut state, leak, update, is_function_scope(self.scopes, scope));
                        }
                    }
                }
            }
        }

        state
    }

    fn analyze_function_scope(&mut self, scope: ScopeId, entry: EvalState) -> FunctionSummary {
        if !self.active_function_scopes.insert(scope) {
            return FunctionSummary {
                final_outward: entry.outward,
                outward_touched: FxHashSet::default(),
            };
        }

        let body = self
            .recorded_program
            .function_bodies
            .get(&scope)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let result = self.analyze_sequence(scope, body, entry, LeakBehavior::Function);
        self.active_function_scopes.remove(&scope);
        FunctionSummary {
            final_outward: result.outward,
            outward_touched: result.outward_touched,
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
                    && matches!(scope.kind, ScopeKind::Subshell | ScopeKind::CommandSubstitution)
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
        state.current.public = next_public.clone();
        state.current.emulation = EmulationState::from_mode(mode);
        if localize {
            state.current.local_options = OptionValue::On;
            return;
        }

        self.apply_explicit_public_state(state, leak, &fields, &next_public);
        state.outward.emulation = state.outward.emulation.merge(EmulationState::from_mode(mode));
        if leak == LeakBehavior::Always
            || (leak == LeakBehavior::Function && state.current.local_options.is_definitely_off())
        {
            state.outward.emulation = EmulationState::from_mode(mode);
        }
    }

    fn apply_option_update(
        &self,
        state: &mut EvalState,
        leak: LeakBehavior,
        update: &RecordedZshOptionUpdate,
        in_function: bool,
    ) {
        match update {
            RecordedZshOptionUpdate::LocalOptions { enable } if in_function => {
                state.current.local_options = if *enable {
                    OptionValue::On
                } else {
                    OptionValue::Off
                };
            }
            RecordedZshOptionUpdate::LocalOptions { .. } => {}
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
                state.outward.public = next.clone();
                state.outward_touched.extend(fields.iter().copied());
            }
            LeakBehavior::Function => match state.current.local_options {
                OptionValue::On => {}
                OptionValue::Off => {
                    state.outward.public = next.clone();
                    state.outward_touched.extend(fields.iter().copied());
                }
                OptionValue::Unknown => {
                    let merged = state.outward.public.merge(next);
                    state.outward.public = merged;
                    state.outward_touched.extend(fields.iter().copied());
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

    fn record_scope_entry(&mut self, scope: ScopeId, state: &ZshOptionState) {
        self.scope_entries
            .entry(scope)
            .or_insert_with(|| state.clone());
    }

    fn record_snapshot(&mut self, scope: ScopeId, offset: usize, state: &ZshOptionState) {
        self.snapshots
            .entry(scope)
            .or_default()
            .push(ZshOptionSnapshot {
                offset,
                state: state.clone(),
            });
    }
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

fn contains_offset(span: shuck_ast::Span, offset: usize) -> bool {
    span.start.offset <= offset && offset <= span.end.offset
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
