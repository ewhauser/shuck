# 004: Semantic Model

## Status

Proposed

## Summary

A new `shuck-semantic` library crate that builds a semantic model from a parsed shell script, enabling lint rules to query variable definitions and uses, function declarations and calls, scoping boundaries, declaration builtin classification, and source/import resolution — without walking the AST themselves. Modeled after ruff's `ruff_python_semantic` crate, adapted for shell-specific scoping rules (subshells, command substitutions, `local`/`export`/`declare` builtins, pipeline isolation).

The semantic model is the bridge between low-level positional indexing (`shuck-indexer`) and high-level rule execution. It answers questions like "is this variable defined before this use?", "what scope does this assignment belong to?", "is this function called before it's overwritten?", and "what does this `source` command import?".

## Motivation

Today `shuck check` only reports parse errors. To support the 332 lint rules defined in the Go frontend, we need a semantic layer that 192 semantic-phase rules, 3 dataflow rules, and 2 project-phase rules depend on. Without it, rules cannot:

- **Track variable definitions and uses** — Rules like SH-003 (unused assignment), SH-039 (unassigned variable reference), and SH-001 (unquoted expansion) need to know where variables are defined, read, and whether they're in scope at a given point.
- **Understand scoping boundaries** — Shell has complex scoping: `local` restricts to function scope, subshells `(...)` and pipelines isolate assignments, command substitutions `$(...)` create nested scopes, and `export` marks variables for subprocess inheritance. Rules need to query these boundaries.
- **Classify declaration builtins** — `declare`, `local`, `export`, `readonly`, `typeset` are not regular commands — they define variables with attributes. Rules need typed access to declaration operands, not string matching on command names.
- **Resolve function calls** — Rules like SH-171 (overwritten function) and SH-052/SH-292 (function scope issues) need a call graph to reason about function reachability and redefinition.
- **Track source imports** — Rules like SH-025 (dynamic source path) and SH-026 (untracked source file) need to resolve `source`/`.` commands to file paths and track imported symbols.
- **Reason about execution paths** — Rules like SH-003 (unused assignment) need to know whether a variable is overwritten before being read on all paths. SH-039 (unassigned variable) needs to know whether a definition reaches a use on all paths. SH-351 (dead code) needs to detect unreachable commands after unconditional `exit`/`return`. These require a control flow graph and dataflow analysis.

The Go frontend solves this with `SemanticIndex`, `VariableVisibilityIndex`, `StatementFlowIndex`, and `ProjectIndex` — a total of ~3000 lines of analysis code. We need the Rust equivalent, following ruff's architecture rather than porting Go's stringly-typed fact system directly.

## Design

### Crate Structure

```
crates/shuck-semantic/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API: SemanticModel struct + construction
    ├── scope.rs            # Scope tree and scope kinds
    ├── binding.rs          # Variable/function bindings and binding kinds
    ├── reference.rs        # Variable/function references (uses)
    ├── declaration.rs      # Declaration builtin classification
    ├── source_ref.rs       # source/. command classification
    ├── call_graph.rs       # Function call graph
    ├── cfg.rs              # Control flow graph types and construction
    ├── dataflow.rs         # Reaching definitions, unused/unset variable analysis
    └── builder.rs          # Single-pass AST visitor that builds the model
```

**Dependencies:** `shuck-ast` (AST types, `Name`, `Span`), `shuck-indexer` (region queries)

The crate does **not** depend on `serde` — it is a pure in-memory model. It does **not** own the source text or AST — it stores derived semantic data (IDs, spans, enums).

### Core Types

#### SemanticModel

The top-level query interface, constructed once per file and shared immutably across all rules.

```rust
pub struct SemanticModel {
    /// All scopes in source order.
    scopes: IndexVec<ScopeId, Scope>,

    /// All variable and function bindings.
    bindings: IndexVec<BindingId, Binding>,

    /// All variable/function references (uses).
    references: IndexVec<ReferenceId, Reference>,

    /// Resolved references: reference → binding it resolves to.
    resolved: FxHashMap<ReferenceId, BindingId>,

    /// Unresolved references (variable used but never defined in reachable scope).
    unresolved: Vec<ReferenceId>,

    /// Function declarations indexed by name.
    functions: FxHashMap<Name, Vec<BindingId>>,

    /// Call sites indexed by callee name.
    call_sites: FxHashMap<Name, Vec<CallSite>>,

    /// Rooted call graph (functions reachable from top-level).
    call_graph: CallGraph,

    /// Source/import references.
    source_refs: Vec<SourceRef>,

    /// Declaration builtin classifications.
    declarations: Vec<Declaration>,

    /// Statement flow context for each command.
    flow_context: FxHashMap<Span, FlowContext>,

    /// Control flow graph (built on demand for dataflow rules).
    cfg: Option<ControlFlowGraph>,

    /// Dataflow analysis results (built on demand).
    dataflow: Option<DataflowResult>,
}
```

