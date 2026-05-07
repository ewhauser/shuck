# 022: Declarative Built-In Contracts

## Status

Proposed

## Summary

Add a repository-local declarative authoring layer for Shuck's built-in ambient
contracts. Contributors write well-known contract data as YAML under a
top-level `contracts/` tree, and `shuck-linter` generates Rust registry tables
from that YAML at build time. Runtime behavior stays the same shape as spec
021: activated contracts compile into the existing `FileContract` fields and
join the current ambient-contract and plugin-resolution pipeline. Shuck does
not parse built-in YAML while linting.

This spec extends [020-zsh-plugin-resolution.md](020-zsh-plugin-resolution.md)
and [021-ambient-contracts.md](021-ambient-contracts.md). Spec 020 resolves
real plugin source files. Spec 021 defines the contract model and runtime
activation semantics. This spec defines how the built-in well-known registry is
authored, generated, validated, documented, and migrated away from ad hoc Rust
tables.

## Motivation

Specs 020 and 021 intentionally made real source loading the first choice.
When Shuck can resolve a sourced helper, zsh plugin entrypoint, theme, or
framework file, it should parse that file and summarize real reads, provided
bindings, provided functions, and source-closure behavior. That already handles
important plugin cases without hard-coded per-plugin data.

The remaining built-in contracts are residual ecosystem facts:

- names provided by a runtime before the file starts;
- options or prefixes consumed by a framework after a file is sourced;
- generated or indirect behavior that cannot be recovered from concrete source;
- plugin-manager conventions that expose a stable contract but not a stable
  source edge;
- common CI or shell runtimes that users should not have to model themselves.

Those facts are easier for AI and contributors to add when they are small,
declarative documents instead of hand-written Rust entries. The current
centralized registry in `crates/shuck-linter/src/ambient_contracts/contracts.rs`
is the right runtime join point, but it is not the right long-term authoring
surface if Shuck needs broad plugin and runtime coverage.

At the same time, built-in contracts are on the hot path for `shuck check`.
Shuck should not deserialize bundled YAML on every invocation or allocate
`Name`s for contracts that do not apply. Build-time generation gives
contributors a data-file workflow while preserving cheap runtime activation.

## Design

### Goals

- Keep source resolution and source-closure summarization as the primary plugin
  model.
- Make well-known contract authoring declarative and approachable for AI and
  contributors.
- Generate Rust tables from declarative YAML at build time.
- Do not parse built-in YAML at runtime.
- Materialize `FileContract` values only after disabled-selector checks,
  activation matching, and optional path matching succeed.
- Reuse the existing `AmbientContractConfig`, custom TOML contract shape, and
  `FileContract` effect model from spec 021.
- Preserve `shuck-linter` ownership of well-known lint-facing contracts.
- Keep dynamic ambient providers in Rust when they inspect source text, paths,
  normalized commands, or shell-specific runtime signals.
- Provide deterministic validation, generated cache-key descriptors, and
  contributor documentation.

### Non-Goals

- This spec does not make plugin-authored contract files a runtime discovery
  mechanism. Built-in declarative YAML is repository source, not a user plugin
  API.
- This spec does not replace user-authored `[[lint.contracts.custom]]` entries
  in `.shuck.toml`.
- This spec does not move source-resolution layout logic out of
  `crates/shuck-semantic/src/source_closure/plugin_managers`.
- This spec does not require complete coverage for every zsh plugin manager in
  the first implementation.
- This spec does not generate parser or semantic logic for dynamic shell
  behavior. Complex source inspection remains Rust code.
- This spec does not add a runtime dependency on `serde_yaml` to
  `shuck-linter`.

### Source-First Boundary

Declarative contracts must pass the same source-first test from spec 021:

> Could Shuck load a real source file and derive this fact instead?

If yes, improve plugin resolution, source closure, helper summarization,
deferred runtime modeling, or semantic facts. Use declarative contracts only
for facts outside the static source graph or facts whose source exists but does
not expose the stable contract in a recoverable way.

