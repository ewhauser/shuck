# 014: Zsh Option-State Tracking

## Status

Proposed

## Summary

Add a zsh option-state tracking layer that models how `emulate`, `setopt`, `unsetopt`, `set -o`, `set +o`, and the `noglob` precommand modifier change analysis-relevant shell behavior at each point in a zsh script. The system builds option facts from source during semantic analysis, scopes them correctly across function boundaries (via `emulate -L` and `LOCAL_OPTIONS`), and threads the resulting state into both the parser (for grammar-affecting options like `IGNORE_BRACES` and `SHORT_LOOPS`) and the linter's expansion analysis (for runtime-behavior options like `SH_WORD_SPLIT`, `GLOB_SUBST`, and `KSH_ARRAYS`). Per-expansion flag overrides (`${=...}`, `${~...}`, `${^...}`) are modeled as point overrides that supersede the ambient option state for a single expansion.

This spec extends the parser-level zsh support designed in [010-zsh.md](010-zsh.md) and the expansion analysis system designed in [008-expansion-analysis.md](008-expansion-analysis.md). It assumes the zsh parser syntax fidelity work (quoted parameter targets, `${#...}` nested targets, `:l/:t/:h` modifier suffixes, dialect-aware arithmetic parsing) has already landed.

## Motivation

Zsh expansion is partly syntax and partly mutable shell state. The same source text `$foo` can mean different things depending on which options are active:

```zsh
# Default zsh: $foo is one field even if it contains whitespace
foo="a b"
for x in $foo; do echo "$x"; done   # prints "a b" (one iteration)

setopt SH_WORD_SPLIT
for x in $foo; do echo "$x"; done   # prints "a" then "b" (two iterations)
```

Today, shuck picks one file-wide `ShellDialect` and carries only that coarse choice through parsing and linting. The CLI does this in `check.rs` (dialect inference at parse time), settings store only `shell` in `LinterSettings`, and parser features are hardcoded from a fixed dialect table in `DialectFeatures`. Expansion analysis has no option-state input — `analyze_word()` and `analyze_parameter_part()` take no dialect or option parameter — and all zsh parameter expansions are treated very conservatively as `PartValueShape::Unknown` with blanket field-splitting and pathname-matching hazards.

This means:

- **S001 (UnquotedExpansion)** cannot distinguish zsh-default behavior (no word splitting on `$var`) from `SH_WORD_SPLIT`-enabled behavior (word splitting on `$var`). It either false-positives on safe zsh expansions or misses real hazards after `setopt SH_WORD_SPLIT`.
- **S003 (UnquotedArrayExpansion)** cannot account for `KSH_ARRAYS` changing array indexing from 1-based to 0-based and altering `$array` semantics.
- **C012 (LeadingGlobArgument)** and **K003 (RmGlobOnVariablePath)** cannot detect `noglob` or `setopt NO_GLOB` disabling globbing entirely.
- **Portability rules** (X013, X019, X078, X079) that reason about array semantics cannot tell when `KSH_ARRAYS` makes zsh arrays behave like ksh arrays.
- **Parser grammar** is fixed at file level, but options like `IGNORE_BRACES`, `SHORT_LOOPS`, `SH_GLOB`, and `EXTENDED_GLOB` change what syntax is valid. A function that starts with `emulate -L sh` should parse differently from surrounding zsh code.
- **Per-expansion overrides** like `${=foo}` (force word splitting), `${~foo}` (force glob substitution), and `${^foo}` (force RC_EXPAND_PARAM) are recognized by the parser as zsh modifier flags but not yet interpreted by the expansion analysis layer.

The `emulate -L` pattern is pervasive in real zsh code. Nearly every well-written plugin, completion function, and library function uses it to scope behavior changes to the current function. Without option-state tracking, the linter cannot reason correctly about the majority of production zsh code.

## Design

### Option Inventory

The following zsh options are analysis-relevant. They are grouped by which system they affect.

#### Word Splitting and Field Generation

| Option | Default (zsh native) | Effect on analysis |
|--------|---------------------|-------------------|
| `SH_WORD_SPLIT` | off | Unquoted `$var` undergoes `$IFS` splitting. Changes whether S001 should flag unquoted scalar expansions. |
| `GLOB_SUBST` | off | Results of parameter/command substitution become glob patterns. Adds pathname-matching hazard to all unquoted expansions. |
| `RC_EXPAND_PARAM` | off | `${array}text` expands element-wise (`${arr[1]}text ${arr[2]}text ...`). Changes multi-field expansion behavior. |

#### Globbing and Pattern Matching

| Option | Default (zsh native) | Effect on analysis |
|--------|---------------------|-------------------|
| `GLOB` | on | Master switch for filename generation. When off, `*` and `?` are literal. |
| `NOMATCH` | on | Unmatched globs are errors. When off, unmatched globs pass through literally. |
| `NULL_GLOB` | off | Unmatched globs expand to nothing. Mutually exclusive with `NOMATCH` semantics. |
| `CSH_NULL_GLOB` | off | Like `NULL_GLOB` but only requires one glob in a set to match. |
| `EXTENDED_GLOB` | off | Enables `#`, `~`, `^` as glob operators. Changes the pattern language. |
| `KSH_GLOB` | off | Enables `@(...)`, `*(...)`, `+(...)`, `?(...)`, `!(...)` glob operators. |
| `SH_GLOB` | off | Disables special glob meaning of `(`, `|`, `)`, and `<` so that ksh-style extended globbing and zsh grouping/alternation/numeric ranges are not recognized. POSIX compatibility option. |
| `BARE_GLOB_QUAL` | on | Trailing `(...)` on a glob is a qualifier, not a group. |
| `GLOB_DOTS` | off | Globs match files starting with `.`. |