#### Typed IDs

All IDs are newtypes over `u32`, stored in `IndexVec` arenas:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceId(u32);
```

`ScopeId(0)` is always the file (global) scope.

### Scopes

A scope represents a region where variable bindings are visible. Shell has fewer scope kinds than Python — most code runs in file scope, with function definitions and subshell constructs creating new boundaries.

```rust
pub struct Scope {
    pub id: ScopeId,
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    pub span: Span,
    /// Bindings directly defined in this scope, keyed by name.
    pub bindings: FxHashMap<Name, Vec<BindingId>>,
}

pub enum ScopeKind {
    /// Top-level file scope.
    File,
    /// Function body (`function f { ... }` or `f() { ... }`).
    Function(Name),
    /// Explicit subshell `(...)`.
    Subshell,
    /// Command substitution `$(...)`.
    CommandSubstitution,
    /// Pipeline segment (each command in `a | b | c` runs in a subshell by default).
    Pipeline,
}
```

**Scoping rules modeled:**

| Construct | Scope Effect |
|-----------|-------------|
| `f() { ... }` / `function f { ... }` | New `Function` scope |
| `( commands )` | New `Subshell` scope — assignments don't propagate to parent |
| `$(commands)` | New `CommandSubstitution` scope — reads parent, writes isolated |
| `cmd1 \| cmd2` | Each pipeline segment gets a `Pipeline` scope (subshell by default) |
| `{ commands; }` | No new scope — brace groups execute in current scope |
| `local var` | Binding restricted to nearest `Function` scope |
| `export var` | Binding in current scope + marked for subprocess inheritance |

### Bindings

A binding represents a single variable or function definition site.

```rust
pub struct Binding {
    pub id: BindingId,
    pub name: Name,
    pub kind: BindingKind,
    pub scope: ScopeId,
    pub span: Span,
    /// References that resolve to this binding.
    pub references: Vec<ReferenceId>,
    /// Attributes from declaration builtins.
    pub attributes: BindingAttributes,
}

pub enum BindingKind {
    /// Direct assignment: `VAR=value`
    Assignment,
    /// Append assignment: `VAR+=value`
    AppendAssignment,
    /// Array assignment: `arr=(a b c)` or `arr[i]=value`
    ArrayAssignment,
    /// Declaration builtin: `declare VAR`, `local VAR`, `export VAR`, `readonly VAR`
    Declaration(DeclarationBuiltin),
    /// Function definition: `f() { ... }` or `function f { ... }`
    FunctionDefinition,
    /// For/select loop variable: `for VAR in ...`
    LoopVariable,
    /// Read builtin target: `read VAR`
    ReadTarget,
    /// Mapfile/readarray target: `mapfile -t ARR`
    MapfileTarget,
    /// Printf -v target: `printf -v VAR ...`
    PrintfTarget,
    /// Getopts variable: `getopts optstring VAR`
    GetoptsTarget,
    /// Arithmetic assignment: `(( VAR = expr ))` or `(( VAR++ ))`
    ArithmeticAssignment,
    /// Nameref target: `declare -n REF=VAR` (the REF binding)
    Nameref,
    /// Imported via `source`/`.` (resolved at project level)
    Imported,
}

bitflags! {
    pub struct BindingAttributes: u16 {
        const EXPORTED  = 0b0000_0001;  // export or declare -x
        const READONLY  = 0b0000_0010;  // readonly or declare -r
        const LOCAL     = 0b0000_0100;  // local or declare (inside function)
        const INTEGER   = 0b0000_1000;  // declare -i
        const ARRAY     = 0b0001_0000;  // declare -a
        const ASSOC     = 0b0010_0000;  // declare -A
        const NAMEREF   = 0b0100_0000;  // declare -n
        const LOWERCASE = 0b1000_0000;  // declare -l
        const UPPERCASE = 0b0000_0001 << 8;  // declare -u
    }
}

pub enum DeclarationBuiltin {
    Declare,
    Local,
    Export,
    Readonly,
    Typeset,
}
```

### References

A reference represents a variable use (read) site.

```rust
pub struct Reference {
    pub id: ReferenceId,
    pub name: Name,
    pub kind: ReferenceKind,
    pub scope: ScopeId,
    pub span: Span,
}

pub enum ReferenceKind {
    /// Plain variable expansion: `$VAR` or `${VAR}`
    Expansion,
    /// Parameter expansion with operator: `${VAR:-default}`, `${VAR:?error}`, etc.
    ParameterExpansion,
    /// Length operator: `${#VAR}`
    Length,
    /// Array access: `${arr[idx]}`
    ArrayAccess,
    /// Indirect expansion: `${!VAR}`
    IndirectExpansion,
    /// Arithmetic read: variable used in `(( ... ))` or `$(( ... ))`
    ArithmeticRead,
    /// Conditional test operand: variable in `[[ $VAR ... ]]`
    ConditionalOperand,
    /// Required read — the variable must be set or the shell errors.
    /// Corresponds to `${VAR:?message}`.
    RequiredRead,
}
```

### Declaration Builtin Classification

The Go frontend's most complex semantic recognition: `declare`, `local`, `export`, `readonly`, and `typeset` are not regular commands — they take typed operands.

```rust
pub struct Declaration {
    pub builtin: DeclarationBuiltin,
    pub span: Span,
    pub operands: Vec<DeclarationOperand>,
}

