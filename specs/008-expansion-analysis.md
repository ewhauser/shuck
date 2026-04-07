# 008: Expansion Analysis

## Status

Implemented

## Summary

A shared expansion analysis layer within `shuck-linter` that answers context-sensitive questions about shell word expansion: what expansions occur, what hazards they carry, whether values are safe to leave unquoted, and how redirect targets and command substitutions behave. The layer replaces per-rule expansion heuristics with a unified set of classifiers that rules consume through a small query API.

This spec documents the system as built. It supersedes the incremental project plan in `docs/EXPANSION_ENHANCEMENTS.md`, which tracked the implementation work.

## Motivation

Shell expansion is context-sensitive. The same word `$x` behaves differently as a command argument (subject to field splitting and pathname matching), a redirect target (expanded once, no splitting), a `case` pattern (interpreted as a glob), and a `[[ =~ ]]` operand (interpreted as a regex). A linter that treats all of these the same will either miss real problems or produce false positives.

Before this system existed, each rule reimplemented its own expansion reasoning:

- `S001` (unquoted expansion) maintained a local safe-value allowlist that could not share knowledge with other rules.
- `C055` (pattern with variable) scanned raw operand text for `$` characters instead of asking the AST whether an expansion was present.
- Test-expression rules (`TruthyLiteralTest`, `ConstantComparisonTest`, `QuotedBashRegex`) each had a different notion of "fixed literal" that disagreed on tilde, glob, and brace-bearing words.
- Redirect rules (`C057`, `C058`) could not distinguish `/dev/null` sinks from runtime-sensitive targets without duplicating redirect analysis.

The expansion analysis layer solves this by providing one set of answers that all rules share. Rules ask questions ("is this word safe to leave unquoted in argv context?", "does this redirect definitely target /dev/null?"), and the layer computes the answer by combining AST structure, quoting state, expansion shape, context rules, and semantic binding information.

## Design

### Architecture Overview

The system is organized into three analytical layers, each building on the one below:

```
 Rules (S001, C055, C057, ...)
   |
   v
 +-----------------------------------------+
 | Safety Judgment Layer (safe_value.rs)    |  "Is this value safe here?"
 |  SafeValueIndex, SafeValueQuery         |
 +-----------------------------------------+
   |
   v
 +-----------------------------------------+
 | Word Analysis Layer (expansion.rs)      |  "What expansion happens here?"
 |  ExpansionAnalysis, RedirectTarget-      |
 |  Analysis, SubstitutionClassification,  |
 |  RuntimeLiteralAnalysis                 |
 +-----------------------------------------+
   |
   v
 +-----------------------------------------+
 | Context Enumeration Layer               |  "Where do expansions occur?"
 |  (query.rs, span.rs, word.rs)           |
 |  visit_expansion_words, span helpers,   |
 |  classify_word, static_word_text        |
 +-----------------------------------------+
   |
   v
 AST (shuck-ast Word, WordPart, Redirect, ...)
```

Rules combine the layers: they iterate contexts with `visit_expansion_words`, analyze words with expansion analysis functions, check safety with `SafeValueIndex`, and locate diagnostic spans with span helpers.

All expansion types live in `crates/shuck-linter/src/rules/common/`. They are internal to the linter crate and not part of any public API.

### Layer 1: Context Enumeration

**Files:** `query.rs`, `span.rs`, `word.rs`

This layer answers "where do expansions occur in this script?" and provides convenience wrappers for word classification.

#### ExpansionContext

The `ExpansionContext` enum classifies where a word appears in shell syntax. It is the key input that makes all subsequent analysis context-sensitive.

```rust
pub enum ExpansionContext {
    CommandName,                        // The command being invoked
    CommandArgument,                    // A positional argument
    AssignmentValue,                    // RHS of x=value
    DeclarationAssignmentValue,         // RHS of declare x=value
    RedirectTarget(RedirectKind),       // Target of > >> < etc.
    DescriptorDupTarget(RedirectKind),  // Target of >&N, <&N
    HereString,                         // <<< word
    ForList,                            // for x in WORDS
    SelectList,                         // select x in WORDS
    CasePattern,                        // case ... in PATTERN)
    StringTestOperand,                  // [[ x == y ]]
    RegexOperand,                       // [[ x =~ y ]]
    ConditionalVarRefSubscript,         // subscript within [[ ]]
    ParameterPattern,                   // ${x#pat}, ${x//pat/repl}
    TrapAction,                         // trap 'action' SIGNAL
}
```

