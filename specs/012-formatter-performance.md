# 012: Formatter Performance Refactor

## Status

Partially Implemented

## Summary

A staged refactor plan for `crates/shuck-formatter` that closes the remaining wall-time gap against `shfmt` without changing Shuck's public formatter API or relaxing its correctness guarantees. The plan targets the measured bottlenecks that remain after Phase 1: the document/printer fallback for nested `$()` formatting, repeated late-stage span and comment attachment heuristics, and secondary parser/CLI setup costs.

The plan keeps `format_source(source, path, options) -> Result<FormattedSource>` and `format_file_ast(source, file, path, options)` as the public entry points. It intentionally prioritizes changes by impact and risk rather than architectural neatness: direct-write rendering landed first, nested formatter unification comes next, parser-integrated comment anchoring follows once the hot path is cheaper, and parser/CLI reuse cleanup comes last.

## Motivation

The current formatter is functionally usable and increasingly close to `shfmt` in output shape, but it is still materially slower on the shared benchmark corpus.

Initial measurements that motivated this plan showed:

| Benchmark | Shuck | shfmt | Gap |
|---|---:|---:|---:|
| Full formatter corpus (`all`) | `76.9 ms` | `11.1 ms` | `6.95x` |
| `nvm.sh` macro benchmark | `68.7 ms` | `6.9 ms` | `9.95x` |
| Parser-only Criterion (`all`) | `13.6 ms` | n/a | n/a |
| Parse + format Criterion (`all`) | `66.6 ms` | n/a | n/a |
| Comment-index Criterion (`all`) | `0.58 ms` | n/a | n/a |

Phase 1 has already landed since those measurements. It moved the streaming formatter's hottest word, assignment, redirect-target, pattern, and var-ref rendering paths onto reusable buffers and direct writes. The follow-up benchmark run showed roughly `4-8%` throughput wins on the `formatter_source/nvm` and `formatter_source/all` Criterion benches, with macro wall time improving to roughly `53.9 ms` on `nvm.sh` and `71.4 ms` on the full formatter corpus.

That improvement matters, but it did not change the overall ranking of the remaining bottlenecks:

- **The parser matters, but it is still not the main remaining gap.** Parser-only time is meaningful, yet it is smaller than Shuck's full format time.
- **Comment indexing alone is not the problem.** The standalone comment benchmark is cheap. The expensive part is the repeated comment- and span-based decision-making performed while formatting.
- **The formatter hot path still dominates.** After Phase 1, the largest remaining cost is the nested `$()` fallback, followed by layout/comment heuristics that still run in inner formatting loops.
- **Parsing with comments attached is necessary but not sufficient.** Moving comment ownership earlier will simplify the architecture, but it still needs the Phase 2 streaming unification to pay off fully.

By contrast, `shfmt` reuses parser and printer objects, attaches comments during parse, and writes formatted output directly to buffered writers. The goal of this refactor is not to copy `shfmt`'s implementation, but to move Shuck toward the same performance shape:

1. One cheap formatting engine on the hot path.
2. Fewer repeated span scans.
3. Earlier ownership of comments and trivia.
4. Less per-file setup overhead once the formatter itself is cheaper.

## Design

### Goals

- Reduce macro formatter wall time on the shared corpus from the current post-Phase-1 baseline toward `<= 2x` the installed `shfmt` binary on the same machine.
- Preserve the current public formatter API and option surface.
- Preserve clean-room independence and do not depend on `shfmt` internals at runtime.
- Keep correctness-first fallbacks available while intermediate phases land.
- Make each remaining phase independently benchmarkable and reversible.

### Non-Goals

- This spec does not change formatter policy or parity goals beyond performance-driven refactors.
- This spec does not replace Shuck's parser with `shfmt` or any external parser.
- This spec does not remove all verbatim fallbacks in one step.
- This spec does not change lint or semantic-model architecture except where formatter metadata needs to be surfaced from the parser.

### Remaining Bottlenecks After Phase 1

