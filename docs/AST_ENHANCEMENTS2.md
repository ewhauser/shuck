# AST Enhancements 2 For Rule Coverage

## Summary

This document captures the follow-on parser and AST work that still looks useful
after `AST_ENHANCEMENTS.md` is fully implemented.

The first roadmap targets the biggest linter-reliability problems:

- quote-aware word parts
- typed pattern and conditional operands
- typed variable references and array structure
- heredoc delimiter metadata
- structured arithmetic

That gets us close to the `gbash` frontend where it matters most for rule
correctness. The remaining gaps are narrower. They are mostly about preserving
surface syntax and parser-owned facts for specific rules in `docs/rules/**`,
not about copying the rest of `gbash`'s AST wholesale.

The default policy for this second phase should be:

- prefer parser-owned facts and small metadata fields over broad AST churn
- add a first-class node only when multiple rules or downstream passes need the
  same structure repeatedly
- avoid chasing full `gbash` parity unless a concrete rule or semantic pass
  actually depends on it

## Priorities

1. Brace syntax classification
2. Function declaration surface-form metadata
3. Word-surface trivia for leading backslashes and backtick closers
4. Richer heredoc closer metadata

## 1. Brace Syntax Classification

### Why this matters for rules

After the first roadmap lands, the main remaining brace-sensitive rules are:

- `docs/rules/X010.yaml` — brace expansion in portable `sh`
- `docs/rules/S029.yaml` — literal braces that bash may treat as expansion
- `docs/rules/C061.yaml` — template placeholders like `{{name}}` in command position

Today these cases are easier to detect with source rescans than with the AST.
That is workable, but it means rule logic has to distinguish:

- real brace expansion candidates like `{a,b}` or `{1..3}`
- literal brace text like `HEAD@{1}`
- doubled template braces like `{{name}}`
- brace characters appearing inside quoted or non-expanding contexts

Those are parser decisions, not semantic-analysis decisions.

### Proposed direction

Preserve brace syntax in a parser-owned form.

At minimum we should expose one of:

- a first-class `BraceExpansion` word-part node
- or a parser fact attached to a word/span that says a concrete brace
  expansion candidate was recognized

The key requirement is that rules can ask the parser:

- whether a brace construct was recognized as expansion syntax
- where the construct starts and ends
- whether the braces were treated literally instead

This does not require full `gbash` parity on day one. A lightweight parser fact
is acceptable if it covers the rule corpus cleanly.

### Rule wins

- `X010` stops guessing from raw `{` and `}` characters
- `S029` can distinguish literal braces from expansion syntax more precisely
- `C061` can detect template placeholders in command position without
  conflating them with ordinary brace expansion

## 2. Function Declaration Surface-Form Metadata

### Why this matters for rules

Several rules care about how a function was written, not just that a function
exists:

- `docs/rules/X004.yaml` — `function` keyword in `sh`
- `docs/rules/X052.yaml` — `function name()` form in `sh`
- `docs/rules/S041.yaml` — function body written as a bare compound command

Shuck already parses `function name { ... }` and `name() { ... }` through
separate parser paths, but the current `FunctionDef` shape does not preserve
that surface-form distinction directly.

### Proposed direction

Add small function-declaration metadata instead of redesigning the function AST.

At minimum:

- whether the `function` keyword was used
- whether `()` appeared after the name
- whether the body was a brace group or another accepted compound command form

This can live either on `FunctionDef` directly or in a small `FunctionSurface`
sub-structure.

### Rule wins

- `X004` can anchor on actual `function` usage instead of parser-mode
  heuristics
- `X052` can distinguish `function f()` from `function f`
- `S041` can detect bare non-brace bodies without reconstructing the source

## 3. Word-Surface Trivia For Leading Backslashes And Backtick Closers

### Why this matters for rules

Most quote and substitution rules are covered by the first roadmap. The
remaining gaps are a small set of surface-syntax checks:

- `docs/rules/S040.yaml` — leading backslash before a command name to bypass aliases
- `docs/rules/C069.yaml` — backslash run immediately before a closing backtick

