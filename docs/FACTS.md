# Linter Fact Creation Roadmap

## Status

Proposed

## Summary

This document proposes a linter-side fact creation layer that is built once per
file and then reused by the implemented rules in `shuck-linter`.

Shuck already has the right precedent for this approach. The semantic model in
`crates/shuck-semantic` performs a single traversal and emits reusable facts for
scopes, bindings, references, call sites, source references, flow contexts, and
CFG-backed dataflow. The linter still performs many separate AST walks after
that point, especially for command, word, test, loop, pipeline, redirect, and
surface-syntax checks.

The roadmap below turns those repeated rule-local walks into a shared
`LinterFacts` layer. The goal is not to remove all rule-specific logic. The
goal is to move repeated structural discovery into one reusable fact builder so
rules can become cheap filters over facts instead of independent tree rescans.

## Motivation

The current architecture repeats the same work in many places:

- `Checker` still dispatches most command-oriented rules one by one from
  `crates/shuck-linter/src/checker.rs`.
- `rules/common/query.rs` provides a shared walker, but many rules still call it
  independently and traverse the whole file again.
- `rules/common/command.rs` normalizes commands repeatedly inside individual
  rules instead of once per command.
- `rules/common/word.rs` and `rules/common/expansion.rs` reclassify the same
  words across multiple rules.
- `rules/common/safe_value.rs` builds another command walk to recover scalar
  bindings for `S001`.
- `C005` and `C046` still maintain bespoke recursive walkers instead of
  consuming shared facts.

This duplication shows up in several clear families:

- command identity and wrapper peeling
- declaration and option-shape normalization
- expansion-context discovery
- word and operand classification
- loop and pipeline shape detection
- redirect and command-substitution intent analysis
- surface-syntax anchoring for literals and fragment spans

The semantic model proves that a one-pass fact builder is a good fit for this
codebase. The missing piece is a linter-focused fact layer that sits beside
`Indexer` and `SemanticModel` rather than forcing every command- and word-level
question back through rule-local walkers.

## Goals

- Build command and word-oriented facts once per file.
- Replace repeated rule-local AST walks with shared fact queries.
- Keep expensive structural discovery out of individual rules.
- Reuse existing `Indexer` and `SemanticModel` primitives instead of duplicating
  them.
- Centralize the highest-traffic helper families: command normalization, word
  classification, test normalization, loop and pipeline shapes, redirect and
  substitution classification, and surface fragment anchors.
- Retire bespoke walkers where the common fact layer can answer the question.

## Non-Goals

- Rewriting the parser or moving linter logic into `shuck-parser`.
- Forcing every rule into `shuck-semantic`.
- Eliminating all rule-specific matching logic.
- Changing rule codes, rule categories, or user-facing diagnostics as part of
  the fact migration alone.
- Solving unrelated parity problems that are primarily parser or policy issues.

## Design

### Current State

Today the implemented rules split into two broad groups.

Rules that already read shared semantic facts:

- `C001` `UnusedAssignment`
- `C002` `DynamicSourcePath`
- `C006` `UndefinedVariable`
- `C014` `LocalTopLevel`
- `C063` `OverwrittenFunction`
- `C124` `UnreachableAfterExit`

Rules that still do one or more linter-side walks:

- `S001` `UnquotedExpansion`
- `S002` `ReadWithoutRaw`
- `S003` `LoopFromCommandOutput`
- `S004` `UnquotedCommandSubstitution`
- `S005` `LegacyBackticks`
- `S006` `LegacyArithmeticExpansion`
- `S007` `PrintfFormatVariable`
- `S008` `UnquotedArrayExpansion`
- `S009` `EchoedCommandSubstitution`
- `S010` `ExportCommandSubstitution`
- `C005` `SingleQuotedLiteral`
- `C008` `TrapStringExpansion`
- `C009` `QuotedBashRegex`
- `C010` `ChainedTestBranches`
- `C011` `LineOrientedInput`
- `C007` `FindOutputToXargs`
- `C013` `FindOutputLoop`
- `C015` `SudoRedirectionOrder`
- `C017` `ConstantComparisonTest`
- `C018` `LoopControlOutsideLoop`
- `C019` `LiteralUnaryStringTest`
- `C020` `TruthyLiteralTest`
- `C021` `ConstantCaseSubject`
- `C022` `EmptyTest`
- `C025` `PositionalTenBraces`
- `C046` `PipeToKill`
- `C047` `InvalidExitStatus`
- `C048` `CasePatternVar`
- `C050` `ArithmeticRedirectionTarget`
- `C055` `PatternWithVariable`
- `C057` `SubstWithRedirect`
- `C058` `SubstWithRedirectErr`

