---
name: conformance-check
description: >
  Verify ShellCheck conformance for a shuck rule by running the large corpus test,
  analyzing deltas, and producing a structured bug document in docs/bugs/. Use this
  skill whenever the user asks to check conformance, verify parity, run the corpus
  test for a rule, investigate shellcheck deltas, create a bug report for a rule's
  conformance gaps, or says things like "check C001 conformance", "verify SC2086
  parity", "run corpus test for C006", "what's the delta for X023". Even if the user
  just says "conformance" or "corpus test" with a rule code, this skill applies.
---

# ShellCheck Conformance Check

This skill runs the large corpus comparison for a specific shuck rule, analyzes the
deltas between shuck and ShellCheck, and produces a structured bug document that an
AI agent can later pick up and work through systematically.

## Overview

The goal is to answer: "How well does shuck rule CXXX match ShellCheck's SCYYYY on
real-world scripts?" The output is a `docs/bugs/CXXX.md` file with categorized deltas,
verdicts on each, and an actionable checklist.

## Step 1: Resolve the rule

Given a rule code (e.g., `C001`, `C005`) or ShellCheck code (e.g., `SC2086`):

1. Read the YAML definition in `docs/rules/CXXX.yaml`
2. Extract: `new_code`, `shellcheck_code`, `description`, `new_category`
3. Confirm the rule is implemented — check `crates/shuck-linter/src/registry.rs` for
   the rule code in the `declare_rules!` macro

If the rule isn't implemented yet, tell the user and suggest using `/implement-rule` first.

## Step 2: Run the corpus test

Run a 10% sample first to get a quick read on the delta size:

```bash
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=CXXX SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=10 SHUCK_LARGE_CORPUS_KEEP_GOING=1
```

This will likely fail (the test asserts zero deltas). That's expected — the failure
output IS the data. Capture the full output.

If the 10% sample shows a manageable number of failures (under ~50), you can optionally
run a larger sample or the full corpus to get more complete data. For large delta counts,
10% is sufficient to identify the patterns.

### Reading the output

Each failure block in the test output follows this structure:

```
compatibility diff (code + location):
  SCXXXX: shellcheck=N shuck=M
  labels:
    location-only | directive-handling | shellcheck-parse-abort | ...
  shellcheck parse aborted: true|false
  shellcheck locations:
    SCXXXX=N
  shuck locations:
    SCXXXX=M
  shellcheck diagnostics:
    SC1234 line:col-endline:endcol level message
  shuck diagnostics:
    CXXX=>SCXXXX line:col-endline:endcol severity message
```

The key fields:
- **shellcheck=N shuck=M**: count comparison. N>M means shuck is under-reporting,
  M>N means shuck is over-reporting
- **labels**: automated classification hints (see label meanings below)
- **shellcheck diagnostics / shuck diagnostics**: the actual warnings each tool emitted,
  which you need to compare line-by-line

### Label meanings

- `location-only` — same diagnostic codes, different spans. Usually a span anchoring
  difference, not a semantic issue.
- `shellcheck-parse-abort` — ShellCheck hit a parse error and bailed. Deltas from these
  scripts are unreliable.
- `directive-handling` — script has ShellCheck directives that may suppress differently.
- `project-closure` — script has `source`/`.` commands; results depend on resolution.
- `unknown-shell-collapse` — script starts with a non-shebang comment before the shell
  marker; shell detection may differ.

## Step 3: Tally the delta

From the test output, compute:

- **Total fixtures sampled** (from the progress output)
- **ShellCheck-only locations** — diagnostics ShellCheck emits but shuck doesn't (count + script count)
- **Shuck-only locations** — diagnostics shuck emits but ShellCheck doesn't (count + script count)
- **Location-only scripts** — scripts where counts match but spans differ

## Step 4: Sample and categorize deltas

Go through the failing fixtures and sample representative cases from each bucket.
For each sample, read the actual script source to understand what the code does. You
need enough samples to identify the distinct *patterns* — usually 3-8 samples per
bucket is sufficient.

### Categorization

