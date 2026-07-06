# 025: Cross-File Call Hierarchy

## Status

Proposed

## Summary

Extend the single-file LSP call hierarchy (spec 023's navigation family, plus
the `prepareCallHierarchy` / `incomingCalls` / `outgoingCalls` engine) so that
functions reached through a `follow-source` hint (spec 024) participate in the
hierarchy. When a script sources a helper by a computed path and annotates it
with `# shuck: follow-source=lib/util.sh`, "outgoing calls" from a function
should descend into `util.sh`, and a function *defined in* `util.sh` should offer
its call sites back in the caller as "incoming calls". This is the payoff that
specs 023 (engine) and 024 (resolution) were built to enable, and the concrete
ask in issue #1144.

## Motivation

Today the two foundations exist but do not meet:

- **Spec 024** resolves computed `source` paths via `assume-source` /
  `follow-source`, and `follow-source` already lints the target during
  `shuck check`. But that following is CLI-only; the LSP server never sees it.
- **Spec 023 / call hierarchy** answers incoming/outgoing calls, but only within
  one file: the server builds each document's semantic model with
  `resolve_source_closure: false` (`crates/shuck-server/src/analysis.rs`,
  `.../editor.rs`), and every `CallHierarchyItem` is addressed with the current
  document's URI (`crates/shuck-server/src/editor_features.rs`,
  `to_lsp_call_hierarchy_item`).

The result: a cursor on `main` that calls `helper` (defined in a followed file)
gets no outgoing edge to `helper`, and a cursor on `helper` gets no incoming edge
from `main`. For real shell projects — plugin loaders, relative includes — this
is exactly where navigation matters most. `follow-source` is the user-supplied
bridge across the computed-path gap; this spec teaches call hierarchy to walk it.

## Design

### Directive scope: follow, not assume

Only `follow-source` targets join the call hierarchy. `assume-source` remains
"import symbols to resolve references" and deliberately does **not** contribute
call sites or definitions to the hierarchy. This matches the split chosen in 024
and the issue wording ("fully parse for lsp hierarchy" vs. "just use to
disambiguate missing symbols"):

| Directive | References resolve | Symbols in completion/hover | In call hierarchy |
| --- | --- | --- | --- |
| `assume-source` | yes | yes | no |
| `follow-source` | yes | yes | **yes** |

### The asymmetry: outgoing is natural, incoming is not

Following a file's sources is a *directed* operation, and call hierarchy's two
directions are not symmetric under it:

- **Outgoing** ("what does `F` call") is naturally answerable. From `F`'s file we
  follow its `follow-source` edges, parse the targets, and resolve `F`'s call
  tokens against the union of function definitions in the file plus its followed
  closure. Everything needed is reachable by walking *outward* from `F`.
- **Incoming** ("who calls `F`") is not. If `F` is defined in `util.sh`, a caller
  lives in some file `B` that does `follow-source=util.sh` — but nothing in
  `util.sh` or its own closure points back to `B`. Discovering `B` requires a
  *reverse* index: which workspace files follow/source/call into this one. The
  closure does not provide that.

This asymmetry drives the phasing below. v1 delivers complete outgoing edges and
the incoming edges that are reachable within the *querying document's* closure;
workspace-wide incoming is deferred to a reverse-index phase.

### Semantic layer: retain the followed closure

The closure today (`crates/shuck-semantic/src/source_closure/`) parses each
sourced file to extract *contracts* (imported bindings/functions) and then
discards the per-file model — so call sites and function-body scopes from
followed files survive nowhere. Two ways to fix that:

- **(A) Merged multi-file model.** Build one `SemanticModel` spanning the whole
  follow closure, with call sites and scopes from every file in a shared arena.
  Cleanest query surface, but a large, invasive change to model construction and
  every span→file mapping.
- **(B) Retained per-file models + a cross-file overlay** *(recommended)*. Keep
  today's per-file models, but for `follow-source` edges retain the target's
  model (or a compact "call-facts" projection: function-definition bindings with
  spans, and resolved call sites with callee + name-span + enclosing function)
  keyed by resolved path. A `CrossFileCallGraph` overlay links a call token to a
  function definition in another retained file. Call hierarchy queries walk the
  overlay; single-file queries are unchanged.

Recommend (B): it is additive, leaves the existing single-file path untouched,
and mirrors how the closure already parses each file once. The projection keeps
retention cheap (no need to hold whole models if only call facts are needed).

New semantic surface (sketch):

```rust
// A function-definition target in a specific file of the closure.
pub struct CrossFileFunction { pub path: PathBuf, pub binding: BindingId, pub name: Name, ... }

impl EditorQuery<'_> {
    // Outgoing edges that resolve into followed files, keyed by target file.
    fn cross_file_outgoing(&self, item: &EditorCallHierarchyItem, closure: &FollowClosure) -> Vec<EditorOutgoingCall>;
    // Incoming edges discoverable within the querying document's own closure.
    fn cross_file_incoming(&self, item: &EditorCallHierarchyItem, closure: &FollowClosure) -> Vec<EditorIncomingCall>;
}
```

The existing `EditorCallHierarchyItem` gains a file identity (its resolved path)
so items can address functions outside the active document; `EditorIncomingCall`
/ `EditorOutgoingCall` already carry the call spans.

### Server layer: enable the closure and address by file

1. **Turn the closure on for call hierarchy.** The LSP analysis path currently
   forces `resolve_source_closure: false`. Enable it for the call-hierarchy
   requests (either always, or lazily when the document contains `follow-source`
   hints, to avoid paying for closure resolution on documents that have none).
   This is the one place 024's resolution feeds the server.
2. **Read followed targets.** Followed files may not be open buffers. The server
   resolves each `follow-source` path (reusing 024's `NativeSourceResolver` logic,
   promoted to a shared crate so both `shuck check` and the server use one
   resolver) and reads the file — preferring the open in-memory buffer when the
   editor has it, falling back to disk. Transitive follows use a visited set, as
   in the CLI.
3. **Address items by their file.** `to_lsp_call_hierarchy_item` must stamp each
   item with the *target file's* `Url`, not the active document's. Items for
   followed files carry their resolved path's URI; the round-trip `data` payload
   (currently a position in the current doc) becomes `{ uri, line, character }`.
4. **Advertise nothing new.** `callHierarchyProvider` is already advertised by
   spec 023's work; this spec only widens what the existing requests return.

### Phasing

- **Phase 1 — outgoing cross-file.** Retain follow-closure call facts; answer
  outgoing calls that descend into followed files; address items by file. This
  alone makes "step into the helper's function" work and is the bulk of the
  value.
- **Phase 2 — closure-local incoming.** Answer incoming calls whose callers live
  in files already in the querying document's follow closure. (Complete for the
  common "main follows helpers" shape.)