Several of those rules ask nearly identical questions. That overlap is the
roadmap target.

### Proposed Layer

Add a linter-owned fact container, tentatively named `LinterFacts`, that is
built once from:

- `Script`
- source text
- `Indexer`
- `SemanticModel`
- file context

The intent is:

```text
parse -> indexer -> semantic model -> linter facts -> rules
```

`LinterFacts` should hold reusable structural summaries, not diagnostics. Rules
remain responsible for policy and wording, but they should stop rediscovering
the same command and word structure independently.

### Initiative 1: `LinterFacts` Container And Builder

Create the container and builder before migrating rules so the rest of the work
has one home.

Action items:

- [x] Add `crates/shuck-linter/src/facts.rs` or an equivalent module tree for
      linter-owned facts.
- [x] Build `LinterFacts` once per file inside `Checker` construction or the
      first `check()` phase.
- [x] Expose read-only fact accessors from `Checker`.
- [x] Use stable keys for lookups.
  The first choice should be spans or lightweight IDs produced during the fact
  build, not cloned AST subtrees.
- [x] Keep semantic-first rules on `SemanticModel` when it already answers the
      question cleanly.
- [x] Define which facts belong in `LinterFacts` versus `SemanticModel`.
  Command and word classification should stay linter-side unless a semantic pass
  needs the same information.
- [x] Move `SafeValueIndex` off its independent command walk and onto shared
      fact inputs.

### Initiative 2: Command Facts

Several rules repeatedly call `normalize_command()` and then inspect the same
derived shape: effective name, wrappers, declaration family, body words, and
selected option semantics.

The command fact family should precompute:

- literal command name
- effective command name
- wrapper chain
- declaration family
- body span
- body words and argument slices
- common option summaries for high-traffic commands

Action items:

- [x] Precompute normalized command facts for every command node.
- [x] Record declaration facts for `export`, `local`, `declare`, and `typeset`.
- [x] Add option summaries for:
  `read`, `printf`, `unset`, `find`, `xargs`, `exit`, `sudo`, `doas`, `run0`.
- [x] Record helper booleans for frequent queries.
  Examples: "uses `-r`", "has `-print0`", "uses null input", "targets a
  function unset", "effective command is `tee`".
- [x] Preserve command body span and argument spans so rules do not rebuild them
      by hand.
- [x] Convert rules that currently normalize commands locally.

Rules to migrate onto command facts:

- [x] `S002` `ReadWithoutRaw`
- [x] `S007` `PrintfFormatVariable`
- [x] `S009` `EchoedCommandSubstitution`
- [x] `S010` `ExportCommandSubstitution`
- [x] `C013` `FindOutputLoop`
- [x] `C015` `SudoRedirectionOrder`
- [x] `C046` `PipeToKill`
- [x] `C047` `InvalidExitStatus`
- [x] `C057` `SubstWithRedirect`
- [x] `C058` `SubstWithRedirectErr`
- [x] `C063` `OverwrittenFunction`
  Only the remaining `unset` scan; the overwrite detection itself already comes
  from semantic facts.

### Initiative 3: Word And Expansion Facts

This is the highest-leverage migration area. Many rules independently walk
expansion words and then call `analyze_word()`, `classify_word()`, or
`classify_contextual_operand()` again.

The word fact family should precompute:

- expansion context for each relevant word occurrence
- quote state
- literalness
- scalar vs array expansion shape
- command-substitution shape
- runtime-sensitive literal classification by context
- reusable part-level anchor spans

Action items:

- [ ] Precompute expansion-word facts for command name, command argument,
      assignment value, declaration assignment value, redirect target, here
      string, loop headers, case patterns, conditional operands, trap action,
      and parameter patterns.