pub enum DeclarationOperand {
    /// Flag: `-a`, `-r`, `-x`, `-i`, `-n`, `-A`, `-l`, `-u`, `-g`, `-p`
    Flag {
        flag: char,
        span: Span,
    },
    /// Variable name without value: `declare VAR`
    Name {
        name: Name,
        span: Span,
    },
    /// Assignment: `declare VAR=value` or `local VAR=value`
    Assignment {
        name: Name,
        name_span: Span,
        value_span: Span,
        append: bool,
    },
    /// Dynamic word that cannot be statically resolved: `declare "$var"`
    DynamicWord {
        span: Span,
    },
}
```

The builder recognizes `SimpleCommand` nodes where the command name is one of the declaration builtins, then parses the arguments into typed operands. This replaces the Go approach of pattern-matching on `CallExpr` names at each rule site.

### Source References

Classification of `source`/`.` commands for project-level analysis.

```rust
pub struct SourceRef {
    pub kind: SourceRefKind,
    pub span: Span,
    /// The argument word span (the path).
    pub path_span: Span,
}

pub enum SourceRefKind {
    /// Literal path resolvable at analysis time: `source ./lib.sh`
    Literal(String),
    /// Path overridden by directive: `# shellcheck source=path`
    Directive(String),
    /// Directive with `/dev/null` (explicitly ignored): `# shellcheck source=/dev/null`
    DirectiveDevNull,
    /// Dynamic path that cannot be resolved: `source "$DIR/lib.sh"`
    Dynamic,
    /// Single variable with static tail: `source "$DIR/lib.sh"` where only `$DIR` is dynamic.
    SingleVariableStaticTail {
        variable: Name,
        tail: String,
    },
}
```

### Call Graph

Function call tracking and reachability analysis.

```rust
pub struct CallSite {
    pub callee: Name,
    pub span: Span,
    pub scope: ScopeId,
    /// Argument words (for function argument analysis rules).
    pub arg_count: usize,
}

pub struct CallGraph {
    /// Functions reachable from top-level code.
    pub reachable: FxHashSet<Name>,
    /// Functions that are defined but never called.
    pub uncalled: Vec<BindingId>,
    /// Functions defined multiple times (overwritten).
    pub overwritten: Vec<OverwrittenFunction>,
}

pub struct OverwrittenFunction {
    pub name: Name,
    pub first: BindingId,
    pub second: BindingId,
    /// Whether the first definition was called before being overwritten.
    pub first_called: bool,
}
```

### Statement Flow Context

Per-command context that tracks structural nesting — needed by rules that check "is this `exit` inside a function?" or "is this `break` inside a loop?".

```rust
pub struct FlowContext {
    /// Whether this command is inside a function body.
    pub in_function: bool,
    /// Loop nesting depth (0 = not in a loop).
    pub loop_depth: u32,
    /// Whether this command is inside a subshell.
    pub in_subshell: bool,
    /// Whether this command is inside a brace group.
    pub in_block: bool,
    /// Whether the exit status of this command is checked (e.g., in `if` condition, `&&`/`||`).
    pub exit_status_checked: bool,
}
```

### Control Flow Graph

The CFG models execution paths through a script or function body. It is built from the AST after the initial semantic pass and is consumed by dataflow analyses.

#### Basic Blocks

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(u32);

pub struct BasicBlock {
    pub id: BlockId,
    /// Commands in this block, in execution order.
    pub commands: Vec<Span>,
    /// Variable bindings created in this block.
    pub bindings: Vec<BindingId>,
    /// Variable references in this block.
    pub references: Vec<ReferenceId>,
}
```

A basic block is a maximal sequence of commands with no internal branching — execution enters at the top and exits at the bottom. Branch points (conditionals, loops) and join points (after `if`/`fi`, after loop body) create block boundaries.

#### Graph Structure