**When to use:** Every rule that reasons about expansion should know the context it is analyzing. The context determines which hazards apply (field splitting does not apply in `[[ ]]` string tests), which runtime sensitivities matter (tilde expansion does not apply in case patterns), and what "safe" means (a value safe in argv context may be unsafe in a regex operand).

#### visit_expansion_words

The `visit_expansion_words` function iterates all words in a command that undergo shell expansion, paired with their context. It is the primary entry point for rules that need to examine expansion across all positions in a command.

```rust
pub fn visit_expansion_words(
    command: &Command,
    source: &str,
    callback: impl FnMut(&Word, ExpansionContext),
);
```

It covers command names, arguments, assignment values, redirect targets, loop/case/select/trap words, and conditional operands. Rules that only care about specific contexts can filter within the callback.

#### WordClassification and classify_word

`WordClassification` is a simplified view of expansion analysis for rules that need basic word categorization without the full `ExpansionAnalysis` record:

```rust
pub struct WordClassification {
    pub quote: WordQuote,              // FullyQuoted | Mixed | Unquoted
    pub literalness: WordLiteralness,  // FixedLiteral | Expanded
    pub expansion_kind: WordExpansionKind,      // None | Scalar | Array | Mixed
    pub substitution_shape: WordSubstitutionShape,  // None | Plain | Mixed
}
```

`classify_word(word, source)` produces a `WordClassification`. `classify_contextual_operand(word, source, context)` goes further, combining expansion analysis with runtime literal analysis to return a `TestOperandClass` that distinguishes truly fixed literals from runtime-sensitive ones.

#### static_word_text

`static_word_text(word, source)` extracts the literal text of a word if it contains no expansions. Returns `None` for any word that contains variables, command substitutions, or other dynamic parts. Used by redirect analysis to check for `/dev/null`, by safe-value checking to validate literal content, and by rules that need to inspect the actual text of literal operands.

#### Span Helpers

`span.rs` provides functions that locate specific expansion parts within a word, used by rules to anchor diagnostics on the relevant expansion rather than the entire word:

- `command_substitution_part_spans` / `unquoted_command_substitution_part_spans` — find `$(...)` and `` `...` `` parts
- `array_expansion_part_spans` / `unquoted_array_expansion_part_spans` — find `${arr[@]}` and similar
- `expansion_part_spans` — all expansion parts
- `scalar_expansion_part_spans` — expansion parts excluding array splats and command substitutions

### Layer 2: Word Analysis

**File:** `expansion.rs`

This layer answers "what expansion happens in this word?" by walking the AST `WordPart` tree and aggregating facts about quoting, value shape, hazards, and substitution structure.

#### ExpansionAnalysis

The primary output of word analysis. Produced by `analyze_word(word, source)`.

```rust
pub struct ExpansionAnalysis {
    pub quote: WordQuote,
    pub literalness: WordLiteralness,
    pub value_shape: ExpansionValueShape,
    pub substitution_shape: WordSubstitutionShape,
    pub hazards: ExpansionHazards,
    pub array_valued: bool,
    pub can_expand_to_multiple_fields: bool,
}
```

Each field captures a distinct aspect of expansion behavior:

| Field | What it answers |
|-------|----------------|
| `quote` | Is the word fully quoted, partially quoted, or unquoted? |
| `literalness` | Does the word contain any expansions at all? |
| `value_shape` | What shape does the expanded value have? (None, Scalar, Array, MultiField, Unknown) |
| `substitution_shape` | Does the word contain command substitutions? Is it a plain `$(cmd)` or mixed with other parts? |
| `hazards` | What runtime hazards does this expansion carry? |
| `array_valued` | Does the expansion produce an array value (e.g., `${arr[@]}`)? |
| `can_expand_to_multiple_fields` | Can the expansion produce multiple argv fields after splitting? |

**When to use:** Any time a rule needs to understand the expansion behavior of a word. Most rules will use `classify_word` (Layer 1) for simple queries and drop down to `analyze_word` when they need the full record — particularly hazard flags, value shape, or the array-valued distinction.

#### ExpansionHazards

Bit-flag struct tracking which shell expansion hazards a word carries:

```rust
pub struct ExpansionHazards {
    pub field_splitting: bool,          // $x in unquoted context
    pub pathname_matching: bool,        // Glob expansion on unquoted result
    pub tilde_expansion: bool,          // ~ at word start or after = or :
    pub brace_fanout: bool,             // {a,b} or {1..5} in literal text
    pub runtime_pattern: bool,          // ${x#pat} where pattern is dynamic
    pub command_or_process_substitution: bool,  // $(cmd) or <(cmd)
    pub arithmetic_expansion: bool,     // $(( expr ))
}
```

These hazards are computed per-word-part and aggregated. Quoting suppresses some hazards: a `$x` inside double quotes does not carry `field_splitting` or `pathname_matching`. The analysis correctly tracks the quoting context of each part.

#### ExpansionValueShape

Classifies the runtime value shape of an expansion:

| Variant | Meaning | Example |
|---------|---------|---------|
| `None` | No expansion | `hello` |
| `Scalar` | Single scalar value | `$x`, `${x}`, `$(cmd)` |
| `Array` | Array value (preserves elements) | `"${arr[@]}"` (quoted, so no split) |
| `MultiField` | Can produce multiple argv fields | `${arr[@]}` (unquoted), `${!prefix@}` |
| `Unknown` | Cannot determine statically | `${!x}` (indirect) |

The distinction between `Array` and `MultiField` matters for rules like `S008` (unquoted array expansion): `"${arr[@]}"` is array-valued but does not produce multiple fields because quoting preserves element boundaries, while unquoted `${arr[@]}` both is array-valued and can produce multiple fields.

#### RuntimeLiteralAnalysis

Some words are syntactically literal (no `$`, no backticks) but still not fixed at runtime because the shell performs additional expansions on literal text in certain contexts. `analyze_literal_runtime(word, source, context)` detects these cases:

```rust
pub struct RuntimeLiteralAnalysis {
    pub runtime_sensitive: bool,
    pub hazards: ExpansionHazards,
}
```

It scans for:

- **Tilde expansion**: `~` at word start, or after `=` or `:` in contexts where tilde expansion applies (argv, assignments, redirects, test operands — but not case patterns or parameter patterns).
- **Pathname matching**: `*`, `?`, `[` in contexts where globbing applies (argv, declaration assignments, redirects — but not `[[ ]]` string tests or case patterns).
- **Brace fanout**: `{a,b}` or `{1..5}` in contexts where brace expansion applies (argv, assignments, redirects — but not case patterns or test operands).

Each hazard is gated by a context check (`context_allows_tilde`, `context_allows_pathname_matching`, `context_allows_brace_fanout`) that encodes where the shell actually performs that expansion. This prevents false positives from flagging `*.sh` in a case pattern where globbing does not apply.

**When to use:** Rules that need to distinguish truly fixed literals from runtime-sensitive ones. `classify_contextual_operand` in Layer 1 wraps this for the common case, but rules that need the specific hazard flags should call `analyze_literal_runtime` directly.

#### RedirectTargetAnalysis

Produced by `analyze_redirect_target(redirect, source)`. Combines word expansion analysis with redirect-specific classification:

```rust
pub struct RedirectTargetAnalysis {
    pub kind: RedirectTargetKind,              // File or DescriptorDup
    pub dev_null_status: Option<RedirectDevNullStatus>,  // Definitely, DefinitelyNot, Maybe
    pub numeric_descriptor_target: Option<i32>,  // For dup targets: the fd number
    pub expansion: ExpansionAnalysis,
    pub runtime_literal: RuntimeLiteralAnalysis,
}
```

**When to use:** Rules that analyze redirects, particularly `C057` and `C058` (redirected substitution rules). The `/dev/null` classification is especially important: it determines whether a command substitution's stdout is being discarded, which changes whether the substitution is useful.

#### SubstitutionClassification

Produced by `classify_substitution(substitution, source)`. Walks all commands inside a command substitution to determine what happens to stdout:

```rust
pub struct SubstitutionClassification {
    pub kind: CommandSubstitutionKind,
    pub span: Span,
    pub stdout_intent: SubstitutionOutputIntent,  // Captured | Discarded | Rerouted | Mixed
    pub has_stdout_redirect: bool,
}
```

The classifier tracks file descriptor state across all commands in the substitution, following redirect chains including `>&N` dup targets. For example:

- `$(printf hi)` → `Captured` (default, no redirects)
- `$(printf hi > out.txt)` → `Rerouted` (stdout goes to a file)
- `$(printf hi >/dev/null 2>&1)` → `Discarded` (stdout goes to /dev/null)
- `$(whiptail 3>&1 1>&2 2>&3)` → `Captured` (fd swap, stdout still returns to capture)
- `$(cmd1 > out.txt || cmd2 >&2)` → `Mixed` (different commands have different intents)

**When to use:** Rules that detect useless or suspicious command substitutions. If stdout is `Discarded` or `Rerouted`, the substitution's captured value is likely empty or unintended.

### Layer 3: Safety Judgment

**File:** `safe_value.rs`

This layer answers "is this expansion safe to leave unquoted in this context?" by combining expansion analysis with semantic binding information.

#### SafeValueQuery

Maps an expansion context to a safety question:

```rust
pub enum SafeValueQuery {
    Argv,           // Safe in command argument position (no field split, no glob)
    RedirectTarget, // Safe as a redirect target
    Pattern,        // Safe in a pattern context (no pattern metacharacters)
    Regex,          // Safe in a regex context (no regex metacharacters)
    Quoted,         // Always safe (the expansion is already quoted)
}
```

Each query defines what "safe" means for literal content:

- `Argv` / `RedirectTarget`: no whitespace, no glob characters (`*`, `?`, `[`)
- `Pattern`: no pattern metacharacters (`*`, `?`, `[`, `]`, `|`, `(`, `)`)
- `Regex`: no regex metacharacters (`.`, `[`, `(`, `{`, `*`, `+`, `?`, `|`, `^`, `$`, `\`)
- `Quoted`: always safe (the value is in a quoted context)

#### SafeValueIndex

A per-file semantic index that recursively checks whether variable bindings produce safe values:

```rust
pub struct SafeValueIndex<'a> {
    semantic: &'a SemanticModel,
    source: &'a str,
    scalar_bindings: FxHashMap<SpanKey, &'a Word>,
    maybe_uninitialized_refs: FxHashSet<SpanKey>,
    memo: FxHashMap<(SpanKey, SafeValueQuery), bool>,
    visiting: FxHashSet<(SpanKey, SafeValueQuery)>,
}
```

Built once per file via `SafeValueIndex::build(semantic, commands, source)`. Rules then call `word_is_safe(word, query)` or `part_is_safe(part, span, query)` to check individual values.

The index handles:

- **Integer-declared variables**: `declare -i x` is always safe (produces a number).
- **Arithmetic assignments**: `(( x = 1 + 2 ))` results are always safe.
- **Recursive binding analysis**: If `x=$y` and `y` is safe, then `x` is safe. Cycle detection prevents infinite recursion.
- **Special parameters**: `$?`, `$#`, `$$`, `$!`, `$-` are always safe (numeric or single-character).
- **Uninitialized references**: Variables that may not be initialized are treated as unsafe.
- **Parameter operators**: `${x:-default}` is safe if both `x` and `default` are safe. `${x:+replacement}` is safe if `replacement` is safe. `${x#pattern}` is safe if `x` is safe.
- **Transformation operators**: `${x@Q}` (shell-quoted) is always safe. `${x@K}` and `${x@k}` are always safe. Other transformations inherit the safety of the underlying variable.
- **Indirect expansions**: `${!x}` is safe if all of x's indirect targets are safe.
- **Array access**: `${arr[0]}` follows scalar safety. `${arr[@]}` is only safe in `Quoted` context (it produces multiple fields otherwise).

**When to use:** Primarily consumed by `S001` (unquoted expansion), which needs to avoid false positives on variables that are provably safe. Also usable by any future rule that needs to distinguish "this unquoted expansion is actually fine" from "this unquoted expansion could break."

### How Rules Consume the System

Rules follow a common pattern:

1. **Build indexes** — If the rule needs safe-value analysis, call `SafeValueIndex::build` once for the file.
2. **Iterate contexts** — Use `visit_expansion_words` to iterate all words in each command, paired with their `ExpansionContext`.
3. **Classify words** — Call `classify_word` for basic queries or `analyze_word` for the full record.
4. **Check safety** — If the rule needs to determine whether an unquoted expansion is acceptable, call `part_is_safe` or `word_is_safe` with the appropriate `SafeValueQuery`.
5. **Anchor diagnostics** — Use span helpers to locate the specific expansion part to highlight, rather than flagging the entire word.

