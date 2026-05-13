# 020: Zsh Plugin Resolution

## Status

Proposed

## Summary

Add first-party plugin resolution for zsh framework loaders so Shuck can
follow real plugin entrypoints instead of relying on rule-local path and
prefix heuristics. The design keeps actual plugin files as the primary source
of truth: when a zsh config statically identifies a framework root and plugin
selection, Shuck resolves the corresponding plugin entrypoints, summarizes
them through the existing source-closure pipeline, and imports the resulting
bindings/functions into the analyzed file.

The user model is intentionally layered:

- Common static dotfiles should work with no config.
- If the file hides the framework root behind `$HOME`, `~`, or a machine-local
  install path, `.shuck.toml` should be able to provide the missing root.
- If the file chooses plugins dynamically, `.shuck.toml` should be able to
  declare the logical plugin load explicitly.
- If the framework is nonstandard, `.shuck.toml` should be able to attach raw
  entrypoint paths directly.

This spec is about resolving real plugin files. It does not define the future
manifest system for non-lexical plugin contracts, though the resolver API is
designed so manifests can be attached later without a second architectural
rewrite.

## Motivation

Shuck already has two useful but incomplete pieces:

- `source_closure` can summarize sourced helper files and import their reads,
  bindings, and provided functions.
- zsh ambient contracts can suppress some false positives for known framework
  paths such as `.zshrc`, `oh-my-zsh`, `powerlevel10k`, and
  `zsh-syntax-highlighting`.

The gap is that plugin ecosystems usually do not look like direct source edges
from the user file being linted. A typical dotfile does this:

```zsh
export ZSH="$HOME/.oh-my-zsh"
plugins=(git docker zsh-autosuggestions)
source "$ZSH/oh-my-zsh.sh"
```

The plugin files exist, but:

- the file does not source `git.plugin.zsh` or `docker.plugin.zsh` directly;
- the current `source_closure` path resolver only starts from source-like
  operands;
- regular `shuck check` has no first-party resolver surface beyond literal or
  source-dir-anchored paths;
- rule-local path heuristics end up hard-coding framework names, config
  prefixes, and expected outputs into Rust.

That leaves users with a bad tradeoff:

- vendoring or directly sourcing every plugin file makes analysis work but does
  not reflect how zsh configs are written;
- keeping real framework loaders produces false positives that have to be
  suppressed or patched one-off;
- local machine installs under `~/.oh-my-zsh`, `~/.zprezto`, or similar are
  invisible unless the file happens to spell those paths in a resolvable form.

The design goal is to move plugin knowledge to the same layer users already
expect to configure:

- framework roots belong in `.shuck.toml`, not rule code;
- plugin selection belongs in the source file when it is statically visible,
  with config only as a fallback;
- resolved plugin files should flow through the same semantic/source-closure
  machinery as ordinary sourced helpers.

## Design

### Goals

- Resolve real zsh plugin entrypoints through a first-party pipeline in normal
  `shuck check`, not only through ShellCheck-compat mode.
- Make common static oh-my-zsh setups work with no config.
- Provide a repo-native `.shuck.toml` fallback for framework roots, explicit
  logical plugin loads, and raw entrypoint paths.
- Reuse the existing helper summarization and source-closure import path
  instead of building a second semantic engine for plugins.
- Keep plugin resolution conservative: if Shuck cannot statically determine a
  plugin load, it should not invent one.
- Include plugin/source-closure dependencies in cache invalidation so changing
  a resolved plugin file updates diagnostics for the consuming file.

### Non-Goals

- This spec does not define a full plugin manifest format for externally
  consumed names, prefixes, or synthetic bindings that are not visible in real
  plugin code. That belongs in a follow-on design.
- This spec does not execute framework loaders, plugin managers, or shell code.
- This spec does not attempt complete support for every zsh plugin manager in
  the first milestone.