```rust
pub struct ControlFlowGraph {
    /// All basic blocks in the graph.
    blocks: IndexVec<BlockId, BasicBlock>,
    /// Successor edges: block → [(target, edge kind)].
    successors: FxHashMap<BlockId, Vec<(BlockId, EdgeKind)>>,
    /// Predecessor edges (derived from successors).
    predecessors: FxHashMap<BlockId, Vec<BlockId>>,
    /// Entry block for the script/function.
    entry: BlockId,
    /// Exit blocks (blocks with no successors, or explicit return/exit).
    exits: Vec<BlockId>,
    /// Unreachable blocks (no path from entry).
    unreachable: Vec<BlockId>,
}

pub enum EdgeKind {
    /// Normal sequential flow.
    Sequential,
    /// Conditional true branch (if/elif condition is true, `&&` LHS succeeds).
    ConditionalTrue,
    /// Conditional false branch (if/elif condition is false, `||` LHS succeeds).
    ConditionalFalse,
    /// Loop back-edge (from end of loop body back to condition).
    LoopBack,
    /// Loop exit (break or condition failure).
    LoopExit,
    /// Case arm (from case head to pattern match).
    CaseArm,
    /// Case fallthrough (`;&`).
    CaseFallthrough,
    /// Case continue (`;;&` — test next pattern).
    CaseContinue,
}
```

#### Shell-Specific CFG Concerns

| Construct | CFG Modeling |
|-----------|-------------|
| `if cond; then A; else B; fi` | Branch from condition block → A (true) and B (false), join after `fi` |
| `cmd1 && cmd2` | Branch: cmd1 succeeds → cmd2, cmd1 fails → skip cmd2 (short-circuit) |
| `cmd1 \|\| cmd2` | Branch: cmd1 fails → cmd2, cmd1 succeeds → skip cmd2 |
| `while cond; do body; done` | Loop: condition → body (true) → back to condition, condition false → exit |
| `until cond; do body; done` | Same as while but with inverted condition sense |
| `for var in words; do body; done` | Loop: entry → body → back to entry, exhausted → exit |
| `for (( init; cond; step ))` | Loop: init → condition → body (true) → step → condition, false → exit |
| `case word in p1) A;; p2) B;; esac` | Branch from case head to each arm, `;;` exits case, `;&` falls through, `;;&` tests next |
| `( commands )` | Separate flow region — assignments don't flow out to parent scope |
| `cmd1 \| cmd2` | Each pipeline segment is a separate flow region (subshell) |
| `$(commands)` | Separate flow region nested in the enclosing expression |
| `return [n]` | Terminates function — edge to function exit block |
| `exit [n]` | Terminates script — edge to script exit block |
| `break [n]` | Exits enclosing loop(s) — edge to loop exit block |
| `continue [n]` | Jumps to loop condition — back-edge to loop header |
| `trap 'handler' SIGNAL` | Handler is reachable from any point after the trap — modeled as an edge from every subsequent block to the handler entry |

**Subshell boundaries:** Pipelines, explicit subshells `(...)`, and command substitutions `$(...)` create isolated flow regions. Variable assignments inside these regions do not propagate to the parent CFG. The CFG models each subshell region as a self-contained sub-graph. Variable references inside a subshell can read from the parent scope (snapshot at subshell entry), but writes are confined.

**Conservative handling:** `eval`, dynamic variable names (`declare "$name"`), and namerefs (`declare -n ref=var`) are treated conservatively — the analysis assumes they may define or read any variable. This prevents false positives at the cost of some false negatives.

### Dataflow Analysis

Dataflow analyses run over the CFG to answer questions that require reasoning about execution paths. Three analyses are needed for the Go frontend's dataflow-phase rules (SH-003, SH-039, SH-351):

#### Reaching Definitions

For each program point, compute the set of variable definitions (bindings) that may reach that point without being overwritten.

```rust
pub struct ReachingDefinitions {
    /// For each block, the set of bindings that reach the block entry.
    pub reaching_in: FxHashMap<BlockId, FxHashSet<BindingId>>,
    /// For each block, the set of bindings that reach the block exit.
    pub reaching_out: FxHashMap<BlockId, FxHashSet<BindingId>>,
}
```

Computed via standard iterative worklist algorithm over the CFG:
- **Gen set:** Bindings created in the block.
- **Kill set:** Bindings for the same variable name created in the block (overwrite earlier definitions).
- **Transfer function:** `reaching_out(B) = gen(B) ∪ (reaching_in(B) - kill(B))`
- **Join:** `reaching_in(B) = ∪ reaching_out(P) for all predecessors P`

Iterates until fixed point. Convergence is guaranteed because the lattice is finite (bounded by total binding count).

#### Unused Assignment Detection (SH-003)

A variable assignment is unused if it is overwritten on all paths before being read, or if the scope ends without a read.

```rust
pub struct UnusedAssignment {
    pub binding: BindingId,
    pub reason: UnusedReason,
}

pub enum UnusedReason {
    /// Overwritten before read on all execution paths.
    Overwritten { by: BindingId },
    /// Scope ends without a read on any execution path.
    ScopeEnd,
}
```

Computed from reaching definitions: a binding is unused if it does not appear in `reaching_in` at any block containing a reference to the same variable name.

#### Uninitialized Variable Detection (SH-039)

A variable reference is uninitialized if no definition for that variable reaches the reference point.

