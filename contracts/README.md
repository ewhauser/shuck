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
  runtime/
  zsh/
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
    effects:
      consumes:
        prefixes:
          - ZSH_TMUX_
```

## Fields

- `id`: Stable built-in selector id.
- `groups`: Stable built-in selector groups.
- `label`: Optional human-facing label.
- `when`: One of `always`, `zsh_plugin`, or `zsh_theme`.
- `files`: Optional path filters matched against the requesting or primary file.
- `effects.reads`: Names read at activation time.
- `effects.consumes.names`: Exact names consumed outside the lexical source.
- `effects.consumes.prefixes`: Name prefixes consumed outside the lexical
  source.
- `effects.consumes.all`: Escape hatch for consuming all non-local
  assignments.
- `effects.provides.variables`: Definite initialized file-entry variables.
- `effects.provides.functions`: Definite function bindings.
- `effects.functions`: Function-specific caller reads and sets.

## Validation

The `shuck-linter` build script validates:

- schema version;
- duplicate ids;
- invalid groups or selector tokens;
- invalid shell names and prefixes;
- invalid file globs;
- empty effect bodies;
- invalid activation field combinations.

## Commands

```bash
cargo test -p shuck-linter build_script
cargo test -p shuck-linter ambient_contracts
cargo test -p shuck-cli contract
```
