# 024: Computed-Source Directives

## Status

Implemented.

- Directive parsing for `# shuck: source=<path>` with an optional `lint=`
  policy flag, distinct from the ShellCheck-compatible `# shellcheck source=`.
- A resolved directive silences C002/C003 and imports the target's symbols
  under the default `shuck check` path.
- `lint=true` additionally lints the resolved target as an extra input and
  honors nested `lint=true` directives inside it transitively.
- `[lint] source-paths` adds project-root-relative search roots; `[lint]
  lint-sources = false` downgrades `lint=true` to symbol import only.
- Target resolution is first-match-wins: the annotating file's own directory,
  then the configured roots in order. A directive names one intended file.

Design note on the unresolved-directive case: rather than a dedicated
diagnostic, an unresolved directive still surfaces the existing
untracked-source diagnostic (C003/SC1091) at the site, which already
communicates "the asserted target was not found." This reuses C003 as the
spec's open question anticipated.

## Summary

A shuck-native inline comment directive lets users annotate a `source`/`.`
statement whose target path shuck cannot resolve statically (a computed or
dynamic path) with a concrete path, separating two orthogonal concerns:

- **Path assertion** — `# shuck: source=<path>` resolves `<path>`, imports its
  function and variable definitions so references in the current file resolve,
  and silences the "can't follow non-constant source" (C002/SC1090) and
  "untracked source file" (C003/SC1091) diagnostics at that site. The target
  is **not** linted.
- **Lint policy** — adding `lint=true` makes the target an analyzed input:
  diagnostics are reported on it, and nested `lint=true` directives inside it
  are honored transitively.

The heavy machinery already exists: shuck parses `# shellcheck source=<path>`
today, lets a directive override dynamic-path classification, and resolves,
parses, and imports sourced files through the source closure. This spec adds a
shuck-native spelling with an explicit, separate lint policy, and wires the
on-disk resolver — previously reachable only from the ShellCheck-compat path
behind `--external-sources` — into the default `shuck check` pipeline.

## Motivation

Shell scripts routinely source files by a computed path:

```bash
DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "$DIR/lib/util.sh"
. "${XDG_CONFIG_HOME:-$HOME/.config}/app/functions.sh"
```

Shuck cannot know what these resolve to at lint time, so it emits C002 (dynamic
source path) and treats every name defined in the sourced file as undefined,
producing C006 ("undefined variable") / undefined-function noise in the caller.
Without a native directive the only escape hatches are the ShellCheck-compat
directive `# shellcheck source=<path>` combined with `--external-sources`, or
blanket suppression of C002/C003 — which also hides genuine mistakes.

Users want a first-class, shuck-native way to say "when this dynamic path is
taken, it resolves to *this* file," and to choose whether shuck should merely
*trust* that file for symbol resolution or also *lint* it. The existing
`# shellcheck source=` directive conflates these: in ShellCheck, `source=` both
resolves symbols and follows the file (under `-x`). Modeling the assertion and
the policy as separate parts of one directive keeps the common "just stop the
false positives" case cheap and explicit, while keeping opt-in cross-file
linting available per site.

## Design

### Directive syntax

The directive uses the established shuck-native form (spec 006): a
case-insensitive `# shuck:` prefix followed by `key=value` tokens. It attaches
to the **next** `source`/`.` command (own-line placement) or to the command on
the same line.

```bash
# shuck: source=lib/util.sh
source "$DIR/util.sh"          # util.sh's symbols are imported; util.sh is NOT linted

# shuck: source=lib/util.sh lint=true
source "$DIR/util.sh"          # util.sh is imported AND linted

source "$maybe"  # shuck: source=/dev/null   # same-line; "nothing to include, stop warning"
```

