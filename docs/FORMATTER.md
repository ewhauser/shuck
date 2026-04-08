# Shell Formatter Roadmap

## Status

- [x] Add a generic formatting/printer layer in `crates/shuck-format`.
- [x] Rework `crates/shuck-formatter` around Ruff-style formatter modules and traits.
- [x] Add shell formatter options aligned with the supported `shfmt` printer knobs.
- [x] Add Bash, POSIX, mksh, and zsh dialect selection and auto inference.
- [x] Wire `shuck format` and `shuck format-stdin` to the formatter pipeline.
- [x] Add `[format]` config support in `shuck.toml`.
- [x] Include formatter options in the cache key.
- [x] Add focused formatter, parser-dialect, CLI integration, and opt-in `shfmt` oracle tests.
- [x] Reuse the five benchmark scripts as an in-memory `shfmt` oracle corpus.

## Current Shape

- [x] Keep the public entrypoint as `format_source(source, path, options) -> Result<FormattedSource>`.
- [x] Support `--dialect`, `--indent-style`, `--indent-width`, `--binary-next-line`, `--switch-case-indent`, `--space-redirects`, `--keep-padding`, `--function-next-line`, `--never-split`, `--simplify`, and `--minify`.
- [x] Preserve input line endings where possible and ensure a single trailing newline.
- [x] Preserve comments by default and drop them in `--minify`.
- [x] Run simplify rewrites before formatting when `--simplify` or `--minify` is enabled.
- [x] Support `--stdin-filename` dialect inference for `.sh`, `.bash`, `.ksh`, `.dash`, `.mksh`, and `.zsh`.
- [x] Keep an opt-in oracle suite that runs `shfmt` from the repo's Nix dev shell.

## Simplify

- [x] Land the first simplify rewrite set in `crates/shuck-formatter/src/simplify.rs`.
- [x] Keep shipped simplify rewrites independently testable and idempotent.
- [ ] Expand simplify coverage only where rewrites stay semantics-safe and source-stable across dialects.
- [ ] Keep adding focused rewrite regressions before enabling broader transformations.
- [ ] Close the remaining `--minify` parity gap around shebang preservation and any compact-layout-only behavior.

## Comment Attachment

- [x] Replace the pure comment stream with `Comments` / `SequenceCommentAttachment` support for leading, trailing, and dangling comments.
- [ ] Anchor comments more directly to AST regions instead of relying mainly on sequence and line-order heuristics.
- [ ] Preserve ambiguous comments with explicit verbatim fallbacks instead of best-effort guessing.
- [ ] Add more regressions for comments around branch boundaries, continuations, compound commands, substitutions, and heredocs.

## Formatter Parity

- [ ] Reduce `verbatim(...)` fallback use in `crates/shuck-formatter/src/command.rs`, especially around compound commands and heredoc-adjacent statements.
- [ ] Finish grouped-command and compound-command layout parity for subshells, brace groups, negated conditions, long boolean lists, and continuation-driven source layouts.
- [ ] Format more multiline command substitutions structurally, especially `$()` bodies that contain heredocs or compound commands.
- [x] Normalize arithmetic, redirect, operator, and continuation spacing so the remaining diffs are policy choices rather than syntax-shape mismatches.
- [ ] Keep improving `case`, `if`, loop, function, and pipeline layout choices to better match `shfmt`.
- [ ] Expand mksh-, POSIX-, and zsh-specific formatting coverage beyond the current round-trip-safe baseline.

## Keep Padding

- [ ] Narrow `keep-padding` fallback regions so safe surrounding code can still be normalized.
- [ ] Track alignment-sensitive spans more explicitly instead of preserving whole statements when padding appears.
- [ ] Add regressions for assignments, declarations, tables of redirects, and mixed-comment alignment.

## Validation

- [x] Grow the `shfmt` oracle fixture set across dialects and option combinations.
- [x] Reuse the benchmark corpus scripts as an in-memory `shfmt` oracle so large real-world deltas stay visible during formatter work.
- [x] Add macro wall-time benchmark targets that compare `shuck format` against `shfmt` on the shared benchmark corpus.
- [x] Add CLI tests for config precedence across nested project roots.
- [x] Add formatter and integration coverage for stdin plus `--stdin-filename` dialect inference.
- [ ] Split oracle expectations more explicitly between default output and option-driven output where that would make failures easier to localize.
- [ ] Keep running the ignored oracle suites and use the remaining mismatches to drive targeted parity fixes.
- [ ] Drive the benchmark oracle failures down from the current broader parity buckets in `fzf-install`, `homebrew-install`, `ruby-build`, `pyenv-python-build`, and `nvm`.

## Nice To Have

- [ ] Add generated formatter glue for `generated.rs` once the node surface stops changing frequently.
- [ ] Revisit whether some formatter decisions should move into parser or AST metadata to avoid source-slice fallbacks.
- [ ] Consider a dedicated formatter fixture corpus under `crates/shuck-formatter/tests/fixtures/`.
- [ ] Document intentional divergences from `shfmt` when Shuck chooses safety over normalization.

## Notes

- The opt-in `shfmt` oracle now compares both targeted fixtures and the five benchmark scripts entirely in memory, with unified diffs truncated to stay readable on large failures.
- The fixture oracle is green aside from the known `minify` shebang divergence and the `function never split` case when the installed `shfmt` binary lacks `-ns`.
- The latest targeted batch improves arithmetic expansions, backslash-continued simple commands, and leading redirect placement from the benchmark corpus.
- Remaining benchmark diffs still include the explicitly deferred inline grouped-command and inline `case ... esac` layout cases, plus the unfinished multiline `$()` and comment-run attachment work.
- Use `make bench-macro-format` for the full formatter corpus and `make bench-macro-format-single BENCH_FILE=/absolute/path/to/script.sh` for a one-off file comparison.
