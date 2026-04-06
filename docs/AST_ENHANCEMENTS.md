# AST Enhancements For Lint Reliability

## Summary

This document captures the highest-value AST changes we can make to reduce linter bugs in `shuck-linter`.

The priority order here is driven by current rule implementation pain, not by parser completeness alone. The strongest signal is where rules currently have to:

- recover syntax from `Span` plus source text
- ask `shuck-indexer` whether something was quoted
- distinguish shell contexts by string matching on flattened `Word` data
- rescan raw slices to recover syntax forms the parser already recognized

Those patterns show up repeatedly in the current linter and are the places where false positives and false negatives are most likely.

The `gbash` frontend already has several AST shapes that address these problems directly. We should borrow the ideas and the test coverage, while still writing the Rust design in shuck's own style.

## Priorities

1. Quote-aware word parts and syntax-form preservation
2. First-class pattern AST and typed `[[ ... ]]` operands
3. First-class `VarRef`, typed `Subscript`, and explicit compound-array nodes
4. Heredoc delimiter metadata
5. Structured arithmetic AST

## 1. Quote-Aware Word Parts And Syntax-Form Preservation

### Why this matters for linting

This is the highest-priority change because several rules already need to reconstruct quoting and syntax form from source text or indexer regions instead of reading that information from the AST.

Examples in the current linter:

- `crates/shuck-linter/src/rules/correctness/single_quoted_literal.rs`
- `crates/shuck-linter/src/rules/correctness/trap_string_expansion.rs`
- `crates/shuck-linter/src/rules/style/unquoted_expansion.rs`
- `crates/shuck-linter/src/rules/style/legacy_backticks.rs`
- `crates/shuck-linter/src/rules/style/legacy_arithmetic_expansion.rs`
- `crates/shuck-linter/src/rules/common/span.rs`

The recurring issue is that `Word` currently gives us:

- `parts`
- `part_spans`
- a word-level `quoted: bool`

That is enough for many checks, but it is not enough to answer:

- which exact parts were single-quoted vs double-quoted vs unquoted
- whether a command substitution came from backticks or `$()`
- whether an arithmetic expansion came from `$[...]` or `$((...))`
- whether quoting boundaries changed meaning inside a mixed word

When rules recover that structure from `Span` or raw text, we create more room for bugs than if the parser simply preserved it.

### Proposed AST direction

Preserve syntax form at the word-part level.

At minimum:

- add explicit single-quoted and double-quoted word-part variants
- preserve backtick command substitution as a distinct syntax form
- preserve legacy `$[...]` arithmetic expansion as a distinct syntax form
- keep exact quote/syntax provenance on nested parts instead of only a word-level `quoted` flag

This does not require us to copy `gbash`'s API exactly. The important property is that a rule can ask the AST, not the indexer, whether a fragment was single-quoted, double-quoted, backtick-delimited, or legacy arithmetic.

### gbash ideas to borrow

Core node shapes:

- `shell/syntax/nodes.go`: `SglQuoted`
- `shell/syntax/nodes.go`: `DblQuoted`
- `shell/syntax/nodes.go`: `CmdSubst`
- `shell/syntax/nodes.go`: `ArithmExp`
- `shell/syntax/nodes.go`: `WordLeadingEscape`
- `shell/syntax/nodes.go`: `AliasExpansion`

Useful supporting docs:

- `docs/AST_ROADMAP.md`

### gbash tests to mine

- `shell/syntax/quote_test.go`
- `shell/syntax/backquote_recovery_test.go`
- `shell/syntax/parser_test.go`
- `shell/syntax/fidelity_test.go`
- `shell/syntax/public_api_test.go`
- `shell/syntax/typedjson/testdata/roundtrip/file.sh`
- `shell/syntax/typedjson/testdata/roundtrip/file.json`

### Expected linter wins

- `SingleQuotedLiteral` stops depending on region lookups for the core quoted/not-quoted question
- `TrapStringExpansion` can anchor directly on quoted expansion parts
- `LegacyBackticks` and `LegacyArithmeticExpansion` stop rescanning full word slices
- `UnquotedExpansion`, `UnquotedCommandSubstitution`, and `UnquotedArrayExpansion` become less dependent on indexer-assisted quoting heuristics