#### Filename Expansion Context

| Option | Default (zsh native) | Effect on analysis |
|--------|---------------------|-------------------|
| `EQUALS` | on | `=cmd` expands to the path of `cmd`. |
| `MAGIC_EQUAL_SUBST` | off | `anything=value` arguments undergo filename expansion on the value. |
| `SH_FILE_EXPANSION` | off | Filename expansion happens before parameter expansion (POSIX order). |
| `GLOB_ASSIGN` | off | Filename expansion occurs on RHS of assignments. |

#### Brace Handling

| Option | Default (zsh native) | Effect on analysis |
|--------|---------------------|-------------------|
| `IGNORE_BRACES` | off | Disables `{a,b}` brace expansion entirely. **Grammar-affecting.** |
| `IGNORE_CLOSE_BRACES` | off | `}` is not special (used for compatibility). **Grammar-affecting.** |
| `BRACE_CCL` | off | `{aeiou}` (no commas) expands to individual characters. |

#### Array Semantics

| Option | Default (zsh native) | Effect on analysis |
|--------|---------------------|-------------------|
| `KSH_ARRAYS` | off | Arrays are 0-indexed, `$arr` means `${arr[0]}` not `${arr[@]}`. Changes S003, X013, X019. |
| `KSH_ZERO_SUBSCRIPT` | off | `${arr[0]}` is valid even in 1-based mode. |

#### Parsing and Syntax

| Option | Default (zsh native) | Effect on analysis |
|--------|---------------------|-------------------|
| `SHORT_LOOPS` | on | Enables short forms for `for`, `select`, `if`, and `function` — allows a single command body without `do`/`done` or `then`/`fi`. For example: `for name in words; command`, `if [[ test ]] command`, `function name command`. **Grammar-affecting.** |
| `SHORT_REPEAT` | on | Enables short form for `repeat`: `repeat count; command` without `do`/`done`. **Grammar-affecting.** |
| `RC_QUOTES` | off | `''` inside single quotes is an escaped quote. **Grammar-affecting.** |
| `INTERACTIVE_COMMENTS` | on (in scripts) | `#` starts a comment. On by default in scripts and non-interactive shells; off by default in interactive shells without `-k`. Since shuck only analyzes script files, the default is on. |
| `C_BASES` | off | Arithmetic output uses `0x`/`0` prefixes. |
| `OCTAL_ZEROES` | off | Leading `0` in arithmetic is octal. |

#### Emulation Modes

`emulate` presets a constellation of options to match another shell's behavior:

| Mode | Key option changes from zsh default |
|------|-------------------------------------|
| `emulate sh` | `SH_WORD_SPLIT` on, `GLOB_SUBST` on, `SH_GLOB` on, `SH_FILE_EXPANSION` on, `POSIX_BUILTINS` on, `BSD_ECHO` off, `KSH_ARRAYS` off, `BARE_GLOB_QUAL` off |
| `emulate ksh` | `SH_WORD_SPLIT` on, `GLOB_SUBST` on, `KSH_GLOB` on, `KSH_ARRAYS` on, `SH_GLOB` on, `BARE_GLOB_QUAL` off |
| `emulate csh` | `CSH_NULL_GLOB` on, `SH_WORD_SPLIT` off, `GLOB_SUBST` off |
| `emulate zsh` | All defaults restored |

The `-L` flag scopes the emulation to the current function. Without `-L`, the emulation applies globally from that point forward.

### ZshOptionState

A new type represents the set of analysis-relevant zsh options at a program point:

```rust
/// Tri-state for options that may be conditionally set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionValue {
    /// Definitively on.
    On,
    /// Definitively off.
    Off,
    /// Set on some paths, unset on others — analysis must be conservative.
    Unknown,
}

/// The subset of zsh options that affect parsing and linting behavior.
///
/// Each field defaults to the zsh-native default for that option.
/// `emulate`, `setopt`, `unsetopt`, and `set -o/+o` update individual fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZshOptionState {
    // Word splitting and field generation
    pub sh_word_split: OptionValue,      // default: Off
    pub glob_subst: OptionValue,         // default: Off
    pub rc_expand_param: OptionValue,    // default: Off

    // Globbing and pattern matching
    pub glob: OptionValue,               // default: On
    pub nomatch: OptionValue,            // default: On
    pub null_glob: OptionValue,          // default: Off
    pub csh_null_glob: OptionValue,      // default: Off
    pub extended_glob: OptionValue,      // default: Off
    pub ksh_glob: OptionValue,           // default: Off
    pub sh_glob: OptionValue,            // default: Off
    pub bare_glob_qual: OptionValue,     // default: On
    pub glob_dots: OptionValue,          // default: Off

    // Filename expansion context
    pub equals: OptionValue,             // default: On
    pub magic_equal_subst: OptionValue,  // default: Off
    pub sh_file_expansion: OptionValue,  // default: Off
    pub glob_assign: OptionValue,        // default: Off

    // Brace handling
    pub ignore_braces: OptionValue,      // default: Off
    pub ignore_close_braces: OptionValue,// default: Off
    pub brace_ccl: OptionValue,          // default: Off

    // Array semantics
    pub ksh_arrays: OptionValue,         // default: Off
    pub ksh_zero_subscript: OptionValue, // default: Off

    // Parsing and syntax
    pub short_loops: OptionValue,        // default: On
    pub short_repeat: OptionValue,       // default: On
    pub rc_quotes: OptionValue,          // default: Off
    pub interactive_comments: OptionValue, // default: On (in scripts)
    pub c_bases: OptionValue,            // default: Off
    pub octal_zeroes: OptionValue,       // default: Off
}
```

