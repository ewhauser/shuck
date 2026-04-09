# Zsh Parse Backlog

## Snapshot

As of 2026-04-08, the dedicated zsh large-corpus parse harness is clean again:

```bash
SHUCK_TEST_LARGE_CORPUS=1 \
SHUCK_LARGE_CORPUS_KEEP_GOING=1 \
SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=100 \
nix --extra-experimental-features 'nix-command flakes' develop --command \
  cargo test -p shuck --test large_corpus large_corpus_zsh_fixtures_parse -- \
  --ignored --exact --nocapture
```

That run now passes across all **709 zsh fixtures**.

The zsh-only regression fixture is now fully promoted. The remaining work described here has been closed out in:

- [`crates/shuck-parser/tests/testdata/oils/zsh-large-corpus-regressions.test.sh`](/Users/ewhauser/.codex/worktrees/3991/shuck/crates/shuck-parser/tests/testdata/oils/zsh-large-corpus-regressions.test.sh)
- [`crates/shuck-parser/tests/testdata/oils_expectations.json`](/Users/ewhauser/.codex/worktrees/3991/shuck/crates/shuck-parser/tests/testdata/oils_expectations.json)

The zsh-mode parser corpus still defaults that regression fixture to `parse_err` and then opts individual snippets into `parse_ok` as they are completed. Right now:

- `103` regression snippets exist in the zsh regression fixture
- `103` snippets are promoted to `parse_ok`
- `0` snippets remain unresolved

This document is intentionally zsh-only. The non-zsh OILS cleanup belongs in the parser corpus and expectation files, not here.

## Closed Buckets

| Code | Count | Primary layer | Representative surface |
| --- | ---: | --- | --- |
| `EDGE-1` | 1 | lexer / redirect plumbing / command parser | token-boundary composition edge |
| `EXPR-1` | 4 | word / conditional parser | zsh parameter flags and conditional pattern forms |
| `EXPR-2` | 1 | arithmetic parser | zsh arithmetic char-literal follow-through |

## Completed Work

Target codes: `EXPR-1`, `EXPR-2`, `EDGE-1`

- Rewrote the remaining six regression snippets into standalone zsh examples that preserve the original surface while including the missing closing context.
- Promoted those six snippets to `parse_ok` in the expectations file.
- Kept the live zsh large-corpus harness green while shrinking the standalone regression backlog to zero.

## Promotion Rules

- Every resolved snippet gets a `parse_ok` entry in [`crates/shuck-parser/tests/testdata/oils_expectations.json`](/Users/ewhauser/.codex/worktrees/3991/shuck/crates/shuck-parser/tests/testdata/oils_expectations.json).
- Do not leave this document ahead of the expectations file. The expectations file is the executable source of truth.
- Keep the large-corpus harness green while shrinking the regression fixture.

Verification commands:

```bash
cargo test -p shuck-parser --test oils_parse -- --nocapture

cargo test -p shuck-parser --test oils_parse \
  zsh_fixture_cases_match_parser_expectations_in_zsh_mode -- \
  --exact --nocapture

SHUCK_TEST_LARGE_CORPUS=1 \
SHUCK_LARGE_CORPUS_KEEP_GOING=1 \
SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=100 \
nix --extra-experimental-features 'nix-command flakes' develop --command \
  cargo test -p shuck --test large_corpus large_corpus_zsh_fixtures_parse -- \
  --ignored --exact --nocapture
```

## Remaining Fixtures

None. All zsh regression snippets in this fixture are now promoted to `parse_ok`.