## 2. First-Class Pattern AST And Typed `[[ ... ]]` Operands

### Why this matters for linting

Pattern-sensitive rules are currently forced to treat patterns as generic words and then infer semantics from operator position or source text.

Examples in the current linter:

- `crates/shuck-linter/src/rules/correctness/case_pattern_var.rs`
- `crates/shuck-linter/src/rules/correctness/pattern_with_variable.rs`
- `crates/shuck-linter/src/rules/correctness/quoted_bash_regex.rs`

Current pain points:

- `case` arm patterns are stored as `Word`
- `[[ ... ]]` regex and pattern operands are stored as `ConditionalExpr::Regex(Word)` and `ConditionalExpr::Pattern(Word)`
- parameter pattern operators still hold `SourceText`, so rules scan strings to find variable-like syntax

That flattening makes it harder to distinguish:

- literal pattern syntax
- expanded dynamic fragments inside a pattern
- regex operands vs shell glob patterns
- `[[ -v ... ]]` variable-reference operands vs ordinary words

### Proposed AST direction

Introduce a shared pattern AST and typed conditional operands.

At minimum:

- add a first-class `Pattern` node shared by `case`, extglob, and parameter pattern operators
- change `ConditionalExpr::Pattern(Word)` into something pattern-aware
- add a conditional variable-reference operand for `[[ -v ... ]]` and `[[ -R ... ]]`
- keep regex operands distinct from pattern operands

This should eliminate most of the string-scanning logic in pattern rules.

### gbash ideas to borrow

Core node shapes:

- `shell/syntax/nodes.go`: `Pattern`
- `shell/syntax/nodes.go`: `PatternPart`
- `shell/syntax/nodes.go`: `PatternAny`
- `shell/syntax/nodes.go`: `PatternSingle`
- `shell/syntax/nodes.go`: `PatternCharClass`
- `shell/syntax/nodes.go`: `PatternGroup`
- `shell/syntax/nodes.go`: `CondPattern`
- `shell/syntax/nodes.go`: `CondRegex`
- `shell/syntax/nodes.go`: `CondVarRef`
- `shell/syntax/nodes.go`: `TestClause`

Useful supporting docs:

- `docs/AST_ROADMAP.md`

### gbash tests to mine

- `shell/syntax/quote_test.go`
- `shell/syntax/parser_test.go`
- `shell/syntax/subscript_test.go`
- `crates/shuck-parser/tests/testdata/oils/dbracket.test.sh`
- `crates/shuck-parser/tests/testdata/oils/case_.test.sh`
- `crates/shuck-parser/tests/testdata/oils/extglob-match.test.sh`
- `crates/shuck-parser/tests/testdata/oils/extglob-files.test.sh`
- `crates/shuck-parser/tests/testdata/oils/var-op-patsub.test.sh`

The oils fixtures are already in this repo and should become the Rust-side regression suite once the AST is upgraded.

### Expected linter wins

- `CasePatternVar` can reason over pattern nodes instead of generic words
- `PatternWithVariable` can inspect pattern parts instead of scanning `SourceText`
- `QuotedBashRegex` can use operand type directly instead of relying on partially flattened word classification
- future rules about extglob, char classes, or regex-vs-glob confusion become straightforward

## 3. First-Class `VarRef`, Typed `Subscript`, And Explicit Compound-Array Nodes

### Why this matters for linting

Array and variable-reference rules are currently working with a flattened model:

- `Assignment { name, index: Option<SourceText>, value, append }`
- `WordPart::ArrayAccess { name, index: SourceText }`
- `AssignmentValue::Array(Vec<Word>)`

That shape loses important distinctions:

- scalar vs indexed-array vs associative-array references
- `[@]` and `[*]` selectors vs ordinary subscripts
- keyed array elements vs sequential array elements
- append-to-element vs assign-to-element
- `[[ -v arr[key] ]]` reference semantics

