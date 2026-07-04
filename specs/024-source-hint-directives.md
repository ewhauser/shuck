# 024: Source Hint Directives

## Status

Proposed

## Summary

Two shuck-native inline comment directives let users annotate a `source`/`.`
statement whose target path shuck cannot resolve statically (a computed or
dynamic path) with a concrete relative path:

- `# shuck: assume-source=<path>` — resolve `<path>`, import its function and
  variable definitions so references in the current file resolve, and silence
  the "can't follow non-constant source" (C002/SC1090) and "untracked source
  file" (C003/SC1091) diagnostics at that site. The target is **not** linted.
- `# shuck: follow-source=<path>` — everything `assume-source` does, **and**
  the target becomes an analyzed input: diagnostics are reported on it and its
  own `source` statements are followed transitively. `follow-source` implies
  `assume-source`.

The heavy machinery already exists: shuck parses `# shellcheck source=<path>`
today, lets a directive override dynamic-path classification, and resolves,
parses, and imports sourced files through the source closure. This spec adds a
shuck-native spelling with an explicit trust/follow distinction, and wires the
on-disk resolver — currently reachable only from the ShellCheck-compat path
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
Today the only escape hatches are the ShellCheck-compat directive
`# shellcheck source=<path>` combined with `--external-sources`, or blanket
suppression of C002/C003 — which also hides genuine mistakes.

Users want a first-class, shuck-native way to say "when this dynamic path is
taken, it resolves to *this* file," and to choose whether shuck should merely
*trust* that file for symbol resolution or also *lint* it. The existing
`# shellcheck source=` directive conflates these: in ShellCheck, `source=` both
resolves symbols and follows the file (under `-x`). Splitting the two intents
into named directives makes the common "just stop the false positives" case
cheap and explicit, while keeping opt-in cross-file linting available.

## Design

### Directive syntax

Both directives use the established shuck-native form (spec 006): a
case-insensitive `# shuck:` prefix followed by `key=value`, with an optional
trailing `# reason` stripped. They attach to the **next** `source`/`.` command
(own-line placement) or to the command on the same line.

```bash
# shuck: assume-source=lib/util.sh
source "$DIR/util.sh"          # util.sh's symbols are imported; util.sh is NOT linted

# shuck: follow-source=lib/util.sh
source "$DIR/util.sh"          # util.sh is imported AND linted, and its sources are followed

source "$maybe"  # shuck: assume-source=/dev/null   # same-line; "nothing to include, stop warning"
```