`ZshOptionState::zsh_default()` initializes all fields to zsh-native defaults. `ZshOptionState::for_emulate(mode)` initializes to the preset for a given emulation mode. Individual fields are updated by `apply_setopt(name)` and `apply_unsetopt(name)`, which handle zsh's case-insensitive, underscore-ignoring option name matching (e.g., `SH_WORD_SPLIT`, `shwordsplit`, `sh_word_split`, and `SHWORDSPLIT` all match).

### ShellProfile

The flat `ShellDialect` enum is insufficient to carry option state. A new `ShellProfile` wraps dialect and options together:

```rust
/// Complete shell configuration at a program point.
///
/// For non-zsh dialects, `options` is `None` and behavior is determined
/// entirely by the dialect. For zsh, `options` carries the mutable
/// option state that modifies parsing and analysis.
pub struct ShellProfile {
    pub dialect: ShellDialect,
    pub options: Option<ZshOptionState>,
}
```

The profile flows through the system as follows:

```
Source text + path/shebang/config
    → ShellDialect (file-level inference, same as today)
    → ShellProfile { dialect, options: ZshOptionState::zsh_default() }
    → Semantic builder tracks option-modifying commands
    → Per-scope ZshOptionState snapshots stored in the model
    → Checker queries option state at each program point
    → Expansion analysis receives effective options per word
    → Parser receives effective options for grammar decisions
```

For non-zsh dialects, `ShellProfile` is transparent — `options` is `None` and all existing code paths are unchanged. The option-state machinery is only active when `dialect == Zsh`.

### Building Option Facts from Source

The semantic model builder already walks the full AST to build scopes, bindings, references, and flow contexts. Option-state tracking extends this walk to recognize option-modifying commands and record state changes.

#### Recognized Commands

The builder recognizes the following commands as option-modifying:

| Command pattern | Effect |
|----------------|--------|
| `emulate zsh` | Reset all options to zsh defaults |
| `emulate sh` | Apply sh-compatible option preset |
| `emulate ksh` | Apply ksh-compatible option preset |
| `emulate csh` | Apply csh-compatible option preset |
| `emulate -L <mode>` | Same as `emulate <mode>` but scoped to current function |
| `emulate -R <mode>` | Same as `emulate` but also resets all options to the mode defaults |
| `setopt <name> [<name>...]` | Enable named options |
| `unsetopt <name> [<name>...]` | Disable named options |
| `set -o <name>` | Enable named option (POSIX-style) |
| `set +o <name>` | Disable named option (POSIX-style) |
| `noglob <command>` | Suppress globbing for a single command |

The builder normalizes option names using zsh's rules: case-insensitive, underscores ignored, and the `NO_` / `no` prefix inverts the sense (e.g., `setopt noglob` is equivalent to `unsetopt glob`).

#### Scope Model

Option state is tracked per scope, building on the existing `ScopeKind` infrastructure:

```
File scope (ZshOptionState::zsh_default())
├── Function A scope (emulate -L sh → sh-compat preset, LOCAL_OPTIONS implied)
│   ├── setopt NULL_GLOB → updates A's local state (does not leak to caller)
│   └── subshell → inherits A's state at point of fork
├── Function B scope (no emulate, no LOCAL_OPTIONS → inherits file-scope state)
│   └── setopt KSH_ARRAYS → updates B's state AND propagates to caller on return
└── top-level code (file-scope state, mutated by any global setopt/emulate/leaked function writes)
```

The scoping rules are:

1. **File scope** starts at `ZshOptionState::zsh_default()` (or `for_emulate(mode)` if the shebang or config specifies an emulation mode).

2. **`emulate -L`** creates a new option scope tied to the enclosing function's `ScopeKind::Function`. It implies `LOCAL_OPTIONS`, so all subsequent option changes within that function are local — they revert when the function returns. If `emulate -L` appears at file scope (outside any function), it behaves like `emulate` without `-L`.

3. **`LOCAL_OPTIONS`** (`setopt LOCAL_OPTIONS`): marks the current function's option state as local. Subsequent `setopt`/`unsetopt` within that function do not propagate to the caller. `emulate -L` implies this.

4. **`setopt`/`unsetopt` inside a function without `LOCAL_OPTIONS`**: updates the current scope's state **and propagates to the caller's scope when the function returns**. In zsh, option writes leak out of functions by default — only `LOCAL_OPTIONS` (or `emulate -L`) prevents this. The builder models this by applying the function's final option state to the call site's scope after the function call.

5. **`setopt`/`unsetopt` at file scope**: updates the file-scope state and affects all subsequent code.

