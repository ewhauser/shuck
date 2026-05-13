# 023: LSP Editor Features

## Status

Proposed

## Summary

Extend `shuck server` beyond diagnostics, quick fixes, directive hovers, and
document synchronization to cover the remaining editor-facing LSP features:
completion, symbol hover, go-to-definition, find references, occurrence
highlighting, document symbols, workspace symbols, rename, and formatting.
The implementation builds on spec 018 by adding a shared document-analysis
cache, editor-symbol indexes over Shuck's semantic model, conservative rename
sets, and advertised formatting support once the existing formatter handlers
are promoted from internal plumbing to a supported LSP surface.

## Motivation

Spec 018 established the server runtime, scheduling model, document snapshots,
settings resolution, diagnostics, code actions, and rule-code hover. That made
`shuck server` useful for lint feedback, but editors still cannot use Shuck as
the primary shell intelligence provider. Users must rely on unrelated editor
extensions for navigation, symbol lists, rename, completions, and formatting,
which means those features do not share Shuck's parser, dialect inference,
semantic model, source-closure data, config layering, or in-memory document
state.

Shuck already owns much of the hard information needed for these features:

- `shuck-semantic` records bindings, references, scopes, call sites, function
  definitions, source references, ambient contracts, and option-sensitive shell
  behavior.
- `shuck-indexer` owns line, comment, heredoc, and source-position indexes.
- `shuck-formatter` can format complete documents.
- `shuck-server` already has request scheduling, cancellation, pull/push
  diagnostics, code action resolution, workspace folders, and incremental text
  sync.

The goal of this spec is to expose that information through standard LSP
requests without duplicating shell analysis in the server crate. The server
should remain a thin editor protocol layer over parser, indexer, semantic,
linter, and formatter crates.

## Design

### Goals

- Advertise only features with implemented request handlers and black-box
  editor coverage.
- Reuse one parse/index/semantic pass per document version across diagnostics,
  hover, symbols, navigation, completion, rename, and formatting decisions.
- Add editor-symbol queries to `shuck-semantic` rather than teaching
  `shuck-server` to walk the AST directly.
- Keep operations conservative when shell dynamism prevents safe answers.
  Returning `None`, an empty result, or a failed `prepareRename` is preferred
  over editing or navigating to an uncertain target.
- Support both open in-memory buffers and workspace files discovered on disk.
- Preserve Shuck's clean-room policy: documentation text and completion
  metadata must be authored in this repository or derived from shell language
  facts Shuck already models.
- Keep LSP-only feature data out of normal lint-mode hot paths. Running
  `shuck check` must not build editor symbol indexes, workspace symbol indexes,
  completion caches, rename sets, or formatting range summaries unless those
  structures are also required for lint diagnostics.

### Non-Goals

- Embedded host documents such as GitHub Actions YAML remain out of scope.
  Cross-file editor features for extracted shell regions need a separate
  position-remapping design.
- Command option completion is out of scope for the first implementation.
  It requires command-specific metadata that Shuck does not currently author.
- Shell command manual-page hovers are out of scope. General hover starts with
  Shuck semantic symbols and repo-authored metadata.
- Semantic tokens, inlay hints, call hierarchy, selection ranges, and folding
  ranges are not covered by this spec.
- Rename across dynamically sourced files is out of scope unless Shuck can map
  every affected document to a concrete workspace URI and prove the rename set.
- This spec does not change lint rule behavior or diagnostic parity.
- This spec does not make `shuck check` pay for editor features. Shared parser,
  indexer, and semantic APIs may grow reusable queries, but LSP-specific
  materialization belongs in `shuck-server` or lazy semantic indexes that are
  only initialized when an editor request asks for them.

### Capability Surface

The server advertises each feature only when the matching handler and tests
land:

| Capability | Request | Initial support |
|---|---|---|
| Completion | `textDocument/completion`, `completionItem/resolve` | Variables, functions, declaration names, runtime names, and shell keywords known to Shuck |
| Hover | `textDocument/hover` | Rule-code hovers plus semantic-symbol hovers |
| Definition | `textDocument/definition` | Variable/function definitions Shuck can resolve in the current analysis graph |
| References | `textDocument/references` | References and definitions in the same proven symbol set |
| Document highlight | `textDocument/documentHighlight` | Read/write highlights for the symbol under the cursor |
| Document symbols | `textDocument/documentSymbol` | Hierarchical symbols for functions plus top-level declarations and assignments |
| Workspace symbols | `workspace/symbol` | Fuzzy search over indexed shell files in workspace folders |
| Rename | `textDocument/prepareRename`, `textDocument/rename` | Conservative variable and function rename sets |
| Formatting | `textDocument/formatting`, `textDocument/rangeFormatting` | Full-document formatting; range requests format the smallest complete statement span that contains the requested range |