Current linter code already compensates for this by checking `index.slice(source)` for `"@"` and `"*"` and by treating many array cases conservatively.

### Proposed AST direction

Introduce:

- `VarRef`
- `Subscript` with explicit kind and interpretation mode
- explicit compound-array nodes for indexed vs associative literals
- explicit array element nodes for sequential vs keyed vs keyed-append cases

We should also consider using the same reference node in:

- assignment targets
- declaration names
- conditional `-v` / `-R` operands
- parameter array access where practical

### gbash ideas to borrow

Core node shapes:

- `shell/syntax/nodes.go`: `Subscript`
- `shell/syntax/nodes.go`: `VarRef`
- `shell/syntax/nodes.go`: `Assign`
- `shell/syntax/nodes.go`: `DeclName`
- `shell/syntax/nodes.go`: `ArrayExpr`
- `shell/syntax/nodes.go`: `ArrayElem`

Useful supporting docs:

- `docs/AST_ROADMAP.md`

### gbash tests to mine

- `shell/syntax/subscript_test.go`
- `shell/syntax/varref_test.go`
- `shell/syntax/decl_operand_test.go`
- `crates/shuck-parser/tests/testdata/oils/array.test.sh`
- `crates/shuck-parser/tests/testdata/oils/array-assoc.test.sh`
- `crates/shuck-parser/tests/testdata/oils/array-assign.test.sh`
- `crates/shuck-parser/tests/testdata/oils/array-literal.test.sh`
- `crates/shuck-parser/tests/testdata/oils/assign.test.sh`
- `crates/shuck-parser/tests/testdata/oils/assign-extended.test.sh`
- `crates/shuck-parser/tests/testdata/oils/nameref.test.sh`
- `crates/shuck-parser/tests/testdata/oils/arith-context.test.sh`

### Expected linter wins

- array expansion rules stop string-matching on `SourceText`
- semantic analysis gets a stronger foundation for array writes and reads
- `-v` and nameref-related checks become less heuristic
- future rules around sparse arrays, associative keys, and suspicious subscripts become feasible without reparsing

## 4. Heredoc Delimiter Metadata

### Why this matters for linting

The current AST stores heredoc targets as ordinary redirect targets plus a redirect kind. That leaves rules without direct answers to questions like:

- was the heredoc delimiter quoted?
- does the body expand?
- was `<<-` used?
- what exact delimiter text was parsed after quote removal?

Those are exactly the facts heredoc-related rules need.

Even though we do not have many heredoc-heavy lint rules yet, this is a classic area where source rescanning tends to go wrong, especially once mixed quoting and indentation are involved.

### Proposed AST direction

Split heredoc syntax into:

- opener operator metadata
- delimiter metadata
- body content

At minimum the delimiter node should preserve:

- raw parts or raw text
- cooked delimiter value
- quoted/unquoted status
- whether body expansion is enabled
- strip-tabs mode

We do not need every parser-recovery detail on day one, but we should keep the semantic facts the linter will need.

### gbash ideas to borrow

Core node shapes:

- `shell/syntax/nodes.go`: `Redirect`
- `shell/syntax/nodes.go`: `HeredocDelim`
- `shell/syntax/nodes.go`: `HeredocCloseCandidate`
- `shell/syntax/nodes.go`: `HeredocIndentMode`

Useful supporting docs:

- `docs/AST_ROADMAP.md`

### gbash tests to mine

- `shell/syntax/parser_test.go`
- `shell/syntax/fidelity_test.go`
- `crates/shuck-parser/tests/testdata/oils/here-doc.test.sh`
- `crates/shuck-parser/tests/testdata/oils/redirect.test.sh`
- `crates/shuck-parser/tests/testdata/oils/redirect-multi.test.sh`
- `crates/shuck-parser/tests/testdata/oils/redirect-command.test.sh`

### Expected linter wins

- future heredoc rules can reason from AST metadata instead of rescanning raw source
- diagnostics can anchor to the delimiter rather than the whole redirect when appropriate
- suppression and region handling around heredoc bodies should become less brittle

## 5. Structured Arithmetic AST

### Why this matters for linting