```rust
pub struct UninitializedReference {
    pub reference: ReferenceId,
    /// Whether the variable is *possibly* uninitialized (defined on some paths but not all)
    /// vs *definitely* uninitialized (defined on no paths).
    pub certainty: UninitializedCertainty,
}

pub enum UninitializedCertainty {
    /// No definition reaches this reference on any path.
    Definite,
    /// A definition reaches on some paths but not all.
    Possible,
}
```

Computed from reaching definitions: if `reaching_in` at the block containing the reference has no binding for the variable name, it is definitely uninitialized. If some predecessor paths have a binding and others don't, it is possibly uninitialized.

#### Dead Code Detection (SH-351)

Code that is unreachable from the CFG entry block.

```rust
pub struct DeadCode {
    /// Spans of unreachable commands.
    pub unreachable: Vec<Span>,
    /// The terminating command that makes the code unreachable
    /// (e.g., unconditional `exit`, `return`).
    pub cause: Span,
}
```

Computed directly from the CFG: blocks not reachable from the entry block via BFS/DFS. The cause is the last command in the predecessor block that terminates flow (unconditional `exit`, `return`, or `break` that exits all enclosing loops).

#### DataflowResult

```rust
pub struct DataflowResult {
    pub reaching_definitions: ReachingDefinitions,
    pub unused_assignments: Vec<UnusedAssignment>,
    pub uninitialized_references: Vec<UninitializedReference>,
    pub dead_code: Vec<DeadCode>,
}
```

### Construction

```rust
impl SemanticModel {
    /// Build a semantic model from a parsed script.
    pub fn build(
        script: &Script,
        source: &str,
        indexer: &Indexer,
    ) -> Self;
}
```

Construction is a single recursive AST walk performed by `SemanticModelBuilder` (in `builder.rs`). The builder maintains a scope stack and processes nodes in source order:

1. **Enter scope** — When visiting a function definition, subshell, command substitution, or pipeline, push a new scope onto the stack.
2. **Collect bindings** — When visiting an assignment, declaration builtin, `for`/`select` loop, `read` command, or function definition, create a `Binding` in the current scope.
3. **Collect references** — When visiting a `WordPart::Variable`, `WordPart::ParameterExpansion`, or arithmetic variable use, create a `Reference` in the current scope.
4. **Classify declarations** — When visiting a `SimpleCommand` whose name matches a declaration builtin, parse operands into `Declaration`.
5. **Classify source refs** — When visiting a `SimpleCommand` whose name is `source` or `.`, classify the path argument.
6. **Record call sites** — When visiting a `SimpleCommand` whose name could be a function call, record a `CallSite`.
7. **Record flow context** — Track loop depth, function membership, and subshell nesting as the walker descends.
8. **Exit scope** — When leaving a scope boundary, pop the scope stack.

After the walk:

9. **Resolve references** — For each reference, walk the scope chain upward to find the nearest visible binding with that name. Unresolved references are collected separately.
10. **Build call graph** — From top-level call sites, compute reachable functions. Identify uncalled and overwritten functions.

On demand (when a dataflow-phase rule is enabled):

11. **Build CFG** — Walk the AST a second time to construct basic blocks and edges. Each scope (file, function body) gets its own CFG. Subshell regions are modeled as isolated sub-graphs.
12. **Run dataflow** — Compute reaching definitions via iterative worklist. Derive unused assignments, uninitialized references, and dead code from the reaching definitions and CFG reachability.

### Query API

The semantic model exposes a query-first API for rule authors. Rules never walk the AST — they query the model.

#### Variable Queries

```rust
impl SemanticModel {
    /// All bindings for a variable name, across all scopes.
    pub fn bindings_for(&self, name: &Name) -> &[BindingId];

    /// The binding visible at a given span for a given name.
    /// Walks the scope chain from the scope containing `span`.
    pub fn visible_binding(&self, name: &Name, at: Span) -> Option<&Binding>;

    /// Whether a variable is defined anywhere in the file.
    pub fn defined_anywhere(&self, name: &Name) -> bool;

    /// Whether a variable is defined in any function scope.
    pub fn defined_in_any_function(&self, name: &Name) -> bool;

    /// Whether a variable has a required-read (${VAR:?}) before a given offset
    /// within the given scope.
    pub fn required_before(&self, name: &Name, scope: ScopeId, offset: usize) -> bool;

    /// Whether a variable might be defined outside the given scope
    /// (e.g., in global scope when checking a function body).
    pub fn maybe_defined_outside(&self, name: &Name, scope: ScopeId) -> bool;

    /// All unused assignments (assigned but never read before overwrite or scope end).
    pub fn unused_assignments(&self) -> &[BindingId];

    /// All unresolved references (used but never defined in reachable scope).
    pub fn unresolved_references(&self) -> &[ReferenceId];
}
```

#### Scope Queries