`documentFormattingProvider` and `documentRangeFormattingProvider` stay unset
until formatting is ready to be supported through normal editor capability
negotiation. The current formatter request handlers are not enough by
themselves; the advertised capability is the compatibility contract.

### Shared Document Analysis

Today several server paths parse independently. This is acceptable for the
small spec 018 surface, but navigation and symbols would multiply that cost.
Introduce a document analysis cache in `shuck-server`:

```rust
pub(crate) struct DocumentAnalysis {
    source: Arc<str>,
    parse: ParseResult,
    indexer: Indexer,
    semantic: SemanticModel,
    diagnostics: Vec<lsp_types::Diagnostic>,
    symbols: EditorSymbolIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AnalysisCacheKey {
    uri: Url,
    version: DocumentVersion,
    settings_fingerprint: u64,
    position_encoding: PositionEncoding,
}
```

`Session::take_analysis_snapshot(uri)` returns a `DocumentAnalysisSnapshot`
that contains the existing `DocumentSnapshot` plus an `Arc<DocumentAnalysis>`.
The cache is invalidated when:

- `didChange` advances the document version;
- workspace or global client options change;
- `.shuck.toml` / `shuck.toml` reloads change the resolved settings;
- the document closes and no workspace index entry still references it.

The cache is an LRU bounded by document count and total source bytes. A good
initial bound is 64 documents or 32 MiB of source text, whichever is reached
first. Eviction is an optimization detail; correctness must not depend on a
cache hit.

All LSP handlers use `DocumentAnalysis` except formatting. Formatting may parse
through `shuck-formatter`, but it must still use the same resolved settings,
position encoding, and in-memory buffer as the analysis snapshot.

### Lint-Mode Performance Boundary

Editor features are opt-in by execution mode. The CLI lint path may share
lower-level semantic query code, but it must not eagerly build data whose only
consumer is an LSP request.

Allowed shared work:

- parser and indexer output that `shuck check` already builds;
- semantic bindings, references, scopes, call sites, source references, and
  lazy indexes that existing diagnostics already use;
- narrowly scoped helper methods on `SemanticModel` that are pure lookups over
  existing semantic data;
- linter facts that remain required by diagnostics.

LSP-only work:

- `DocumentAnalysis` caching;
- `EditorSymbolIndex`;
- document-symbol trees;
- workspace-symbol summaries;
- completion candidate lists;
- hover presentation strings;
- rename sets;
- range-formatting statement expansion summaries.

LSP-only work lives in `shuck-server` unless there is a clear semantic
ownership reason to place a lazy query in `shuck-semantic`. Any lazy semantic
query added for editor features must be backed by `OnceLock` or equivalent and
must not initialize during ordinary `AnalysisRequest::analyze` or
`shuck check`.

Performance regressions are evaluated separately for CLI linting and server
requests. It is acceptable for an editor request to pay for an editor index.
It is not acceptable for that index to appear in `make profile-cli`,
`shuck check`, or large-corpus lint timing unless a lint rule also consumes the
same structure.

### Semantic Editor Queries

Add an editor-facing query module to `shuck-semantic`:

```rust
pub struct EditorSymbolQuery<'model> {
    model: &'model SemanticModel,
}

pub enum EditorSymbolTarget {
    Binding(BindingId),
    Reference(ReferenceId),
    FunctionCall { name_span: Span, binding: Option<BindingId> },
    RuntimeName { name: Name, span: Span },
}

pub struct EditorSymbol {
    pub name: Name,
    pub kind: EditorSymbolKind,
    pub definition_span: Span,
    pub selection_span: Span,
    pub scope: ScopeId,
    pub binding: Option<BindingId>,
}

pub enum EditorSymbolKind {
    Function,
    Variable,
    Array,
    AssociativeArray,
    Declaration,
    RuntimeName,
}
```

The query module is responsible for answering:

- symbol at offset;
- definition span for a symbol target;
- references for a symbol target;
- binding/declaration spans that should be treated as writes;
- visible completion candidates at offset;
- document symbol tree;
- conservative rename set for a symbol target.