Examples:

| Case | Preferred mechanism |
|---|---|
| A plugin hook reads an option variable from a resolved callback | Source closure and deferred callback summarization |
| A sourced helper sets `reply` for its caller | Source closure and function contract summarization |
| A framework consumes `ZSH_TMUX_*` after a user file has run | Declarative `zsh_plugin` contract |
| A CI runtime initializes `GITHUB_OUTPUT` before a script starts | Declarative `always` contract |
| A zsh special parameter is always available in interactive zsh | Rust ambient runtime provider if it depends on shell/path/source signals |
| A plugin manager command DSL discovers plugin requests | Rust plugin manager adapter |

The declarative layer is an authoring surface for the existing contract model,
not a second semantic model.

### Repository Layout

Built-in contract sources live at the repository root:

```text
contracts/
  README.md
  zsh/
    oh-my-zsh/
      contracts.yaml
      plugins/
        tmux.yaml
      themes/
        agnoster.yaml
    zdot/
      contracts.yaml
  runtime/
    github-actions.yaml
```

`contracts/README.md` documents the schema, examples, validation rules, and
source-first policy for contributors.

`crates/shuck-linter` exposes the files to Cargo through a crate-local symlink:

```text
crates/shuck-linter/contracts -> ../../contracts
```

This mirrors the existing `crates/shuck-linter/rules -> ../../docs/rules`
packaging pattern. The build script reads `crates/shuck-linter/contracts` so
packaged builds see the same files Cargo included for the crate.

The directory layout is organizational. Contract identity comes from the
document `id`, not from the path, but validation should warn through build
errors when a path and id clearly disagree, such as an
`oh-my-zsh/plugins/tmux.yaml` file declaring a non-`tmux` zsh plugin activation.

### YAML Document Shape

Every YAML file contains one document with a schema version and one or more
contracts:

```yaml
version: 1
contracts:
  - id: zsh/oh-my-zsh/plugin/tmux
    groups:
      - zsh
      - zsh/oh-my-zsh
      - zsh/oh-my-zsh/plugin
    label: Oh My Zsh tmux plugin
    when:
      type: zsh_plugin
      framework: oh-my-zsh
      plugin: tmux
    effects:
      consumes:
        prefixes:
          - ZSH_TMUX_
```

The same file shape also covers global file-entry contracts:

```yaml
version: 1
contracts:
  - id: runtime/github-actions/env
    groups:
      - runtime
      - runtime/github-actions
    label: GitHub Actions environment files
    when:
      type: always
    files:
      - ".github/**/*.sh"
    effects:
      provides:
        variables:
          - GITHUB_OUTPUT
          - GITHUB_ENV
```

The `label` is human-facing and optional. It is not a selector and does not
participate in cache keys.

### Schema

The build script deserializes this Rust shape in build-only code:

```rust
#[derive(Debug, Deserialize)]
struct ContractDocument {
    version: u32,
    contracts: Vec<DeclarativeContract>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeContract {
    id: String,
    groups: Vec<String>,
    label: Option<String>,
    when: DeclarativeActivation,
    files: Option<Vec<String>>,
    effects: DeclarativeEffects,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum DeclarativeActivation {
    Always,
    ZshPlugin { framework: String, plugin: String },
    ZshTheme { framework: String, theme: String },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeEffects {
    reads: Vec<String>,
    consumes: DeclarativeConsumes,
    provides: DeclarativeProvides,
    functions: Vec<DeclarativeFunctionEffects>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeConsumes {
    names: Vec<String>,
    prefixes: Vec<String>,
    all: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeProvides {
    variables: Vec<String>,
    functions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DeclarativeFunctionEffects {
    name: String,
    reads: Option<Vec<String>>,
    sets: Option<Vec<String>>,
}
```

Build-time deserialization can use `serde_yaml`, which is already a
`shuck-linter` build dependency. Runtime `shuck-linter` must keep
`serde_yaml` out of normal dependencies.

