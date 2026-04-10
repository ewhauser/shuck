# Rule Implementation Roadmap

## Summary

| Status | Count |
|--------|-------|
| Implemented | 83 |
| Scheduled (Tranches 1-3) | 18 |
| Remaining | 217 |
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
- [ ] **L** X026 (SC3024) `bash-file-slurp` — `$(< file)` in sh
- [ ] **L** X045 (SC3055) `plus-equals-append` — `+=` assignment in sh
- [ ] **L** X055 (SC3062) `dollar-string-in-sh` — `$"string"` in sh
- [ ] **L** X064 (SC3071) `plus-equals-in-sh` — `+=` append operator in sh
- [ ] **L** X071 (SC3078) `array-keys-in-sh` — `${!arr[*]}` in sh
- [ ] **L** X081 (SC3085) `star-glob-removal-in-sh` — `${*%%pattern}` in sh

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

- [ ] **L** X027 (SC3037) `echo-flags` — echo flags depend on shell implementation
- [ ] **L** X028 (SC2018) `tr-lower-range` — locale-dependent lower-case tr range
- [ ] **L** X029 (SC2019) `tr-upper-range` — locale-dependent upper-case tr range
- [ ] **M** X030 (SC2028) `echo-backslash-escapes` — echo backslash escapes are non-portable

### Portability — POSIX sh Function and Variable Syntax

Detect non-portable function definitions and variable operations.

- [ ] **L** X035 (SC1065) `function-params-in-sh` — parameter syntax in sh function
- [ ] **L** X067 (SC3074) `hyphenated-function-name` — hyphen in function name
- [ ] **L** X072 (SC3079) `unset-pattern-in-sh` — pattern-based unset in sh
- [ ] **M** X077 (SC3083) `nested-default-expansion` — nested default-value expansion in sh

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

- [ ] **L** C082 (SC2302) `escaped-negation-in-test` — backslash-escaped `!` in test
- [ ] **M** C086 (SC2308) `greater-than-in-test` — `>` in `[ ]` creates file instead of comparing
- [ ] **M** C087 (SC2309) `string-comparison-for-version` — `<` in `[[ ]]` compares lexicographically
- [ ] **M** C088 (SC2310) `mixed-and-or-in-condition` — `&&`/`||` without grouping in `[[ ]]`
- [ ] **M** C089 (SC2311) `quoted-command-in-test` — pipeline quoted as string in test
- [ ] **M** C090 (SC2312) `glob-in-test-comparison` — glob on RHS of `==` in `[ ]`
- [ ] **M** C091 (SC2314) `tilde-in-string-comparison` — literal tilde in quoted comparison
- [ ] **M** C092 (SC2315) `if-dollar-command` — command substitution output as condition
- [ ] **M** C093 (SC2316) `backtick-in-command-position` — backtick substitution as command name
- [ ] **M** C102 (SC2331) `glob-in-test-directory` — glob in `[ -d ]` test
- [ ] **M** C110 (SC2341) `constant-in-test-assignment` — `=` in test looks like assignment
- [ ] **M** C118 (SC2357) `malformed-arithmetic-in-condition` — malformed arithmetic in condition
- [ ] **M** C120 (SC2360) `expr-substr-in-test` — `expr substr` inside test
- [ ] **M** C121 (SC2361) `string-compared-with-eq` — string compared with `-eq`
- [ ] **L** C122 (SC2363) `a-flag-in-double-bracket` — `-a` in `[[ ]]` is ambiguous
- [ ] **M** S011 (SC2166) `compound-test-operator` — `-a`/`-o` inside `[` expression
- [ ] **L** S065 (SC2351) `x-prefix-in-test` — `x$var` idiom for empty-string safety

### Glob and Pattern Matching

Rules about glob expansion in command arguments, find, grep, and comparisons.
Filter command facts and word facts for unquoted glob characters.

- [ ] **M** C078 (SC2295) `unquoted-globs-in-find` — unquoted variable+glob in find -exec
- [ ] **M** C080 (SC2299) `glob-in-grep-pattern` — glob character in grep pattern
- [ ] **M** C081 (SC2301) `glob-in-string-comparison` — variable in string comparison treated as glob
- [ ] **M** C083 (SC2304) `glob-in-find-substitution` — glob in find command substitution
- [ ] **M** C084 (SC2305) `unquoted-grep-regex` — grep regex may be glob-expanded
- [ ] **M** C114 (SC2349) `glob-with-expansion-in-loop` — glob+variable in for loop
- [ ] **M** S055 (SC2326) `glob-assigned-to-variable` — glob pattern assigned without quoting

### Quoting and Expansion

