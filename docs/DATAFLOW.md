# Dataflow Checklist

This document tracks the remaining dataflow and semantic modeling work behind the
`C006` (`SC2154`) parity gap.

The main lesson from the recent `C006` work is that the remaining failures are no
longer mostly rule-local policy gaps in
[`undefined_variable.rs`](../crates/shuck-linter/src/rules/correctness/undefined_variable.rs).
The larger remaining buckets come from missing semantic inputs:

1. helper- or project-provided bindings that arrive through source closure
2. framework- or file-class-provided bindings that exist before file execution

The goal here is to keep structural logic in `shuck-semantic` and linter facts,
not to grow more one-off suppressions in the rule itself.

## Current State

The current implementation already has the basic contract seam in place:

- [`crates/shuck-semantic/src/contract.rs`](../crates/shuck-semantic/src/contract.rs)
  defines `FileContract`, `ProvidedBinding`, `ProvidedBindingKind`, and
  `ContractCertainty`.
- [`crates/shuck-semantic/src/source_closure.rs`](../crates/shuck-semantic/src/source_closure.rs)
  now summarizes helpers as contracts, not just synthetic reads.
- [`crates/shuck-semantic/src/lib.rs`](../crates/shuck-semantic/src/lib.rs)
  imports helper-provided bindings as real `BindingKind::Imported` state and can
  seed file-entry contracts.
- [`crates/shuck-semantic/src/dataflow.rs`](../crates/shuck-semantic/src/dataflow.rs)
  treats imported bindings as definite or possible initialization inputs.
- [`crates/shuck-semantic/src/cfg.rs`](../crates/shuck-semantic/src/cfg.rs)
  preserves per-scope exit blocks so helper export certainty can be summarized
  correctly.
- [`crates/shuck-linter/src/ambient_contracts.rs`](../crates/shuck-linter/src/ambient_contracts.rs)
  is the first explicit registry for file-entry framework contracts.

That foundation removed the biggest "project closure needs real definitions"
gap. The remaining work is mostly about coverage, classification, and safety.

## Problem 1: Helper And Project Closure State

### What This Problem Is

Some scripts do not initialize all of their globals directly in the current
file. Instead, they rely on:

- sourced helper libraries
- nested sourced helpers
- project bootstrap scripts
- test harness setup files
- generated helper wrappers

Those files often provide names that behave like shell globals from the caller's
point of view. A read later in the file is only valid if the semantic model can
see that the helper contract exported that name.

This is the "project-closure" side of the residual `C006` gap.

### Why Rule-Side Filtering Is Not Enough

Rule-side exemptions can hide a missing report, but they cannot answer:

- which helper provided the name
- whether the helper always initializes it or only does so on some paths
- whether the helper was sourced or executed
- whether the binding becomes visible to later name resolution

Those are semantic questions, not `C006` policy questions.

### What Is Already Implemented

- [x] Add a first-class contract model for helper summaries.
- [x] Let sourced helpers contribute both required reads and provided bindings.
- [x] Keep executed local helpers read-only from the caller's point of view.
- [x] Materialize imported bindings as real semantic bindings.
- [x] Feed imported bindings into reaching-definitions and uninitialized analysis.
- [x] Distinguish `Definite` and `Possible` helper-provided initialization.
- [x] Use scope exit blocks when summarizing helper certainty.

### What Still Needs To Be Fixed

- [ ] Expand helper coverage for real-world project closure paths that are still
  missed by current source resolution.
- [ ] Audit directive-heavy source paths and templated source commands that still
  fall back to unresolved reads.
- [ ] Improve helper summarization for helper-library and test-harness families
  that rely on layered bootstrap files rather than one direct `source`.
- [ ] Investigate recurring `directive-handling` residuals to determine whether
  they are really source-closure misses, directive parsing misses, or corpus-only
  noise.
- [ ] Review remaining unlabeled `(none)` residuals and sort them into
  project-closure, shell-collapse, or genuine rule-policy gaps.
- [ ] Add more semantic regressions for transitive helper chains, cyclic helper
  graphs, mixed sourced/executed helper graphs, and generated helper wrappers.
- [ ] Check whether any remaining helper families need imported function
  contracts, not just imported variable contracts.
- [ ] Verify that expanding helper contracts does not increase the
  ShellCheck-only side by over-importing names that are only conditionally or
  locally visible.

### Families To Target Next

- [ ] `rvm` helper chains such as `manage__base_install`, `manage__base_fetch`,
  `pkg`, and `mount`
- [ ] `powerlevel10k` `gitstatus` build/bootstrap helpers
- [ ] `pyenv` helper-library state
- [ ] helper-library and test-harness scripts where the harness supplies globals
  outside the current file
- [ ] generated helper/state pipelines such as configure-like outputs where the
  helper relation is real and reusable, not a one-off script exception