This keeps shell-specific name, scope, and binding interpretation inside the
semantic crate. `shuck-server` converts query results into LSP types and
handles URI, range, cancellation, and client capability details.

### Symbol Identity

Shell variables are mutable storage cells, not immutable declarations. Rename
therefore cannot use a single `BindingId` blindly. The semantic layer exposes
a `RenameSet` that groups only spans proven to refer to the same symbol:

```rust
pub struct RenameSet {
    pub name: Name,
    pub kind: EditorSymbolKind,
    pub defining_spans: Vec<Span>,
    pub reference_spans: Vec<Span>,
    pub write_spans: Vec<Span>,
}

pub enum RenameUnavailable {
    DynamicName,
    IndirectReference,
    Nameref,
    ImportedBinding,
    AmbiguousResolution,
    InvalidIdentifier,
    CrossFileUnindexed,
}
```

Rules:

- Function rename starts at a function definition or a call site that resolves
  to one function binding. It edits the definition name and resolved call-site
  callee spans. If multiple same-name functions may be visible at the cursor,
  `prepareRename` fails.
- Variable rename starts at a binding, declaration operand, or reference. It
  edits all same-name binding and reference spans in the same lexical storage
  family. It does not cross nameref, indirect expansion, dynamic declaration,
  imported binding, or unresolved-reference boundaries.
- Runtime names, positional parameters, special parameters, keywords, and
  command names that do not resolve to a Shuck function binding are not
  renameable.
- The new name is validated according to the target kind. Variable names must
  be valid shell identifiers. Function names must be static command names and
  must not include expansion syntax, whitespace, redirection syntax, or path
  separators.

`textDocument/prepareRename` returns the exact editable range and optional
placeholder. `textDocument/rename` recomputes the `RenameSet` from a fresh
snapshot before producing the `WorkspaceEdit`; it does not trust stale
`prepareRename` state.

### Navigation and References

Definition and references use the same symbol target resolution as rename:

1. Convert the cursor position into a byte offset through the snapshot's
   `PositionEncoding`.
2. Ask `EditorSymbolQuery::target_at_offset`.
3. Resolve the target to a definition or reference set.
4. Convert spans back into LSP ranges.

Definitions return a single location when the target is uniquely resolved and a
location list when multiple static definitions are intentionally visible, such
as a top-level variable with multiple declaration forms that Shuck groups into
one storage family. If all candidates are ambiguous, the result is `None`.

References honor `ReferenceContext::include_declaration`:

- `include_declaration = true`: include definition/write spans and read spans.
- `include_declaration = false`: include only read-like references and call
  sites.

Document highlights are file-local and classify spans as:

| Span source | Highlight kind |
|---|---|
| Read-like references and call sites | `Read` |
| Assignments, declaration operands, function definitions | `Write` |
| Ambiguous static mentions | omitted |

### Document Symbols

Document symbols are returned as `DocumentSymbol[]`, not the flat
`SymbolInformation[]` form. The tree mirrors shell structure where Shuck can do
so cheaply:

```text
function build
  local package
  local status
function test
  local filter
TOP_LEVEL_VAR
```

The first implementation includes:

- function definitions;
- top-level assignments and declaration operands;
- local declaration operands inside function bodies;
- loop variables when their span is explicit.

Nested functions appear as children of the enclosing function when the semantic
scope model proves that relationship. Other command-local bindings are omitted
until there is a clear editor use case, because large shell scripts can contain
many transient assignments that make outlines noisy.

### Workspace Symbols

`WorkspaceSymbolIndex` stores compact symbol summaries for each shell document:

```rust
pub(crate) struct WorkspaceSymbolSummary {
    uri: Url,
    version: Option<DocumentVersion>,
    content_hash: [u8; 32],
    symbols: Vec<WorkspaceSymbol>,
}

pub(crate) struct WorkspaceSymbol {
    name: String,
    kind: lsp_types::SymbolKind,
    container_name: Option<String>,
    range: lsp_types::Range,
    selection_range: lsp_types::Range,
}
```

Open documents are indexed from their in-memory contents. Closed workspace
files are indexed in a background task using the same discovery rules as
`shuck check`, capped by a client option:

```toml
[server.workspaceSymbols]
enabled = true
maxFiles = 5000
```

When the cap is exceeded, Shuck indexes open documents and the first `maxFiles`
workspace files in deterministic path order, then sends a log message telling
the client that the workspace symbol index is partial. The partial index still
serves `workspace/symbol`; it just does not claim completeness.

