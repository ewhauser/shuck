# Parser Performance Roadmap

This document tracks the current parser-performance shape after the
`make flame-parser` run on 2026-04-08 and turns the flamegraph into a staged
exploration plan.

The goal here is not to lock in one implementation up front. It is to keep the
next profiling and optimization passes focused on the parts of the parser that
look most likely to pay off.

## Current Baseline

- Profiling entrypoint: `make flame-parser`
- Current default case: `parser/nvm`
- Fixture: `crates/shuck-benchmark/resources/files/nvm.sh`
- Fixture size: `150,227` bytes
- Latest observed runtime: `7.78 ms .. 8.17 ms`
- Latest observed throughput: `17.5 MiB/s .. 18.4 MiB/s`
- Latest artifact: `.cache/profiles/flame-parser-nvm.svg`

Notes:

- The widest parser stacks in the flamegraph are inclusive control-flow frames
  like `parse_command_list`, `parse_pipeline`, and `parse_command`. They are
  useful for orientation, but they are not the best optimization targets by
  themselves.
- The roadmap below is ordered mostly by repeated leaf work and estimated
  exclusive self time, not by the widest inclusive stack.

## Biggest Opportunities

| Area | Approx. direct share | Why it looks promising | First things to try |
| --- | ---: | --- | --- |
| Lexer and span bookkeeping | ~16% | Position tracking and token-boundary work show up repeatedly in leaf frames. | Carry offsets forward more cheaply, reduce `PositionMap` work, and defer line/column conversion where we can. |
| Word lookup and decode | ~13% | `current_word`, `scan_source_word`, and word-part decode are all hot and appear to duplicate work. | Reduce rescanning, reduce cloning, and fuse classification with decode where possible. |
| AST ownership churn | ~5% | `WordPart` and related drop/clone traffic is visible even on one parse benchmark. | Cut transient allocations, avoid eager clones, and consider small-vector style storage for tiny hot collections. |
| Comment attachment and lowering | ~3% to 4% | Post-parse tree walking for comments and lowering is measurable. | Replace repeated queue draining with an indexed cursor and test whether more attachment work can happen during parse. |
| Simple-command classification | ~2% to 3% | Assignment detection and simple-command classification inspect token text before full decode. | Reuse one parsed view of the token instead of classifying and decoding separately. |
| Brace-syntax rescanning | ~2% to 3% | `word_with_parts()` always rescans parts to derive brace metadata. | Gate it behind a cheap source check or make it lazy. |

## Roadmap

### 1. Lexer And Span Bookkeeping

This is the highest-value area to explore first. The flamegraph points at
`Lexer::current_position`, `next_lexed_token_with_comments`,
`next_lexed_token_inner`, and nearby token-advance helpers often enough that
small wins here should compound across the whole parse.

- [ ] Count how often `current_position`, `set_current_spanned`, and
      `advance_raw` run during the `parser/nvm` benchmark.
- [ ] Confirm whether we are paying for line/column materialization earlier
      than callers actually need it.
- [ ] Prototype an offset-first path that keeps byte offsets hot and only
      computes full `Position` data when spans escape the parser hot path.
- [ ] If full deferral is too invasive, prototype a cheaper incremental path
      that carries the current line/column forward without repeated map lookups.
- [ ] Re-run `make flame-parser` and `make bench-parser` after each prototype.

### 2. Word Lookup And Decode

The next cluster is the word pipeline: `current_word`, `scan_source_word`,
`decode_word_parts_into_with_quote_fragments`, and related helpers.

- [ ] Measure cache-hit versus decode-hit behavior inside `current_word`.
- [ ] Check how often the same token goes through
      `current_source_like_word_text`, `is_assignment`, and `current_word` in
      the same simple-command parse.
- [ ] Prototype a cheaper cached representation for the current word so the hot
      path does not need to clone a full `Word` just to read it again.
- [ ] Prototype a source-backed fast path that avoids rebuilding a fresh
      `String` in `scan_source_word` when a span or slice is sufficient.
