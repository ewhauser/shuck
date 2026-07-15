# 025: Cross-File Call Hierarchy

## Status

Implemented (v1).

Delivered: the workspace call-graph index (`FileCallFacts` / `WorkspaceCallIndex`
in `shuck-semantic`), the shared source-edge resolver, and session-scoped
`incomingCalls` / `outgoingCalls` handlers that build the index over open
buffers plus discovered workspace files (plus files reachable only through
resolved source edges, e.g. gitignored vendored targets) and answer both
directions across files. The built index is cached on the session behind an
epoch counter and invalidated by any document, workspace, or configuration
change, so expanding a call tree reuses one build. Call sites the semantic
model binds in-file stay binding-accurate (definition order and shadowing are
honored), while source edges retain their positions so later sourced and local
definitions override earlier ones; only sites without an effective local
binding are matched by name across source edges. Top-level MODULE nodes
round-trip through `CallHierarchyItem.data`, so
their outgoing calls expand. Edges come from all determinable sources
(literal-resolvable paths and `source=` directives with and without
`lint=true`), resolved against the annotating file's own directory and the
configured `[lint] source-paths` roots (memoized per project root, matching
`shuck check`). The indexed-file count is hard-bounded by
`server.callHierarchy.maxFiles` (default 10k): one budget is shared by all
three population phases — open buffers, closed-file discovery, and source-edge
expansion — and every insertion checks it, so a runaway workspace degrades to a
partial graph (with a warning) rather than an unbounded scan. Covered by
semantic unit tests, an index-size bound test, and black-box multi-file LSP
tests (including one that resolves only via `source-paths`).

Known limitation (noted follow-up, not blocking): nodes are keyed by function
name within a file, so two same-named definitions in one file collapse onto the
first.

## Summary

Make LSP call hierarchy complete across files. Building on the single-file
engine (spec 023's `prepareCallHierarchy` / `incomingCalls` / `outgoingCalls`)
and the computed-source resolution from spec 024 (the
`# shuck: source=` directive), this spec introduces a **workspace call-graph index**: for
every shell file the server knows about, the functions it defines, the call sites
it contains, and the source edges that connect files. Both directions of the
hierarchy — "what does this function call" and "who calls this function" — are
answered as symmetric traversals of that one index, across the whole workspace,
not just the file under the cursor. This is the payoff specs 023 and 024 were
built to enable and the concrete ask in issue #1144.

## Motivation

The two foundations exist but do not meet, and the naive way to join them is
wrong:

- **Spec 024** makes computed `source` paths statically determinable via hints,
  but that resolution is confined to the CLI and to a file's own outward closure.
- **Spec 023 / call hierarchy** answers incoming/outgoing, but per file: the
  server builds each document with `resolve_source_closure: false`
  (`crates/shuck-server/src/analysis.rs`, `.../editor.rs`) and stamps every
  `CallHierarchyItem` with the active document's URI
  (`crates/shuck-server/src/editor_features.rs`).

A tempting shortcut is to answer cross-file calls by walking only the *querying
document's* follow closure. That gives correct **outgoing** edges (a file's own
sources are reachable outward) but **incorrect incoming** edges: "who calls `F`"
where `F` is defined in `util.sh` depends on every workspace file that sources
`util.sh` — information absent from `util.sh` or its closure. A call hierarchy
that silently under-reports callers is a refactoring hazard. Incoming must be
complete or it should not ship; therefore the design commits to a workspace-wide
index from the start, which — usefully — also makes outgoing fall out of the same
structure.

### What "complete" covers

"Complete" here means complete over the **statically-resolvable call graph**.
Shell also dispatches at runtime — `eval`, indirect expansion, `$1`-driven
`case` dispatch, or sourcing a path that stays unresolvable even with a hint —
and those edges cannot be known without running the script. They are out of
scope. Call hierarchy returns every edge the resolver can determine (the 024
hints exist to widen that set) and never returns *partial* results within the
resolvable graph; it simply does not model runtime-only dispatch.

## Design

### Which source edges resolve

A call site in file `B` resolves to a function `F` defined in file `A` when `B`
can statically be shown to source `A` (directly or transitively) and no nearer
definition of `F` shadows it. "Statically shown to source" covers every
*determinable* source edge, not only linted (`lint=true`) edges:

