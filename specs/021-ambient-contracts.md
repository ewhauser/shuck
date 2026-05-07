# 021: Ambient Contracts

## Status

Proposed

## Summary

Add a general contract layer for external behavior that Shuck cannot recover
from the analyzed source graph. Contracts are shell-agnostic at the config
surface: a contract has an activation (`when`) and a set of effects (`reads`,
`consumes`, `provides`, and function `sets`). Zsh plugin loading from spec 020
is one activation type, not a separate contract system.

The default posture stays source-first. When Shuck can load a sourced helper,
framework module, or plugin entrypoint, it should parse that file, follow its
source closure, run the ambient collector for that file entry, and derive
`FileContract`s from real code. Contracts only describe residual behavior such
as generated code, external runtime consumption, unavailable source, or
project-specific framework conventions.

## Motivation

Spec 020 and the current zsh plugin fixtures already solve a large class of
false positives by loading real plugins:

- a Prezto module request can resolve the `zsh-autosuggestions` plugin root;
- a `zdot_use_plugin zsh-users/zsh-autosuggestions` call can resolve the same
  standalone plugin;
- the resolved plugin file registers a zsh hook;
- source-closure summarization follows the deferred hook callback and discovers
  that `ZSH_AUTOSUGGEST_STRATEGY` is read later;
- the caller assignment is then kept live without any plugin-specific variable
  table.

That is the model to preserve. It also fixes return-by-reference patterns when
callee source is reachable: sourced helpers and loaded plugin files can expose
function contracts that provide `reply` or `REPLY` to callers. Those should not
be re-modeled as hand-written contracts.

The remaining need is broader than zsh plugins. Projects may need to describe
names consumed or provided by CI runtimes, shell frameworks, generated code,
test harnesses, deployment wrappers, or plugins whose source is unavailable or
too dynamic to summarize. A single top-level contract feature gives users one
mental model instead of separate zsh, plugin, framework, and global escape
hatches.

## Design

### Goals

- Keep real source loading and source summarization as the first choice.
- Make contracts a general lint feature, not a zsh-only feature.
- Use activation types to decide when a contract applies.
- Treat `zsh_plugin` and future plugin/framework integrations as activation
  types over the same contract body.
- Keep end-user names concise and close to the user intent.
- Compile contracts into the existing `FileContract` type.
- Avoid rule-local or framework-local tables of plugin variable names.

### Non-Goals

- This spec does not replace zsh plugin resolution from spec 020.
- This spec does not model language semantics such as zsh `$+name` existence
  tests. Those belong in parser/semantic/linter logic.
- This spec does not require every plugin to ship metadata.
- This spec does not execute plugin managers or user shell code.
- This spec does not emit diagnostics from plugin files as top-level user-file
  diagnostics.

### Source-First Rule

Every proposed contract should pass this question first:

> Could Shuck load a real source file and derive this fact instead?

If yes, prefer improving source resolution, plugin resolution, helper
summarization, deferred runtime modeling, or ambient collector threading. Use a
contract only when the fact is intentionally outside the static source graph or
when the source is unavailable to the analyzed project.

Examples:

| Case | Preferred mechanism |
|---|---|
| `zsh-autosuggestions` hook reads `ZSH_AUTOSUGGEST_STRATEGY` | Load plugin source and summarize deferred hook callback |
| Sourced helper sets `reply` for caller | Source closure and function contract summarization |
| `$+functions[name]` reports undefined `+functions` | Fix zsh parameter/arithmetic semantics |
| CI injects names that a checked script reads | Contract activated for matching files |
| `ZSH_TMUX_*` options are consumed by plugin/runtime code Shuck cannot statically reach | Contract activated by `zsh_plugin` |
| `zdot` module metadata is assigned in module files and consumed by a dispatcher | Contract activated for matching files or by a resolved plugin/module |

### Contract Shape

Use one top-level config array:

```toml
[[lint.contracts]]
name = "github-actions-env"
when = "always"
files = [".github/**/*.sh"]
provides = { variables = ["GITHUB_OUTPUT", "GITHUB_ENV"] }

[[lint.contracts]]
name = "oh-my-zsh-tmux"
when = { type = "zsh_plugin", framework = "oh-my-zsh", plugin = "tmux" }
files = ["**/.zshrc"]
consumes = { prefixes = ["ZSH_TMUX_"] }

[[lint.contracts]]
name = "zdot-module-metadata"
when = "always"
files = ["modules/**/*.zsh"]
consumes = { names = ["ZDOT_MODULE"], prefixes = ["ZDOT_"] }
```

