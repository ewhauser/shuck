# Expansion Enhancements

## Status

Proposed

## Summary

This document turns the current expansion-related linter gaps into a shared implementation plan. The goal is to improve rule accuracy by teaching the linter more about shell expansion context, value shape, and runtime sensitivity.

The proposal is intentionally scoped to helper infrastructure and rule precision. It is not a plan to build a general-purpose shell executor inside the linter.

After comparing this plan against the AST work captured in `docs/AST_ENHANCEMENTS.md`, this document should now be read as the AST-aware follow-on plan:

- some current expansion projects become simpler once richer AST shapes exist
- one project becomes mainly a temporary bridge if AST pattern work is delayed
- the remaining long-term work is the runtime and context-sensitive reasoning that AST shape alone will not solve

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
- Prefer consuming richer AST shape when it exists rather than rebuilding syntax from spans and source text.
- Improve accuracy for existing rules without rebuilding parser concerns in linter code.
- Keep rollout incremental so each helper can land with concrete consumer rules.
- Preserve room for later parser and AST work without making this document depend on it.

## Non-Goals

- Redesigning the AST or adding new word-part node types.
- Building a full shell expansion interpreter inside the linter.
- Requiring exact shell-runtime parity for every edge case in the first pass.
- Replacing focused unit and snapshot tests with only corpus-based checks.
- Duplicating runtime-prelude work already tracked in [`docs/rules.md`](./rules.md).
- Re-implementing AST enhancements in linter helpers when the parser can preserve the same facts directly.

## Relationship To AST Enhancements

The AST plan in `docs/AST_ENHANCEMENTS.md` changes the shape of this roadmap.

- Quote-aware word parts and syntax-form preservation would simplify Projects 1, 2, and 6 by replacing a large share of today’s quoting and syntax-form recovery work.
- First-class pattern AST and typed `[[ ... ]]` operands would largely absorb Project 3 and simplify Projects 1, 2, and 5.
- First-class `VarRef`, typed `Subscript`, and compound-array nodes would simplify Projects 2 and 6 by replacing current index-string heuristics.
- Heredoc delimiter metadata is mostly orthogonal to this document. It may improve future redirect and heredoc rules, but it does not replace the current expansion projects.
- Structured arithmetic AST would simplify Projects 2 and 6 by replacing opaque arithmetic text with typed reads, writes, and operands.

The practical consequence is:

- AST priorities 1 through 3 should land before most of the rule-facing expansion migrations in this document.
- Project 3 should be treated as a transitional bridge if pattern AST work is delayed.
- Projects 1, 2, 4, 5, and 6 still matter after the AST work, but they become more semantic and less stringly.

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

This project still matters after AST changes. Richer AST shape reduces syntax recovery, but it does not remove the need to normalize expansion behavior by shell context.

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

- [x] Add a common expansion-context type and thread it through linter traversal helpers.
- [x] Replace rule-local assumptions about "argument-like" behavior with explicit context checks.
- [x] Add traversal tests that prove the same word can be classified differently in argument, redirect, pattern, and regex positions.

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

If the AST priorities in `docs/AST_ENHANCEMENTS.md` land first, this classifier should consume those richer nodes directly:

- quote-aware word parts instead of a word-level quoted heuristic
- pattern and regex operands instead of generic `Word`
- `VarRef` and typed `Subscript` nodes instead of string-matching selectors
- structured arithmetic expressions instead of opaque source slices

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

- [x] Introduce a new expansion analysis record under `rules/common`.
- [x] Re-implement `classify_word` on top of the new analysis rather than local `WordPart` shortcuts.
- [x] Distinguish "array-valued" from "can expand to multiple argv fields".
- [x] Add regression tests for `${arr[@]}`, `${arr[*]}`, `${!prefix@}`, indirect expansions, and transformation operators.

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

### Project 3: Transitional SourceText Operand Analyzer

Many expansion-bearing constructs still store operands as `SourceText`, but first-class pattern AST and typed conditional operands already landed. That leaves a much smaller bridge: keep a source-backed helper for the remaining flattened operands and rebase any nested expansion spans back onto the original source.

Proposed additions:

- a helper that reparses source-backed `SourceText` as a shell word and reports nested expansion spans in original coordinates
- focused coverage for escaped dollars, quoted dollars, nested substitutions, and mixed literals in operand text
- source-backed span mapping so diagnostics still point at the original operand text

The original pattern-walking use case is already covered by the AST-backed path, so this project now exists mainly as a bridge for any remaining flattened operands.

Action items:

- [x] Add a source-backed `SourceText` analysis helper in the common expansion layer.
- [x] Add tests for escaped dollars, quoted dollars, nested substitutions, and mixed literals inside source-backed operand text.
- [x] Delete or fold this bridge into Project 2 once the last flattened operand callers disappear.

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

This project is mostly unchanged by the AST roadmap. Better AST shape can improve anchoring and classification inputs, but redirect target semantics remain runtime- and context-sensitive.

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

- [x] Add a redirect-target classifier under `rules/common`.
- [x] Teach the classifier about descriptor duplication versus file redirection.
- [x] Model "not statically literal" separately from "definitely not `/dev/null`".
- [x] Add regressions for redirected substitutions, numeric dup targets, and words that may fan out into multiple redirect fields.

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

If pattern AST and quote-aware word parts land first, this project narrows to words that are still syntactically literal in the AST but runtime-sensitive in shell evaluation.

