# AST Gaps for CFG Construction

This document tracks gaps in bashkit's AST that need to be addressed before building a control flow graph and dataflow analysis layer.

## 1. Flow Control Keywords Are Generic SimpleCommands

`break`, `continue`, `return`, and `exit` are parsed as ordinary `SimpleCommand` nodes. There is no way to distinguish them from a regular command without inspecting the command name string.

```bash
break 2       # SimpleCommand { name: "break", args: ["2"] }
exit 1        # SimpleCommand { name: "exit",  args: ["1"] }
echo hello    # SimpleCommand { name: "echo",  args: ["hello"] }
```

### Proposed fix

Add variants to `CompoundCommand` or a new `BuiltinCommand` enum:

```rust
enum FlowControl {
    Break { depth: Option<Word> },
    Continue { depth: Option<Word> },
    Return { code: Option<Word> },
    Exit { code: Option<Word> },
}
```

The parser already recognizes these as reserved words in some contexts. Promoting them to distinct AST nodes makes CFG edges explicit without post-hoc string matching.

### Alternative

Handle in HIR lowering instead of changing the bashkit AST. This avoids touching the upstream parser but pushes the classification cost to every consumer.

## 2. `trap` Is a Generic SimpleCommand

`trap` defines signal handlers that execute asynchronously. For CFG purposes, trap bodies are reachable from any point after the `trap` call, which makes them important to model.

```bash
trap 'cleanup' EXIT    # SimpleCommand { name: "trap", args: ["cleanup", "EXIT"] }
```

### Proposed fix

Add a distinct AST node:

```rust
struct TrapCommand {
    pub action: TrapAction,
    pub signals: Vec<Word>,
    pub span: Span,
}

enum TrapAction {
    Handler(Word),   // string to eval
    Default,         // trap - SIG (reset)
    Ignore,          // trap '' SIG (ignore)
}
```

### Alternative

Classify in HIR. The trap body is a string evaluated at signal time, so even with an AST node the body remains opaque unless we parse it separately.

## 3. `source` / `.` Are Generic SimpleCommands

`source` and `.` include other scripts, which affects variable scope and control flow across file boundaries.

```bash
source ./lib.sh      # SimpleCommand { name: "source", args: ["./lib.sh"] }
. ./lib.sh           # SimpleCommand { name: ".",      args: ["./lib.sh"] }
```

### Proposed fix

Add a distinct AST node:

```rust
struct SourceCommand {
    pub path: Word,
    pub args: Vec<Word>,
    pub span: Span,
}
```

### Alternative

Classify in HIR. This is the more likely choice since resolving the sourced file is a linter concern, not a parser concern.

## 4. Arithmetic For Loop Expressions Are Opaque Strings

`ArithmeticForCommand` stores `init`, `condition`, and `step` as unparsed `String` values. Variable assignments and references inside these expressions are invisible to analysis.

```bash
for (( i=0; i<n; i++ )); do    # init: "i=0", condition: "i<n", step: "i++"
    ...
done
```

### Impact

- Cannot track that `i` is assigned in `init` and `step`.
- Cannot track that `i` and `n` are referenced in `condition`.
- Must treat the entire for-header as an opaque "uses and defines unknown variables" node.

### Proposed fix

Parse arithmetic expressions into a structured sub-AST:

```rust
enum ArithExpr {
    Literal(i64),
    Variable(String),
    Assign { name: String, value: Box<ArithExpr> },
    BinaryOp { op: ArithOp, left: Box<ArithExpr>, right: Box<ArithExpr> },
    UnaryOp { op: ArithOp, operand: Box<ArithExpr> },
    Ternary { condition: Box<ArithExpr>, then_val: Box<ArithExpr>, else_val: Box<ArithExpr> },
    Comma(Vec<ArithExpr>),
}
```

Then change `ArithmeticForCommand` fields from `String` to `ArithExpr`.

### Scope

This also fixes gap 5 below since `((...))` uses the same expression language.

## 5. Arithmetic Commands `((...))` Are Opaque Strings

`CompoundCommand::Arithmetic(String)` stores the entire expression as a flat string.

```bash
(( count++ ))          # Arithmetic("count++")
(( x = y + z ))        # Arithmetic("x = y + z")
```

### Impact

- Cannot detect variable assignments (`x = ...`, `x++`, `x += ...`).
- Cannot detect variable references.
- Arithmetic commands are common sites of variable mutation in shell scripts.

### Proposed fix

Reuse the `ArithExpr` type from gap 4:

```rust
CompoundCommand::Arithmetic(ArithExpr),
```

## 6. Conditional Expressions `[[ ... ]]` Lack Operator Structure

`CompoundCommand::Conditional(Vec<Word>)` stores the conditional as a flat word list with no parsed operator structure.

```bash
[[ -f $file && -r $file ]]    # Conditional(vec!["-f", "$file", "&&", "-r", "$file"])
```

### Impact

- Cannot model `&&` / `||` short-circuit within conditionals.
- Cannot distinguish test operators (`-f`, `-z`, `==`, `=~`) from operands.
- Lower priority than arithmetic expressions since conditionals don't assign variables (except `=~` which sets `BASH_REMATCH`).

### Proposed fix

Parse into a structured conditional expression:

```rust
enum CondExpr {
    Unary { op: String, operand: Word },
    Binary { op: String, left: Word, right: Word },
    And(Box<CondExpr>, Box<CondExpr>),
    Or(Box<CondExpr>, Box<CondExpr>),
    Not(Box<CondExpr>),
    Group(Box<CondExpr>),
}
```

### Priority

Low for initial CFG. Conditionals are test-only (no variable assignment except `=~` → `BASH_REMATCH`). Can be treated as opaque "references these variables" nodes initially.

## Summary

| Gap | Severity for CFG | Recommended layer |
|-----|-------------------|-------------------|
| 1. Flow control keywords | **High** — defines CFG edges | AST or HIR |
| 2. `trap` | **Medium** — async reachability | HIR |
| 3. `source` / `.` | **Medium** — cross-file flow | HIR |
| 4. Arithmetic for expressions | **Medium** — variable defs in loops | AST (parser) |
| 5. `((...))` expressions | **Medium** — variable mutation | AST (parser) |
| 6. `[[ ... ]]` structure | **Low** — read-only, no assignments | AST (parser), deferred |

### Recommended order

1. Flow control keywords (gap 1) — unblocks basic CFG construction.
2. Arithmetic expression parsing (gaps 4 + 5) — unblocks variable tracking through arithmetic.
3. `source` / `.` classification (gap 3) — unblocks cross-file analysis.
4. `trap` classification (gap 2) — needed for complete reachability.
5. Conditional expression parsing (gap 6) — nice-to-have, defer until needed.