#### Example: S001 (Unquoted Expansion)

```
for each command in file:
  build SafeValueIndex from semantic model
  visit_expansion_words(command):
    if context is not relevant: skip
    classify_word(word):
      if not scalar expansion: skip
    for each expansion part span:
      if part_is_safe(part, Argv): skip   // e.g., integer variable
      emit diagnostic at part span
```

#### Example: C055 (Pattern With Variable)

```
visit_expansion_words(command):
  if context != ParameterPattern: skip
  if classify_word(word).is_expanded():
    emit diagnostic at word span
```

#### Example: TruthyLiteralTest

```
for each [[ test ]] expression:
  if classify_contextual_operand(operand, StringTestOperand).is_fixed_literal():
    check for truthy-literal test pattern
```

### Context-Sensitivity Rules

The following table summarizes which shell expansion behaviors apply in each context. These rules are encoded in the `context_allows_*` functions and in the per-part hazard computation:

| Context | Field splitting | Pathname matching | Tilde expansion | Brace expansion |
|---------|:-:|:-:|:-:|:-:|
| CommandName | yes | yes | yes | yes |
| CommandArgument | yes | yes | yes | yes |
| AssignmentValue | no | no | yes | yes |
| DeclarationAssignmentValue | yes | yes | yes | yes |
| RedirectTarget | no | yes | yes | yes |
| DescriptorDupTarget | no | no | no | no |
| HereString | no | no | no | no |
| ForList | yes | yes | yes | yes |
| SelectList | yes | yes | yes | yes |
| CasePattern | no | no | no | no |
| StringTestOperand | no | no | yes | no |
| RegexOperand | no | no | yes | no |
| ParameterPattern | no | no | no | no |
| TrapAction | no | no | no | no |

These rules reflect Bash semantics. The "Field splitting" and "Pathname matching" columns describe what happens to unquoted expansions. "Tilde expansion" and "Brace expansion" describe what happens to literal text in those positions.

## Alternatives Considered

### Alternative A: Per-Rule Expansion Heuristics

Keep each rule implementing its own expansion checks. This was the original state. Rejected because the same questions recur across style, correctness, and substitution rules, leading to inconsistent answers and duplicated maintenance.

### Alternative B: Full Shell Expansion Engine

Build a complete expansion interpreter that executes tilde expansion, parameter expansion, field splitting, and pathname matching. Rejected because the linter does not need to produce expanded values — it needs to classify expansion behavior. A classifier is simpler, faster, and does not require a runtime environment.

### Alternative C: Expansion Analysis in the Parser

Move expansion classification into `shuck-parser` or `shuck-ast` so it is available to all consumers, not just the linter. Rejected for now because expansion analysis depends on linter-specific concepts (semantic bindings, safe-value queries, rule-specific context models) that do not belong in the parser. The analysis is a linter concern layered on top of parser-provided AST structure.

### Alternative D: Dataflow-Based Safety Analysis

Use the semantic model's reaching-definition and dataflow infrastructure to track value shapes through the entire program, instead of a per-binding safe-value index. Rejected as premature: the current approach of checking the nearest visible binding is sufficient for the rules that exist today and avoids the complexity of whole-program value tracking. This could be revisited if rules need cross-function or cross-scope safety reasoning.

## Verification

The expansion analysis layer is verified at three levels:

- **Unit tests** in `expansion.rs`: Test `analyze_word`, `analyze_literal_runtime`, `analyze_redirect_target`, and `classify_substitution` directly with parsed shell fragments. Cover array vs. scalar shape, quoting effects, redirect /dev/null classification, descriptor dup tracking, and fd-swap patterns.

- **Snapshot and fixture tests** per consumer rule: Each rule that uses the expansion system has its own test suite validating that the combined analysis produces correct diagnostics for known patterns.

- **Large corpus conformance**: The expansion system's accuracy is validated indirectly through the large corpus comparison against ShellCheck for affected rules:
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=S001`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=S004,S008`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C048,C055`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C057,C058`

Verifying a new rule that consumes the expansion system:

1. `cargo test -p shuck-linter -- <rule_test_name>` — rule-level tests pass.
2. `cargo test -p shuck-linter` — no regressions across the linter.
3. `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=<rule_code>` — corpus conformance holds.