### Effect Mapping

Declarative effects compile to the same fields as spec 021 custom contracts:

| YAML field | Internal effect |
|---|---|
| `effects.reads` | `FileContract.required_reads` |
| `effects.consumes.names` | `FileContract.externally_consumed_binding_names` |
| `effects.consumes.prefixes` | `FileContract.externally_consumed_binding_prefixes` |
| `effects.consumes.all` | `FileContract.externally_consumed_bindings` |
| `effects.provides.variables` | initialized definite variable `ProvidedBinding`s |
| `effects.provides.functions` | definite function `ProvidedBinding`s |
| `effects.functions[].reads` | `FunctionContract.required_reads` |
| `effects.functions[].sets` | definite variable bindings provided by the function |

The build script does not generate `FileContract` constants. `FileContract`
contains `Vec` fields and `Name` values, so the generated registry should store
static string slices and use small runtime materializer helpers only after a
contract matches.

### Generated Runtime Data

`crates/shuck-linter/build.rs` generates
`$OUT_DIR/ambient_contracts_data.rs`.

The checked-in Rust side owns the runtime descriptor types:

```rust
struct DeclarativeContractDescriptor {
    id: &'static str,
    groups: &'static [&'static str],
    label: Option<&'static str>,
    activation: DeclarativeActivationDescriptor,
    files: &'static [&'static str],
    effects: DeclarativeEffectsDescriptor,
}

enum DeclarativeActivationDescriptor {
    Always,
    ZshPlugin {
        framework: &'static str,
        plugin: &'static str,
    },
    ZshTheme {
        framework: &'static str,
        theme: &'static str,
    },
}

struct DeclarativeEffectsDescriptor {
    reads: &'static [&'static str],
    consumes_names: &'static [&'static str],
    consumes_prefixes: &'static [&'static str],
    consumes_all: bool,
    provides_variables: &'static [&'static str],
    provides_functions: &'static [&'static str],
    functions: &'static [DeclarativeFunctionDescriptor],
}

struct DeclarativeFunctionDescriptor {
    name: &'static str,
    reads: &'static [&'static str],
    sets: &'static [&'static str],
}
```

The generated file contains only static data:

```rust
pub(super) const DECLARATIVE_CONTRACTS: &[DeclarativeContractDescriptor] = &[
    DeclarativeContractDescriptor {
        id: "zsh/oh-my-zsh/plugin/tmux",
        groups: &["zsh", "zsh/oh-my-zsh", "zsh/oh-my-zsh/plugin"],
        label: Some("Oh My Zsh tmux plugin"),
        activation: DeclarativeActivationDescriptor::ZshPlugin {
            framework: "oh-my-zsh",
            plugin: "tmux",
        },
        files: &[],
        effects: DeclarativeEffectsDescriptor {
            reads: &[],
            consumes_names: &[],
            consumes_prefixes: &["ZSH_TMUX_"],
            consumes_all: false,
            provides_variables: &[],
            provides_functions: &[],
            functions: &[],
        },
    },
];
```

`contracts.rs` includes that generated data and adapts it into the existing
well-known registry flow. The build script should emit deterministic output:
files sorted by path, contracts sorted by `id`, groups and effect lists
deduplicated in stable order.

### Runtime Activation Flow

The runtime flow stays lazy:

1. Config parsing builds `AmbientContractConfig`.
2. `ResolvedAmbientContracts::resolve` collects disabled selectors and
   `custom[].replaces`.
3. Disabled selectors are validated against all built-in Rust and declarative
   ids/groups.
4. Enabled declarative file-entry and request contract ids are stored in
   `ResolvedAmbientContracts`.
5. During file-entry collection, only enabled declarative `always` contracts
   are considered.
6. During plugin request resolution, only enabled declarative request
   contracts whose activation matches the observed request are considered.
7. Optional `files` matchers are compiled lazily only after selector and
   activation checks pass.
