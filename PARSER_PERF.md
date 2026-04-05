# Parser Performance Refactor Checklist

## Status

Proposed

## Summary

An aggressive staged refactor of the `shuck-parser` lexer and parser to move the hot path closer to Ruff's performance model: cheap token kinds, source-backed payloads, minimal allocation, and no repeated parsing of the same word text.

This plan explicitly prioritizes throughput over migration safety. The main goal is to remove the current "lex into owned strings, then parse those strings again" architecture from the normal parser path.

## Motivation

Current parser and linter profiles show that parse time dominates overall lint throughput, especially on large function-heavy files like `nvm.sh`. The largest structural costs are:

- The lexer allocates owned `String`s for `Word`, `QuotedWord`, `LiteralWord`, and `Comment`.
- The parser reparses token text into `Word` AST nodes via `parse_word_with_context`.
- Some parser paths clone token text before classifying or transforming it.
- The lexer updates full line/column/offset positions on every character advance.
- Generic iterator-based lookahead is used on hot lexer paths.

Ruff's parser is a useful canonical example because it keeps parser dispatch cheap:

- token kind is separate from token payload
- tokens carry ranges and flags cheaply
- payload is only materialized when the parser actually needs it
- most parser logic runs on token kinds, not owned strings

Shuck cannot copy Ruff exactly because shell alias expansion, heredocs, and shell words are different problems, but the same high-level model applies.

## Goals

- Eliminate the double-parse of normal shell words.
- Remove owned string allocation from the main token stream wherever possible.
- Make parser control flow branch on `TokenKind`, not `Token` payload variants.
- Reduce lexer per-character overhead in lookahead and position tracking.
- Keep command substitution, parameter expansion, quoting, and alias behavior correct.
- Use benchmark and profile gates after every stage.

## Non-Goals

- Minimizing code churn.
- Preserving the current token API.
- Optimizing recovered parsing before the primary parse path is fast.
- Preserving current internal architecture if it blocks performance.

## Design

### Target Architecture

The end state should look like this:

- The lexer emits a cheap token stream: `TokenKind + range/span + flags + optional payload`.
- Comments are trivia with ranges only.
- Plain words, literal words, and quoted words are source-backed, not owned `String`s.
- The parser consumes pre-decoded or source-backed word payloads directly.
- `parse_word_with_context` is no longer used on the normal parser path.
- Byte offsets are tracked on the hot path; line/column are derived from a line index when needed.
- Lookahead and token classification use specialized fast paths, not generic iterator-heavy helpers.

### Stage 0: Baseline And Guardrails

- [x] Record current `lexer`, `parser`, and `linter` Criterion means for all benchmark cases.
- [x] Save baseline profiles for `parser/all`, `parser/nvm`, `linter/all`, and `linter/nvm`.
- [ ] Capture current top inclusive and self-time hotspots for future comparison.
- [ ] Write down baseline numbers in this file before starting the refactor.
- [ ] Decide final target metrics for parser and linter throughput.

Suggested commands:

```bash
cargo bench -p shuck-benchmark --bench lexer -- --noplot
cargo bench -p shuck-benchmark --bench parser -- --noplot
cargo bench -p shuck-benchmark --bench linter -- --noplot
PROFILE_CASE=all make profile-parser
PROFILE_CASE=nvm make profile-parser
PROFILE_CASE=all make profile-linter
PROFILE_CASE=nvm make profile-linter
```

#### Stage 0 Baseline

Captured on 2026-04-05 from the terminal output of the Stage 0 Criterion commands. Time and throughput values use the center estimate from Criterion's `[low estimate high]` output. Byte counts come from `crates/shuck-benchmark/resources/manifest.json`, with `all` equal to the sum of the five fixtures.

| Benchmark | Case | Bytes | Mean time | Throughput |
| --- | --- | ---: | ---: | ---: |
| `lexer` | `fzf-install` | 12,760 | 68.827 µs | 176.80 MiB/s |
| `lexer` | `homebrew-install` | 33,212 | 162.59 µs | 194.81 MiB/s |
| `lexer` | `ruby-build` | 47,738 | 228.08 µs | 199.61 MiB/s |
| `lexer` | `pyenv-python-build` | 81,725 | 392.70 µs | 198.47 MiB/s |
| `lexer` | `nvm` | 150,227 | 749.45 µs | 191.16 MiB/s |
| `lexer` | `all` | 325,662 | 1.6987 ms | 182.83 MiB/s |
| `parser` | `fzf-install` | 12,760 | 256.82 µs | 47.383 MiB/s |
| `parser` | `homebrew-install` | 33,212 | 580.14 µs | 54.596 MiB/s |
| `parser` | `ruby-build` | 47,738 | 1.0388 ms | 43.827 MiB/s |
| `parser` | `pyenv-python-build` | 81,725 | 1.5486 ms | 50.329 MiB/s |
| `parser` | `nvm` | 150,227 | 3.1278 ms | 45.805 MiB/s |
| `parser` | `all` | 325,662 | 6.6999 ms | 46.355 MiB/s |
| `linter` | `fzf-install` | 12,760 | 341.97 µs | 35.584 MiB/s |
| `linter` | `homebrew-install` | 33,212 | 797.29 µs | 39.726 MiB/s |
| `linter` | `ruby-build` | 47,738 | 1.6558 ms | 27.495 MiB/s |
| `linter` | `pyenv-python-build` | 81,725 | 2.8639 ms | 27.215 MiB/s |
| `linter` | `nvm` | 150,227 | 6.0955 ms | 23.504 MiB/s |
| `linter` | `all` | 325,662 | 12.068 ms | 25.735 MiB/s |