6. **Subshells** (`( ... )`, command substitutions `$(...)`): inherit the option state at the point of creation. Changes inside the subshell do not propagate back.

7. **Pipelines**: in zsh, the **last** segment of a pipeline runs in the current shell (unlike Bash, where all segments are subshells by default). This means option writes in the rightmost pipeline command propagate to the caller. For example, `printf x | setopt SH_WORD_SPLIT` sets `SH_WORD_SPLIT` in the calling scope. The builder models the last pipeline segment as running in the current scope and all preceding segments as subshells. When `emulate sh` is active (which implies `SH_FILE_EXPANSION` and POSIX-like behavior), all pipeline segments run in subshells — the builder checks the effective emulation mode to determine pipeline semantics.

8. **Function calls without `LOCAL_OPTIONS`**: at each call site, the builder must account for the callee's option side effects. When the callee's final option state is statically known (no conditional branches, single `setopt` sequence), those changes are applied to the caller's state after the call. When the callee's side effects are ambiguous (conditional option changes, dynamic dispatch), the affected options become `Unknown` in the caller after the call.

9. **Conditional branches**: when `setopt`/`unsetopt` appears inside only one branch of an `if`/`case`, the state after the conditional is `Unknown` for affected options. This is the conservative merge — the analysis does not try to prove which branch was taken.

The builder records option state snapshots at each scope entry and at each option-modifying command. The semantic model exposes these as queryable per-span state:

```rust
impl SemanticModel {
    /// Returns the effective zsh option state at a given source location.
    /// Returns `None` for non-zsh dialects.
    pub fn zsh_options_at(&self, offset: usize) -> Option<&ZshOptionState> { ... }
}
```

The implementation walks the recorded option snapshots in source order within the enclosing scope, returning the most recent state that precedes the query offset. For offsets before any option-modifying command in a scope, the scope's entry state is returned.

#### Conditional Merging

When control flow merges after a conditional that modifies options on some paths:

```zsh
if [[ -n $ZSH_COMPAT ]]; then
    setopt SH_WORD_SPLIT
fi
# SH_WORD_SPLIT is Unknown here
echo $foo
```

The builder merges option states at the join point. For each option, the merge compares the **post-branch values** (not just whether a write occurred):

- If both branches produce the same value for an option: use that value. This includes the case where only one branch writes an option but the write is idempotent with the incoming state (e.g., `unsetopt SH_WORD_SPLIT` when it was already off — both branches end with `Off`, so the merged result is `Off`).
- If the two branches produce different values: `Unknown`.
- If either branch's value is already `Unknown`: `Unknown`.

This avoids unnecessary precision loss from idempotent writes. For example:

```zsh
# SH_WORD_SPLIT is Off here (zsh default)
if [[ -n $COMPAT ]]; then
    unsetopt SH_WORD_SPLIT   # still Off
fi
# Merged: Off (both branches agree), not Unknown
echo $foo
```

`Unknown` is always conservative: rules that depend on an option being definitively on or off treat `Unknown` as "might be either" and apply the more cautious analysis. For field-splitting rules, `Unknown` for `SH_WORD_SPLIT` means "assume splitting might happen" — which is the safe default for flagging potential hazards.

### Threading into the Parser

Some options change the grammar itself. These require the parser to consult option state, not just the static dialect feature table.

#### Grammar-Affecting Options

| Option | Grammar change |
|--------|---------------|
| `IGNORE_BRACES` | `{` and `}` are not special — no brace expansion, no brace groups |
| `IGNORE_CLOSE_BRACES` | `}` is not special |
| `SHORT_LOOPS` | `for name in words; command` without `do`/`done` |
| `SHORT_REPEAT` | `repeat count; command` without `do`/`done` |
| `RC_QUOTES` | `''` inside single quotes is an escaped `'` |
| `INTERACTIVE_COMMENTS` | `#` starts comments in interactive mode |
| `SH_GLOB` | `(`, `## `, `~` are not glob operators |
| `KSH_GLOB` | `@(...)`, `*(...)` etc. are glob operators |
| `EXTENDED_GLOB` | `#`, `~`, `^` are glob operators |

The current `DialectFeatures` table is a static mapping from dialect to feature flags. For zsh, several of these features become dynamic — their availability depends on the option state at the parse point.

The parser gains an `OptionAwareFeatures` layer that wraps `DialectFeatures`:

```rust
impl Parser<'_> {
    /// Query whether a feature is enabled at the current parse position.
    ///
    /// For non-zsh dialects, delegates directly to the static feature table.
    /// For zsh, consults the current option state for grammar-affecting options.
    fn feature_enabled(&self, feature: ParserFeature) -> bool {
        match self.dialect {
            ShellDialect::Zsh => self.zsh_feature_at_current_position(feature),
            _ => self.dialect.features().has(feature),
        }
    }
}
```

The parser does not re-run the full semantic analysis to determine option state. Instead, it uses a lightweight single-pass pre-scan that identifies `emulate` and `setopt`/`unsetopt` commands by their lexical shape before the main parse. This pre-scan builds a sorted list of `(offset, option_change)` entries. During parsing, the parser binary-searches this list to determine the effective grammar options at the current token position.

The pre-scan recognizes `emulate`, `setopt`, `unsetopt`, `set -o`, and `set +o` commands **at any position** in the source, not only at function entry or file top level. It scans all simple command words that match these patterns and records the grammar-affecting option changes with their source offsets. This ensures that a mid-function `setopt SHORT_LOOPS` followed by short-form syntax is parsed correctly.

