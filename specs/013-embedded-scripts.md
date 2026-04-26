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
    /// Which embedded format this snippet was extracted from.
    pub format: EmbeddedFormat,
    /// Mapping from placeholder variables back to the original template expressions.
    /// Empty when the format has no template expression syntax.
    pub placeholders: Vec<PlaceholderMapping>,
    /// Shell flags implied by the host environment (e.g., GHA's default bash template
    /// sets errexit and pipefail). Rules use this to know what error-handling behavior
    /// is active even when the script doesn't set it explicitly.
    pub implicit_flags: ImplicitShellFlags,
}

pub enum ExtractedDialect {
    Bash,
    Sh,
    /// The snippet targets a shell we don't support (e.g. PowerShell, cmd).
    /// The pipeline should skip these.
    Unsupported,
}

/// Identifies which embedded format produced the snippet.
/// Rules use this to enable format-specific checks.
pub enum EmbeddedFormat {
    GitHubActions,
    // Future: GitLabCi, Dockerfile, Justfile, ...
}

/// Records the relationship between a synthetic placeholder variable and
/// the original template expression it replaced.
pub struct PlaceholderMapping {
    /// The placeholder variable name, e.g. "_SHUCK_GHA_1".
    pub name: String,
    /// The full original expression including delimiters, e.g. "${{ github.ref }}".
    pub original: String,
    /// The inner expression text, e.g. "github.ref".
    pub expression: String,
    /// Taint classification for security rules.
    pub taint: ExpressionTaint,
    /// Byte range of the `$_SHUCK_GHA_N` reference in the substituted source.
    pub substituted_span: Range<usize>,
    /// Byte range of the original `${{ ... }}` in the host file.
    pub host_span: Range<usize>,
}

/// Classifies how trustworthy a template expression's value is at runtime.
pub enum ExpressionTaint {
    /// Attacker can inject arbitrary content: PR titles, issue bodies, branch
    /// names, commit messages, review comments, discussion bodies, etc.
    /// Using these directly in shell code is a script injection vulnerability.
    UserControlled,
    /// Secret value — not attacker-controlled, but should not appear inline
    /// in command arguments (visible via /proc or `ps`) or be echoed/logged.
    Secret,
    /// Repository/workflow scoped and not directly attacker-controlled:
    /// github.repository, github.sha, github.run_id, runner.os, env.*, etc.
    Trusted,
    /// Expression we can't classify — inputs.*, custom contexts, format() calls
    /// with mixed arguments, etc. Rules should treat as potentially tainted.
    Unknown,
}

