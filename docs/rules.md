# Linter Helper Refactor Platform

## Status

Proposed

## Summary

This document turns the repeated bug patterns in `docs/bugs/` into a shared refactor plan for Shuck's linter and semantic layers. The goal is to replace rule-by-rule parity work with reusable helpers for traversal, command normalization, span anchoring, suppression handling, runtime semantics, helper-file context, and corpus noise classification.

The proposal is based on the current bug backlog rather than on a greenfield redesign. The desired outcome is that future rule work happens on top of a stable helper platform instead of each rule reimplementing its own walkers, span choices, and policy exceptions.

## Motivation

The current bug backlog is highly repetitive.

- Parser or shell-detection noise appears in most bug reports.
- Helper files, project-closure scripts, wrappers, ShellSpec fixtures, and generated scripts appear across a large share of the backlog.
- Several rules miss nested command-substitution and declaration contexts for the same structural reason.
- Many `location-only` diffs are caused by per-rule span selection rather than by semantic disagreement.
- Several suppression bugs are repeated across unrelated rules.
- The largest semantic gaps are concentrated in a few shared semantic capabilities: runtime variables, interprocedural reads, branch joins, guarded exits, and dynamic indirection.

The current architecture makes these problems easy to repeat:

- [crates/shuck-linter/src/checker.rs](crates/shuck-linter/src/checker.rs) dispatches rules one by one with little shared query infrastructure.
- [crates/shuck-linter/src/rules/style/syntax.rs](crates/shuck-linter/src/rules/style/syntax.rs) and [crates/shuck-linter/src/rules/correctness/syntax.rs](crates/shuck-linter/src/rules/correctness/syntax.rs) already duplicate traversal and syntax helpers.
- Span selection and suppression compatibility are handled inconsistently across rules.
- The semantic model already contains useful primitives in [crates/shuck-semantic/src/builder.rs](crates/shuck-semantic/src/builder.rs), [crates/shuck-semantic/src/dataflow.rs](crates/shuck-semantic/src/dataflow.rs), [crates/shuck-semantic/src/cfg.rs](crates/shuck-semantic/src/cfg.rs), and [crates/shuck-semantic/src/source_closure.rs](crates/shuck-semantic/src/source_closure.rs), but those capabilities are not yet broad enough to cover the repeated failure modes in the backlog.

## Goals

- Reduce duplicate per-rule traversal and matching logic.
- Make nested command, declaration, wrapper, and substitution contexts available through shared helpers.
- Standardize diagnostic span anchoring and deduplication.
- Standardize ShellCheck-compatible suppression behavior.
- Expand the semantic model where one capability unlocks multiple rules.
- Separate implementation bugs from intentional divergence and corpus noise.
- Create a phased migration plan that can be implemented incrementally.

## Non-Goals

- Rewriting the shell parser.
- Renaming rule codes or changing user-facing categories.
- Forcing exact ShellCheck parity in places where Shuck intentionally diverges.
- Replacing focused per-rule regressions with only corpus-based testing.

## Design

### Initiative 1: Shared AST Query Layer

Create a shared linter-side query module to replace the duplicated syntax helper stacks and give rules consistent access to nested commands, words, declarations, assignments, redirects, and substitutions.

Proposed module shape:

- `crates/shuck-linter/src/rules/common/mod.rs`
- `crates/shuck-linter/src/rules/common/query.rs`
- `crates/shuck-linter/src/rules/common/word.rs`
- `crates/shuck-linter/src/rules/common/command.rs`

The query layer should expose:

- command walking that always descends through nested substitutions and wrapper contexts
- declaration-aware iteration for `local`, `declare`, `typeset`, and `export`
- assignment and redirect enumeration with stable source spans
- word-part inspection helpers for quoted, unquoted, literal, mixed, and substitution-bearing words
- reusable query entrypoints instead of ad hoc callbacks in each rule

Action items:

- [x] Introduce `rules/common/query.rs` and move shared traversal primitives out of `style/syntax.rs` and `correctness/syntax.rs`.
- [x] Add common iterators for commands, declaration operands, assignments, redirects, and nested command substitutions.
- [x] Add tests that verify traversal reaches nested substitutions, assignment values, here-strings, `[[ ... ]]`, wrapper commands, and compound commands.
- [x] Migrate rules incrementally and delete duplicated helper logic once the last rule moves.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `S001` | New implementation should not start with a one-off walker. |
| `S004` | Needs nested substitution coverage beyond plain command arguments. |
| `S007` | Misses nested `printf` inside command substitutions. |
| `S008` | Needs more complete command-argument traversal. |
| `S009` | Needs nested `echo $(...)` detection in assignments and wrappers. |
| `S010` | Needs declaration assignments outside `export`. |
| `C008` | Needs per-expansion traversal within trap handlers. |
| `C013` | Needs better `find` substitution matching in loop headers. |
| `C015` | Needs more precise access to redirects and wrapper structure. |
| `C017` | Needs unary and binary test traversal in one shared model. |
| `C020` | Needs heredoc and literal/test context traversal. |
| `C057`, `C058` | Need robust inspection of nested substitution contents and redirects. |

### Initiative 2: Command and Context Normalization

Several rules need to know the effective command rather than just the immediate AST node. Wrapper commands, declaration forms, and command aliases should be normalized through shared helpers.

Proposed additions:

- effective command-name resolution shared across rule categories
- wrapper peeling for `command`, `exec`, `busybox`, `find -exec`, and known helper wrappers
- declaration normalization for `export`, `local`, `declare`, and `typeset`
- normalized command context records that include command name, wrapper chain, declaration kind, and body span

Action items:

- [x] Move effective-command helpers into the new common layer instead of keeping local copies inside individual rules.
- [x] Add declaration normalization helpers so assignment-bearing declaration rules can match on a shared representation.
- [x] Add regression coverage for wrapper commands and declaration variants.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C005` | Command-specific exemptions and wrapper awareness already dominate the rule. |
| `C007` | `find | xargs` matching benefits from normalized `find` handling. |
| `C013` | `find`-driven loop matching should use the same normalized command logic. |
| `C015` | Must distinguish real `sudo` redirection hazards from safe wrapper forms. |
| `S007` | `printf` detection should work through wrappers and nested contexts. |
| `S009` | Wrapper-heavy helper files need normalized command identity. |
| `S010` | Must treat `local`, `declare`, and related declarations like one family. |

### Initiative 3: Shared Word and Substitution Classifiers

Many rules ask the same structural questions about words and substitutions but answer them with slightly different local logic. A shared classifier layer should answer those once.

Proposed classifier families:

- literal vs expanded word
- quoted vs unquoted word
- plain command substitution vs mixed word containing substitution
- array expansion vs scalar expansion
- fixed-literal test operand vs runtime-sensitive operand
- command substitution redirect/capture shape
  - command-substitution intent stays deferred to Initiative 10

Action items:

- [x] Add reusable word classifiers under `rules/common/word.rs`.
- [x] Add substitution classifiers that describe nested commands, redirect presence, and whether stdout is still captured.
- [x] Add test vectors for mixed quote forms, nested substitutions, heredoc payloads, and arithmetic/file-test operators.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `S001`, `S004`, `S008` | All depend on accurate expansion and quoting classification. |
| `S007` | Needs to distinguish literal interpolated formats from variable-supplied formats. |
| `S009` | Needs reliable plain-substitution detection inside `echo`. |
| `S010` | Needs consistent detection of command substitutions in declaration assignments. |
| `C005` | Needs better classification of single-quoted fragments and mixed quoting. |
| `C008` | Needs precise expansion detection inside trap strings. |
| `C009` | Needs better literal-vs-runtime regex operand checks. |
| `C017`, `C019`, `C020` | Need consistent literal test classification and operator sensitivity. |
| `C057`, `C058` | Need structural classification of redirected substitutions. |

### Initiative 4: Shared Span Anchoring and Diagnostic Deduplication

Many `location-only` diffs are the same class of bug: the rule detects the right issue but reports the wrong span. Span and dedup decisions should be centralized.

Proposed helpers:

- anchor on variable-name span
- anchor on operator token span
- anchor on inner command substitution span
- anchor on quoted fragment span
- anchor per redirect target rather than per whole command
- deduplicate identical diagnostics that arise through multiple AST paths

Action items:

- [ ] Add a shared `rules/common/span.rs` with named anchor helpers.
- [ ] Add a dedup pass keyed by rule and normalized span so rules can opt into shared dedup behavior.
- [ ] Convert existing rules away from direct use of broad command or assignment spans where a more specific anchor exists.
- [ ] Add rule-specific tests that assert span choice directly.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C001` | Needs variable-name spans instead of assignment or command spans. |
| `C005` | Needs consistent quoted-fragment anchoring and dedup. |
| `C007` | Needs `find`-side anchoring rather than broad pipeline spans. |
| `C008` | Needs per-expansion spans inside traps. |
| `C010` | Needs operator-token anchoring for `&&` and `||`. |
| `C015` | Needs per-redirect spans rather than whole `sudo tee` command spans. |
| `S004`, `S005`, `S008`, `S009`, `S010` | Each has recurring location-only drift caused by broad spans. |

