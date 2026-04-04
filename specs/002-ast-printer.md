# 002: AST Printer (Typed JSON)

## Status

Proposed

## Summary

A new `shuck-ast-printer` library crate that serializes shuck's AST into the same typed JSON format that gbash produces. This enables diff-testing between `gbash --typed-json` and shuck's parser to verify feature parity and catch regressions. The crate is test-only — it has no CLI surface and no stability guarantees.

## Motivation

Shuck's parser needs to reach feature parity with gbash's Bash parser. Today there is no automated way to compare their outputs. By producing identical typed JSON for the same input, we can diff the two outputs and identify:

- **Missing syntax support** — constructs gbash parses that shuck doesn't
- **AST structure divergence** — cases where both parse successfully but produce different trees
- **Position tracking bugs** — off-by-one errors in line/column/offset

The typed JSON format is gbash's existing serialization format, documented by its roundtrip test suite. Matching it exactly avoids inventing a new interchange format and lets us reuse gbash's test corpus directly.

## Design

### Crate Structure

```
crates/shuck-ast-printer/
├── Cargo.toml
└── src/
    └── lib.rs          # Public API + serialization logic
```

**Dependencies:** `shuck-ast`, `serde`, `serde_json`

The crate exposes a single public function:

```rust
/// Serialize a parsed Script to gbash-compatible typed JSON.
pub fn to_typed_json(script: &Script, source: &str) -> serde_json::Value
```

The `source` parameter is needed to compute byte offsets for positions that shuck tracks as line/column but gbash expects as `Offset`.

### Output Format

The output matches gbash's typed JSON encoding exactly:

#### Positions

Every position is serialized as:

```json
{"Offset": 0, "Line": 1, "Col": 1}
```

- `Offset`: 0-based byte offset from start of file
- `Line`: 1-based line number
- `Col`: 1-based column number (byte offset within line)
- Fields use PascalCase
- Invalid/zero positions are omitted entirely

#### Type Tags