```rust
impl SemanticModel {
    /// The scope containing a given byte offset.
    pub fn scope_at(&self, offset: usize) -> ScopeId;

    /// The scope kind.
    pub fn scope_kind(&self, scope: ScopeId) -> &ScopeKind;

    /// Walk ancestor scopes.
    pub fn ancestor_scopes(&self, scope: ScopeId) -> impl Iterator<Item = ScopeId>;

    /// The flow context for a command at a given span.
    pub fn flow_context_at(&self, span: &Span) -> Option<&FlowContext>;
}
```

#### Function Queries

```rust
impl SemanticModel {
    /// All function definitions with the given name.
    pub fn function_definitions(&self, name: &Name) -> &[BindingId];

    /// All call sites for a given function name.
    pub fn call_sites_for(&self, name: &Name) -> &[CallSite];

    /// The rooted call graph.
    pub fn call_graph(&self) -> &CallGraph;
}
```

#### Declaration and Source Queries

```rust
impl SemanticModel {
    /// All declaration builtin invocations.
    pub fn declarations(&self) -> &[Declaration];

    /// All source/. references.
    pub fn source_refs(&self) -> &[SourceRef];
}
```

#### CFG and Dataflow Queries

```rust
impl SemanticModel {
    /// The control flow graph. Built on first access.
    pub fn cfg(&mut self) -> &ControlFlowGraph;

    /// Dataflow analysis results. Built on first access (triggers CFG build).
    pub fn dataflow(&mut self) -> &DataflowResult;

    /// All unused assignments (precise, dataflow-based).
    /// Falls back to the conservative "zero references" heuristic
    /// if dataflow has not been computed.
    pub fn unused_assignments(&self) -> &[BindingId];

    /// All unresolved references (precise when dataflow is available,
    /// scope-chain-based otherwise).
    pub fn unresolved_references(&self) -> &[ReferenceId];

    /// Whether a block is reachable from the CFG entry.
    pub fn is_reachable(&mut self, span: &Span) -> bool;

    /// Dead code spans with their causes.
    pub fn dead_code(&mut self) -> &[DeadCode];
}
```

The CFG and dataflow results are computed lazily — they are only built when a dataflow-phase rule first accesses them. The `&mut self` receiver on these methods reflects the interior lazy initialization. The variable query methods (`unused_assignments`, `unresolved_references`) work in both modes: without dataflow they use the conservative heuristic (zero references / scope-chain resolution), with dataflow they use the precise results.

### Integration Point

The semantic model sits between the indexer and rule execution:

```
Source text
  → shuck-parser: parse() → Script + Comments
  → shuck-indexer: Indexer::new() → Indexer
  → shuck-semantic: SemanticModel::build() → SemanticModel
  → Rule execution: each rule receives &Script, &Indexer, &SemanticModel, &str
```

Rules declare their required analysis level (syntax-only, semantic, dataflow, project) and the runner only constructs what's needed. A syntax-only rule never triggers `SemanticModel` construction.

### Rule Phases and Fact Dependencies

Mirroring the Go frontend's fact system, rules declare what analysis they need:

| Phase | Available Data | Rule Count (Go) |
|-------|---------------|-----------------|
| Syntax | `&Script`, `&Indexer`, `&str` | 129 |
| Directive | `&Script`, `&Indexer`, `&SuppressionIndex`, `&str` | 1 |
| Semantic | Above + `&SemanticModel` (scopes, bindings, references, call graph) | 192 |
| Dataflow | Above + `&SemanticModel` with CFG and dataflow results | 3 |
| Project | Above + `&ProjectClosure` | 2 |

The semantic model is constructed lazily — only when at least one enabled rule requires the semantic phase. Within the semantic model, CFG and dataflow are a further lazy tier: they are only computed when a dataflow-phase rule triggers them. This keeps the cost proportional to what rules actually need.

## Alternatives Considered

### Build a full HIR first, then semantic model on top

We could introduce a normalized HIR with typed arenas (`CommandId`, `WordId`, etc.) as a lowered intermediate representation, then build the semantic model on top of it rather than directly over `shuck-ast`.