The pre-scan is intentionally approximate in other ways: it does not attempt to resolve aliases, evaluate conditionals, or follow control flow. A `setopt` inside an `if` branch is recorded at its source offset and treated as effective from that point forward within its lexical scope (matched by brace/function nesting depth). This is a conservative over-approximation — it may enable a grammar feature slightly early on paths where the `setopt` is not reached, but it will not miss a grammar change that does execute. The semantic builder's more precise scope-aware analysis handles the full picture for linting; the parser pre-scan only needs to avoid false parse errors on valid code.

### Threading into Expansion Analysis

The expansion analysis layer (spec 008) currently has no option-state input. `analyze_word()` and `analyze_parameter_part()` hardcode behavior based on quoting context alone. For zsh, the effective behavior of an expansion depends on both quoting and option state.

#### Extending the Analysis API

The expansion analysis functions gain an optional `ZshOptionState` parameter:

```rust
pub(crate) fn analyze_word(
    word: &Word,
    source: &str,
    options: Option<&ZshOptionState>,
) -> ExpansionAnalysis { ... }

fn analyze_parameter_part(
    param: &ParameterExpansion,
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> PartAnalysis { ... }
```

When `options` is `None` (non-zsh dialects), behavior is unchanged. When `Some`, the analysis consults the option state for each decision point.

#### Option-Sensitive Analysis Decisions

The following analysis decisions change based on option state:

**Field splitting (`SH_WORD_SPLIT`)**:

In zsh-native mode (SH_WORD_SPLIT off), unquoted `$scalar` does **not** undergo field splitting. This is the opposite of Bash. The analysis must track this:

```rust
// Current (conservative): always assumes splitting
field_splitting: !in_double_quotes,

// With option state:
field_splitting: match options {
    Some(opts) if opts.sh_word_split.is_definitely_on() => !in_double_quotes,
    Some(opts) if opts.sh_word_split.is_definitely_off() => false,
    Some(_) => !in_double_quotes,  // Unknown → conservative
    None => !in_double_quotes,     // Non-zsh → Bash rules
},
```

**Glob substitution (`GLOB_SUBST`)**:

In zsh-native mode (GLOB_SUBST off), expansion results are not treated as glob patterns. With GLOB_SUBST on, they are:

```rust
pathname_matching: match options {
    Some(opts) if opts.glob_subst.is_definitely_on() => !in_double_quotes,
    Some(opts) if opts.glob_subst.is_definitely_off() => false,
    Some(_) => !in_double_quotes,  // Unknown → conservative
    None => !in_double_quotes,     // Non-zsh → Bash rules
},
```

**Globbing (`GLOB`)**:

When `GLOB` is off, pathname matching never happens on any word:

```rust
// In RuntimeLiteralAnalysis:
pathname_matching: match options {
    Some(opts) if opts.glob.is_definitely_off() => false,
    _ => /* existing glob-char detection */,
},
```

**Array semantics (`KSH_ARRAYS`)**:

With `KSH_ARRAYS` on, `$arr` means `${arr[0]}` (scalar, first element) instead of `${arr[@]}` (array splat). This changes `value_shape` and `array_valued`:

```rust
// When analyzing a plain variable reference that resolves to an array:
if ksh_arrays.is_definitely_on() {
    // $arr is scalar (first element only)
    value_shape: PartValueShape::Scalar,
    array_valued: false,
} else {
    // $arr is array splat (zsh default)
    value_shape: PartValueShape::Array,
    array_valued: true,
}
```

**RC_EXPAND_PARAM**:

With `RC_EXPAND_PARAM` on, `${arr}text` expands element-wise. This affects multi-field analysis for arrays adjacent to literal text.

#### Context-Sensitivity Table (Zsh)

The existing context-sensitivity table (spec 008) assumes Bash semantics. Zsh defaults change several cells:

| Context | Field splitting (zsh default) | Field splitting (SH_WORD_SPLIT on) | Pathname matching (zsh default) | Pathname matching (GLOB_SUBST on) |
|---------|:---:|:---:|:---:|:---:|
| CommandArgument | **no** | yes | **no** | yes |
| CommandName | **no** | yes | **no** | yes |
| ForList | **no** | yes | **no** | yes |
| SelectList | **no** | yes | **no** | yes |
| DeclarationAssignmentValue | **no** | yes | **no** | yes |

Bold cells mark where zsh-native defaults differ from Bash. Other contexts (AssignmentValue, RedirectTarget, CasePattern, etc.) are unchanged because they are not subject to field splitting or glob substitution in either shell.

### Per-Expansion Flag Overrides

Zsh supports per-expansion flags that override the ambient option state for a single expansion. The parser already captures these as `ZshModifier` entries in `ZshParameterExpansion`. The expansion analysis layer interprets the following flags:

| Flag | Modifier char | Effect | Overrides option |
|------|:---:|--------|-----------------|
| `${=var}` | `=` | Force word splitting on this expansion | `SH_WORD_SPLIT` → on |
| `${~var}` | `~` | Force glob substitution on this expansion | `GLOB_SUBST` → on |
| `${^var}` | `^` | Force RC_EXPAND_PARAM on this expansion | `RC_EXPAND_PARAM` → on |
| `${==var}` | `=` (doubled) | Force word splitting off | `SH_WORD_SPLIT` → off |
| `${~~var}` | `~` (doubled) | Force glob substitution off | `GLOB_SUBST` → off |
| `${^^var}` | `^` (doubled) | Force RC_EXPAND_PARAM off | `RC_EXPAND_PARAM` → off |