`<path>` is resolved relative to the directory of the file containing the
directive, then against configured source roots (see
[Path resolution](#path-resolution)). `source=/dev/null` is the explicit
"there is nothing to follow" form and maps to the existing
`SourceRefKind::DirectiveDevNull`.

### Semantics

| Aspect | `source=<path>` | `source=<path> lint=true` |
| --- | --- | --- |
| Silences C002 (SC1090) / C003 (SC1091) at the site | Yes | Yes |
| Imports target's functions/variables into caller analysis | Yes | Yes |
| Target file is linted (diagnostics reported on it) | No | Yes |
| Nested `lint=true` directives in the target are honored | No | Yes |
| Requires filesystem access to the target | Yes | Yes |
| `=/dev/null` accepted (no-op include) | Yes | N/A (`lint=` is meaningless) |

A `lint=true` target is linted **like a directly checked input**: its ordinary
resolvable `source` statements are imported for symbol resolution (exactly as
they are for a direct input) but are not themselves linted; only nested
`lint=true` directives extend the linted set, transitively and cycle-safe.
Linted targets are never auto-fixed.

### Directive parsing

Source directives are parsed separately from suppression directives.
Suppression parsing (`crates/shuck-linter/src/suppression/directive.rs`)
understands only `disable=`/`ignore=` and is unchanged. Source directives are
parsed in `crates/shuck-semantic/src/builder/mod.rs` by
`parse_source_directive_override`.

1. **Token scan.** Under the `# shuck:` prefix, the whole token list is
   scanned before deciding: the first `source=` token sets the target and the
   first `lint=` token sets the policy (`true`/`false`; other values are
   ignored). The result is therefore independent of token order, and duplicate
   keys are deterministic — the first occurrence wins. A `lint=` flag without
   a `source=` target is not a source directive.
2. **Internal model.** A parsed directive carries the target
   (`SourceRefKind::Directive` / `DirectiveDevNull`), its origin
   (`SourceDirectiveOrigin::Shuck` vs `::ShellCheck`), and the lint policy
   (`SourceDirectiveInfo { origin, lint }`). A `SourceRef` for an ordinary,
   un-annotated reference has `directive: None`, so "no directive" is a
   distinct state rather than a default enum variant shared with the
   ShellCheck spelling.
3. **ShellCheck compatibility.** `# shellcheck source=<path>` maps to origin
   `ShellCheck` with `lint: false`; whether the compat path follows remains
   governed by `--external-sources` as today. The spelling
   `# shuck: shellcheck source=<path>` is honored as the ShellCheck form (the
   `shellcheck` token defers native parsing).
4. Per-line collection and own-line propagation are unchanged
   (`parse_source_directives`, `source_directive_for_line`).

### Classification

No change to precedence: `classify_source_ref`
(`crates/shuck-semantic/src/builder/source_refs.rs`) already lets a line's
directive override win over dynamic-path classification, so a hinted dynamic
`source "$DIR/util.sh"` becomes `SourceRefKind::Directive(path)` instead of
`Dynamic`. The `SourceRef` carries the directive info alongside the existing
`explicitly_provided` marker so downstream consumers can distinguish
trust-only from lint.

### Path resolution

Resolution searches the directory of the annotating file first, then the
configured source roots in order, and is **first-match-wins**: a directive
names one intended file, so a target that exists both next to the script and
under a configured root resolves to the local one (`resolve_candidate_targets`
in `crates/shuck-semantic/src/source_resolve.rs`, shared by the CLI and the
language server). The `SourcePathResolver` trait
(`crates/shuck-semantic/src/source_closure`) is the injection point; the
closure already threads a resolver, a plugin resolver, and `analyzed_paths`.

The default `shuck check` path attaches a native resolver
(`crates/shuck-cli/src/commands/check/source_resolver.rs`), so directives work
under plain `shuck check` without the compat flags.

### Linting and input feeding

For a plain `source=` directive, the closure resolves + parses + imports the
target so the caller's analysis sees its symbols; nothing else changes. This
is exactly what `SourceClosureContracts` already provides.

For `lint=true`, the resolved target additionally becomes an analyzed input:
its diagnostics are surfaced (remapped to the target file), and nested
`lint=true` directives inside it are enqueued transitively with a visited set
(`analyzed_paths`) to terminate on cycles. Linted targets join the
analyzed-path set so path-sensitive rules treat them like direct inputs, are
recorded in the referrer's cache entry and fingerprints (a warm cache hit
still lints them; editing a target invalidates the referrer), and are never
auto-fixed.

### Configuration

Native config in `crates/shuck-config` (parallel to the compat
`source_paths` / `external_sources`), threaded into `LinterSettings`
(`crates/shuck-linter/src/settings.rs`) and consumed by the check command:

```toml
[lint]
# Directories searched (after the annotating file's own directory) when
# resolving `# shuck: source=` directive targets.
source-paths = ["scripts", "lib"]

# When false, lint=true directives are downgraded to symbol import only
# (targets are not linted). Lets a project keep directives without opting
# into cross-file linting. Default: true.
lint-sources = true
```

`--external-sources` / `-x` and `--source-path` / `-P` remain compat-only and
unchanged. The native options are independent and always available.

### Edge cases

- **Unresolved directive.** If `<path>` does not resolve to a readable file,
  the directive still suppresses C002 at the site (the user asserted intent),
  but the untracked-source diagnostic (C003) reports the unresolved target so
  typos are visible.
- **`source=` on a *literal* path.** Allowed; it forces resolution to the
  given path even when the written path differs, and silences C003 if the
  literal would otherwise be untracked.
- **Duplicate keys.** `source=a.sh source=b.sh` or `lint=true lint=false` on
  one directive are deterministic: the first occurrence of each key wins.
  (The old two-directive conflict case is no longer expressible.)
- **Relative path escaping the project.** Permitted but see Security.
- **`lint=true` with `lint-sources = false`.** Downgraded to symbol import
  only; no target diagnostics.

## Alternatives Considered

### Reuse `# shellcheck source=` only

Rejected. It cannot express the trust-vs-lint distinction (ShellCheck's
`source=` always follows under `-x`), and overloading it for shuck-native
"trust only" semantics would silently diverge from ShellCheck, confusing users
who know both tools. Compatibility with `# shellcheck source=` is retained
separately.

### Two named directives (`assume-source=` / `follow-source=`)

Rejected after review. The earlier draft of this spec used a directive pair,
but the two spellings shared almost all behavior (both read, parse, and import
the target) and differed only in lint policy, which made the distinction hard
to explain and left the internal model overloading a `None` hint for both
"ordinary reference" and "ShellCheck spelling". One directive with an explicit
`lint=` flag separates the path assertion from the lint policy, and the
order-dependent conflict between the paired spellings disappears structurally.

### Global-only following (status quo `--external-sources`)

Rejected as insufficient. Following is all-or-nothing and cannot resolve
dynamic paths without a per-site hint. Per-site directives give users precise
control and compose with (rather than replace) the global switch.

### Make the target a discovered lint input via file walking

Rejected. Discovery (`crates/shuck-cli/src/discover.rs`) is a filesystem walk
by extension/shebang and has no notion of source edges. Following belongs in
the semantic source closure, which already models the edges and a visited set.

## Security Considerations

The directive causes shuck to **read files named by the script under
analysis**, and `lint=true` causes it to parse and lint them and honor their
nested directives transitively. This is the same trust posture as ShellCheck's
`-x`, but it is reachable from plain `shuck check`:

- Resolution is confined to the annotating file's directory plus configured
  `source-paths`. A directive that resolves outside those roots is not
  followed.
- shuck only reads and parses; it never executes sourced content.
- Transitive `lint=true` uses the `analyzed_paths` visited set to prevent
  cycles and unbounded fan-out.
- Symlinks and `..` traversal are resolved through the normal filesystem; a
  project that lints untrusted scripts should scope `source-paths` narrowly.
  (Open question: whether to canonicalize and reject targets outside the
  project root by default.)

## Verification

- **Directive parsing** (`crates/shuck-semantic`): unit tests that
  `# shuck: source=lib/util.sh` produces a `SourceRefKind::Directive` with
  origin `Shuck` and `lint: false`, that adding `lint=true` (in either token
  order) sets the policy, that duplicate keys are first-wins, that
  `source=/dev/null` yields `DirectiveDevNull`, and that a bare `lint=true`
  with no target is not a source directive.
- **Classification precedence**: a dynamic `source "$x"` annotated with the
  directive classifies as `Directive`, not `Dynamic`.
- **Symbol import** (plain `source=`): a caller referencing a function defined
  only in the asserted file reports no undefined-function/variable diagnostic,
  and C002/C003 are silent at the site; the target file itself is not present
  in the diagnostics.
- **Linting** (`lint=true`): the target file's own diagnostics appear
  (remapped to the target path); a symbol defined in a file it sources is
  visible; a cycle terminates.
- **Resolution precedence**: a target present both next to the annotating
  script and under a configured root resolves to (and lints) only the local
  one; roots are searched in configured order.
- **Default check path**: the above works under `make run ARGS="check <dir>"`
  with no `--external-sources` flag.
- **Config**: `lint-sources = false` downgrades `lint=true` to symbol import
  only; `source-paths` extends resolution.
- **Compat unchanged**: existing `# shellcheck source=` + `--external-sources`
  behavior and the large-corpus comparison for C002/C003 are unaffected.

Clean-room: all directive names, diagnostic wording, and documentation in this
spec are authored in-repo; no ShellCheck source or wiki text is referenced.