- [ ] Cache `ExpansionAnalysis` per fact instead of recomputing it per rule.
- [ ] Cache contextual operand classification for string tests, regex operands,
      and redirect targets.
- [ ] Record part-level anchor spans for:
  scalar expansions, array expansions, command substitutions, backticks, legacy
  arithmetic expansions, and single-quoted fragments.
- [ ] Extend the fact builder to retain expansion facts for subscript words so
      `S004` does not need a second local traversal path.
- [ ] Rework `SafeValueIndex` to consume shared expansion facts and shared
      scalar-binding facts.

Rules to migrate onto word and expansion facts:

- [ ] `S001` `UnquotedExpansion`
- [ ] `S003` `LoopFromCommandOutput`
- [ ] `S004` `UnquotedCommandSubstitution`
- [ ] `S005` `LegacyBackticks`
- [ ] `S006` `LegacyArithmeticExpansion`
- [ ] `S008` `UnquotedArrayExpansion`
- [ ] `S009` `EchoedCommandSubstitution`
- [ ] `S010` `ExportCommandSubstitution`
- [ ] `C005` `SingleQuotedLiteral`
- [ ] `C008` `TrapStringExpansion`
- [ ] `C009` `QuotedBashRegex`
- [ ] `C011` `LineOrientedInput`
- [ ] `C013` `FindOutputLoop`
- [ ] `C021` `ConstantCaseSubject`
- [ ] `C025` `PositionalTenBraces`
- [ ] `C048` `CasePatternVar`
- [ ] `C055` `PatternWithVariable`

### Initiative 4: Test And Conditional Facts

`C017`, `C019`, `C020`, and `C022` all normalize `test` / `[` / `[[ ... ]]`
forms separately and then ask closely related operand questions.

The test fact family should precompute:

- simple-test shape for `test` and `[ ... ]`
- operand list
- operator family
- conditional expression kind for `[[ ... ]]`
- fixed-literal vs runtime-sensitive operand classification
- rule-relevant gating context

Action items:

- [x] Add a `SimpleTestFact` that captures `test` and `[ ... ]` operands without
      each rule re-deriving them.
- [x] Add a `ConditionalFact` for `[[ ... ]]` that records unary, binary,
      regex, pattern, and bare-word shapes.
- [x] Cache operand classification for each operand in its effective context.
- [x] Carry file-context gates needed by `C022`, including ShellSpec parameter
      block suppression.
- [x] Convert rules that currently call `simple_test_operands()` and then do
      more local operand analysis.

Rules to migrate onto test and conditional facts:

- [x] `C009` `QuotedBashRegex`
- [x] `C017` `ConstantComparisonTest`
- [x] `C019` `LiteralUnaryStringTest`
- [x] `C020` `TruthyLiteralTest`
- [x] `C022` `EmptyTest`

### Initiative 5: Loop, List, And Pipeline Facts

Loop headers, pipeline segment identity, and `&&` / `||` list shape are each
rediscovered independently today.

The loop and pipeline fact family should precompute:

- `for` and `select` header words
- whether a loop header contains command substitution
- whether a loop header contains `find`-driven substitution
- pipeline segment facts with normalized command names
- list operator chains and mixed short-circuit patterns
- loop depth or direct flow facts where semantic data already exists

Action items:

- [x] Add `ForHeaderFact` and `SelectHeaderFact`.
- [x] Add `PipelineFact` with normalized segment identity.
- [x] Add `ListFact` for `&&` / `||` operator sequences.
- [x] Decide whether loop-depth queries should move to linter facts or reuse
      `SemanticModel::flow_context_at`.
      Reused semantic flow context for loop-control checks instead of
      duplicating loop depth in `LinterFacts`.
- [x] Convert loop, list, and pipeline rules away from generic `walk_commands`
      scans.

Rules to migrate onto loop, list, and pipeline facts:

- [x] `S003` `LoopFromCommandOutput`
- [x] `C010` `ChainedTestBranches`
- [x] `C011` `LineOrientedInput`
- [x] `C013` `FindOutputLoop`
- [x] `C018` `LoopControlOutsideLoop`
- [x] `C046` `PipeToKill`
- [x] `C007` `FindOutputToXargs`