Rules about missing or incorrect quoting, word splitting, and expansion
contexts. Use word facts and expansion word facts.

- [ ] **M** C096 (SC2320) `unquoted-pipe-in-echo` — pipe/brace in echo may be interpreted
- [ ] **M** C099 (SC2325) `quoted-array-slice` — quoted array slice prevents splitting
- [ ] **M** C100 (SC2327) `quoted-bash-source` — `$BASH_SOURCE` quoted without array syntax
- [ ] **M** C105 (SC2334) `export-with-positional-params` — export with `$@`
- [ ] **M** C111 (SC2344) `at-sign-in-string-compare` — `$@` in string comparison folds args
- [ ] **M** C112 (SC2345) `array-slice-in-comparison` — array slice in string comparison
- [ ] **M** S014 (SC2048) `unquoted-dollar-star` — `$*` without quotes
- [ ] **M** S015 (SC2066) `quoted-dollar-star-loop` — `"$*"` in loop turns args into one item
- [ ] **M** S017 (SC2206) `unquoted-array-split` — unquoted value split into array
- [ ] **M** S018 (SC2207) `command-output-array-split` — raw command output into array
- [ ] **M** S021 (SC2145) `positional-args-in-string` — positional params folded into string
- [ ] **L** S050 (SC2300) `unquoted-word-between-quotes` — unquoted word between single-quoted segments
- [ ] **M** S052 (SC2307) `unquoted-variable-in-test` — unquoted variable in `[ -n ]`
- [ ] **M** S058 (SC2335) `unquoted-path-in-mkdir` — unquoted variable in mkdir
- [ ] **M** S062 (SC2346) `default-value-in-colon-assign` — unquoted default in colon-assign
- [ ] **M** S067 (SC2366) `backtick-output-to-command` — backtick output word-split as args
- [ ] **M** S070 (SC2376) `double-quote-nesting` — double-quoted var between unquoted text
- [ ] **M** S071 (SC2379) `env-prefix-quoting` — unnecessary quoting on env prefix
- [ ] **M** S076 (SC2140) `mixed-quote-word` — alternating quoted/bare fragments in one arg

### Array Operations

Rules about array assignment, conversion, and element access patterns.

- [ ] **M** C106 (SC2336) `append-to-array-as-string` — string appended to array with `+=`
- [ ] **M** C108 (SC2338) `unset-associative-array-element` — associative array element unset with quoted key
- [ ] **M** C133 (SC2381) `array-to-string-conversion` — array flattened to string
- [ ] **M** C148 (SC2399) `broken-assoc-key` — associative array key missing closing bracket
- [ ] **M** C151 (SC2054) `comma-array-elements` — commas in bash array literal

### Variable and Assignment

Rules about assignment syntax, variable naming, and value issues.

- [x] **M** C095 (SC2319) `assignment-looks-like-comparison` — assignment value with dash may be typo
- [x] **M** C101 (SC2329) `ifs-set-to-literal-backslash-n` — IFS set to literal `\n` not newline
- [x] **L** C116 (SC2353) `assignment-to-numeric-variable` — numeric string as variable name
- [x] **L** C117 (SC2354) `plus-prefix-in-assignment` — `+` before variable assignment
- [ ] **M** C130 (SC2377) `append-with-escaped-quotes` — `+=` with escaped quotes
- [ ] **M** C136 (SC2384) `local-cross-reference` — local assigns from same-line variable
- [ ] **L** C139 (SC2387) `spaced-assignment` — assignment-like word with stray spaces
- [ ] **L** C140 (SC2388) `bad-var-name` — variable name starts with invalid character
- [ ] **L** S042 (SC2280) `ifs-equals-ambiguity` — `IFS==` looks like comparison

### Command-Specific Checks

Rules about specific command usage patterns (find, grep, ls, tr, set, etc.).
Filter command facts by `effective_name_is()` and check options/arguments.