Arithmetic contexts currently preserve source spans and source-backed text, but not a structured arithmetic tree. That is enough for formatting and some syntax checks, but weak for rule logic that needs to know:

- whether a variable is being read or written
- whether an expression has side effects
- whether a subscript is arithmetic or string-like
- whether a comparison is numeric or string-like

Rules and semantic passes eventually end up treating arithmetic as opaque text, which is a poor long-term foundation.

### Proposed AST direction

Add a structured arithmetic expression tree shared across:

- `(( ... ))`
- `$(( ... ))`
- arithmetic `for (( init ; cond ; step ))`
- arithmetic subscripts where relevant
- `let` if we add it later

We do not need to expose every shell dialect feature immediately. A Bash-focused tree covering the operators we already parse would still be a major improvement.

### gbash ideas to borrow

Core node shapes:

- `shell/syntax/nodes.go`: `ArithmExp`
- `shell/syntax/nodes.go`: `ArithmCmd`
- `shell/syntax/nodes.go`: `ArithmExpr`
- `shell/syntax/nodes.go`: `BinaryArithm`
- `shell/syntax/nodes.go`: `UnaryArithm`
- `shell/syntax/nodes.go`: `ParenArithm`
- `shell/syntax/nodes.go`: `CStyleLoop`

Implementation pointers:

- `shell/syntax/parser_arithm.go`

### gbash tests to mine

- `shell/syntax/parser_test.go`
- `internal/runtime/fuzz_expr_test.go`
- `crates/shuck-parser/tests/testdata/oils/arith.test.sh`
- `crates/shuck-parser/tests/testdata/oils/arith-context.test.sh`
- `crates/shuck-parser/tests/testdata/oils/arith-div-zero.test.sh`
- `crates/shuck-parser/tests/testdata/oils/arith-dynamic.test.sh`
- `crates/shuck-parser/tests/testdata/oils/dparen.test.sh`
- `crates/shuck-parser/tests/testdata/oils/for-expr.test.sh`

### Expected linter wins

- semantic analysis can model arithmetic reads and writes directly
- rules about suspicious arithmetic in redirects, tests, and assignments become more precise
- future array/subscript rules can reuse the same arithmetic tree instead of reparsing fragments

## Why These Five First

These five changes are the best tradeoff between implementation cost and linter reliability.

They address the parts of the current linter that are most heuristic today:

- quote recovery from spans and region indexes
- pattern and regex inference from generic words
- array/reference inference from flattened `SourceText`
- heredoc behavior reconstructed from redirect kind plus source
- arithmetic reasoning over raw source slices

By contrast, lower-priority ideas like top-level `Stmt` wrappers, file-level comment attachment, or full alias provenance may still be useful, but they are not the main source of current rule bugs.

## Suggested Rollout Order

1. Quote-aware word parts and syntax-form preservation
2. Pattern AST and typed conditional operands
3. `VarRef` / `Subscript` / compound-array nodes
4. Heredoc delimiter metadata
5. Structured arithmetic AST

## Verification Strategy

For each enhancement:

1. add parser-level shape tests in Rust that mirror the relevant `gbash` parser/unit tests
2. add focused parser fixtures for the structural edge cases the new node shape is meant to represent
3. add rule-level regression tests in `crates/shuck-linter/resources/test/fixtures`
4. rerun the most relevant corpus samples for affected rules

Recommended targeted follow-up after implementation:

- quote-aware parts: `S001`, `S004`, `C005`, `C008`, `C009`
- pattern and conditional AST: `C048`, `C009`
- var refs and arrays: `S008`, array-heavy correctness rules and future semantic queries
- heredocs: heredoc-specific rule work once the metadata lands
- arithmetic AST: arithmetic-sensitive semantic and correctness rules

## Notes

- Borrow ideas from `gbash`, not byte-for-byte structure unless parity clearly helps.
- Prefer importing or adapting tests over copying implementation shape mechanically.
- When a `gbash` test covers parser fidelity but our lint concern is narrower, keep the parser test and add a Rust linter fixture that proves the bug reduction directly.