- This spec does not surface diagnostics from third-party plugin files as
  top-level diagnostics in the user file. Resolved plugin files influence the
  contract imported into the user file, but their own warnings are not emitted
  through this mechanism.
- This spec does not add new global flags outside `shuck check`.

### User Experience

The intended user experience is a four-step ladder:

1. **No config when the file is already clear.**

   This should work out of the box when the framework root and plugin list are
   statically visible:

   ```zsh
   ZSH=/opt/oh-my-zsh
   plugins=(git docker)
   source "$ZSH/oh-my-zsh.sh"
   ```

2. **Small config when the file hides a machine-local root.**

   This should work with a single config root even if the file spells the root
   dynamically:

   ```toml
   [lint.zsh.plugins.roots]
   oh-my-zsh = "~/.oh-my-zsh"
   ```

3. **Explicit logical plugin loads when the source is dynamic.**

   If the file builds `plugins` dynamically or chooses a framework module list
   through conditionals, users should be able to declare the intended load
   without spelling raw entrypoint paths:

   ```toml
   [[lint.zsh.plugins.plugin-loads]]
   pattern = "**/.zshrc"
   framework = "oh-my-zsh"
   name = "git"

   [[lint.zsh.plugins.plugin-loads]]
   pattern = "**/.zshrc"
   framework = "oh-my-zsh"
   name = "docker"

   [[lint.zsh.plugins.plugin-loads]]
   pattern = "**/.zshrc"
   framework = "oh-my-zsh"
   name = "zsh-autosuggestions"
   ```

4. **Raw entrypoint fallback for nonstandard layouts.**

   If the framework is custom or unsupported, users should still be able to
   attach the exact plugin file(s):

   ```toml
   [[lint.zsh.plugins.entrypoints]]
   pattern = "**/.zshrc"
   paths = ["./vendor/prompt/prompt.plugin.zsh"]
   ```

This ladder is intentionally biased toward the source file as the primary
description of the environment. Config is a fallback and augmentation layer,
not the main place where users should have to duplicate their zsh setup.

### Supported Framework Scope

The first milestone focuses on the loader style that most directly matches the
current false-positive cluster:

| Framework style | Detection source | Entry resolution | Status |
|---|---|---|---|
| oh-my-zsh plugins | `plugins=(...)` plus `source .../oh-my-zsh.sh` | `$root/plugins/<name>/<name>.plugin.zsh` | v1 |
| oh-my-zsh built-in themes | `ZSH_THEME=<name>` plus `source .../oh-my-zsh.sh` | `$root/themes/<name>.zsh-theme` | v1, plain built-in themes only |
| explicit raw entrypoints | `.shuck.toml` | exact path list | v1 |
| Prezto modules | `source .../init.zsh` plus zstyle-driven module selection | `$root/modules/<name>/init.zsh` | deferred |
| plugin-manager command DSLs (`zinit`, `zplug`, `antigen`, etc.) | manager-specific commands | manager cache/install layout | deferred |

The key design choice is that v1 optimizes for correctness and good UX on the
largest static-loader case rather than pretending every zsh ecosystem can be
captured in one pass.

### Config Surface

Add a new nested plugin section under `[lint.zsh]`:

```toml
[lint.zsh.plugins]
# Default: true
resolution = true

[lint.zsh.plugins.roots]
oh-my-zsh = "~/.oh-my-zsh"
prezto = "~/.zprezto"

[[lint.zsh.plugins.plugin-loads]]
pattern = "**/.zshrc"
framework = "oh-my-zsh"
name = "git"

[[lint.zsh.plugins.plugin-loads]]
pattern = "**/.zshrc"
framework = "oh-my-zsh"
name = "docker"

[[lint.zsh.plugins.theme-loads]]
pattern = "**/.zshrc"
framework = "oh-my-zsh"
name = "agnoster"

[[lint.zsh.plugins.entrypoints]]
pattern = "**/.zshrc"
paths = ["./vendor/prompt/prompt.plugin.zsh"]
```

Configuration types:

```rust
pub struct LintZshConfig {
    pub plugins: Option<ZshPluginsConfig>,
}

pub struct ZshPluginsConfig {
    pub resolution: Option<bool>,
    pub roots: Option<BTreeMap<String, String>>,
    pub plugin_loads: Option<Vec<ZshPluginLoadConfig>>,
    pub theme_loads: Option<Vec<ZshThemeLoadConfig>>,
    pub entrypoints: Option<Vec<ZshPluginEntrypointConfig>>,
}

pub struct ZshPluginLoadConfig {
    pub pattern: String,
    pub framework: String,
    pub name: String,
}

pub struct ZshThemeLoadConfig {
    pub pattern: String,
    pub framework: String,
    pub name: String,
}

pub struct ZshPluginEntrypointConfig {
    pub pattern: String,
    pub paths: Vec<String>,
}
```

Behavior rules:

- `resolution = false` disables the entire plugin-resolution pipeline while
  leaving ordinary source closure intact.
- `roots` is a fallback map keyed by framework name. If the source file itself
  resolves a concrete framework root, that root wins.
- `plugin-loads` augments automatic plugin discovery. If Shuck statically
  extracts plugin names from the file, configured names are unioned in after
  de-duplication.
- `theme-loads` augments automatic theme discovery using the same
  pattern/framework matching rules.
- `entrypoints` is the lowest-level override. These paths are treated as
  additional plugin entrypoints for matching files even if no known framework
  is detected.
- config layering follows the existing Shuck model:
  - later config layers replace duplicate `roots` keys;
  - `plugin-loads`, `theme-loads`, and `entrypoints` append across layers;
  - `--config` overrides can still set or replace nested plugin values when a
    user wants the generic config-override path instead of dedicated flags.

### CLI Surface

Every user-facing config in `[lint.zsh.plugins]` gets a matching `shuck check`
flag. The CLI follows the same replace-vs-extend pattern already used by
`--per-file-shell` and `--extend-per-file-shell`.

```text
--zsh-plugin-resolution
--no-zsh-plugin-resolution
--zsh-plugin-root <FRAMEWORK=PATH>
--extend-zsh-plugin-root <FRAMEWORK=PATH>
--zsh-plugin <PATTERN:FRAMEWORK:NAME>
--extend-zsh-plugin <PATTERN:FRAMEWORK:NAME>
--zsh-theme <PATTERN:FRAMEWORK:NAME>
--extend-zsh-theme <PATTERN:FRAMEWORK:NAME>
--zsh-plugin-entrypoint <PATTERN:PATH>
--extend-zsh-plugin-entrypoint <PATTERN:PATH>
```

Examples:

```bash
shuck check . \
  --zsh-plugin-root oh-my-zsh=~/.oh-my-zsh \
  --extend-zsh-plugin '**/.zshrc:oh-my-zsh:git' \
  --extend-zsh-plugin '**/.zshrc:oh-my-zsh:docker' \
  --extend-zsh-theme '**/.zshrc:oh-my-zsh:agnoster'
```

CLI semantics:

- `--zsh-plugin-resolution` and hidden `--no-zsh-plugin-resolution` follow
  the existing `check` bool-flag convention used by flags such as
  `--respect-gitignore` and `--no-respect-gitignore`.
- `--zsh-plugin-resolution` sets `lint.zsh.plugins.resolution = true`.
- `--no-zsh-plugin-resolution` sets `lint.zsh.plugins.resolution = false`.
- `--zsh-plugin-root` replaces configured root mappings in the CLI layer;
  `--extend-zsh-plugin-root` adds or overwrites individual framework keys on
  top of earlier layers.
- `--zsh-plugin` and `--zsh-theme` replace configured logical loads in the CLI
  layer for their respective categories.
- `--zsh-plugin-entrypoint` replaces configured raw entrypoints in the CLI
  layer.
