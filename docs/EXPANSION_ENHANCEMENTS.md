# Expansion Enhancements

## Status

Proposed

## Summary

This document turns the current expansion-related linter gaps into a shared implementation plan. The goal is to improve rule accuracy by teaching the linter more about shell expansion context, value shape, and runtime sensitivity without waiting for broader AST redesign work.

The proposal is intentionally scoped to helper infrastructure and rule precision. It is not a plan to build a general-purpose shell executor inside the linter.

## Motivation

Several current rules rely on expansion reasoning, but the shared helpers are still mostly syntactic.

- [`crates/shuck-linter/src/rules/common/word.rs`](../crates/shuck-linter/src/rules/common/word.rs) classifies words largely from `WordPart` shape and simple source slicing.
- [`crates/shuck-linter/src/rules/style/unquoted_expansion.rs`](../crates/shuck-linter/src/rules/style/unquoted_expansion.rs) and [`crates/shuck-linter/src/rules/style/unquoted_array_expansion.rs`](../crates/shuck-linter/src/rules/style/unquoted_array_expansion.rs) need better scalar-vs-array and field-splitting precision.
- [`crates/shuck-linter/src/rules/correctness/pattern_with_variable.rs`](../crates/shuck-linter/src/rules/correctness/pattern_with_variable.rs) scans raw operand text for `$` instead of asking whether a pattern operand is actually expansion-bearing.
- [`crates/shuck-linter/src/rules/correctness/case_pattern_var.rs`](../crates/shuck-linter/src/rules/correctness/case_pattern_var.rs), [`crates/shuck-linter/src/rules/correctness/truthy_literal_test.rs`](../crates/shuck-linter/src/rules/correctness/truthy_literal_test.rs), [`crates/shuck-linter/src/rules/correctness/constant_comparison_test.rs`](../crates/shuck-linter/src/rules/correctness/constant_comparison_test.rs), and [`crates/shuck-linter/src/rules/correctness/quoted_bash_regex.rs`](../crates/shuck-linter/src/rules/correctness/quoted_bash_regex.rs) all depend on a stronger notion of "fixed literal" versus "runtime-sensitive".
- Redirect classification in [`crates/shuck-linter/src/rules/common/word.rs`](../crates/shuck-linter/src/rules/common/word.rs) still depends on literal redirect targets, which leaves redirected-substitution rules with avoidable ambiguity.

These gaps show up across both style and correctness rules:

- `S001`, `S004`, and `S008` need more accurate expansion shape and hazard modeling.
- `C048` and `C055` need better pattern-operand analysis.
- `C057` and `C058` need better redirect-target semantics.
- Test and regex rules need to stop treating tilde, glob, and brace-sensitive words as fixed literals when the shell does not.

## Goals

- Make expansion reasoning context-sensitive instead of word-shape-only.
- Reuse one helper layer across rules instead of repeating local expansion heuristics.
- Improve accuracy for existing rules without blocking on AST redesign.
- Keep rollout incremental so each helper can land with concrete consumer rules.
- Preserve room for later parser and AST work without making this document depend on it.

## Non-Goals

- Redesigning the AST or adding new word-part node types.
- Building a full shell expansion interpreter inside the linter.
- Requiring exact shell-runtime parity for every edge case in the first pass.
- Replacing focused unit and snapshot tests with only corpus-based checks.
- Duplicating runtime-prelude work already tracked in [`docs/rules.md`](./rules.md).

## Local gbash Reference Checkout

This plan assumes a local `gbash` checkout at `/Users/ewhauser/working/gbash`.

Use that tree as a behavior and decomposition reference while implementing these projects:

- prefer reading the smallest relevant source file and test file pair before writing helper logic
- use the tests to identify shell-facing behavior and edge cases
- reauthor code, comments, and diagnostics in Rust rather than copying Go structure or wording mechanically

High-value reference files:

| Area | gbash source | gbash tests |
| --- | --- | --- |
| expansion entrypoints and context-sensitive behavior | `shell/expand/expand.go` | `shell/expand/expand_test.go`, `shell/expand/helper_test.go` |
| parameter expansion, array shape, indirection, transforms | `shell/expand/param.go`, `shell/expand/varref.go` | `shell/expand/param_test.go`, `shell/expand/varref_test.go`, `internal/shell/interp/varref_test.go` |
| brace expansion | `shell/expand/braces.go` | `shell/expand/braces_test.go` |
| pattern, regex, and parameter-pattern structure | `shell/syntax/nodes.go`, `shell/expand/param.go` | `shell/expand/param_test.go`, `shell/expand/expand_test.go` |
| redirect target and descriptor-dup behavior | `shell/expand/expand.go`, `internal/shell/interp/runner.go` | `internal/shell/interp/fds_test.go`, `shell/expand/helper_test.go` |
| IFS splitting and read-style field parsing | `shell/expand/expand.go` | `shell/expand/read_fields_test.go` |
| tilde, assignment-like tilde, startup-home behavior | `shell/expand/expand.go` | `shell/expand/expand_test.go`, `internal/shell/interp/varref_test.go` |
| extglob and glob option behavior | `shell/expand/expand.go`, `shell/syntax/nodes.go`, `internal/shell/interp/runner.go` | `shell/expand/expand_test.go`, `internal/shell/interp/shopt_test.go` |
| special-parameter and runtime-sensitive expansion behavior | `shell/expand/param.go`, `shell/expand/arith.go` | `internal/shell/interp/special_vars_test.go`, `shell/expand/arith_test.go` |