Important examples:

- leading `~` in contexts where tilde expansion applies
- assignment-like tilde segments such as `PATH=~/bin`
- unquoted glob characters in contexts where pathname matching applies
- brace-style fanout candidates that are still stored as raw words

The goal is not to fully execute these features. The goal is to stop misclassifying these words as permanently fixed.

Action items:

- [x] Add a source-based runtime-sensitivity scanner for words that remain literal in the AST.
- [x] Make the scanner context-aware so tilde, glob, and brace sensitivity only apply where the shell would use them.
- [x] Add focused tests for `~`, `~user`, `x=~`, `*.sh`, and `{a,b}`-style words.

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

This project should wait for the richer AST foundations where possible:

- quote-aware word parts
- first-class pattern and conditional operands
- `VarRef` / `Subscript` / array nodes
- structured arithmetic, when arithmetic facts are part of the safety question

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

- [x] Refactor `SafeValueIndex` to depend on the new expansion analysis layer.
- [x] Replace literal-only safe checks with context-specific safe queries.
- [x] Add recursion and cycle tests for binding analysis involving indirect and transformed values.
- [x] Add focused `S001` tests that cover known-safe integers, known-safe literals, and unsafe multi-field expansions.

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

1. AST quote-aware word parts and syntax-form preservation
2. AST pattern AST and typed `[[ ... ]]` operands
3. AST `VarRef` / typed `Subscript` / compound-array nodes
4. Expansion Projects 1 and 2 on top of the richer AST
5. `C055` and `CasePatternVar`
6. `TruthyLiteralTest`, `ConstantComparisonTest`, and `QuotedBashRegex`
7. `C057` and `C058`
8. AST structured arithmetic
9. `S008`, then `S004`, then `S001`

Rationale:

- AST priorities 1 through 3 remove the highest-value syntax-recovery work from the linter and make the follow-on expansion helpers cleaner.
- `C055` and `CasePatternVar` are still the best first rule consumers once pattern-aware AST data exists.
- The test and regex rules validate the new fixed-literal and runtime-sensitivity helpers after the AST stops flattening their operands.
- The redirected-substitution rules validate redirect-target semantics, which the AST work does not solve by itself.
- `S008` and `S004` validate shared value-shape analysis after array and quoting structure become less heuristic.
- `S001` should still land last so it can consume the stable version of the helper layer rather than forcing premature API choices.

gbash references:

- use the project-specific source and test files listed above when implementing each migration
- when a migrated rule still feels under-specified, start from the nearest `shell/expand/*_test.go` or `internal/shell/interp/*_test.go` coverage rather than broad code reading

## Rollout Plan

### Phase 1: AST Foundations

- Land AST priorities 1 through 3 from `docs/AST_ENHANCEMENTS.md`.
- Treat Project 3 in this document as temporary bridge work only if AST pattern work is delayed.
- Revisit the shared expansion helper APIs after the richer AST shapes are available.

### Phase 2: Shared Expansion Infrastructure

- Build Projects 1 and 2 on top of the richer AST.
- Land small consumer migrations for `C055` and `CasePatternVar`.
- Keep helper APIs narrow until at least two rule families share them.

### Phase 3: Runtime Sensitivity and Test Precision

- Build Project 5.
- Migrate `TruthyLiteralTest`, `ConstantComparisonTest`, and `QuotedBashRegex`.
- Add direct tests for tilde, glob, and brace-like literals.

### Phase 4: Redirect Semantics

- Build Project 4.
- Re-run `C057` and `C058` after every redirect-classifier change.
- Confirm that substitution-intent changes improve signal rather than only shifting span choices.

### Phase 5: Style Rule Adoption

- Land structured arithmetic AST if safe-value and arithmetic facts still need it.
- Build Project 6.
- Migrate `S008`, then `S004`, then `S001`.
- Delete duplicated expansion heuristics once the shared helpers become the only path.

## Alternatives Considered

### Alternative A: Keep Extending `classify_word` and `static_word_text`

Rejected because the current problems are mostly about missing context and value-shape information. Adding more one-off booleans to the existing helpers would keep the same ambiguity, just with more branches.

### Alternative B: Do Only AST Redesign And Defer Expansion Helper Work

Rejected because richer AST shape still does not solve context normalization, redirect target semantics, runtime-sensitive literal classification, or `S001` safety reasoning by itself.

### Alternative C: Build a Full Expansion Engine for the Linter

Rejected because the linter does not need to execute the shell to benefit from stronger expansion modeling. A classifier-oriented helper layer is a better fit for rule precision and maintenance cost.

### Alternative D: Fix Each Rule Independently

Rejected because the same expansion questions already recur across style, correctness, and substitution rules. Per-rule patches would keep reintroducing slightly different notions of "literal", "array", and "safe".

## Risks

- A shared expansion layer may become too broad if it tries to answer every future shell question at once.
- Project 3 may linger longer than intended if AST pattern work is delayed, leaving temporary bridge logic in place.
- Source-based runtime-sensitivity scanning may drift if it is not validated with focused tests.
- Redirect-target classification could overfit to current substitution rules unless its API stays generic.
- `S001` may pressure the design toward premature complexity if migrated before smaller consumers harden the helper layer.

## Verification

For each project and migrated rule:

- [ ] For AST-backed changes, land parser-level shape tests first and then simplify the corresponding expansion helpers.
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
