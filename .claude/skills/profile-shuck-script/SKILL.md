---
name: profile-shuck-script
description: Profile shuck scripts and large-corpus fixtures, especially requests to profile a corpus script/fixture, reprofile after a shuck performance change, or produce a hotspot table from a samply profile. Use when Codex needs to run the shuck large-corpus profiling harness, avoid corpus setup artifacts, interpret sampled profiles, and return an attributed-exclusive hotspot table.
---

# Profile Shuck Script

## Workflow

1. Work from the current `shuck` checkout. If the user asks for a fresh branch, create it from `main` before editing.
2. Ensure the profiling harness profiles the lint loop, not corpus setup:
   - Prefer `scripts/profiling/profile_large_corpus.sh <fixture> .cache/profiles 2000 1 12`.
   - The script should build the profiling binary, prepare any large-corpus fixture manifest before `samply record`, then run the sampled process using that manifest.
   - If the sampled profile includes corpus discovery, fixture-table construction, or resolver canonicalization, call that out as a setup artifact and fix the harness or exclude it before presenting hotspots.
3. Run the profile at least once after a rebuild and again warm if the first run includes rebuild/cold-start noise.
4. Summarize the newest saved profile with `scripts/summarize_samply_hotspots.py`.
5. Return an attributed-exclusive table, not a raw inclusive table.

## Commands

Build/profile a large-corpus fixture:

```bash
scripts/profiling/profile_large_corpus.sh xwmx__nb__nb .cache/profiles 2000 1 12
```

Summarize the latest samply profile:

```bash
python3 ~/.codex/skills/profile-shuck-script/scripts/summarize_samply_hotspots.py \
  .cache/profiles/large-corpus/xwmx__nb__nb.json.gz \
  target/profiling/examples/large_corpus_profile \
  --limit 12
```

Use the profile output's reported average from the harness stderr, not a profile-wide estimate, when stating wall-clock time.

## Table Style

Use "attributed exclusive" samples:

- Count each sample once.
- Attribute generic frames such as iterator adapters, `OnceLock`, `memchr`, allocation, and syscalls to the nearest meaningful shuck/semantic/parser caller when possible.
- Keep separate rows for setup artifacts only when they are truly inside the sampled workload.
- Do not lead with parent orchestration frames such as `main`, `Checker::check`, or `lint_file_at_path...`; those are usually inclusive wrappers.

Return a concise table like:

```markdown
Latest profile: `<fixture>`, 12 iterations, avg `<N> ms`.

| Rank | Hotspot | Attributed Exclusive | Representative Frame | Read |
|---:|---|---:|---|---|
| 1 | Linter facts builder | 55.8% | `facts::LinterFactsBuilder::build` | Clear next big target |
| 2 | Possible-variable-misspelling rule | 5.2% | `possible_variable_misspelling` | Rule-local matching |
```

If the user is debugging whether a row is real, add a short note after the table explaining the evidence from stack context.

## Interpretation Rules

- Treat unexpectedly large file open/read/canonicalization rows with suspicion. In this workflow they often mean the profiler captured corpus discovery or resolver setup, not linting.
- If I/O remains after loop-only profiling, split it into target fixture source read, source-closure sibling reads, manifest load, and path canonicalization.
- If C063 appears, mention whether it is still material after the activation-window index. Around 1-2% is no longer the main target.
- If `LinterFactsBuilder::build` dominates, identify it as the next broad optimization area rather than a single rule hotspot.

## Validation

For harness edits, run:

```bash
cargo fmt --check
cargo build --profile profiling -p shuck-benchmark --features large-corpus-hotspots --example large_corpus_profile
git diff --check
```

For rule-performance edits, also run the targeted rule tests and corpus gate relevant to the changed rule.