`workspace/didChangeWatchedFiles` invalidates affected closed-file summaries.
Open buffers always win over disk summaries for the same URI.

### Completion

Completion is syntax-context aware but conservative. It should not complete
inside comments, single-quoted strings, here-doc bodies, or command words where
the parser cannot identify a useful context.

Initial completion contexts:

| Context | Candidates |
|---|---|
| `$` or `${` parameter expansion | visible variables, arrays, associative arrays, runtime names |
| declaration command operand | visible variable names and snippets for declaration forms |
| command position | functions visible at the position, shell keywords/reserved words modeled by Shuck, known builtins modeled by Shuck |
| ordinary word | no completion unless the client explicitly requests completion |

Completion items carry `data` only when resolve can add cheap repo-authored
detail:

```rust
#[derive(Serialize, Deserialize)]
pub(crate) enum CompletionData {
    Symbol { uri: Url, definition_span: Span },
    RuntimeName { name: String },
    Keyword { name: String },
}
```

`completionItem/resolve` may add Markdown that Shuck authors itself: symbol
kind, definition location, declaration flags, array/scalar attributes, and
whether the name is runtime-provided. It must not shell out to external tools
or scrape command documentation.

### Hover

Hover keeps the spec 018 rule-code behavior and adds semantic-symbol hover.
Priority order:

1. Rule code inside a `# shuck:` or supported `# shellcheck` directive.
2. Semantic symbol at the cursor.
3. Runtime name known to Shuck.
4. No hover.

Semantic hover content includes:

- symbol name and kind;
- definition location;
- scope summary, such as top-level, function-local, loop-local, or imported;
- attributes such as exported, readonly, local, array, associative array,
  nameref, integer, and runtime-provided;
- for functions, a short call-site count when already available from the
  analysis cache.

Hover should avoid diagnostic-style advice. Rule explanations remain attached
to diagnostics and directive rule-code hovers.

### Formatting

Promote formatting to a supported capability only after the following behavior
is true:

- `textDocument/formatting` formats the full in-memory buffer with resolved
  `.shuck.toml` / `shuck.toml` and client options.
- `textDocument/rangeFormatting` expands the requested range to complete
  statement boundaries before formatting. If Shuck cannot find complete
  statement boundaries, it returns `None` instead of formatting a syntactically
  partial fragment.
- Formatting returns minimal text edits using the existing `single_replacement`
  diff logic.
- Formatting does not publish diagnostics or mutate server state directly.
  The editor applies edits and sends the resulting `didChange`.
- The legacy `shuck.applyFormat` command remains unadvertised unless a future
  client compatibility need appears. Normal LSP formatting requests are the
  supported surface.

### Workspace Edits

Rename and future cross-file quick fixes use `WorkspaceEdit` with
`documentChanges` when the client supports them. Otherwise they fall back to
the `changes` map. Edits are sorted by URI and descending start position within
each document before conversion so overlapping edits can be detected and
reported as an internal error during development.

Closed-file edits are allowed only when the file was indexed from disk and the
content hash still matches at edit construction time. If the file changed on
disk since indexing, rename fails with `CrossFileUnindexed` and asks the user
to retry after re-indexing.

### Request Scheduling

Feature handlers use the existing scheduler:

| Request | Schedule |
|---|---|
| Completion | Worker |
| Completion resolve | Sync if it only reads serialized data; Worker if it needs a snapshot |
| Hover | Worker |
| Definition | Worker |
| References | Worker |
| Document highlight | Worker |
| Document symbol | Worker |
| Workspace symbol | Worker |
| Prepare rename | Worker |
| Rename | Worker |
| Formatting | Fmt |
| Range formatting | Fmt |

All worker and formatter requests must honor `$/cancelRequest`. Late responses
for cancelled requests are dropped by the existing request queue.

### Client Options

Add server-specific options under `initializationOptions` and
`workspace/didChangeConfiguration`:

```rust
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerFeatureOptions {
    pub workspace_symbols: WorkspaceSymbolOptions,
    pub completion: CompletionOptions,
    pub rename: RenameOptions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceSymbolOptions {
    pub enabled: bool,
    pub max_files: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletionOptions {
    pub include_runtime_names: bool,
    pub include_keywords: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenameOptions {
    pub allow_cross_file: bool,
}
```

Defaults:

- workspace symbols enabled, `max_files = 5000`;
- runtime-name and keyword completions enabled;
- cross-file rename disabled until source-closure-backed workspace edits have
  enough black-box coverage.