- [ ] **M** C098 (SC2324) `set-flags-without-dashes` — flags to `set` without leading dash
- [ ] **M** C103 (SC2332) `find-or-without-grouping` — find `-o` without grouping
- [ ] **M** C109 (SC2339) `mapfile-process-substitution` — mapfile from process substitution
- [ ] **M** C113 (SC2348) `find-output-in-loop` — find output captured in word-splitting loop
- [ ] **M** C132 (SC2380) `misspelled-option-name` — configure option name typo
- [ ] **L** S012 (SC2009) `ps-grep-pipeline` — piping ps into grep
- [ ] **L** S013 (SC2010) `ls-grep-pipeline` — piping ls into grep
- [ ] **L** S016 (SC2116) `echo-inside-command-substitution` — echo in `$()` is unnecessary
- [ ] **M** S019 (SC2143) `grep-output-in-test` — grep text as boolean check
- [ ] **L** S036 (SC2258) `bare-read` — `read` without options
- [ ] **L** S037 (SC2263) `redundant-spaces-in-echo` — extra spaces in echo collapsed
- [ ] **M** S044 (SC2291) `unquoted-variable-in-sed` — unquoted variable in sed
- [ ] **L** S046 (SC2293) `ls-piped-to-xargs` — ls piped to xargs
- [ ] **L** S047 (SC2294) `ls-in-substitution` — ls in command substitution
- [ ] **L** S049 (SC2298) `unquoted-tr-range` — unquoted tr character class
- [ ] **L** S051 (SC2303) `unquoted-tr-class` — unquoted tr class may glob-expand
- [ ] **L** S054 (SC2322) `su-without-flag` — su without `-l` or `-c`
- [ ] **L** S056 (SC2328) `command-substitution-in-alias` — command substitution in alias
- [ ] **L** S057 (SC2330) `function-in-alias` — function definition inside alias
- [ ] **L** S059 (SC2340) `deprecated-tempfile-command` — deprecated `tempfile` command
- [ ] **L** S060 (SC2342) `egrep-deprecated` — `egrep` instead of `grep -E`
- [ ] **L** S061 (SC2343) `fgrep-deprecated` — `fgrep` instead of `grep -F`
- [ ] **L** S063 (SC2347) `relative-symlink-target` — deep relative symlink path
- [ ] **L** S064 (SC2350) `xargs-with-inline-replace` — deprecated `-i` flag for xargs
- [ ] **L** S068 (SC2369) `trap-signal-numbers` — numeric signal IDs in trap

### Shebang and Script Structure

Rules about shebang lines and script-level metadata.

- [ ] **L** C073 (SC2286) `indented-shebang` — shebang has leading whitespace
- [ ] **L** C074 (SC2287) `space-after-hash-bang` — space between `#` and `!`
- [ ] **L** C075 (SC2288) `shebang-not-on-first-line` — shebang on second line
- [ ] **L** S043 (SC2285) `missing-shebang-line` — no shebang, starts with comment
- [ ] **L** S053 (SC2318) `duplicate-shebang-flag` — repeated flag in shebang

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

- [x] **M** C077 (SC2290) `subshell-in-arithmetic` — command substitution in arithmetic
- [x] **L** S022 (SC2219) `avoid-let-builtin` — `let` is unnecessarily indirect
- [x] **L** S034 (SC2254) `array-index-arithmetic` — arithmetic expansion in array subscript
- [x] **L** S035 (SC2257) `arithmetic-score-line` — long arithmetic expansion in assignment
- [x] **L** S045 (SC2292) `dollar-in-arithmetic` — `$` before variable in `$(( ))`
- [x] **L** S048 (SC2297) `dollar-in-arithmetic-context` — `$` in double-paren context

### Redirection and Pipe Issues

Rules about redirection ordering, clobbering, and pipe interactions.

- [ ] **M** C085 (SC2306) `stderr-before-stdout-redirect` — stderr redirected before stdout
- [ ] **M** C094 (SC2317) `redirect-clobbers-input` — read and write same file via redirect
- [ ] **M** C119 (SC2358) `redirect-before-pipe` — redirect before pipe only affects LHS
- [ ] **M** S075 (SC2129) `combine-appends` — multiple commands append same file separately

### Boolean Logic and Short-Circuit

Rules about `&&`/`||` chain semantics and boolean shortcut patterns.

- [ ] **M** C079 (SC2296) `short-circuit-fallthrough` — `&&`/`||` chain may not branch as intended
- [ ] **M** C115 (SC2352) `default-else-in-short-circuit` — `||` catches all failures in ternary
- [ ] **M** S020 (SC2165) `single-iteration-loop` — loop that exits immediately
- [ ] **M** S032 (SC2114) `conditional-assignment-shortcut` — boolean-style assignment shortcut

### Function and Scope

Rules about function definitions, local variables, and scope issues. Some
require semantic model access for call site analysis.

- [ ] **H** C097 (SC2323) `function-called-without-args` — function referencing `$1` called without args
- [ ] **H** C123 (SC2364) `function-references-unset-param` — function references unset positional param
- [ ] **M** C125 (SC2367) `cd-without-error-check-in-func` — cd without error handling in function
- [ ] **M** C126 (SC2368) `continue-outside-loop-in-func` — continue inside function but outside loop
- [ ] **M** C131 (SC2378) `variable-as-command-name` — unquoted variable as command name
- [ ] **L** C147 (SC2398) `keyword-function-name` — reserved word as function name
- [ ] **M** S038 (SC2265) `redundant-return-status` — returns status function already propagates
- [ ] **L** S041 (SC2276) `function-body-without-braces` — bare compound command as body
- [ ] **L** S066 (SC2362) `local-declare-combined` — `local` and `declare` combined

