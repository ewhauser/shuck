# shuck

A shell script linter, written in Rust.

Shuck parses and analyzes shell scripts to catch common bugs, style issues, and portability problems. It also lints shell embedded in supported non-shell files such as GitHub Actions workflows. A caching layer keeps incremental runs fast.

## Features

- High performance — ~20x faster than ShellCheck
- Linting with rules across correctness and style categories
- Safe and unsafe fix support for selected diagnostics
- Multi-dialect support: bash, sh/POSIX, mksh, zsh
- Automatic file discovery via extensions and shebang detection
- Embedded shell extraction for GitHub Actions workflows and composite actions
- ShellCheck suppression compatibility (`# shellcheck disable=SC2086`)

## Installation

### Homebrew

```sh
brew install ewhauser/tap/shuck
```

### From source

```sh
cargo install shuck-cli
```

### Pre-built binaries

Pre-built binaries are available for macOS (aarch64) and Linux (x86_64) from the [releases page](https://github.com/ewhauser/shuck/releases).

## Usage

### Lint

```sh
# Check files and directories
shuck check script.sh src/

# Check the current directory
shuck check .

# Check GitHub Actions workflow `run:` blocks
shuck check .github/workflows/ci.yml

# Check a composite action
shuck check action.yml

# Read from stdin
echo 'echo $foo' | shuck check -

# Apply safe fixes automatically
shuck check --fix .

# Apply opt-in unsafe fixes too
shuck check --unsafe-fixes .

# Skip the cache
shuck check --no-cache .

# Override the cache location
shuck --cache-dir .tmp/shuck-cache check .
```

### Clean caches

```sh
# Remove cache entries for the current project
shuck clean
```

## Output

`shuck check` prints rich code-frame diagnostics by default:

```
warning[C001]: variable `tmp` is assigned but never used
 --> deploy.sh:14:1
  |
14 | tmp=$(mktemp)
  | ^^^
  |
```

Use `--output-format concise` to keep the legacy one-line format:

```
path:line:col: severity[CODE] message
```

```
deploy.sh:14:1: warning[C001] variable `tmp` is assigned but never used
deploy.sh:31:10: error[C006] undefined variable `DEPLY_ENV`
deploy.sh:45:3: warning[S005] legacy backtick command substitution
.github/workflows/ci.yml:12:11: warning[C001] jobs.test.steps[0].run: variable `summary` is assigned but never used
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0`  | No issues found |
| `1`  | Lint violations or parse errors detected |
| `2`  | Runtime error (bad arguments, I/O failure) |

## Rules

Shuck ships with rules organized into four categories:

| Category | Prefix | Description |
|----------|--------|-------------|
| Correctness | C | Bugs, errors, and likely mistakes. Enabled by default. |
| Style | S | Code quality and best-practice suggestions. |
| Performance | P | Inefficient patterns that have simpler or faster alternatives. |
| Portability | X | Bash-isms and shell-specific constructs that break under POSIX or other shells. |

Each rule has a short code (e.g., `C006`, `S001`) that appears in diagnostics and can be used in suppression directives. Diagnostics are classified as error, warning, or hint depending on severity.

### ShellCheck compatibility

Where possible, shuck rules align with ShellCheck rules. Shuck supports ShellCheck suppression syntax (`# shellcheck disable=SC2086`) and maps ShellCheck codes to their shuck equivalents, so existing suppression comments continue to work without changes. Both suppression syntaxes accept either code namespace, and native `# shuck: disable=...` follows ShellCheck's scope rules: before the first statement it is file-wide, otherwise it applies to the next command.

That said, shuck is not a port of ShellCheck. It is a clean-room reimplementation built on its own parser and analysis engine, so results will sometimes differ:

- Shuck's parser and analysis logic were written from scratch. Edge cases may be handled differently, and some diagnostics may fire in slightly different locations or contexts.
- In cases where ShellCheck's behavior appears incorrect or inconsistent with shell semantics, shuck intentionally chooses correctness over compatibility.
- `C001`/`SC2034` differs by default for resolved indirect expansions such as `${!name}`. Shuck treats the resolved target as live, while ShellCheck warns when that target is only referenced indirectly. Set `[lint.rule-options.c001].treat-indirect-expansion-targets-as-used = false` to opt into ShellCheck-compatible `C001` behavior.

Compatibility is continuously validated against a large corpus of shell scripts from popular open-source projects including [acme.sh](https://github.com/acmesh-official/acme.sh), [oh-my-zsh](https://github.com/ohmyzsh/ohmyzsh), [nvm](https://github.com/nvm-sh/nvm), [pyenv](https://github.com/pyenv/pyenv), [pi-hole](https://github.com/pi-hole/pi-hole), [bats-core](https://github.com/bats-core/bats-core), [powerlevel10k](https://github.com/romkatv/powerlevel10k), [dokku](https://github.com/dokku/dokku), [gentoo](https://github.com/gentoo/gentoo), and [many others](scripts/corpus-download.sh). The latest conformance report is published at [ewhauser.github.io/shuck/reports/corpus](https://ewhauser.github.io/shuck/reports/corpus/).

## Suppression

Suppress diagnostics with inline comments. Both native and ShellCheck-style directives are supported.

```sh
# Suppress a specific rule for the next command
# shuck:disable=C001
unused_var="ok"

# Suppress multiple rules
# shuck:disable=C001,S001
code_here

# Suppress for the entire file (place anywhere)
# shuck:disable-file=S001,S002

# ShellCheck-compatible syntax (also works)
# shellcheck disable=SC2034,SC2086

# Code aliases are interchangeable in either style
# shuck: disable=SC2086
# shellcheck disable=S001

# Before the first statement, disable becomes file-wide
# shuck: disable=S001
```

For embedded GitHub Actions scripts, put suppression comments inside the `run:` block as shell comments:

```yaml
- run: |
    # shellcheck disable=SC2086
    echo $FOO
```

YAML comments outside the `run:` scalar are not visible to the shell parser and do not suppress shell diagnostics.

## Configuration

Project settings live in `.shuck.toml` or `shuck.toml`.

Use the `[check]` section to control embedded-script extraction:

```toml
[check]
# Lint supported embedded shell scripts in non-shell files such as
# GitHub Actions workflows and composite actions.
# Default: true
embedded = true
```

## File discovery

When given a directory, shuck recursively discovers standalone shell scripts by:

1. **Extension**: `.sh`, `.bash`, `.zsh`, `.ksh`, `.dash`, `.mksh`, `.bats`
2. **Shebang**: files starting with `#!/bin/bash`, `#!/usr/bin/env sh`, etc.

Shuck also discovers embedded shell in supported non-shell files:

1. **GitHub Actions workflows**: `.github/workflows/*.yml` and `.github/workflows/*.yaml`
2. **Composite actions**: `action.yml` and `action.yaml`

For GitHub Actions files, shuck lints `run:` blocks independently, remaps diagnostics back to the host YAML file, and includes the step path (for example `jobs.test.steps[0].run`) in the message. Steps that target unsupported shells such as PowerShell or `cmd` are skipped.

The following directories are skipped by default: `.git`, `.hg`, `.svn`, `.jj`, `.bzr`, `.cache`, `node_modules`, `vendor`, `.shuck_cache`.

Gitignore and `.ignore` files are respected by default. Use `--no-respect-gitignore` to disable.

## Caching

Shuck caches lint results per file in a shared cache root outside the project tree by default. The default location follows the OS cache directory convention, which is typically `~/Library/Caches/shuck` on macOS and `$XDG_CACHE_HOME/shuck` or `~/.cache/shuck` on Linux.

Override the cache root with `--cache-dir` or `SHUCK_CACHE_DIR`.

Disable caching with `--no-cache` or remove a project's cache entries with `shuck clean [PATH]`.

## Acknowledgements

Shuck builds on ideas and inspiration from several excellent open-source projects. This section is a thank-you to those communities — it does not imply endorsement, affiliation, or any formal relationship between shuck and these projects.

- **[bashkit](https://github.com/everruns/bashkit)** — shuck-parser was originally forked from bashkit's bash lexer and parser; it has since evolved substantially to meet the needs of a linter (comment and trivia preservation, error recovery, multi-dialect parse views, extended AST coverage).
- **[Ruff](https://github.com/astral-sh/ruff)** — Linter architecture inspiration, particularly around caching, rule organization, and diagnostic output.
- **[ShellCheck](https://github.com/koalaman/shellcheck)** — An amazing project and the original source of inspiration for shuck. ShellCheck set the standard for shell script analysis.
- **[gbash](https://github.com/ewhauser/gbash)** — A lot of lessons learned from this earlier project carried forward into shuck.

## License

MIT
