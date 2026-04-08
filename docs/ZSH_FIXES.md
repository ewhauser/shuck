# Zsh Parse Failure Roadmap

## Snapshot

On 2026-04-07 I refreshed the corpus cache with:

```bash
make ensure-cache
```

Then I ran the dedicated zsh parse harness directly:

```bash
SHUCK_TEST_LARGE_CORPUS=1 \
SHUCK_LARGE_CORPUS_KEEP_GOING=1 \
SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=100 \
nix --extra-experimental-features 'nix-command flakes' develop --command \
  cargo test -p shuck --test large_corpus large_corpus_zsh_fixtures_parse -- \
  --ignored --exact --nocapture
```

That run reported **85 blocking parse failures across 709 zsh fixtures**.

All 85 failures look like parser-front-end work in `crates/shuck-parser` and `crates/shuck-ast`. None of them look like `shuck-linter`, `shuck-syntax`, or `large_corpus.rs` harness bugs.

The per-fixture classification below is a **primary-cause inference** from the reported error plus nearby source context. A few EOF, `fi`, and `}` locations are almost certainly downstream symptoms of an earlier grammar gap in the same file. The roadmap is ordered to collapse those cascades first.

## Layer Legend

- `Command parser`: [`crates/shuck-parser/src/parser/commands.rs`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-parser/src/parser/commands.rs)
- `Word / conditional / arithmetic parser`: [`crates/shuck-parser/src/parser/words.rs`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-parser/src/parser/words.rs) and [`crates/shuck-parser/src/parser/arithmetic.rs`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-parser/src/parser/arithmetic.rs)
- `Lexer / redirect plumbing`: [`crates/shuck-parser/src/parser/lexer.rs`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-parser/src/parser/lexer.rs) and [`crates/shuck-parser/src/parser/redirects.rs`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-parser/src/parser/redirects.rs)
- `AST surface`: [`crates/shuck-ast/src/ast.rs`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-ast/src/ast.rs)

## Classification Codes

