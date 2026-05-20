# 009: Formatter

## Status

Partially Implemented

## Summary

A shell script formatter built on Shuck's parser and AST. The live implementation is centered in `shuck-formatter`, which owns shell-specific formatting rules, comment handling, option resolution, and the optional simplify pass. CLI and editor callers invoke the same `format_source()` / `format_file_ast()` entry points.

The formatter supports Bash, POSIX, and mksh dialects with auto-inference from shebangs and file extensions. It exposes formatting layout options through CLI flags and `[format]` config sections, while `--dialect` remains a CLI-only override. An optional simplify pass applies safe AST rewrites before formatting, and a minify mode produces compact output without comments.

## Motivation

Shell scripts accumulate inconsistent formatting — mixed indentation, inconsistent spacing around redirects, varying brace placement, redundant quoting and subshell nesting. Unlike languages with established formatters built into their toolchains, shell has limited options. Existing formatters do not integrate with linting or share a parser, requiring users to maintain separate tool configurations and accept potential parser disagreements.

Shuck already parses shell scripts into a full AST. Building a formatter on top of that AST means:

- **Single parser, multiple tools.** Formatting and linting share the same parse result, eliminating parser disagreements and enabling combined workflows (`shuck check` + `shuck format` in one pass).
- **AST-level rewrites.** Because the formatter operates on a typed AST rather than token streams, it can offer safe simplification rewrites (removing redundant parentheses, tightening quotes, flattening nested subshells) that token-level formatters cannot express.
- **Dialect awareness.** The parser already understands Bash, POSIX, and mksh grammars. The formatter inherits this, rejecting invalid constructs (e.g., `[[ ]]` in POSIX mode) at parse time rather than silently reformatting them.

## Design

### Architecture Overview

The formatter is organized around one shell-specific formatting crate:

```
  CLI (shuck format / shuck format-stdin)
  LSP (textDocument/formatting / textDocument/rangeFormatting)
    |
    v
  +----------------------------------------------+
  | shuck-formatter                               |
  |  format_source() / format_file_ast()          |
  |                                               |
  |  +-----------------------------------------+  |
  |  | Simplify Pass (simplify.rs)             |  |
  |  |  AST → AST rewrites (optional)         |  |
  |  +-----------------------------------------+  |
  |           |                                   |
  |           v                                   |
  |  +-----------------------------------------+  |
  |  | Streaming Formatter                     |  |
  |  |  streaming.rs, command.rs, word.rs      |  |
  |  |  AST + source facts → formatted text    |  |
  |  +-----------------------------------------+  |
  |           |                                   |
  |           v                                   |
  |  +-----------------------------------------+  |
  |  | Comment Attachment (comments.rs)        |  |
  |  |  Line-based leading/trailing/dangling   |  |
  |  +-----------------------------------------+  |
  +----------------------------------------------+
  Formatted source text
```

### Shell Formatter (`shuck-formatter`)

The shell-specific layer that formats parsed AST nodes using source-aware layout facts.

#### Public API

Two entry points, both pure functions with no I/O:

```rust
/// Parse source and format. The primary entry point.
pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource>;

/// Format an already-parsed AST. Used when the caller has a parse
/// result from another pipeline (e.g., lint-then-format).
pub fn format_file_ast(
    source: &str,
    file: File,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource>;
```

Both return `FormattedSource`:

```rust
pub enum FormattedSource {
    Unchanged,           // Output matches input byte-for-byte
    Formatted(String),   // Reformatted source
}
```

Returning `Unchanged` when nothing changed avoids unnecessary cache invalidation and file writes.

#### Formatting Options

```rust
pub struct ShellFormatOptions {
    dialect: ShellDialect,          // Auto, Bash, Posix, Mksh
    indent_style: IndentStyle,      // Tab (default) or Space
    indent_width: u8,               // Default 8, minimum 1
    binary_next_line: bool,         // Place binary operators (|, &&, ||) on the next line
    switch_case_indent: bool,       // Indent case patterns and bodies
    space_redirects: bool,          // Insert spaces around redirect operators
    keep_padding: bool,             // Preserve alignment padding in source
    function_next_line: bool,       // Opening brace on next line for functions
    never_split: bool,              // Prefer single-line compact layouts
    simplify: bool,                 // Run AST simplification rewrites
    minify: bool,                   // Compact output, drop comments
}
```

Options are resolved before formatting via `ShellFormatOptions::resolve()`, which produces a `ResolvedShellFormatOptions` with the dialect concretized (Auto → Bash/Posix/Mksh based on shebang and extension) and line endings detected from the input.