- `--extend-zsh-plugin`, `--extend-zsh-theme`, and
  `--extend-zsh-plugin-entrypoint` append to earlier config or CLI values.
- global `--config` overrides still work and remain the escape hatch for any
  future nested setting that lacks a dedicated parser.

The corresponding argument types live in a dedicated flattened
`ZshPluginArgs` group on `CheckCommand`, with a help heading such as
`Zsh plugin resolution`, rather than being folded into generic rule-selection
flags.

Path expansion for config values is deliberately limited and deterministic:

- `~`, `$HOME`, and `${HOME}` are expanded.
- Relative paths are resolved relative to the project root.
- Other environment variables, command substitutions, and glob expansion are
  not evaluated.

This keeps config portable enough for local dotfiles while avoiding implicit
execution semantics inside configuration parsing.

### Internal Architecture

Plugin resolution becomes a peer of source resolution inside semantic build:

```text
File path + source text
    -> parser + semantic model
    -> file-entry ambient contracts
    -> source references
    -> plugin requests
    -> source resolver + plugin resolver
    -> helper summaries
    -> imported bindings/functions + synthetic reads
    -> final semantic analysis + lint facts
```

The new internal types are:

```rust
pub enum PluginFramework {
    OhMyZsh,
    Prezto,
    ExplicitFilesystem,
}

pub enum PluginRequestKind {
    Plugin,
    Theme,
    Entrypoint,
}

pub struct PluginRequest {
    pub framework: PluginFramework,
    pub kind: PluginRequestKind,
    pub name: String,
    pub span: Span,
    pub explicit: bool,
}

pub struct PluginResolution {
    pub entrypoints: Vec<PathBuf>,
    pub file_entry_contracts: Vec<FileContract>,
}

pub trait PluginResolver {
    fn resolve_plugin_request(
        &self,
        source_path: &Path,
        request: &PluginRequest,
    ) -> PluginResolution;
}
```

`file_entry_contracts` is intentionally present even though v1 returns an empty
list. That is the forward-compatible hook for a future manifest system.

`SemanticBuildOptions` grows:

```rust
pub struct SemanticBuildOptions<'a> {
    pub source_path: Option<&'a Path>,
    pub source_path_resolver: Option<&'a (dyn SourcePathResolver + Send + Sync)>,
    pub plugin_resolver: Option<&'a (dyn PluginResolver + Send + Sync)>,
    pub file_entry_contract: Option<FileContract>,
    pub file_entry_contract_collector: Option<&'a mut dyn FileEntryContractCollector>,
    pub analyzed_paths: Option<&'a FxHashSet<PathBuf>>,
    pub shell_profile: Option<ShellProfile>,
    pub resolve_source_closure: bool,
}
```

Regular `shuck check` will now construct and pass a resolver. ShellCheck-compat
mode can reuse the same low-level resolver stack rather than continuing to own
the only resolver implementation in the codebase.

### Plugin Request Extraction

Plugin request extraction happens after the initial semantic model and ambient
contracts are built, alongside source-closure collection. The implementation
lives next to `source_closure.rs` and uses the parsed file plus semantic facts
to extract known static loader patterns.

For v1, automatic extraction supports:

- unconditional top-level `plugins=(...)` array assignments with static
  elements;
- unconditional top-level `plugins+=(...)` appends with static elements;
- unconditional top-level `ZSH_THEME=...` scalar assignment with a static
  value;
- source-like bootstrap commands whose literal or statically recoverable tail
  is `oh-my-zsh.sh`.

The extractor maintains a small state machine while walking the root scope in
source order:

```text
track framework root candidates
track current static plugins array
track current static theme
when bootstrap source is observed:
    emit plugin requests for the current state
```

Conservative rules:

- assignments inside conditionals, loops, functions, or command substitutions
  do not participate in automatic plugin extraction;
- if a `plugins` mutation is dynamic, Shuck stops trusting the automatic plugin
  list for that bootstrap site and falls back to config if available;