## Design

### Project 1: Expansion Context Matrix

Create a shared model for where expansion is being interpreted, because the same word is treated differently in command arguments, redirect targets, test operands, patterns, and trap strings.

Proposed additions:

- `ExpansionContext` enum in a new common helper module such as `crates/shuck-linter/src/rules/common/expansion.rs`
- shared query helpers that pair each visited word with an expansion context
- dedicated contexts for:
  - command arguments
  - assignment values
  - redirect targets
  - descriptor-dup redirect targets
  - here-strings
  - `for`/`select` lists
  - `case` patterns
  - `[[ ... ]]` string-test operands
  - `[[ ... =~ ... ]]` regex operands
  - parameter-pattern operands such as `${x#pat}` and `${x//pat/repl}`
  - trap action strings

Action items:

- [ ] Add a common expansion-context type and thread it through linter traversal helpers.
- [ ] Replace rule-local assumptions about "argument-like" behavior with explicit context checks.
- [ ] Add traversal tests that prove the same word can be classified differently in argument, redirect, pattern, and regex positions.

gbash references:

- source: `shell/expand/expand.go` for the split between `FieldsSeq`, `RedirectFields`, `DupFields`, `LiteralNoTilde`, `PatternNoTilde`, and `RegexpNoTilde`
- tests: `shell/expand/expand_test.go`, `shell/expand/helper_test.go`, and `internal/shell/interp/fds_test.go` for context-specific behavior around argv, redirects, and duplication targets

Rules to migrate first:

| Rules | Why |
| --- | --- |
| `S001`, `S004`, `S008` | Expansion hazards differ between argv and redirect contexts. |
| `C048` | `case` patterns should not be treated like plain arguments. |
| `C055` | Parameter-pattern operands need a separate context from ordinary words. |
| `QuotedBashRegex` | `=~` operands need regex-specific handling. |
| `TrapStringExpansion` | Trap action strings are evaluated at a different time than command arguments. |

### Project 2: Expansion Shape and Hazard Classifier

Add a richer shared classifier that answers not only whether a word is expanded, but also what kind of runtime behavior it can trigger.

Proposed classifier output:

- value shape: `None`, `Scalar`, `Array`, `MultiField`, `Unknown`
- substitution shape: plain substitution, mixed word, or none
- hazard flags:
  - field splitting
  - pathname matching
  - tilde expansion
  - brace-style fanout
  - runtime-sensitive pattern content
  - command or process substitution
- fixed-literal status under a given context

This should replace the current narrow shape checks in [`classify_word`](../crates/shuck-linter/src/rules/common/word.rs) and become the single source of truth for expansion-sensitive rules.

Action items:

- [ ] Introduce a new expansion analysis record under `rules/common`.
- [ ] Re-implement `classify_word` on top of the new analysis rather than local `WordPart` shortcuts.
- [ ] Distinguish "array-valued" from "can expand to multiple argv fields".
- [ ] Add regression tests for `${arr[@]}`, `${arr[*]}`, `${!prefix@}`, indirect expansions, and transformation operators.

gbash references:

- source: `shell/expand/param.go`, `shell/expand/varref.go`, and `shell/expand/expand.go`
- tests: `shell/expand/param_test.go`, `shell/expand/varref_test.go`, and `internal/shell/interp/varref_test.go`

Rules to migrate first:

| Rules | Why |
| --- | --- |
| `S001` | Needs precise field-splitting and globbing hazard checks. |
| `S004` | Needs more reliable plain-vs-mixed substitution detection. |
| `S008` | Needs array-vs-scalar precision rather than index-string heuristics. |
| `CasePatternVar` | Needs to distinguish literal pattern text from runtime-built patterns. |

### Project 3: SourceText Operand Analyzer

Many expansion-bearing constructs still store operands as `SourceText`. The linter should stop treating that source as opaque text and instead analyze it with shell-aware helpers.

Proposed additions:

- a helper that inspects `SourceText` for nested expansions with quote and escape awareness
- dedicated helpers for:
  - parameter-pattern operands
  - replacement-pattern operands
  - replacement-text operands
  - substring and slice expressions where needed
- source-backed span mapping so diagnostics still point at the original operand text

This project should explicitly replace the raw `$` scan in [`pattern_with_variable.rs`](../crates/shuck-linter/src/rules/correctness/pattern_with_variable.rs).

Action items:

- [ ] Add `SourceText` analysis helpers to the common expansion layer.
- [ ] Replace raw byte scans with source-aware operand inspection.
- [ ] Add tests for escaped dollars, quoted dollars, nested substitutions, and mixed literals inside parameter-pattern operands.

gbash references:

- source: `shell/expand/param.go` and `shell/syntax/nodes.go`
- tests: `shell/expand/param_test.go` and targeted cases in `shell/expand/expand_test.go`

Rules to migrate first:

| Rules | Why |
| --- | --- |
| `C055` | Current implementation is text-scanning instead of shell-aware. |
| `S001` | Safe-value analysis often depends on `SourceText` operands in parameter expansions. |
| future pattern-sensitive rules | The helper should be reusable beyond the current backlog. |

### Project 4: Redirect and Substitution Target Semantics

Redirect targets need their own helper layer because redirect expansion follows different shell rules from normal argv expansion, and descriptor-dup redirects are different again.

Proposed additions:

- redirect-target classification:
  - fixed file target
  - known `/dev/null` sink
  - descriptor-dup target
  - ambiguous multi-field target
  - runtime-sensitive target
- context-aware tilde and pathname sensitivity for redirect targets
- a redirect helper that answers whether a command substitution's stdout is captured, discarded, rerouted, or uncertain after target expansion semantics are considered

This should extend the current substitution-intent logic rather than replace it.

Action items:

- [ ] Add a redirect-target classifier under `rules/common`.
- [ ] Teach the classifier about descriptor duplication versus file redirection.
- [ ] Model "not statically literal" separately from "definitely not `/dev/null`".
- [ ] Add regressions for redirected substitutions, numeric dup targets, and words that may fan out into multiple redirect fields.

gbash references:

- source: `shell/expand/expand.go` and `internal/shell/interp/runner.go`
- tests: `internal/shell/interp/fds_test.go` and `shell/expand/helper_test.go`

Rules to migrate first:

| Rules | Why |
| --- | --- |
| `C057`, `C058` | Both rely on correct redirect-target interpretation inside substitutions. |
| `S004` | Nested substitution classification becomes more reliable with stronger redirect semantics. |
| `ArithmeticRedirectionTarget` | Redirect-target context should be shared instead of rule-local. |

### Project 5: Runtime-Sensitive Literal Classifier

Some words are syntactically literal but still not fixed literals at runtime. The linter should model those cases directly.

Important examples:

- leading `~` in contexts where tilde expansion applies
- assignment-like tilde segments such as `PATH=~/bin`
- unquoted glob characters in contexts where pathname matching applies
- brace-style fanout candidates that are still stored as raw words
- extglob-like patterns that the current AST does not model directly

The goal is not to fully execute these features. The goal is to stop misclassifying these words as permanently fixed.

Action items:

- [ ] Add a source-based runtime-sensitivity scanner for words that remain literal in the AST.
- [ ] Make the scanner context-aware so tilde, glob, and brace sensitivity only apply where the shell would use them.
- [ ] Add focused tests for `~`, `~user`, `x=~`, `*.sh`, `{a,b}`, and `+(foo)`-style words.

gbash references:

- source: `shell/expand/expand.go` and `shell/expand/braces.go`
- tests: `shell/expand/braces_test.go`, `shell/expand/expand_test.go`, `internal/shell/interp/varref_test.go`, and `internal/shell/interp/shopt_test.go`

Rules to migrate first:

| Rules | Why |
| --- | --- |
| `TruthyLiteralTest` | Should not warn on words that are runtime-sensitive despite literal AST shape. |
| `ConstantComparisonTest` | Needs a stronger notion of "fixed literal". |
| `QuotedBashRegex` | Regex-literal checks should separate truly fixed text from runtime-sensitive text. |
| `CasePatternVar` | Pattern literals should remain distinct from runtime-built patterns. |

### Project 6: Safe Value Index v2

`S001` already has a local safe-value index, but it is still shaped by simplified expansion assumptions. It should be rebuilt on top of the new shared analysis layer.

Proposed additions:

- context-specific "field-safe" queries built on expansion analysis instead of hand-coded part allowlists
- recursive binding analysis that understands:
  - integer-like safe scalars
  - runtime prelude and shell-provided special parameters
  - array-vs-scalar distinctions
  - indirect expansions and prefix matches
  - transformation operators