### Implementation Stages

1. **Analysis cache and capability cleanup.** Add `DocumentAnalysis`, route
   diagnostics and hover through it, and keep advertised capabilities unchanged.
2. **Formatting support.** Finish range-boundary behavior, advertise document
   and range formatting, and add black-box editor tests.
3. **Symbols and hover.** Add semantic editor queries, document symbols,
   workspace symbols, and semantic-symbol hover.
4. **Navigation.** Add definition, references, and document highlight using the
   same symbol target resolution.
5. **Completion.** Add context classification and semantic completions.
6. **Rename.** Add conservative rename sets, `prepareRename`, and same-file
   `rename`; cross-file rename can be enabled after closed-file edit safety
   is proven.

Each stage must advertise only the capabilities implemented in that stage.

## Alternatives Considered

### Implement editor features directly in `shuck-server`

Rejected. It would duplicate AST and semantic interpretation in the protocol
layer, drift from lint behavior, and violate the existing ownership boundary:
parser/indexer/semantic crates understand shell structure; the server converts
their answers into LSP responses.

### Use linter facts as the editor-symbol source

Rejected as the primary model. Linter facts are policy-oriented and optimized
for diagnostics. Editor features need neutral definitions, references, scopes,
and rename groups even when no rule cares about them. Facts may still provide
useful derived annotations for hover, but semantic remains the source of truth.

### Advertise every planned capability up front

Rejected. Editors route requests based on advertised capabilities. Advertising
handlers before they are complete creates confusing failures and makes the
server look less stable than it is. Capability advertisement is the contract.

### Rename by raw text search

Rejected. Shell scripts frequently contain comments, strings, here-docs,
dynamic expansions, command names, and same-name variables in unrelated scopes.
Raw search would edit too much. Rename must be based on semantic resolution and
must fail when resolution is uncertain.

### Format arbitrary range fragments

Rejected. Shell grammar is context-sensitive enough that formatting a partial
fragment can produce invalid edits. Range formatting expands to complete
statement boundaries or returns `None`.

## Security Considerations

Editor features must not execute user shell scripts. Completion, hover,
navigation, rename, and formatting operate on parsed source and repository
metadata only.

Workspace indexing reads files under configured workspace folders using the
same ignore and shell-discovery policy as `shuck check`. It must not follow
unbounded symlink cycles, read outside workspace roots for indexing, or retain
closed-file contents longer than needed for the bounded analysis cache.

Completion resolve and hover must not shell out to documentation tools or
network services. This avoids leaking source text, cursor context, or workspace
paths and keeps Shuck's authored documentation boundary clear.

Rename that edits closed files must verify the indexed content hash before
constructing edits. If the file changed since indexing, the operation fails
instead of applying stale ranges.

## Verification

Unit tests:

- `cargo test -p shuck-semantic editor_symbol`
- `cargo test -p shuck-server completion`
- `cargo test -p shuck-server hover`
- `cargo test -p shuck-server navigation`
- `cargo test -p shuck-server symbols`
- `cargo test -p shuck-server rename`
- `cargo test -p shuck-server formatting`

Workspace validation:

```bash
cargo fmt --all --check
cargo clippy -p shuck-server -p shuck-cli --all-targets -- -D warnings
cargo test -p shuck-server
cargo test -p shuck-semantic
make test
```

Lint-mode performance validation:

```bash
make profile-cli
make test-large-corpus SHUCK_LARGE_CORPUS_TIMING=1
```

The implementation should compare these against the pre-feature baseline and
confirm that LSP-only structures do not appear in CLI lint profiles or timing
hotspots.

Manual editor black-box scenarios should cover:

- completion after `$` and `${` includes visible variables and excludes
  out-of-scope locals;
- command-position completion includes local functions and omits variables;
- hover on a variable shows definition and attributes;
- hover on a rule code still shows the spec 018 rule documentation;
- go-to-definition from a function call lands on the function name;
- references honor `includeDeclaration`;
- highlights mark reads and writes differently;
- document symbols show functions and local declarations without noisy transient
  command assignments;
- workspace symbols find functions from closed files and prefer open-buffer
  contents when a file is edited;
- prepare-rename rejects positional parameters, special parameters, dynamic
  names, namerefs, imported bindings, and ambiguous references;
- same-file rename edits only the proven symbol set;
- formatting and range formatting return stable, minimal edits and do not
  publish diagnostics directly.
