# Declarative Built-In Contracts

This directory contains Shuck's repository-authored built-in ambient contracts.
They are source files for the build, not runtime configuration files.

## Source First

Before adding a contract, ask:

> Could Shuck load real source and derive this fact instead?

If yes, prefer improving source resolution, source closure, helper
summarization, or deferred runtime modeling.

Use a declarative contract only for residual behavior such as:

- runtime-provided names available before file entry;
- names or prefixes consumed by a framework after the file runs;
- stable plugin or theme conventions that are not recoverable from reachable
  source.

## Layout

Contracts live under a top-level `contracts/` tree, grouped by ecosystem:

```text
contracts/
  bash/
  bash-it/
  zsh/
    runtime/
    plugins/
    zdot/
    themes/
```

Each YAML file uses this shape:

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
    files:
      - "**/plugins/tmux/**"
    effects:
      consumes:
        prefixes:
          - ZSH_TMUX_
```

## Fields

- `id`: Stable built-in selector id.
- `groups`: Stable built-in selector groups.
- `label`: Optional human-facing label.
- `when`: One of:
  - `always`
  - `zsh_plugin` with `framework` and `plugin`
  - `zsh_theme` with `framework` and `theme`
- `match`: Optional extra predicates for `when.type: always`.
- `files`: Optional path filters matched against the primary file for
  `always` contracts and the requesting file for plugin/theme contracts.
- `effects.reads`: Names read at activation time.
- `effects.consumes.names`: Exact names consumed outside the lexical source.
- `effects.consumes.prefixes`: Name prefixes consumed outside the lexical
  source.
- `effects.consumes.all`: Escape hatch for consuming all non-local
  assignments.
- `effects.provides.variables`: Definite initialized file-entry variables.
- `effects.provides.ambient-variables`: Ambient-only variables that exist
  before file entry but are not initialized by the file itself.
- `effects.provides.functions`: Definite function bindings.
- `effects.provides.caller-scoped-array-length-names: true`: Re-export the
  caller-scoped zsh array length names discovered in the file.
- `effects.vocabulary.names`: Names that belong to the active runtime or
  ecosystem vocabulary without implying that the name is initialized, read, or
  externally consumed.
- `effects.functions`: Function-specific caller reads and sets.

## Match Predicates

`match` is only supported for `when.type: always`.

### `match.shell`

- `any`: No shell gating.
- `zsh`: Requires a zsh file.
- `zsh_or_unknown`: Allows zsh and unknown shell files.
- `zsh_runtime`: Allows zsh files, plus unknown-shell files whose path shape
  looks like zsh runtime or dotfile content.

### `match.source`

These predicates are ANDed together:

- `contains-any`
- `mentions-any-names`
- `mentions-all-names`
- `assigns-any-names`
- `assigns-all-names`
- `assigns-any-prefixes`
- `loads-zsh-modules-any`
- `loads-zsh-modules-all`
- `static-assignment-function-defs`
- `probable-function-definition: true`
- `source-command: true`
- `completion-initializer-invoked: true`
- `loads-zsh-colors: true`
- `caller-scoped-array-length-names: true`

Use the narrowest predicate that matches the residual runtime fact you are
trying to encode.

## Validation

The `shuck-linter` build script validates:

- schema version;
- duplicate ids;
- invalid groups or selector tokens;
- invalid shell names and prefixes;
- invalid file globs;
- empty effect bodies;
- invalid activation field combinations;
- invalid `match` usage on non-`always` contracts.

## Commands

```bash
cargo test -p shuck-linter build_script
cargo test -p shuck-linter ambient_contracts
cargo test -p shuck-cli contract
```