| Bottleneck | Evidence | Why it hurts |
|---|---|---|
| Nested `$()` formatting still uses the old document/printer path | `render_command_substitution` in `word.rs` builds a fresh `ShellFormatContext`, generic `Formatter`, and printer result for each substitution | Expensive on large scripts with many substitutions; `nvm.sh` contains hundreds of `$()` sites |
| Late comment and span heuristics still run during formatting | `streaming.rs` builds attachment spans, calls `attach_sequence`, checks ambiguity, and derives some inline-layout decisions while formatting | Keeps span analysis in inner loops and repeats work across nested sequences |
| Parser and CLI setup costs are not amortized | Parser creation, path/config resolution, and file-mode orchestration still repeat per file | Smaller than the main formatter cost, but increasingly visible once Phases 1 and 2 are done |

### Target Architecture

The target design keeps the current public API while changing the internal organization:

```
source text
   |
   v
parse + formatter metadata
   |
   +--> AST
   |
   +--> formatter facts
         - attached comments / trivia anchors
         - statement sequence layout facts
         - padding-sensitive spans
         - explicit line-break facts
         - safe verbatim regions
   |
   v
single streaming formatter
   |
   +--> direct-write node renderers
   +--> shared scratch buffers only where needed
   +--> nested $() stays on the same engine
   |
   v
output buffer
   |
   v
trailing-newline normalization + unchanged check
```

This is intentionally not "parse comments into final printer nodes." Shuck still benefits from a structured formatter layer that can support simplification, dialect-specific rules, and safe fallbacks. The key change is that comment ownership and layout facts are computed once and consumed cheaply, rather than rediscovered while printing.

### Phase Plan

#### Phase 1: Eliminate Hot-Path Temporary Strings

Status: Implemented.

This phase reduced allocation pressure without changing the parser or comment model.

##### Landed Changes

- Added `_to_buf` or direct-write helpers for hot-path syntax rendering.
- Kept string-returning helpers as compatibility wrappers for non-hot-path call sites.
- Added reusable scratch storage to `ShellStreamFormatter`.
- Updated the streaming formatter to reuse rendered command names, redirect targets, patterns, assignments, and related syntax fragments instead of allocating fresh `String` values per node.

##### Outcome

- Lower allocator pressure on large files.
- Smaller constant-factor cost in the streaming formatter.
- Useful wins in Criterion and macro benchmarks, but not enough to close the gap by themselves.

#### Phase 2: Remove the Nested Document/Printer Fallback for `$()`

This phase unifies nested formatting under the streaming engine.

##### Current State

`render_command_substitution` in `crates/shuck-formatter/src/word.rs` still formats substitution bodies by creating a fresh `ShellFormatContext`, running the generic document formatter, printing the result, trimming trailing newlines, and then wrapping or indenting the rendered text. That means normal `$()` formatting still pays for a second formatter stack inside the streaming formatter.

##### Changes

- Replace the current command substitution path that creates a separate `ShellFormatContext`, generic `Formatter`, and printer result.
- Add a streaming subformatter path for `$()` bodies that:
  - formats nested statement sequences with the same formatter engine,
  - writes into a temporary nested output buffer only once,
  - applies indentation and newline wrapping after formatting, not by round-tripping through the old IR printer.
- Prefer an internal API shape like:

```rust
fn format_command_substitution(
    &mut self,
    body: &StmtSeq,
    multiline: bool,
) -> Result<()>;
```

- If borrow-checking makes in-place nested formatting awkward, add a scoped nested-buffer helper rather than reviving the full document/printer stack.
- Keep a correctness fallback only for constructs that the streaming engine still cannot handle structurally.

##### Constraints

- Preserve existing empty-body behavior: an empty formatted body still renders as `$()`.
- Preserve current multiline wrapping semantics:
  - single-line bodies stay `$(...)` without extra trailing newlines,
  - multiline bodies still emit `$(`, an indented body, and `)` on its own line.
- Preserve safety around heredocs and other constructs that may still need verbatim handling inside substitutions.
- Do not expand this phase into general comment-attachment redesign; it should reuse the current comment model until Phase 3.

##### Expected Impact

- Significant gains on scripts with many command substitutions.
- Removes one entire layer of duplicated formatter machinery from the normal hot path.
- Makes subsequent profiling easier because the streaming formatter becomes the overwhelmingly dominant formatting engine.

##### Acceptance Criteria

- The largest improvements appear on `$()`-heavy fixtures such as `nvm`.
- The normal `$()` path no longer creates the generic document/printer pipeline.
- Existing formatter output stays stable aside from already-accepted parity differences.
- Regressions cover:
  - empty substitutions,
  - single-line substitutions,
  - multiline substitutions,
  - substitutions containing compound commands,
  - substitutions adjacent to redirects, comments, and heredocs.