- built-in theme resolution only applies to plain theme names such as
  `agnoster`; custom themes and names containing `/` require an explicit config
  entry in v1.

This is intentionally narrower than "whatever zsh can execute," but it matches
how most static dotfiles are written and avoids unsound imports.

### Framework Root Resolution

Framework root resolution uses the first successful source in this order:

1. a concrete bootstrap path already resolved from the file's source expression;
2. a statically extracted framework root variable from the file;
3. a configured framework root from `[lint.zsh.plugins.roots]`.

Static root extraction supports:

- literal absolute paths;
- literal relative paths anchored to the source file directory;
- leading `~`;
- `$HOME` and `${HOME}`;
- one level of statically assigned indirection through a plain assignment.

The extractor does not evaluate command substitutions, nested parameter
expansion operators, or shell arithmetic. If a root expression exceeds that
surface, config must provide the fallback root.

### Entrypoint Resolution Rules

For v1, oh-my-zsh resolution rules are:

- plugin `git` resolves to `$root/plugins/git/git.plugin.zsh`;
- plugin `docker` resolves to `$root/plugins/docker/docker.plugin.zsh`;
- built-in theme `agnoster` resolves to `$root/themes/agnoster.zsh-theme`.

Configured raw entrypoints are normalized the same way source candidates are:

- absolute paths are used directly;
- relative paths are resolved against the project root;
- non-files are ignored.

Each successful entrypoint path is passed to the existing helper summarization
pipeline. That means plugin files participate in:

- recursive source closure inside the plugin itself;
- unresolved-reference summarization;
- provided binding/function summarization;
- synthetic read import.

No plugin-specific semantic shortcut path is added in v1.

### Contract Import and Explicitness

Resolved plugin entrypoints are imported like source-closure helpers, but they
are not surfaced as `SourceRef`s because the user file did not literally source
them. That yields two deliberate behaviors:

- plugin imports can keep user bindings live and satisfy unresolved reads;
- rules such as `untracked_source_file` do not emit diagnostics for plugin
  entrypoints, because there is no user-written source operand to point at.

Configured logical loads and raw entrypoints are marked `explicit = true`
internally so future tooling can distinguish "Shuck inferred this" from "the
user asked for this." v1 does not expose that distinction in diagnostics.

### Caching and Dependency Tracking

Plugin resolution makes helper dependencies more important because a user's
diagnostics can change when `~/.oh-my-zsh/plugins/git/git.plugin.zsh` changes
even if `.zshrc` does not.

The cache needs one new concept: dependency fingerprints attached to each
checked file.

```rust
pub struct ResolvedDependencyFingerprint {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub permissions_mode: u32,
}
```

`CheckCacheData` stores the resolved source/plugin dependency list for each
file. Before reusing cached diagnostics, Shuck re-stats those dependencies and
invalidates the cache entry if any fingerprint differs or a dependency no
longer exists.

Because the current cache-hit path accepts entries inside
`ProjectRun::take_pending_files` before any `check`-specific logic runs, v1
needs an explicit hit-validation hook. The simplest acceptable design is to
add a `check`-specific validator callback to the project-runner cache path so
stale dependency fingerprints turn a would-be cache hit back into a pending
file analysis.

This is broader than plugin resolution alone, but the feature is too confusing
without it. The dependency list should include both ordinary source-closure
helpers and plugin entrypoints so all imported semantic inputs share the same
cache correctness model.

### Watch Mode

`shuck check --watch` should react to plugin-file edits in the same way it
already reacts to source-file or config-file edits.

The current watch target collection only knows about input paths and resolved
config files, so v1 needs one more layer:

- each completed `check` run reports the dependency fingerprints collected for
  its analyzed files;
- watch mode rebuilds its file-watch target set from the union of:
  - original input paths;
  - resolved config file paths;
  - resolved source-closure helper paths;
  - resolved plugin entrypoints;
- external dependency files are watched the same way existing file targets are
  watched now: by watching the parent directory and exact-matching the file
  path.