### Initiative 5: Shared Suppression Compatibility Layer

Suppression bugs recur across multiple rules and are currently easy to reintroduce because suppression is applied after diagnostics are emitted and is keyed mainly by line number.

The compatibility layer should support:

- `SC####` ShellCheck suppressions
- bare numeric ShellCheck suppressions such as `disable=2016`
- node-aware suppression checks for the AST region actually being reported
- shared tests for file-level, next-command, range-based, and alias-based suppression behavior

This work will likely extend [crates/shuck-linter/src/suppression/index.rs](crates/shuck-linter/src/suppression/index.rs) and [crates/shuck-linter/src/lib.rs](crates/shuck-linter/src/lib.rs), with rule-facing helpers exposed from the common layer.

Action items:

- [ ] Teach the suppression parser and index to normalize bare numeric ShellCheck codes.
- [ ] Add helper APIs that let rules consult suppression against a target node or normalized anchor span.
- [ ] Add conformance tests covering line-based, next-command, file-wide, and bare numeric suppressions.
- [ ] Audit large-corpus runners to ensure suppression is applied consistently during comparisons.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C005` | Explicit backlog item for bare numeric `2016` suppressions. |
| `C008` | `SC2064` suppressions are currently missed. |
| `C009` | `SC2076` suppressions are currently missed. |
| `S002` | `SC2162` suppressions are currently missed. |
| `S004` | `SC2046` suppressions are part of the remaining false-positive tail. |
| `S005` | `SC2006` suppressions are currently missed. |
| `S007` | `SC2059` suppressions are currently missed. |
| `S001` | Should launch with suppression support already in place. |

### Initiative 6: Runtime Prelude and Special-Variable Semantics

The semantic model needs a first-class catalog of shell-provided and environment-provided names so rules stop treating those as ordinary missing bindings.

Proposed additions:

- runtime-name catalog for Bash state variables and common environment names
- dialect-gated runtime variable activation
- explicit special-variable use modeling where a builtin consumes a variable implicitly

This work likely belongs near:

- `crates/shuck-semantic/src/lib.rs`
- `crates/shuck-semantic/src/builder.rs`
- `crates/shuck-semantic/src/dataflow.rs`

Action items:

- [X] Add a runtime prelude model for shell-provided variables and common environment variables.
- [X] Thread runtime-prelude knowledge into uninitialized-reference analysis.
- [X] Add builtin-specific implicit-use hooks where shell semantics consume variables without a visible `$name`.
- [X] Add focused semantic tests for `IFS`, `RANDOM`, `BASH_REMATCH`, `READLINE_LINE`, `USER`, `HOME`, `PWD`, `OSTYPE`, and related names.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C006` | Largest direct beneficiary; current backlog is dominated by missing runtime/environment modeling. |
| `C001` | Needs special-variable and implicit-use modeling for unused-assignment accuracy. |