| Source form in `B` | Contributes a resolvable edge `B → A`? |
| --- | --- |
| Literal path that resolves on disk (`source ./lib/a.sh`) | yes |
| Computed path with `# shuck: source=a.sh` (resolves) | yes |
| Computed path with `# shuck: source=a.sh lint=true` (resolves) | yes |
| Computed path, no hint, unresolvable | no (runtime-only) |
| `source=/dev/null` | no (explicitly nothing) |

**Decision:** the call graph uses *all* determinable edges — resolvable literal
sources and directive-asserted edges with or without `lint=true`. Incoming
completeness, the property this spec exists to guarantee, requires counting every
real caller, so a caller whose directive omits `lint=true` must contribute an edge
just as a linted one does. Within the workspace graph the lint policy is
therefore equivalent; this supersedes the 023-era rule that only `follow`
participates. The `assume` vs `follow` difference persists where it began — in
per-document analysis cost and in whether `shuck check` lints the target (spec
024) — not in graph membership. Hinted edges are the whole point: computed source
lines are exactly where the resolvable graph would otherwise have holes, and the
hints fill them.

### The workspace call-graph index

The core new structure. Over the shell files in the server's workspace folders:

```
WorkspaceCallIndex
  files: Map<Path, FileCallFacts>            // one entry per indexed file
  ...
FileCallFacts
  definitions: [{ name, binding, def_span, selection_span }]   // functions defined here
  call_sites:  [{ callee_name, name_span, enclosing_function }] // calls made here
  source_edges:[{ resolved_path, span }]      // determinable sources in execution order
```

Cross-file resolution layered on top:

- `resolve(call_site in B) -> Option<(A, definition)>`: combine `B`'s visible
  local definition with its transitive `source_edges` (visited-set bounded) in
  shell execution order. Edges after a top-level call do not participate; later
  sourced or local definitions override earlier ones.
- `outgoing(F in A)`: for each call site enclosed by `F`, `resolve` it; group by
  target definition. (Targets may be in `A` or any followed file.)
- `incoming(F in A)`: scan the index for call sites whose `resolve` lands on
  `F` — i.e. files that transitively source `A` and call `F.name` without a
  nearer shadow. Both directions are the same edge set traversed opposite ways.

The index is what makes incoming complete: it holds every file's call sites and
source edges, so the reverse lookup is a scan over known data rather than an
impossible walk outward from the callee.

### Construction, laziness, and invalidation