`files` is an optional path filter. If omitted, the activation decides
applicability. Users can intentionally make a repository-wide contract with
`when = "always"` and no `files`, but docs should recommend path filters for
most contracts.

### Activation Types

Activation decides when a contract is eligible.

```toml
when = "always"
when = { type = "zsh_plugin", framework = "oh-my-zsh", plugin = "tmux" }
when = { type = "zsh_theme", framework = "oh-my-zsh", theme = "agnoster" }
```

Initial activation types:

| Activation | Meaning |
|---|---|
| `always` | Apply to matching files without an additional runtime condition. This is the global-contract form. |
| `zsh_plugin` | Apply when Shuck observes or configures a matching zsh plugin request and plugin resolution is enabled. |
| `zsh_theme` | Apply when Shuck observes or configures a matching zsh theme request and plugin resolution is enabled. |

Future activation types can reuse the same contract body:

- `sourced_file`, for contracts attached to a specific helper path;
- `command`, for framework commands that establish ambient state;
- `shell`, for shell-specific entry contracts that are too project-specific for
  built-in ambient providers.

### Effects

Effect names should be short and user-facing. They compile to `FileContract`
fields internally.

```toml
reads = ["CALLER_VALUE"]
consumes = { names = ["FAST_WORK_DIR"], prefixes = ["ABBR_"], all = false }
provides = { variables = ["reply"], functions = ["helper"] }
functions = [
  { name = "helper", reads = ["CALLER_VALUE"], sets = ["REPLY"] },
]
```

Mapping:

| User field | Internal meaning |
|---|---|
| `reads` | `required_reads`; creates synthetic reads in the importer. |
| `consumes.names` | `externally_consumed_binding_names`; keeps exact assignments live. |
| `consumes.prefixes` | `externally_consumed_binding_prefixes`; keeps prefix assignments live. |
| `consumes.all` | `externally_consumed_bindings`; rare escape hatch. |
| `provides.variables` | `provided_bindings` with variable kind. |
| `provides.functions` | `provided_bindings` and empty `provided_functions` for callable names. |
| `functions[].reads` | Required reads for a provided function. |
| `functions[].sets` | Variables a provided function may set for its caller, such as `reply` or `REPLY`. |

Advanced metadata such as certainty and file-entry initialization can be added
later through object forms if real users need them. The first user-facing
version should prefer simple string lists.

### Plugin-Provided Metadata

Plugin repositories may expose residual contracts in a sidecar file:

```toml
# .shuck-contracts.toml
version = 1

[[contracts]]
name = "abbr-options"
consumes = { prefixes = ["ABBR_"] }
```

When a sidecar is discovered from a resolved plugin entrypoint, `when` defaults
to the plugin activation that discovered it. A sidecar may still include an
explicit `when` to narrow one file to a specific plugin or theme.

Supported filenames:

- `.shuck-contracts.toml`
- `shuck-contracts.toml`

Discovery starts from resolved plugin entrypoints. Shuck searches:

1. the plugin root known to the resolver, if any;
2. the entrypoint directory.

Sidecar discovery never replaces source loading. If both a source summary and a
sidecar contract provide facts, Shuck merges them as candidate contracts and
deduplicates repeated names.

### Internal Flow

The existing source-closure flow remains the join point:

```text
primary file
  -> file-entry ambient collector
  -> source refs and plugin requests
  -> resolved helper/plugin entrypoints
  -> helper/plugin source summaries
  -> activated config and sidecar contracts
  -> merged FileContract
  -> final semantic resolution and linter facts
```

`PluginResolution.file_entry_contracts` remains the right hook for contracts
activated by plugin resolution. The resolver should fill this field from
compiled config and sidecar metadata after resolving the logical request. It
should not fill it from hard-coded framework/plugin name tables.

Contracts activated by `always` can use the existing
`SemanticBuildOptions.file_entry_contract` path for the primary file and the
existing `FileEntryContractCollectorFactory` path for helper/plugin summaries.
If implementation needs a named provider, the provider should compile to
`FileContract` before semantic application.

### Crate Boundaries

