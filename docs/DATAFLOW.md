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
  defines `FileContract`, `FunctionContract`, `ProvidedBinding`,
  `ProvidedBindingKind`, and `ContractCertainty`.
- [`crates/shuck-semantic/src/source_closure.rs`](../crates/shuck-semantic/src/source_closure.rs)
  now summarizes helpers as contracts, not just synthetic reads, and applies
  imported helper-function contracts at later call sites.
- [`crates/shuck-semantic/src/lib.rs`](../crates/shuck-semantic/src/lib.rs)
  imports helper-provided bindings as real `BindingKind::Imported` state and can
  seed file-entry contracts.
- [`crates/shuck-semantic/src/dataflow.rs`](../crates/shuck-semantic/src/dataflow.rs)
  treats imported bindings as definite or possible initialization inputs.
- [`crates/shuck-semantic/src/source_closure.rs`](../crates/shuck-semantic/src/source_closure.rs)
  now renders resolver-backed static-tail candidates such as
  `"${rvm_path}/scripts/rvm"` through `SourcePathResolver`.
- [`crates/shuck-semantic/src/builder.rs`](../crates/shuck-semantic/src/builder.rs)
  and [`crates/shuck-semantic/src/source_closure.rs`](../crates/shuck-semantic/src/source_closure.rs)
  now keep quoted heredoc bodies and backslash-escaped `$` placeholders out of
  semantic/source-closure traversal.
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
- [x] Let sourced helpers export callable helper-function contracts.
- [x] Keep executed local helpers read-only from the caller's point of view.
- [x] Materialize imported bindings as real semantic bindings.
- [x] Feed imported bindings into reaching-definitions and uninitialized analysis.
- [x] Distinguish `Definite` and `Possible` helper-provided initialization.
- [x] Use scope exit blocks when summarizing helper certainty.
- [x] Apply imported helper-function reads and writes only when the imported
  helper is actually called.
- [x] Broaden resolver-backed source candidates to cover single-variable static
  tails such as `"${rvm_path}/scripts/rvm"`.
- [x] Keep generated shell text inside quoted heredocs and escaped-dollar
  heredoc placeholders out of semantic traversal.

### What Still Needs To Be Fixed

- [ ] Finish the remaining `powerlevel10k` `gitstatus/build` generated-bootstrap
  tail, which still dominates the exact `project-closure` bucket.
- [ ] Audit directive-heavy helper/generator cases such as the remaining
  `pyenv`, `pi-hole`, `acme.sh`, and `bats` residuals to separate real semantic
  misses from parser/directive-handling issues.
- [ ] Investigate recurring `directive-handling` residuals to determine whether
  they are really source-closure misses, directive parsing misses, or corpus-only
  noise.
- [ ] Review remaining unlabeled `(none)` residuals and sort them into
  project-closure, shell-collapse, or genuine rule-policy gaps.
- [ ] Move the remaining `rvm` library globals plus the `xbps-src` framework
  families that only make sense as file-entry/bootstrap state into Problem 2
  ambient contracts rather than adding more helper-side guessing.

### Families To Target Next

- [ ] `powerlevel10k` `gitstatus` build/bootstrap helpers
- [ ] `pyenv` helper-library state and generated PowerShell helper stubs
- [ ] directive-heavy helper/generator chains in `pi-hole`, `acme.sh`, and
  `bats`
- [ ] remaining `rvm` helper-library globals such as
  `manage__base_install`, `mount`, `selector`, and related library files
  after they are reclassified under Problem 2
- [ ] remaining build-style/framework files that are really ambient contracts,
  not source edges

### Done Criteria

- [x] `project-closure` is materially smaller in the targeted large-corpus run.
- [x] `project-closure,test-harness` also drops, not just the plain
  `project-closure` bucket.
- [x] The imported-binding machinery remains reusable through
  `semantic().uninitialized_references()` without new rule-local exemptions.

### 2026-04-08 Follow-Up

Re-ran `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C006` on April 8, 2026
after landing sourced-helper function contracts, resolver-backed static-tail
source resolution, and generated-shell traversal guards.

- Fixed in Problem 1:
  imported helper-function contracts now affect caller globals only at actual
  call sites; resolver-backed static tails now collapse the `rvm`
  bootstrap/test-harness chain from `66` exact `project-closure,test-harness`
  locations to `16`; generated shell output with escaped-dollar placeholders no
  longer feeds `C006`.
- Moved to Problem 2:
  the remaining large `rvm` library globals (`manage__base_install`, `mount`,
  `selector`, `manage__rubinius`, `cli`, `info`, `list`, `manage__base`) and
  the reusable `xbps-src` framework files still behave like file-entry/bootstrap
  contracts, not helper-import edges.
- Current acceptance snapshot:
  `shuck-only=1740`, `shellcheck-only=53`, `project-closure=640`,
  `project-closure,test-harness=16`, `project-closure,shell-collapse=48`,
  `shell-collapse=318`.
- Remaining exact `project-closure` leaders:
  `powerlevel10k` `gitstatus/build` (`129`) plus the `rvm` library cluster above.
- No new semantic allowlists were added for reviewed divergence; any later
  reviewed oracle differences should stay in corpus metadata rather than in
  semantics.

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

- [x] `cargo test -p shuck-semantic`
- [ ] targeted semantic regressions for helper export certainty and file-entry
  contracts
- [x] targeted linter regressions for explicit ambient providers
- [x] `cargo test -p shuck-linter rule_undefinedvariable_path_new_c006_sh_expects -- --nocapture`
- [x] `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C006`
- [x] compare new bucket counts against the most recent baseline in
  [`docs/bugs/C006.md`](./bugs/C006.md)

## Near-Term Outcome We Want

- [ ] shrink the remaining `project-closure` bucket again
- [ ] shrink the remaining `shell-collapse` bucket again
- [ ] avoid increasing the ShellCheck-only side
- [ ] leave behind better semantic infrastructure, not more name-shape patches