### Initiative 6: Redirect And Substitution Facts

Redirect and substitution classification helpers already exist, but they still
perform repeated work. In particular, command substitutions are classified by
walking the nested commands again for each consumer.

The redirect and substitution fact family should precompute:

- redirect target analysis
- descriptor-dup vs file-target shape
- `/dev/null` certainty
- arithmetic-expansion hazard on redirect targets
- command-substitution stdout intent
- whether substitution stdout is captured, discarded, or rerouted

Action items:

- [ ] Precompute redirect analysis for every redirect target once.
- [ ] Precompute command-substitution classification once per substitution span.
- [ ] Attach substitution facts to the containing word fact or command fact.
- [ ] Reuse redirect facts for both direct redirect rules and substitution
      rules.
- [ ] Remove rule-local nested substitution rescans.

Rules to migrate onto redirect and substitution facts:

- [ ] `C015` `SudoRedirectionOrder`
- [ ] `C050` `ArithmeticRedirectionTarget`
- [ ] `C057` `SubstWithRedirect`
- [ ] `C058` `SubstWithRedirectErr`
- [ ] `S004` `UnquotedCommandSubstitution`
  For the substitution inventory inside subscript words and nested words.

### Initiative 7: Surface Trivia And Literal Fragment Facts

Two rules stand out as the clearest signs that the current shared walker layer
is still insufficient:

- `C005` `SingleQuotedLiteral`
- `C046` `PipeToKill`

Both keep bespoke recursive walks to recover a mix of literal fragments and
structural shape. Smaller surface-sensitive rules also benefit from shared
fragment facts.

The surface-trivia fact family should precompute:

- single-quoted fragment spans with local rule context tags
- positional-parameter fragment spans for `${10}`-style checks
- backtick fragment spans
- legacy arithmetic fragment spans
- static command-name facts for simple utility checks such as `kill`

Action items:

- [ ] Add fragment facts for single-quoted spans and the rule-relevant local
      context that affects whether they should report.
- [ ] Add positional-parameter fragment facts so `C025` no longer scans parts
      itself.
- [ ] Expose static utility-name facts for lightweight command queries such as
      `kill`.
- [ ] Retire the bespoke walker in `C005`.
- [ ] Retire the bespoke walker in `C046`.

Rules to migrate onto surface trivia and literal fragment facts:

- [ ] `S005` `LegacyBackticks`
- [ ] `S006` `LegacyArithmeticExpansion`
- [ ] `C005` `SingleQuotedLiteral`
- [ ] `C025` `PositionalTenBraces`
- [ ] `C046` `PipeToKill`

### Initiative 8: Checker Integration And Cleanup

The final value of this work is not just new helpers. It is a simpler
`Checker::check()` that routes rules toward facts instead of repeated walkers.

Action items:

- [ ] Group rules by fact family inside `Checker` rather than by raw AST access
      pattern alone.
- [ ] Remove fact-building logic from individual rules once the shared builder
      owns it.
- [ ] Delete duplicated helper code that becomes obsolete.
- [ ] Keep semantic-only rules unchanged unless a fact migration clearly
      simplifies them.
- [ ] Benchmark before and after the first large migration cluster to confirm
      that the fact layer is reducing total walk volume rather than just moving
      it around.

## Rule Migration Checklist

Already primarily fact-backed through `SemanticModel`:

- [x] `C001` `UnusedAssignment`
- [x] `C002` `DynamicSourcePath`
- [x] `C006` `UndefinedVariable`
- [x] `C014` `LocalTopLevel`
- [x] `C124` `UnreachableAfterExit`

Partially fact-backed and still worth tightening:

- [ ] `C063` `OverwrittenFunction`

Needs migration to `LinterFacts`:

