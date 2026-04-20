# 006: Suppressions

## Status

Proposed

## Summary

A suppression layer that allows users to silence specific lint diagnostics via inline comments. Suppressions are parsed from two directive formats (`# shuck:` and `# shellcheck`), built into a per-file `SuppressionIndex`, and applied as a post-hoc filter on linter output. This spec covers directive parsing, shellcheck-style scope resolution (next-command or file-wide), explicit whole-file disables, index construction, and integration with the linter from spec 005.

## Motivation

Lint rules produce false positives. Users need a way to acknowledge and silence specific diagnostics without disabling rules globally. Shell scripts in particular carry two established suppression conventions:

- **shuck native**: `# shuck: disable=C001` — supports shellcheck-style disable semantics plus explicit `disable-file`
- **shellcheck compatible**: `# shellcheck disable=SC2086` — widely used in existing codebases, applies to the next command only

Both directive styles accept either code namespace. Native shuck codes (`C001`, `S001`, `SH-001`) and ShellCheck codes (`SC2086`, `2154`) resolve to the same underlying rules.

The Go frontend implements both. The Rust rewrite must preserve this behavior so users migrating from the Go tool or from shellcheck don't need to rewrite their suppression comments.

Following ruff's architecture, suppression filtering happens **after** the linter produces diagnostics — the checker runs without knowledge of suppressions, and a post-processing step filters the results. This keeps the checker simple and makes suppressions independently testable.

## Design

### Directive Formats

#### Shuck Native: `# shuck: <action>=<codes>`

Case-insensitive prefix match on `shuck:`. The body after the prefix is `action=codes` where:

- **action** is one of `disable` or `disable-file`
- **codes** is a comma-separated list of rule codes (e.g., `C001,S003` or `SC2086,2154`)
- An optional reason after `#` is stripped: `# shuck: disable=C001 # legacy code`

```shell
# shuck: disable=C001          # shellcheck-style: suppress the next command
echo $undefined

# shuck: disable-file=S001     # file: suppress S001 for entire file
```

`# shuck: disable=...` follows the same placement rules as shellcheck: before the first statement it becomes file-wide, otherwise it applies to the next command, including the same inline control-flow header forms that shellcheck accepts. `disable-file` remains an explicit whole-file escape hatch.

#### ShellCheck Compatible: `# shellcheck disable=<codes>`

Case-insensitive prefix match on `shellcheck`. Additional constraints:

- **Must be an own-line comment** — inline `# shellcheck disable=...` after code is ignored (matches shellcheck's behavior)
- Only the `disable` action is supported
- Supports multiple code groups: `# shellcheck disable=SC2086 disable=SC2034`
- Codes can be shellcheck `SC` codes or shuck-native codes. ShellCheck codes are mapped to shuck rules via a resolution table, and shuck codes resolve through the native registry.

Scope depends on position:

- **Before first statement** → file-wide suppression
- **Otherwise** → applies to the **next command statement** only (the immediately following `Stmt` node in the AST)

```shell
# shellcheck disable=SC2086    # before first stmt: whole-file
#!/bin/bash

echo $foo                      # SC2086 suppressed (whole-file)

# shellcheck disable=SC2034    # after first stmt: next command only
x=1                            # SC2034 suppressed here
echo $x                        # SC2034 NOT suppressed here
```

### Directive Parsing

```rust
/// A parsed suppression directive from a comment.
pub struct SuppressionDirective {
    /// The action: disable or disable-file.
    pub action: SuppressionAction,
    /// Which directive syntax produced this.
    pub source: SuppressionSource,
    /// Rule codes this directive applies to.
    pub codes: Vec<Rule>,
    /// The comment's source range (for diagnostics pointing at the directive).
    pub range: TextRange,
    /// 1-based line number of the directive.
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionAction {
    Disable,
    DisableFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionSource {
    /// `# shuck: ...`
    Shuck,
    /// `# shellcheck disable=...`
    ShellCheck,
}
```

Parsing entry point:

```rust
/// Parse all suppression directives from a file's comments.
///
/// Comments are provided by the `CommentIndex`. For each comment,
/// tries shuck-native parsing first, then shellcheck parsing.
/// Returns directives sorted by (line, offset).
pub fn parse_directives(
    source: &str,
    comment_index: &CommentIndex,
    shellcheck_map: &ShellCheckCodeMap,
) -> Vec<SuppressionDirective> {
    let mut directives = Vec::new();
    for comment in comment_index.comments() {
        let text = comment_text(source, comment);
        if let Some(d) = parse_shuck_directive(text, comment) {
            directives.push(d);
        } else if let Some(d) = parse_shellcheck_directive(
            source, text, comment, shellcheck_map,
        ) {
            directives.push(d);
        }
    }
    directives.sort_by_key(|d| (d.line, d.range.start()));
    directives
}
```

**ShellCheck code mapping** uses a static lookup table (`ShellCheckCodeMap`) that maps SC codes to `Rule` values. Unmapped SC codes are silently ignored — this matches the Go behavior where unknown codes are dropped during normalization.

```rust
/// Maps shellcheck SC codes to shuck Rule values.
pub struct ShellCheckCodeMap {
    map: FxHashMap<u32, Rule>,  // SC number → Rule
}

