# AGENTS.md

These instructions apply to `crates/shuck-indexer`. Follow the repo-level
`/Users/ewhauser/working/shuck-rs/AGENTS.md` first, then this file.

## Indexing guardrails

- Performance is part of correctness in this crate. `Indexer::new()` may touch
  the full source, but avoid adding extra whole-source passes unless the benefit
  is clear and documented.
- Prefer compact positional data such as `TextSize`, `TextRange`, sorted
  vectors, and small enums over storing source text, duplicated spans, or cloned
  AST subtrees.
- Query methods should stay cheap and allocation-free. Do not add per-query
  source rescans, AST walks, or temporary `String`/`Vec` building in common
  lookup paths.
- Preserve the current design where indexes are precomputed once and queried
  many times. If a new feature needs repeated lookups, add precomputed metadata
  rather than recomputing from `source` on every call.
- Keep byte-offset semantics. Do not introduce character-based indexing or
  Unicode reindexing in core indexing paths.
- Prefer sorted vectors plus `partition_point`/binary search style lookups when
  they fit the access pattern. Do not switch to heavier map/set structures in
  hot paths without a measured reason.
- Avoid duplicate indexing of the same concept. Before adding another cached
  structure, check whether an existing index can answer the question with small
  additional metadata.
- If a new index stores anything larger than offsets, ranges, or tiny flags,
  document why the extra memory is worth the query-time win.

## Scope notes

- Tests and benchmarks can do more work than production lookups, but keep them
  aligned with the intended production performance model.