8. The effect descriptor materializes a `FileContract` and merges it through
   the same helpers used by custom contracts.

This preserves the startup target:

- no built-in YAML parsing during `shuck check`;
- no `Name` allocation for inactive contracts;
- disabled families are skipped before path matching and materialization;
- request contracts for unrelated frameworks/plugins are skipped by string
  comparison;
- path glob cost is paid only for contracts that reached path matching.

### File Pattern Matching

Declarative `files` uses the same matching semantics as custom contracts and
per-file settings:

- basename match;
- project-root relative match;
- absolute path match;
- `!` negation entries;
- positive entries are ORed, negations exclude.

Generated descriptors store pattern strings. The runtime side uses a small
`OnceLock<Vec<CompiledPathMatcher>>` cache per descriptor when file matching is
needed. Empty `files` means the activation alone decides applicability.

Request-based activations match the file that requested the plugin or theme,
not the resolved plugin entrypoint.

### Dynamic Rust Providers

Not every current well-known contract should move to YAML.

Keep Rust providers for contracts that inspect source text, normalized command
streams, path tags, or shell-specific runtime conditions:

- `zsh_runtime`;
- `zsh_config`;
- `zsh_module_metadata`;
- `sourced_runtime`;
- `zsh_caller_arrays`;
- future plugin-manager adapters that parse command DSLs or infer dynamic
  plugin requests.

Move contracts to declarative YAML when the behavior is a static activation
plus static effects, such as:

- exact plugin consumes a prefix;
- exact plugin reads one or more caller names at load time;
- exact theme provides or consumes stable names;
- a runtime file pattern provides initialized environment variables;
- a framework convention consumes exact metadata names or prefixes.

### Crate Boundaries

- `shuck-linter` owns declarative built-in contract descriptors, generation,
  validation, and materialization.
- `shuck-linter` build code may parse YAML; runtime `shuck-linter` may not.
- `shuck-semantic` continues to own `FileContract`, `PluginRequest`,
  `PluginResolution`, and semantic application.
- `shuck-semantic` does not parse declarative YAML and does not own the
  well-known registry.
- `shuck-config` continues to own user TOML config shapes.
- `shuck-cli` continues to compile user config into `ResolvedAmbientContracts`
  and `ResolvedZshPluginSettings`.

### Build Script Integration

Extend `crates/shuck-linter/build.rs`:

- keep the existing rule metadata generation;
- add `contracts_dir = manifest_dir.join("contracts")`;
- emit `cargo:rerun-if-changed` for the directory and every YAML file;
- parse every `*.yaml` recursively;
- validate schema and semantic rules;
- generate `ambient_contracts_data.rs` into `OUT_DIR`;
- keep build-script parser functions testable under `#[cfg(test)]`.

`crates/shuck-linter/src/ambient_contracts/contracts.rs` should include:

```rust
include!(concat!(env!("OUT_DIR"), "/ambient_contracts_data.rs"));
```

or delegate to a new sibling module such as
`ambient_contracts/declarative.rs` that owns descriptor types and includes the
generated constants.

If the contracts directory is missing in a packaged crate, build should fail
with a clear error. A missing directory means the package is incomplete, not
that Shuck should silently ship without built-in contracts.

### Validation Rules

Build-time validation rejects:

- unsupported schema versions;
- duplicate contract ids across all YAML files;
- empty ids or groups;
- ids, groups, framework names, plugin names, theme names, variables, function
  names, and prefixes with leading/trailing whitespace;
- ids or groups containing whitespace;
- a group that is equal to the id;
- an empty `effects` body;
- a `zsh_plugin` activation without `framework` or `plugin`;
- a `zsh_theme` activation without `framework` or `theme`;
- `always` activations that include plugin or theme fields;
- invalid globs in `files`;
- duplicated names inside one effect list after normalization;
- invalid shell identifier names for fields that require complete names;
- invalid identifier prefixes for `consumes.prefixes`.