impl ShellCheckCodeMap {
    /// Look up a shellcheck code like "SC2086".
    pub fn resolve(&self, sc_code: &str) -> Option<Rule> {
        let num: u32 = sc_code.strip_prefix("SC")?.parse().ok()?;
        self.map.get(&num).copied()
    }
}
```

### Suppression Scopes

There are three scope types:

| Scope | Trigger | Range |
|-------|---------|-------|
| **File** | `# shuck: disable-file=C001` | Entire file |
| **File** | `# shuck: disable=C001` before first statement | Entire file |
| **File** | `# shellcheck disable=SC2086` before first statement | Entire file |
| **Next-command** | `# shuck: disable=C001` after first statement | Start line through end line of the next `Stmt` AST node |
| **Next-command** | `# shellcheck disable=SC2086` after first statement | Start line through end line of the next `Stmt` AST node |

### SuppressionIndex

The index is the query structure built from parsed directives. It answers: "is rule R suppressed on line L?"

```rust
/// Per-file suppression index. Built once, queried per diagnostic.
pub struct SuppressionIndex {
    /// Per-rule suppression state.
    by_rule: FxHashMap<Rule, RuleSuppressionIndex>,
}

impl SuppressionIndex {
    /// Build from parsed directives.
    pub fn new(
        directives: &[SuppressionDirective],
        script: &Script,
        first_stmt_line: u32,
    ) -> Self { /* ... */ }

    /// Check if rule `rule` is suppressed on `line`.
    pub fn is_suppressed(&self, rule: Rule, line: u32) -> bool {
        match self.by_rule.get(&rule) {
            None => false,
            Some(index) => index.is_suppressed(line),
        }
    }
}
```

#### RuleSuppressionIndex

Per-rule state combining file and next-command scopes:

```rust
struct RuleSuppressionIndex {
    /// If set, the rule is suppressed for the entire file.
    whole_file: bool,
    /// Line ranges from next-command suppressions.
    ranges: Vec<LineRange>,
}

struct LineRange {
    start_line: u32,
    end_line: u32,
}
```

**Query logic** (`is_suppressed`), checked in priority order:

1. If `whole_file` is true → suppressed
2. Binary search `ranges` for any range containing the query line → suppressed
3. Otherwise → not suppressed

#### Building the Index

Construction mirrors `BuildCodeSuppressionIndex` from Go:

1. Group directives by rule code
2. For each rule, iterate its directives in source order:
   - **`disable-file`** → set `whole_file = true`
   - **`disable` before first statement** → set `whole_file = true`
   - **`disable` after first statement** → find the next `Stmt` after the directive's offset via AST walk, push a `LineRange { start_line, end_line }` for that statement
3. Sort `ranges` by `(start_line, end_line)`

**Finding the next command** for shellcheck scopes requires walking statement nodes in the AST to find the first `Stmt` whose start offset is after the directive's end offset:

```rust
/// Find the line range of the next statement after `offset`.
fn next_command_range(script: &Script, offset: TextSize) -> Option<LineRange> {
    // Walk all statements, find the one with the smallest start
    // offset that is still > `offset`.
    // ...
}
```

This is equivalent to the Go `NextCommandLineRange()` function. It walks all statements (including inside command substitutions) and picks the nearest one after the directive.

**Whole-file detection for shellcheck** uses `first_stmt_line` — the line number of the first statement in the file. A shellcheck directive on a line before `first_stmt_line` is promoted to file scope. This parameter is computed by the caller from the AST.

### Integration with the Linter

Following ruff's post-hoc pattern, suppressions are applied **after** the checker runs:

```rust
/// Lint a single parsed file and return diagnostics.
pub fn lint_file(
    script: &Script,
    source: &str,
    semantic: &SemanticModel,
    indexer: &Indexer,
    settings: &LinterSettings,
    suppression_index: Option<&SuppressionIndex>,
) -> Vec<Diagnostic> {
    let checker = Checker::new(script, source, semantic, indexer, &settings.rules);
    let mut diagnostics = checker.check();

    // Apply severity overrides
    for diag in &mut diagnostics {
        if let Some(&severity) = settings.severity_overrides.get(&diag.rule) {
            diag.severity = severity;
        }
    }

    // Filter suppressed diagnostics
    if let Some(suppressions) = suppression_index {
        diagnostics.retain(|diag| {
            let line = indexer.line_index().line_for_offset(diag.span.start());
            !suppressions.is_suppressed(diag.rule, line)
        });
    }

    diagnostics.sort_by_key(|d| (d.span.start(), d.span.end()));
    diagnostics
}
```

The `SuppressionIndex` is built **before** `lint_file` is called — the caller (CLI pipeline) constructs it from the comment index and passes it in. This keeps the linter crate's dependency on suppression parsing explicit and optional.

### Data Flow

Updated pipeline from spec 005:

