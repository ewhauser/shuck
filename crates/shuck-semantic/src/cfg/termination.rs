//! Script-termination reasoning for function calls.
//!
//! A plain command can terminate the rest of a script when it resolves to a
//! function whose reachable paths all call `exit`. For example:
//!
//! ```sh
//! fatal() {
//!   echo "$1" >&2
//!   exit 1
//! }
//!
//! fatal "missing config"
//! echo unreachable
//! ```
//!
//! This module marks the `fatal` call as script-terminating only when the
//! visible function body really exits on every path and no earlier top-level
//! `return` can prevent that definition from being installed. CFG construction
//! then treats later top-level commands as unreachable for the same reason it
//! treats commands after a direct `exit` as unreachable.

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};
use smallvec::SmallVec;

use crate::cfg::{
    CommandId, RecordedCaseArmRange, RecordedCommandKind, RecordedCommandRange,
    RecordedElifBranchRange, RecordedListItemRange, RecordedProgram,
};
use crate::function_resolution::resolved_function_calls_with_callee_scope;
use crate::{Binding, BindingId, CallSite, ScopeId, SpanKey};

#[derive(Debug, Clone, Copy, Default)]
struct ExitEffect {
    may_continue: bool,
    may_return: bool,
    may_exit: bool,
}

impl ExitEffect {
    fn continuing() -> Self {
        Self {
            may_continue: true,
            may_return: false,
            may_exit: false,
        }
    }

    fn returning() -> Self {
        Self {
            may_continue: false,
            may_return: true,
            may_exit: false,
        }
    }

    fn exiting() -> Self {
        Self {
            may_continue: false,
            may_return: false,
            may_exit: true,
        }
    }

    fn combine_alternative(&mut self, other: Self) {
        self.may_continue |= other.may_continue;
        self.may_return |= other.may_return;
        self.may_exit |= other.may_exit;
    }
}

pub(super) fn compute_script_terminating_call_spans(
    program: &RecordedProgram,
    bindings: &[Binding],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
) -> FxHashSet<SpanKey> {
    if program.function_body_scopes.is_empty() || call_sites.is_empty() {
        return FxHashSet::default();
    }

    let mut terminating_call_spans = FxHashSet::default();
    let mut always_exiting_scopes = FxHashSet::default();

    loop {
        let mut changed = false;
        for &scope in program.function_bodies().keys() {
            if always_exiting_scopes.contains(&scope) {
                continue;
            }
            if function_scope_always_exits_script(program, scope, &terminating_call_spans) {
                always_exiting_scopes.insert(scope);
                changed = true;
            }
        }
        changed |= resolve_script_terminating_call_spans(
            program,
            bindings,
            call_sites,
            visible_function_call_bindings,
            &always_exiting_scopes,
            &mut terminating_call_spans,
        );
        if !changed {
            break;
        }
    }

    terminating_call_spans
}

fn file_entry_can_return_before_function_definition(
    program: &RecordedProgram,
    function_definition_offset: usize,
) -> bool {
    command_range_can_return_before(program, program.file_commands(), function_definition_offset)
}

fn command_range_can_return_before(
    program: &RecordedProgram,
    commands: RecordedCommandRange,
    before_offset: usize,
) -> bool {
    program
        .commands_in(commands)
        .iter()
        .copied()
        .any(|command| command_can_return_before(program, command, before_offset))
}

fn command_can_return_before(
    program: &RecordedProgram,
    command_id: CommandId,
    before_offset: usize,
) -> bool {
    let command = program.command(command_id);
    if command.span.start.offset >= before_offset {
        return false;
    }

    match program.command(command_id).kind {
        RecordedCommandKind::Return => command.span.start.offset < before_offset,
        RecordedCommandKind::List { first, rest } => {
            command_can_return_before(program, first, before_offset)
                || program
                    .list_items(rest)
                    .iter()
                    .any(|item| command_can_return_before(program, item.command, before_offset))
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            command_range_can_return_before(program, condition, before_offset)
                || command_range_can_return_before(program, then_branch, before_offset)
                || program.elif_branches(elif_branches).iter().any(|branch| {
                    command_range_can_return_before(program, branch.condition, before_offset)
                        || command_range_can_return_before(program, branch.body, before_offset)
                })
                || command_range_can_return_before(program, else_branch, before_offset)
        }
        RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body } => {
            command_range_can_return_before(program, body, before_offset)
        }
        RecordedCommandKind::Always { body, always_body } => {
            command_range_can_return_before(program, body, before_offset)
                || command_range_can_return_before(program, always_body, before_offset)
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            command_range_can_return_before(program, condition, before_offset)
                || command_range_can_return_before(program, body, before_offset)
        }
        RecordedCommandKind::Case { arms } => program
            .case_arms(arms)
            .iter()
            .any(|arm| command_range_can_return_before(program, arm.commands, before_offset)),
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Exit
        | RecordedCommandKind::Subshell { .. }
        | RecordedCommandKind::Pipeline { .. } => false,
    }
}