These are not really semantic AST questions. They are parser-trivia questions
about exact token form.

### Proposed direction

Expose small parser-owned trivia facts instead of adding broad new node families.

At minimum:

- a word-level flag or trivia span for a leading unquoted backslash escape
- backtick command-substitution close trivia that records the contiguous
  backslash run before the closing backtick

We should treat this as surface metadata, not as a reason to make the command
AST alias-aware or token-stream-shaped.

### Rule wins

- `S040` can detect `\command` and `\rm` directly from parsed trivia
- `C069` can point at the suspicious `\ ` before a closing backtick without
  rescanning the entire substitution body

## 4. Richer Heredoc Closer Metadata

### Why this matters for rules

`AST_ENHANCEMENTS.md` already adds the semantic delimiter facts the linter needs
most. The remaining heredoc rules need more detail about the closing line:

- `docs/rules/C138.yaml` — missing closing marker
- `docs/rules/C144.yaml` — closer not alone on its line
- `docs/rules/C145.yaml` — near-match or misquoted closer
- `docs/rules/S030.yaml` — trailing whitespace on closer
- `docs/rules/S073.yaml` — spaces used with `<<-` instead of tabs

Those checks need facts about the closer candidate line itself, not just the
opening delimiter.

### Proposed direction

Extend heredoc metadata to preserve closer-line facts in a parser-owned form.

At minimum:

- whether a closer was matched at all
- the exact raw closer line or token text
- whether trailing text followed the closer
- whether the closer line used spaces or tabs before the marker
- whether a line was a near-match candidate
- whether EOF terminated the heredoc instead of a closer

This can be modeled as an extension of the dedicated heredoc metadata from the
first roadmap. We do not need to make heredoc bodies themselves more elaborate
unless a later rule demands it.

### Rule wins

- `C138` can report missing terminators from parser facts rather than end-of-file guesses
- `C144` can detect content-plus-closer lines directly
- `C145` can distinguish quoted or otherwise near-match closers from valid ones
- `S030` and `S073` can inspect whitespace on the actual closer line

## Why These Four

These items are the parts of `gbash`-style fidelity that seem most justified by
the current rule corpus.

They each map cleanly to concrete rules in `docs/rules/**`, and they are mostly
about syntax provenance rather than deep semantic restructuring.

By contrast, the following still look optional for current rule coverage:

- full `File` / `Stmt` parity with comment attachment
- full alias provenance on files and words
- a standalone `LValue` node
- complete `gbash`-style parameter-expansion unification
- specialty nodes like Bats test declarations

Those may still become valuable later, but they are not the next obvious rule
blockers after the first roadmap.

## Suggested Rollout Order

1. Function declaration surface-form metadata
2. Richer heredoc closer metadata
3. Word-surface trivia for leading backslashes and backtick closers
4. Brace syntax classification

This order is intentionally rule-driven:

- function-form metadata unlocks clean portability/style checks quickly
- heredoc closer facts remove some of the most brittle source rescans
- word-surface trivia is small and isolated
- brace syntax classification is important, but it is also the easiest place to
  overbuild, so it should be done after the parser-fact policy is clear

## Verification Strategy

For each follow-on enhancement:

1. add parser-level shape tests for the new metadata or node
2. add rule-level regression tests for the exact `docs/rules/**` cases it is
   meant to support
3. verify that the implementation reduces source rescanning instead of just
   duplicating it behind a helper

Recommended targeted follow-up:

- function surface form: `X004`, `X052`, `S041`
- heredoc close facts: `C138`, `C144`, `C145`, `S030`, `S073`
- word-surface trivia: `S040`, `C069`
- brace syntax: `X010`, `S029`, `C061`

## Notes

- Prefer parser facts when the question is about exact token surface rather than
  shell semantics.
- Promote a parser fact into a first-class AST node only when several rules or
  downstream passes need to traverse or transform the same structure.
- The goal of this document is not full `gbash` parity. The goal is to close the
  remaining rule-driven gaps without paying for unnecessary AST churn.