When `analyze_parameter_part` encounters a `ParameterExpansionSyntax::Zsh(syntax)`, it first scans `syntax.modifiers` for override flags, then constructs an effective option state by overlaying the overrides onto the ambient `ZshOptionState`. The analysis proceeds using the effective state.

This means `${=foo}` triggers field-splitting hazards regardless of whether `SH_WORD_SPLIT` is globally on, and `${~~foo}` suppresses glob-substitution hazards regardless of whether `GLOB_SUBST` is globally on.

### `noglob` Precommand Modifier

`noglob` is a precommand modifier that suppresses globbing for a single simple command:

```zsh
noglob git checkout HEAD -- *.txt   # *.txt is literal, not a glob
```

The parser already recognizes `noglob` during command parsing. The linter's command fact builder records whether a command has the `noglob` modifier. When expansion analysis runs on arguments of a `noglob` command, the effective `GLOB` state is forced to `Off` for pathname-matching hazard computation.

This is modeled as a command-level annotation on the command fact, not as a scope-level option change, because `noglob` affects only the single command it modifies.

### Rule Impact

The following rules change behavior with option-state tracking:

#### S001 (UnquotedExpansion)

Today: flags all unquoted `$var` in argv context regardless of shell.

With option state: in zsh mode with `SH_WORD_SPLIT` definitively off, unquoted scalar expansions in argv context do **not** undergo word splitting and are safe to leave unquoted (unless `GLOB_SUBST` is on, in which case pathname matching is still a hazard). When `SH_WORD_SPLIT` is `Unknown`, the rule conservatively flags the expansion.

The `${=var}` flag explicitly forces splitting, so `${=var}` in argv context is flagged even if `SH_WORD_SPLIT` is off.

#### S003 (UnquotedArrayExpansion)

Today: flags unquoted `${arr[@]}` to preserve element boundaries.

With option state: when `KSH_ARRAYS` is on, `$arr` is a scalar reference (first element), not an array splat. The rule should not flag `$arr` as an unquoted array expansion in `KSH_ARRAYS` mode. Explicit `${arr[@]}` is still an array splat regardless of `KSH_ARRAYS`.

#### C012 (LeadingGlobArgument)

Today: flags `*` or `?` as leading command arguments that might be unintended globs.

With option state: when `GLOB` is definitively off (via `setopt NO_GLOB` or `noglob` modifier), glob characters are literal. The rule should suppress diagnostics for commands under `noglob` or in `NO_GLOB` scopes.

#### K003 (RmGlobOnVariablePath)

Today: flags `rm` with glob patterns on variable paths.

With option state: when `GLOB` is off, the glob is literal and the risk is different. When `NULL_GLOB` is on, unmatched globs expand to nothing (empty argument list), which may be a different risk than the default `NOMATCH` error.

#### X013/X019 (Array Assignment/Reference Portability)

Today: flags Bash-specific array syntax.

With option state: when `KSH_ARRAYS` is on, array behavior matches ksh, so ksh-style portability warnings should be adjusted. When `emulate sh` is active, array syntax is entirely different.

#### Glob-Pattern Rules (C005, EXTENDED_GLOB-sensitive rules)

Today: assume default glob syntax.

With option state: `EXTENDED_GLOB` enables `#`, `~`, `^` as glob operators. `KSH_GLOB` enables `@(...)`, `*(...)` etc. Pattern analysis rules need to know which glob operators are active to correctly classify patterns.

### Validation Harness

Zsh is not part of the ShellCheck-backed compatibility path. ShellCheck does not support zsh, so the large-corpus oracle cannot validate zsh behavior. The existing test infrastructure explicitly excludes zsh from ShellCheck conformance:

```rust
// large_corpus.rs
if shell == "zsh" {
    return false;  // skip ShellCheck comparison
}
```

Validation of zsh option-state behavior requires a `zsh -fc` black-box harness that verifies shuck's analysis matches zsh's actual behavior.

#### Harness Design

The harness is a test fixture runner that:

1. **Takes a zsh script snippet** with embedded assertions about expansion behavior.
2. **Runs the snippet through `zsh -fc`** to observe actual behavior (e.g., how many fields `$var` produces after word splitting).
3. **Runs the same snippet through shuck's expansion analysis** with option-state tracking.
4. **Compares** shuck's predicted hazards against zsh's actual behavior.

Fixture format:

```zsh
# @option SH_WORD_SPLIT=on
# @expect-split $foo 2
foo="a b"
for x in $foo; do echo "$x"; done
```

The harness extracts `@option` directives to configure the option state, `@expect-split` directives to declare expected field counts, and runs both zsh and shuck to verify agreement.

#### Fixture Categories