For each distinct delta pattern, assign a verdict:

- **shuck-fix** — Shuck's behavior is wrong and should be changed to match ShellCheck.
  This is the most common case: shuck over-reports (false positive) or under-reports
  (false negative) compared to ShellCheck's intentional behavior.

- **shellcheck-quirk** — ShellCheck reports something but shuck intentionally does not,
  and we believe shuck is right to stay silent. ShellCheck's behavior appears to be a
  bug, limitation, or design choice we disagree with. Document *why* — cite the shell
  spec or semantic reasoning. These get added to the ShellCheck-side allowlist
  (`crates/shuck/tests/testdata/allowlists/scNNNN.yaml`).

- **shuck-correct** — Shuck reports something that ShellCheck does not, and we've
  reviewed it and decided shuck is right to flag it. The diagnostic is genuinely useful
  even though ShellCheck misses it. Document *why* — cite the shell spec or semantic
  reasoning. These get added to the shuck-side allowlist
  (`crates/shuck/tests/testdata/allowlists/shuck/cNNN.yaml`).

- **location-only** — Same diagnostic, different span anchoring. Not a semantic
  disagreement but worth tracking if the offset pattern is systematic.

- **environment** — The delta comes from parse-abort, directive handling, source
  resolution, or other environmental differences. Not actionable at the rule level.

### How to investigate a delta

For each failing fixture, the test output gives you the fixture name (e.g.,
`HariSekhon__DevOps-Bash-tools__.bash.d__teamcity.sh`). To find the actual script:

```bash
find .cache/large-corpus -name "HariSekhon__DevOps-Bash-tools__.bash.d__teamcity.sh" -o \
  -path "*/scripts/HariSekhon__DevOps-Bash-tools__.bash.d__teamcity.sh"
```

Read the script around the lines mentioned in the diagnostics. Compare what ShellCheck
flagged vs what shuck flagged. To understand ShellCheck's reasoning, you can run it
directly through nix:

```bash
nix --extra-experimental-features 'nix-command flakes' develop --command \
  shellcheck --format=json /path/to/script.sh 2>/dev/null | jq '.[] | select(.code == NNNN)'
```

Look for patterns across multiple samples — most deltas cluster into a few root causes.

## Step 5: Write the bug document

Create or update `docs/bugs/CXXX.md` using this template:

```markdown
# CXXX: [short title describing the conformance gap]

## Rule

| Field | Value |
|-------|-------|
| Shuck code | CXXX |
| ShellCheck code | SCYYYY |
| Category | [Correctness/Style/etc.] |
| Rule file | `crates/shuck-linter/src/rules/{category}/{name}.rs` |

## Delta snapshot

Sampled on [date] using `make test-large-corpus` at [N]% sample.

| Bucket | Locations | Scripts |
|--------|-----------|---------|
| ShellCheck-only | N | N |
| Shuck-only | N | N |
| Location-only | N | N |

## Delta analysis

### [Pattern name] `[verdict]`

**Bucket:** ShellCheck-only | Shuck-only | Location-only
**Impact:** N locations / N scripts in sample

[1-3 sentence description of the pattern and why it happens.]

**Samples:**
- `fixture_name:line` — [brief description of the specific case]
- `fixture_name:line` — [brief description]

---

### [Next pattern name] `[verdict]`

...repeat for each distinct pattern...

---

## Checklist

Each item links back to the delta pattern it addresses. Items are grouped by
implementation locality (changes to the same function/module are adjacent).

- [ ] **[Short task description]** — addresses: [Pattern name] `[verdict]`
  - [Implementation hint: what to change and where]
- [ ] **[Next task]** — addresses: [Pattern name] `[verdict]`
  - [Implementation hint]
- [ ] **Re-run corpus test and update this document**

## Allowlisted divergences

Entries added to suppress reviewed intentional divergences in the corpus test.

### ShellCheck allowlist (`crates/shuck/tests/testdata/allowlists/scNNNN.yaml`)

| Fixture | Line | Reason |
|---------|------|--------|
| [fixture_name] | N:N | [reason from the shellcheck-quirk pattern] |

### Shuck allowlist (`crates/shuck/tests/testdata/allowlists/shuck/cNNN.yaml`)

| Fixture | Line | Reason |
|---------|------|--------|
| [fixture_name] | N:N | [reason from the shuck-correct pattern] |

(Omit either section if no entries were added for that side.)

## Notes

[Any additional context: known ShellCheck bugs, upstream issues, spec references,
or decisions about which shellcheck-quirk/shuck-correct items we intentionally diverge on.]
```