/// Shell flags that the host environment sets implicitly, independent of
/// what the script itself contains.
pub struct ImplicitShellFlags {
    /// `set -e` / `set -o errexit` is active.
    pub errexit: bool,
    /// `set -o pipefail` is active.
    pub pipefail: bool,
    /// The raw shell template string, e.g. "bash --noprofile --norc -eo pipefail {0}".
    /// Retained for diagnostics and for rules that need to reason about
    /// non-standard templates.
    pub template: Option<String>,
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

#### Shell Template and Implicit Flags

Beyond resolving the dialect, the extractor also determines the effective shell invocation template and derives `ImplicitShellFlags` from it. GHA's default templates inject error-handling flags that aren't visible in the `run:` script itself:

| `shell:` value | Effective template | `errexit` | `pipefail` |
|---|---|---|---|
| `bash` (or absent on Unix) | `bash --noprofile --norc -eo pipefail {0}` | `true` | `true` |
| `sh` | `sh -e {0}` | `true` | `false` |
| `bash -e {0}` (custom) | `bash -e {0}` | `true` | `false` |
| `bash {0}` (custom) | `bash {0}` | `false` | `false` |

When the `shell:` value is a custom template (contains `{0}`), the extractor parses the flags from the template string directly. Otherwise it uses GHA's documented default templates.

The `ImplicitShellFlags` are attached to each `EmbeddedScript` so that error-handling rules can reason about what's active without the script explicitly containing `set -e` or `set -o pipefail`.

#### `${{ }}` Placeholder Substitution

GitHub Actions interpolates `${{ expression }}` syntax into `run:` blocks before the shell sees them. These are not valid shell syntax and would cause parse errors.

The extractor replaces each `${{ ... }}` occurrence with a synthetic environment variable reference: `$_SHUCK_GHA_1`, `$_SHUCK_GHA_2`, etc. (monotonically increasing per snippet). This approach:

- Produces valid shell syntax — no parse errors from template expressions
- Preserves expansion semantics — quoting rules (S001/SC2086) still fire on `echo $_SHUCK_GHA_1` just as they would on `echo ${{ github.ref }}`
- Is deterministic — same input always produces same placeholders

Nested expressions like `${{ format('refs/heads/{0}', matrix.branch) }}` are handled by matching the outermost `${{` ... `}}` pair. The regex pattern is `\$\{\{.*?\}\}` applied non-greedily.

#### Placeholder Provenance

Each substitution is recorded in a `PlaceholderMapping` that preserves the original expression text, its span in both the substituted source and the host file, and a taint classification. This metadata enables GHA-specific security and correctness rules (see [GHA-Specific Rules](#gha-specific-rules)) without requiring those rules to re-parse the YAML.

#### Taint Classification

The extractor classifies each `${{ }}` expression's taint based on the context prefix:

| Expression pattern | Taint | Rationale |
|---|---|---|
| `github.event.issue.title`, `.body` | `UserControlled` | Issue author controls content |
| `github.event.pull_request.title`, `.body`, `.head.ref` | `UserControlled` | PR author controls content |
| `github.event.comment.body` | `UserControlled` | Comment author controls content |
| `github.event.review.body` | `UserControlled` | Reviewer controls content |
| `github.event.discussion.title`, `.body` | `UserControlled` | Discussion author controls content |
| `github.event.pages.*.page_name` | `UserControlled` | Wiki editor controls content |
| `github.event.commits.*.message`, `.author.name`, `.author.email` | `UserControlled` | Committer controls content |
| `github.head_ref` | `UserControlled` | PR author controls branch name |
| `secrets.*` | `Secret` | Should not appear in command args or logs |
| `github.token` | `Secret` | The automatic GITHUB_TOKEN |
| `github.repository`, `github.sha`, `github.ref`, `github.run_id`, `runner.os`, `runner.arch` | `Trusted` | Repo-scoped, not attacker-controlled |
| `env.*`, `vars.*` | `Trusted` | Set by repo/org admins |
| `matrix.*`, `needs.*`, `steps.*` | `Trusted` | Controlled by workflow definition |
| `inputs.*` | `Unknown` | Depends on caller — could be attacker-controlled for `workflow_dispatch` with public repos |
| `format(...)`, `toJSON(...)`, other function calls | `Unknown` | Taint depends on arguments; conservative default |
| Anything else | `Unknown` | Conservative default for unrecognized expressions |

The taint table is intentionally conservative: `Unknown` is "treat as potentially tainted", not "ignore". Rules that flag `UserControlled` expressions should also flag `Unknown` expressions at a lower severity or with a different message suggesting manual review.

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
   b. Run the linter with embedded-snippet metadata available to the checker (see below)
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

#### Embedded Snippet Context

When a snippet originates from an extractor, the checker receives metadata from the `EmbeddedScript`, including the embedded format. This serves two purposes:

1. Rules that are inherently file-level (shebang checks, file-level directive checks) skip silently for extracted snippets.
2. Format-specific rules (see [GHA-Specific Rules](#gha-specific-rules)) check the embedded format to enable GHA-aware analysis.

The `EmbeddedScript` struct's `placeholders` and `implicit_flags` are also made available to rules through the `Checker` — either via a new `EmbeddedContext` field on `Checker` or by extending `LinterFacts` with an `embedded` section. The exact plumbing is an implementation detail, but the contract is: any rule can query placeholder provenance, taint classification, and implicit shell flags for the current snippet.

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

### GHA-Specific Rules

The metadata carried by `EmbeddedScript` — format identity, placeholder provenance, taint classification, and implicit shell flags — enables a class of rules that are only meaningful in a GitHub Actions context. These rules check for `EmbeddedFormat::GitHubActions` before activating. They are listed below in priority order; individual rule codes will be assigned during implementation.

#### Script Injection via `${{ }}` (Security, Error)

The highest-value GHA-specific rule. GitHub Actions interpolates `${{ }}` expressions *textually* into the `run:` script before bash sees it. If an attacker controls the expression's value, they get arbitrary code execution.

```yaml
# DANGEROUS — attacker controls PR title, injected raw into shell
- run: echo "${{ github.event.pull_request.title }}"

# DANGEROUS — quoting doesn't help, interpolation happens before bash
- run: |
    title="${{ github.event.pull_request.title }}"
    echo "$title"

# SAFE — use an environment variable
- run: echo "$TITLE"
  env:
    TITLE: ${{ github.event.pull_request.title }}
```

The rule fires when any `${{ }}` placeholder with `taint == UserControlled` appears anywhere in the shell source. The fix is always the same: move the expression to `env:` and reference the environment variable instead.

For `taint == Unknown` expressions, the rule fires at a lower severity (warning instead of error) with a message suggesting manual review.

This rule is format-specific because the injection mechanism (`${{ }}` textual interpolation before shell execution) is a GHA-specific behavior. Other CI systems interpolate differently or not at all.

#### Secrets in Command Arguments (Security, Warning)

Secrets passed as inline `${{ }}` expressions in command arguments are visible in the process table (`ps`, `/proc/$pid/cmdline`) to other processes on the runner.

```yaml
# WARNING — secret visible in process table
- run: curl -H "Authorization: Bearer ${{ secrets.TOKEN }}" https://api.example.com

# OK — environment variable is not visible in process args
- run: curl -H "Authorization: Bearer $TOKEN" https://api.example.com
  env:
    TOKEN: ${{ secrets.TOKEN }}
```

The rule fires when a placeholder with `taint == Secret` appears in a command argument position (as opposed to being assigned to a variable and used via the variable). This catches `secrets.*` and `github.token`.

#### Injection via `GITHUB_ENV` / `GITHUB_OUTPUT` / `GITHUB_PATH` Writes (Security, Warning)

Writing attacker-controlled expressions to GitHub's environment files enables indirect injection — an attacker can break out of a value boundary and set arbitrary environment variables or PATH entries for subsequent steps.

```yaml
# DANGEROUS — attacker can inject newline + arbitrary KEY=VALUE
- run: echo "TITLE=${{ github.event.issue.title }}" >> "$GITHUB_ENV"

# SAFER — use heredoc delimiter syntax for multiline-safe writes
- run: |
    echo "TITLE<<EOF" >> "$GITHUB_ENV"
    echo "$TITLE" >> "$GITHUB_ENV"
    echo "EOF" >> "$GITHUB_ENV"
  env:
    TITLE: ${{ github.event.issue.title }}
```

The rule fires when a `UserControlled` or `Unknown` placeholder appears in a string that is redirected/appended to `$GITHUB_ENV`, `$GITHUB_OUTPUT`, or `$GITHUB_PATH`.

#### Deprecated Workflow Commands (Correctness, Warning)

GitHub deprecated `::set-output` and `::save-state` workflow commands in October 2022 and `::add-path` earlier. These are disabled by default in new repositories.

```yaml
# DEPRECATED
- run: echo "::set-output name=result::$value"

# CURRENT
- run: echo "result=$value" >> "$GITHUB_OUTPUT"
```

The rule scans for `echo "::set-output`, `echo "::save-state`, and `echo "::add-path` patterns in the shell source. This is a straightforward string/AST match that doesn't require placeholder metadata — but it only fires when `format == GitHubActions` since these commands are meaningless outside GHA.

#### Redundant `set -e` / `set -o pipefail` (Style, Info)

GHA's default bash template (`bash --noprofile --norc -eo pipefail {0}`) already enables `errexit` and `pipefail`. Explicitly adding `set -e`, `set -eo pipefail`, or `set -euo pipefail` at the top of a `run:` block is redundant noise.

```yaml
# REDUNDANT — already active from the default template
- run: |
    set -eo pipefail
    make build
```

The rule checks `implicit_flags.errexit` and `implicit_flags.pipefail` before firing. If the script uses a custom template like `shell: bash {0}` (where implicit flags are both false), the rule stays silent.

Conversely, `set +e` disabling the implicit `errexit` could be flagged as a companion informational diagnostic, since it silently changes the error-handling contract that GHA users expect.

#### Commands That Fail Under Implicit `errexit` (Correctness, Warning)

Several common commands return non-zero for non-error conditions. Under `set -e` (which is implicit in GHA's default bash template), these kill the step unexpectedly:

```yaml
# grep returns 1 when no matches — step fails
- run: |
    count=$(grep -c "pattern" file.txt)
    echo "Found $count matches"

# diff returns 1 when files differ — step fails
- run: |
    diff old.txt new.txt > changes.patch
```

This rule is not GHA-exclusive — it applies to any script with `set -e` — but `implicit_flags` lets it fire on GHA scripts that never explicitly set `set -e`. Without the implicit flags metadata, the rule would miss every GHA script that relies on the default template.

#### Unquoted `$GITHUB_OUTPUT` / `$GITHUB_ENV` / `$GITHUB_PATH` (Correctness, Warning)

These environment variables contain file paths that could theoretically contain spaces (and have done so on some runner images). Unquoted usage in redirections is a bug:

```yaml
# BUG — word splitting if path contains spaces
- run: echo "foo=bar" >> $GITHUB_OUTPUT

# OK
- run: echo "foo=bar" >> "$GITHUB_OUTPUT"
```

This is a specialization of the general unquoted-variable rule (S001/SC2086), but with GHA context the diagnostic message can be more specific: it names the variable and explains that the file path is runner-dependent.

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

### Classify taint at rule time instead of extraction time

The taint classification could live in the rule rather than the extractor — the rule would inspect placeholder expressions and classify them on the fly. Rejected because (1) the taint table is a property of the GHA platform, not a lint policy, so it belongs with the extractor; (2) multiple rules need taint (injection, secrets-in-args, GITHUB_ENV writes), so classifying once avoids duplication and inconsistency; (3) keeping taint in `PlaceholderMapping` makes it testable independently of the rule engine.

### Default `embedded = false` (opt-in)

Safer rollout, but reduces the value of the feature — most users wouldn't know to enable it. Since the probe step prevents false positives on non-GHA YAML files, and the extraction is read-only, defaulting to `true` is low-risk and matches the expected behavior of `shuck check .` scanning everything relevant.

## Verification

### Unit Tests

- **Extractor trait**: Test `matches()`, `probe()`, and `extract()` independently for the GHA extractor.
- **Shell resolution**: Verify the three-level hierarchy (workflow → job → step) resolves correctly with all combinations of `shell:` and `runs-on:`.
- **Placeholder substitution**: Confirm `${{ expr }}` is replaced with `$_SHUCK_GHA_N`, including nested braces and multi-line expressions.
- **Placeholder provenance**: Verify each `PlaceholderMapping` records the correct original expression text, inner expression, and spans in both the substituted source and host file.
- **Taint classification**: Test that known attacker-controlled expressions (`github.event.pull_request.title`, `github.head_ref`, etc.) are classified as `UserControlled`, secrets as `Secret`, repo-scoped values as `Trusted`, and unrecognized expressions as `Unknown`.
- **Implicit shell flags**: Verify that the default bash template produces `errexit: true, pipefail: true`, `shell: sh` produces `errexit: true, pipefail: false`, and custom templates like `bash {0}` produce `errexit: false, pipefail: false`.
- **Block scalar handling**: Test literal (`|`), folded (`>`), strip (`|-`), and flow scalar extraction with correct offset computation.
- **Line remapping**: Verify diagnostic positions map back to the correct YAML line/column.
- **Unsupported shell skip**: Confirm PowerShell steps on Windows runners produce no diagnostics.
- **Composite actions**: Test extraction from `action.yml` with `runs.using: composite`.

### GHA-Specific Rule Tests

- **Script injection**: Verify that `run: echo ${{ github.event.pull_request.title }}` produces a security error, that `run: echo ${{ github.sha }}` (trusted) does not, and that `run: echo ${{ inputs.name }}` (unknown) produces a lower-severity warning.
- **Secrets in args**: Verify that `run: curl -H "${{ secrets.TOKEN }}" url` fires but `env: { TOKEN: "${{ secrets.TOKEN }}" }` with `run: curl -H "$TOKEN" url` does not.
- **GITHUB_ENV injection**: Verify that writing a `UserControlled` placeholder to `>> "$GITHUB_ENV"` fires.
- **Deprecated commands**: Verify that `echo "::set-output name=foo::bar"` fires only when `format == GitHubActions`.
- **Redundant set -e**: Verify that `set -eo pipefail` fires when `implicit_flags.errexit && implicit_flags.pipefail`, but not when the step uses `shell: bash {0}`.

### Integration Tests

- **End-to-end check**: Run `shuck check` on a fixture `.github/workflows/test.yml` containing known lint violations. Verify diagnostics appear with correct file paths and line numbers.
- **Mixed project**: Run `shuck check .` on a project with both `.sh` files and `.github/workflows/*.yml`. Verify both are discovered and linted.
- **Config opt-out**: Set `embedded = false` in `.shuck.toml` and confirm YAML files are skipped.
- **Suppression in embedded**: Verify `# shellcheck disable=SC2086` inside a `run: |` block suppresses the expected diagnostic.
- **Security rules end-to-end**: Run `shuck check` on a workflow with script injection, secrets-in-args, and `GITHUB_ENV` injection patterns. Verify all security diagnostics appear with correct severity and remapped locations.

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

```bash
# Security-focused test workflow
cat > /tmp/test-security.yml << 'EOF'
on:
  pull_request_target:
    types: [opened, edited]
jobs:
  greet:
    runs-on: ubuntu-latest
    steps:
      # Script injection — attacker controls PR title
      - run: echo "PR: ${{ github.event.pull_request.title }}"

      # Secret in command argument — visible via ps
      - run: curl -H "Authorization: Bearer ${{ secrets.API_KEY }}" https://api.example.com

      # GITHUB_ENV injection — attacker can inject arbitrary env vars
      - run: echo "GREETING=Hello ${{ github.event.pull_request.title }}" >> "$GITHUB_ENV"

      # Deprecated workflow command
      - run: echo "::set-output name=result::done"

      # Redundant set -e (default template already sets it)
      - run: |
          set -eo pipefail
          make build

      # Safe — uses env: instead of inline expression
      - run: echo "PR: $TITLE"
        env:
          TITLE: ${{ github.event.pull_request.title }}

      # Trusted expression — no injection risk
      - run: echo "SHA: ${{ github.sha }}"
EOF

# Expect security diagnostics on the injection, secrets, GITHUB_ENV, and
# deprecated command lines. The safe and trusted lines should be clean.
shuck check /tmp/test-security.yml
```