| Category | What it validates | Example |
|----------|------------------|---------|
| Word splitting | SH_WORD_SPLIT on/off changes field count | `$var` with spaces |
| Glob substitution | GLOB_SUBST on/off changes pathname matching | `$var` containing `*` |
| Array semantics | KSH_ARRAYS changes `$arr` meaning | `$arr` vs `${arr[@]}` |
| Glob control | GLOB/NOMATCH/NULL_GLOB changes glob behavior | `*.nonexistent` |
| Emulate scoping | `emulate -L sh` scopes correctly | Function-local options |
| Per-expansion flags | `${=var}`, `${~var}`, `${^var}` | Override ambient state |
| noglob | `noglob cmd *.txt` suppresses globbing | Precommand modifier |
| Conditional merge | Options set in branches | `if ...; then setopt ...; fi` |

The harness runs as part of `make test` when `zsh` is available on the system (detected at test time). When `zsh` is not available, the harness tests are skipped with a diagnostic message. The harness does not require nix, unlike the ShellCheck large-corpus comparison.

#### Integration with Large Corpus

The existing zsh parse test track in `large_corpus.rs` (`large_corpus_zsh_fixtures_parse`) should be extended to include option-state validation. For each zsh fixture that parses successfully, the harness can additionally verify that option-modifying commands are correctly identified and that the predicted option state at each expansion matches zsh's actual behavior.

This is a separate test target from the ShellCheck conformance corpus:

```bash
# Zsh option-state validation (requires zsh on PATH)
cargo test -p shuck -- zsh_option_state

# Zsh option-state with the large corpus
make test-large-corpus-zsh
```

### Phased Rollout

#### Phase 1: Option State Type and Builder

- Define `ZshOptionState`, `OptionValue`, and `ShellProfile` in `shuck-semantic`.
- Implement `ZshOptionState::zsh_default()`, `for_emulate(mode)`, `apply_setopt(name)`, `apply_unsetopt(name)`.
- Implement zsh option name normalization (case-insensitive, underscore-ignoring, `NO_` prefix inversion).
- Extend the semantic builder to recognize `emulate`, `setopt`, `unsetopt`, `set -o/+o` commands and record option state snapshots.
- Implement scope-aware option tracking with `emulate -L` and `LOCAL_OPTIONS`.
- Implement conditional merge (branch divergence → `Unknown`).
- Add `SemanticModel::zsh_options_at(offset)` query.
- Add unit tests for option state transitions, emulate presets, scope entry/exit, and conditional merging.

#### Phase 2: Expansion Analysis Integration

- Extend `analyze_word()` and `analyze_parameter_part()` to accept `Option<&ZshOptionState>`.
- Implement option-sensitive field splitting, pathname matching, glob substitution, and array semantics.
- Implement per-expansion flag override interpretation (`${=...}`, `${~...}`, `${^...}`).
- Implement `noglob` command annotation and its effect on expansion analysis.
- Update the context-sensitivity table for zsh-specific defaults.
- Extend `Checker` to thread `zsh_options_at()` into expansion analysis calls.
- Add unit tests for each option-sensitive analysis decision.

#### Phase 3: Rule Updates

- Update S001 to use option-aware splitting/globbing analysis.
- Update S003 to account for `KSH_ARRAYS`.
- Update C012 and K003 to account for `GLOB`/`NULL_GLOB`/`noglob`.
- Update X013/X019 and other array-related portability rules for `KSH_ARRAYS`.
- Update pattern analysis for `EXTENDED_GLOB`/`KSH_GLOB`/`SH_GLOB`.
- Audit all zsh-specific portability rules (X043, X044, X051, X076, X078, X079) for option-state sensitivity.

#### Phase 4: Parser Integration

- Implement the lightweight pre-scan for `emulate`/`setopt`/`unsetopt` at lexical level.
- Add `OptionAwareFeatures` layer to the parser.
- Gate `IGNORE_BRACES`/`IGNORE_CLOSE_BRACES` on option state.
- Gate `SHORT_LOOPS`/`SHORT_REPEAT` on option state.
- Gate `RC_QUOTES` on option state.
- Gate `EXTENDED_GLOB`/`KSH_GLOB`/`SH_GLOB` on option state for pattern parsing.
- Add parser tests for grammar changes under different option states.

#### Phase 5: Validation Harness

- Build the `zsh -fc` black-box harness.
- Write fixture suites for each option-sensitive behavior category.
- Integrate with the existing zsh parse test track.
- Add `make test-large-corpus-zsh` target.

## Alternatives Considered

### Alternative A: File-Wide Option Inference Only

Infer one `ZshOptionState` per file from the shebang, file-level `emulate`, and initial `setopt` commands. Do not track option changes within the file.

Rejected because `emulate -L` at function entry is the dominant pattern in real zsh code. A file that defines ten functions, each with `emulate -L sh`, would be analyzed as if the entire file is in zsh-native mode. This produces systematic false positives (flagging safe unquoted expansions in sh-emulated functions) or false negatives (missing hazards in zsh-native functions that follow sh-emulated ones).

### Alternative B: Full CFG-Based Option Dataflow

Run a full forward dataflow analysis on the CFG to compute option state at every program point, handling all branch conditions, loops, and indirect control flow.

Rejected as disproportionate to the problem. Zsh option changes are overwhelmingly at function entry or file top-level. The rare case of conditional `setopt` inside a branch is handled correctly by the conservative merge to `Unknown`. Full dataflow would add complexity to the CFG infrastructure for a marginal improvement in precision on edge cases that barely occur in practice.

### Alternative C: Dynamic Option Tracking in the Parser