```
Source text
    → shuck-parser::Parser::parse()       → ParseOutput (Script + Comments)
    → shuck-indexer::Indexer::new()        → Indexer (includes CommentIndex)
    → parse_directives()                   → Vec<SuppressionDirective>
    → SuppressionIndex::new()              → SuppressionIndex
    → shuck-semantic::SemanticModel::new() → SemanticModel
    → shuck-linter::lint_file()            → Vec<Diagnostic>  (filtered by suppressions)
    → CLI formats and prints diagnostics
```

### Module Location

Suppression logic lives in `shuck-linter`, matching ruff's pattern where `ruff_linter` owns both rule dispatch and noqa filtering:

```
crates/shuck-linter/src/
├── lib.rs
├── suppression/
│   ├── mod.rs           # parse_directives(), SuppressionIndex
│   ├── directive.rs     # SuppressionDirective, parse_shuck_directive(),
│   │                    #   parse_shellcheck_directive()
│   ├── index.rs         # SuppressionIndex, RuleSuppressionIndex, query logic
│   └── shellcheck_map.rs # ShellCheckCodeMap, SC→Rule mapping table
├── checker.rs
├── registry.rs
└── ...
```

The suppression module depends on `shuck-ast` (for `Script`, `TextRange`), `shuck-indexer` (for `CommentIndex`, `IndexedComment`), and the linter's own `registry` (for `Rule`).

## Alternatives Considered

### Alternative A: Suppressions Inside the Checker

Have the `Checker` consult suppressions during `report()` — skip emitting a diagnostic if the line is suppressed. This is how the Go frontend works (in `RuleContext.ReportDiagnostic`).

**Rejected because:** Ruff deliberately separates these concerns. Post-hoc filtering means the checker doesn't need a reference to the suppression index, rules don't need to think about suppressions, and suppression logic can be tested independently against a list of diagnostics. It also keeps the door open for future features like unused-suppression detection, which requires knowing what *would* have been emitted.

### Alternative B: Suppression Index in shuck-indexer

Put directive parsing and the suppression index in `shuck-indexer` alongside `CommentIndex`, since comments are already collected there.

**Rejected because:** The indexer provides raw positional data — it doesn't interpret comment content or depend on rule definitions. Suppression parsing requires knowing valid rule codes (from the `Rule` enum in `shuck-linter`), which would create a circular dependency. Ruff keeps noqa logic in `ruff_linter`, not in `ruff_python_index`, for the same reason.

### Alternative C: Support `enable` for shellcheck Directives

Allow `# shellcheck enable=SC2086` to re-enable a suppressed rule.

**Rejected because:** ShellCheck itself doesn't support `enable`. Adding non-standard extensions to shellcheck's directive format would confuse users who expect shellcheck-compatible behavior.

### Alternative D: Unused Suppression Warnings

Report diagnostics when a suppression directive doesn't silence anything, similar to ruff's `RUF100`.

**Rejected because:** ShellCheck doesn't do this (it's an open feature request, koalaman/shellcheck#645). Since we aim for behavioral compatibility with shellcheck's suppression model, and this adds significant complexity (tracking which suppressions were "used" during filtering), we defer this to a future spec.

## Verification

Once implemented, verify with:

- **Shuck directive parsing**: A comment `# shuck: disable=C001,S001` produces a `SuppressionDirective` with `action == Disable`, `codes == [Rule::UndefinedVariable, Rule::UnquotedExpansion]`, and `source == Shuck`.
- **ShellCheck directive parsing**: A comment `# shellcheck disable=SC2086` on its own line maps to the correct shuck `Rule` via `ShellCheckCodeMap`. The same comment inline after code is ignored.
- **Disable-file scope**: `# shuck: disable-file=C001` at any position causes `is_suppressed(Rule::UndefinedVariable, line)` to return true for all lines.
- **Shuck next-command scope**: `# shuck: disable=C001` on line 5 (after the first statement) suppresses only the lines spanned by the next `Stmt` node. A diagnostic on a subsequent statement is not suppressed.
- **Shuck whole-file detection**: `# shuck: disable=C001` before the first statement suppresses the rule for all lines.
- **ShellCheck next-command scope**: `# shellcheck disable=SC2086` on line 5 (after first statement) suppresses only the lines spanned by the next `Stmt` node. A diagnostic on a subsequent statement is not suppressed.
- **ShellCheck whole-file detection**: `# shellcheck disable=SC2086` before the first statement suppresses the rule for all lines.
- **Post-hoc filtering**: `lint_file()` with a suppression index filters out diagnostics on suppressed lines while preserving diagnostics on non-suppressed lines.
- **Multiple codes**: `# shuck: disable=C001,C002,C003` suppresses all three rules.
- **Reason stripping**: `# shuck: disable=C001 # legacy code` parses correctly, ignoring the reason.
- **Unknown codes**: `# shellcheck disable=SC7777` (unmapped) is silently ignored and produces no suppression entries.
- **Empty/malformed directives**: `# shuck: disable=`, `# shuck: foobar=C001`, and `# shuck disable=C001` (missing colon) are all ignored.
- **Sort stability**: Diagnostics remain sorted by source position after suppression filtering.