- **Population.** Reuse the workspace-symbol discovery that already enumerates
  shell files (spec 023's `workspace/symbol` path) to seed the file set. Each
  file's `FileCallFacts` is a cheap projection of its parsed model — function
  definition bindings, call sites, and resolved source edges (the last via 024's
  resolver, promoted to a shared crate so `shuck check` and the server share one
  implementation). No full semantic model is retained; only the projection.
- **Laziness.** Build on first call-hierarchy request, not at startup, to stay
  within spec 018's latency budget. Index build is parallel over files.
- **Incremental invalidation.** On `didChange`/`didSave`/watched-file events,
  reproject only the changed file and drop cached resolutions that touched it.
  Open buffers shadow on-disk content, so unsaved edits are honored. A file's
  entry lists its `source_edges`, giving the reverse dependency needed to know
  which callers' results to invalidate.

### Server layer

1. **Feed determinable edges from 024.** Each file's `source_edges` come from the
   shared resolver; this is where 024's hint resolution reaches the server.
2. **Read files by preference.** A file in the index may be an open buffer or
   on disk; prefer the buffer, fall back to disk. The server thus reads workspace
   shell files it did not open — see Security.
3. **Address items by file.** `to_lsp_call_hierarchy_item` stamps each item with
   its *own* file's `Url`; the round-trip `data` payload becomes
   `{ uri, line, character }` so incoming/outgoing can re-resolve an item in any
   file, not just the active document.
4. **No new capability.** `callHierarchyProvider` is already advertised; this
   spec only widens what the three existing requests return.

### Build sequencing (all ships in one feature; this is internal order)

1. Shared source resolver crate (extract 024's `NativeSourceResolver`).
2. `FileCallFacts` projection in `shuck-semantic` + per-file unit coverage.
3. `WorkspaceCallIndex` with cross-file `resolve`, `outgoing`, `incoming`.
4. Server wiring: lazy build, buffer/disk reads, per-file item URIs,
   invalidation on document/watched-file events.
5. Black-box multi-file LSP tests for both directions, including an incoming
   caller that the queried file does not itself source.

## Alternatives Considered

### Closure-scoped overlay (outgoing-complete, incoming-partial)

Rejected — this was the earlier draft of this spec. Retaining only the querying
document's follow closure answers outgoing correctly but incoming only for
callers inside that closure, silently missing callers elsewhere in the
workspace. Partial incoming in a navigation tool is a correctness bug, and the
user requirement is explicit: both directions complete, or neither.

### Lint-only edges (exclude import-only directives from the graph)

Rejected. It would preserve the 023-era "linted targets participate, imported
ones do not" distinction crisply — cross-file hierarchy would require
`lint=true` everywhere and a plain `source=` would mean "symbols only, never in
the graph" — but it loses incoming completeness for any project whose caller
directives omit `lint=true`, which directly contradicts the completeness
requirement. All directive-asserted edges contribute; the lint policy survives
where it began — in per-document cost and CLI linting. If a future need arises
for "resolve symbols but stay out of the call graph," that is a new, separately
named mode, not a reinterpretation of the plain `source=` directive.

### Merged multi-file semantic model

Rejected for this feature. A single arena spanning the workspace would give the
richest query surface but is a deep change to model construction and every
span→file assumption. The index holds a compact projection instead and leaves
per-file models untouched; a merged model can come later if several features
need it.

### Eager full-workspace index at startup

Rejected. Indexing every shell file on server start burns latency for sessions
that never use call hierarchy. Lazy first-use build plus incremental maintenance
keeps the common path cheap.

## Security Considerations

Completing incoming calls requires the server to **index shell files across the
workspace**, and resolving edges means **reading files named by other files'
source statements** (including computed paths a hint makes determinable). This
widens the trust surface beyond spec 024's CLI following into an always-on editor
process:

- Indexing is confined to the server's workspace folders and to targets that
  resolve within a file's directory plus configured `source-paths`; paths outside
  those roots are not read.
- The server only reads and parses; it never executes shell content.
- Transitive resolution is visited-set bounded to prevent cycles and runaway
  fan-out.
- Open buffers are preferred over disk, so the server honors unsaved editor state
  and does not act on stale content.
- The index respects the same file-discovery exclusions as the rest of the
  server (gitignore, excluded dirs), so vendored or ignored trees are not walked.

## Performance Considerations

- The index is the new cost center. Build is lazy (first call-hierarchy request),
  parallel across files, and produces a compact projection rather than retained
  models.
- Incremental reprojection on edit touches one file; resolution caches are keyed
  so only dependent callers are invalidated.
- Measure against spec 018's latency budget: first-request index build on a
  representative repo, and steady-state incoming/outgoing latency with a warm
  index. Fall back to single-file results if index build exceeds a budget rather
  than blocking the request.

## Verification

- **Projection** (`shuck-semantic`): `FileCallFacts` for a file lists its
  function definitions, call sites with enclosing function, and resolved source
  edges (literal, `source=` directives with and without `lint=true`); `/dev/null` and unresolvable
  dynamic sources contribute no edge.
- **Index** (`shuck-semantic`): over a three-file graph `a.sh` (defines `greet`)
  ← `b.sh` (`source=a.sh lint=true`, calls `greet`) ← `c.sh` (`source=a.sh`,
  calls `greet`), `incoming(greet in a.sh)` returns both `b.sh` and `c.sh` call
  sites; `outgoing` from `b.sh`'s caller lands on `a.sh`'s `greet`. A nearer
  local `greet` in `b.sh` shadows the cross-file one.
- **Server** (`shuck-server`, black-box LSP): `prepareCallHierarchy` on a call to
  a followed function returns an item with the *target* file's URI;
  `outgoingCalls` descends into followed files; `incomingCalls` on a function
  returns callers **in files the queried file does not itself source**, proving
  the reverse index rather than a closure walk. Verify buffer-preferred reads and
  that editing a caller (buffer change) updates the callee's incoming results
  without restart.
- **Completeness guard**: a caller reachable only through an unresolvable dynamic
  source is *absent* (documented limitation), while the same caller with an
  `# shuck: source=` directive *appears* — demonstrating hints maximize
  the resolvable graph.
- **Regression**: single-file call hierarchy (023) and all 024 behaviors
  unchanged; `make test` and the black-box LSP suite green.

Clean-room: all names, types, and documentation are authored in-repo; no
ShellCheck source or wiki text is referenced.
