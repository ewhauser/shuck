# 013: Embedded Shell Script Extraction

## Status

Proposed

## Summary

Adds support for linting shell scripts embedded inside non-shell files, starting with GitHub Actions workflow files (`.github/workflows/*.yml`, `action.yml`). The design introduces an `Extractor` trait that takes a file's source and yields a list of embedded shell snippets with offset, dialect, and metadata. The check pipeline gains an extraction phase between file discovery and parsing: discovered YAML files are probed for GHA structure, `run:` blocks are extracted with shell dialect resolved from the `shell:` / `defaults.run.shell` hierarchy, `${{ }}` expressions are replaced with synthetic placeholder variables, and each snippet is linted independently with line numbers mapped back to the host YAML file.

Other embedded formats (GitLab CI, Dockerfiles, Justfiles) are explicitly out of scope but the trait boundary makes them addable without touching the core pipeline.

## Motivation

GitHub Actions workflows are one of the most common places developers write shell. Every `run:` block is a bash (or sh) script, yet today `shuck check .` ignores them entirely because discovery only matches shell file extensions and shebangs. Users must manually extract snippets to lint them.

ShellCheck has the same gap — it only operates on standalone shell files. Adding GHA support makes shuck strictly more useful than a ShellCheck replacement for CI-heavy projects, which is the majority of open-source repositories on GitHub.

The extraction abstraction is worth the small upfront cost because the pattern repeats across CI systems (GitLab CI `script:` blocks, Dockerfiles `RUN` instructions, Justfile recipes). Building the trait now means the second format is a single-file addition rather than a pipeline refactor.

## Design

### Extractor Trait

A new `shuck-extract` crate provides the core abstraction:

```rust
/// A shell snippet extracted from a host file.
pub struct EmbeddedScript {
    /// The shell source code, after placeholder substitution.
    pub source: String,
    /// Byte offset of the snippet's first character within the host file.
    pub host_offset: usize,
    /// 1-based line number of the snippet's first character in the host file.
    pub host_start_line: usize,
    /// 1-based column of the snippet's first character in the host file.
    pub host_start_column: usize,
    /// The shell dialect for this snippet (Bash, Sh, etc.).
    pub dialect: ExtractedDialect,
    /// Human-readable label for diagnostic context, e.g. "jobs.test.steps[0].run".
    pub label: String,
}

pub enum ExtractedDialect {
    Bash,
    Sh,
    /// The snippet targets a shell we don't support (e.g. PowerShell, cmd).
    /// The pipeline should skip these.
    Unsupported,
}

/// Trait for extracting embedded shell scripts from a host file.
pub trait Extractor {
    /// Returns true if this extractor handles the given file path.
    /// Called during discovery to decide whether to attempt extraction.
    fn matches(&self, path: &Path) -> bool;

    /// Probe the file source to confirm it is the expected format.
    /// For example, a GHA extractor checks for top-level `on:` or `jobs:` keys.
    fn probe(&self, source: &str) -> bool;

    /// Extract all embedded shell snippets from the file.
    fn extract(&self, source: &str) -> Result<Vec<EmbeddedScript>>;
}
```

### GitHub Actions Extractor

#### File Matching

The GHA extractor matches files when:

1. The path ends in `.yml` or `.yaml`, **and**
2. The path is under `.github/workflows/`, **or** the filename is `action.yml` / `action.yaml`

This is checked in `matches()` using path inspection only (no I/O).

#### Structural Probe

After reading the file source, `probe()` verifies it looks like a GHA file by checking for the presence of top-level YAML keys that indicate a workflow (`on:` and `jobs:`) or a composite action (`runs:` with `using: composite`). This avoids false positives on arbitrary YAML files.

#### Shell Dialect Resolution

GHA has a three-level shell inheritance hierarchy:

1. **Workflow-level default**: `defaults.run.shell`
2. **Job-level default**: `jobs.<id>.defaults.run.shell`
3. **Step-level override**: `jobs.<id>.steps[*].shell`

The most specific level wins. When no `shell:` is specified at any level, the effective shell depends on the runner OS:

| `runs-on` pattern | Default shell |
|---|---|
| Contains `ubuntu`, `macos`, `linux`, or unrecognized | `bash` |
| Contains `windows` | `pwsh` |

