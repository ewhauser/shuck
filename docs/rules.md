# Rule Implementation Roadmap

## Summary

| Status | Count |
|--------|-------|
| Implemented | 99 |
| Scheduled (Tranches 1-3) | 18 |
| Remaining | 201 |
| **Total** | **318** |

## Difficulty Legend

- **L** (Low) — Simple fact filter or AST pattern match; minimal false-positive logic
- **M** (Medium) — Cross-references multiple facts, needs option parsing, context-aware filtering, or moderate false-positive avoidance
- **H** (High) — Needs new fact infrastructure, semantic/dataflow analysis, cross-function reasoning, or complex scope logic
- Entry markers use two checkboxes: first = implemented, second = vetted (`V`)
- `V` means the rule was reviewed for performance, direct AST traversal in rule files, and duplication; it does not mean the implementation is issue-free
- The checker currently contains additional implemented rules that are not yet enumerated in this roadmap, so vetted markers below apply to the documented implemented rows only

## Scheduled Tranches

These rules are queued for implementation and tracked separately.

**Tranche 1:** ~~C003~~, ~~C004~~, ~~C012~~, C016, C023, C024, C026, C027, C028, C029, C030, C031, C032, C033, C034

**Tranche 2:** ~~C035~~, ~~C036~~, ~~C037~~, ~~C038~~, ~~C039~~, ~~C040~~, ~~C041~~, ~~C042~~, ~~C043~~, C044, C045, C049, C051, C052, C053

**Tranche 3:** ~~C054~~, ~~C056~~, ~~C059~~, ~~C060~~, ~~C061~~, ~~C062~~, ~~C064~~, ~~C065~~, ~~C066~~, ~~C067~~, ~~C068~~, ~~C069~~, ~~C070~~, ~~C071~~, ~~C072~~ *(complete)*

## Vetting Findings

Review scope: all currently dispatched rule entrypoints under `crates/shuck-linter/src/rules/`, with focus on performance costs, direct AST traversal from rule files, and duplication.

- No direct AST-traversal violations were found in rule files during this pass. The architecture guard `cargo test -p shuck-linter rule_modules_avoid_direct_ast_traversal_tokens` passed.

---


## Validation Review (2026-04-10)

The reviewed implemented rules below were checked against three gates:

1. rule logic uses facts APIs only (no direct AST walks or traversal helpers in rule modules)
2. rule logic avoids duplicating command/AST extraction work that belongs in facts
3. test coverage includes both triggering and non-trigger/edge scenarios

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C141 `loop-without-end` | ✅ | ✅ | ✅ positive plus balanced/nested non-trigger tests | vetted |
| C142 `missing-done-in-for-loop` | ✅ | ✅ | ✅ positive plus heredoc/line-continuation EOF and valid `done` negative tests | vetted |
| C143 `dangling-else` | ✅ | ✅ | ✅ positive plus nested empty/non-empty parse-recovery tests | vetted |
| C146 `until-missing-do` | ✅ | ✅ | ✅ positive plus multiline-header and comment/blank-line `do` tests | vetted |
| C157 `if-bracket-glued` | ✅ | ✅ | ✅ positive plus spacing-variant and quoted-text negative tests | vetted |
| K001 `rm-glob-on-variable-path` | ✅ | ✅ | ✅ positive plus safe-`rm`, literal-path, and expansion-precision tests | vetted |
| K004 `find-execdir-with-shell` | ✅ | ✅ | ✅ positive plus `sh -c`, `bash -c`, and safe `-execdir` tests | vetted |

## Validation Review (2026-04-12)

The portability expansion batch from `Implement remaining sh portability rules (#46)` was checked against the same three gates.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| X026 `bash-file-slurp` | ✅ | ✅ | ✅ positive snapshots plus redirect-shape, portable-substitution, and bash-shell negatives | vetted |
| X045 `plus-equals-append` | ✅ | ✅ | ✅ positive snapshots plus arithmetic negative, bash/ksh shell split, and X064 overlap guard | vetted |
| X055 `dollar-string-in-sh` | ✅ | ✅ | ✅ positive snapshots plus embedded-word and bash-shell negative coverage | vetted |
| X064 `plus-equals-in-sh` | ✅ | ✅ | ✅ positive snapshots plus arithmetic negative and bash/ksh shell gating tests | vetted |
| X071 `array-keys-in-sh` | ✅ | ✅ | ✅ positive snapshots plus non-array indirect negatives and X018 overlap guard | vetted |
| X081 `star-glob-removal-in-sh` | ✅ | ✅ | ✅ positive snapshot plus `%`/`$@` non-trigger and bash-shell negative coverage | vetted |

The echo/tr portability batch from `Implement echo and tr portability rules (#80)` was also checked against the same three gates.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| X027 `echo-flags` | ✅ | ✅ | ✅ positive snapshots plus wrapped-echo, plain-operand, and bash-shell negative coverage | vetted |
| X028 `tr-lower-range` | ✅ | ✅ | ✅ positive snapshots plus bracketed/lookalike, wrapped-`tr`, and out-of-scope shell negatives | vetted |
| X029 `tr-upper-range` | ✅ | ✅ | ✅ positive snapshots plus bracketed/lookalike, wrapped-`tr`, and out-of-scope shell negatives | vetted |
| X030 `echo-backslash-escapes` | ✅ | ✅ | ✅ positive snapshots plus `-e`/wrapper/`printf` negatives and shell-target gating tests | vetted |