The watch set can change after a rerun, so the watcher must refresh dependency
targets after each completed analysis pass rather than assuming the initial
target set is permanent.

### Integration Points

Implementation will touch these areas:

- `crates/shuck-config`
  - add `[lint.zsh.plugins]` config types and override validation metadata;
- `crates/shuck-cli/src/args.rs`
  - add dedicated `check` flags for every `[lint.zsh.plugins]` setting;
- `crates/shuck-cli/src/commands/check/settings.rs`
  - parse plugin config and CLI flags into one effective plugin-resolution
    plan and thread it into resolved check settings and cache keys;
- `crates/shuck-cli/src/commands/check/analyze.rs`
  - pass a first-party resolver into linter entrypoints during normal
    `shuck check`;
- `crates/shuck-semantic`
  - add `PluginResolver`, `PluginRequest`, and plugin-resolution collection
    alongside `source_closure`;
- `crates/shuck-linter`
  - keep ambient contracts, but stop adding new framework-specific rule-local
    heuristics where resolved plugin files or future manifests should own the
    behavior instead.

### Concrete Code Changes

This section is intentionally more concrete than the architectural sections
above. It is the checklist of repo changes required to make the design
implementable in this codebase.

#### `crates/shuck-config`

- extend `LintConfig` with `zsh: Option<LintZshConfig>`.
- add nested config structs for:
  - `LintZshConfig`
  - `ZshPluginsConfig`
  - `ZshPluginLoadConfig`
  - `ZshThemeLoadConfig`
  - `ZshPluginEntrypointConfig`
- implement nested `apply_overrides` behavior so CLI/config override layering
  matches the replace-vs-extend rules in this spec.
- extend `CONFIG_OVERRIDE_LINT_KEYS` and nested override validation so
  `--config 'lint = { zsh = ... }'` accepts:
  - `[lint.zsh]`
  - `[lint.zsh.plugins]`
  - `[lint.zsh.plugins.roots]`
  - `[[lint.zsh.plugins.plugin-loads]]`
  - `[[lint.zsh.plugins.theme-loads]]`
  - `[[lint.zsh.plugins.entrypoints]]`
- add configuration metadata entries so generated settings docs include the new
  zsh plugin sections.
- update config parser/unit tests to cover valid nested overrides and
  unsupported-key failures.

#### `crates/shuck-cli/src/args.rs`

- add a dedicated flattened `ZshPluginArgs` group on `CheckCommand`.
- add concrete parser types for:
  - `<FRAMEWORK=PATH>`
  - `<PATTERN:FRAMEWORK:NAME>`
  - `<PATTERN:PATH>`
- make the boolean toggle follow the repo convention:
  - visible `--zsh-plugin-resolution`
  - hidden `--no-zsh-plugin-resolution`
- add CLI parsing tests for the new value forms, replacement flags, extend
  flags, and the hidden negative bool form.

#### `crates/shuck-cli/src/commands/check/settings.rs`

- add a compiled `ZshPluginResolutionPlan` (or equivalent) that represents the
  fully layered plugin-resolution settings for one project root.
- parse `[lint.zsh.plugins]` config and `ZshPluginArgs` into that plan using
  the same config-then-CLI layering model already used for rule selection.
- expand `~`, `$HOME`, and `${HOME}` during plan compilation.
- resolve relative config paths against the project root.
- compile glob matchers for `plugin-loads`, `theme-loads`, and `entrypoints`
  so per-file matching is cheap at analysis time.
- thread the plan through `ResolvedCheckSettings`.
- add the effective plugin-resolution state to `EffectiveCheckSettings` so it
  participates in cache keys.

#### `crates/shuck-cli/src/commands/check/analyze.rs`

- build a per-file plugin resolver from `ResolvedCheckSettings` and the file's
  absolute path.
- pass the per-file plugin resolver through `AnalysisRequest` during normal
  `shuck check`.