- [ ] Try fusing simple-command classification with decode so we inspect the
      token once instead of classifying first and fully decoding later.
- [ ] Re-profile all parser benchmark cases, not just `nvm`, after any change
      here.

### 3. Brace-Syntax Metadata

Brace-syntax scanning shows up as a real subset of the word-decode cost.
Because it is metadata derived from already-decoded parts, it is a good
candidate for gating or laziness.

- [ ] Count how often `brace_syntax_from_parts` runs on words that do not
      contain `{`, `}`, `{{`, or `}}`.
- [ ] Add a cheap pre-check to skip brace scanning when the source text cannot
      contain brace syntax.
- [ ] Explore making brace-syntax derivation lazy so we only compute it when a
      caller asks for it.
- [ ] Verify that any lazy path still preserves existing behavior for quoted
      placeholders, brace expansion, and zsh-qualified glob edge cases.

### 4. Comment Attachment And Lowering

Comment attachment is not the biggest item, but it is visible enough that it is
worth keeping on the plan, especially because it is a second pass over the
already-built tree.

- [ ] Measure comment-attachment cost separately on a comment-heavy fixture.
- [ ] Replace repeated `VecDeque` draining with an index-based cursor and
      compare profiles.
- [ ] Check whether leading and inline comment decisions can be attached
      earlier, during parse, without making recovery or nesting logic worse.
- [ ] Keep the lowering and comment-attachment work separate while profiling so
      we can tell which pass is paying for what.

### 5. Allocation And Ownership Churn

Drop and clone traffic around `Word`, `WordPart`, and related syntax nodes is
large enough to be worth an explicit pass.

- [ ] Capture an allocation-oriented profile on `make bench-parser` before
      changing storage shapes.
- [ ] Audit hot-path `Vec` allocations for tiny collections such as command
      words, redirects, assignments, and word parts.
- [ ] Explore `SmallVec`-style storage only where the common case is clearly
      small and the code stays readable.
- [ ] Reduce eager `Word` cloning in parser caches and helper APIs before
      attempting larger AST-shape changes.
- [ ] Re-check drop-heavy frames after each ownership change so we do not trade
      fewer clones for more expensive cleanup elsewhere.

### 6. Secondary Parser Cleanup

These items looked smaller in the current profile, but they are still worth
tracking once the bigger items are cheaper.

- [ ] Revisit `parse_simple_command` once word decode is cheaper and see which
      branches still dominate.
- [ ] Re-check assignment parsing and split indexed assignment logic after any
      token/decode fusion work lands.
- [ ] Re-profile comment-heavy, array-heavy, and zsh-heavy scripts to see
      whether the hot set changes meaningfully by dialect or script shape.

## Working Checklist

Use this checklist for each experiment so we keep results comparable and do not
lose track of regressions.

- [ ] Record the current branch, benchmark case, and runtime before changing
      anything.
- [ ] Run `make flame-parser` on the default case before and after the change.
- [ ] Run `make bench-parser` before and after the change.
- [ ] Run at least one non-`nvm` parser profile, for example
      `PROFILE_CASE=homebrew-install make flame-parser`, before declaring a win.
- [ ] Run `cargo test -p shuck-parser`.
- [ ] Run any targeted parser or syntax regressions touched by the experiment.
- [ ] Save a short note in this document about what changed, what improved, and
      what did not move.
- [ ] Keep experiments isolated enough that a regression can be tied back to a
      single idea.

## Near-Term Order Of Operations

1. Start with lexer/span bookkeeping.
2. Move next to word lookup and decode reuse.
3. Gate or lazify brace-syntax derivation.
4. Revisit comment attachment and lowering.
5. Do a focused allocation/ownership pass only after the earlier work settles.

## Notes

- The current parser benchmark does not include linter or formatter work. This
  roadmap is intentionally parser-only.
- If a future flamegraph shifts the hot set materially, update this file before
  starting the next optimization pass.