### Initiative 7: Interprocedural Reads, Helper Contracts, and Source Closure

Several bugs are caused by Shuck treating helpers, sourced files, and function boundaries as analysis dead ends. Shuck already has source-closure support, but it needs to become a broader contract system.

Proposed additions:

- function summary records for reads, writes, and maybe-mutates behavior
- helper-file summaries for sourced and executed helper scripts
- project-closure annotations that distinguish reviewed semantic divergence from implementation gaps
- contract-aware dataflow joins when globals are read through helper code later

This work will build on:

- [crates/shuck-semantic/src/source_closure.rs](crates/shuck-semantic/src/source_closure.rs)
- [crates/shuck-semantic/src/call_graph.rs](crates/shuck-semantic/src/call_graph.rs)
- [crates/shuck-semantic/src/dataflow.rs](crates/shuck-semantic/src/dataflow.rs)

Action items:

- [ ] Expand helper-file summaries from synthetic reads into reusable read/write contracts.
- [ ] Add interprocedural summaries for function-to-caller and caller-to-helper flows.
- [ ] Decide where reviewed project-closure behavior is a semantic feature versus a corpus divergence.
- [ ] Add tests that cover sourced helpers, executed helpers, recursive helpers, and generated helper wrappers.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C001` | Already relies on globals across functions and helper files. |
| `C006` | Helper-heavy undefined-variable reports point to the same missing capability. |
| `C002` | Source-path and project-closure policy work should share the same helper model. |

### Initiative 8: Control-Flow and Dataflow Precision for Branches and Guarded Exits

Two of the biggest remaining semantic gaps are branch-join precision and guarded-exit reachability. These should be handled in the semantic layer rather than patched inside individual rules.

Proposed additions:

- more precise reaching-definitions accounting across `if`, `elif`, `case`, and loop joins
- guarded-exit modeling so `cmd || exit 1` does not mark later success-path code unreachable
- control-flow summaries for mixed helper/control structures

Action items:

- [ ] Improve reaching-definition joins in the dataflow pass.
- [ ] Improve CFG/dataflow interaction for guarded exits and fallthrough paths.
- [ ] Add semantic regressions for branch joins, loop-header reads, and guarded exit paths.
- [ ] Re-run the largest semantic rules after each change instead of waiting for a full backlog sweep.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C001` | Depends heavily on reaching definitions and branch joins. |
| `C124` | Dominated by guarded-exit false positives. |
| `C010` | Short-circuit branch-chain reasoning should benefit from better flow precision. |

### Initiative 9: Indirect and Dynamic Name Resolution

Dynamic-name and indirect-expansion cases appear in both unused-assignment and undefined-variable reports. They should be handled once in the semantic model.

Proposed additions:

- resolved target hints for `${!name}` and dynamic array-like expansions
- helper APIs to connect carrier variables to likely target bindings
- stronger use-accounting for dynamically chosen variables

Action items:

- [ ] Expand indirect target hints into a reusable resolution API instead of a rule-specific workaround.
- [ ] Thread indirect target resolution through both unused-assignment and uninitialized-reference analyses.
- [ ] Add tests for `${!name}`, generated helper name dispatch, and array-like indirect expansions.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C001` | Directly called out in the backlog for indirect and dynamic uses. |
| `C006` | Same family of dynamic-name scope failures shows up in helper-heavy scripts. |

### Initiative 10: Command-Substitution Intent Classifier

The substitution rules need shared semantics for whether output is actually being discarded or intentionally captured. Without that, redirected substitutions keep producing false positives across multiple rules.

Proposed additions:

- classify substitution stdout as captured, discarded, rerouted, or mixed
- distinguish stderr logging and fd swapping from true data loss
- expose substitution intent to linter rules through a shared helper rather than local redirect scans

Action items:

- [ ] Build a shared substitution-intent classifier in the common layer, backed by semantic or structural helpers where needed.
- [ ] Rewrite local redirect checks in substitution rules to use the classifier.
- [ ] Add regressions for `getopt`, `jq`, `awk`, `dialog`, `whiptail`, and helper-wrapper substitutions.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C057` | Needs to ignore internal fd plumbing when output is still captured. |
| `C058` | Needs the same captured-vs-discarded distinction. |
| `S004`, `S009`, `S007` | Structural substitution awareness should improve nested matching consistency. |

### Initiative 11: File and Fixture Context Classification

Many remaining bugs are concentrated in ShellSpec files, generated configure scripts, test harnesses, helper wrappers, and project-closure fixtures. Those contexts should be classified explicitly instead of being rediscovered by each rule.

Proposed additions:

- file-context classification for ShellSpec, generated helpers, wrappers, tests, and project-closure files
- optional rule-facing policy hooks for context-specific exclusions or reviewed divergences
- shared fixture metadata to keep DSL-specific or harness-specific behavior explicit

Action items:

- [ ] Add file-context classification helpers using path, shebang, and local syntax cues.
- [ ] Add minimal policy hooks so rules can opt into reviewed context-specific behavior without hardcoding file names.
- [ ] Add explicit tests for ShellSpec DSL blocks, generated `configure` scripts, and wrapper-heavy helpers.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `C022` | Needs to recognize ShellSpec parameter blocks as DSL, not shell tests. |
| `C063` | Needs to distinguish helper factories and test doubles from real overwrite bugs. |
| `C002`, `C006` | Need better project-closure and helper context awareness. |
| `S002`, `S003`, `S004`, `S005`, `S007`, `S009`, `S010` | Each has a helper-heavy or wrapper-heavy tail that should be handled through shared context metadata. |
| `C011`, `C124` | Context metadata should keep closure-heavy and directive-heavy files from polluting the core signal. |

### Initiative 12: Divergence and Corpus Metadata

Some bug reports are not really implementation bugs. They are mapping questions, reviewed divergence, or corpus noise. That information should live in structured metadata rather than only in prose bug notes.

Proposed additions:

- reviewed divergence records
- comparison-target notes for questionable ShellCheck mappings
- corpus-noise categories for unsupported-shell, patch, fish, parse-abort, and shell-collapse fixtures

Action items:

- [ ] Define a metadata format for reviewed divergence and mapping notes.
- [ ] Teach the large-corpus reporting pipeline to emit implementation diffs separately from noise and reviewed divergence.
- [ ] Audit rules whose docs say the remaining work is mainly policy or mapping.

Rules to refactor or reclassify through this helper:

| Rules | Why |
| --- | --- |
| `C002` | Remaining work is primarily project-closure policy. |
| `C019` | Needs a policy decision more than a matcher expansion. |
| `C046` | Looks like a comparison-target or mapping issue rather than a rule bug. |
| `C048`, `C050`, `C055` | Docs currently describe the sampled behavior as likely correct or policy-driven. |
| `S002`, `S003`, `S005`, `S009` | Each has a tail that may need divergence review rather than matcher churn. |

### Initiative 13: S001 as the First Consumer of the New Platform

`S001` should be implemented after the shared traversal, substitution, suppression, and span helpers exist. The rule is large enough that implementing it first on top of ad hoc logic would likely recreate the same problems already visible in the rest of the backlog.

Action items:

- [ ] Implement `S001` only after Initiatives 1, 3, 4, and 5 land.
- [ ] Treat `S001` as the proving ground for the common query, classifier, span, and suppression helpers.
- [ ] Reuse the same helper patterns to simplify follow-on refactors in `S004` and `S008`.

