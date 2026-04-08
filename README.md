# shuck

A shell script linter and formatter, written in Rust.

Shuck parses and analyzes shell scripts to catch common bugs, style issues, and portability problems. It includes a built-in formatter and a caching layer for fast incremental runs.

## Features

- **Linting** with rules across correctness and style categories
- **Formatting** with configurable indentation, operator placement, and layout options
- **Auto-fix** support for safe and unsafe fixes
- **Multi-dialect** support: bash, sh/POSIX, dash, ksh, mksh, zsh, bats
- **Automatic file discovery** via extensions and shebang detection
- **File-level caching** for fast re-runs on unchanged files
- **ShellCheck suppression compatibility** (`# shellcheck disable=SC2086`)

## Installation

### From source

```sh
cargo install shuck
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

# Read from stdin
echo 'echo $foo' | shuck check -

# Apply safe fixes
shuck check --fix .

# Apply all fixes, including unsafe ones
shuck check --unsafe-fixes .

# Skip the cache
shuck check --no-cache .

# Override the cache location
shuck --cache-dir .tmp/shuck-cache check .
```

### Format

```sh
# Format files in-place
shuck format .

# Check formatting without modifying files (exit 1 if changes needed)
shuck format --check .

# Show diffs instead of writing
shuck format --diff .

# Format with specific options
shuck format --indent-style space --indent-width 4 .

# Minify (compact form, strip comments)
shuck format --minify script.sh
```

### Clean caches

```sh
# Remove cache entries for the current project
shuck clean .
```

## Output

Diagnostics are printed one per line:

```
path:line:col: severity[CODE] message
```

```
deploy.sh:14:1: warning[C001] variable 'tmp' is assigned but never used
deploy.sh:31:10: error[C006] undefined variable 'DEPLY_ENV'
deploy.sh:45:3: style[S005] legacy backtick command substitution
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0`  | No issues found |
| `1`  | Lint violations or formatting changes detected |
| `2`  | Runtime error (bad arguments, I/O failure) |

## Rules

Rules are organized by category and severity.

### Correctness (C)

Bugs and errors. Enabled by default.

| Code | Name | Severity |
|------|------|----------|
| C001 | UnusedAssignment | Warning |
| C002 | DynamicSourcePath | Warning |
| C005 | SingleQuotedLiteral | Warning |
| C006 | UndefinedVariable | Error |
| C007 | FindOutputToXargs | Warning |
| C008 | TrapStringExpansion | Warning |
| C009 | QuotedBashRegex | Warning |
| C010 | ChainedTestBranches | Warning |
| C011 | LineOrientedInput | Warning |
| C013 | FindOutputLoop | Warning |
| C014 | LocalTopLevel | Error |
| C015 | SudoRedirectionOrder | Warning |
| C017 | ConstantComparisonTest | Warning |
| C018 | LoopControlOutsideLoop | Error |
| C019 | LiteralUnaryStringTest | Warning |
| C020 | TruthyLiteralTest | Warning |
| C021 | ConstantCaseSubject | Warning |
| C022 | EmptyTest | Error |
| C025 | PositionalTenBraces | Warning |
| C046 | PipeToKill | Warning |
| C047 | InvalidExitStatus | Error |
| C048 | CasePatternVar | Warning |
| C050 | ArithmeticRedirectionTarget | Warning |
| C055 | PatternWithVariable | Warning |
| C057 | SubstWithRedirect | Warning |
| C058 | SubstWithRedirectErr | Warning |
| C063 | OverwrittenFunction | Warning |
| C124 | UnreachableAfterExit | Warning |

### Style (S)

Code style improvements. Must be explicitly selected.

| Code | Name |
|------|------|
| S001 | UnquotedExpansion |
| S002 | ReadWithoutRaw |
| S003 | LoopFromCommandOutput |
| S004 | UnquotedCommandSubstitution |
| S005 | LegacyBackticks |
| S006 | LegacyArithmeticExpansion |
| S007 | PrintfFormatVariable |
| S008 | UnquotedArrayExpansion |
| S009 | EchoedCommandSubstitution |
| S010 | ExportCommandSubstitution |

## Suppression

Suppress diagnostics with inline comments. Both native and ShellCheck-style directives are supported.

```sh
# Suppress a specific rule for the next line
# shuck:disable=C001
unused_var="ok"

# Suppress multiple rules
# shuck:disable=C001,S001
code_here

# Re-enable a rule
# shuck:enable=C001

# Suppress for the entire file (place anywhere)
# shuck:disable-file=S001,S002

# ShellCheck-compatible syntax (also works)
# shellcheck disable=SC2034,SC2086
```

## Configuration

Create a `.shuck.toml` or `shuck.toml` at your project root:

```toml
[format]
dialect = "bash"           # auto | bash | posix | mksh | zsh
indent-style = "space"     # tab | space
indent-width = 4           # 1-255, used when indent-style = "space"
binary-next-line = false   # place binary operators on the next line
switch-case-indent = false # indent case branch bodies
space-redirects = false    # spaces around redirect operators
keep-padding = false       # preserve original source padding
function-next-line = false # opening brace on its own line
never-split = false        # compact single-line layouts
```

## File discovery

When given a directory, shuck recursively discovers shell scripts by:

1. **Extension**: `.sh`, `.bash`, `.zsh`, `.ksh`, `.dash`, `.mksh`, `.bats`
2. **Shebang**: files starting with `#!/bin/bash`, `#!/usr/bin/env sh`, etc.

The following directories are skipped by default: `.git`, `.hg`, `.svn`, `.jj`, `.bzr`, `.cache`, `node_modules`, `vendor`, `.shuck_cache`.

Gitignore and `.ignore` files are respected by default. Use `--no-respect-gitignore` to disable.

## Caching

Shuck caches lint and format results per file in a shared cache root outside the project tree by default. The default location follows the OS cache directory convention, which is typically `~/Library/Caches/shuck` on macOS and `$XDG_CACHE_HOME/shuck` or `~/.cache/shuck` on Linux.

Override the cache root with `--cache-dir` or `SHUCK_CACHE_DIR`.

Disable caching with `--no-cache` or remove the current project's cache entries with `shuck clean`.

## License

MIT