| Code | Count | Primary layer | Representative syntax | Planned step |
| --- | ---: | --- | --- | ---: |
| `CMD-1` | 29 | Command parser | `f() { echo hi }`, `sudo(){}`, `function quit() {}` | 1 |
| `CMD-2` | 8 | Command parser | `|| { cmd }`, `&& { cmd }`, `if (( cond )) { cmd }` | 1 |
| `CMD-3` | 4 | Command parser | `elif [[ ... ]]; then` followed only by comments | 1 |
| `AST-1` | 5 | AST surface + command parser | `function { ... }`, `function music itunes()`, punctuated function names | 2 |
| `AST-2` | 3 | AST surface + command parser | `() { ... }` in command position | 2 |
| `LOOP-1` | 11 | AST surface + command parser | `for x ( list )`, `for k v in ...`, `for 1 2 3; do` | 3 |
| `CASE-1` | 7 | Command parser + word parser | `plugin::(disable|enable|load))`, `<->)`, `(#* | <->..<->)` | 4 |
| `CASE-2` | 1 | AST surface + lexer + command parser | `;|` | 4 |
| `EXPR-1` | 8 | Word / conditional / arithmetic parser | `${(Az)LBUFFER}`, `[(Ie)$word]`, `${^$(...)}`, `[[ ... (#b)... ]]` | 5 |
| `EXPR-2` | 2 | Arithmetic parser | `#c` | 5 |
| `FLOW-1` | 4 | Command parser | `} always { ... }` | 6 |
| `EDGE-1` | 3 | Lexer / redirects / command parser | `&|;`, `<file while ...`, quoted word followed by `\` newline | 6 |

## Roadmap

### 1. Normalize short zsh bodies and separator handling

Target codes: `CMD-1`, `CMD-2`, `CMD-3`

This is the highest-leverage step because it should clear **41 of 85** failures and likely remove several cascade errors that currently surface at later `fi`, `}`, and EOF positions.

- Teach zsh mode to accept same-line closing braces where zsh allows them instead of forcing a Bash-style `;` before `}`.
- Cover function bodies, brace groups used after `&&` and `||`, and brace-style `if` bodies.
- Allow empty brace bodies in zsh mode when they appear in otherwise-valid function definitions.
- Allow comment-only `elif` bodies in zsh mode rather than treating them as a syntax error.
- Add focused parser tests before touching the large regression file so the separator rules stay isolated.

### 2. Expand function surface modeling

Target codes: `AST-1`, `AST-2`

This step is the first place where the current AST shape is probably too Bash-shaped to represent the source honestly.

- Extend `FunctionDef` surface handling so zsh-only headers do not have to be squeezed into `name: Name`.
- Decide how to model:
  - nameless `function { ... }`
  - anonymous `() { ... }` commands
  - multi-name headers such as `function music itunes()`
  - punctuated function names such as `cfh.()` and `cfh~()`
  - multi-line function header lists such as the prompt-info stub block
- Prefer explicit surface enums or dedicated variants over silently normalizing everything to one Bash spelling.

### 3. Extend zsh loop grammar and loop AST surface

Target code: `LOOP-1`

The current `ForCommand` shape only captures one loop variable and does not preserve zsh-specific surface forms.

- Add support for parenthesized zsh iteration lists: `for x ( list )`.
- Add support for brace-bodied loop forms such as `for ...; { ... }`.
- Add support for multi-variable iteration headers such as `for k v in ...`.
- Add support for zsh shorthand like `for 1 2 3; do`.
- Keep surface preservation explicit enough that later formatter and rule work can distinguish zsh forms from Bourne forms.

### 4. Finish `case` grammar and case surface preservation

Target codes: `CASE-1`, `CASE-2`

- Broaden zsh `case` pattern parsing to accept the patterns seen in the corpus:
  - parenthesized alternation at the start of a pattern
  - zsh numeric range tokens like `<->`
  - mixed glob-and-alternation forms
- Add the zsh `;|` terminator.
- If the current `CaseTerminator` enum cannot preserve `;|` without lying, extend the AST instead of collapsing it into a Bash spelling.
- Add minimizations for each pattern family to the zsh regression corpus before rerunning the harness.

### 5. Finish zsh word, conditional, and arithmetic expression support

Target codes: `EXPR-1`, `EXPR-2`

This bucket is mostly about zsh flag-rich expressions that already look like words to the lexer but still confuse `words.rs`, `arithmetic.rs`, or the `[[ ... ]]` parser.

- Add or harden support for zsh parameter flags such as `${(Az)...}`, `${(s./.)...}`, `${^...}`, and `${(@)...}`.
- Add or harden support for zsh subscript flags such as `[(Ie)...]`.
- Add or harden support for zsh conditional patterns inside `[[ ... ]]`, including `(#b)` forms and numeric-pattern fragments like `<->`.
- Add arithmetic support for zsh character-literal-style `#c`.
- Keep parser-layer tests split by file so word-parser regressions do not get hidden behind command-parser failures.

### 6. Harden composed control-flow and token-boundary edges

Target codes: `FLOW-1`, `EDGE-1`

The codebase already has parser tests for `always`, zsh brace-`if`, and zsh background operators. The remaining failures in this bucket look like composition bugs rather than entirely missing feature families.

- Fix `always` handling when it is nested inside larger command sequences.
- Fix `&|` handling when followed by a semicolon or used inside compact one-line bodies.
- Fix redirect-prefixed compound commands such as `<file while read ...`.
- Fix the long quoted-word continuation edge seen in the `git-extras` completion script.
- Add one regression test per edge before rerunning the zsh harness.

### 7. Re-run and shrink the regression surface after each pass

- After each roadmap step, re-run the dedicated zsh harness instead of waiting for the full mixed-shell corpus.
- Promote every newly-understood shape into [`crates/shuck-parser/tests/testdata/oils/zsh-large-corpus-regressions.test.sh`](/Users/ewhauser/.codex/worktrees/f1e8/shuck/crates/shuck-parser/tests/testdata/oils/zsh-large-corpus-regressions.test.sh).
- Once the dedicated zsh harness is clean, run `make test-large-corpus` to confirm nothing regressed outside the zsh slice.

## Failure Map

Each fixture below is mapped to one primary code from the legend above.

| Fixture | Code |
| --- | --- |
| `ohmyzsh__ohmyzsh__lib__cli.zsh` | `CASE-1` |
| `ohmyzsh__ohmyzsh__lib__clipboard.zsh` | `EDGE-1` |
| `ohmyzsh__ohmyzsh__lib__functions.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__lib__git.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__lib__prompt_info_functions.zsh` | `AST-1` |
| `ohmyzsh__ohmyzsh__lib__termsupport.zsh` | `CASE-1` |
| `ohmyzsh__ohmyzsh__lib__theme-and-appearance.zsh` | `CASE-1` |
| `ohmyzsh__ohmyzsh__plugins__autoenv__autoenv.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__battery__battery.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__cabal__cabal.plugin.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__plugins__chruby__chruby.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__cloudfoundry__cloudfoundry.plugin.zsh` | `AST-1` |
| `ohmyzsh__ohmyzsh__plugins__colored-man-pages__colored-man-pages.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__command-not-found__command-not-found.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__dash__dash.plugin.zsh` | `EXPR-1` |
| `ohmyzsh__ohmyzsh__plugins__debian__debian.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__extract__extract.plugin.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__plugins__gcloud__gcloud.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__genpass__genpass-apple` | `EXPR-2` |
| `ohmyzsh__ohmyzsh__plugins__genpass__genpass-xkcd` | `EXPR-2` |
| `ohmyzsh__ohmyzsh__plugins__git-extras__git-extras.plugin.zsh` | `EDGE-1` |
| `ohmyzsh__ohmyzsh__plugins__git__git.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__globalias__globalias.plugin.zsh` | `EXPR-1` |
| `ohmyzsh__ohmyzsh__plugins__keychain__keychain.plugin.zsh` | `AST-1` |
| `ohmyzsh__ohmyzsh__plugins__macos__music` | `AST-1` |
| `ohmyzsh__ohmyzsh__plugins__pj__pj.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__rake-fast__rake-fast.plugin.zsh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__plugins__rbenv__rbenv.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__scd__scd` | `EDGE-1` |
| `ohmyzsh__ohmyzsh__plugins__scd__scd.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__screen__screen.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__shrink-path__shrink-path.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__sublime-merge__sublime-merge.plugin.zsh` | `AST-2` |
| `ohmyzsh__ohmyzsh__plugins__term_tab__term_tab.plugin.zsh` | `EXPR-1` |
| `ohmyzsh__ohmyzsh__plugins__urltools__urltools.plugin.zsh` | `CMD-1` |
| `ohmyzsh__ohmyzsh__plugins__virtualenvwrapper__virtualenvwrapper.plugin.zsh` | `AST-1` |
| `ohmyzsh__ohmyzsh__plugins__wd__wd.sh` | `EXPR-1` |
| `ohmyzsh__ohmyzsh__plugins__xcode__xcode.plugin.zsh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__plugins__z__z.plugin.zsh` | `AST-2` |
| `ohmyzsh__ohmyzsh__tools__changelog.sh` | `LOOP-1` |
| `ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh` | `CMD-2` |
| `ohmyzsh__ohmyzsh__tools__upgrade.sh` | `CASE-1` |
| `romkatv__powerlevel10k__config__p10k-classic.zsh` | `CMD-3` |
| `romkatv__powerlevel10k__config__p10k-lean-8colors.zsh` | `CMD-3` |
| `romkatv__powerlevel10k__config__p10k-lean.zsh` | `CMD-3` |
| `romkatv__powerlevel10k__config__p10k-rainbow.zsh` | `CMD-3` |
| `romkatv__powerlevel10k__gitstatus__gitstatus.plugin.zsh` | `EXPR-1` |
| `romkatv__powerlevel10k__gitstatus__mbuild` | `CASE-2` |
| `romkatv__powerlevel10k__internal__configure.zsh` | `FLOW-1` |
| `romkatv__powerlevel10k__internal__p10k.zsh` | `EXPR-1` |
| `romkatv__powerlevel10k__internal__parser.zsh` | `EXPR-1` |
| `romkatv__powerlevel10k__internal__wizard.zsh` | `CMD-1` |
| `romkatv__powerlevel10k__internal__worker.zsh` | `AST-2` |
| `zsh-users__zsh-autosuggestions__src__bind.zsh` | `CASE-1` |
| `zsh-users__zsh-autosuggestions__zsh-autosuggestions.zsh` | `CASE-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__main-highlighter.zsh` | `LOOP-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-loop.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-nested-precommand.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument1.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument2.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument3.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument4.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__array-cmdsep1.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__cmdpos-elision-partial.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__commmand-parameter.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__off-by-one.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__opt-shwordsplit1.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__param-precommand-option-argument1.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__param-precommand-option-argument3.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__precommand-unknown-option.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__precommand4.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-command.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-comment.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection2.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection3.zsh` | `CMD-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__pattern__pattern-highlighter.zsh` | `EXPR-1` |
| `zsh-users__zsh-syntax-highlighting__highlighters__root__root-highlighter.zsh` | `CMD-2` |
| `zsh-users__zsh-syntax-highlighting__tests__generate.zsh` | `FLOW-1` |
| `zsh-users__zsh-syntax-highlighting__tests__tap-colorizer.zsh` | `CASE-1` |
| `zsh-users__zsh-syntax-highlighting__tests__test-highlighting.zsh` | `CMD-2` |
| `zsh-users__zsh-syntax-highlighting__tests__test-perfs.zsh` | `FLOW-1` |
| `zsh-users__zsh-syntax-highlighting__tests__test-zprof.zsh` | `FLOW-1` |
| `zsh-users__zsh-syntax-highlighting__zsh-syntax-highlighting.zsh` | `CMD-1` |