Rules to refactor onto this helper:

| Rules | Why |
| --- | --- |
| `S001` | Missing implementation path. |
| `S004`, `S008` | Immediate follow-on consumers of the same expansion and substitution helpers. |

## Rollout Plan

### Phase 1: Common Lint Infrastructure

- Build Initiatives 1 through 5.
- Migrate the rules with the clearest helper payoffs first: `S004`, `S007`, `S009`, `S010`, `C008`, `C015`.
- Keep rule behavior changes small and test-driven while the common layer stabilizes.

### Phase 2: Semantic Precision

- Build Initiatives 6 through 10.
- Re-run `C001`, `C006`, and `C124` after each semantic milestone.
- Only broaden rule logic after the semantic data they depend on becomes trustworthy.

### Phase 3: Context and Corpus Policy

- Build Initiatives 11 and 12.
- Use explicit metadata for reviewed divergence and mapping questions.
- Reclassify rules whose remaining backlog is mainly policy, not implementation.

### Phase 4: New Rule and Remaining Migrations

- Implement `S001`.
- Sweep remaining rules onto the common helpers.
- Delete obsolete helper code and collapse duplicated test scaffolding.

## Alternatives Considered

### Alternative A: Fix Each Rule Independently

Rejected because the bug backlog already shows the same structural failures repeating across many rules. Continuing with isolated rule fixes would add more duplicate walkers, duplicate span logic, and duplicate policy exceptions.

### Alternative B: Solve Most Parity Gaps with Allowlists and Divergence Notes

Rejected because many of the largest backlogs are real implementation gaps, especially in `C001`, `C006`, `C063`, `C124`, and the nested-command style rules. Metadata is useful for reviewed divergence, but it should not replace shared helper work.

### Alternative C: Push All Logic into the Semantic Layer

Rejected because many issues are linter-local concerns such as command normalization, quoted-word classification, suppression behavior, and diagnostic anchoring. The semantic layer should become more precise, but not every rule should require a semantic rewrite.

### Alternative D: Implement `S001` Immediately and Generalize Later

Rejected because `S001` is broad enough that it would likely reproduce the same traversal, suppression, and span problems already present elsewhere. It is better used as the first deliberate consumer of the shared platform.

## Risks

- The common helper layer may become too abstract if it tries to solve every rule shape at once.
- Semantic upgrades could change existing rule behavior in subtle ways if not validated incrementally.
- Context classification can become a pile of special cases if it is not grounded in explicit policy.
- Span normalization may temporarily reshuffle large-corpus diffs even when semantics improve.

## Verification

For each initiative and migrated rule:

- [x] Add focused unit or snapshot coverage in `crates/shuck-linter/src/lib.rs`, rule fixture files, or `crates/shuck-semantic/src/lib.rs` as appropriate.
- [x] Run `cargo test -p shuck-linter`.
- [x] Run `cargo test -p shuck-semantic`.
- [ ] Run `cargo test -p shuck --test large_corpus`.
- [ ] Run targeted corpus checks for the affected rule set:
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=S001`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=S004,S007,S008,S009,S010`
  - `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001,C006,C008,C015,C057,C058,C063,C124`
- [ ] Confirm that new helpers reduce repeated bug buckets rather than only moving mismatches between `shellcheck-only`, `shuck-only`, and `location-only`.
- [ ] Update the corresponding `docs/bugs/*.md` files when an initiative materially changes the remaining backlog for a rule.

## Appendix: Rules With Lowest Immediate Refactor Priority

These rules should mostly wait for parser, corpus-noise, or policy-layer improvements rather than receive dedicated matcher refactors first:

- `C014`
- `C018`
- `C021`
- `C025`
- `C047`
- `S006`

Their current bug notes are mostly environment-noise or parity-clean reports rather than evidence of a shared matcher deficiency.