- `shuck-semantic` owns `FileContract`, contract merging, and semantic
  application.
- `shuck-semantic` should not parse TOML contract metadata.
- `shuck-config` owns deserializable config shapes.
- `shuck-cli` compiles config and sidecar metadata into `FileContract`s and
  installs them in the resolver/collector pipeline.
- `shuck-linter` consumes semantic facts and should not know contract origins.

### Validation Rules

- `name` is optional but recommended for diagnostics and generated docs.
- `files` patterns use the same matching rules as other per-file settings.
- variable names must be valid shell names.
- prefixes must be non-empty and valid shell-name prefixes.
- `when = "always"` is the only string shorthand in v1.
- unknown activation types are configuration errors.
- sidecar metadata outside resolved plugin roots, configured plugin
  entrypoints, or the analyzed project tree is ignored unless explicitly
  configured.
- malformed config contracts are configuration errors.
- malformed sidecar contracts produce a diagnostic tied to the sidecar file and
  do not apply partial contract data from that file.

### Cache And Watch

Contract inputs affect diagnostics and must participate in dependency tracking.

Cache fingerprints include:

- resolved plugin entrypoints;
- sidecar contract files that contributed to a plugin resolution;
- project config files that define contracts;
- helper files summarized because of source closure.

Watch mode refreshes these targets after each run, matching the dependency
refresh model from spec 020.

## Alternatives Considered

### Keep Contracts Under `[lint.zsh]`

Rejected. The same model is useful for non-zsh scripts and project runtimes.
Zsh plugin loading should be an activation type over a common contract body,
not the namespace that owns all contracts.

### Separate Global And Plugin Contract Systems

Rejected. This creates duplicate names, duplicate validation, and confusing
mental models. `always` and `zsh_plugin` activations are enough to distinguish
the two cases.

### Use Verbose Internal Field Names In Config

Rejected. Names like `externally-consumed-binding-prefixes` describe
implementation details, not user intent. Config should use short terms:
`reads`, `consumes`, `provides`, and `sets`.

### Hard-Code Plugin Variable Tables In Rust

Rejected. It is quick for a handful of known plugins, but it puts framework and
plugin policy into semantic layout code and repeats the false-positive
heuristic problem that plugin loading was meant to remove.

### Make Contracts The Primary Plugin Model

Rejected. The current fixtures already prove that loading real plugin source
solves meaningful cases, including deferred zsh hook reads. Contracts should
not become a substitute for improving resolution and source summarization.

## Verification

### Unit Tests

- parse valid `[[lint.contracts]]` entries;
- parse `when = "always"`;
- parse `zsh_plugin` and `zsh_theme` activations;
- reject unknown activation types;
- reject malformed names and prefixes;
- compile `reads`, `consumes`, `provides`, and `functions[].sets` into expected
  `FileContract` values;
- parse `.shuck-contracts.toml` sidecar metadata through the same contract body
  schema;
- deduplicate repeated names and prefixes deterministically.

### Semantic Tests

- plugin source summaries still derive `ZSH_AUTOSUGGEST_STRATEGY` reads without
  any contract metadata;
- source closure still derives `reply`/`REPLY` function contracts when helper
  source is reachable;
- `always` contracts apply to matching primary files and helper/plugin files;
- `zsh_plugin` contracts apply only when the matching plugin request is loaded;
- `consumes.prefixes` keeps matching C001 candidates live without creating fake
  lexical reads;
- `reads` produces synthetic reads at the importer/load site.

### End-To-End Tests

- an `always` contract can model a project runtime variable in a non-zsh shell
  script;
- a `tmux` oh-my-zsh contract keeps `ZSH_TMUX_*` assignments live only when the
  `tmux` plugin is loaded;
- a `zsh-abbr` sidecar contract keeps `ABBR_*` assignments live when the
  standalone plugin is resolved;
- a `zdot` contract can describe project-specific module metadata without
  making those names first-class zsh runtime globals;
- disabling plugin resolution prevents `zsh_plugin` contracts from applying,
  while `always` contracts still apply to matching files.

### Commands

```bash
cargo test -p shuck-config contracts
cargo test -p shuck-semantic contract
cargo test -p shuck-linter unused_assignment
cargo test -p shuck-cli contract
make test
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001,C006
```

The large-corpus run belongs after implementation, not after this spec-only
change.
