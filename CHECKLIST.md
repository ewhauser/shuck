# Shuck Rust Frontend Checklist

This checklist is for turning the `bashkit` fork into a usable Rust frontend for shuck.

## 1. Syntax Foundation

- [x] Add comment-preserving lexer APIs without changing existing parser behavior.
- [x] Add a minimal `shuck-syntax` crate with strict Bash parsing and comment collection.
- [ ] Add a stable public syntax API surface for shuck-owned consumers.
- [x] Add initial dialect profiles and parse-view planning to keep parser policy out of downstream rules.
- [ ] Define parser configuration types for dialect, parse mode, and resource limits.
- [ ] Decide whether to wrap the existing AST directly or introduce a full-fidelity CST first.

## 2. Source Fidelity

- [x] Add explicit spans to important leaf nodes such as words, redirects, and assignments.
- [x] Add explicit spans to expansion-form nodes where diagnostics need finer attachment than whole words.
- [x] Preserve enough token/source information to reconstruct directive attachment points, including nested substitutions.
- [ ] Audit range behavior against ShellCheck-compatible reporting expectations.
- [x] Add regression tests for line/column accuracy on tricky constructs.

## 3. Comment And Directive Handling

- [x] Classify comments into ordinary comments vs shuck/ShellCheck directives.
- [x] Parse inline suppression directives and attach them to the right source ranges.
- [x] Preserve leading/trailing comment placement needed for facts and suppression logic.
- [x] Add tests for directive aliases, malformed directives, and multiple disables on one line.

## 4. Parse Modes And Recovery

- [x] Implement `strict-recovered` mode.
- [x] Implement `permissive` mode.
- [x] Implement `permissive-recovered` mode.
- [x] Define how parse errors, partial trees, and recovery diagnostics are surfaced to callers.
- [ ] Add corpus-style tests that prove recovery behavior on broken shell input.

## 5. Dialect Support

- [x] Add initial dialect selection behavior for native Bash parsing and Bash-backed permissive fallbacks for `sh`, `dash`, `ksh`, and `mksh`.
- [ ] Add dialect selection behavior for `bats` and `zsh`.
- [x] Decide that dialect support should flow through parse-view planning first, with grammar-specific parser branches only where the superset model is wrong.
- [ ] Model dialect-specific syntax allowances and reserved-word differences.
- [ ] Add fixture coverage for dialect-only constructs and dialect portability failures.

## 6. Linter-Oriented IR

- [ ] Design a shuck-specific HIR/facts layer over the parser output.
- [ ] Represent command calls, redirects, parameter expansions, command substitutions, process substitutions, and declarations explicitly.
- [ ] Preserve traversal order and parent/child relationships needed by rule runners.
- [ ] Provide stable node IDs or equivalent handles for cross-pass analysis.
- [ ] Add AST/HIR walk helpers that mirror the current Go rule authoring ergonomics.

## 7. Facts And Semantic Indexes

- [ ] Build line offset and source text helpers.
- [ ] Build comment and directive indexes.
- [ ] Build scope and variable indexes.
- [ ] Build source/import/project-closure indexes.
- [x] Build an initial suppression index from parsed directives.
- [x] Expose suppression queries on `ParsedSyntax` for future rule execution.
- [ ] Apply suppression indexes inside rule execution.
- [ ] Add tests that mirror current Go fact-building behavior where practical.

## 8. Control Flow Graph And Dataflow Analysis

- [ ] Build a CFG from the HIR for function bodies and top-level scripts.
- [ ] Model branching constructs (`if/elif/else`, `case`, `&&/||` short-circuit, `while/until/for`).
- [ ] Model early exits (`return`, `exit`, `break`, `continue`) and unreachable blocks.
- [ ] Model subshell boundaries as separate flow regions (pipelines, `(...)`, command substitutions).
- [ ] Implement reaching-definitions analysis for variable assignments.
- [ ] Implement uninitialized/possibly-unset variable detection.
- [ ] Implement dead code detection (unreachable code after unconditional `exit`/`return`).
- [ ] Define conservative handling for `eval`, `source`, and dynamic variable names.
- [ ] Add tests for CFG construction on non-trivial control flow (nested loops, traps, early returns).

## 9. Execution View Selection

- [ ] Recreate shuck's logic for choosing which parse view a rule sees.
- [ ] Define which rule phases can run on recovered or permissive trees.
- [ ] Add compatibility tests for variant-selection edge cases.

## 10. Rule Porting Preparation

- [ ] Decide the Rust rule registry shape and phase model.
- [ ] Port a small set of syntax/directive rules first as proving cases.
- [ ] Port one dataflow rule (e.g., uninitialized variable or dead code) to validate the CFG layer.
- [ ] Port one project-closure-sensitive rule before committing to the full migration.
- [ ] Compare rule authoring ergonomics against the current Go implementation and adjust the frontend if needed.

## 11. Verification

- [ ] Add focused unit tests for lexer, parser, comments, directives, and spans.
- [ ] Add snapshot/fixture tests for syntax and directive behavior.
- [ ] Add parity tests against shuck's current Go frontend on representative fixtures.
- [ ] Add a corpus runner for Rust frontend compatibility checks.
- [ ] Define stop/go criteria before larger-scale rule porting.

## 12. Tooling

- [ ] Add fast developer commands for targeted syntax tests.
- [ ] Add CI coverage for the new crate(s).
- [ ] Add benchmark cases for parse throughput and memory use.
- [ ] Add lint/format/doc expectations for the Rust workspace.

## Suggested Milestones

- [x] Milestone A: strict Bash parse + comment/directive capture + source fidelity baseline.
- [ ] Milestone B: recovered/permissive parse modes + execution-view selection.
- [ ] Milestone C: HIR/facts layer + suppression/project indexes.
- [ ] Milestone D: CFG + dataflow analysis (reaching definitions, dead code, uninitialized variables).
- [ ] Milestone E: first meaningful rule port set with parity checks.
