# AGENTS.md

These instructions apply to `crates/shuck-linter`. Follow the repo-level
`AGENTS.md` at the repo root first, then this file.

## The layered architecture

Linting is split into layers and each layer has a single job. Work at the
lowest layer that gets the answer right; do not duplicate that work higher up.

1. **Lexer / Parser** (`crates/shuck-parser`) — tokenizes and parses shell
   source into an AST. Owns all source/text scanning.
2. **AST** (`crates/shuck-ast`) — typed AST node definitions and span data.
3. **Indexer** (`crates/shuck-indexer`) — precomputed positional indexes over
   the source/AST. Cheap query surface.
4. **Semantic model** (`crates/shuck-semantic`) — bindings, references,
   scopes, declarations, source closure, call graph, CFG, dataflow.
5. **Linter facts** (`crates/shuck-linter/src/facts.rs` and
   `src/facts/`) — linter-owned structural summaries built once per file:
   normalized commands, wrapper chains, option-shape summaries, word/expansion
   facts, pipeline/loop/list facts, redirect/substitution facts, surface
   fragment facts, test/conditional facts.
6. **Rules** (`crates/shuck-linter/src/rules/{category}/`) — cheap filters
   over facts plus rule-specific policy and wording.

## Hard rules for rule files

Rule files (`src/rules/{correctness,style,performance,portability,security}/*.rs`)
**must not** parse, scan, walk, or otherwise re-derive structural information
from source or AST. They are filters over precomputed data.

Specifically, rule files **must not**:

- Walk or recurse through AST nodes. No calls to `walk_commands`,
  `iter_commands`, or any tree-traversal helper.
- Re-parse or re-tokenize source text. No string scanning of `checker.source()`
  to discover structure (substring searches for substantive analysis, regex
  over raw source, manual quote/escape handling, etc.). Span slicing for a
  literal equality check on already-classified data is fine; rediscovering
  shell structure from raw text is not.
- Normalize commands, classify words/redirects/substitutions, reconstruct
  test operands, parse command options, or otherwise recompute anything that
  is the job of the fact builder.
- Import from `crate::rules::common::*`. Rule-facing shared types and helpers
  must come from the crate root or a rule-local helper module.
- Reach into AST node variants that signal traversal intent. The architecture
  test in `src/rules/mod.rs` blocks tokens like `WordPart`, `WordPartNode`,
  `ConditionalExpr`, `PatternPart`, `ParameterExpansionSyntax`,
  `ZshExpansionTarget`, `ConditionalCommand`, `BourneParameterExpansion`,
  `iter_commands`, and `query::` from appearing in rule files.

If you reach for one of these, **stop**. The fix is to extend a lower layer:

- Need new structural data per command/word/loop/test/redirect? Add it to
  `LinterFacts` (or one of the fact submodules in `src/facts/`) and consume
  the new field from the rule.
- Need new bindings/references/scope/dataflow data? Extend
  `crates/shuck-semantic` and surface it via `checker.semantic()`.
- Need new tokenization or parsing behavior? Extend `crates/shuck-parser` and
  let it propagate up through the AST and semantic layers.

## What rule files look like

A rule file should be small. The shape is:

1. A `Violation` impl that returns the `Rule` code and a message.
2. A free function `pub fn rule_name(checker: &mut Checker)` that:
   - Iterates `checker.facts().*` (most rules) or `checker.semantic().*`
     (binding/reference rules).
   - Filters with rule-specific predicates.
   - Collects spans and reports via `checker.report_all` or
     `checker.report_all_dedup`.
3. A `#[cfg(test)]` module with snippet tests covering positive and negative
   cases. Snapshot-style fixtures live under `resources/test/fixtures/` and
   are wired through the category `mod.rs`.

Anything more — tree walking, source rescans, ad hoc command/word analysis —
belongs in a lower layer.

## Extending facts (the right escape hatch)

When a rule needs information that no fact exposes:

1. Add the new field/method to the appropriate fact in `src/facts.rs` or one
   of the submodules under `src/facts/`. Build it inside `LinterFacts::build`
   (or the relevant builder) so it is computed once per file.
2. Re-export rule-facing types from the crate root if rules need to name them.
3. Update the rule to consume the new fact field.
4. Add or update unit tests at the fact layer alongside the rule's tests.

This keeps repeated structural discovery in one place, keeps rule files cheap,
and makes the same structural data available to every other rule that needs it.

## Semantic vs. facts: which to use

- Use `checker.semantic()` for variable bindings, references, scopes,
  declarations, source closure, call graph, CFG, and dataflow facts. These are
  the semantic model's responsibility.
- Use `checker.facts()` for everything else: command shape, options, words,
  expansions, pipelines, loops, tests, redirects, substitutions, surface
  fragments. If the data feels structural and is not about variable
  definition/use, it belongs in facts.

If a rule looks like it needs both, pull whatever it needs out of each layer
in the rule body — do not invent a new traversal in the rule to bridge them.

## Enforcement

- `src/rules/mod.rs` contains an architecture test
  (`rule_modules_avoid_direct_ast_traversal_tokens`) that scans rule files for
  forbidden tokens. CI fails if a rule file imports or names them. If you need
  one of those types, the work belongs in the fact builder or semantic layer,
  not in the rule.
- New rules should be reviewed against this file. If a rule cannot be written
  without breaking these rules, the missing piece is almost always a fact that
  has not been built yet — build it.