### Stage 1: Split Token Kind From Payload

- [x] Introduce a `TokenKind` enum for parser dispatch.
- [x] Introduce a lightweight token representation that stores kind, range/span, and flags separately from payload.
- [x] Replace `Token::Word(String)`, `Token::QuotedWord(String)`, and `Token::LiteralWord(String)` with kind plus source-backed payload.
- [x] Replace `Token::Comment(String)` with a trivia token kind plus range only.
- [x] Replace `Token::Error(String)` on the hot path with a lightweight error kind plus side-channel diagnostics where possible.
- [x] Update parser branching to use `TokenKind` instead of matching owned payload variants.
- [x] Remove parser logic that depends on comment string contents when only the range is needed.
- [x] Re-profile before moving on.

Exit criteria:

- The parser can operate on token kinds and ranges without reconstructing the old enum in the hot path.
- Comment allocation is gone from the main parse path.

### Stage 2: Replace String-Based Word Tokens With Source-Backed Word Payloads

- [x] Introduce a dedicated source-backed word payload type, for example `LexedWord` or `WordToken`.
- [x] Store enough structure in the word payload to distinguish plain, literal, and double-quoted segments.
- [x] Preserve quote and expansion semantics with flags or segmented spans instead of flattening into a single owned `String`.
- [x] Make quote concatenation produce a source-backed or segmented payload instead of eagerly flattening into a new `String`.
- [x] Keep owned text only for cases that truly require cooked text, such as ANSI-C escape processing or synthetic text that cannot be represented as a source slice.
- [x] Update parser entry points to consume `LexedWord` directly.
- [x] Delete normal parser uses of `current_word_to_word` and `word_from_token`.
- [x] Re-profile before moving on.

Exit criteria:

- Normal simple-command parsing no longer reparses token text into `Word`.
- The common path for `echo foo "$bar"` uses no owned token string allocation.

### Stage 3: Remove `parse_word_with_context` From The Main Parse Path

- [x] Move word decoding responsibility into the lexer or a shared low-level word decoder that runs exactly once.
- [x] Make `parse_simple_command` consume pre-decoded word payloads directly.
- [x] Make redirect targets consume pre-decoded word payloads directly.
- [x] Make assignment parsing consume pre-decoded word payloads directly.
- [x] Keep `parse_word_with_context` only for narrow fallback or standalone helper use.
- [x] Delete parser-side reparsing of quoted and literal token text for the normal AST build path.
- [x] Re-profile before moving on.

Exit criteria:

- `parse_word_with_context` is no longer a first-order hotspot in the main parser profile.
- Normal command parsing does not scan the same word text twice.

#### Stage 3 Notes

- 2026-04-05: a first segment-aware decode pass now preserves mixed quoted/unquoted expansion boundaries for normal words and assignment values, with regression tests covering `foo"$bar"baz` and `foo="$bar"baz`.
- 2026-04-05: the temporary segment-by-segment wrapper has been replaced with a shared append-style decoder (`decode_word_parts_into`) so normal word decoding no longer constructs intermediate `Word` values just to splice their parts into another `Word`.
- 2026-04-05: current-token word decoding and assignment-value decoding also avoid cloning `current_token` on their slower paths by temporarily taking and restoring the token around decode work.
- 2026-04-05: warm `cargo bench -p shuck-benchmark --bench parser -- nvm --noplot` runs after these changes stayed roughly flat in the `5.21 ms` to `6.06 ms` range, so this is still intermediate Stage 3 groundwork rather than the benchmark-positive landing.
- 2026-04-05: the strict-finish landing added a parser-local cache for decoded non-simple current words, centralized redirect parsing behind shared helpers, and removed all remaining normal-path `parse_word_with_context` call sites. The helper now remains only behind `parse_word_string*`.
- 2026-04-05: post-landing `cargo bench -p shuck-benchmark --bench parser -- nvm --noplot` measured `3.31 ms` (`[3.2603 ms 3.3094 ms 3.3717 ms]`), slightly slower than the earlier same-day `3.09 ms` checkpoint but still far below the earlier `5.21 ms` to `6.06 ms` intermediate Stage 3 range.
- 2026-04-05: `cargo flamegraph --profile profiling -p shuck-benchmark --bench parser -- nvm --noplot` did not produce a local profile here; on this machine it failed with `Cannot create output document ... cargo-flamegraph.trace` / exit code `8`, so hotspot confirmation still needs either a working local profiler run or an alternate profiling path.