| Option | Default | Effect |
|---|---|---|
| `dialect` | `Auto` | Selects parser grammar. Auto infers from shebang (`#!/bin/bash` → Bash, `#!/bin/sh` → POSIX) then file extension (`.bash`, `.sh`, `.ksh`, `.mksh`). Falls back to Bash. |
| `indent_style` | `Tab` | Tab or space indentation for nested blocks. |
| `indent_width` | `8` | Character width of each indentation level when using spaces. |
| `binary_next_line` | `false` | When a pipeline or list operator breaks across lines, place the operator at the start of the continuation line rather than the end of the preceding line. |
| `switch_case_indent` | `false` | Indent `case` patterns by one level and bodies by two levels relative to `case`/`esac`. |
| `space_redirects` | `false` | Insert spaces around redirect operators (`> out` instead of `>out`). |
| `keep_padding` | `false` | Preserve intra-line alignment padding (tabs or multi-space runs) instead of normalizing to single spaces. |
| `function_next_line` | `false` | Place the opening `{` of function definitions on its own line. |
| `never_split` | `false` | Prefer single-line layouts for compound commands that fit (`if true; then echo hi; fi`). |
| `simplify` | `false` | Run AST simplification rewrites before formatting. |
| `minify` | `false` | Produce compact output: single-line layouts, no comments, implies simplify. |

#### Streaming Formatter

The formatter is source-aware rather than a generic document printer. The major modules:

- **`streaming.rs`** — Owns the main output buffer, indentation state, sequence layout, comment placement, and unchanged checks.
- **`command.rs`** — The largest module. Handles simple commands, pipelines, command lists (`&&`/`||`/`;`/`&`), and all compound commands (`if`/`for`/`while`/`until`/`case`/`select`/`subshell`/`brace-group`/`arithmetic`/`conditional`/`coproc`/`time`). Also handles function definitions and declaration builtins (`declare`, `export`, `local`, `readonly`, `typeset`).
- **`word.rs`** — Word and word-part formatting. Reconstructs the textual form of words from their AST parts, handling quoting, expansions, parameter operations, and escape sequences.
- **`facts.rs`** — Computes source-layout facts used during formatting, including spacing-sensitive regions and comment attachment inputs.
- **`scan.rs`** — Contains source scanning helpers for shell comments, indentation, heredocs, and keyword/operator boundaries.

When the formatter cannot yet produce correct structured output for a construct, it emits the original source slice unchanged. This is a correctness safety net: the formatter never silently corrupts code. The roadmap tracks reducing verbatim fallback usage over time.

#### Formatting Pipeline

```
format_source(source, path, options)
  1. Resolve options (dialect inference, line ending detection)
  2. Parse source → File
  3. If simplify or minify: rewrite the owned AST in place
  4. Build source maps, comment attachments, and formatter facts
  5. Format File through the streaming formatter
  6. Produce raw output string
  7. Ensure single trailing newline
  8. Compare output to input → Unchanged or Formatted
```

Step 8 is important: if the formatter's output is byte-identical to the input, it returns `Unchanged` rather than a redundant copy. This makes `--check` mode (exit non-zero if changes needed) a zero-allocation path for already-formatted files.

#### Comment Handling

Comments are not part of the AST — the parser emits them as a separate `Vec<Comment>` alongside the script. The formatter must reattach them to the correct positions in the output.

The current implementation computes comment placement from `SourceMap` and sequence-level attachment facts:

- **`SourceMap`** — Pre-computes line start offsets, first-non-whitespace positions per line, and fast lookup indexes for `#`, tab, and double-space characters. Provides O(log n) offset-to-line mapping and O(1) queries for inline vs. own-line comments and alignment padding detection.
- **`SourceComment`** — A comment with its text, span, line number, and inline flag (whether other content precedes it on the same line).
- **`SequenceCommentAttachment`** — For a sequence of N commands, partitions comments into leading (before each command), trailing (after each command on the same line), and dangling (comments in otherwise-empty bodies). Also tracks ambiguity when the line-based heuristic cannot confidently assign a comment.

The streaming formatter consumes comments while rendering command sequences and writes them into the output buffer at the appropriate positions. In `--minify` mode, comments are dropped entirely.

**Limitation:** The current approach attaches comments by line proximity rather than by anchoring them to AST nodes. This works well for most cases but can misplace comments in constructs with continuation lines, nested substitutions, or heredocs. The roadmap tracks replacing this with true AST-anchored comment attachment.

### Simplify Pass

The simplify pass applies safe, idempotent AST-to-AST rewrites before the formatting stage. It operates on a cloned AST so the original parse result is unmodified.

#### Architecture

```rust
pub struct SimplifyRewrite {
    pub name: &'static str,
    pub apply: fn(&mut Script, &str) -> usize, // returns change count
}

pub fn simplify_script(script: &mut Script, source: &str) -> SimplifyReport;
```

Each rewrite is a standalone function that walks the AST, applies transformations, and returns the number of changes made. Rewrites run sequentially in a fixed order. The `SimplifyReport` records which rewrites fired and how many changes each made.

#### Rewrites

| Rewrite | Description |
|---|---|
| `paren-cleanup` | Strips unnecessary outer parentheses from arithmetic expressions and subshell-wrapped single commands. |
| `arithmetic-vars` | Simplifies variable references in arithmetic contexts (e.g., `$x` → `x` where the `$` prefix is redundant). |
| `conditionals` | Optimizes conditional expressions: removes double negation, simplifies tautological comparisons, and reduces redundant test structures. |
| `nested-subshells` | Flattens nested subshells `(( cmd ))` → `( cmd )` when the outer subshell has no redirects and contains only a single inner subshell. Also applies to nested process substitutions. |
| `quote-tightening` | Tightens quote scopes — removes unnecessary quoting around literals that don't require protection, and simplifies doubly-quoted expansions. |