fn resolve_script_terminating_call_spans(
    program: &RecordedProgram,
    bindings: &[Binding],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
    always_exiting_scopes: &FxHashSet<ScopeId>,
    terminating_call_spans: &mut FxHashSet<SpanKey>,
) -> bool {
    let mut changed = false;

    for call in resolved_function_calls_with_callee_scope(
        call_sites,
        visible_function_call_bindings,
        &program.function_body_scopes,
    ) {
        if !always_exiting_scopes.contains(&call.callee_scope) {
            continue;
        }
        if file_entry_can_return_before_function_definition(
            program,
            bindings[call.binding.index()].span.start.offset,
        ) {
            continue;
        }
        let command_span = recorded_command_span_for_call_site(program, call.site);
        changed |= terminating_call_spans.insert(SpanKey::new(command_span));
    }

    changed
}

pub(crate) fn recorded_command_span_for_call_site(
    program: &RecordedProgram,
    site: &CallSite,
) -> Span {
    if let Some(command_span) = program
        .call_command_spans
        .get(&SpanKey::new(site.span))
        .copied()
    {
        return command_span;
    }

    program
        .commands()
        .iter()
        .filter(|command| {
            matches!(command.kind, RecordedCommandKind::Linear)
                && span_contains(command.span, site.span)
        })
        .min_by_key(|command| {
            (
                command.span.end.offset - command.span.start.offset,
                command.span.start.offset,
            )
        })
        .or_else(|| {
            program
                .commands()
                .iter()
                .filter(|command| span_contains(command.span, site.span))
                .min_by_key(|command| {
                    (
                        command.span.end.offset - command.span.start.offset,
                        command.span.start.offset,
                    )
                })
        })
        .map(|command| command.span)
        .unwrap_or(site.span)
}