Runtime config validation rejects:

- disabled selectors that are not `*`, a known built-in id, or a known
  built-in group;
- `custom[].replaces` selectors that are not known built-in ids or groups;
- duplicate custom contract ids;
- malformed custom contract names, prefixes, and file globs.

The first implementation may validate names using the existing portable shell
identifier subset. If zsh-specific names need broader syntax, add a
shell-profile-aware validator rather than weakening all validation.

### Cache Keys

Built-in declarative contracts are part of the compiled binary. Cache
fingerprints do not need to include YAML file mtimes at runtime.

The existing effective settings snapshot must include:

- enabled declarative contract ids after disabled and replacement selectors are
  applied;
- custom contract descriptors;
- plugin-resolution enabled state.

It does not need to preserve the original disabled selector text when two
different selector spellings produce the same enabled built-in set.

When a built-in contract changes, the Shuck version or binary hash changes with
the compiled artifact. During local development, Cargo rebuilds
`shuck-linter` because the build script emits `rerun-if-changed` for the YAML
inputs.

### Documentation And Contributor Workflow

Add `contracts/README.md` with:

- the source-first rule;
- a minimal plugin contract example;
- a minimal runtime contract example;
- field-by-field schema reference;
- guidance for choosing `reads` vs `consumes`;
- activation examples for `always`, `zsh_plugin`, and `zsh_theme`;
- validation and test commands;
- clean-room reminders for compatibility-oriented research.

The README should tell contributors to add contract data from project-owned
knowledge, source inspection, official shell/runtime documentation, or
black-box behavior probes. It must not encourage copying ShellCheck diagnostic
text, examples, or source-derived wording.

Generated website or docs inventory can be added after the initial
implementation, but the generated data should make it easy to list built-in ids
and groups later.

### Migration Strategy

Phase 1:

- add `contracts/` and the crate-local symlink;
- add build-script parsing and generated descriptor output;
- add runtime descriptor types and materializers;
- move `zsh/oh-my-zsh/plugin/tmux` from the hard-coded request registry into
  `contracts/zsh/oh-my-zsh/plugins/tmux.yaml`;
- keep existing Rust providers for dynamic zsh runtime/config/module behavior;
- add focused unit and end-to-end tests for the moved tmux contract.

Phase 2:

- move other static well-known plugin/theme/runtime facts into YAML as they are
  added or touched;
- add `contracts/README.md` examples for common contribution paths;
- tighten runtime disabled/replaces validation against the generated selector
  inventory;
- add generated docs or website data if contributors need a browsable catalog.

Phase 3:

- audit `ambient_contracts` for static tables that should become declarative;
- keep dynamic source/path/command-sensitive providers in Rust;
- add broader large-corpus verification for rule families affected by new
  contracts, especially unused-assignment and uninitialized-read behavior.

## Concrete Code Changes

### `contracts/`

- create the root directory;
- add `README.md`;
- add initial YAML for `zsh/oh-my-zsh/plugin/tmux`;
- use `version: 1` and `contracts: [...]` in every YAML file.

### `crates/shuck-linter`

- add `contracts -> ../../contracts` symlink;
- extend `build.rs` to parse contract YAML and emit
  `ambient_contracts_data.rs`;
- add build-script tests for parsing, validation, sorting, and generated Rust
  snippets;
- add runtime descriptor/materializer code under `ambient_contracts`;
- merge generated declarative contracts into
  `ResolvedAmbientContracts::resolve`;
- include generated ids in `EffectiveAmbientContracts`;
- replace the hard-coded `oh_my_zsh_tmux_requesting_file_contract` registry
  entry with generated data;
- keep `merge_contract`, `imported_contract_from_effects`, and
  `requesting_file_contract_from_effects` as shared materialization helpers
  where practical.

### `crates/shuck-config`

- keep the existing user TOML schema;
- add or tighten tests for disabled and replacement selectors once
  `shuck-linter` exposes the built-in selector inventory needed for
  validation;