The POSIX sh function/variable portability batch from `Implement portability function and expansion rules (#81)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| X035 `function-params-in-sh` | ❌ fallback matcher pairs `structural_commands()` entries and rescans source text in the rule file | ❌ parse-diagnostic and fallback ownership is split across separate rule-local matchers | ✅ positive plus brace/subshell/comment/continuation coverage | keep unvetted |
| X067 `hyphenated-function-name` | ✅ | ✅ | ✅ positive plus non-hyphen and shell-gating tests | vetted |
| X072 `unset-pattern-in-sh` | ✅ | ✅ | ✅ positive plus non-prefix, shell-gating, and X018 overlap tests | vetted |
| X077 `nested-default-expansion` | ✅ | ✅ | ❌ only negative coverage today; the rule entrypoint is still a no-op | keep unvetted |

The test/conditional batch from `Implement test/conditional lint rules C082 and C086-C093 (#75)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C082 `escaped-negation-in-test` | ✅ | ✅ | ✅ escaped-negation positives plus literal/non-leading/operator negatives | vetted |
| C086 `greater-than-in-test` | ✅ | ✅ | ✅ redirect-in-bracket positives plus post-test/escaped/quoted/`test`/`[[` negatives | vetted |
| C087 `string-comparison-for-version` | ✅ | ✅ | ✅ dotted-version positives plus integer/plain-string/non-lexical negatives | vetted |
| C088 `mixed-and-or-in-condition` | ✅ | ✅ | ✅ mixed-operator positives plus grouped and single-operator negatives | vetted |
| C089 `quoted-command-in-test` | ✅ | ✅ | ✅ simple-test and `[[ ]]` positives plus negated cases and comparison/unary negatives | vetted |
| C090 `glob-in-test-comparison` | ✅ | ✅ | ✅ unquoted RHS glob positives plus quoted/escaped/non-bracket negatives | vetted |
| C091 `tilde-in-string-comparison` | ✅ | ✅ | ✅ quoted `~/...` positives plus unquoted-tilde and `~user` negatives | vetted |
| C092 `if-dollar-command` | ✅ | ✅ | ✅ if/while/until and compound-condition positives plus wrapper/non-condition negatives | vetted |
| C093 `backtick-in-command-position` | ✅ | ✅ | ✅ command-position positives plus wrapper/quoted/affixed/argument negatives | vetted |

The glob/pattern batch from `Implement glob and pattern matching rules (#74)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C078 `unquoted-globs-in-find` | ✅ | ✅ | ✅ `find -exec` positives plus quoted and non-`find -exec` negatives | vetted |
| C080 `glob-in-grep-pattern` | ✅ | ✅ | ✅ glob-style `*` positives plus regex/fixed-string/non-pattern negatives | vetted |
| C081 `glob-in-string-comparison` | ✅ | ✅ | ✅ standalone-variable positives plus nested-command-substitution and non-pattern negatives | vetted |
| C083 `glob-in-find-substitution` | ✅ | ✅ | ✅ `find` pattern positives plus quoted, escaped, wrapped, and dynamic-pattern negatives | vetted |
| C084 `unquoted-grep-regex` | ✅ | ✅ | ✅ unquoted regex positives plus quoted, `-f`, escaped, and fixed-string negatives | vetted |
| C114 `glob-with-expansion-in-loop` | ✅ | ✅ | ✅ loop-prefix positives plus quoted-prefix, no-glob, and brace-expansion negatives | vetted |
| S055 `glob-assigned-to-variable` | ✅ | ✅ | ✅ assignment positives plus quoted, escaped, scalar, and compound-assignment negatives | vetted |

The quoting/expansion batch from `Implement quoting and expansion lint rules (#76)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C096 `unquoted-pipe-in-echo` | ✅ | ✅ | ✅ escaped pipe/brace positives plus quoted and non-echo negatives | vetted |
| C099 `quoted-array-slice` | ✅ | ✅ | ✅ scalar-binding positives plus unquoted, non-slice, and array-assignment negatives | vetted |
| C100 `quoted-bash-source` | ✅ | ✅ | ✅ quoted unindexed positives plus unquoted, indexed, and modifier negatives | vetted |
| C105 `export-with-positional-params` | ✅ | ✅ | ✅ export-splat positives plus assignment, non-export, and non-splat negatives | vetted |
| C111 `at-sign-in-string-compare` | ✅ | ✅ | ✅ simple-test and `[[ ]]` positives plus array, escaped, and `$*` negatives | vetted |
| C112 `array-slice-in-comparison` | ✅ | ✅ | ✅ string-comparison positives plus full-array, escaped, and `[` negatives | vetted |
| S014 `unquoted-dollar-star` | ✅ | ✅ | ✅ command/list/name positives plus quoted, affixed, assignment, and test negatives | vetted |
| S015 `quoted-dollar-star-loop` | ✅ | ✅ | ✅ for-list positives plus select and non-`*` negatives | vetted |
| S017 `unquoted-array-split` | ✅ | ✅ explicit S018 handoff for command substitutions | ✅ positives plus keyed, quoted, safe-special, and overlap-guard negatives | vetted |
| S018 `command-output-array-split` | ✅ | ✅ paired with S017 handoff coverage | ✅ unquoted substitution positives plus quoted and non-split negatives | vetted |
| S021 `positional-args-in-string` | ✅ | ✅ | ✅ command/name string-folding positives plus pure-splat and assignment negatives | vetted |
| S050 `unquoted-word-between-quotes` | ✅ | ✅ | ✅ literal-middle positives plus punctuation, escaped, and dynamic-middle negatives | vetted |
| S052 `unquoted-variable-in-test` | ✅ | ✅ | ✅ unary `-n` positives plus quoted, `-z`, literal, and substitution negatives | vetted |
| S058 `unquoted-path-in-mkdir` | ✅ | ✅ | ✅ mkdir-path positives plus quoted-path and mode-operand negatives | vetted |
| S062 `default-value-in-colon-assign` | ✅ | ✅ | ✅ `:` positives plus non-colon, non-default, and quoted negatives | vetted |
| S067 `backtick-output-to-command` | ✅ | ✅ | ✅ command-argument positives plus quoted, `$(...)`, and assignment negatives | vetted |
| S070 `double-quote-nesting` | ✅ | ✅ | ✅ scalar/substitution positives plus array, arithmetic, and non-nested negatives | vetted |
| S071 `env-prefix-quoting` | ✅ | ✅ | ✅ env-prefix positives plus behavior-changing and non-prefix negatives | vetted |
| S076 `mixed-quote-word` | ✅ | ✅ | ✅ command/assignment/test/case positives plus separator, dynamic, and single-quote negatives | vetted |

The array-operation batch from `Implement array operation correctness rules (#70)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C106 `append-to-array-as-string` | ✅ semantic-model binding lookup plus scalar-binding facts, with no rule-local AST/source rescans | ✅ | ✅ positives plus non-array, element-append, and local-shadow negatives | vetted |
| C108 `unset-associative-array-element` | ❌ rule-local operand parsing reparses raw `unset name[key]` text and quote state in the rule file | ✅ | ✅ associative positives plus indexed, unquoted-key, and quoted-whole-word negatives | keep unvetted |
| C133 `array-to-string-conversion` | ✅ semantic resolution plus scalar-binding and word facts, with no rule-local AST/source rescans | ✅ | ✅ positives plus unknown-name, scalar-self-reference, and shadowing negatives | vetted |
| C148 `broken-assoc-key` | ✅ | ✅ | ✅ positives plus valid assoc, indexed, and dynamic-key negatives | vetted |
| C151 `comma-array-elements` | ✅ | ✅ | ✅ comma positives plus quoted and brace-expansion negatives | vetted |

The command-specific style batch from `Implement style rules S061, S063, S064, and S068 (#85)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| S061 `fgrep-deprecated` | ✅ | ✅ | ✅ plain-call positives plus wrapper and `grep -F` negatives | vetted |
| S063 `relative-symlink-target` | ❌ rule-local `ln` option and operand parsing reconstructs symlink-target selection in the rule file | ❌ `ln` parsing ownership sits outside facts/shared option data | ✅ deep-relative positives plus non-deep, dynamic, non-symbolic, and zsh negatives | keep unvetted |
| S064 `xargs-with-inline-replace` | ✅ consumes parsed xargs option facts | ✅ | ✅ inline `-i` positives plus modern `-I`/`--replace` and null-input negatives | vetted |
| S068 `trap-signal-numbers` | ✅ filters command facts and uses the shared trap-arg parser | ✅ shared `trap` parsing is already centralized | ✅ numeric-signal positives plus symbolic, listing-mode, and shell-gating negatives | vetted |

The shebang/script-structure batch from `Implement shebang and script structure rules (#86)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C073 `indented-shebang` | ✅ | ✅ centralized in shared shebang-header facts | ✅ positive plus non-header and malformed-header negatives | vetted |
| C074 `space-after-hash-bang` | ✅ | ✅ centralized in shared shebang-header facts | ✅ space/tab positives plus valid and non-header negatives | vetted |
| C075 `shebang-not-on-first-line` | ✅ | ✅ centralized in shared shebang-header facts | ✅ second-line positives plus non-header, malformed, and later-line negatives | vetted |
| S043 `missing-shebang-line` | ✅ | ✅ centralized in shared shebang-header facts | ✅ comment-header positives plus directive, known-shell, and malformed-header negatives | vetted |
| S053 `duplicate-shebang-flag` | ✅ | ✅ centralized in shared shebang-header facts | ✅ repeated-flag positives plus distinct-flag and non-header negatives | vetted |

The boolean/short-circuit batch from `Implement boolean logic and short-circuit rules (#73)` was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C079 `short-circuit-fallthrough` | ✅ filters preclassified mixed short-circuit list facts | ✅ shared ternary/list ownership stays in facts | ✅ fallthrough positives plus exemption, assignment-shape, and other-chain negatives | vetted |
| C115 `default-else-in-short-circuit` | ✅ filters preclassified assignment-ternary list facts | ✅ shared ternary/list ownership stays in facts | ✅ fallback-anchor positive plus reversed-chain and non-ternary negatives | vetted |
| S020 `single-iteration-loop` | ✅ filters loop-header facts and expansion analysis | ✅ | ✅ single-item positives plus glob, splat, dynamic, and multi-field negatives | vetted |
| S032 `conditional-assignment-shortcut` | ✅ filters list facts and assignment-only segment facts | ✅ ownership stays separated from C115 via list kind/segment shape checks | ✅ shortcut positives plus assignment-ternary, generic-command, and assignment-only negatives | vetted |

The function/scope batch was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C097 `function-called-without-args` | ❌ rule-local semantic scans resolve function scopes, visible bindings, and call arity instead of consuming a shared fact/helper | ❌ binding/call-site resolution is duplicated with C123, including shared span trimming and visibility logic | ✅ rich positive and negative coverage for guarded params, resets, redefinitions, shadowing, and nested scopes | keep unvetted |
| C123 `function-references-unset-param` | ❌ rule-local semantic scans resolve function scopes, visible bindings, and call arity instead of consuming a shared fact/helper | ❌ binding/call-site resolution is duplicated with C097, including shared span trimming and visibility logic | ✅ rich positive and negative coverage for guarded params, resets, mixed arity, redefinitions, shadowing, and nested scopes | keep unvetted |
| C125 `cd-without-error-check-in-func` | ✅ filters command facts plus flow/scope data through the shared directory-change helper | ✅ function-specific ownership stays coordinated with the general unchecked-directory-change rule | ✅ helper positives plus function-specific overlap and shell-gating tests | vetted |
| C126 `continue-outside-loop-in-func` | ✅ filters the shared loop-control helper over command facts and flow context | ✅ overlap with the general loop-control rule is explicitly handed off in one place | ✅ positive plus loop/top-level negatives and dual-rule overlap coverage | vetted |
| C131 `variable-as-command-name` | ✅ filters expansion-word facts and command facts with lightweight semantic scope checks | ✅ | ✅ positive plus quoted/top-level and command-substitution negatives | vetted |
| C147 `keyword-function-name` | ✅ filters function-header facts only | ✅ | ✅ POSIX and bash-reserved-word positives plus ordinary-name negatives | vetted |
| S038 `redundant-return-status` | ✅ consumes precomputed redundant-return spans from facts | ✅ ownership stays centralized in facts | ✅ terminal/non-terminal, branch, compound-command, and control-flow negatives | vetted |
| S041 `function-body-without-braces` | ✅ consumes precomputed function-body spans from facts | ✅ ownership stays centralized in facts | ✅ bare-compound positives plus braced-body negatives | vetted |
| S066 `local-declare-combined` | ✅ filters declaration facts only | ✅ | ✅ combined-declaration positives plus plain declaration and shell-gating negatives | vetted |

The case/getopts batch was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C128 `case-glob-reachability` | ✅ consumes shared case-pattern shadow facts only | ✅ shadow analysis stays centralized in facts and cleanly hands off target anchoring to C129 | ✅ positives plus unanalyzable/extglob, escaped-wildcard, quoted-fragment, and fallthrough coverage | vetted |
| C129 `case-default-before-glob` | ✅ consumes shared case-pattern shadow facts only | ✅ shadow analysis stays centralized in facts and cleanly hands off source anchoring to C128 | ✅ positives plus unanalyzable/extglob, escaped-wildcard, first-later-only, and fallthrough coverage | vetted |
| C134 `getopts-option-not-in-case` | ✅ consumes shared getopts/case facts only | ✅ missing-option ownership stays centralized in facts and separated from C135/S069 by fact shape | ✅ positives plus fallback, early-return fallback, non-target case, and branch/function-local handler negatives | vetted |
| C135 `case-arm-not-in-getopts` | ✅ consumes shared getopts/case facts only | ✅ unexpected/invalid-arm ownership stays centralized in facts and separated from C134/S069 by fact shape | ✅ positives plus fallback/special-arm negatives, invalid-static-pattern coverage, and branch/function-local handler negatives | vetted |
| S069 `single-letter-case-label` | ✅ consumes shared getopts/case facts only | ✅ style-only follow-up stays scoped to incomplete bare-label handlers after C134/C135 filters | ✅ positives plus quoted-label, complete-handler, fallback, unexpected-arm, invalid-pattern, and branch/function-local handler negatives | vetted |

The subshell/pipeline side-effect batch was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C107 `dollar-question-after-command` | ✅ consumes precomputed status-followup spans from facts | ✅ ownership for intervening-command and nested followup analysis stays centralized in facts | ✅ output-command positives plus immediate-check, saved-status, case-subject, short-circuit, and pipeline coverage | vetted |
| C150 `subshell-local-assignment` | ✅ consumes precomputed nonpersistent later-use spans from facts | ✅ shared nonpersistent-assignment analysis cleanly separates later-use anchoring from C155 assignment-site anchoring | ✅ subshell and command-substitution positives plus pipeline exclusion, local-declaration, same-scope-read, and parent-reset negatives | vetted |
| C155 `subshell-side-effect` | ✅ consumes precomputed nonpersistent assignment-site spans from facts | ✅ shared nonpersistent-assignment analysis cleanly separates assignment-site anchoring from C150 later-use anchoring | ✅ subshell, pipeline, and command-substitution positives plus no-later-use, local-declaration, and parent-reset negatives | vetted |
| C156 `possible-variable-misspelling` | ✅ semantic unresolved-reference scan plus shared variable-reference filter, with no AST/source rescans | ✅ shared filtering logic is centralized with the undefined-variable path; rule-local work is limited to candidate ranking | ✅ fold-match and `X`-prefix positives plus duplicate-name, runtime-name, short-name, mixed-case, and later-definition negatives | vetted |

The heredoc batch was reviewed next.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C127 `unused-heredoc` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ heredoc ownership stays centralized in facts | ✅ positives plus command-attached negatives | vetted |
| C138 `heredoc-missing-end` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ heredoc ownership stays centralized in facts | ✅ unclosed marker positives plus empty-delimiter and no-trailing-newline closure coverage | vetted |
| C144 `heredoc-closer-not-alone` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ overlap with C145 is resolved centrally by only flagging content-prefixed closer lines here | ✅ plain and tab-stripped positives plus proper-close negatives | vetted |
| C145 `misquoted-heredoc-close` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ overlap with C144 is resolved centrally by excluding content-prefixed closer lines from this rule | ✅ quoted-close positive plus overlap-avoidance and proper-close negatives | vetted |
| S030 `heredoc-end-space` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ style-only trailing-space ownership stays centralized in facts | ✅ space/tab/tab-strip positives plus proper-close and first-bad-only negatives | vetted |
| S033 `echo-here-doc` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ command-specific heredoc misuse ownership stays centralized in facts | ✅ plain and tab-strip positives plus non-echo negatives | vetted |
| S073 `spaced-tabstrip-close` | ✅ consumes precomputed heredoc spans from the shared heredoc summary | ✅ style-only `<<-` closer ownership stays centralized in facts | ✅ spaced/mixed-indent positives plus tab-only and plain-heredoc negatives | vetted |

The remaining structural parse-diagnostic rows from the older `2026-04-10` batch were reconfirmed next so their roadmap markers could be synced.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C142 `missing-done-in-for-loop` | ✅ parse-diagnostic routing stays centralized in `parse_diagnostics.rs` | ✅ missing-`done` ownership stays shared with the parse-diagnostic classifier | ✅ dedicated parse-diagnostic positives plus heredoc, line-continuation, and valid-`done` negatives | vetted |
| C143 `dangling-else` | ✅ parse-diagnostic routing stays centralized in `parse_diagnostics.rs` | ✅ empty-`else` ownership stays shared with the parse-diagnostic classifier | ✅ dedicated parse-diagnostic positives plus nested empty/non-empty recovery coverage | vetted |
| C146 `until-missing-do` | ✅ parse-diagnostic routing stays centralized in `parse_diagnostics.rs` | ✅ missing-`do` ownership stays shared with the parse-diagnostic classifier | ✅ dedicated parse-diagnostic positives plus multiline-header and comment/blank-line `do` negatives | vetted |
| C157 `if-bracket-glued` | ✅ parse-diagnostic routing stays centralized in `parse_diagnostics.rs` | ✅ glued-`if[` ownership stays shared with the parse-diagnostic classifier | ✅ dedicated parse-diagnostic positives plus spacing variants, quoted text, comments, and parameter-expansion negatives | vetted |

The remaining security rows from the older `2026-04-10` batch were reconfirmed next so their roadmap markers could be synced.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| K001 `rm-glob-on-variable-path` | ✅ filters parsed `rm` option facts and zsh glob-setting facts only | ✅ dangerous-path extraction stays centralized in shared command facts | ✅ positives plus safe-`rm`, literal-path, expansion-shape, and indirect-expansion negatives | vetted |
| K004 `find-execdir-with-shell` | ✅ filters parsed `find -execdir` shell-command facts only | ✅ `find -execdir` shell-script extraction stays centralized in shared command facts | ✅ `sh -c` and `bash -c` positives plus safe helper and non-interpolating shell negatives | vetted |

## Validation Review (2026-04-14)

The remaining implemented test/conditional and redirection rows that were not yet called out above were reviewed next against the same three gates.

| Rule | Facts-only / no walks | Duplication | Coverage status | Outcome |
|---|---|---|---|---|
| C102 `glob-in-test-directory` | ✅ filters simple-test and conditional facts plus shared unquoted-glob helpers only | ✅ file-test operator and glob-shape detection stay centralized in shared helpers | ✅ positives plus quoted and non-file-test negatives | vetted |
| C110 `constant-in-test-assignment` | ✅ filters simple-test/conditional facts and word facts only | ✅ assignment-looking operand detection stays centralized in shared word-classification helpers | ✅ positives plus literal and unexpanded negatives | vetted |
| C118 `malformed-arithmetic-in-condition` | ✅ filters simple-test/conditional facts and word facts only | ✅ arithmetic/operator classification stays shared through word facts and conditional operator families | ✅ positives plus arithmetic-expansion and plain-comparison negatives | vetted |
| C120 `expr-substr-in-test` | ✅ filters command, substitution, and test facts only | ✅ `expr` option parsing and substitution ownership stay centralized in shared facts | ✅ positives plus non-substr and non-test negatives | vetted |
| C121 `string-compared-with-eq` | ✅ filters simple-test/conditional facts plus shared word/arithmetic classifiers only | ✅ string-vs-arithmetic discrimination stays centralized in shared helpers | ✅ positives plus numeric and non-`-eq` negatives | vetted |
| C122 `a-flag-in-double-bracket` | ✅ filters conditional unary facts only | ✅ operator classification stays centralized in conditional facts | ✅ positives plus bracket-test and other-unary negatives | vetted |
| S011 `compound-test-operator` | ✅ filters simple-test facts and malformed-bracket guard facts only | ✅ compound-operator span extraction stays on the shared simple-test fact | ✅ positives plus plain-test and malformed-bracket negatives | vetted |
| S065 `x-prefix-in-test` | ✅ filters simple-test/conditional facts plus the shared literal-prefix helper only | ✅ x-prefix detection stays centralized in the shared prefix helper | ✅ positives plus single-sided and non-prefix negatives | vetted |
| C085 `stderr-before-stdout-redirect` | ✅ filters redirect facts only | ✅ malformed numeric-target detection stays scoped to redirect facts without duplicating path or pipeline analysis | ✅ positives plus standard and non-numeric redirect negatives | vetted |
| C094 `redirect-clobbers-input` | ✅ filters structural command facts plus shared comparable-path expansion only | ✅ read/write path normalization stays centralized in shared expansion helpers | ✅ matching-path positives plus echo/printf, distinct-path, and safe-sort negatives | vetted |
| C119 `redirect-before-pipe` | ✅ filters pipeline and redirect facts only | ✅ stderr-dup overlap handling stays scoped to shared redirect analysis | ✅ positives plus stderr-only, pipeall, and non-file-target negatives | vetted |
| S075 `combine-appends` | ✅ filters statement/pipeline/redirect facts plus shared comparable-path expansion only | ✅ append-target normalization stays centralized in shared comparable-path helpers | ✅ three-command-run positives plus grouped, two-command, and case-arm negatives | vetted |

## Post-Implementation Cleanup

- [x] Validate the portability expansion batch (`X026`, `X045`, `X055`, `X064`, `X071`, `X081`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the echo/tr portability batch (`X027`, `X028`, `X029`, `X030`) for facts-only rule logic, shared-helper scope, and regression coverage.
- [x] Validate the POSIX sh function/variable portability rules that already clear the review bar (`X067`, `X072`).
- [x] Validate the first implemented test/conditional batch (`C082`, `C086`, `C087`, `C088`, `C089`, `C090`, `C091`, `C092`, `C093`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the remaining implemented test/conditional rules that already clear the review bar (`C102`, `C110`, `C118`, `C120`, `C121`, `C122`, `S011`, `S065`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the glob/pattern batch (`C078`, `C080`, `C081`, `C083`, `C084`, `C114`, `S055`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the quoting/expansion batch (`C096`, `C099`, `C100`, `C105`, `C111`, `C112`, `S014`, `S015`, `S017`, `S018`, `S021`, `S050`, `S052`, `S058`, `S062`, `S067`, `S070`, `S071`, `S076`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the array-operation rules that already clear the review bar (`C106`, `C133`, `C148`, `C151`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the command-specific style rules that already clear the review bar (`S061`, `S064`, `S068`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the shebang/script-structure batch (`C073`, `C074`, `C075`, `S043`, `S053`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the remaining implemented redirection/pipe rules that already clear the review bar (`C085`, `C094`, `C119`, `S075`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the boolean/short-circuit batch (`C079`, `C115`, `S020`, `S032`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the function/scope rules that already clear the review bar (`C125`, `C126`, `C131`, `C147`, `S038`, `S041`, `S066`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the case/getopts batch (`C128`, `C129`, `C134`, `C135`, `S069`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the subshell/pipeline side-effect batch (`C107`, `C150`, `C155`, `C156`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Validate the heredoc batch (`C127`, `C138`, `C144`, `C145`, `S030`, `S033`, `S073`) for facts-only rule logic, overlap ownership, and regression coverage.
- [x] Sync the already-reviewed structural parse-diagnostic roadmap markers (`C142`, `C143`, `C146`, `C157`) after reconfirming their shared parse-diagnostic ownership and regression coverage.
- [x] Sync the already-reviewed security roadmap markers (`K001`, `K004`) after reconfirming their shared command-fact ownership and regression coverage.
- [ ] Move `C097` function binding and call-arity resolution into facts or a shared semantic helper so the rule file stops duplicating visible-binding lookup and function-span trimming.
- [ ] Move `C123` function binding and call-arity resolution into facts or a shared semantic helper so the rule file stops duplicating visible-binding lookup and function-span trimming.

## Remaining Rules

### Portability — Bash Conditionals in sh

Detect bash-specific test/conditional syntax in POSIX sh scripts. All share a
common pattern: verify dialect is sh, detect the specific syntax form in
conditional facts.

- [x] [x] **L** X001 (SC3010) `double-bracket-in-sh` — `[[ ]]` conditional not portable to sh
- [x] [x] **L** X002 (SC3014) `test-equality-operator` — `==` inside `[` not portable
- [x] [x] **L** X033 (SC3011) `if-elif-bash-test` — `[[ ]]` in elif clause
- [x] [x] **L** X034 (SC2221) `extended-glob-in-test` — extended glob in `[[` match
- [x] [x] **L** X040 (SC2102) `array-subscript-test` — array subscript in `[` test
- [x] [x] **L** X041 (SC2103) `array-subscript-condition` — array subscript in `[[ ]]`
- [x] [x] **L** X046 (SC2269) `extglob-in-test` — extended glob in test bracket
- [x] [x] **L** X058 (SC3065) `greater-than-in-double-bracket` — `>` inside `[[ ]]` in sh
- [x] [x] **L** X059 (SC3066) `regex-match-in-sh` — `=~` regex match in sh
- [x] [x] **L** X060 (SC3067) `v-test-in-sh` — `-v` variable-is-set test in sh
- [x] [x] **L** X061 (SC3068) `a-test-in-sh` — `-a` file test inside `[[ ]]` in sh
- [x] [x] **L** X073 (SC3080) `option-test-in-sh` — `-o` option test in `[[ ]]` in sh
- [x] [x] **L** X074 (SC3081) `sticky-bit-test-in-sh` — `-k` sticky-bit test in sh
- [x] [x] **L** X075 (SC3082) `ownership-test-in-sh` — `-O` ownership test in sh

### Portability — Bash Keywords and Builtins in sh

Detect bash-specific keywords and builtins used in POSIX sh. Simple command-name
or keyword checks gated on dialect.

- [x] [x] **L** X003 (SC3043) `local-variable-in-sh` — `local` in sh
- [x] [x] **L** X004 (SC2112) `function-keyword` — `function` keyword in sh
- [x] [x] **L** X015 (SC3042) `let-command` — `let` in sh
- [x] [x] **L** X016 (SC3044) `declare-command` — `declare` in sh
- [x] [x] **L** X031 (SC3046) `source-builtin-in-sh` — `source` instead of `.` in sh
- [x] [x] **L** X052 (SC2321) `function-keyword-in-sh` — `function` with parens in sh
- [x] [x] **L** X080 (SC3084) `source-inside-function-in-sh` — `source` inside function in sh

### Portability — Bash Expansion Syntax in sh

Detect bash-specific parameter expansion, process substitution, arrays, and
related syntax in POSIX sh. Mostly surface-level AST node type checks.

- [x] [x] **L** X006 (SC3001) `process-substitution` — `<()` / `>()` in sh
- [x] [x] **L** X007 (SC3003) `ansi-c-quoting` — `$'...'` in sh
- [x] [x] **L** X010 (SC3009) `brace-expansion` — `{a,b}` expansion in sh
- [x] [x] **L** X011 (SC3011) `here-string` — `<<<` in sh
- [x] [x] **L** X013 (SC3030) `array-assignment` — array variable assignment in sh
- [x] [x] **L** X018 (SC3053) `indirect-expansion` — `${!var}` in sh
- [x] [x] **L** X019 (SC3054) `array-reference` — array reference in sh
- [x] [x] **L** X023 (SC3057) `substring-expansion` — `${var:offset:len}` in sh
- [x] [x] **L** X024 (SC3059) `uppercase-expansion` — case-modification expansion in sh
- [x] [x] **L** X025 (SC3060) `replacement-expansion` — replacement expansion in sh
- [x] [x] **L** X026 (SC3024) `bash-file-slurp` — `$(< file)` in sh
- [x] [x] **L** X045 (SC3055) `plus-equals-append` — `+=` assignment in sh
- [x] [x] **L** X055 (SC3062) `dollar-string-in-sh` — `$"string"` in sh
- [x] [x] **L** X064 (SC3071) `plus-equals-in-sh` — `+=` append operator in sh
- [x] [x] **L** X071 (SC3078) `array-keys-in-sh` — `${!arr[*]}` in sh
- [x] [x] **L** X081 (SC3085) `star-glob-removal-in-sh` — `${*%%pattern}` in sh

### Portability — Bash Control Flow in sh

Detect bash-specific control flow constructs in POSIX sh.

- [x] [x] **L** X005 (SC3058) `bash-case-fallthrough` — `;&` / `;;&` in case
- [x] [x] **L** X008 (SC3018) `standalone-arithmetic` — `(( ))` command in sh
- [x] [x] **L** X009 (SC3033) `select-loop` — `select` loop in sh
- [x] [x] **L** X014 (SC3007) `coproc` — `coproc` in sh
- [x] [x] **L** X056 (SC3063) `c-style-for-in-sh` — `for ((...))` in sh
- [x] [x] **L** X057 (SC3064) `legacy-arithmetic-in-sh` — `$[...]` in sh
- [x] [x] **L** X062 (SC3069) `c-style-for-arithmetic-in-sh` — C-style for arithmetic in sh

### Portability — Bash Redirection and Pipes in sh

Detect bash-specific redirection and pipe operators in POSIX sh.

- [x] [x] **L** X012 (SC3052) `ampersand-redirection` — `&>` combined redirect in sh
- [x] [x] **L** X020 (SC3050) `brace-fd-redirection` — `{fd}>` brace-based FD in sh
- [x] [x] **L** X063 (SC3070) `ampersand-redirect-in-sh` — `>&` combined redirect in sh
- [x] [x] **L** X066 (SC3073) `pipe-stderr-in-sh` — `|&` pipe-stderr in sh

### Portability — Bash Options and Traps in sh

Detect bash-specific set/trap options in POSIX sh.

- [x] [x] **L** X017 (SC3047) `trap-err` — trapping ERR in sh
- [x] [x] **L** X021 (SC3040) `pipefail-option` — `set -o pipefail` in sh
- [x] [x] **L** X022 (SC3048) `wait-option` — wait flags in sh
- [x] [x] **L** X032 (SC3025) `printf-q-format-in-sh` — `%q` printf conversion in sh
- [x] [x] **L** X068 (SC3075) `errexit-trap-in-sh` — `set -E` in sh
- [x] [x] **M** X069 (SC3076) `signal-name-in-trap` — symbolic signal names in trap
- [x] [x] **L** X070 (SC3077) `base-prefix-in-arithmetic` — `10#` base prefix in sh

### Portability — Extended Glob Patterns

Detect extended glob syntax in contexts where it is not supported.

- [x] [x] **L** X037 (SC1075) `extglob-case` — non-POSIX case pattern syntax
- [x] [x] **L** X048 (SC2277) `extglob-in-case-pattern` — extended-glob alternation in case
- [x] [x] **L** X054 (SC3061) `extglob-in-sh` — `@()` extended glob in sh
- [x] [x] **L** X065 (SC3072) `caret-negation-in-bracket` — `[^...]` negation in sh

### Portability — Echo, tr, and printf Locale

Detect locale-dependent and non-portable echo/tr behavior.

- [x] [x] **L** X027 (SC3037) `echo-flags` — echo flags depend on shell implementation
- [x] [x] **L** X028 (SC2018) `tr-lower-range` — locale-dependent lower-case tr range
- [x] [x] **L** X029 (SC2019) `tr-upper-range` — locale-dependent upper-case tr range
- [x] [x] **M** X030 (SC2028) `echo-backslash-escapes` — echo backslash escapes are non-portable

### Portability — POSIX sh Function and Variable Syntax

Detect non-portable function definitions and variable operations.

- [x] **L** X035 (SC1065) `function-params-in-sh` — parameter syntax in sh function
- [x] [x] **L** X067 (SC3074) `hyphenated-function-name` — hyphen in function name
- [x] [x] **L** X072 (SC3079) `unset-pattern-in-sh` — pattern-based unset in sh
- [x] **M** X077 (SC3083) `nested-default-expansion` — nested default-value expansion in sh

### Portability — Zsh-specific Syntax

Detect zsh-only syntax in scripts targeting other shells.

- [x] [x] **L** X036 (SC1070) `zsh-redir-pipe` — zsh-only redirection operator
- [x] [x] **L** X038 (SC1129) `zsh-brace-if` — zsh-style conditional bracing
- [x] [x] **L** X039 (SC1130) `zsh-always-block` — zsh `always` block
- [x] [x] **L** X042 (SC2240) `sourced-with-args` — sourced file with extra args
- [x] [x] **L** X043 (SC2251) `zsh-flag-expansion` — zsh-only parameter expansion form
- [x] [x] **L** X044 (SC2252) `nested-zsh-substitution` — nested zsh-style expansion
- [x] [x] **M** X047 (SC2275) `multi-var-for-loop` — for loop binds multiple variables
- [x] [x] **L** X049 (SC2278) `zsh-prompt-bracket` — zsh prompt escape in sh
- [x] [x] **L** X050 (SC2279) `csh-syntax-in-sh` — csh-style set assignment in sh
- [x] [x] **L** X051 (SC2313) `zsh-nested-expansion` — zsh nested parameter expansion
- [x] [x] **L** X053 (SC2355) `zsh-assignment-to-zero` — assigning to `$0` (zsh idiom)
- [x] [x] **L** X076 (SC2359) `zsh-parameter-flag` — zsh parameter flag in sh
- [x] [x] **L** X078 (SC2371) `zsh-array-subscript-in-case` — zsh array subscript in case
- [x] [x] **L** X079 (SC2375) `zsh-parameter-index-flag` — zsh parameter index flag

### Test and Conditional Expressions

Rules about `[`, `[[`, test operators, and conditional structure. Use
`simple_test()` and `conditional()` facts.

- [x] [x] **L** C082 (SC2302) `escaped-negation-in-test` — backslash-escaped `!` in test
- [x] [x] **M** C086 (SC2308) `greater-than-in-test` — `>` in `[ ]` creates file instead of comparing
- [x] [x] **M** C087 (SC2309) `string-comparison-for-version` — `<` in `[[ ]]` compares lexicographically
- [x] [x] **M** C088 (SC2310) `mixed-and-or-in-condition` — `&&`/`||` without grouping in `[[ ]]`
- [x] [x] **M** C089 (SC2311) `quoted-command-in-test` — pipeline quoted as string in test
- [x] [x] **M** C090 (SC2312) `glob-in-test-comparison` — glob on RHS of `==` in `[ ]`
- [x] [x] **M** C091 (SC2314) `tilde-in-string-comparison` — literal tilde in quoted comparison
- [x] [x] **M** C092 (SC2315) `if-dollar-command` — command substitution output as condition
- [x] [x] **M** C093 (SC2316) `backtick-in-command-position` — backtick substitution as command name
- [x] [x] **M** C102 (SC2331) `glob-in-test-directory` — glob in `[ -d ]` test
- [x] [x] **M** C110 (SC2341) `constant-in-test-assignment` — `=` in test looks like assignment
- [x] [x] **M** C118 (SC2357) `malformed-arithmetic-in-condition` — malformed arithmetic in condition
- [x] [x] **M** C120 (SC2360) `expr-substr-in-test` — `expr substr` inside test
- [x] [x] **M** C121 (SC2361) `string-compared-with-eq` — string compared with `-eq`
- [x] [x] **L** C122 (SC2363) `a-flag-in-double-bracket` — `-a` in `[[ ]]` is ambiguous
- [x] [x] **M** S011 (SC2166) `compound-test-operator` — `-a`/`-o` inside `[` expression
- [x] [x] **L** S065 (SC2351) `x-prefix-in-test` — `x$var` idiom for empty-string safety

### Glob and Pattern Matching

Rules about glob expansion in command arguments, find, grep, and comparisons.
Filter command facts and word facts for unquoted glob characters.

- [x] [x] **M** C078 (SC2295) `unquoted-globs-in-find` — unquoted variable+glob in find -exec
- [x] [x] **M** C080 (SC2299) `glob-in-grep-pattern` — glob character in grep pattern
- [x] [x] **M** C081 (SC2301) `glob-in-string-comparison` — variable in string comparison treated as glob
- [x] [x] **M** C083 (SC2304) `glob-in-find-substitution` — glob in find command substitution
- [x] [x] **M** C084 (SC2305) `unquoted-grep-regex` — grep regex may be glob-expanded
- [x] [x] **M** C114 (SC2349) `glob-with-expansion-in-loop` — glob+variable in for loop
- [x] [x] **M** S055 (SC2326) `glob-assigned-to-variable` — glob pattern assigned without quoting

### Quoting and Expansion

Rules about missing or incorrect quoting, word splitting, and expansion
contexts. Use word facts and expansion word facts.

- [x] [x] **M** C096 (SC2320) `unquoted-pipe-in-echo` — pipe/brace in echo may be interpreted
- [x] [x] **M** C099 (SC2325) `quoted-array-slice` — quoted array slice prevents splitting
- [x] [x] **M** C100 (SC2327) `quoted-bash-source` — `$BASH_SOURCE` quoted without array syntax
- [x] [x] **M** C105 (SC2334) `export-with-positional-params` — export with `$@`
- [x] [x] **M** C111 (SC2344) `at-sign-in-string-compare` — `$@` in string comparison folds args
- [x] [x] **M** C112 (SC2345) `array-slice-in-comparison` — array slice in string comparison
- [x] [x] **M** S014 (SC2048) `unquoted-dollar-star` — `$*` without quotes
- [x] [x] **M** S015 (SC2066) `quoted-dollar-star-loop` — `"$*"` in loop turns args into one item
- [x] [x] **M** S017 (SC2206) `unquoted-array-split` — unquoted value split into array
- [x] [x] **M** S018 (SC2207) `command-output-array-split` — raw command output into array
- [x] [x] **M** S021 (SC2145) `positional-args-in-string` — positional params folded into string
- [x] [x] **L** S050 (SC2300) `unquoted-word-between-quotes` — unquoted word between single-quoted segments
- [x] [x] **M** S052 (SC2307) `unquoted-variable-in-test` — unquoted variable in `[ -n ]`
- [x] [x] **M** S058 (SC2335) `unquoted-path-in-mkdir` — unquoted variable in mkdir
- [x] [x] **M** S062 (SC2346) `default-value-in-colon-assign` — unquoted default in colon-assign
- [x] [x] **M** S067 (SC2366) `backtick-output-to-command` — backtick output word-split as args
- [x] [x] **M** S070 (SC2376) `double-quote-nesting` — double-quoted var between unquoted text
- [x] [x] **M** S071 (SC2379) `env-prefix-quoting` — unnecessary quoting on env prefix
- [x] [x] **M** S076 (SC2140) `mixed-quote-word` — alternating quoted/bare fragments in one arg

### Array Operations

Rules about array assignment, conversion, and element access patterns.

- [x] [x] **M** C106 (SC2336) `append-to-array-as-string` — string appended to array with `+=`
- [x] **M** C108 (SC2338) `unset-associative-array-element` — associative array element unset with quoted key
- [x] [x] **M** C133 (SC2381) `array-to-string-conversion` — array flattened to string
- [x] [x] **M** C148 (SC2399) `broken-assoc-key` — associative array key missing closing bracket
- [x] [x] **M** C151 (SC2054) `comma-array-elements` — commas in bash array literal

### Variable and Assignment

Rules about assignment syntax, variable naming, and value issues.

- [x] [x] **M** C095 (SC2319) `assignment-looks-like-comparison` — assignment value with dash may be typo
- [x] [x] **M** C101 (SC2329) `ifs-set-to-literal-backslash-n` — IFS set to literal `\n` not newline
- [x] [x] **L** C116 (SC2353) `assignment-to-numeric-variable` — numeric string as variable name
- [x] [x] **L** C117 (SC2354) `plus-prefix-in-assignment` — `+` before variable assignment
- [x] [x] **M** C130 (SC2377) `append-with-escaped-quotes` — `+=` with escaped quotes
- [x] [x] **M** C136 (SC2384) `local-cross-reference` — local assigns from same-line variable
- [x] [x] **L** C139 (SC2387) `spaced-assignment` — assignment-like word with stray spaces
- [x] [x] **L** C140 (SC2388) `bad-var-name` — variable name starts with invalid character
- [x] [x] **L** S042 (SC2280) `ifs-equals-ambiguity` — `IFS==` looks like comparison

### Command-Specific Checks

Rules about specific command usage patterns (find, grep, ls, tr, set, etc.).
Filter command facts by `effective_name_is()` and check options/arguments.

- [x] [x] **M** C098 (SC2324) `set-flags-without-dashes` — flags to `set` without leading dash
- [x] [x] **M** C103 (SC2332) `find-or-without-grouping` — find `-o` without grouping
- [x] [x] **M** C109 (SC2339) `mapfile-process-substitution` — mapfile from process substitution
- [x] [x] **M** C113 (SC2348) `find-output-in-loop` — find output captured in word-splitting loop
- [x] [x] **M** C132 (SC2380) `misspelled-option-name` — configure option name typo
- [x] [x] **L** S012 (SC2009) `ps-grep-pipeline` — piping ps into grep
- [x] [x] **L** S013 (SC2010) `ls-grep-pipeline` — piping ls into grep
- [x] [x] **L** S016 (SC2116) `echo-inside-command-substitution` — echo in `$()` is unnecessary
- [x] [x] **M** S019 (SC2143) `grep-output-in-test` — grep text as boolean check
- [x] [x] **L** S036 (SC2258) `bare-read` — `read` without options
- [x] [x] **L** S037 (SC2263) `redundant-spaces-in-echo` — extra spaces in echo collapsed
- [x] [x] **M** S044 (SC2291) `unquoted-variable-in-sed` — unquoted variable in sed
- [x] [x] **L** S046 (SC2293) `ls-piped-to-xargs` — ls piped to xargs
- [x] [x] **L** S047 (SC2294) `ls-in-substitution` — ls in command substitution
- [x] [x] **L** S049 (SC2298) `unquoted-tr-range` — unquoted tr character class
- [x] [x] **L** S051 (SC2303) `unquoted-tr-class` — unquoted tr class may glob-expand
- [x] [x] **L** S054 (SC2322) `su-without-flag` — su without `-l` or `-c`
- [x] [x] **L** S056 (SC2328) `command-substitution-in-alias` — command substitution in alias
- [x] [x] **L** S057 (SC2330) `function-in-alias` — function definition inside alias
- [x] [x] **L** S059 (SC2340) `deprecated-tempfile-command` — deprecated `tempfile` command
- [x] [x] **L** S060 (SC2342) `egrep-deprecated` — `egrep` instead of `grep -E`
- [x] [x] **L** S061 (SC2343) `fgrep-deprecated` — `fgrep` instead of `grep -F`
- [x] **L** S063 (SC2347) `relative-symlink-target` — deep relative symlink path
- [x] [x] **L** S064 (SC2350) `xargs-with-inline-replace` — deprecated `-i` flag for xargs
- [x] [x] **L** S068 (SC2369) `trap-signal-numbers` — numeric signal IDs in trap

### Shebang and Script Structure

Rules about shebang lines and script-level metadata.

- [x] [x] **L** C073 (SC2286) `indented-shebang` — shebang has leading whitespace
- [x] [x] **L** C074 (SC2287) `space-after-hash-bang` — space between `#` and `!`
- [x] [x] **L** C075 (SC2288) `shebang-not-on-first-line` — shebang on second line
- [x] [x] **L** S043 (SC2285) `missing-shebang-line` — no shebang, starts with comment
- [x] [x] **L** S053 (SC2318) `duplicate-shebang-flag` — repeated flag in shebang

### Escape and Backslash Sequences

Rules about needless or misleading backslash escapes. Most use surface fragment
facts or word facts for single-quoted strings.

- [x] [x] **L** C137 (SC2385) `unicode-single-quote-in-single-quotes` — Unicode smart quote in single-quoted string
- [x] [x] **L** S023 (SC1001) `escaped-underscore` — needless backslash in plain word
- [x] [x] **L** S024 (SC1003) `single-quote-backslash` — literal backslash in quoted string
- [x] [x] **L** S025 (SC1004) `literal-backslash` — backslash before normal letter is literal
- [x] [x] **L** S026 (SC1012) `needless-backslash-underscore` — backslash before normal char in word
- [x] [x] **L** S027 (SC1002) `escaped-underscore` — backslash before `_` is unnecessary
- [x] [x] **L** S039 (SC2267) `literal-backslash-in-single-quotes` — backslash in single quotes is literal
- [x] [x] **L** S040 (SC2268) `backslash-before-command` — backslash before command to bypass aliases

### Arithmetic Expressions

Rules about arithmetic expansion and arithmetic-context issues.

- [x] [x] **M** C077 (SC2290) `subshell-in-arithmetic` — command substitution in arithmetic
- [x] [x] **L** S022 (SC2219) `avoid-let-builtin` — `let` is unnecessarily indirect
- [x] [x] **L** S034 (SC2254) `array-index-arithmetic` — arithmetic expansion in array subscript
- [x] [x] **L** S035 (SC2257) `arithmetic-score-line` — long arithmetic expansion in assignment
- [x] [x] **L** S045 (SC2292) `dollar-in-arithmetic` — `$` before variable in `$(( ))`
- [x] [x] **L** S048 (SC2297) `dollar-in-arithmetic-context` — `$` in double-paren context

### Redirection and Pipe Issues

Rules about redirection ordering, clobbering, and pipe interactions.

- [x] [x] **M** C085 (SC2306) `stderr-before-stdout-redirect` — stderr redirected before stdout
- [x] [x] **M** C094 (SC2317) `redirect-clobbers-input` — read and write same file via redirect
- [x] [x] **M** C119 (SC2358) `redirect-before-pipe` — redirect before pipe only affects LHS
- [x] [x] **M** S075 (SC2129) `combine-appends` — multiple commands append same file separately

### Boolean Logic and Short-Circuit

Rules about `&&`/`||` chain semantics and boolean shortcut patterns.

- [x] [x] **M** C079 (SC2296) `short-circuit-fallthrough` — `&&`/`||` chain may not branch as intended
- [x] [x] **M** C115 (SC2352) `default-else-in-short-circuit` — `||` catches all failures in ternary
- [x] [x] **M** S020 (SC2165) `single-iteration-loop` — loop that exits immediately
- [x] [x] **M** S032 (SC2114) `conditional-assignment-shortcut` — boolean-style assignment shortcut

### Function and Scope

Rules about function definitions, local variables, and scope issues. Some
require semantic model access for call site analysis.

- [x] **H** C097 (SC2120) `function-called-without-args` — function that reads positional parameters is called with no arguments
- [x] **H** C123 (SC2364) `function-references-unset-param` — function references unset positional param
- [x] [x] **M** C125 (SC2367) `cd-without-error-check-in-func` — cd without error handling in function
- [x] [x] **M** C126 (SC2368) `continue-outside-loop-in-func` — continue inside function but outside loop
- [x] [x] **M** C131 (SC2378) `variable-as-command-name` — unquoted variable as command name
- [x] [x] **L** C147 (SC2398) `keyword-function-name` — reserved word as function name
- [x] [x] **M** S038 (SC2265) `redundant-return-status` — returns status function already propagates
- [x] [x] **L** S041 (SC2276) `function-body-without-braces` — bare compound command as body
- [x] [x] **L** S066 (SC2362) `local-declare-combined` — `local` and `declare` combined

### Case Statements

Rules about case pattern reachability and getopts integration. Glob reachability
rules require pattern analysis and are high complexity.

- [x] [x] **H** C128 (SC2373) `case-glob-reachability` — case glob pattern shadows later arm
- [x] [x] **H** C129 (SC2374) `case-default-before-glob` — default case before matching glob
- [x] [x] **M** C134 (SC2382) `getopts-option-not-in-case` — getopts option not handled in case
- [x] [x] **M** C135 (SC2383) `case-arm-not-in-getopts` — case arm not listed in getopts string
- [x] [x] **L** S069 (SC2372) `single-letter-case-label` — bare single letter as case label

### Subshell and Pipeline Side Effects

Rules about variable mutations inside subshells and pipelines that do not
propagate. Require semantic scope analysis and are high complexity.

- [x] [x] **H** C107 (SC2337) `dollar-question-after-command` — `$?` checked after intervening command
- [x] [x] **H** C150 (SC2031) `subshell-local-assignment` — variable assigned in subshell does not propagate
- [x] [x] **H** C155 (SC2030) `subshell-side-effect` — value updated in pipeline child, read afterward
- [x] [x] **H** C156 (SC2153) `possible-variable-misspelling` — referenced variable looks like misspelling

### Heredoc Issues

Rules about heredoc structure: missing/mismatched markers, whitespace, and
misuse.

- [x] [x] **M** C127 (SC2370) `unused-heredoc` — heredoc opened without consuming command
- [x] [x] **M** C138 (SC2386) `heredoc-missing-end` — heredoc never gets closing marker
- [x] [x] **L** C144 (SC2394) `heredoc-closer-not-alone` — closer on same line as content
- [x] [x] **M** C145 (SC2395) `misquoted-heredoc-close` — closing marker is only a near match
- [x] [x] **L** S030 (SC1040) `heredoc-end-space` — trailing whitespace on terminator
- [x] [x] **L** S033 (SC2127) `echo-here-doc` — heredoc attached to echo
- [x] [x] **L** S073 (SC2393) `spaced-tabstrip-close` — spaces before `<<-` closer

### Structural and Syntax Issues

Rules about control flow structure, continuation lines, braces, and syntax
oddities. Mostly AST-level checks.

- [x] [x] **M** C076 (SC2289) `commented-continuation-line` — line continuation followed by comment
- [x] [x] **M** C104 (SC2333) `non-shell-syntax-in-script` — C or other non-shell code in script
- [x] [x] **L** C141 (SC2389) `loop-without-end` — loop body never closed
- [x] [x] **L** C142 (SC2390) `missing-done-in-for-loop` — for loop reaches EOF without `done`
- [x] [x] **L** C143 (SC2391) `dangling-else` — else branch has no body
- [x] [x] **L** C146 (SC2396) `until-missing-do` — until loop skips `do`
- [x] [x] **L** C157 (SC1069) `if-bracket-glued` — `if` concatenated with `[`
- [x] [x] **M** S028 (SC1079) `suspect-closing-quote` — quote closed but next char is suspicious
- [x] [x] **M** S029 (SC1083) `literal-braces` — literal braces may be treated as expansion
- [x] [x] **L** S031 (SC1113) `trailing-directive` — directive after code is ignored
- [x] [x] **L** S072 (SC2392) `linebreak-before-and` — control operator starts new line
- [x] [x] **L** S074 (SC2397) `ampersand-semicolon` — backgrounded command followed by `;`

### Security

Rules about dangerous patterns that could lead to data loss or command
injection.

- [x] [x] **M** K001 (SC2115) `rm-glob-on-variable-path` — variable+glob in `rm -rf`
- [x] [x] **M** K002 (SC2029) `ssh-local-expansion` — ssh command expanded by local shell
- [x] [x] **M** K003 (SC2294) `eval-on-array` — eval used to execute composed command text
- [x] [x] **M** K004 (SC2156) `find-execdir-with-shell` — find -execdir passes `{}` to shell

### Performance

Rules about inefficient patterns that can be replaced with builtins or simpler
constructs.

- [x] [x] **L** P001 (SC2003) `expr-arithmetic` — expr for arithmetic when shell can do it
- [x] [x] **L** P002 (SC2126) `grep-count-pipeline` — `grep | wc -l` instead of `grep -c`
- [x] [x] **L** P003 (SC2233) `single-test-subshell` — lone test in subshell
- [x] [x] **L** P004 (SC2259) `subshell-test-group` — grouped test in subshell instead of braces