- stable answers for:
  - safe in argv
  - safe in pattern context
  - safe in regex context
  - safe only when quoted

This work should build on the runtime-prelude and semantic infrastructure already proposed in [`docs/rules.md`](./rules.md) rather than duplicate it.

Action items:

- [ ] Refactor `SafeValueIndex` to depend on the new expansion analysis layer.
- [ ] Replace literal-only safe checks with context-specific safe queries.
- [ ] Add recursion and cycle tests for binding analysis involving indirect and transformed values.
- [ ] Add focused `S001` tests that cover known-safe integers, known-safe literals, and unsafe multi-field expansions.

gbash references:

- source: `shell/expand/param.go`, `shell/expand/varref.go`, and `shell/expand/arith.go`
- tests: `shell/expand/param_test.go`, `internal/shell/interp/varref_test.go`, and `internal/shell/interp/special_vars_test.go`

Rules to migrate first:

| Rules | Why |
| --- | --- |
| `S001` | Primary consumer and proving ground. |
| future expansion rules | Shared safe-value reasoning should be reusable once stabilized. |

### Project 7: First Consumer Migrations

The helper layer should be rolled out through a few narrow, high-signal migrations before broad sweeps.

Recommended order:

1. `C055` and `CasePatternVar`
2. `TruthyLiteralTest`, `ConstantComparisonTest`, and `QuotedBashRegex`
3. `C057` and `C058`
4. `S008`
5. `S004`
6. `S001`

Rationale:

- `C055` and `CasePatternVar` are small consumers with clear win conditions.
- The test and regex rules validate the new fixed-literal and runtime-sensitivity helpers.
- The redirected-substitution rules validate redirect-target semantics.
- `S008` and `S004` validate shared value-shape analysis.
- `S001` should land last so it can consume the stable version of the helper layer rather than forcing premature API choices.

gbash references:

- use the project-specific source and test files listed above when implementing each migration
- when a migrated rule still feels under-specified, start from the nearest `shell/expand/*_test.go` or `internal/shell/interp/*_test.go` coverage rather than broad code reading

## Rollout Plan

### Phase 1: Shared Expansion Infrastructure

- Build Projects 1 through 3.
- Land small consumer migrations for `C055` and `CasePatternVar`.
- Keep helper APIs narrow until at least two rule families share them.

### Phase 2: Runtime Sensitivity and Test Precision

- Build Project 5.
- Migrate `TruthyLiteralTest`, `ConstantComparisonTest`, and `QuotedBashRegex`.
- Add direct tests for tilde, glob, brace-like, and extglob-like literals.

### Phase 3: Redirect Semantics

- Build Project 4.
- Re-run `C057` and `C058` after every redirect-classifier change.
- Confirm that substitution-intent changes improve signal rather than only shifting span choices.

### Phase 4: Style Rule Adoption

- Build Project 6.
- Migrate `S008`, then `S004`, then `S001`.
- Delete duplicated expansion heuristics once the shared helpers become the only path.

## Alternatives Considered

### Alternative A: Keep Extending `classify_word` and `static_word_text`

Rejected because the current problems are mostly about missing context and value-shape information. Adding more one-off booleans to the existing helpers would keep the same ambiguity, just with more branches.

### Alternative B: Wait for AST Redesign First

Rejected because several high-value improvements do not require new node types. Context-aware analysis of existing `Word`, `WordPart`, and `SourceText` structures should already unlock better accuracy.

### Alternative C: Build a Full Expansion Engine for the Linter

Rejected because the linter does not need to execute the shell to benefit from stronger expansion modeling. A classifier-oriented helper layer is a better fit for rule precision and maintenance cost.

### Alternative D: Fix Each Rule Independently

Rejected because the same expansion questions already recur across style, correctness, and substitution rules. Per-rule patches would keep reintroducing slightly different notions of "literal", "array", and "safe".

## Risks

- A shared expansion layer may become too broad if it tries to answer every future shell question at once.
- Source-based runtime-sensitivity scanning may drift if it is not validated with focused tests.
- Redirect-target classification could overfit to current substitution rules unless its API stays generic.
- `S001` may pressure the design toward premature complexity if migrated before smaller consumers harden the helper layer.

## Verification

For each project and migrated rule:

- [ ] Add focused unit tests for the shared helper being introduced.
- [ ] Add snapshot or fixture coverage for each consumer rule.
- [ ] Run `cargo test -p shuck-linter`.
- [ ] Run targeted corpus checks for the affected rule set:
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C048,C055`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C057,C058`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=S004,S008`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=S001`
- [ ] Confirm that helper-driven changes reduce false positives and false negatives rather than only shifting spans.
- [ ] Update the corresponding `docs/bugs/*.md` files when a project materially changes the remaining backlog for a rule.