- [ ] `S001` `UnquotedExpansion`
- [ ] `S002` `ReadWithoutRaw`
- [x] `S003` `LoopFromCommandOutput`
- [ ] `S004` `UnquotedCommandSubstitution`
- [ ] `S005` `LegacyBackticks`
- [ ] `S006` `LegacyArithmeticExpansion`
- [ ] `S007` `PrintfFormatVariable`
- [ ] `S008` `UnquotedArrayExpansion`
- [ ] `S009` `EchoedCommandSubstitution`
- [ ] `S010` `ExportCommandSubstitution`
- [ ] `C005` `SingleQuotedLiteral`
- [ ] `C008` `TrapStringExpansion`
- [x] `C009` `QuotedBashRegex`
- [x] `C010` `ChainedTestBranches`
- [x] `C011` `LineOrientedInput`
- [x] `C013` `FindOutputLoop`
- [ ] `C015` `SudoRedirectionOrder`
- [x] `C017` `ConstantComparisonTest`
- [x] `C018` `LoopControlOutsideLoop`
- [x] `C019` `LiteralUnaryStringTest`
- [x] `C020` `TruthyLiteralTest`
- [ ] `C021` `ConstantCaseSubject`
- [x] `C022` `EmptyTest`
- [ ] `C025` `PositionalTenBraces`
- [x] `C046` `PipeToKill`
- [ ] `C047` `InvalidExitStatus`
- [ ] `C048` `CasePatternVar`
- [ ] `C050` `ArithmeticRedirectionTarget`
- [ ] `C055` `PatternWithVariable`
- [ ] `C057` `SubstWithRedirect`
- [ ] `C058` `SubstWithRedirectErr`
- [x] `C007` `FindOutputToXargs`

## Suggested Rollout

### Phase 1: Skeleton

- [ ] Add `LinterFacts` and wire it through `Checker`.
- [ ] Move `SafeValueIndex` dependency planning into the fact design.
- [ ] Add focused unit tests for fact identity and lookup stability.

### Phase 2: High-Traffic Facts

- [ ] Implement command facts.
- [ ] Implement word and expansion facts.
- [ ] Migrate the style-rule cluster first.
  Target: `S001`, `S002`, `S003`, `S004`, `S005`, `S006`, `S007`, `S008`,
  `S009`, `S010`.

### Phase 3: Structured Correctness Facts

- [ ] Implement test and conditional facts.
- [ ] Implement loop, list, and pipeline facts.
- [ ] Implement redirect and substitution facts.
- [ ] Migrate the corresponding correctness rules.

### Phase 4: Bespoke Walker Retirement

- [ ] Replace `C005` with shared fragment facts.
- [ ] Replace `C046` with shared pipeline and utility facts.
- [ ] Remove dead traversal helpers that were only kept alive by those rules.

### Phase 5: Cleanup And Measurement

- [ ] Audit remaining rule-local walks and classify each one.
  Keep it local only if the query is truly rule-specific.
- [ ] Compare linter benchmark results before and after the migration.
- [ ] Update developer docs if new fact builders become the preferred extension
      point for future rule work.

## Alternatives Considered

### Alternative A: Continue Fixing Each Rule Independently

Rejected because the duplication is already obvious in the implemented rule
set. Continuing per-rule fixes would preserve repeated command normalization,
repeated word classification, repeated test normalization, and repeated full
walks.

### Alternative B: Move All Linter Facts Into `shuck-semantic`

Rejected because several of the missing capabilities are linter policy facts
rather than semantic facts. Command wrappers, quote shape, literal fragments,
and specific option summaries are useful to the linter even when the semantic
model does not need them.

### Alternative C: Keep Building Facts Lazily Inside Individual Rules

Rejected because the current code already demonstrates the downside of that
approach. Lazy rule-local builders tend to become one-off walkers that are not
reused by the next rule asking the same structural question.

## Verification

We should treat this roadmap as successful when the implementation shows both
correctness and structural simplification.

Verification checklist:

- [ ] Add unit tests for each fact family builder.
- [ ] Keep existing per-rule regression tests green in `crates/shuck-linter`.
- [ ] Add at least one direct test per migrated rule that proves the rule is
      reading shared facts rather than a rule-local bespoke traversal path.
- [ ] Add benchmark comparisons using the existing linter benchmark target in
      `crates/shuck-benchmark/benches/linter.rs`.
- [ ] Re-audit `Checker` after each phase and confirm repeated whole-file walks
      have decreased.
