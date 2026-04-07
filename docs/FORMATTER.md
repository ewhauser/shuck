# Shell Formatter Roadmap

## Status

- [x] Add a generic formatting/printer layer in `crates/shuck-format`.
- [x] Rework `crates/shuck-formatter` around Ruff-style formatter modules and traits.
- [x] Add shell formatter options aligned with the supported `shfmt` printer knobs.
- [x] Add Bash, POSIX, and mksh dialect selection and auto inference.
- [x] Wire `shuck format` to the new formatter pipeline.
- [x] Add `[format]` config support in `shuck.toml`.
- [x] Include formatter options in the cache key.
- [x] Add focused formatter, parser-dialect, CLI integration, and opt-in `shfmt` oracle tests.

## Current Shape

- [x] Keep the public entrypoint as `format_source(source, path, options) -> Result<FormattedSource>`.
- [x] Support `--dialect`, `--indent-style`, `--indent-width`, `--binary-next-line`, `--switch-case-indent`, `--space-redirects`, `--keep-padding`, `--function-next-line`, `--never-split`, `--simplify`, and `--minify`.
- [x] Preserve input line endings where possible and ensure a single trailing newline.
- [x] Preserve comments by default and drop them in `--minify`.
- [x] Keep an opt-in oracle suite that runs `shfmt` from the repo's Nix dev shell.

## Next Milestone

- [ ] Implement real `simplify` rewrites in `crates/shuck-formatter/src/simplify.rs`.
- [ ] Keep each simplify rewrite byte-stable, idempotent, and independently testable.
- [ ] Add golden tests for each simplify rewrite before enabling broader transformations.
- [ ] Make `--minify` rely on the real simplify pass instead of just the current compact-print path.

## Comment Attachment

- [ ] Replace the current line-based comment stream with true leading / trailing / dangling attachment.
- [ ] Anchor comments to AST nodes rather than consuming them only by line order.
- [ ] Preserve ambiguous comments with explicit verbatim fallbacks instead of best-effort guessing.
- [ ] Add regressions for comments around continuations, compound commands, substitutions, and heredocs.

## Formatter Parity

- [ ] Reduce `verbatim(...)` fallback use in `crates/shuck-formatter/src/command.rs`.
- [ ] Replace source-slice summaries for compound bodies with structured formatting.
- [ ] Format heredoc-bearing commands structurally where safe instead of preserving the whole command verbatim.
- [ ] Improve `case`, `if`, loop, function, and pipeline layout choices to better match `shfmt`.
- [ ] Expand mksh- and POSIX-specific formatting coverage beyond the current parser-level support.

## Keep Padding

- [ ] Narrow `keep-padding` fallback regions so safe surrounding code can still be normalized.
- [ ] Track alignment-sensitive spans explicitly instead of preserving whole commands when padding appears.
- [ ] Add regressions for assignments, declarations, tables of redirects, and mixed-comment alignment.

## Validation

- [x] Grow the `shfmt` oracle fixture set across dialects and option combinations.
- [x] Reuse the benchmark corpus scripts as an in-memory `shfmt` oracle so large real-world deltas stay visible during formatter work.
- [x] Add macro wall-time benchmark targets that compare `shuck format` against `shfmt` on the shared benchmark corpus.
- [ ] Add fixtures that compare default output and option-specific output separately.
- [ ] Add more CLI tests for config precedence across nested project roots.
- [ ] Add formatter regressions for stdin plus `--stdin-filename` inference for `.sh`, `.bash`, `.ksh`, `.dash`, and `.mksh`.
- [ ] Periodically run the ignored oracle suite and use mismatches to drive targeted parity fixes.

## Nice To Have

- [ ] Add generated formatter glue for `generated.rs` once the node surface stops changing frequently.
- [ ] Revisit whether some formatter decisions should move into parser or AST metadata to avoid source-slice fallbacks.
- [ ] Consider a dedicated formatter fixture corpus under `crates/shuck-formatter/tests/fixtures/`.
- [ ] Document intentional divergences from `shfmt` when Shuck chooses safety over normalization.

## Notes

- The opt-in `shfmt` oracle now compares both targeted fixtures and the five benchmark scripts entirely in memory, with unified diffs truncated to stay readable on large failures.
- Use `make bench-macro-format` for the full formatter corpus and `make bench-macro-format-single BENCH_FILE=/absolute/path/to/script.sh` for a one-off file comparison.