### Case Statements

Rules about case pattern reachability and getopts integration. Glob reachability
rules require pattern analysis and are high complexity.

- [ ] **H** C128 (SC2373) `case-glob-reachability` — case glob pattern shadows later arm
- [ ] **H** C129 (SC2374) `case-default-before-glob` — default case before matching glob
- [ ] **M** C134 (SC2382) `getopts-option-not-in-case` — getopts option not handled in case
- [ ] **M** C135 (SC2383) `case-arm-not-in-getopts` — case arm not listed in getopts string
- [ ] **L** S069 (SC2372) `single-letter-case-label` — bare single letter as case label

### Subshell and Pipeline Side Effects

Rules about variable mutations inside subshells and pipelines that do not
propagate. Require semantic scope analysis and are high complexity.

- [ ] **H** C107 (SC2337) `dollar-question-after-command` — `$?` checked after intervening command
- [ ] **H** C150 (SC2031) `subshell-local-assignment` — variable assigned in subshell does not propagate
- [ ] **H** C155 (SC2030) `subshell-side-effect` — value updated in pipeline child, read afterward
- [ ] **H** C156 (SC2153) `possible-variable-misspelling` — referenced variable looks like misspelling

### Heredoc Issues

Rules about heredoc structure: missing/mismatched markers, whitespace, and
misuse.

- [ ] **M** C127 (SC2370) `unused-heredoc` — heredoc opened without consuming command
- [ ] **M** C138 (SC2386) `heredoc-missing-end` — heredoc never gets closing marker
- [ ] **L** C144 (SC2394) `heredoc-closer-not-alone` — closer on same line as content
- [ ] **M** C145 (SC2395) `misquoted-heredoc-close` — closing marker is only a near match
- [ ] **L** S030 (SC1040) `heredoc-end-space` — trailing whitespace on terminator
- [ ] **L** S033 (SC2127) `echo-here-doc` — heredoc attached to echo
- [ ] **L** S073 (SC2393) `spaced-tabstrip-close` — spaces before `<<-` closer

### Structural and Syntax Issues

Rules about control flow structure, continuation lines, braces, and syntax
oddities. Mostly AST-level checks.

- [x] **M** C076 (SC2289) `commented-continuation-line` — line continuation followed by comment
- [x] **M** C104 (SC2333) `non-shell-syntax-in-script` — C or other non-shell code in script
- [x] **L** C141 (SC2389) `loop-without-end` — loop body never closed
- [x] **L** C142 (SC2390) `missing-done-in-for-loop` — for loop reaches EOF without `done`
- [x] **L** C143 (SC2391) `dangling-else` — else branch has no body
- [x] **L** C146 (SC2396) `until-missing-do` — until loop skips `do`
- [x] **L** C157 (SC1069) `if-bracket-glued` — `if` concatenated with `[`
- [x] **M** S028 (SC1079) `suspect-closing-quote` — quote closed but next char is suspicious
- [x] **M** S029 (SC1083) `literal-braces` — literal braces may be treated as expansion
- [x] **L** S031 (SC1113) `trailing-directive` — directive after code is ignored
- [x] **L** S072 (SC2392) `linebreak-before-and` — control operator starts new line
- [x] **L** S074 (SC2397) `ampersand-semicolon` — backgrounded command followed by `;`

### Security

Rules about dangerous patterns that could lead to data loss or command
injection.

- [x] **M** K001 (SC2115) `rm-glob-on-variable-path` — variable+glob in `rm -rf`
- [x] **M** K002 (SC2029) `ssh-local-expansion` — ssh command expanded by local shell
- [x] **M** K003 (SC2294) `eval-on-array` — eval used to execute composed command text
- [x] **M** K004 (SC2156) `find-execdir-with-shell` — find -execdir passes `{}` to shell

### Performance

Rules about inefficient patterns that can be replaced with builtins or simpler
constructs.

- [x] [x] **L** P001 (SC2003) `expr-arithmetic` — expr for arithmetic when shell can do it
- [x] [x] **L** P002 (SC2126) `grep-count-pipeline` — `grep | wc -l` instead of `grep -c`
- [x] [x] **L** P003 (SC2233) `single-test-subshell` — lone test in subshell
- [x] [x] **L** P004 (SC2259) `subshell-test-group` — grouped test in subshell instead of braces