- do not add runtime YAML config parsing.

### `crates/shuck-cli`

- keep current config compilation flow;
- ensure selector validation errors surface as configuration errors with the
  offending selector and the accepted selector form;
- keep effective contract state in check cache keys;
- add integration tests that prove declarative built-ins behave the same as the
  old Rust tmux contract.

### `crates/shuck-semantic`

- no declarative YAML changes;
- keep `PluginResolution.file_entry_contracts` and
  `PluginResolution.requesting_file_contract` as the request-site import hooks;
- add semantic tests only if the migration reveals a gap in how generated
  request contracts are applied.

## Alternatives Considered

### Runtime-Parse Built-In YAML

Rejected. This gives contributors a declarative format but makes every Shuck
run pay deserialization and validation costs for built-in data. It also creates
more startup work for contracts that cannot possibly activate for the analyzed
file. Build-time generation gives the same authoring ergonomics with static
runtime data.

### Keep All Built-Ins As Hand-Written Rust

Rejected. Centralized Rust tables are acceptable for a few entries, but they do
not scale to broad plugin and runtime coverage. Hand-written entries make small
data additions look like code changes, which is harder for AI and casual
contributors to produce consistently.

### Store Generated Rust In The Repository

Rejected for the first implementation. Checked-in generated Rust makes reviews
larger and creates a stale-output failure mode. The existing rule metadata path
already generates Rust in `OUT_DIR`, and contract generation should follow that
local pattern.

### Put The Registry In `shuck-semantic`

Rejected. `shuck-semantic` owns contract application types and plugin resolver
interfaces, but the list of lint-facing ecosystem assumptions belongs next to
the linter ambient-contract machinery.

### Make YAML The User Config Format For Contracts

Rejected. Users already have TOML config under `[lint.contracts]`, and spec 021
defines that surface. Built-in YAML is for repository authorship and build-time
generation only.

### Move Dynamic Providers To YAML

Rejected. Providers that inspect source text, path context, command streams, or
shell-specific runtime signals are logic, not static contract data. They should
remain Rust code until a concrete declarative form can express them without
losing precision.

## Verification

### Build-Script Tests

- parse a single-contract YAML document;
- parse a multi-contract YAML document;
- reject unknown schema versions;
- reject duplicate ids across files;
- reject invalid activation fields;
- reject invalid names and prefixes;
- reject invalid globs;
- produce deterministic sorted generated output;
- include all contract files in `cargo:rerun-if-changed` output.

### Linter Unit Tests

- generated tmux contract appears in the built-in selector inventory;
- disabling `zsh/oh-my-zsh/plugin/tmux` skips the generated contract;
- disabling `zsh/oh-my-zsh` skips the generated contract by group;
- generated request contracts are not materialized for unrelated plugin
  requests;
- generated request contracts materialize the expected `FileContract` only
  after activation matches;
- generated `always` contracts apply to matching file paths and skip
  non-matching paths;
- generated path matchers handle basename, relative, absolute, and negated
  patterns.

### CLI And End-To-End Tests

- a `.zshrc` that loads the oh-my-zsh `tmux` plugin keeps `ZSH_TMUX_*`
  assignments live through the generated contract;
- the same `.zshrc` reports the assignment when the built-in contract is
  disabled by exact id;
- disabling the `zsh/oh-my-zsh` group has the same effect;
- custom `[[lint.contracts.custom]]` entries still work and can replace a
  generated built-in;
- plugin resolution disabled prevents generated `zsh_plugin` contracts from
  applying, while generated `always` contracts still apply.

### Commands

```bash
cargo test -p shuck-linter build_script
cargo test -p shuck-linter ambient_contracts
cargo test -p shuck-cli contract
make test
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001,C006
```

The large-corpus command is required before landing the implementation because
these contracts affect unused-assignment and uninitialized-read behavior. It is
not required for this spec-only change.
