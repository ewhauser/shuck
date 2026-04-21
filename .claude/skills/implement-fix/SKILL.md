---
name: implement-fix
description: >
  Implement an autofix for an existing shuck-rs lint rule. Use this skill whenever
  the user asks to add a fix to a rule (e.g., "add a fix for S074", "make C001
  fixable", "implement the autofix for X023", "add safe fix for Y010"). This is
  about wiring an existing diagnostic up to produce edits — not about implementing
  new rules from scratch (that's the implement-rule skill) or fixing rule bugs
  (that's the fix-rule skill).
---

# Implement a Lint Fix

This skill turns an existing rule into a fixable rule: it attaches a `Fix` to
each diagnostic, sets the right `FixAvailability` and applicability, adds tests
that prove the rewrite is correct, and runs the conformance corpus to make sure
the fix doesn't shift parity numbers.

The reference implementation is **S074 / `AmpersandSemicolon`** at
`crates/shuck-linter/src/rules/style/ampersand_semicolon.rs`. Read it first if
anything below is unclear — it covers every moving part end-to-end.

## Before you start

Read these files:

1. `docs/rules/{CODE}.yaml` — the YAML metadata. It tells you whether the fix is
   safe and what the edit should do.
2. The current rule file (`crates/shuck-linter/src/rules/{category}/{rule_name}.rs`).
3. `crates/shuck-linter/CLAUDE.md` — the layered architecture and the "Authoring
   fixes" section. The skill summarizes it but the source is canonical.
4. `crates/shuck-linter/src/fix.rs` — the shared `Edit`, `Fix`, `Applicability`,
   `FixAvailability` primitives.
5. `crates/shuck-linter/src/rules/style/ampersand_semicolon.rs` — the reference
   implementation, including its tests.

If the rule lacks a `fix_description` in YAML, **stop and ask the user** — the
metadata is the source of truth and an absent fix description means the design
work hasn't been done yet.

## Step 1: Read the YAML metadata

```yaml
new_code: S074
safe_fix: true                        # → Applicability::Safe (use Fix::safe_edit)
fix_description: "Delete the …"       # → describes what the edit must do
```

Two fields drive everything:

| Field | Maps to |
|-------|---------|
| `safe_fix: true` | `Applicability::Safe`, `Fix::safe_edit` / `Fix::safe_edits` |
| `safe_fix: false` (with a `fix_description`) | `Applicability::Unsafe`, `Fix::unsafe_edit` / `Fix::unsafe_edits` |
| `fix_description: "..."` | The behaviour the edit must produce. Use it to pick deletion vs replacement vs insertion and to phrase the `fix_title()`. |

The YAML is authoritative. Do **not** override `safe_fix` based on your own
judgment — if the metadata says safe, implement safe; if it says unsafe,
implement unsafe. If you genuinely think the metadata is wrong, raise it with
the user before changing the rule.

## Step 2: Find the exact span the edit acts on

Open the rule file. The first question, lifted from `crates/shuck-linter/CLAUDE.md`:

> Do we already have an exact span for the token/text we want to edit?

There are three cases:

1. **The rule already collects the span it would edit.** Most fix-friendly rules
   do — they report at the span they want to delete or replace (e.g. S074 reports
   the `;` itself). Skip to Step 4.
2. **The rule has the diagnostic span but the edit needs a different span.** For
   example, the diagnostic points at a command but the fix wants to delete a
   single flag. Check whether the corresponding fact already exposes the edit
   span (option spans, redirect spans, word spans, etc.). If yes, thread it
   through. Skip to Step 4.
3. **No layer exposes the span.** Go to Step 3 and add it to facts first.

Do **not** rediscover the edit span from raw source inside the rule file. Rule
files cannot scan source, walk AST, or rebuild structure — the architecture test
in `src/rules/mod.rs` enforces this.

## Step 3: Extend facts (only if Step 2 needs it)

When the existing facts don't expose the edit span:

1. Pick the right fact type in `crates/shuck-linter/src/facts.rs` (or one of the
   submodules under `src/facts/`). Almost always one of: `CommandFact`,
   `RedirectFact`, `SubstitutionFact`, `WordFact`, `PipelineFact`,
   `SimpleTestFact`, `ConditionalFact`, or one of the option-shape facts under
   `CommandOptionFacts`.
2. Add the new span/method, populate it inside `LinterFacts::build` (or the
   relevant builder), and re-export any rule-facing types from the crate root
   if needed.
3. Add fact-layer tests for the new field if the existing coverage doesn't
   already exercise it.
4. Then return to Step 4 and consume the new fact field from the rule.

If you find yourself wanting to add a one-off helper that only this rule will
ever use, that's a yellow flag — facts are shared by design, so the data should
be useful to other rules too. If it really is one-off, still build it in facts;
the architecture test will reject it in the rule file.

## Step 4: Update the `Violation` impl

Three changes:

```rust
use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

impl Violation for AmpersandSemicolon {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;  // or Sometimes

    fn rule() -> Rule { Rule::AmpersandSemicolon }

    fn message(&self) -> String { "...".to_owned() }

    fn fix_title(&self) -> Option<String> {
        Some("remove the stray `;` after `&`".to_owned())
    }
}
```

**Picking `FIX_AVAILABILITY`:**

| Variant | Use when |
|---------|----------|
| `FixAvailability::Always` | Every diagnostic the rule emits will carry a fix. |
| `FixAvailability::Sometimes` | Only some emitted diagnostics carry a fix (e.g. fix only when a specific structural shape is present). |
| `FixAvailability::None` | No fix exists — leave as the default (don't set the constant). |

If you set `Always` you must actually attach a fix to **every** diagnostic the
rule emits. If any path emits a bare diagnostic, downgrade to `Sometimes`.

**Picking `fix_title()`:** a short imperative phrase describing the edit, in the
voice of "what shuck will do for you" (e.g. "remove the stray `;` after `&`"). It
should not duplicate the diagnostic message. The YAML `fix_description` is a
useful starting point but tighten it for terminal output.

## Step 5: Switch the rule body to attach the fix

Replace the `report*` call with a `report_diagnostic*` call that builds a
`Diagnostic` and attaches a `Fix`:

```rust
pub fn ampersand_semicolon(checker: &mut Checker) {
    let spans = checker.facts().background_semicolon_spans().to_vec();
    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(AmpersandSemicolon, span)
                .with_fix(Fix::safe_edit(Edit::deletion(span))),
        );
    }
}
```

Two API points to know:

- `Checker::report_diagnostic(diagnostic)` and `report_diagnostic_dedup(diagnostic)`
  accept a fully built `Diagnostic`. Use these (not `report` / `report_dedup`)
  whenever you want to attach a fix.
- `Diagnostic::new(violation, span).with_fix(fix)` is the standard builder.
  Prefer this over reaching into `Diagnostic` fields directly.

**Edit primitives** (from `src/fix.rs`):

| Primitive | Use for |
|-----------|---------|
| `Edit::deletion(span)` | Remove a span outright (e.g. a stray `;`). |
| `Edit::replacement(content, span)` | Replace a span with new text. |
| `Edit::insertion(offset, content)` | Insert text at an offset (zero-width). |
| `Edit::deletion_at(start, end)` / `replacement_at(start, end, content)` | Same, but with raw offsets when you have them. Avoid when a span is available. |

**Fix primitives:**

| Primitive | Use for |
|-----------|---------|
| `Fix::safe_edit(edit)` / `Fix::safe_edits([edit, ...])` | When `safe_fix: true` in YAML. |
| `Fix::unsafe_edit(edit)` / `Fix::unsafe_edits([edit, ...])` | When `safe_fix: false` in YAML. |

A single fix can carry multiple edits — keep them tightly related and
non-overlapping. The shared fixer in `src/fix.rs` handles deconflicting and
ordering across diagnostics; you do not.

**For `FixAvailability::Sometimes`:** branch in the rule body — attach the fix
only on the path where it's safe to apply, and emit a bare `Diagnostic::new(...)`
on the other paths.

## Step 6: Update tests

Keep the existing positive/negative diagnostic tests. Add three more:

### a. Snippet test that asserts the rewrite

```rust
#[test]
fn applies_safe_fix_to_background_semicolons() {
    let source = "#!/bin/sh\necho x &;\necho y & ;\n";
    let result = test_snippet_with_fix(
        source,
        &LinterSettings::for_rule(Rule::AmpersandSemicolon),
        Applicability::Safe,
    );

    assert_eq!(result.fixes_applied, 2);
    assert_eq!(result.fixed_source, "#!/bin/sh\necho x &\necho y & \n");
    assert!(result.fixed_diagnostics.is_empty());
}
```

`fixed_diagnostics` must be empty — if the fix is correct, re-linting the fixed
source should not surface the same diagnostic again. If it does, the edit is
wrong.

For unsafe fixes, pass `Applicability::Unsafe`; the helper will then exercise
the unsafe edits.

### b. "Looks similar but should not fix" snippet test

If the rule has cases that look like the violation but should not fix (S074's
case-arm `&;;`/`& ;;` terminators), add a test that asserts `fixes_applied == 0`
and the source is unchanged. This protects against the fix being too greedy.

### c. Snapshot test using `assert_diagnostics_diff!`

```rust
#[test]
fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
    let result = test_path_with_fix(
        Path::new("style").join("S074.sh").as_path(),
        &LinterSettings::for_rule(Rule::AmpersandSemicolon),
        Applicability::Safe,
    )?;

    assert_diagnostics_diff!("S074_fix_S074.sh", result);
    Ok(())
}
```

Imports needed:

```rust
use std::path::Path;
use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};
```

## Step 7: Update the fixture if needed

`crates/shuck-linter/resources/test/fixtures/{category}/{CODE}.sh` should cover:

- The basic violation (already there from the original rule).
- A case that fires the diagnostic and demonstrates the fix.
- A "near miss" that looks like a violation but must not be fixed (if the rule
  has one).

Don't widen the fixture to cover every imaginable edge case — keep it the same
shape as the existing fixture and only add what the fix needs.

## Step 8: Run tests and accept snapshots

```bash
cargo test -p shuck-linter
```

The new `assert_diagnostics_diff!` snapshot will fail the first time. Inspect
the rendered diff in the failure output — it shows the original source, applied
edit count, unified diff, fixed source, and any remaining diagnostics. If it
matches the YAML's `fix_description`, accept it:

```bash
cargo insta accept --workspace
```

Then run the full workspace:

```bash
cargo test --workspace
```

If the architecture test in `src/rules/mod.rs` fails, you've reached for an
AST/traversal token in a rule file — back out and push the work into facts
(Step 3).

## Step 9: Run the large-corpus conformance test

Fixes can shift parity numbers (the diagnostic the corpus expected at a given
line may be gone after the rewrite, or a previously-suppressed line may now
re-trigger). Always run:

```bash
make test-large-corpus SHUCK_LARGE_CORPUS_RULES={CODE}
```

This uses the nix-pinned ShellCheck and limits the comparison to the rule you
just made fixable. If the deltas change, decide whether the new behaviour is
correct (fix is doing its job and corpus expectations need updating) or whether
the fix is too aggressive (back to Step 5/6).

If you can't easily tell, run the targeted comparison again with
`SHUCK_LARGE_CORPUS_KEEP_GOING=1` to see every affected fixture.

## Step 10: Summarize

Report:

- Rule code and name, and whether the fix was implemented as Safe or Unsafe (per
  the YAML).
- `FIX_AVAILABILITY` chosen and why (Always vs Sometimes).
- Whether new fact-layer data was added, and which fact.
- New tests added (snippet rewrite + snapshot + any negative).
- Conformance result: parity unchanged / parity shifted with explanation.

Do **not** automatically:

- Add the rule to `LinterSettings::default_rules()` — that's a separate
  decision the user will make.
- Add CLI integration tests in `crates/shuck/tests/` — the S074 commit added
  those because it was the first fixable rule. Skip unless the user explicitly
  asks.