#### Phase 3: Introduce Formatter Facts and Early Comment Anchoring

This phase moves repeated attachment and layout decisions out of the printing hot path.

##### Current State

The streaming formatter still computes or re-derives several layout decisions while walking the AST:

- per-sequence attachment spans,
- leading/trailing/dangling comment assignments,
- ambiguity checks,
- some line-gap and explicit-break decisions,
- padding- and verbatim-related span queries.

That work is concentrated in `crates/shuck-formatter/src/streaming.rs`, `crates/shuck-formatter/src/comments.rs`, and the span/layout helpers consumed from `command.rs`.

##### Changes

- Add a `FormatterFacts` sidecar, built once per file before printing.
- Populate it with:
  - comment ownership and anchors,
  - sequence-level leading/trailing/dangling comment assignments,
  - ambiguity flags,
  - explicit line-break facts,
  - alignment-sensitive padding regions,
  - safe verbatim spans.
- Update `ShellStreamFormatter` to consume these facts instead of recalculating span relationships mid-format.
- Move "should this stay verbatim?" and "where do these comments belong?" decisions as far upstream as practical.
- Where practical, surface parser-produced comment/trivia data directly into the fact builder rather than reconstructing it from source slices after the fact.

##### Possible Internal Shape

`FormatterFacts` does not need to be public. A likely internal shape is:

```rust
struct FormatterFacts<'source> {
    comments: AttachedComments<'source>,
    sequence_facts: SequenceFacts,
    padding_regions: PaddingRegions,
    verbatim_regions: VerbatimRegions,
    line_break_facts: LineBreakFacts,
}
```

This spec does not require a particular representation, only that the facts are:

- computed once per file,
- queried cheaply during formatting,
- easier to test in isolation than the current interleaved heuristics.

##### Why This Phase Is Not First

Comment anchoring is architecturally important, but it was not the highest-ROI first change. The benchmark split showed that Phase 1's string churn and Phase 2's nested formatting overhead were more immediate cost centers. Landing this phase after the formatter hot path is cheaper keeps the blast radius manageable and makes its impact easier to measure.

##### Expected Impact

- Cleaner internal architecture.
- Fewer repeated span scans and attachment checks during formatting.
- Easier reasoning about ambiguity and verbatim fallback boundaries.
- A clearer path to parser-provided trivia in the future without hard-coupling the public formatter API to parser internals.

##### Acceptance Criteria

- Formatting code no longer builds per-sequence attachment-span vectors in inner formatting loops.
- The fact builder runs once per file.
- The streaming formatter queries fact tables rather than re-running attachment analysis ad hoc.
- New regressions cover comments around continuations, branch boundaries, substitutions, and heredocs.

#### Phase 4: Parser and CLI Reuse Cleanup

This phase captures the remaining secondary costs after the formatter hot path is cheaper.

##### Current State

Shuck still pays some per-file setup cost in the parser boundary and CLI layer:

- parser state is constructed per format call,
- project-root and config resolution still happen in file-mode orchestration,
- file grouping and formatting setup repeat across batches,
- any formatter-specific source metadata still has to be rebuilt per call.

These costs were not the best first target, but they will matter more once Phases 2 and 3 reduce inner-loop formatter work.

##### Changes

- Reuse parser state where the parser crate API permits it safely.
- Revisit `shuck format` and `shuck format-stdin` orchestration to reduce repeated setup work in file mode.
- Narrow remaining option-resolution and per-file setup costs.
- If profiling still justifies it after Phases 2 and 3, add a private reusable formatter session object that owns parser/fact-builder scratch state across files.
- Only if still warranted, extend parser output with formatter-specific trivia metadata rather than rebuilding it later.

##### Constraints

- Public APIs remain unchanged.
- Reuse must remain single-thread-safe and explicit; no hidden global mutable parser/printer state.
- CLI cleanup should not regress config precedence, dialect inference, or cache key correctness.

##### Expected Impact

- Smaller but meaningful end-to-end improvements once the formatter dominates less.
- Better large-batch formatting throughput across many files.
- Less overhead when formatting many small files, where orchestration and parser setup are a larger fraction of total time.

##### Acceptance Criteria