`<path>` is resolved relative to the directory of the file containing the
directive, then against configured source roots (see [Path resolution](#path-resolution)).
`assume-source=/dev/null` is the explicit "there is nothing to follow" form and
maps to the existing `SourceRefKind::DirectiveDevNull`.

### Semantics

| Aspect | `assume-source` | `follow-source` |
| --- | --- | --- |
| Silences C002 (SC1090) / C003 (SC1091) at the site | Yes | Yes |
| Imports target's functions/variables into caller analysis | Yes | Yes |
| Target file is linted (diagnostics reported on it) | No | Yes |
| Target's own `source` statements are followed | No | Yes |
| Requires filesystem access to the target | Yes | Yes |
| `=/dev/null` accepted (no-op include) | Yes | N/A |

`follow-source` is a strict superset of `assume-source`. When global following
is already enabled (`--external-sources` / config, see below), a resolvable
literal `source` path is followed regardless; these directives matter for paths
that are *dynamic* or that live outside the configured search roots.

### Directive parsing

Source directives are parsed separately from suppression directives. Suppression
parsing (`crates/shuck-linter/src/suppression/directive.rs`) understands only
`disable=`/`ignore=` and is unchanged. Source directives are parsed in
`crates/shuck-semantic/src/builder/mod.rs` by `parse_source_directive_override`,
which today requires the comment text to contain `"shellcheck"` and recognizes
only the `source=` key.

Changes:

1. Recognize the shuck-native prefix. Accept `# shuck: assume-source=<path>`
   and `# shuck: follow-source=<path>` in addition to the existing
   `# shellcheck source=<path>`. The `# shuck:` colon form is required for the
   native spellings, matching spec 006.
2. Carry the follow intent. Extend `SourceDirectiveOverride`
   (`crates/shuck-semantic/src/lib.rs`) with a `follow: bool` (or an enum
   `SourceDirectiveMode { Assume, Follow }`). `# shellcheck source=` maps to
   `Assume` for symbol import, preserving current behavior; whether the compat
   path follows remains governed by `--external-sources` as today.
3. Keep per-line collection and own-line propagation unchanged
   (`parse_source_directives`, `source_directive_for_line`).

### Classification

No change to precedence: `classify_source_ref`
(`crates/shuck-semantic/src/builder/source_refs.rs`) already lets a line's
directive override win over dynamic-path classification, so a hinted dynamic
`source "$DIR/util.sh"` becomes `SourceRefKind::Directive(path)` instead of
`Dynamic`. `SourceRef` gains the follow flag alongside the existing
`explicitly_provided` marker so downstream consumers can distinguish trust-only
from follow.

### Path resolution

Resolution reuses the candidate logic that the compat resolver already implements
(`crates/shuck-cli/src/shellcheck_compat/mod.rs`, `resolve_source_ref_paths` /
`resolve_candidate_paths`): the directory of the annotating file first, then
configured source roots. The `SourcePathResolver` trait
(`crates/shuck-semantic/src/source_closure`) is the injection point; the closure
already threads a resolver, a plugin resolver, and `analyzed_paths`.

The default `shuck check` path does **not** attach a resolver today
(`crates/shuck-cli/src/commands/check/analyze.rs` builds the `AnalysisRequest`
with `with_source_path`, `with_shellcheck_map`, `with_optional_plugin_resolver`
only). This spec adds a native resolver on that path, so `assume-source` /
`follow-source` work under plain `shuck check` without the compat flags.

### Following and input feeding

For `assume-source`, the closure resolves + parses + imports the target so the
caller's analysis sees its symbols; nothing else changes. This is exactly what
`SourceClosureContracts` already provides.

For `follow-source`, the resolved target additionally becomes an analyzed input:
its diagnostics are surfaced (remapped to the target file), and its own source
refs are resolved transitively. This mirrors what the compat path already does
when it computes `resolved_paths` from `analysis.semantic.source_refs()` and
gates on `external_sources` (`shellcheck_compat/mod.rs`), but keyed off the
per-site `follow` flag rather than a global switch. Transitive following honors
a visited set (`analyzed_paths`) to terminate on cycles.

### Configuration

Add native config to `crates/shuck-config` (parallel to the compat
`source_paths` / `external_sources`), threaded into `LinterSettings`
(`crates/shuck-linter/src/settings.rs`, next to `resolve_source_closure`) and
consumed by the check command:

```toml
[lint]
# Directories searched (after the annotating file's own directory) when
# resolving assume-source / follow-source paths.
source-paths = ["scripts", "lib"]

# When false, follow-source is downgraded to assume-source (symbols imported,
# target not linted). Lets a project keep hints without opting into cross-file
# linting. Default: true.
follow-sources = true
```

`--external-sources` / `-x` and `--source-path` / `-P` remain compat-only and
unchanged. The native options are independent and always available.

### Edge cases

- **Unresolved hint.** If `<path>` does not resolve to a readable file, the
  directive still suppresses C002 at the site (the user asserted intent), but a
  new low-severity diagnostic reports the unresolved hint so typos are visible.
  (Open question: reuse C003 vs. a dedicated code — decide during implementation.)
- **`assume-source` on a *literal* path.** Allowed; it forces resolution to the
  given path even when the written path differs, and silences C003 if the literal
  would otherwise be untracked.
- **Both directives on one site.** `follow-source` wins (it is the superset);
  emit a diagnostic for the redundant/conflicting pair.
- **Relative path escaping the project.** Permitted but see Security.
- **`follow-source` with `follow-sources = false`.** Downgraded to
  `assume-source`; no target diagnostics.

## Alternatives Considered

### Reuse `# shellcheck source=` only

Rejected. It cannot express the trust-vs-follow distinction (ShellCheck's
`source=` always follows under `-x`), and overloading it for shuck-native
"trust only" semantics would silently diverge from ShellCheck, confusing users
who know both tools. Compatibility with `# shellcheck source=` is retained
separately.

### A single directive with a modifier (e.g. `source=path follow`)

Rejected. Two named directives read better at call sites, are trivially
greppable, and each maps to one intent. A modifier grammar invites parsing
ambiguity and a worse error surface.

### Global-only following (status quo `--external-sources`)

Rejected as insufficient. Following is all-or-nothing and cannot resolve dynamic
paths without a per-site hint. Per-site directives give users precise control and
compose with (rather than replace) the global switch.

### Make the target a discovered lint input via file walking

Rejected. Discovery (`crates/shuck-cli/src/discover.rs`) is a filesystem walk by
extension/shebang and has no notion of source edges. Following belongs in the
semantic source closure, which already models the edges and a visited set.

## Security Considerations

Both directives cause shuck to **read files named by the script under analysis**,
and `follow-source` causes it to parse and lint them and follow their sources
transitively. This is the same trust posture as ShellCheck's `-x`, but it is now
reachable from plain `shuck check`:

- Resolution is confined to the annotating file's directory plus configured
  `source-paths`. A hint that resolves outside those roots is not followed.
- shuck only reads and parses; it never executes sourced content.
- Transitive `follow-source` uses the `analyzed_paths` visited set to prevent
  cycles and unbounded fan-out.
- Symlinks and `..` traversal are resolved through the normal filesystem; a
  project that lints untrusted scripts should scope `source-paths` narrowly.
  (Open question: whether to canonicalize and reject targets outside the project
  root by default.)

## Verification

- **Directive parsing** (`crates/shuck-semantic`): unit tests that
  `# shuck: assume-source=lib/util.sh` and `# shuck: follow-source=lib/util.sh`
  produce a `SourceRefKind::Directive` with the correct `follow` flag, and that
  `assume-source=/dev/null` yields `DirectiveDevNull`.
- **Classification precedence**: a dynamic `source "$x"` annotated with either
  directive classifies as `Directive`, not `Dynamic`.
- **Symbol import** (assume): a caller referencing a function defined only in the
  hinted file reports no undefined-function/variable diagnostic, and C002/C003 are
  silent at the site; the target file itself is not present in the diagnostics.
- **Following** (follow): the target file's own diagnostics appear (remapped to
  the target path), and a symbol defined in a file it sources is visible; a cycle
  terminates.
- **Default check path**: the above works under `make run ARGS="check <dir>"`
  with no `--external-sources` flag.
- **Config**: `follow-sources = false` downgrades `follow-source` to
  `assume-source` (no target diagnostics); `source-paths` extends resolution.
- **Compat unchanged**: existing `# shellcheck source=` + `--external-sources`
  behavior and the large-corpus comparison for C002/C003 are unaffected.

Clean-room: all directive names, diagnostic wording, and documentation in this
spec are authored in-repo; no ShellCheck source or wiki text is referenced.