### Template guidelines

The document is optimized for an AI agent to pick up later and work through:

- The **Rule** table gives the agent immediate access to file paths and codes.
- The **Delta analysis** sections each have a verdict so the agent knows which patterns
  need code changes and which are intentional divergences.
- The **Checklist** items reference specific patterns, so the agent can trace each fix
  back to the delta that motivated it. Include implementation hints — file paths, function
  names, the general approach.
- Verdict tags (`shuck-fix`, `shellcheck-quirk`, `shuck-correct`, `location-only`,
  `environment`) are inline with pattern names so they're scannable.
- Only `shuck-fix` items should appear in the checklist. `shellcheck-quirk` and
  `shuck-correct` items are documented for the record but don't generate code change
  work items — instead they get allowlisted (see Step 6).

## Step 6: Allowlist reviewed divergences

For deltas where we've reviewed the behavior and decided one tool is correct, add
allowlist entries so the corpus test passes despite the intentional divergence.

There are two allowlist sides:

### ShellCheck allowlists (for `shellcheck-quirk` verdicts)

When ShellCheck reports something that shuck intentionally does NOT report (because
ShellCheck is wrong or we disagree), add an entry to filter out the ShellCheck diagnostic.

**Location:** `crates/shuck/tests/testdata/allowlists/scNNNN.yaml`

These are keyed by ShellCheck code (lowercase). The span values come from ShellCheck's
diagnostic output.

**Note:** ShellCheck allowlists are currently loaded per-rule in the test harness
(`large_corpus_conforms_with_shellcheck()`). If the SC code doesn't already have loading
wired up, you'll need to add it — follow the `sc2034_allowlist` pattern as a model.

### Shuck allowlists (for `shuck-correct` verdicts)

When shuck reports something that ShellCheck does NOT report, and we've reviewed it and
decided shuck is right to flag it, add an entry to filter out the shuck diagnostic.

**Location:** `crates/shuck/tests/testdata/allowlists/shuck/cNNN.yaml`

These are keyed by shuck rule code (lowercase) and live in the `shuck/` subdirectory.
The span values come from shuck's diagnostic output. Unlike the ShellCheck side, shuck
allowlists are auto-discovered — any YAML file in the `shuck/` directory is loaded
automatically. No test harness changes needed.

### Allowlist format (same for both sides)

```yaml
# Large-corpus allowlist entries for reviewed CXXX/SCNNNN divergences.
# Keep this file narrow and review-backed: every entry must name the exact
# location we intentionally diverge on and why.
entries:
  - path_suffix: "owner__repo__path__to__script.sh"
    line: 42
    end_line: 42
    column: 5
    end_column: 18
    reason: "brief explanation of why this divergence is intentional"
```

Fields:
- `path_suffix` — the fixture filename (the `__`-delimited path form used in the corpus)
- `line`, `end_line`, `column`, `end_column` — exact span from the tool's diagnostic
- `reason` — a concise explanation citing the shell spec or semantic reasoning

### What NOT to allowlist

- `shuck-fix` verdicts — these need code changes, not allowlisting
- `environment` verdicts — these are filtered by labels already
- `location-only` verdicts — fix the span instead

## Step 7: Report to the user

Summarize:
- The delta snapshot (counts)
- How many distinct patterns you found
- The verdict breakdown (N shuck-fix, N shellcheck-quirk, N location-only, N environment)
- How many allowlist entries were added (if any)
- The path to the bug document
- A recommendation: is this rule close to parity (small targeted fixes) or far off
  (needs significant rework)?