For `self-hosted` runners without an OS label, we assume `bash` — this matches the common case and users can override with `shell:`.

Supported shell values and their dialect mapping:

| `shell:` value | `ExtractedDialect` |
|---|---|
| `bash` | `Bash` |
| `sh` | `Sh` |
| `bash -e {0}`, `bash --noprofile --norc -eo pipefail {0}` | `Bash` |
| `sh -e {0}` | `Sh` |
| `pwsh`, `powershell`, `cmd`, `python` | `Unsupported` |
| Absent (Unix runner) | `Bash` |
| Absent (Windows runner) | `Unsupported` |

The extractor strips the custom template suffix (e.g., `-e {0}`) by taking only the first whitespace-delimited token and matching on that.

#### `${{ }}` Placeholder Substitution

GitHub Actions interpolates `${{ expression }}` syntax into `run:` blocks before the shell sees them. These are not valid shell syntax and would cause parse errors.

The extractor replaces each `${{ ... }}` occurrence with a synthetic environment variable reference: `$_SHUCK_GHA_1`, `$_SHUCK_GHA_2`, etc. (monotonically increasing per snippet). This approach:

- Produces valid shell syntax — no parse errors from template expressions
- Preserves expansion semantics — quoting rules (S001/SC2086) still fire on `echo $_SHUCK_GHA_1` just as they would on `echo ${{ github.ref }}`
- Is deterministic — same input always produces same placeholders

Nested expressions like `${{ format('refs/heads/{0}', matrix.branch) }}` are handled by matching the outermost `${{` ... `}}` pair. The regex pattern is `\$\{\{.*?\}\}` applied non-greedily.

#### YAML Block Scalar Handling

The extractor must correctly handle YAML block scalars since `run:` values are almost always multi-line:

| Style | Syntax | Behavior |
|---|---|---|
| Literal block | `run: \|` | Preserves newlines as-is |
| Literal block (strip) | `run: \|-` | Same, but strips trailing newline |
| Folded block | `run: >` | Replaces newlines with spaces (except double-newlines) |
| Flow scalar | `run: "echo hi"` | Inline string, escape sequences processed |
| Plain scalar | `run: echo hi` | Single-line only |

All styles are handled by the YAML parser library, which resolves the scalar value before we see it. The extractor operates on the resolved string value, not the raw YAML text. The key concern is computing the correct `host_start_line` and `host_start_column` — for block scalars, the content starts on the line after the `run: |` indicator, indented relative to the parent mapping.

#### Composite Actions

