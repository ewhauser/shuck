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

The remaining zsh parser work is no longer in the live large-corpus harness. It is isolated to the unresolved snippets in:

- [`crates/shuck-parser/tests/testdata/oils/zsh-large-corpus-regressions.test.sh`](/Users/ewhauser/.codex/worktrees/ee11/shuck/crates/shuck-parser/tests/testdata/oils/zsh-large-corpus-regressions.test.sh)
- [`crates/shuck-parser/tests/testdata/oils_expectations.json`](/Users/ewhauser/.codex/worktrees/ee11/shuck/crates/shuck-parser/tests/testdata/oils_expectations.json)

The zsh-mode parser corpus still defaults that regression fixture to `parse_err` and then opts individual snippets into `parse_ok` as we finish them. Right now:

- `103` regression snippets exist in the zsh regression fixture
- `89` snippets are promoted to `parse_ok`
- `14` snippets remain unresolved

This document is intentionally zsh-only. The non-zsh OILS cleanup belongs in the parser corpus and expectation files, not here.

## Bucket Counts

| Code | Count | Primary layer | Representative surface |
| --- | ---: | --- | --- |
| `CMD-1` | 5 | command parser | compact same-line function bodies |
| `CMD-2` | 3 | command parser | compact brace groups after `&&` / `||` |
| `EDGE-1` | 1 | lexer / redirect plumbing / command parser | token-boundary composition edge |
| `EXPR-1` | 4 | word / conditional parser | zsh parameter flags and conditional pattern forms |
| `EXPR-2` | 1 | arithmetic parser | zsh arithmetic char-literal follow-through |

## Execution Order

### 1. Finish compact command-body parsing

Target codes: `CMD-1`, `CMD-2`

- Clear the remaining compact function-body and compact brace-group cases.
- Keep zsh-only same-line body handling isolated so it does not regress non-zsh parsing.

### 2. Finish expression and edge cleanup

Target codes: `EXPR-1`, `EXPR-2`, `EDGE-1`

- Resolve the remaining zsh parameter-flag, conditional-pattern, and arithmetic char-literal cases.
- Finish the final token-boundary composition edge only after the expression bucket is stable.

## Promotion Rules

- Every resolved snippet gets a `parse_ok` entry in [`crates/shuck-parser/tests/testdata/oils_expectations.json`](/Users/ewhauser/.codex/worktrees/ee11/shuck/crates/shuck-parser/tests/testdata/oils_expectations.json).
- Do not leave this document ahead of the expectations file. The expectations file is the executable source of truth.
- Keep the large-corpus harness green while shrinking the regression fixture.

After each bucket:

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

| Fixture | Code |
| --- | --- |
| `ohmyzsh__ohmyzsh__lib__clipboard.zsh` | `EDGE-1` |
| `ohmyzsh__ohmyzsh__lib__git.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__plugins__battery__battery.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__dash__dash.plugin.zsh` | `EXPR-1` |
| `ohmyzsh__ohmyzsh__plugins__extract__extract.plugin.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__plugins__genpass__genpass-xkcd` | `EXPR-2` |
| `ohmyzsh__ohmyzsh__plugins__rake-fast__rake-fast.plugin.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__plugins__rbenv__rbenv.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__urltools__urltools.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__wd__wd.sh` | `EXPR-1` |
| `romkatv__powerlevel10k__internal__p10k.zsh` | `EXPR-1` |
| `romkatv__powerlevel10k__internal__parser.zsh` | `EXPR-1` |
| `romkatv__powerlevel10k__internal__wizard.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__zsh-syntax-highlighting.zsh` | `CMD-1` |