Rejected for the first milestone because: the semantic model's queries (scope chains, binding resolution, call graphs) don't require normalized node IDs — they operate on `Name`, `Span`, and scope relationships. Building a full HIR first delays the ability to write rules by one additional phase. The AST already has the source fidelity needed (exact spans, compact `Name`, structured conditionals). If rule authoring later reveals friction from working with the raw AST (e.g., needing parent pointers or ordered iteration that the AST doesn't provide), we can introduce HIR as an intermediate layer without changing the semantic model's public API.

### Port Go's stringly-typed fact system directly

Go shuck uses string-keyed scope identifiers (`"function@42:100"`, `"global@0:500"`) and stores facts as loosely-typed maps. We could port this approach directly for familiarity.

Rejected because: ruff demonstrates that typed IDs (`ScopeId`, `BindingId`) with arena storage are safer, faster, and more ergonomic in Rust. String-keyed lookups are a Go idiom that doesn't translate well. Typed enums for `ScopeKind`, `BindingKind`, and `SourceRefKind` catch classification errors at compile time instead of runtime string comparisons.

### Embed the semantic model in shuck-indexer

The indexer already walks the AST and stores derived data. We could extend it to include scope/binding/reference tracking.

Rejected because: the indexer is deliberately limited to positional/structural data (byte ranges, line numbers, regions). Semantic analysis involves name resolution, scope chains, and cross-reference linking — a fundamentally different concern. Keeping them separate follows ruff's architecture (`ruff_python_index` vs `ruff_python_semantic`) and allows the indexer to remain fast and simple.

### Visitor trait instead of direct AST walk

We could define a `trait Visitor` with `visit_command`, `visit_word`, etc., and have the builder implement it. This is how ruff's `Checker` works.

Deferred because: a single builder module with explicit `match` arms on `Command` variants is simpler for the initial implementation and easier to follow. A visitor trait adds value when multiple consumers need to walk the AST with different logic (rule execution, type checking, etc.) — we can extract the trait later when the second consumer appears. The builder's internal structure doesn't affect the public API.

### Separate CFG crate

We could put the CFG and dataflow analysis in a separate `shuck-cfg` crate, keeping `shuck-semantic` focused on name resolution and scoping.

Rejected because: ruff includes CFG functionality within `ruff_python_semantic` rather than a separate crate. The CFG operates directly on the same scopes, bindings, and references that the semantic model owns — splitting them would require either duplicating types or adding a circular dependency. Keeping them together also simplifies the lazy computation: the semantic model can build the CFG on demand using its own internal state without an external orchestrator.

### Eager dataflow on every file

We could always compute the CFG and dataflow for every file, rather than lazily.

Rejected because: only 3 of 332 rules (< 1%) require dataflow analysis. The CFG construction and iterative worklist have measurably higher cost than the single-pass scope/binding collection. Lazy computation avoids this cost for the common case where no dataflow rules are enabled, or when running on files where only syntax/semantic rules fire.

## Verification

### Unit Tests

```bash
cargo test -p shuck-semantic
```

#### Scope tests

1. **File scope** — Top-level assignments produce bindings in `ScopeId(0)`.
2. **Function scope** — Assignments inside `f() { ... }` produce bindings in a `Function` scope.
3. **Nested function scopes** — Function defined inside function creates nested scope chain.
4. **Subshell scope** — `( VAR=x )` creates binding in `Subshell` scope, not visible in parent.
5. **Command substitution scope** — `$(VAR=x)` creates binding in `CommandSubstitution` scope.
6. **Pipeline scope** — Each segment of `a | b | c` gets its own scope.
7. **Brace group** — `{ VAR=x; }` does NOT create a new scope.

#### Binding tests

8. **Simple assignment** — `VAR=value` → `BindingKind::Assignment`.
9. **Declaration builtins** — `declare VAR`, `local VAR`, `export VAR=x`, `readonly VAR` → correct `DeclarationBuiltin` variant and attributes.
10. **Loop variable** — `for i in 1 2 3` → `BindingKind::LoopVariable` for `i`.
11. **Function definition** — `f() { ... }` → `BindingKind::FunctionDefinition`.
12. **Read target** — `read -r VAR` → `BindingKind::ReadTarget`.
13. **Arithmetic assignment** — `(( x = 5 ))` → `BindingKind::ArithmeticAssignment`.
14. **Declaration attributes** — `declare -xr VAR` → `EXPORTED | READONLY`.
15. **Local scope restriction** — `local VAR` binding is in the nearest function scope, not file scope.

#### Reference tests

16. **Variable expansion** — `echo $VAR` → `ReferenceKind::Expansion`.
17. **Parameter expansion** — `echo ${VAR:-default}` → `ReferenceKind::ParameterExpansion`.
18. **Arithmetic read** — `(( x + y ))` → `ReferenceKind::ArithmeticRead` for `x` and `y`.
19. **Required read** — `${VAR:?error}` → `ReferenceKind::RequiredRead`.

#### Resolution tests

20. **Simple resolution** — `VAR=x; echo $VAR` → reference resolves to the assignment binding.
21. **Scope chain** — Assignment in file scope, reference in function scope → resolves via parent chain.
22. **Local shadowing** — `VAR=x; f() { local VAR=y; echo $VAR; }` → reference resolves to local binding, not global.
23. **Subshell isolation** — `(VAR=x); echo $VAR` → reference resolves to outer binding (if any), not subshell binding.
24. **Unresolved** — `echo $UNDEFINED` with no prior assignment → unresolved reference.

#### Declaration tests

25. **Operand parsing** — `declare -a -r ARR=(1 2 3)` → correct flags and assignment operand.
26. **Dynamic operand** — `declare "$varname"` → `DeclarationOperand::DynamicWord`.
27. **Export without value** — `export VAR` → `DeclarationOperand::Name`.

#### Source reference tests

28. **Literal path** — `source ./lib.sh` → `SourceRefKind::Literal`.
29. **Dynamic path** — `source "$DIR/lib.sh"` → `SourceRefKind::Dynamic`.
30. **Directive override** — `# shellcheck source=path.sh` followed by `source "$x"` → `SourceRefKind::Directive`.
31. **Dev null** — `# shellcheck source=/dev/null` → `SourceRefKind::DirectiveDevNull`.

#### Call graph tests

32. **Simple call** — `f() { echo hi; }; f` → `f` is reachable.
33. **Uncalled function** — `f() { echo hi; }` with no call → `f` in `uncalled`.
34. **Overwritten function** — `f() { echo 1; }; f() { echo 2; }` → `OverwrittenFunction` entry.
35. **Transitive reachability** — `f() { g; }; g() { echo hi; }; f` → both `f` and `g` reachable.

#### Flow context tests

36. **Loop depth** — `for x in 1 2; do break; done` → `break` has `loop_depth: 1`.
37. **Nested loops** — `while true; do for x in 1 2; do break; done; done` → inner `break` has `loop_depth: 2`.
38. **In function** — `f() { exit 1; }` → `exit` has `in_function: true`.
39. **Exit status checked** — `if cmd; then ...; fi` → `cmd` has `exit_status_checked: true`.

#### CFG construction tests

40. **Linear script** — `a; b; c` → single block with three commands, one entry, one exit.
41. **If/else branching** — `if cond; then A; else B; fi; C` → condition block branches to A-block and B-block, both join at C-block.
42. **Short-circuit** — `a && b || c` → a branches to b (true) or c (false), b branches to join (true) or c (false).
43. **While loop** — `while cond; do body; done` → condition block, body block with back-edge to condition, exit edge from condition.
44. **For loop** — `for x in 1 2 3; do body; done` → entry block, body block with back-edge, exit block.
45. **Case statement** — `case $x in a) A;; b) B;; esac` → head block branches to A-arm and B-arm, both exit to join. Fallthrough `;&` creates edge to next arm body.
46. **Break/continue** — `while true; do if x; then break; fi; cmd; done` → break creates edge to loop exit, continue creates back-edge to loop header.
47. **Early return** — `f() { if err; then return 1; fi; cmd; }` → return creates edge to function exit, `cmd` block is still reachable from the false branch.
48. **Unreachable code** — `exit 0; echo "never"` → `echo` block has no predecessors (unreachable).
49. **Subshell isolation** — `(VAR=x); echo $VAR` → assignment in subshell sub-graph does not appear in parent CFG's reaching definitions.
50. **Pipeline segments** — `a | b | c` → three separate sub-graphs, one per pipeline segment.
51. **Nested loops with break N** — `while true; do while true; do break 2; done; done` → `break 2` edges to outer loop exit.

#### Dataflow tests

52. **Simple reaching definition** — `VAR=x; echo $VAR` → `VAR=x` reaches the reference.
53. **Overwritten variable** — `VAR=x; VAR=y; echo $VAR` → only `VAR=y` reaches the reference; `VAR=x` is unused.
54. **Branch-dependent definition** — `if cond; then VAR=x; else VAR=y; fi; echo $VAR` → both definitions reach the reference; neither is unused.
55. **Partial definition** — `if cond; then VAR=x; fi; echo $VAR` → `VAR=x` reaches on one path, no definition on the other → possibly uninitialized.
56. **Unused assignment** — `VAR=x; VAR=y; echo $VAR` → `VAR=x` is unused (overwritten by `VAR=y` on all paths).
57. **Dead code after exit** — `exit 0; echo "dead"` → `echo` is in `dead_code`.
58. **Dead code after unconditional return** — `f() { return 0; echo "dead"; }` → `echo` is in `dead_code`.
59. **Loop-carried definition** — `for f in *.sh; do echo $VAR; VAR=$f; done` → `VAR` is possibly uninitialized on first iteration, defined on subsequent iterations.
60. **Subshell doesn't propagate** — `(VAR=x); echo $VAR` → `VAR=x` does not reach the reference in the parent scope.

### Integration Test

61. **Full pipeline** — Parse a non-trivial multi-function script, build indexer and semantic model (including CFG and dataflow), verify scope tree, binding/reference resolution, call graph, declaration classification, reaching definitions, and dead code detection are all consistent. Compare against Go frontend output on the same script for parity.

### Parity Tests

62. **Go semantic parity** — For representative fixture scripts, compare Rust `SemanticModel` output against Go `SemanticIndex` output: same scopes, same variable visibility answers, same unresolved references, same call graph reachability, same overwritten functions, same unused assignments, same uninitialized variable reports. Mismatches are investigated and either fixed or documented as intentional divergences.