- return dependency fingerprints alongside diagnostics so cache insertion and
  watch-mode refresh both see the same dependency set.

#### `crates/shuck-cli/src/commands/check/add_ignore.rs`

- use the same resolver-aware analysis path as normal `check` so
  `shuck check --add-ignore` sees the same zsh plugin environment instead of
  silently bypassing plugin resolution.

#### `crates/shuck-cli/src/commands/check/cache.rs`

- extend `CheckCacheData` with dependency fingerprints for resolved helper and
  plugin files.
- add serialization tests for the new cache payload shape.

#### `crates/shuck-cli/src/commands/project_runner.rs`

- add a cache-hit validation hook, or an equivalent `check`-specific wrapper,
  so cached `CheckCacheData` can be rejected when a dependency fingerprint is
  stale even if the main source file's own `FileCacheKey` is unchanged.

#### `crates/shuck-cli/src/commands/check/watch.rs`

- refresh watch targets after each completed run using dependency fingerprints.
- add watch-target tests that cover:
  - plugin entrypoint files outside the analyzed tree;
  - dependency changes that appear only after the first analysis pass;
  - de-duplication when multiple files share the same plugin entrypoint.

#### `crates/shuck-cli/src/config_docs.rs`

- ensure the generated settings reference can render the new nested zsh plugin
  sections cleanly.
- update config-doc tests so the generated reference includes the new zsh
  headings.

#### `crates/shuck-linter`

- add resolver-aware public or crate-visible entrypoints so CLI code can pass a
  plugin resolver through the normal lint path.
- thread `plugin_resolver` through the same internal analysis path that
  currently threads `source_path_resolver`.
- extend suppression-rewrite analysis helpers used by `add_ignores_to_path`
  so they can participate in plugin resolution too.

#### `crates/shuck-semantic`

- extend `SemanticBuildOptions` with `plugin_resolver`.
- add `PluginFramework`, `PluginRequestKind`, `PluginRequest`,
  `PluginResolution`, and `PluginResolver`.
- add plugin-request extraction alongside source-closure collection.
- add entrypoint resolution and helper summarization reuse for resolved plugin
  files.
- expose the resolved dependency paths needed by CLI cache/watch layers.
- add semantic tests for:
  - static extraction
  - root resolution
  - entrypoint imports
  - dependency reporting

#### `crates/shuck-linter/src/ambient_contracts`

- keep existing ambient zsh contracts for non-lexical runtime/config behavior
  in phase 1.
- stop expanding framework-specific heuristics in these modules where resolved
  plugin files or future manifests should take over.

### Migration Strategy

This design intentionally separates entrypoint resolution from contract data.

Phase 1:

- implement first-party plugin resolver support in regular `shuck check`;
- support oh-my-zsh plugin resolution and explicit entrypoint fallback;
- add cache dependency tracking;
- keep existing zsh ambient contracts in place for non-lexical runtime/config
  behavior.

Phase 2:

- move framework-specific externally consumed names/prefixes into data-driven
  plugin manifests;
- add framework adapters for Prezto and selected plugin managers once their
  loader semantics are modeled cleanly.

That sequencing gives users immediate value from real plugin files without
blocking on the harder manifest design.

## Alternatives Considered

### Keep Adding Rule-Local Heuristics

We could continue the current pattern of adding path substrings, exact
framework names, and config prefixes in rule code or ambient-contract modules.

Rejected because:

- it mixes core shell semantics with framework contracts;
- every new framework requires another hand-maintained path list;
- analyzing the real plugin file is more accurate than guessing which names it
  probably reads or publishes.

### Reuse Only Generic Source Resolution

We could improve `SourcePathResolver` roots and stop there, asking users to
make plugin files reachable through generic source-path search.

Rejected because:

- plugin frameworks usually do not expose plugin entrypoints as explicit
  source operands in the user file;
- normal `shuck check` currently has no first-party source-path surface;
- users should not have to translate `plugins=(git docker)` into raw helper
  paths by hand when the framework layout is well known.