fn function_scope_always_exits_script(
    program: &RecordedProgram,
    scope: ScopeId,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> bool {
    let effect = sequence_exit_effect(
        program,
        program.function_body(scope),
        terminating_call_spans,
    );
    !effect.may_continue && !effect.may_return && effect.may_exit
}

fn sequence_exit_effect(
    program: &RecordedProgram,
    commands: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut sequence_effect = ExitEffect::default();
    let mut may_continue = true;

    for &command_id in program.commands_in(commands) {
        if !may_continue {
            break;
        }

        let command_effect = command_exit_effect(program, command_id, terminating_call_spans);
        sequence_effect.may_exit |= command_effect.may_exit;
        sequence_effect.may_return |= command_effect.may_return;
        may_continue = command_effect.may_continue;
    }

    sequence_effect.may_continue = may_continue;
    sequence_effect
}

fn command_exit_effect(
    program: &RecordedProgram,
    command_id: CommandId,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let command = program.command(command_id);

    match command.kind {
        RecordedCommandKind::Linear => {
            let span_key = SpanKey::new(command.span);
            if terminating_call_spans.contains(&span_key) {
                ExitEffect::exiting()
            } else {
                ExitEffect::continuing()
            }
        }
        RecordedCommandKind::Break { .. } | RecordedCommandKind::Continue { .. } => {
            ExitEffect::continuing()
        }
        RecordedCommandKind::Return => ExitEffect::returning(),
        RecordedCommandKind::Exit => ExitEffect::exiting(),
        RecordedCommandKind::List { first, rest } => {
            list_exit_effect(program, first, rest, terminating_call_spans)
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => if_exit_effect(
            program,
            condition,
            then_branch,
            elif_branches,
            else_branch,
            terminating_call_spans,
        ),
        RecordedCommandKind::BraceGroup { body } => {
            sequence_exit_effect(program, body, terminating_call_spans)
        }
        RecordedCommandKind::Always { body, always_body } => {
            always_exit_effect(program, body, always_body, terminating_call_spans)
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            loop_exit_effect(program, condition, body, terminating_call_spans)
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body } => {
            counted_loop_exit_effect(program, body, terminating_call_spans)
        }
        RecordedCommandKind::Case { arms } => {
            case_exit_effect(program, arms, terminating_call_spans)
        }
        RecordedCommandKind::Subshell { .. } | RecordedCommandKind::Pipeline { .. } => {
            ExitEffect::continuing()
        }
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn list_exit_effect(
    program: &RecordedProgram,
    first: CommandId,
    rest: RecordedListItemRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let first_effect = command_exit_effect(program, first, terminating_call_spans);
    if !first_effect.may_continue {
        return first_effect;
    }

    let mut effect = first_effect;
    for item in program.list_items(rest) {
        let item_effect = command_exit_effect(program, item.command, terminating_call_spans);
        effect.may_continue = true;
        effect.may_return |= item_effect.may_return;
        effect.may_exit |= item_effect.may_exit;
        effect.may_continue |= item_effect.may_continue;
    }
    effect
}

fn if_exit_effect(
    program: &RecordedProgram,
    condition: RecordedCommandRange,
    then_branch: RecordedCommandRange,
    elif_branches: RecordedElifBranchRange,
    else_branch: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let condition_effect = sequence_exit_effect(program, condition, terminating_call_spans);
    let mut effect = ExitEffect {
        may_continue: false,
        may_return: condition_effect.may_return,
        may_exit: condition_effect.may_exit,
    };

    if !condition_effect.may_continue {
        return effect;
    }

    effect.combine_alternative(sequence_exit_effect(
        program,
        then_branch,
        terminating_call_spans,
    ));
    effect.combine_alternative(elif_chain_exit_effect(
        program,
        elif_branches,
        else_branch,
        terminating_call_spans,
    ));
    effect
}

fn elif_chain_exit_effect(
    program: &RecordedProgram,
    elif_branches: RecordedElifBranchRange,
    else_branch: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut false_path = if else_branch.is_empty() {
        ExitEffect::continuing()
    } else {
        sequence_exit_effect(program, else_branch, terminating_call_spans)
    };

    for branch in program.elif_branches(elif_branches).iter().rev() {
        let condition_effect =
            sequence_exit_effect(program, branch.condition, terminating_call_spans);
        let mut branch_effect = ExitEffect {
            may_continue: false,
            may_return: condition_effect.may_return,
            may_exit: condition_effect.may_exit,
        };

        if condition_effect.may_continue {
            branch_effect.combine_alternative(sequence_exit_effect(
                program,
                branch.body,
                terminating_call_spans,
            ));
            branch_effect.combine_alternative(false_path);
        }

        false_path = branch_effect;
    }

    false_path
}

fn loop_exit_effect(
    program: &RecordedProgram,
    condition: RecordedCommandRange,
    body: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut effect = ExitEffect::continuing();
    let condition_effect = sequence_exit_effect(program, condition, terminating_call_spans);
    let body_effect = sequence_exit_effect(program, body, terminating_call_spans);
    effect.may_return |= condition_effect.may_return || body_effect.may_return;
    effect.may_exit |= condition_effect.may_exit || body_effect.may_exit;
    effect
}

fn counted_loop_exit_effect(
    program: &RecordedProgram,
    body: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let mut effect = ExitEffect::continuing();
    let body_effect = sequence_exit_effect(program, body, terminating_call_spans);
    effect.may_return |= body_effect.may_return;
    effect.may_exit |= body_effect.may_exit;
    effect
}

fn case_exit_effect(
    program: &RecordedProgram,
    arms: RecordedCaseArmRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let arms = program.case_arms(arms);
    let mut effect = if arms.iter().any(|arm| arm.matches_anything) {
        ExitEffect::default()
    } else {
        ExitEffect::continuing()
    };

    for arm in arms {
        effect.combine_alternative(sequence_exit_effect(
            program,
            arm.commands,
            terminating_call_spans,
        ));
    }

    effect
}

fn always_exit_effect(
    program: &RecordedProgram,
    body: RecordedCommandRange,
    always_body: RecordedCommandRange,
    terminating_call_spans: &FxHashSet<SpanKey>,
) -> ExitEffect {
    let body_effect = sequence_exit_effect(program, body, terminating_call_spans);
    let always_effect = sequence_exit_effect(program, always_body, terminating_call_spans);
    let mut effect = ExitEffect::default();

    if body_effect.may_continue {
        effect.combine_alternative(always_effect);
    }
    if body_effect.may_return {
        effect.may_return = true;
    }
    if body_effect.may_exit {
        effect.may_exit = true;
    }

    effect
}