A `"Type"` field is emitted as the **first key** in any object where the concrete type would be ambiguous (interface fields in gbash's Go types). In practice this means:

- Root node (`File`)
- The `Cmd` field inside a `Stmt` (could be `CallExpr`, `IfClause`, `BinaryCmd`, etc.)
- Word parts (`Lit`, `SglQuoted`, `DblQuoted`, `ParamExp`, `CmdSubst`, etc.)
- Loop iterators inside `ForClause` (`WordIter`, `CStyleLoop`)
- Condition nodes inside `TestClause` (`CondBinary`, `CondUnary`, `CondParen`, `CondWord`, `CondPattern`, `CondRegex`)
- Arithmetic expression nodes (`UnaryArithm`, `BinaryArithm`, `ParenArithm`)
- Declaration operands (`DeclName`, `DeclFlag`, `DeclAssign`, `DeclDynamicWord`)
- Redirect `N` field and heredoc-related fields

#### Zero-Value Omission

Fields with default/empty values are omitted (`omitempty` semantics):

- `bool` fields: omitted when `false`
- `string` fields: omitted when `""`
- `uint` fields (including operator codes): omitted when `0`
- Slice/array fields: omitted when empty or nil
- Pointer/optional fields: omitted when nil/None

### Node Type Mapping

The following table maps gbash node types to shuck AST types. Rows marked **GAP** indicate constructs shuck does not yet support.

#### Top-Level Structure

| gbash Type | gbash Fields | shuck Type | Notes |
|---|---|---|---|
| `File` | `Stmts`, `Last` | `Script` | shuck has `commands`, no trailing comment tracking |
| `Stmt` | `Cmd`, `Position`, `Semicolon`, `Negated`, `Background`, `Redirs` | (implicit) | shuck folds negation into `Pipeline`, background into `ListOperator::Background`, and redirects into individual commands. Mapping requires synthesizing `Stmt` wrappers. |

#### Commands

| gbash Type | shuck Type | Notes |
|---|---|---|
| `CallExpr` | `Command::Simple(SimpleCommand)` | `Args` = name + args as Words; `Assigns` maps to `assignments` |
| `BinaryCmd` | `Command::List(CommandList)` or `Command::Pipeline(Pipeline)` | gbash uses binary tree; shuck uses flat list. `Op` 11=`&&`, 12=`||` map to `CommandList`; 13=`|`, 14=`|&` map to `Pipeline`. Printer must unflatten shuck's lists into nested `BinaryCmd` nodes. |
| `IfClause` | `CompoundCommand::If(IfCommand)` | `Cond`/`Then` map to `condition`/`then_branch`; elif chains map to `elif_branches`; `Else` maps to `else_branch` |
| `ForClause` | `CompoundCommand::For(ForCommand)` or `CompoundCommand::ArithmeticFor(ArithmeticForCommand)` | `Loop` is `WordIter` or `CStyleLoop` depending on variant |
| `WhileClause` | `CompoundCommand::While(WhileCommand)` | `Until` bool=false for while, =true for until |
| `CaseClause` | `CompoundCommand::Case(CaseCommand)` | `Items` maps to `cases`; `Op` uses case operator codes |
| `Block` | `CompoundCommand::BraceGroup(Vec<Command>)` | |
| `Subshell` | `CompoundCommand::Subshell(Vec<Command>)` | |
| `FuncDecl` | `Command::Function(FunctionDef)` | |
| `ArithmCmd` | `CompoundCommand::Arithmetic(String)` | `(( expr ))` |
| `TestClause` | `CompoundCommand::Conditional(Vec<Word>)` | **STRUCTURAL GAP** — see below |
| `TimeClause` | `CompoundCommand::Time(TimeCommand)` | |
| `CoprocClause` | `CompoundCommand::Coproc(CoprocCommand)` | |
| `DeclClause` | — | **GAP**: `declare`, `local`, `typeset`, `export`, `readonly` |
| `LetClause` | — | **GAP**: `let` expressions |
| `TestDecl` | — | **GAP**: gbash-specific test declaration |

#### Word Parts

| gbash Type | shuck Type | Notes |
|---|---|---|
| `Lit` | `WordPart::Literal(String)` | |
| `SglQuoted` | Single-quoted word | shuck uses `Token::LiteralWord`; printer maps to `SglQuoted` |
| `DblQuoted` | Double-quoted word | shuck uses `Token::QuotedWord`; printer maps to `DblQuoted` with inner parts |
| `ParamExp` | `WordPart::ParameterExpansion`, `Variable`, `Length`, `Substring`, `ArrayAccess`, `ArrayLength`, `ArrayIndices`, `IndirectExpansion`, `PrefixMatch`, `Transformation` | Multiple shuck variants collapse into gbash's single `ParamExp` with different sub-fields |
| `CmdSubst` | `WordPart::CommandSubstitution(Vec<Command>)` | |
| `ArithmExp` | `WordPart::ArithmeticExpansion(String)` | |
| `ProcSubst` | `WordPart::ProcessSubstitution` | `Op` 78=`<(`, 80=`>(` |
| `ExtGlob` | — | **GAP**: `@()`, `?()`, `*()`, `+()`, `!()` extended globs |
| `BraceExp` | — | **GAP**: `{a,b,c}` brace expansion as AST node (shuck handles in lexer as raw Word) |

#### Conditional Expression Nodes (`[[ ... ]]`)

gbash parses `[[ ... ]]` into a structured condition tree. Shuck currently stores `Conditional(Vec<Word>)` — an unstructured list of words.

| gbash Type | shuck Type | Notes |
|---|---|---|
| `CondBinary` | — | **STRUCTURAL GAP** |
| `CondUnary` | — | **STRUCTURAL GAP** |
| `CondParen` | — | **STRUCTURAL GAP** |
| `CondWord` | — | **STRUCTURAL GAP** |
| `CondVarRef` | — | **STRUCTURAL GAP** |
| `CondPattern` | — | **STRUCTURAL GAP** |
| `CondRegex` | — | **STRUCTURAL GAP** |

This is the largest structural gap. The printer cannot synthesize these nodes from `Vec<Word>` — the parser must be extended to produce a structured conditional AST.

#### Arithmetic Expression Nodes

gbash parses arithmetic expressions (`$((...))`, `((...))`, `let`, C-style `for`) into a structured tree. Shuck currently stores arithmetic as raw strings.

| gbash Type | shuck Type | Notes |
|---|---|---|
| `UnaryArithm` | — | **STRUCTURAL GAP** |
| `BinaryArithm` | — | **STRUCTURAL GAP** |
| `ParenArithm` | — | **STRUCTURAL GAP** |

The printer cannot synthesize these from raw strings — the parser must be extended to produce structured arithmetic AST nodes.

#### Redirects

| gbash Field | shuck Field | Notes |
|---|---|---|
| `Op` (RedirOperator) | `kind: RedirectKind` | Numeric mapping needed |
| `N` | `fd` / `fd_var` | gbash uses a `Lit` node for fd number or `{var}` name |
| `Word` | `target: Word` | |
| `Hdoc` | embedded in `target` for heredocs | |
| `HdocDelim` | — | **GAP**: shuck doesn't track heredoc delimiter metadata separately |

#### Operator Numeric Codes

The printer must emit the same numeric operator codes as gbash. Key mappings:

**Binary Command Operators (`BinaryCmd.Op`):**

| Code | Symbol | shuck Equivalent |
|---|---|---|
| 11 | `&&` | `ListOperator::And` |
| 12 | `||` | `ListOperator::Or` |
| 13 | `\|` | Pipeline pipe |
| 14 | `\|&` | Pipeline pipe-all (stderr) |

**Redirection Operators (`Redirect.Op`):**

| Code | Symbol | shuck `RedirectKind` |
|---|---|---|
| 63 | `>` | `Output` |
| 64 | `>>` | `Append` |
| 65 | `<` | `Input` |
| 67 | `<&` | `DupInput` |
| 68 | `>&` | `DupOutput` |
| 69 | `>\|` | `Clobber` |
| 71 | `<<` | `HereDoc` |
| 72 | `<<-` | `HereDocStrip` |
| 73 | `<<<` | `HereString` |
| 74 | `&>` | `OutputBoth` |

**Case Operators (`CaseItem.Op`):**

| Code | Symbol | shuck `CaseTerminator` |
|---|---|---|
| 35 | `;;` | `Break` |
| 36 | `;&` | `FallThrough` |
| 37 | `;;&` | `Continue` |

**Process Substitution Operators:**

| Code | Symbol | shuck Field |
|---|---|---|
| 78 | `<(` | `ProcessSubstitution { is_input: true }` |
| 80 | `>(` | `ProcessSubstitution { is_input: false }` |

**Parameter Expansion Operators (`ParamExp.Op`):**

| Code | Symbol | shuck `ParameterOp` |
|---|---|---|
| 81 | `+` (unset) | `UseReplacement` (without colon) |
| 82 | `:+` | `UseReplacement` (with colon) |
| 83 | `-` (unset) | `UseDefault` (without colon) |
| 84 | `:-` | `UseDefault` (with colon) |
| 85 | `?` (unset) | `Error` (without colon) |
| 86 | `:?` | `Error` (with colon) |
| 87 | `=` (unset) | `AssignDefault` (without colon) |
| 88 | `:=` | `AssignDefault` (with colon) |
| 89 | `%` | `RemoveSuffixShort` |
| 90 | `%%` | `RemoveSuffixLong` |
| 91 | `#` | `RemovePrefixShort` |
| 92 | `##` | `RemovePrefixLong` |
| 96 | `^` | `UpperFirst` |
| 97 | `^^` | `UpperAll` |
| 98 | `,` | `LowerFirst` |
| 99 | `,,` | `LowerAll` |
| 100 | `@` | `Transformation` |

### Key Structural Transformations

The printer must transform between shuck's AST representation and gbash's. These are the non-trivial cases:

#### 1. Flat Lists → Nested BinaryCmd

Shuck represents `a && b || c` as:

```rust
CommandList {
    first: a,
    rest: [(And, b), (Or, c)],
}
```

gbash represents it as nested binary nodes:

```json
{
  "Type": "BinaryCmd", "Op": 12,
  "X": {
    "Type": "BinaryCmd", "Op": 11,
    "X": {"Cmd": ...a...},
    "Y": {"Cmd": ...b...}
  },
  "Y": {"Cmd": ...c...}
}
```

The printer must fold `CommandList.rest` into a left-associative binary tree.

#### 2. Pipeline → BinaryCmd with Pipe Op

Shuck's `Pipeline { commands: [a, b, c] }` becomes nested `BinaryCmd` nodes with `Op: 13`.

The `Pipeline.negated` field maps to `Stmt.Negated` on the outermost `Stmt` wrapper.

#### 3. Stmt Synthesis

gbash wraps every command in a `Stmt` node. Shuck has no `Stmt` equivalent — background execution is `ListOperator::Background`, negation is `Pipeline.negated`, and redirects live on individual commands.

The printer must synthesize `Stmt` wrappers:
- `Position` from the command's span start
- `Semicolon` from the command's span end (when terminated by `;`)
- `Background` from `ListOperator::Background`
- `Negated` from `Pipeline.negated`
- `Redirs` extracted from `SimpleCommand.redirects` or compound command redirect lists

#### 4. Multiple Parameter Expansion Variants → Single ParamExp

Shuck has ~10 `WordPart` variants for parameter expansion. gbash has one `ParamExp` type with optional sub-fields. The printer must collapse:

| shuck Variant | gbash ParamExp Fields |
|---|---|
| `Variable(name)` | `Short: true, Param: {Value: name}` |
| `Length(name)` | `Length: true, Param: {Value: name}` |
| `ParameterExpansion {name, op, operand, colon}` | `Param`, `Op`, `Exp: {Word}` |
| `Substring {name, offset, length}` | `Param`, `Index`, `Length` (using `Slice` sub-struct) |
| `ArrayAccess {name, index}` | `Param: {Value: name}, Index: {Word}` |
| `ArrayLength(name)` | `Length: true, Param: {Value: name}, Index: {Word: "@"}` |
| `Transformation {name, op}` | `Param`, `Op: 100`, `Exp: {Word: op}` |

### Gaps to Close (Parser/AST Work)

Before the printer can produce output matching gbash for all inputs, these parser/AST gaps must be addressed. They are listed in priority order based on frequency in real-world scripts:

#### P0 — Required for meaningful diff-testing

1. **Structured `[[ ... ]]` parsing** — Replace `Conditional(Vec<Word>)` with a condition tree (`CondBinary`, `CondUnary`, `CondParen`, `CondWord`). This affects virtually every script that uses `[[ ]]`.

2. **Structured arithmetic parsing** — Replace `Arithmetic(String)` and `ArithmeticExpansion(String)` with expression trees (`UnaryArithm`, `BinaryArithm`, `ParenArithm`). Affects `(( ))`, `$(( ))`, and C-style `for` loops.

3. **`DeclClause` support** — `declare`, `local`, `typeset`, `export`, `readonly` are among the most common builtins. gbash treats them as a dedicated AST node with typed operands, not a `CallExpr`.

#### P1 — Required for full parity

4. **`LetClause` support** — `let` with structured arithmetic operands.

5. **Extended glob AST nodes** — `@()`, `?()`, `*()`, `+()`, `!()` as `ExtGlob` nodes with `Op` codes and parsed `Patterns`.

6. **Brace expansion AST nodes** — `{a,b,c}` and `{1..5}` as `BraceExp` nodes instead of raw words.

7. **Heredoc delimiter metadata** — Track quoting style and delimiter text separately (`HeredocDelim` node).

8. **`WhileClause` unification** — gbash uses a single `WhileClause` with an `Until` bool. Shuck has separate `WhileCommand` and `UntilCommand` types. The printer can handle this mapping, but unifying in the AST would be cleaner.

#### P2 — Nice to have

9. **Select command** — gbash may encode this differently; verify against test corpus.
10. **Colon-variant tracking for parameter ops** — shuck's `colon_variant: bool` on `ParameterExpansion` needs to map to distinct gbash operator codes (e.g., `:-` = 84 vs `-` = 83).

### Implementation Approach

Since the primary use case is diff-testing, the implementation should be incremental:

**Phase 1: Printer skeleton + simple constructs.** Implement the printer for the subset of constructs shuck already handles correctly: `CallExpr`, `BinaryCmd` (from `CommandList`/`Pipeline`), `IfClause`, `ForClause`, `WhileClause`, `CaseClause`, `Block`, `Subshell`, `FuncDecl`, basic `ParamExp`, `CmdSubst`, `Lit`, `SglQuoted`, `DblQuoted`, `Redirect`, `Assign`. Write tests comparing against gbash output for simple scripts.

**Phase 2: Close P0 gaps.** Extend the parser for structured conditionals and arithmetic, add `DeclClause` support, then extend the printer.

**Phase 3: Close P1 gaps.** Extended globs, brace expansion, heredoc metadata, `LetClause`.

Each phase produces a diff-testable checkpoint — we can run the full test corpus and track the number of mismatches decreasing.

## Alternatives Considered

### Custom diff format instead of matching gbash exactly

We could define our own JSON schema and write a semantic differ that understands both formats. Rejected because: (a) it doubles the specification surface, (b) structural differences are harder to diff semantically than textually, and (c) gbash's format already exists and is tested.

### serde derive-based serialization

We could add `#[derive(Serialize)]` to all AST types with `#[serde(rename_all = "PascalCase")]` and field-level attributes. Rejected because: the mapping between shuck's AST structure and gbash's is non-trivial (flat lists → binary trees, multiple expansion variants → single ParamExp, Stmt synthesis). A manual serialization approach with `serde_json::Value` construction gives us full control over the output shape without polluting the AST types with serialization concerns.

### Extending shuck's AST to mirror gbash exactly

We could restructure shuck's AST to be a 1:1 match of gbash's Go types. Rejected because: shuck's AST is designed for linting, not for matching another tool's internals. The printer is the right place to bridge the structural differences. However, some gaps (structured conditionals, arithmetic) are genuine missing features that should be added to the AST regardless.

### Embedding the printer in shuck-ast or shuck-syntax

Keeping it in an existing crate avoids a new crate. Rejected because: the printer depends on `serde_json` and contains gbash-specific mapping logic that doesn't belong in the core AST or syntax crates. A separate crate keeps the dependency graph clean and makes the test-only nature explicit.

## Verification

### Unit Tests

```bash
cargo test -p shuck-ast-printer
```

Test cases should cover:

1. **Simple command**: `echo hello` → matches gbash `CallExpr` output
2. **Pipeline**: `ls | grep foo` → nested `BinaryCmd` with `Op: 13`
3. **Logic operators**: `a && b || c` → left-associative nested `BinaryCmd`
4. **If/elif/else**: full chain → `IfClause` with `Elifs`
5. **For loop (word)**: `for i in a b; do cmd; done` → `ForClause` + `WordIter`
6. **For loop (C-style)**: `for ((i=0; i<3; i++))` → `ForClause` + `CStyleLoop`
7. **Case statement**: multiple patterns and terminators → correct `Op` codes
8. **Redirects**: `cmd > file 2>&1` → correct `Op` codes and `N` fields
9. **Parameter expansion**: all variants map to correct `ParamExp` sub-fields
10. **Quoting**: single, double, ANSI-C → `SglQuoted`, `DblQuoted`, `Lit`
11. **Position tracking**: offsets, lines, columns match gbash exactly

### Diff Testing Against gbash

The primary verification method. For a corpus of shell scripts:

```bash
# Generate gbash output
gbash --typed-json < test.sh > expected.json

# Generate shuck output (in test harness)
# parse test.sh with shuck-parser, then to_typed_json(), write to actual.json

# Diff (ignoring position fields initially, then including them)
diff expected.json actual.json
```

The test corpus should include:
- gbash's own roundtrip test file (`shell/syntax/typedjson/testdata/roundtrip/file.sh`)
- Real-world scripts from popular open-source projects
- Edge cases from shuck-parser's existing test suite

### Gap Tracking

Maintain a test that runs the full corpus and reports:
- Number of scripts that produce identical output
- Number of scripts with structural differences (AST gaps)
- Number of scripts with position-only differences
- Number of scripts that fail to parse in shuck

This gives a single metric to track progress toward parity.