### Jump Straight to a Full Plugin Manifest System

We could define manifests first and rely on them for both resolution and
contracts.

Rejected because:

- many existing false positives disappear as soon as Shuck can analyze the real
  plugin entrypoint file;
- resolution is a prerequisite for manifest attachment anyway;
- the manifest question is broader than the immediate need and would delay the
  simplest user-visible win.

### Execute Framework Loaders or Plugin Managers

We could run `oh-my-zsh.sh`, `zinit`, or similar tooling in a sandbox to learn
the final plugin set.

Rejected because:

- it would make static analysis depend on execution, shell state, networked or
  cloned repos, and local machine behavior;
- it would be slow, non-deterministic, and difficult to cache safely;
- it would violate the current analysis model of "parse and summarize files,"
  not "run the user's shell."

## Verification

Implementation should be verified with targeted unit tests plus end-to-end
check runs over temporary filesystems.

### Unit Tests

- config and CLI plumbing
  - nested `[lint.zsh.plugins]` config parses correctly;
  - `--config` nested overrides for `lint.zsh.plugins` validate and layer
    correctly;
  - `--zsh-plugin-root`, `--extend-zsh-plugin-root`, `--zsh-plugin`,
    `--extend-zsh-plugin`, `--zsh-theme`, and
    `--zsh-plugin-entrypoint` parse into the expected CLI structs;
  - hidden `--no-zsh-plugin-resolution` disables the feature.
- plugin request extraction
  - static `plugins=(git docker)` plus bootstrap source emits two plugin
    requests;
  - `plugins+=(fzf)` appends are included when static;
  - dynamic plugin mutations stop automatic extraction;
  - `ZSH_THEME=agnoster` emits a built-in theme request;
  - conditional/function-local assignments do not count as automatic loads.
- framework root resolution
  - root from literal source path;
  - root from static `ZSH=...`;
  - root from `~`, `$HOME`, and `${HOME}`;
  - config root fills in when source root is dynamic.
- raw entrypoint normalization
  - relative paths resolve against project root;
  - missing files are ignored.

### End-to-End Lint Tests

- oh-my-zsh plugin file reads a binding from the user config, and resolving the
  plugin suppresses an `undefined_variable` or `unused_assignment` false
  positive that appears without plugin resolution.
- a `.zshrc` using `$HOME/.oh-my-zsh` resolves correctly with only
  `[lint.zsh.plugins.roots]`.
- a dynamic plugin list plus `[[lint.zsh.plugins.plugin-loads]]` resolves
  correctly.
- `[[lint.zsh.plugins.entrypoints]]` attaches a nonstandard plugin file to a
  matching user config.
- unrelated files do not inherit plugin loads configured for a different
  pattern.
- `shuck check --add-ignore` produces the same diagnostic set with plugin
  resolution enabled as ordinary `shuck check` before it writes directives.

### Cache Tests

- changing a resolved plugin file invalidates the cached diagnostics for the
  consuming file;
- unchanged plugin dependencies keep the cache hot;
- dependency fingerprints participate in cache misses across runs.
- stale dependency fingerprints turn a would-be cache hit back into a pending
  analysis instead of returning cached diagnostics.

### Watch Tests

- editing a resolved plugin entrypoint in `--watch` mode triggers a rerun.
- plugin entrypoints discovered only after the first run are added to the watch
  set before the next wait cycle.
- removing a plugin dependency stops watching that dependency after the next
  rerun.

### Commands

The expected repo-native checks after implementation are:

```bash
cargo test -p shuck-config zsh
cargo test -p shuck-semantic plugin_resolution
cargo test -p shuck-linter zsh
cargo test -p shuck-cli check::cache
cargo test -p shuck-cli check::watch
make test
```

If corpus fixtures are added for real zsh plugin repos, run the corresponding
targeted large-corpus subset after the first implementation lands.