For `action.yml` / `action.yaml` files with `runs.using: composite`, the extractor walks `runs.steps[*]` using the same logic as workflow steps. The only difference is the path into the YAML structure; the shell resolution, placeholder substitution, and extraction are identical (composite action steps also support `shell:` and default to the runner's default shell).

#### YAML Parsing Library

The extractor uses a YAML library that provides byte-offset/line information for scalar values. Candidates:

- **`serde_yaml`** (or its successor `unsafe-libyaml`) — widely used, but extracting span information for individual scalar values requires custom deserialization
- **`yaml-rust2`** — provides `Marker` with line/column for each node, making offset computation straightforward
- **`marked-yaml`** — specifically designed for span tracking, wraps each node with its source span

The implementation should choose whichever provides the most reliable span information with the least complexity. The critical requirement is: given a `run:` scalar value node, we must know the byte offset and line/column of the first character of the resolved string content within the host YAML file.

### Pipeline Integration

#### Crate Responsibilities

`shuck-extract` owns all extraction-related logic, including path classification:

| Responsibility | Owner |
|---|---|
| "Is this path extractable?" (`matches`) | `shuck-extract` (via `Extractor` trait) |
| "Is this source actually GHA?" (`probe`) | `shuck-extract` (via `Extractor` trait) |
| YAML parsing and span tracking | `shuck-extract` |
| Shell dialect resolution | `shuck-extract` |
| `${{ }}` placeholder substitution | `shuck-extract` |
| Offset computation for remapping | `shuck-extract` |

`shuck-extract` exposes a registry of extractors and a convenience function:

```rust
/// Returns true if any registered extractor handles the given path.
pub fn is_extractable(path: &Path) -> bool {
    EXTRACTORS.iter().any(|e| e.matches(path))
}

/// Run all matching extractors against the source. Probes first, then extracts.
pub fn extract_all(path: &Path, source: &str) -> Result<Vec<EmbeddedScript>> {
    // ...
}
```

Discovery calls `shuck_extract::is_extractable(path)` — a pure path check with no I/O. This means adding a new format (e.g., GitLab CI) requires only a new `impl Extractor` in `shuck-extract`; discovery and the rest of the pipeline don't change.

#### Discovery Changes

`discover.rs` calls `shuck_extract::is_extractable(path)` for files that don't match shell extensions or shebangs. When it returns true, the file is included in discovery results with an `Embedded` marker.

The `DiscoveredFile` struct gains a field:

```rust
pub(crate) enum FileKind {
    /// A standalone shell script (current behavior).
    Shell,
    /// A file that may contain embedded shell scripts.
    Embedded,
}
```

Discovery does **not** read file contents — the `matches()` check is path-only. The probe (which reads source) happens later in `analyze_file`.

#### Analysis Changes

`analyze_file` is extended with an extraction branch. When `file.kind == FileKind::Embedded`:

1. Read the file source
2. Call `shuck_extract::extract_all(path, source)` — this probes and extracts in one call
3. For each `EmbeddedScript` with a supported dialect:
   a. Parse the extracted `source` as a standalone shell script
   b. Run the linter with the `FileContext` tagged as embedded (see below)
   c. Remap diagnostic line/column numbers by adding `host_start_line` and `host_start_column` offsets
4. Collect all diagnostics under the host file's path

Each `run:` block is parsed and linted independently — there is no cross-step analysis.

#### Line/Column Remapping

Diagnostics from the embedded parser report positions relative to the extracted snippet (starting at line 1, column 1). Before output, these are remapped:

```
output_line   = diagnostic_line + host_start_line - 1
output_column = if diagnostic_line == 1 {
    diagnostic_column + host_start_column - 1
} else {
    diagnostic_column
}
```

This gives users diagnostics pointing to the correct location in the YAML file.

#### Diagnostic Labels

Each `EmbeddedScript` carries a `label` (e.g., `jobs.test.steps[0].run`) that provides context about where in the YAML structure the snippet lives. The diagnostic output includes this label so users can quickly identify which step triggered the warning:

```
.github/workflows/ci.yml:12:16: warning[S001] jobs.test.steps[0].run: variable expansion "$FOO" should be double-quoted
```

The label appears between the rule code and the message.

#### FileContext for Embedded Snippets

A new `FileContextTag::EmbeddedScript` is added. The `classify_file_context` function sets this tag when the snippet originates from an extractor. Rules that are inherently file-level (shebang checks, file-level directive checks) check for this tag and skip silently when present.

#### Caching

Embedded files are cached at the host file level: the cache key is the YAML file's content hash, and the cached value contains diagnostics for all extracted snippets. If the YAML file changes, all snippets are re-extracted and re-linted. This is simpler than per-snippet caching and matches the existing per-file cache model.

### Configuration

A new `[check]` section in `.shuck.toml`:

```toml
[check]
# Enable/disable extraction of embedded shell scripts from non-shell files.
# When true, YAML files matching known CI formats are automatically discovered
# and their shell snippets are linted.
# Default: true
embedded = true
```

When `embedded = false`, the extractor pipeline is skipped entirely — YAML files are not discovered, probed, or extracted. This is a single boolean for now; per-format toggles can be added later if needed.

### Suppression in Embedded Scripts

Inline shell comments within a `run:` block work as suppression directives, just as they do in standalone scripts:

```yaml
- run: |
    # shellcheck disable=SC2086
    echo $FOO
```

YAML comments (lines starting with `#` outside the `run:` scalar) are not visible to the shell parser and cannot serve as suppressions. This is the expected behavior — YAML comments are metadata about the YAML structure, not about the shell code.

### Error Handling

- **YAML parse failure**: If the YAML file cannot be parsed, report a single diagnostic at line 1 indicating the parse error. Do not attempt extraction.
- **Extraction failure for a single snippet**: Report the error as a diagnostic at the `run:` key's location and continue extracting remaining snippets.
- **Shell parse failure**: Handled identically to standalone files — report the parse error at the remapped location.

### Future Formats

The `Extractor` trait is designed to support additional formats without pipeline changes. Likely candidates in priority order:

| Format | File pattern | Shell location | Complexity |
|---|---|---|---|
| GitLab CI | `.gitlab-ci.yml` | `script:`, `before_script:`, `after_script:` (list of strings) | Low |
| Dockerfiles | `Dockerfile*` | `RUN` instructions (shell form) | Medium |
| Justfiles | `justfile`, `Justfile`, `*.just` | Recipe bodies | Medium |
| Makefiles | `Makefile`, `*.mk` | Recipe lines | High |

Each would be a new `impl Extractor` in the `shuck-extract` crate. The pipeline does not need to change — just the registry of extractors.

## Alternatives Considered

### Parse YAML with regex instead of a YAML library

Simpler, no dependency on a YAML parser. Rejected because YAML block scalar semantics (indentation stripping, chomping indicators, folded vs. literal) are non-trivial to get right with regex. A proper YAML parser handles these correctly and provides span information.

### Inline extraction into shuck-linter instead of a separate crate

Would avoid the new crate, but the extraction logic (YAML parsing, shell dialect resolution, placeholder substitution) has no overlap with the linter's AST-based rule checking. A separate crate keeps the linter focused on shell analysis and makes it possible to test extraction independently.

### Skip `${{ }}` blocks entirely instead of replacing with placeholders

Simpler, but loses the ability to lint quoting around GHA expressions. `echo ${{ secrets.TOKEN }}` is a real quoting bug that placeholder substitution catches (S001 would fire on the unquoted expansion). Skipping would produce false negatives on one of the most common GHA mistakes.

### Correlate variables across steps

Would catch cases like a variable exported in step 1 and used in step 2. Rejected for now because GHA steps run in separate shell invocations (unless `shell: bash` with explicit sourcing), making cross-step dataflow analysis unreliable. Each step is a logically separate script.

### Default `embedded = false` (opt-in)

Safer rollout, but reduces the value of the feature — most users wouldn't know to enable it. Since the probe step prevents false positives on non-GHA YAML files, and the extraction is read-only, defaulting to `true` is low-risk and matches the expected behavior of `shuck check .` scanning everything relevant.

## Verification

### Unit Tests

- **Extractor trait**: Test `matches()`, `probe()`, and `extract()` independently for the GHA extractor.
- **Shell resolution**: Verify the three-level hierarchy (workflow → job → step) resolves correctly with all combinations of `shell:` and `runs-on:`.
- **Placeholder substitution**: Confirm `${{ expr }}` is replaced with `$_SHUCK_GHA_N`, including nested braces and multi-line expressions.
- **Block scalar handling**: Test literal (`|`), folded (`>`), strip (`|-`), and flow scalar extraction with correct offset computation.
- **Line remapping**: Verify diagnostic positions map back to the correct YAML line/column.
- **Unsupported shell skip**: Confirm PowerShell steps on Windows runners produce no diagnostics.
- **Composite actions**: Test extraction from `action.yml` with `runs.using: composite`.

### Integration Tests

- **End-to-end check**: Run `shuck check` on a fixture `.github/workflows/test.yml` containing known lint violations. Verify diagnostics appear with correct file paths and line numbers.
- **Mixed project**: Run `shuck check .` on a project with both `.sh` files and `.github/workflows/*.yml`. Verify both are discovered and linted.
- **Config opt-out**: Set `embedded = false` in `.shuck.toml` and confirm YAML files are skipped.
- **Suppression in embedded**: Verify `# shellcheck disable=SC2086` inside a `run: |` block suppresses the expected diagnostic.

### Manual Verification

```bash
# Create a test workflow
cat > /tmp/test-workflow.yml << 'EOF'
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo $FOO
      - run: |
          if [ -z $BAR ]; then
            echo $BAZ
          fi
  windows:
    runs-on: windows-latest
    steps:
      - run: echo $FOO          # PowerShell — should be skipped
      - run: echo $FOO
        shell: bash              # Bash on Windows — should be linted
EOF

# Run check — expect diagnostics on lines 7, 9, 10, 15 but NOT line 13
shuck check /tmp/test-workflow.yml

# Verify no YAML files are checked when disabled
echo -e '[check]\nembedded = false' > .shuck.toml
shuck check .  # should skip .github/workflows/
```