### Done Criteria

- [ ] `project-closure` is materially smaller in the targeted large-corpus run.
- [ ] `project-closure,test-harness` also drops, not just the plain
  `project-closure` bucket.
- [ ] The imported-binding machinery remains reusable through
  `semantic().uninitialized_references()` without new rule-local exemptions.

## Problem 2: Ambient Framework And File-Context Contracts

### What This Problem Is

Some files do not receive their state from a sourced helper at all. They run
inside a framework that guarantees a set of names before execution starts.

Typical examples:

- packaging/build helper files
- framework hook scripts
- trigger scripts
- shell fragments embedded in a larger system contract

Those names are "already initialized at file entry" from the file's point of
view, even though there is no normal shell assignment in the current file.

This is the "shell-collapse" and framework-contract side of the residual `C006`
gap.

### Why This Is Different From Project Closure

Project closure says "another shell file provided this state through execution or
source relationships."

Ambient contracts say "files of this class begin life inside a runtime contract,
regardless of which specific helper was sourced."

The distinction matters because the second case should not depend on discovering
an explicit helper edge.

### What Is Already Implemented

- [x] Add `file_entry_contract` support to semantic build options.
- [x] Seed file-entry imported bindings into dataflow entry state.
- [x] Add an explicit ambient-contract registry on the linter side.
- [x] Add an initial `void-packages` contract provider as proof of shape.
- [x] Keep ambient matching explicit and reviewable instead of keying only on
  broad tags like `project-closure`.

### What Still Needs To Be Fixed

- [ ] Expand the provider registry to cover the repeated framework families that
  still dominate `shell-collapse`.
- [ ] Split "real framework contract" cases from "not really a shell file" cases
  so we do not model non-shell DSL fragments as normal shell programs.
- [ ] Strengthen provider matching with path and syntax signatures, not just
  context tags.
- [ ] Group large framework families into reusable provider modules rather than
  one provider per script.
- [ ] Decide where the line is between ambient variable contracts and broader
  file-class classification.
- [ ] Add negative tests that prove broad tags alone do not inject names.
- [ ] Verify that new ambient contracts help other semantic consumers beyond
  `C006`, not just undefined-variable reporting.

### Families To Target Next

- [ ] More `void-packages` file classes beyond the current common-path provider
- [ ] `xbps-src`-style packaging entry contracts
- [ ] `pycompile` and related trigger/helper families
- [ ] framework header/helper files like `makeself-header.sh`
- [ ] remaining build-style and hook scripts where the contract is stable across
  many files

### Questions To Answer Before Adding A Provider

- [ ] Is the binding set really guaranteed for this file family?
- [ ] Is the guarantee path-based, syntax-based, or both?
- [ ] Can we express it as a reusable file-entry contract instead of a one-off
  name allowlist?
- [ ] Would a stronger file classification be more correct than a contract?
- [ ] Does the provider reduce `shell-collapse` without increasing
  ShellCheck-only locations?

### Done Criteria

- [ ] `shell-collapse` drops materially in the targeted large-corpus run.
- [ ] The remaining files in that bucket are mostly either true divergences or
  cases that do not share a reusable framework contract.
- [ ] Provider coverage is explicit enough that reviewers can understand why a
  file received a contract.

## Cross-Cutting Guardrails

- [ ] Keep
  [`undefined_variable.rs`](../crates/shuck-linter/src/rules/correctness/undefined_variable.rs)
  thin. It should consume facts and semantic results, not reimplement project or
  framework semantics.
- [ ] Prefer reusable semantic concepts over `C006`-only special cases.
- [ ] Preserve clean-room policy. ShellCheck remains an oracle, not a source.
- [ ] Add targeted unit tests before large-corpus validation for each new
  semantic behavior.
- [ ] Track ShellCheck-only changes alongside Shuck-only improvements so parity
  does not improve by over-suppressing.
- [ ] Record genuine oracle divergences in corpus metadata instead of encoding
  them as semantics.

## Verification Checklist

- [ ] `cargo test -p shuck-semantic`
- [ ] targeted semantic regressions for helper export certainty and file-entry
  contracts
- [ ] targeted linter regressions for explicit ambient providers
- [ ] `cargo test -p shuck-linter rule_undefinedvariable_path_new_c006_sh_expects -- --nocapture`
- [ ] `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C006`
- [ ] compare new bucket counts against the most recent baseline in
  [`docs/bugs/C006.md`](./bugs/C006.md)

## Near-Term Outcome We Want

- [ ] shrink the remaining `project-closure` bucket again
- [ ] shrink the remaining `shell-collapse` bucket again
- [ ] avoid increasing the ShellCheck-only side
- [ ] leave behind better semantic infrastructure, not more name-shape patches