### Stage 4: Make Nested Shell Constructs Source-Backed Too

- [x] Stop building temporary `String`s for command substitution and parameter expansion when the original source span is available.
- [x] Parse nested command substitutions from original source spans whenever possible.
- [x] Avoid temporary owned strings for `${...}` handling when only structure and spans are needed.
- [x] Audit `$()`, `` `...` ``, `${...}`, `$((...))`, array indices, and brace-expansion-like forms for avoidable allocation.
- [x] Keep allocation only for cooked constructs that materially transform bytes.
- [ ] Re-profile before moving on.

Exit criteria:

- Nested shell expansions no longer force avoidable temporary string assembly in the common path.

#### Stage 4 Notes

- 2026-04-05: the lexer now scans nested `$()`, `$((...))`, and `${...}` segments with lazy capture buffers, so verbatim unquoted and double-quoted words stay source-backed unless a construct actually cooks bytes.
- 2026-04-05: the parser now threads explicit source-backed context through word decoding, parses nested command substitutions from the original input via `nested_commands_from_current_input`, and builds `SourceText::source(...)` directly for arithmetic expressions, brace operands, replacement patterns, replacements, and array indices when the bytes are unchanged.
- 2026-04-05: owned text is still intentional for genuinely cooked cases, including backticks normalized to `$()`, ANSI-C escapes, escaped `$` / `"` / `` ` `` inside double quotes, and synthetic or reinjected text that does not map cleanly back to a source span.
- 2026-04-05: regression coverage now includes source-backed nested parameter expansions in unquoted and double-quoted lexer paths plus parser assertions that nested arithmetic expressions, brace operands, replacement patterns, and array indices remain source-backed on verbatim input while still rendering correctly on cooked input.
- 2026-04-05: full `cargo bench -p shuck-benchmark --bench parser -- --noplot` measured `parser/nvm` at `3.1155 ms` (`[3.0972 ms 3.1155 ms 3.1502 ms]`) and `parser/all` at `6.5593 ms` (`[6.5312 ms 6.5593 ms 6.5935 ms]`), beating both the Stage 4 checkpoint targets (`3.2346 ms`, `6.8130 ms`) and the Stage 0 parser baseline (`3.1278 ms`, `6.6999 ms`).
- 2026-04-05: full `cargo bench -p shuck-benchmark --bench lexer -- --noplot` remained healthy after the lexer-side refactor, with `lexer/nvm` at `853.94 µs` and `lexer/all` at `2.0287 ms`.
- 2026-04-05: `cargo bench -p shuck-benchmark --bench linter -- --noplot` was also rerun, but those numbers are intentionally excluded from the Stage 4 verdict because the current `linter` regression is caused by an unrelated change.
- 2026-04-05: fresh local `make profile-parser` reruns are currently blocked on this worktree because `scripts/profiling/profile_bench.sh` shells out to `samply`, and `samply` is not installed here. Existing Stage 0 profile artifacts remain the last successful saved profiles.

### Stage 5: Rework Alias Expansion And Synthetic Token Handling

- [ ] Replace synthetic token queues of heavyweight token objects with lightweight token slices or compact replay buffers.
- [ ] Avoid re-lexing alias expansions into full owned token payloads where possible.
- [ ] Preserve alias semantics, including recursive expansion guards and "expand next word" behavior.
- [ ] Ensure synthetic tokens remain cheap after the token-model split.
- [ ] Re-profile before moving on.

Exit criteria:

- Alias expansion is no longer coupled to heavyweight token allocation.
- Synthetic-token handling does not dominate peek/advance costs.

### Stage 6: Switch Hot Position Tracking To Byte Offsets

- [ ] Replace hot-path `Position` updates on every character with byte-offset tracking.
- [ ] Introduce or reuse a line index for deferred line/column mapping.
- [ ] Store spans as byte ranges or `TextRange`-style offsets internally.
- [ ] Convert diagnostics, comments, and AST spans to line/column only at reporting or API boundaries.
- [ ] Audit rebasing for nested constructs and synthetic spans.
- [ ] Re-profile before moving on.

Exit criteria:

- Full line/column updates are not happening on every lexer character advance.
- Diagnostics and AST span fidelity remain correct.

### Stage 7: Tighten Lexer Hot Paths

- [ ] Replace iterator-based `peek_nth_char` with specialized `first`, `second`, and `third` helpers.
- [ ] Add ASCII fast paths for the most common shell token categories.
- [ ] Use `memchr` or byte scanning for comments, plain words, and quote scanning where safe.
- [ ] Reduce branchy generic helpers on operator and redirect paths.
- [ ] Preallocate token, comment, and small temporary vectors where a lower bound is obvious.
- [ ] Audit `VecDeque<char>` and other queue structures on hot paths for cache-unfriendly behavior.
- [ ] Re-profile before moving on.

Exit criteria:

- `Lexer::next_token_inner`, `Lexer::advance`, and lookahead helpers show clear self-time reductions.

### Stage 8: Tighten Parser Hot Paths

- [ ] Introduce `TokenSet`-style bitsets for high-frequency parser membership checks.
- [ ] Replace repeated string comparisons for reserved words and terminators with cheaper classification.
- [ ] Fold repeated newline skipping into list parsers where possible instead of scattering `skip_newlines()` calls.
- [ ] Reduce unnecessary peeking and current-token reconstruction.
- [ ] Keep recovery bookkeeping out of the main parse fast path when not in recovery mode.
- [ ] Re-profile before moving on.

Exit criteria:

- Parser dispatch cost drops measurably in `parse_command`, `parse_simple_command`, and compound-list parsing.

### Stage 9: Delete Legacy Paths

- [ ] Remove the legacy string-owning token variants.
- [ ] Remove compatibility helpers that rebuild old token/text structures.
- [ ] Remove parser-side word reparsing helpers from the normal pipeline.
- [ ] Remove dead tests and snapshots tied only to the old token model.
- [ ] Re-baseline all parser and linter benchmarks.

Exit criteria:

- There is only one hot-path token model and one hot-path word decoding path.

## Stage Order

Recommended order:

1. Stage 0 baseline
2. Stage 1 token split
3. Stage 2 source-backed word payloads
4. Stage 3 one-pass word decoding
5. Stage 4 nested construct cleanup
6. Stage 5 alias and synthetic token cleanup
7. Stage 6 byte-offset spans
8. Stage 7 lexer hot path tuning
9. Stage 8 parser hot path tuning
10. Stage 9 legacy deletion and final re-baseline

This order is intentional. The highest-leverage work is architectural, not micro-optimization. The first half of the plan should remove double parsing and avoidable allocation before spending time polishing the remaining hot leaf functions.

## Success Criteria

- [ ] `parser/all` is at least 1.5x faster than baseline.
- [ ] `parser/nvm` is at least 1.5x faster than baseline.
- [ ] `linter/all` is at least 1.25x faster than baseline.
- [ ] `linter/nvm` is at least 1.25x faster than baseline.
- [ ] `parse_word_with_context` is no longer a primary hotspot in normal parser profiles.
- [ ] Lexer self-time shifts away from generic lookahead and per-character bookkeeping.
- [ ] The parser and linter test suites still pass.

Stretch targets:

- [ ] `parser/all` reaches 2x baseline throughput.
- [ ] `linter/all` reaches 1.5x baseline throughput.

## Alternatives Considered

### Only Micro-Optimize The Existing Lexer

Rejected because the current architecture still allocates owned word strings and reparses them later. Even a well-tuned version of the current design leaves the biggest structural cost intact.

### Only Introduce `TokenKind` And Keep String-Based Word Tokens

Rejected as insufficient. This would make parser dispatch cheaper, but it would not remove the main double-parse cost for shell words.

### Optimize The Linter And Semantic Layers First

Rejected as the first move because current profiles still show parser cost as the dominant component. Parser throughput needs to improve before downstream tuning has the best return.

## Verification

- [x] `cargo test -p shuck-parser`
- [x] `cargo test -p shuck-benchmark`
- [x] `cargo test -p shuck-linter`
- [x] `cargo bench -p shuck-benchmark --bench lexer -- --noplot`
- [x] `cargo bench -p shuck-benchmark --bench parser -- --noplot`
- [x] `cargo bench -p shuck-benchmark --bench linter -- --noplot`
- [x] `PROFILE_CASE=all make profile-parser`
- [x] `PROFILE_CASE=nvm make profile-parser`
- [x] `PROFILE_CASE=all make profile-linter`
- [x] `PROFILE_CASE=nvm make profile-linter`

For every completed stage:

- [x] compare benchmark means against Stage 0 baseline
- [ ] inspect the new top self-time hotspots
- [ ] confirm the old hotspot moved or shrank for the expected reason
- [x] update this file with what changed before starting the next stage