Have the parser maintain a mutable option state that updates as it encounters `setopt`/`emulate` commands during the parse, rather than doing a pre-scan.

Rejected because it entangles parsing with semantic analysis. The parser would need to recognize and evaluate `setopt` commands mid-parse, which means it needs command resolution, alias expansion, and scope tracking — all of which belong in the semantic layer. The pre-scan approach keeps the parser's responsibilities clean: it gets a sorted list of `(offset, grammar_change)` pairs and binary-searches them. The semantic builder handles the full complexity.

### Alternative D: Ignore Grammar-Affecting Options in the Parser

Only track options for linting purposes. Keep the parser's grammar fixed per-dialect and accept that some grammar-affecting options (like `IGNORE_BRACES`) will cause parse failures on affected code.

Rejected because `emulate -L sh` inside zsh functions commonly changes grammar-affecting options (`SH_GLOB`, `SHORT_LOOPS`). Ignoring these would cause parse failures on valid zsh code that uses sh-emulated functions — a regression from the current state where the parser at least accepts zsh-native grammar throughout.

### Alternative E: Model Options as Boolean Only (No Tri-State)

Use `bool` instead of `OptionValue` for each option, defaulting to the zsh default when state is ambiguous.

Rejected because conditional option changes are common enough that the analysis needs to represent uncertainty. A bool-only model would be forced to pick one value when both paths are reachable, leading to either false positives or false negatives with no way for rules to distinguish "definitively off" from "might be on." The `Unknown` tri-state lets rules choose their own conservative direction based on what they're checking.

## Security Considerations

Option-state tracking does not introduce new security-sensitive computation. The pre-scan and builder walk the same AST that already undergoes depth and fuel limits. The per-span binary search is O(log n) in the number of option-modifying commands, which is bounded by the total number of commands (already fuel-limited).

The `noglob` annotation must not be spoofable by aliasing or function wrapping. The parser recognizes `noglob` only as a syntactic precommand modifier, not as a resolved command name. If `noglob` is aliased to something else, the parser will not recognize the alias target as `noglob`, and the expansion analysis will not suppress glob hazards — this is the correct conservative behavior.

The `zsh -fc` validation harness executes user-provided test fixtures in a real zsh process. The harness must run only on explicitly marked test fixtures within the repository, never on arbitrary user input. Test fixtures are treated as trusted code.

## Verification

### Unit Tests

- **Option state transitions**: `ZshOptionState` correctly applies `setopt`, `unsetopt`, `emulate` with all recognized option names. Option name normalization handles case, underscores, and `NO_` prefix.
- **Emulate presets**: `for_emulate("sh")`, `for_emulate("ksh")`, `for_emulate("csh")`, `for_emulate("zsh")` produce correct option constellations.
- **Scope tracking**: `emulate -L` scopes to the enclosing function. `LOCAL_OPTIONS` prevents leakage. Options at file scope propagate to functions without `LOCAL_OPTIONS`.
- **Conditional merge**: Options set in one branch of an `if`/`case` produce `Unknown` after the join point.
- **Expansion analysis**: `analyze_word` with `SH_WORD_SPLIT=Off` reports no field-splitting hazard on unquoted `$var`. With `SH_WORD_SPLIT=On`, reports field-splitting hazard. With `SH_WORD_SPLIT=Unknown`, reports field-splitting hazard (conservative).
- **Per-expansion flags**: `${=var}` forces field splitting regardless of ambient state. `${~~var}` suppresses glob substitution regardless of ambient state.
- **noglob annotation**: Arguments of `noglob cmd` report no pathname-matching hazard.
- **KSH_ARRAYS**: `$arr` is classified as scalar when `KSH_ARRAYS` is on, array when off.

### Integration Tests

- **S001 with emulate -L sh**: Inside a function with `emulate -L sh`, unquoted `$var` is flagged (SH_WORD_SPLIT is on in sh mode). Outside the function, unquoted `$var` is not flagged (zsh default, SH_WORD_SPLIT off).
- **S003 with KSH_ARRAYS**: `$arr` is not flagged as unquoted array when `KSH_ARRAYS` is on.
- **C012 with noglob**: `noglob rm *.txt` does not flag `*.txt` as a leading glob.
- **Parser with IGNORE_BRACES**: Code after `setopt IGNORE_BRACES` does not parse `{a,b}` as brace expansion.
- **Mixed function modes**: A file with both `emulate -L sh` and `emulate -L zsh` functions applies correct option state to each.

### Validation Harness

- `zsh -fc` harness agrees with shuck on field splitting for all fixture scripts.
- `zsh -fc` harness agrees with shuck on glob expansion for all fixture scripts.
- `zsh -fc` harness agrees with shuck on array indexing for all fixture scripts.
- `zsh -fc` harness agrees with shuck on `emulate -L` scoping for all fixture scripts.

```bash
# Option state unit tests
cargo test -p shuck-semantic -- zsh_option
cargo test -p shuck-linter -- zsh_option

# Expansion analysis with options
cargo test -p shuck-linter -- expansion

# Rule-level tests
cargo test -p shuck-linter -- unquoted_expansion
cargo test -p shuck-linter -- unquoted_array_expansion
cargo test -p shuck-linter -- leading_glob

# zsh validation harness (requires zsh)
cargo test -p shuck -- zsh_option_state

# Full workspace
make test
```