Each rewrite is:
- **Independent** — rewrites do not depend on each other's results within a single pass.
- **Idempotent** — running the same rewrite twice produces no additional changes.
- **Safe** — rewrites preserve observable behavior. They do not change the semantics of the script.
- **Testable** — each rewrite has its own unit tests independent of the formatter.

### CLI Integration

The `shuck format` subcommand wires the formatter into the CLI:

```
shuck format [OPTIONS] [PATHS...]
shuck format-stdin [OPTIONS] [--stdin-filename <NAME>]
```

**Modes:**
- Default: format files in-place, writing changes back to disk.
- `--check`: exit non-zero if any file would change, without writing.
- `--diff`: print unified diffs to stdout instead of writing.

**Option resolution precedence** (highest to lowest):
1. CLI flags (`--indent-style space`)
2. `[format]` section in the nearest `.shuck.toml` / `shuck.toml`
3. Built-in defaults

Dialect stays on auto-inference unless the user passes `--dialect` on the CLI.

**Caching:** Formatted results are cached via `shuck-cache` (SHA-256 keyed by file content, mtime, permissions, and formatting options). The `--no-cache` flag bypasses caching. Formatting options are part of the cache key, so changing options invalidates cached results.

**stdin:** `shuck format-stdin` reads from stdin and writes formatted output to stdout. The `--stdin-filename` flag provides a filename hint for dialect inference (e.g., `--stdin-filename script.sh` infers POSIX).

### Data Flow

End-to-end for `shuck format`:

```
CLI parses args + config → ShellFormatOptions
  |
  v
Discover files (same walker as `shuck check`)
  |
  v
For each file:
  Cache lookup (options included in key)
    |-- hit → skip
    |-- miss:
        |
        v
      format_source(source, path, options)
        → Parse (dialect-aware)
        → Optional simplify (owned AST rewrite)
        → Build formatter facts and comment attachments
        → Streaming format → String
        → Ensure trailing newline
        → Compare to input
        |
        v
      Unchanged → cache as clean
      Formatted → write / diff / check, cache result
```

## Alternatives Considered

### Alternative A: Token-Stream Formatter

Format by manipulating the token stream directly (insert/remove whitespace tokens, adjust indentation) without using the AST as the main formatting structure.

Rejected because token-stream formatting cannot express semantic layout choices, structured fallbacks, or simplify rewrites cleanly. The formatter needs AST ownership for dialect-sensitive constructs and safe rewrites.

### Alternative B: Separate Generic Printer Crate

Keep a language-agnostic document/printer crate and have `shuck-formatter` lower shell AST nodes into that IR.

Initially selected, then removed. The shell formatter's hot path evolved toward source-aware streaming with shell-specific facts, and the generic crate became mostly dead abstraction around a formatter that no longer used it.

### Alternative C: Unstructured Direct String Building

Have isolated node helpers append text to a string buffer without central formatter state or source facts.

Rejected because it spreads indentation, comments, and fallback decisions across many helpers. The current streaming formatter still writes directly, but through one stateful engine with shared source maps, formatter facts, indentation helpers, and scratch buffers.

## Verification

- **Idempotence**: formatting an already-formatted file returns `Unchanged`. The test suite verifies this for all fixture files and benchmark corpus scripts.
- **Source/AST path equivalence**: `format_source()` and `format_file_ast()` produce identical output for the same input. The test `format_file_ast_matches_format_source_for_benchmark_corpus` verifies this.
- **Option effects**: each formatting option has dedicated tests showing its effect (e.g., `switch_case_indent_indents_patterns_and_bodies`, `space_redirects_insert_spaces_between_operator_and_target`).
- **Dialect enforcement**: formatting with an explicit POSIX dialect rejects Bash-only constructs at parse time (`posix_dialect_propagates_parse_errors`).
- **Comment preservation**: inline and own-line comments survive formatting and appear at the correct positions. Minify mode drops them (`minify_drops_comments`).
- **Simplify safety**: each rewrite is independently tested and verified idempotent.
- **Oracle suite**: opt-in tests (`SHUCK_RUN_SHFMT_ORACLE=1`) compare formatter output against a reference formatter across targeted fixtures and the benchmark corpus, producing unified diffs on mismatch.
- **Benchmarks**: Criterion benchmarks in `shuck-benchmark` measure both `format_source()` (full pipeline) and `format_file_ast()` (already-parsed) throughput, plus comment indexing overhead.

```bash
# Run formatter tests
cargo test -p shuck-formatter

# Run CLI integration tests
cargo test -p shuck -- format

# Run oracle comparison (requires nix dev shell)
SHUCK_RUN_SHFMT_ORACLE=1 cargo test -p shuck-formatter -- oracle

# Run formatter benchmarks
cargo bench -p shuck-benchmark -- formatter
```