- Macro benchmarks improve further after Phases 2 and 3 have landed.
- Profiling shows reduced time outside the formatting engine itself.
- Existing CLI/config integration coverage stays green.

### Detailed Refactor Notes

#### 1. Rendering API Direction

The formatter should standardize on one of two patterns:

- **Direct write:** the node renderer writes directly into `ShellStreamFormatter`.
- **Scratch buffer:** the node renderer appends to a caller-provided buffer that is reused across calls.

Returning owned `String` values from hot-path helpers should remain the exception, not the default.

#### 2. Compatibility and Fallbacks

Each remaining phase must preserve the ability to fall back to verbatim output when the formatter cannot yet normalize a construct safely. The refactor is about making the common path cheap, not about deleting safety rails.

#### 3. API Stability

The public formatter entry points remain:

```rust
pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource>;

pub fn format_file_ast(
    source: &str,
    file: File,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource>;
```

Callers should not need to know whether the internals use a fact builder, attached comments, scratch buffers, a reusable parser session, or a streaming-only engine.

### Rollout Strategy

Phase 1 already landed. The remaining work should land as separate, benchmarked PRs:

1. `$()` streaming unification.
2. Formatter facts and early comment anchoring.
3. Parser/CLI reuse cleanup.

Each PR should:

- include before/after Criterion numbers,
- include before/after macro wall times on at least `nvm.sh` and `all`,
- avoid unrelated parity changes,
- keep new test coverage focused on the touched behavior.

## Alternatives Considered

### Only Parse Comments Earlier

Rejected as a standalone plan. It improves the architecture, but it does not address the main remaining hot-path cost of the nested `$()` fallback. This is part of the solution, not the whole solution.

### Optimize the Parser First

Rejected as the next phase. Parser cost is meaningful, but the benchmark split still shows the formatter itself is the larger remaining problem. Improving parser throughput before reducing formatter overhead would likely produce smaller end-to-end wins.

### Replace the Formatter with `shfmt`

Rejected. Shuck's formatter must share Shuck's parser, dialect handling, simplification passes, and safety fallbacks. Replacing it with `shfmt` would give up the single-parser architecture and move the project away from its own formatter surface.

### Keep the Current Architecture and Chase Micro-Optimizations

Rejected as insufficient. Small wins in branch cleanup or residual allocation tuning may help, but they will not remove the structural costs of invoking a second formatter engine inside `$()`, rediscovering comment ownership, or repeating per-file setup work.

## Security Considerations

This refactor does not add new trust boundaries, network access, or external execution. The main safety risk is semantic corruption from over-aggressive normalization. Each phase must preserve correctness-first fallbacks and keep regression tests for heredocs, comments, substitutions, and dialect-sensitive constructs.

## Verification

### Benchmark Verification

Run the formatter and parser micro-benchmarks:

```bash
cargo bench -p shuck-benchmark --bench parser -- --noplot
cargo bench -p shuck-benchmark --bench formatter -- --noplot
```

Run the formatter macro benchmarks against the installed `shfmt` in the Nix shell:

```bash
make bench-macro-format
```

For focused one-off runs:

```bash
make bench-macro-format-single BENCH_FILE=/absolute/path/to/script.sh
```

For comparison against a locally built `shfmt` checkout, put that binary first on `PATH` before running the formatter benchmark helpers. In this workspace, the local checkout lives at `/Users/ewhauser/working/shfmt`.

### Correctness Verification

Run formatter tests and oracle coverage:

```bash
cargo test -p shuck-formatter
make test-oracle-shfmt-fixtures
make test-oracle-shfmt-benchmark
```

### Profiling Verification

Use the profiling helpers to confirm that time moves out of the targeted hot paths:

```bash
make profile-formatter PROFILE_CASE=nvm
make flame-formatter PROFILE_CASE=nvm
```

### Exit Criteria

This refactor is considered successful when all of the following are true:

- The shared formatter corpus macro benchmark is no longer dominated by nested `$()` fallback work or repeated attachment/layout recomputation.
- The formatter architecture uses a single normal hot path for nested command substitutions.
- Comment and layout ownership are computed once per file rather than repeatedly rediscovered while formatting.
- End-to-end wall time on the shared formatter corpus is materially closer to `shfmt` than today's post-Phase-1 `~6-10x` gap.
