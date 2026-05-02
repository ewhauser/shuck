# 017: Google Shell Style Guide Rules

## Status

Proposed

## Summary

Add a set of opt-in lint rules that cover the [Google Shell Style Guide](https://google.github.io/styleguide/shellguide.html) — both the rules ShellCheck does not implement (novel shuck rules) and the rules behind ShellCheck's optional checks plus a handful of default-on ShellCheck checks shuck does not yet map. The rules slot into the existing `S` (Style), `C` (Correctness), and `X` (Portability) categories. They are all disabled by default, configurable through `[lint.rule-options.<code>]`, and require new linter facts for file shape, comment-to-function association, TODO parsing, and Bash feature versioning. A new `google` named-group selector pulls the full Google-style preset together — both the new rules and the existing shuck rules that already cover Google-aligned ShellCheck checks.

## Motivation

ShellCheck and shuck's existing rule set already cover a large slice of shell hygiene — quoting, brace expansions, `[[ ]]` over `[ ]`, `read -r`, return-status handling, and so on. The Google Shell Style Guide goes further: it prescribes a *script shape* (where constants live, where functions live, where executable code lives), a *documentation contract* (file headers, function docs that name globals, args, outputs, and return values), and a *naming policy* (snake_case locals, SCREAMING_SNAKE_CASE constants, `package::function` namespaces).

Today shuck has no way to enforce these. Users writing shell at a Google-style codebase fall back to manual review, ad-hoc grep, or independent tooling. The rules in this spec close that gap without competing with ShellCheck on its own ground.

The rules are deliberately opinionated and project-policy oriented, so they must be opt-in and configurable. A monorepo that requires `#!/bin/bash` shebangs, a 100-line script ceiling, and `pkg::function` namespacing should be able to express that policy in `shuck.toml`. A casual user who wants none of it should see no behavior change.

## Design

### Scope

This spec covers two classes of rules:

1. **Novel Google-style rules** — checks the Google guide prescribes that ShellCheck does not implement and that the formatter cannot enforce. These have no `shellcheck_code` mapping and use shuck-only diagnostic wording.
2. **ShellCheck-mapped rules** — checks the Google guide aligns with that map directly to ShellCheck rules (default-on or optional) shuck has not yet imported. These follow the project's existing conformance pattern (parity testing via `make test-large-corpus`).

Both classes share the same constraints:

- Operate on a single file's source plus its existing `LinterFacts`, `SemanticModel`, and `Indexer`.
- Do not require filesystem inspection (executable bit, filename casing, repo directory layout).
- Do not require cross-file resolution (which files are sourced by which, whether a library is sourceable under test).

The rules in the original Google-style proposal that fall outside that scope — shell library exec bit, filename convention, path role, sourced-file main logic, sourceable-under-test approximation — are explicitly deferred. They need a `ProjectContext` layer that the linter does not have today and are tracked separately.

### Rule Inventory

The new rules split into two groups: **novel** rules with no ShellCheck counterpart and **ShellCheck-mapped** rules that import a check shuck has not yet adopted. Numbering picks up after the existing highest rule in each category (`S077`, `C157`, `X081`).

#### Style (S) — novel rules

| Code | Name | Trigger |
|------|------|---------|
| S078 | `shebang-not-bash` | Executable script's shebang names a shell other than the configured policy (default `bash`). |
| S079 | `shebang-form-mismatch` | Shebang line does not match the configured form (e.g., `#!/bin/bash` vs `#!/usr/bin/env bash`). |
| S080 | `script-size-threshold` | Source line count exceeds configured threshold (default 100). |
| S081 | `missing-file-description` | First non-shebang, non-blank line is not a comment block describing the file's purpose. |
| S082 | `todo-format` | A `TODO` / `FIXME` / `XXX` comment lacks a `(name)` or `(handle)` owner annotation, or lacks a message. |
| S083 | `missing-function-doc` | A function definition has no leading comment block when policy thresholds say it should (visibility, length, parameter use). |
| S084 | `function-doc-content` | A function's leading comment block does not document the items present in the function: globals read/written, args used, stdout produced, and return values. |
| S085 | `missing-main-function` | A non-trivial script (configurable predicate) does not call `main "$@"` (or the configured equivalent) as its final top-level statement. |
| S086 | `top-level-code-not-minimal` | Top-level executable statements appear outside the allowed set (constants, `readonly`, `declare`, function definitions, sourcing, and a single `main "$@"` invocation). |
| S087 | `constants-not-at-top` | A `readonly` / `declare -r` / `export` constant declaration appears below the first function definition. |
| S088 | `functions-not-grouped` | Function definitions are not contiguous: an executable statement appears between two function definitions. |
| S089 | `function-name-case` | A function name does not match the configured naming pattern (default snake_case, optionally allowing `pkg::function`). |
| S090 | `package-function-namespace` | Functions in files matching the configured library predicate do not use a `package::function_name` form. |
| S091 | `variable-name-case` | A `local` variable name (or function-scoped assignment target) does not match the configured naming pattern (default snake_case). |
| S092 | `constant-name-case` | A `readonly` / `declare -r` / `export` target does not match the configured naming pattern (default `SCREAMING_SNAKE_CASE`). |
| S093 | `alias-definition` | An `alias` builtin call appears in a script. |
| S094 | `undocumented-globals` | A function reads or writes a non-local variable without that variable being named in the function's leading comment block. |
| S095 | `missing-cli-usage` | A script that parses arguments (`getopts`, indexing into `$@`/`$1`..`$9`, or referencing `$1` outside a function) does not define a `usage` function or handle `--help`/`-h`. |
| S096 | `error-not-on-stderr` | A command in a likely error branch (`die`, after a failed `||`, inside an `if ... then` that ends in `exit N` with `N != 0`) writes to stdout without `>&2`. |
| S097 | `suppression-without-reason` | A `# shuck:` or `# shellcheck` directive does not include trailing free-text justification. |
| S098 | `blanket-suppression` | A directive disables a wide selector (`ALL`, `S`, `C`, etc.) or uses `disable-file` outside the configured allowlist. |

#### Style (S) — ShellCheck-mapped imports

| Code | Maps to | Name | Trigger |
|------|---------|------|---------|
| S099 | SC2230 / `deprecate-which` | `which-vs-command-v` | A script invokes `which`; the Google guide and ShellCheck both prefer `command -v`. |
| S100 | SC2248 / `quote-safe-variables` | `quote-safe-variables` | Even "safe-looking" expansions are unquoted; the rule extends S001 to cases S001 currently allows. |
| S101 | SC2250 / `require-variable-braces` | `require-variable-braces` | A scalar expansion uses `$var` form rather than `${var}`. |
| S102 | SC2292 / `require-double-brackets` | `require-double-brackets` | A test uses `[ ... ]` in Bash where `[[ ... ]]` is preferred. |
| S103 | SC2243 / `avoid-nullary-conditions` | `avoid-nullary-conditions` | A `[[ ]]` test uses a bare command substitution (`[[ $(cmd) ]]`) where explicit `-n` / `-z` is preferred. |
| S104 | SC2249 / `add-default-case` | `missing-case-default` | A `case` statement has no `*)` default branch. (Distinct from S069, which covers the `getopts` invalid-flag handler under SC2220.) |

#### Correctness (C) — novel rules

| Code | Name | Trigger |
|------|------|---------|
| C158 | `implicit-global-in-function` | An assignment inside a function targets a name that was not declared `local`/`declare`/`readonly` in the function's scope and is not a documented intentional global. |
| C159 | `mutable-global` | A non-`readonly` global variable is assigned more than once at the top level, or assigned at the top level and reassigned from a function. |
| C160 | `unanchored-source-path` | A `source` / `.` invocation uses a relative path that does not begin with `"${BASH_SOURCE[0]%/*}"`, `"$(dirname "$0")"`, or another script-dir anchor. |

#### Correctness (C) — ShellCheck-mapped imports

| Code | Maps to | Name | Trigger |
|------|---------|------|---------|
| C161 | SC2218 | `function-called-before-defined` | A function is invoked at a top-level position that is reached before the function's definition is evaluated. |
| C162 | SC2312 / `check-extra-masked-returns` | `extra-masked-returns` | A command-substitution failure is masked in cases beyond the `local`/`declare`/`export` set already covered by S010 (e.g., on the right-hand side of a non-declaration assignment that the rule policy treats as masking). |

#### Portability (X) — novel rules

| Code | Name | Trigger |
|------|------|---------|
| X082 | `bash-feature-too-new` | A construct used in the script requires a Bash version newer than the project's configured minimum. |
| X083 | `bash-feature-too-old` | A construct used in the script is below the project's required modern-Bash floor (e.g., a project that requires Bash 5+ uses syntax that works on 3.2). |

### Relationship to Existing Rules

None of the rules in this spec are implemented today, but several rule families already exist that this spec must integrate with rather than duplicate.

**Shebang cluster.** C060 (non-absolute-shebang), C073 (indented-shebang), C074 (space-after-hash-bang), C075 (shebang-not-on-first-line), S043 (missing-shebang-line), and S053 (duplicate-shebang-flag) already inspect the shebang for placement, indentation, absoluteness, and duplicate flags. They do not classify *which* shell the shebang names or *which form* (`#!/bin/bash` vs `#!/usr/bin/env bash`) it uses. S078 and S079 fill that gap by extending `ShebangHeaderFacts` with `shell` and `form` fields, and they should be documented as "checks the policy of the shebang" while the existing rules cover its mechanics. A rule-doc cross-reference under each shebang rule makes the cluster discoverable.

**`local` policy pair.** C014 (local-top-level) flags `local` used at script scope. C158 (`implicit-global-in-function`) is the inverse: it flags an assignment inside a function that should have used `local`. The rules are complementary and should reference each other in their docs. C136 (local-cross-reference), C150 (subshell-local-assignment), S066 (local-declare-combined), and X003 (local-variable-in-sh) round out the existing `local`-related rules and remain unchanged.

**Alias rules.** S056 (command-substitution-in-alias) and S057 (function-in-alias) inspect alias *bodies* for substitutions and positional parameters. S093 (`alias-definition`) flags the *existence* of any alias in a script. The three rules are independent and can be enabled in any combination.

**Source-command rules.** X042 (sourced-with-args) and X080 (source-inside-function-in-sh) are existing portability rules over `source`/`.`. C160 (`unanchored-source-path`) adds a correctness check that the source target is anchored to the script's directory. The three rules read from the existing source-reference fact set.

**Stderr.** C085 (stderr-before-stdout-redirect) checks the *ordering* of `2>&1` and `>file` redirects. S096 (`error-not-on-stderr`) checks whether error-branch output is *routed* to stderr at all. The two rules touch stderr from opposite directions and do not conflict.

**Quoting and bracing imports.** S001 (SC2086, scalar expansion) and S004 (SC2046, command substitution) cover ShellCheck's default-on quoting checks. S100 (SC2248) extends S001 to flag expansions S001 currently allows when the project policy says even safe-looking expansions should be quoted. S101 (SC2250) is independent: it requires brace form (`${var}`) on every scalar expansion regardless of quoting. S077 (SC1087, `brace-variable-before-bracket`) is a narrower lexical rule about a specific `$var[` ambiguity and remains separate.

**Test-form imports.** S102 (SC2292) flags `[ ... ]` in Bash and prefers `[[ ... ]]`. S103 (SC2243) flags bare command-substitution operands inside `[[ ]]` (`[[ $(cmd) ]]`) and prefers an explicit `-n` test or a direct exit-status conditional. The two rules layer on the existing test-condition fact set (`facts/simple_tests.rs`, `facts/conditionals.rs`); they do not introduce a new fact builder.

**Case-default and getopts pair.** S069 implements the `getopts` invalid-flag handler under SC2220. S104 (`missing-case-default`, mapping SC2249 / `add-default-case`) is a separate rule that flags general `case` statements lacking a `*)` branch. The two rules cover unrelated situations in different ShellCheck codes; they are kept independent so users can enable either.

**Masked-returns pair.** S010 (SC2155) flags returns masked by `local`/`declare`/`export` declaration assignments. C162 (`extra-masked-returns`, mapping SC2312 under ShellCheck's `check-extra-masked-returns` optional check) extends the policy to additional masking forms the optional check covers. C162 reads from the existing assignment fact set with an extended policy filter rather than introducing a new fact.

**Function-call ordering.** C063 (`overwritten-function`) and the existing call-site fact set already track function definitions and invocations. C161 (`function-called-before-defined`, SC2218) reuses the call-site fact set with a top-level evaluation-order filter — invocation positions are compared to the spans of definitions reachable on the same evaluation path. No new fact builder.

**`which` rule.** S099 (SC2230, `which-vs-command-v`) is a structural command-name check with no existing adjacent rule. It reads from the normalized command fact set (`facts/normalized_commands.rs`).

### Default Enablement

Every rule introduced in this spec is **off by default**. They appear in the registry, in the documentation, and in selectors, but a fresh `shuck.toml` does not enable them.

Rationale: these rules encode project policy, not universal correctness. Forcing them on would surprise existing users and would conflict with codebases that intentionally use POSIX `sh`, large shell scripts, or coding conventions other than Google's.

Users opt in via `shuck.toml`:

```toml
[lint]
select = ["C", "K"]
extend-select = [
    "S078", "S079",         # shebang policy
    "S081", "S083", "S084", # documentation policy
    "S085", "S086",         # script shape
    "S089", "S091", "S092", # naming policy
    "S097", "S098",         # suppression hygiene
    "C158", "C160",         # implicit-global / unanchored-source
]
```

A `google` named group is also added for convenience, equivalent to the union of every rule in this spec:

```toml
extend-select = ["google"]
```

Named groups are a thin selector type that resolves to a static `RuleSet` at config-load time. The existing `RuleSelector` enum gains a `Named(NamedGroup)` variant.

### New Linter Facts

Most rules in this spec need structural information that is not in `LinterFacts` today. Following the project convention from `CLAUDE.md`, the new logic lives in `crates/shuck-linter/src/facts.rs` rather than as ad-hoc walks inside rule files.

#### `FileShape`

Classifies the script's top-level statements into the slots allowed by Google style. Built once per file, consumed by `S080`, `S081`, `S085`, `S086`, `S087`, `S088`, and `S095`.

```rust
pub struct FileShape {
    /// Span of the shebang line, if present.
    pub shebang: Option<ShebangFact>,
    /// The first non-shebang, non-blank, non-trailing-attribute comment
    /// block, if present. Used by S081.
    pub file_header_comment: Option<CommentBlockId>,
    /// Top-level statements classified by role.
    pub top_level: Vec<TopLevelEntry>,
    /// Total source line count (for S080 and "non-trivial" predicates).
    pub line_count: u32,
    /// Whether the file ends with a `main "$@"` (or configured equivalent)
    /// invocation as its last top-level statement.
    pub trailing_main_call: Option<MainCallFact>,
}

pub struct TopLevelEntry {
    pub stmt: StmtId,
    pub role: TopLevelRole,
    pub span: Span,
}

pub enum TopLevelRole {
    Shebang,
    Comment,
    Constant,            // readonly / declare -r / export NAME=...
    Declaration,         // declare without -r and without rvalue
    FunctionDefinition,
    SourceCommand,       // source / . X
    MainInvocation,      // main "$@" or configured form
    OtherExecutable,
}
```

#### Extending `ShebangHeaderFacts`

A `ShebangHeaderFacts` builder already exists in `crates/shuck-linter/src/facts/comments.rs` and powers C060, C073, C074, C075, S043, and S053. Today it stores anti-pattern spans (indented shebang, missing shebang, duplicate flags, non-absolute shebang) but does not classify the shell name or the shebang form.

S078 and S079 extend that struct rather than introducing a parallel fact:

```rust
pub struct ShebangHeaderFacts {
    // ...existing fields...

    /// Classified shell from the shebang's interpreter path. Populated when
    /// a shebang is present and parseable, regardless of any anti-pattern flags.
    pub shell: Option<ShebangShell>,
    /// AbsolutePath: `#!/bin/bash`. EnvLookup: `#!/usr/bin/env bash`. Other: anything else.
    pub form: Option<ShebangForm>,
    /// Span of the interpreter token (the path or the `env`-resolved shell).
    pub interpreter_span: Option<Span>,
    /// Span of the entire shebang line, used by S078/S079 diagnostics.
    pub shebang_span: Option<Span>,
}

pub enum ShebangShell { Bash, Sh, Zsh, Ksh, Dash, Other }
pub enum ShebangForm  { AbsolutePath, EnvLookup, Other }
```

The new fields are populated unconditionally — every existing shebang-aware rule already pays the parse cost, so there is no gating concern. The flag list S053 reads remains where it is.

#### `FunctionDocs`

Associates each function definition with the comment block immediately preceding it (after stripping blank lines), and parses Google-style structured fields out of that comment block.

```rust
pub struct FunctionDocs {
    pub by_function: FxHashMap<FunctionId, FunctionDocBlock>,
}

pub struct FunctionDocBlock {
    pub comments: IdRange<CommentId>,
    pub span: Span,
    /// Documented sections recovered from `Globals:`, `Arguments:`,
    /// `Outputs:`, `Returns:` headings. Detection is permissive: header
    /// match is case-insensitive and allows `:` or no separator. The
    /// concrete grammar is documented inline with the parser.
    pub sections: FunctionDocSections,
}

pub struct FunctionDocSections {
    pub globals: Option<DocSection>,
    pub arguments: Option<DocSection>,
    pub outputs: Option<DocSection>,
    pub returns: Option<DocSection>,
}
```

`FunctionDocs` powers `S083`, `S084`, and `S094`.

#### `TodoComments`

```rust
pub struct TodoComment {
    pub comment_id: CommentId,
    pub kind: TodoKind,                // TODO | FIXME | XXX
    pub owner: Option<TodoOwner>,
    pub message: Option<String>,
    pub span: Span,
}

pub enum TodoOwner {
    Name(String),
    Url(String),
    BugReference(String),
}
```

Powers `S082`.

#### `SuppressionDirectiveText`

The existing suppression layer (spec 006) parses directives but discards trailing free-text. Powering `S097` and `S098` requires the directive index to retain the trailing reason span and the original selector strings.

```rust
pub struct SuppressionDirectiveText {
    pub directive_id: DirectiveId,
    pub trailing_reason: Option<Span>,  // text after the codes, before EOL
    pub selector_strings: Vec<String>,  // raw selectors as written
    pub action: DirectiveAction,
}
```

This lives next to the existing `SuppressionIndex` and is built lazily — `S097`/`S098` are off by default, so most runs skip the work.

#### `BashFeatureUse`

Used by `X082`/`X083`. Each entry records a syntactic or builtin use that has a known minimum Bash version.

```rust
pub struct BashFeatureUse {
    pub feature: BashFeature,
    pub min_version: BashVersion,
    pub span: Span,
}

pub enum BashFeature {
    AssocArray,                 // declare -A
    NamerefDeclareN,            // declare -n
    GlobstarPattern,            // shopt -s globstar
    CaseFallthroughSemicolons,  // ;& and ;;&
    LowercasePatternExpansion,  // ${var,,}
    SubstringSlicing,           // ${var:offset:len}
    // ... extended over time
}
```

The feature catalog is bounded by what `X082`/`X083` are configured to flag. Unrecognized features are not synthesized.

### Rule-Option Schema

Every rule that needs configuration registers options under `[lint.rule-options.<code>]` matching the existing pattern (see `C001`'s `treat-indirect-expansion-targets-as-used`).

#### S078 / S079 — shebang policy

```toml
[lint.rule-options.s078]
allowed-shells = ["bash"]   # also accepts "sh", "zsh", "ksh"

[lint.rule-options.s079]
allowed-forms = ["env-lookup"]   # "absolute-path" | "env-lookup"
allowed-paths = ["/bin/bash", "/usr/bin/env bash"]
```

#### S080 — script size

```toml
[lint.rule-options.s080]
max-lines = 100
count = "physical"   # "physical" | "non-comment-non-blank"
```

#### S083 — function doc requirement

```toml
[lint.rule-options.s083]
require-for = ["all"]            # "all" | "exported" | "long" | "parameterized"
long-function-line-threshold = 10
```

#### S085 / S086 — script shape

```toml
[lint.rule-options.s085]
non-trivial-line-threshold = 30
non-trivial-function-count = 2
main-name = "main"

[lint.rule-options.s086]
allowed-statements = [
    "constant", "declaration", "function-definition", "source", "main-invocation"
]
```

#### S089 / S091 / S092 — naming

```toml
[lint.rule-options.s089]
pattern = "^[a-z_][a-z0-9_]*(::[a-z_][a-z0-9_]*)?$"

[lint.rule-options.s091]
pattern = "^[a-z_][a-z0-9_]*$"

[lint.rule-options.s092]
pattern = "^[A-Z_][A-Z0-9_]*$"
```

#### S090 — package namespace

```toml
[lint.rule-options.s090]
library-predicate = "filename-suffix:.lib.sh"
```

(The library predicate is purely textual — it inspects the source path string passed to the linter. It does not stat the file.)

#### S100 — quote-safe-variables

```toml
[lint.rule-options.s100]
# Scalar expansions S001 already considers safe (numeric-only, single-char known-quote-safe)
# are still flagged unless they appear in this allowlist. Default empty.
allowed-bare-expansions = []
```

#### S101 — require-variable-braces

```toml
[lint.rule-options.s101]
# Some expansions are unambiguous without braces (single-letter positional `$1`,
# special params `$?`, `$$`, etc.). The default exempts them.
exempt-special-params = true
exempt-single-digit-positional = true
```

#### S102 — require-double-brackets

```toml
[lint.rule-options.s102]
# Only fires in Bash/Ksh/Zsh; ignored when dialect is sh/dash regardless of selection.
# This option is informational; the rule cannot be enabled in pure-sh contexts anyway.
allow-in-portable-mode = false
```

#### S103 — avoid-nullary-conditions

```toml
[lint.rule-options.s103]
# Some teams prefer `[[ "$x" ]]` over `[[ -n "$x" ]]`. Default off — the rule
# fires on bare nullary tests as ShellCheck's optional check does.
allow-nullary-on-quoted-expansion = false
```

#### C161 — function-called-before-defined

```toml
[lint.rule-options.c161]
# When true, the rule treats sourced files as opaque (any function might be defined
# inside) and does not flag calls to names that look like they could come from a
# preceding `source` command.
ignore-after-source = true
```

#### C162 — extra-masked-returns

```toml
[lint.rule-options.c162]
# Additional assignment forms beyond the S010 set that should be treated as
# masking. Default matches ShellCheck's check-extra-masked-returns.
treat-as-masking = ["readonly", "typeset"]
```

#### S098 — blanket suppressions

```toml
[lint.rule-options.s098]
disallowed-selectors = ["ALL", "C", "S", "P", "X", "K"]
allow-disable-file = false
```

#### X082 / X083 — Bash version policy

```toml
[lint.rule-options.x082]
max-version = "4.4"   # features above this floor are flagged

[lint.rule-options.x083]
min-version = "5.0"   # features supported only below this floor are flagged
```

### Checker Integration

Rules dispatch through the existing phase model from spec 005:

- File shape, shebang, file header, TODO, suppression-text, and Bash feature rules slot into a new `check_file_shape` phase that runs after `check_commands` and reads exclusively from `LinterFacts`.
- Naming and local/global rules slot into the existing `check_bindings` and `check_scopes` phases, gated on the new rule codes.
- `C160` (`unanchored-source-path`) extends the existing `check_source_refs` phase.

No rule in this spec adds an AST walk inside its rule file. Every rule reads from `LinterFacts` per the project convention. New facts are constructed in `facts.rs` behind `LinterFactsBuilder` flags so they only run when at least one rule that needs them is enabled — the file-shape and function-doc passes are not free.

### CLI and Documentation

Each rule gets a YAML in `docs/rules/`. Rules in this spec set `origin: novel`, which makes `legacy_code`, `legacy_name`, and `source` optional and disables the cross-repo `shell-checks` validation. ShellCheck-mapped rules in this spec still set `shellcheck_code` and `shellcheck_level` so the corpus harness can run parity comparison.

```yaml
origin: novel
new_category: Style
new_code: S081
runtime_kind: ast
shellcheck_code: ~
shellcheck_level: ~
shells: [bash]
description: Files do not begin with a comment block describing their purpose.
rationale: A file-level header comment makes a script's intent discoverable...
safe_fix: false
fix_description: ~
default_enabled: false
options:
  - name: ignore-shebang-only-files
    type: bool
    default: false
    description: Skip files whose only content is a shebang.
examples:
  - kind: invalid
    code: |
      #!/bin/bash
      ls -la
  - kind: valid
    code: |
      #!/bin/bash
      # List files with timestamps.
      ls -la
```

The validator (`docs/rules/validate.sh`) gains an `origin: imported | novel` field (default `imported` for backwards compatibility), an optional `default_enabled` boolean, and an optional `options` list. Novel rules omit `legacy_code`/`legacy_name`/`source` and use inline `examples[].code` without a `source` field. Existing rules continue to validate without modification.

## Alternatives Considered

### A new `G` category for Google / governance rules

Putting these under their own letter would make selection easy (`--select G`) and would cleanly separate project-policy lints from language-correctness lints. We rejected it: the categories `C/S/P/X/K` are organized by *kind of issue* (correctness, style, performance, portability, security), not by *where the rule comes from*. A dedicated `G` would mix taxonomy levels. The `google` named group provides one-shot selection without polluting the category axis.

### Default-on with a "google" preset toggle

A `[lint] preset = "google"` switch could enable the bundle in one line. We rejected enabling-by-default for any of these rules because they encode policy choices a project must make (which shebang form, what naming pattern, what script size ceiling). The named `google` group covers the same ergonomics for users who do want them all.

### Filesystem-aware rules in this spec

The original proposal included rules that need the executable bit, the filename, and the repo path role. These would require threading a `FileMetadata` (path, mode, project root) through `lint_file` and adding filesystem stats to the linter pipeline. We deferred them: the linter is currently a pure function over source plus an indexer, and breaking that invariant in this spec would entangle the rule additions with an API change that deserves its own design pass.

### Cross-file analysis for sourced-file effects

`Sourced files should not execute main logic` and `sourceable under test` need to know which files are sourced from where. We deferred them rather than approximating: a static "no `exit`, no `cd`, no argument parsing at top level" check has high false-positive rates on the kind of init scripts that legitimately do those things. A future spec can revisit once cross-file source resolution exists.

### Per-rule walks instead of new facts

Several rules (file shape, function docs, TODO scanning) could be implemented as one-off walks inside the rule files. We rejected that: `CLAUDE.md` mandates that rule files stay as cheap filters over `LinterFacts`. Putting file-shape detection in `facts.rs` also lets multiple rules share one pass over the top-level statements rather than each rule re-scanning.

### Embed the Google guide as a hard dependency

We could mirror the Google guide's prescriptions verbatim — fixed snake_case, fixed 80-char line limit, fixed `#!/bin/bash` shebang. Rejected: every team using these rules will deviate at the margins, and a non-configurable rule is a rule that gets turned off entirely. Configurability is the difference between a rule that ships and one that ends up in `extend-ignore`.

## Verification

Each rule lands with:

- A YAML in `docs/rules/SNNN.yaml` or `docs/rules/CNNN.yaml` containing `description`, `rationale`, `examples` (at least one valid and one invalid), `default_enabled: false`, and the rule's option schema.
- A Rust module in `crates/shuck-linter/src/rules/{style,correctness,portability}/` with a fixture-driven test that asserts the diagnostic span and message on the YAML's invalid example, and asserts no diagnostic on the valid example.
- A `LinterFacts` extension in `crates/shuck-linter/src/facts.rs` (where applicable) plus a unit test for the fact builder.

End-to-end checks:

- `cargo test -p shuck-linter` — covers per-rule fixtures and fact-builder unit tests.
- `cargo test -p shuck-cli` — covers CLI integration, including:
  - `shuck check` with default config produces no diagnostics from any rule in this spec on a fixture that would trigger them.
  - `shuck check --select google` produces the expected set on the same fixture.
  - `shuck check --select S078 --shuck-toml <path>` honors the `allowed-shells` option.
- `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=...` runs each new rule against the corpus. The two rule classes verify differently:
  - **Novel rules** (S078–S098, C158–C160, X082–X083) have no ShellCheck counterpart, so parity comparison is skipped; the harness should treat unmapped shuck-only rules as informational rather than diff-failing. A small change to the corpus runner is in scope for this spec.
  - **ShellCheck-mapped rules** (S099–S104, C161, C162) follow the existing conformance pattern: the corpus harness compares shuck output against ShellCheck output for the mapped code, and a `docs/bugs/` document captures any deltas before the rule is shipped.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.

Documentation checks:

- The generated rule reference includes every new rule with its `default_enabled: false` badge and option schema.
- The `google` named group resolves at config-load time to exactly the membership listed in the *Google Preset* section below; a unit test asserts the membership.

## Google Preset

The `google` named-group selector resolves to the union of every rule below — both new rules introduced in this spec and existing shuck rules that already cover Google-aligned ShellCheck checks. Existing `C` rules in the table are default-on in shuck regardless of the preset; they appear here for completeness so the preset is a self-contained declaration of what "Google-style linting" means.

| Rule | Status | Default-on in shuck? | Maps to | Source |
|------|--------|----------------------|---------|--------|
| S001 | existing | no | SC2086 | quote scalar expansions |
| S002 | existing | no | SC2162 | `read -r` |
| S004 | existing | no | SC2046 | quote command substitutions |
| S005 | existing | no | SC2006 | backticks → `$(...)` |
| S006 | existing | no | SC2007 | obsolete `$[...]` |
| S008 | existing | no | SC2068 | unquoted `$@`/`$*` |
| S010 | existing | no | SC2155 | masked returns in `local`/`declare`/`export` |
| S017 | existing | no | SC2206 | array assignment from a string |
| S022 | existing | no | SC2219 | `let` → `$((...))` |
| S069 | existing | no | SC2220 (getopts flavor) | `getopts` invalid-flag handler |
| S078 | new | no | — | `shebang-not-bash` |
| S079 | new | no | — | `shebang-form-mismatch` |
| S080 | new | no | — | `script-size-threshold` |
| S081 | new | no | — | `missing-file-description` |
| S082 | new | no | — | `todo-format` |
| S083 | new | no | — | `missing-function-doc` |
| S084 | new | no | — | `function-doc-content` |
| S085 | new | no | — | `missing-main-function` |
| S086 | new | no | — | `top-level-code-not-minimal` |
| S087 | new | no | — | `constants-not-at-top` |
| S088 | new | no | — | `functions-not-grouped` |
| S089 | new | no | — | `function-name-case` |
| S090 | new | no | — | `package-function-namespace` |
| S091 | new | no | — | `variable-name-case` |
| S092 | new | no | — | `constant-name-case` |
| S093 | new | no | — | `alias-definition` |
| S094 | new | no | — | `undocumented-globals` |
| S095 | new | no | — | `missing-cli-usage` |
| S096 | new | no | — | `error-not-on-stderr` |
| S097 | new | no | — | `suppression-without-reason` |
| S098 | new | no | — | `blanket-suppression` |
| S099 | new | no | SC2230 | `which-vs-command-v` |
| S100 | new | no | SC2248 | `quote-safe-variables` |
| S101 | new | no | SC2250 | `require-variable-braces` |
| S102 | new | no | SC2292 | `require-double-brackets` |
| S103 | new | no | SC2243 | `avoid-nullary-conditions` |
| S104 | new | no | SC2249 | `missing-case-default` |
| C001 | existing | yes | SC2034 | unused variables |
| C004 | existing | yes | SC2164 | `cd` return value |
| C006 | existing | yes | SC2154 | variable used before assignment |
| C010 | existing | yes | SC2015 | `A && B \|\| C` masking |
| C012 | existing | yes | SC2035 | `rm *` glob hazards |
| C099 | existing | yes | SC2124 | array slicing |
| C100 | existing | yes | SC2128 | array referenced as scalar |
| C158 | new | no | — | `implicit-global-in-function` |
| C159 | new | no | — | `mutable-global` |
| C160 | new | no | — | `unanchored-source-path` |
| C161 | new | no | SC2218 | `function-called-before-defined` |
| C162 | new | no | SC2312 | `extra-masked-returns` |
| X082 | new | no | — | `bash-feature-too-new` |
| X083 | new | no | — | `bash-feature-too-old` |

The preset is implemented as a static `RuleSet` constant in `crates/shuck-linter/src/named_groups.rs` (a new module). `RuleSelector::Named(NamedGroup::Google)` resolves to that constant. `extend-select = ["google"]` in `shuck.toml` unions it into the active rule set; existing `C` rules remain on regardless.

A unit test in `named_groups.rs` asserts that the `google` group's membership matches the table above byte-for-byte. The test is the source of truth: if a future PR adds a rule to the group without updating the table, CI fails.

### Excluded from the Preset

For clarity, the following adjacent rules are deliberately **not** in the `google` preset:

- **C014** (`local-top-level`), **C136** (`local-cross-reference`), **C150** (`subshell-local-assignment`), **S066** (`local-declare-combined`), **X003** (`local-variable-in-sh`) — already default-on (the C-prefix ones) or independently selectable; they are useful but not part of Google's prescribed style.
- **C060**, **C073**, **C074**, **C075**, **S043**, **S053** (shebang mechanics) — orthogonal to Google's *which-shell* policy. A project can select them alongside the preset if desired.
- **S056**, **S057** (alias contents) — flagged independently from S093 (alias existence). A project that bans aliases entirely (Google policy) only needs S093.
- **X042**, **X080** (source-command portability) — about portability, not Google style. C160 covers Google's source-anchoring rule.

These exclusions keep the preset focused on the Google guide. Users who want a stricter posture can compose `["google", "C014", "C060", ...]` in `extend-select`.