- **Phase 3 — workspace incoming (optional / later).** A workspace-wide reverse
  index (which files follow/call which) to answer "who calls `F`" across files
  that the current document does not itself follow. Larger; may reuse the
  workspace-symbol indexing machinery. Explicitly out of scope for the first PR.

### Caching and invalidation

The document analysis cache must track the resolved follow-target paths as
dependencies, so editing a followed file invalidates the caller's call-hierarchy
results. This mirrors `imported_dependency_paths` already produced by the closure
and the `dependency_paths` the CLI check path records.

## Alternatives Considered

### Merged multi-file model (approach A above)

Rejected for v1. A single arena spanning the closure gives the simplest query
surface, but it is a deep change to model construction, span ownership, and every
consumer that assumes one model = one file. The overlay (B) delivers the same
navigation with an additive change; a merged model can come later if multiple
features need it.

### Feed `assume-source` into the hierarchy too

Rejected. It would blur the 024 distinction users just gained: `assume-source`
means "I only want symbols resolved, don't pull this file in." Overriding that to
also add call edges removes the cheap "just silence the noise" option.

### Workspace-index-first (incoming before outgoing)

Rejected as the starting point. Incoming across the workspace needs a reverse
call index that does not exist yet, while outgoing is reachable today via the
follow closure. Shipping outgoing first delivers most of the value at a fraction
of the cost and defers the index to when incoming demands it.

### Always resolve the closure in the LSP path

Considered. Simpler than gating on the presence of `follow-source` hints, but it
makes every hover/definition/symbol request pay closure-resolution cost even for
documents with no hints. Prefer lazy enablement keyed on hint presence, measured
against the latency budget from spec 018.

## Security Considerations

Enabling the closure in the server means the LSP server **reads files named by
the document under edit** (the `follow-source` targets), transitively. This is the
same trust posture as 024's CLI following and ShellCheck's `-x`, now in an
always-on editor process:

- Resolution stays confined to the annotating file's directory plus configured
  `source-paths`; targets outside those roots are not followed.
- The server only reads and parses; it never executes sourced content.
- Transitive following uses a visited set to bound fan-out and prevent cycles.
- Reads prefer open buffers, so unsaved editor state is honored over stale disk
  content and the server does not race the user's edits.

## Verification

- **Semantic** (`shuck-semantic`): given a caller model with a `follow-source`
  edge to a helper defining `greet`, `cross_file_outgoing` for a function that
  calls `greet` returns an edge whose target item carries the helper's path; and
  a helper-local query surfaces the caller's call span via `cross_file_incoming`
  when the caller is in the closure.
- **Server** (`shuck-server`): a black-box LSP session over two files — `main.sh`
  with `# shuck: follow-source=helper.sh` and `helper.sh` defining `greet` —
  where `prepareCallHierarchy` on `main`'s call to `greet` plus `outgoingCalls`
  returns an item with `helper.sh`'s URI, and `incomingCalls` on `greet` returns
  `main`'s call site. Verify the followed file is read from disk when not open and
  from the buffer when open.
- **Invalidation**: editing `helper.sh` (buffer change) updates the caller's
  outgoing/incoming results without a server restart.
- **Scope guard**: an `assume-source` edge does *not* produce cross-file call
  edges.
- **Regression**: single-file call hierarchy (spec 023) and all 024 behaviors are
  unchanged; `make test` and the black-box LSP suite stay green.

Clean-room: all directive names, types, and documentation are authored in-repo;
no ShellCheck source or wiki text is referenced.
