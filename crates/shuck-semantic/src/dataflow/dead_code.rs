use super::*;

/// Groups unreachable CFG blocks into user-facing dead-code regions.
///
/// The CFG already marks blocks made unreachable by syntax like `return`,
/// `exit`, or always-false branches. This layer turns those blocks back into
/// source spans and collapses nested spans so diagnostics point at the outer
/// unreachable command rather than every token within it:
///
/// ```sh
/// return
/// echo never-runs
/// ```
pub(super) fn build_dead_code(cfg: &ControlFlowGraph) -> Vec<DeadCode> {
    let mut dead_code_by_cause: FxHashMap<
        (usize, usize, UnreachableCauseKind),
        (crate::cfg::UnreachableCause, Vec<Span>),
    > = FxHashMap::default();
    for block_id in cfg.unreachable() {
        let block = cfg.block(*block_id);
        if block.commands.is_empty() {
            continue;
        }
        let cause =
            cfg.unreachable_cause(*block_id)
                .unwrap_or_else(|| crate::cfg::UnreachableCause {
                    span: block.commands[0],
                    kind: UnreachableCauseKind::ShellTerminator,
                });
        dead_code_by_cause
            .entry((cause.span.start.offset, cause.span.end.offset, cause.kind))
            .or_insert_with(|| (cause, Vec::new()))
            .1
            .extend(block.commands.iter().copied());
    }
    let mut dead_code = dead_code_by_cause
        .into_iter()
        .map(|(_, (cause, unreachable))| DeadCode {
            unreachable: outermost_unreachable_spans(unreachable),
            cause: cause.span,
            cause_kind: cause.kind,
        })
        .collect::<Vec<_>>();
    dead_code.sort_by_key(|dead| (dead.cause.start.offset, dead.cause.end.offset));
    dead_code
}

fn outermost_unreachable_spans(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by(|left, right| {
        left.start
            .offset
            .cmp(&right.start.offset)
            .then_with(|| right.end.offset.cmp(&left.end.offset))
    });

    let mut outermost = Vec::new();
    for span in spans {
        if outermost
            .iter()
            .any(|outer| span_contained_by(span, *outer))
        {
            continue;
        }
        if outermost.contains(&span) {
            continue;
        }
        outermost.push(span);
    }
    outermost
}

fn span_contained_by(inner: Span, outer: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}
