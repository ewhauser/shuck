---
name: fix-rule
description: >
  Fix a shuck lint rule that has conformance deltas against ShellCheck. Use this
  skill whenever the user asks to fix a rule (e.g., "fix C001", "fix the deltas
  for C006", "address the corpus failures in C019"), debug why a rule is
  producing wrong results, or work through a bug document in docs/bugs/. This
  skill is about diagnosing whether a delta is a shuck bug or a ShellCheck quirk
  and then fixing at the right layer — not about implementing new rules from
  scratch (that's the implement-rule skill) or just running the corpus test
  (that's the conformance-check skill).
---

# Fix a Lint Rule

This skill takes a rule with known conformance deltas and fixes it. The core
judgment call on every delta is: **is ShellCheck right or is shuck right?** When
shuck is wrong, the fix belongs at whatever layer actually caused the problem —
not hacked into the rule just to satisfy the oracle.

## The hard architectural constraint

Before you change anything, internalize this: **rule files in
`crates/shuck-linter/src/rules/` must not parse, scan, or walk anything.**
They are cheap filters over precomputed facts and semantic data. All
structural discovery lives in lower layers:

- Tokenizing/parsing source → `crates/shuck-parser`
- Bindings, references, scopes, CFG, dataflow → `crates/shuck-semantic`
- Normalized commands, words, redirects, tests, pipelines, loops, surface
  fragments, etc. → `crates/shuck-linter/src/facts.rs` and `src/facts/`

Rule files **must not**:

- Walk or recurse through AST nodes (no `walk_commands`, `iter_commands`,
  manual child recursion).
- Re-parse or re-scan `checker.source()` to discover shell structure.
- Normalize commands, classify words/tests/redirects, parse command options,
  or otherwise recompute what the fact builder is responsible for.
- Import from `crate::rules::common::*` (rule-facing types come from the
  crate root or a rule-local helper module).
- Reference AST traversal types blocked by the architecture test in
  `src/rules/mod.rs` (`WordPart`, `ConditionalExpr`, `iter_commands`,
  `query::`, etc.).

If a fix tempts you to do any of those things in the rule file, **stop and
push the work down a layer.** See `crates/shuck-linter/AGENTS.md` for the full
contract.

## Before you start

Read these files to understand the rule and its current state:

1. The rule's YAML definition: `docs/rules/CXXX.yaml`
2. The rule implementation: `crates/shuck-linter/src/rules/{category}/{name}.rs`
3. The rule's facts (if any): `crates/shuck-linter/src/facts.rs` — search for the rule name
4. The bug document (if one exists): `docs/bugs/CXXX.md`
5. Existing corpus-metadata (if any): `crates/shuck/tests/testdata/corpus-metadata/cXXX.yaml`
6. The linter architecture contract: `crates/shuck-linter/AGENTS.md`

## Step 1: Set up the worktree

If you're in a git worktree (not the main worktree), the large corpus cache
won't exist. Symlink it from the main worktree so you don't re-download
everything:

```bash
make ensure-cache
```

This is idempotent — safe to run even if `.cache` already exists.

## Step 2: Run the corpus test for this rule

Run a targeted comparison for just the rule in question:

```bash
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=CXXX
```

The test will likely fail — the failure output IS the data you need. Capture and
read the full output.

If the full run takes too long or produces overwhelming output, start with a
sample:

```bash
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=CXXX SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=10
```

### Reading the output

Each failure block shows a fixture where shuck and ShellCheck disagree:

```
compatibility diff (code + location):
  SCXXXX: shellcheck=N shuck=M
  labels:
    location-only | directive-handling | shellcheck-parse-abort | ...
  shellcheck diagnostics:
    SC1234 line:col-endline:endcol level message
  shuck diagnostics:
    CXXX=>SCXXXX line:col-endline:endcol severity message
```

Key signals:
- **shellcheck=N shuck=0** — shuck is under-reporting (missing detections)
- **shellcheck=0 shuck=M** — shuck is over-reporting (false positives)
- **shellcheck=N shuck=N but different lines** — span anchoring issue
- **`shellcheck-parse-abort` label** — ShellCheck bailed on a parse error; delta is unreliable
- **`project-closure` label** — delta depends on source/dot resolution differences

## Step 3: Investigate each delta pattern

For each failing fixture, read the actual script source to understand what the
code is doing. Find the script in the corpus:

```bash
find .cache/large-corpus -name "fixture_name_here"
```

Then read the script around the lines mentioned in the diagnostics. Group
failures by root cause — most deltas cluster into a few distinct patterns.

### The core judgment: who is right?

For each pattern, decide:

**ShellCheck is right, shuck is wrong** — shuck has a bug. Proceed to Step 4.

**ShellCheck is wrong, shuck is right** — the oracle is imperfect. Common cases:
- ShellCheck warns on code that's actually safe (e.g., variables consumed by
  a sourced script, dynamic parameter expansion patterns)
- ShellCheck's parse aborted and it emitted partial/incorrect results
- ShellCheck misunderstands the script's intended shell dialect

For oracle issues, skip to Step 5 to record the divergence so it won't block
the corpus test or waste future investigation time.

**Genuine disagreement on policy** — shuck intentionally diverges from
ShellCheck (stricter or more lenient by design). Also goes to Step 5.

To understand ShellCheck's reasoning, run it directly:

```bash
nix --extra-experimental-features 'nix-command flakes' develop --command \
  shellcheck --format=json /path/to/script.sh 2>/dev/null | jq '.[] | select(.code == NNNN)'
```

## Step 4: Fix shuck at the right layer

This is the most important part. Fixing at the wrong layer creates tech debt
that compounds — a hack in the rule to work around a parser bug means every
future rule hitting the same construct will need the same hack. **And rules
are not allowed to compensate for missing structural data by walking or
parsing themselves** (see "The hard architectural constraint" above).

### Identify the layer

Work from the bottom up. The issue lives at the **lowest layer that gets it wrong**:

**Layer 1 — Lexer** (`crates/shuck-parser/src/parser/lexer.rs`)
Symptoms: tokens are wrong, quoting boundaries are off, heredoc content is
misattributed. Check by printing the token stream for a minimal reproducer.

**Layer 2 — Parser/AST** (`crates/shuck-parser/src/parser/`)
Symptoms: the AST structure doesn't match what the script actually says. A
command is parsed as the wrong type, redirections are attached to the wrong
node, compound commands have wrong boundaries. Check by dumping the AST for a
minimal reproducer.

**Layer 3 — Semantic model** (`crates/shuck-semantic/`)
Symptoms: bindings/references are wrong, scope assignment is off, def-use
chains miss a connection. The AST is correct but the semantic analysis
misinterprets it. Check by querying the semantic model directly.

**Layer 4 — Fact generation** (`crates/shuck-linter/src/facts.rs` and
`src/facts/`)
Symptoms: the semantic model is correct but the linter-level facts derived
from it are wrong. A test fact misclassifies an operand, a command fact
normalizes incorrectly, a needed structural summary doesn't exist yet. Check
by inspecting the facts for the failing fixture. **If a rule needs structural
data that no fact exposes, the fix is to add the fact here — not to walk the
AST in the rule.**

**Layer 5 — Rule implementation** (`crates/shuck-linter/src/rules/`)
Symptoms: the facts are correct but the rule's filtering predicate has a gap.
It reports a violation where it shouldn't (missing filter) or misses one it
should catch (incomplete matching over correct facts). This is the only layer
where the fix belongs in the rule file itself, and the fix should still be a
filter over existing facts — not new traversal or parsing.

### The "where does this fix go?" decision

Ask, in order:

1. Is the AST wrong for this snippet? → fix the parser (Layer 1/2).
2. Is the AST right but the semantic model misinterprets it? → fix the
   semantic layer (Layer 3).
3. Is the semantic model right but the rule has no fact that exposes the
   distinction it needs? → add or extend a fact in `facts.rs` (Layer 4).
4. Are the facts right and the rule is just filtering wrong? → fix the
   predicate in the rule file (Layer 5).

If you ever find yourself wanting to add `walk_commands`, `iter_commands`,
substring/regex scanning of `checker.source()`, `WordPart`/`ConditionalExpr`
matching, command-name normalization, option parsing, or test-operand
reconstruction inside a rule file — the answer is Layer 4 (add or extend a
fact), not Layer 5. The architecture test in
`crates/shuck-linter/src/rules/mod.rs` will fail the build if you try.

### Making the fix

Once you've identified the layer:

1. Write a **minimal reproducer** — the smallest shell snippet that triggers the wrong behavior
2. Add it as a unit test at the appropriate layer (parser test, semantic test, fact test, or linter fixture)
3. Fix the issue at that layer
4. If the fix is a new/extended fact, surface it via `LinterFacts` and update
   the rule to consume it. Do not bypass the fact and add traversal in the
   rule.
5. Run the layer's own tests to confirm the fix: `cargo test -p shuck-parser`, `cargo test -p shuck-semantic`, `cargo test -p shuck-linter`, etc.
6. Run the full workspace tests to check for regressions: `cargo test --workspace`
7. Accept any updated snapshots: `cargo insta accept --workspace`

## Step 5: Record oracle divergences in corpus-metadata

When a delta is a ShellCheck issue (or an intentional policy divergence) that
we won't fix, record it in the corpus-metadata so the test passes and future
investigators don't re-analyze the same case.

**File:** `crates/shuck/tests/testdata/corpus-metadata/cXXX.yaml`

Create the file if it doesn't exist. The format:

```yaml
reviewed_divergences:
  - side: shellcheck-only        # or "shuck-only"
    path_suffix: "owner__repo__path__to__script.sh"  # optional, narrows to one fixture
    line: 42                     # optional, narrows to one location
    end_line: 42                 # optional
    column: 5                    # optional
    end_column: 18               # optional
    labels:                      # optional, requires all listed labels to match
      - project-closure
    reason: "brief explanation citing shell spec or semantic reasoning"
```

### Field guidelines

- **side**: `shellcheck-only` when ShellCheck reports something shuck
  intentionally doesn't. `shuck-only` when shuck reports something ShellCheck
  misses and we've decided shuck is right.
- **path_suffix + line + column fields**: Use these to narrow the match. Omit
  any field to match broadly. A divergence with only `side` and `reason` (no
  location fields) applies to *all* diagnostics of that type for the rule.
  Prefer narrow matches when possible — broad matches can accidentally suppress
  real bugs.
- **reason**: Write this from scratch in your own words (clean-room policy).
  Explain the shell semantics that justify the divergence.

### When to use broad vs. narrow matches

**Narrow** (specific fixture + line): Use when the divergence is about a specific
script's context (e.g., "this variable is consumed by a sibling script after
sourcing").

**Broad** (side + reason only): Use when the divergence reflects a systematic
policy difference (e.g., "shuck intentionally flags dynamic parameter-expansion
patterns that ShellCheck ignores").

### comparison_target_notes

If the ShellCheck code mapping is wrong for corpus testing purposes (the rule
maps to an SC code but the corpus shows a different SC code's behavior), add a
note:

```yaml
comparison_target_notes:
  - current_shellcheck_code: "SC2124"
    reason: "The corpus surfaces SC2124 array-to-scalar warnings, not the behavior this rule targets."
```

## Step 6: Verify the fix

Re-run the corpus test for the rule:

```bash
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=CXXX
```

Check that:
- Fixed deltas are now gone
- Corpus-metadata entries suppress the reviewed divergences
- No new regressions appeared

If there are still failures, go back to Step 3 and work through the remaining
patterns.

## Step 7: Run the full test suite

```bash
make test
cargo test --workspace
```

Confirm nothing else broke.

## Step 8: Report

Summarize to the user:
- How many deltas existed and how many were addressed
- Which were shuck fixes (and at what layer) vs. oracle divergences
- The corpus-metadata entries that were added
- Whether the rule now passes the corpus test clean
