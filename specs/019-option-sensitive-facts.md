# 019: Option-Sensitive Semantic Facts

## Status

Proposed

## Summary

Add a rule-facing behavior layer for stateful shell options so zsh option support can scale beyond one-off rule checks. The semantic model remains responsible for tracking option state, but linter rules do not read raw `ZshOptionState` fields directly. Instead, semantic and fact builders resolve option-sensitive shell behavior into behavior-partitioned facts such as array-reference policy, field-splitting behavior, pathname-expansion behavior, glob-failure behavior, and arithmetic/subscript behavior. Rules must match the fact variants to access reportable spans, so new option-dependent behavior cannot be ignored accidentally without making that choice visible in code.

This spec extends [014-zsh-option-state.md](014-zsh-option-state.md). Spec 014 defines the raw zsh option-state timeline. This spec defines the next layer: how that timeline becomes reusable, performance-conscious semantic behavior and linter facts.

## Motivation

Stateful zsh options are becoming a shared correctness concern, not a single-rule exception. The `ksh_arrays` work for C100 demonstrated the shape of the problem: the semantic layer can conservatively track a stateful option, but the first useful API became option-specific and rule-specific:

```rust
semantic.zsh_ksh_arrays_runtime_state_at(reference.span.start.offset)
```

That pattern is acceptable for a prototype, but it will not scale across the rest of the option inventory. Options such as `SH_WORD_SPLIT`, `GLOB_SUBST`, `GLOB`, `NULL_GLOB`, `GLOB_DOTS`, `KSH_ZERO_SUBSCRIPT`, `C_BASES`, and `OCTAL_ZEROES` all affect facts that multiple rules consume. If every rule asks the semantic model for a raw option and interprets `On` / `Off` / `Unknown` locally, we get four problems:

- **Rules can forget an option.** A future rule can inspect a plain reference, glob, or arithmetic expression and never ask whether zsh changed the behavior at that point.
- **Ambiguity handling diverges.** One rule may treat `Unknown` as hazardous, another may suppress, and a third may accidentally use the default behavior.
- **Performance fragments.** Per-option runtime queries invite per-option reanalysis and per-rule caches instead of one shared behavior summary.
- **Facts lose their contract.** The linter already has a convention that structural discovery belongs in facts and rules should be cheap policy filters. Option-sensitive behavior should follow that same pattern.

The goal is to make the correct path the easy path: rules consume facts that already encode option-sensitive behavior, and adding a new behavior variant forces downstream code to decide what to do.

## Design

### Goals

- Preserve the semantic option-state engine from spec 014 as the source of truth for zsh option propagation.
- Replace option-specific semantic APIs such as `zsh_ksh_arrays_runtime_state_at` with whole-state behavior queries.
- Move option-sensitive decisions into fact construction wherever rules currently consume raw structural facts.
- Encode behavior in fact variants, not optional side fields, so rules must choose a behavior branch before they can access reportable data.
- Keep performance close to the current one-pass semantic and fact-building model.
- Make non-zsh behavior explicit through the same facts, so rules do not need a parallel zsh path.

### Non-Goals

- This spec does not add support for every tracked zsh option at once. It defines the structure that future option implementations use.
- This spec does not change parser pre-scan behavior for grammar-affecting options. Parser-time option handling remains covered by spec 014.
- This spec does not make linter rules depend on a zsh runtime or execute scripts.
- This spec does not remove `ZshOptionState` from the parser API. `ShellProfile` still needs parser-visible option state.

### Architecture

The option-sensitive path has four layers:

```text
Parser + semantic traversal
    |
    v
Zsh option timeline
    - raw ZshOptionState snapshots
    - setopt/unsetopt/emulate effects
    - function leak/localization modeling
    |
    v
Shell behavior model
    - whole-state runtime queries at offsets
    - conservative function-entry ambiguity
    - option fields tracked with compact masks
    |
    v
Linter facts
    - ArrayReferenceBehavior
    - FieldSplittingBehavior
    - PathnameExpansionBehavior
    - GlobFailureBehavior
    - SubscriptIndexBehavior
    - ArithmeticLiteralBehavior
    |
    v
Rules
    - exhaustive matches over behavior facts
    - no direct raw zsh option reads
```

The semantic model owns raw option propagation because it already records scopes, function bodies, command order, dynamic command references, and function-call effects. The linter owns behavior facts because rule policy should be expressed over linter concepts: references, words, redirects, substitutions, globs, assignments, arithmetic expressions, and commands.

### Whole-State Runtime Query

The current `ksh_arrays` prototype re-evaluates an enclosing function with `ksh_arrays = Unknown` when the file may enable `ksh_arrays` anywhere. The scalable form is a whole-state query that can answer every option-sensitive decision from one conservative analysis.

```rust
pub struct ShellBehaviorModel {
    zsh: Option<ZshBehaviorModel>,
}

pub struct ZshBehaviorModel {
    ordinary: ZshOptionAnalysis,
    may_enable_runtime_options: ZshOptionMask,
    runtime_by_function: OnceLock<FxHashMap<ScopeId, OnceLock<Option<ZshOptionAnalysis>>>>,
}

impl SemanticModel {
    pub fn shell_behavior_at(&self, offset: usize) -> ShellBehaviorAt<'_> {
        // Non-zsh dialects return a behavior object with dialect defaults.
        // Zsh returns an object backed by the conservative option state at offset.
    }
}
```

`ShellBehaviorAt` is the public query object. It exposes behavior-level methods, not raw option fields:

```rust
pub struct ShellBehaviorAt<'model> {
    shell: ShellDialect,
    zsh_options: Option<&'model ZshOptionState>,
    runtime_options: Option<ZshOptionState>,
}

impl ShellBehaviorAt<'_> {
    pub fn field_splitting(&self) -> FieldSplittingBehavior;
    pub fn pathname_expansion(&self) -> PathnameExpansionBehavior;
    pub fn glob_failure(&self) -> GlobFailureBehavior;
    pub fn array_reference_policy(&self) -> ArrayReferencePolicy;
    pub fn subscript_indexing(&self) -> SubscriptIndexBehavior;
    pub fn arithmetic_literals(&self) -> ArithmeticLiteralBehavior;
}
```

The object may internally merge the ordinary lexical option state with a conservative runtime-entry state for enclosing functions. Callers do not ask whether `ksh_arrays` is `Off`; they ask what array-reference policy applies.

### Compact Option Masks

The option analyzer should replace `FxHashSet<ZshOptionField>` for touched fields with a compact mask:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ZshOptionMask(u32);

impl ZshOptionMask {
    pub const ALL: Self = Self((1 << ZSH_OPTION_FIELD_COUNT) - 1);

    pub fn insert(&mut self, field: ZshOptionField);
    pub fn contains(self, field: ZshOptionField) -> bool;
    pub fn union(self, other: Self) -> Self;
    pub fn is_empty(self) -> bool;
}
```

There are 27 tracked options today, so a `u32` is enough. If the inventory grows beyond 32, the type can move to `u64` without changing callers. This keeps branch merges, function summaries, and unknown-effect propagation cheap:

```rust
struct FunctionSummary {
    final_outward: InternalState,
    outward_touched: ZshOptionMask,
}
```

Unknown `emulate`, dynamic `setopt`, and pattern-mode option changes set the relevant mask to `ALL` when the analyzer cannot prove which option changed. Known changes set only the affected field.

### Runtime Reanalysis Cache

Runtime ambiguity should be cached per function, not per option. A function can be re-evaluated once with an ambiguous entry state for all options that may vary due to dynamic dispatch or caller state:

```rust
fn runtime_analysis_for_function(
    &self,
    function_scope: ScopeId,
    ambiguous_entry: ZshOptionMask,
) -> Option<&ZshOptionAnalysis>;
```

The common case stays cheap:

- If a file has no option-changing command that can affect runtime behavior, `shell_behavior_at` uses the ordinary option snapshot directly.
- If an offset is outside a function, no function-entry reanalysis is needed.
- If a function has already been analyzed under the needed ambiguous-entry mask, the cached analysis is reused.
- If multiple facts in the same command ask for behavior, the command fact stores the behavior snapshot and downstream word/redirect facts reuse it.

The implementation should start with one runtime analysis per function using the union of all runtime-sensitive options that may vary in the file. If benchmarks later show excessive ambiguity, the cache key can become `(ScopeId, ZshOptionMask)` to support narrower reanalysis.

### Raw Option State Visibility

Rules should not call raw option-state APIs. The target API boundary is:

- `shuck-parser` may expose `ZshOptionState` and `ShellProfile` for parser configuration.
- `shuck-semantic` may keep raw `zsh_options_at` internally for tests and behavior construction.
- `shuck-linter` fact builders consume `SemanticModel::shell_behavior_at`.
- Rule modules consume behavior-partitioned facts.

Once fact migration is complete, `SemanticModel::zsh_options_at` should stop being part of the public cross-crate API. If external callers still need it, prefer an explicitly named escape hatch such as `raw_zsh_options_at_for_diagnostics` with documentation that linter rules must not use it.

### Behavior-Partitioned Facts

Facts should encode the option-sensitive behavior at the point where rules need it. The behavior must be part of the fact's shape, not a side field that a rule can ignore.

The weak form is this:

```rust
pub struct PlainUnindexedReferenceFact {
    pub span: Span,
    pub selector_requirement: ArraySelectorRequirement,
}
```

That does not force anything. A rule can still read `fact.span` and never inspect `selector_requirement`.

The strong form partitions the fact into variants and does not expose behavior-independent accessors on the enum:

#### Array References

Plain unindexed references to array-like bindings need to say whether a selector is required.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainUnindexedArrayReferenceFact {
    SelectorRequired(SelectorRequiredArrayReference),
    NativeZshScalar(NativeZshScalarArrayReference),
    Ambiguous(AmbiguousArrayReference),
}

pub struct SelectorRequiredArrayReference {
    reference_id: ReferenceId,
    diagnostic_span: Span,
}

pub struct NativeZshScalarArrayReference {
    reference_id: ReferenceId,
    expansion_span: Span,
}

pub struct AmbiguousArrayReference {
    reference_id: ReferenceId,
    diagnostic_span: Span,
}
```

The enum intentionally has no `span()` method. To get a reportable span, C100 must match the behavior:

```rust
match fact {
    PlainUnindexedArrayReferenceFact::SelectorRequired(reference) => {
        report(reference.diagnostic_span());
    }
    PlainUnindexedArrayReferenceFact::NativeZshScalar(_reference) => {}
    PlainUnindexedArrayReferenceFact::Ambiguous(reference) => {
        report_conservative(reference.diagnostic_span());
    }
}
```

This moves the `ksh_arrays` decision out of the rule and makes the rule choose what to do with every behavior bucket. If `ksh_zero_subscript` or another array option later changes selector policy, the fact builder changes the enum or adds a variant, and C100 must handle it before it can compile cleanly without a wildcard arm.

The raw span iterator that exists today for plain unindexed references should be removed from the rule-facing `LinterFacts` API or made private to the fact builder. The rule-facing API should be:

```rust
impl LinterFacts<'_> {
    pub fn plain_unindexed_array_references(
        &self,
    ) -> impl Iterator<Item = PlainUnindexedArrayReferenceFact> + '_;
}
```

Any shared helper that genuinely needs the span for every variant should accept the enum and perform an explicit match. It should not be added as `PlainUnindexedArrayReferenceFact::span()`, because that recreates the bypass.

#### Expansion Hazards

Expansion analysis should replace boolean hazard decisions that implicitly assume Bash-like behavior with behavior enums:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSplittingBehavior {
    Never,
    UnquotedOnly,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathnameExpansionBehavior {
    Disabled,
    LiteralGlobsOnly,
    SubstitutionResultsWhenUnquoted,
    Ambiguous,
}
```

For high-volume expansion facts, the same partitioning rule applies. A fact that can be reported differently by behavior should be split by behavior:

```rust
pub enum ExpansionHazardFact {
    FieldSplitting(FieldSplittingHazard),
    PathnameExpansion(PathnameExpansionHazard),
    Ambiguous(AmbiguousExpansionHazard),
    NoHazard(NoExpansionHazard),
}
```

`ExpansionAnalysis` can still expose convenient booleans such as `hazards.field_splitting` inside the fact builder, but rule-facing facts should not expose a behavior-independent report span when behavior matters. Rules that only need already-actionable diagnostics can consume a narrower iterator such as `facts.unquoted_expansion_hazards()`, whose item type is already partitioned by behavior.

#### Glob Failure

Rules that reason about missing or broad globs need to distinguish zsh failure behavior:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobFailureBehavior {
    ErrorOnNoMatch,
    KeepLiteralOnNoMatch,
    DropUnmatchedPattern,
    CshNullGlob,
    Ambiguous,
}
```

This covers `NOMATCH`, `NULL_GLOB`, and `CSH_NULL_GLOB` without requiring each glob-related rule to inspect three raw options.

#### Subscripts and Arithmetic

Array subscripts and arithmetic literals should likewise be facts, not option checks inside rules:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptIndexBehavior {
    OneBased,
    ZeroBased,
    OneBasedWithZeroAlias,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticLiteralBehavior {
    DecimalUnlessExplicitBase,
    LeadingZeroOctal,
    CStyleAndLeadingZeroOctal,
    Ambiguous,
}
```

These behaviors are consumed by facts over array references, indexed assignments, arithmetic expressions, and rules that report portability or correctness issues around numeric interpretation.

### Compiler Enforcement

The main enforcement mechanism is not "an enum field exists." That is insufficient because a rule can ignore the field. The enforceable shape is:

- Behavior-sensitive facts are enum variants.
- The enum has no behavior-independent accessor for reportable data.
- The old raw iterator or raw span accessor is removed from the rule-facing API.
- Rule code must match a variant before it can reach the span/reference/word needed to report.

Enum exhaustiveness then gives useful compiler pressure:

- Behavior enums are not marked `#[non_exhaustive]` inside the crate.
- Rule code matches explicit variants rather than using `_`.
- New option behavior adds or changes enum variants, causing downstream matches to fail until rules make an explicit decision.

To make this harder to bypass, the linter crate should deny wildcard matches for behavior enums:

```rust
#[deny(clippy::wildcard_enum_match_arm)]
mod rules;
```

If crate-wide denial is too noisy, start with the modules that consume option-sensitive facts and expand from there. A simpler first gate is a source check in CI:

```bash
rg "zsh_options_at|ZshOptionState|OptionValue" crates/shuck-linter/src/rules
```

The expected result after migration is no production rule hits. Tests may mention these types only when constructing fixtures or asserting fact behavior.

### Fact Ownership

The following ownership boundaries should guide implementation:

| Concern | Owner | Rationale |
| --- | --- | --- |
| Parsing grammar options | `shuck-parser` | Parser needs option state before semantic analysis exists. |
| Raw option propagation | `shuck-semantic` | Requires scopes, command order, function calls, and dynamic dispatch. |
| Behavior at source offset | `shuck-semantic` | Converts raw options into shell behavior consistently. |
| Word/reference/command facts | `shuck-linter` facts | Facts are the rule-facing contract and already own structural discovery. |
| Diagnostics and suppression | Rules/checker | Rules decide whether a behavior should report under the enabled policy. |

This means new rule files should not add direct AST walks or raw option queries. If a rule needs option-sensitive structural data, add a fact or extend an existing fact.

### Initial Migration Path

#### Phase 1: Normalize the Semantic Runtime Model

- Replace touched-field hash sets in `zsh_options.rs` with `ZshOptionMask`.
- Replace `zsh_ksh_arrays_runtime_state_at` with `shell_behavior_at`.
- Make runtime function reanalysis whole-state rather than option-specific.
- Keep the existing `ksh_arrays` tests as regression coverage, but assert through behavior APIs.

#### Phase 2: Move C100 to Fact-Level Policy

- Replace the plain unindexed reference span path with `PlainUnindexedArrayReferenceFact`.
- Update C100 to report from behavior-partitioned fact variants rather than querying semantic options.
- Add regression tests for native zsh, `setopt ksh_arrays`, `emulate ksh`, dynamic option names, dynamic function calls, and ambiguous function-local state.
- Add a source check that C100 no longer reads `zsh_options_at`, `zsh_ksh_arrays_runtime_state_at`, `ZshOptionState`, or `OptionValue`.

#### Phase 3: Expansion Behavior Facts

- Add `FieldSplittingBehavior` and `PathnameExpansionBehavior` to expansion facts.
- Update S001 and adjacent expansion-safety code to consume those behaviors.
- Add `GlobFailureBehavior` for glob-related facts and update C012/K003-style rules.
- Preserve existing boolean hazard accessors as derived convenience APIs until all rules migrate.

#### Phase 4: Remaining Stateful Options

- Add fact behavior for `NULL_GLOB`, `CSH_NULL_GLOB`, `GLOB_DOTS`, `EXTENDED_GLOB`, `KSH_GLOB`, and `SH_GLOB` in glob/pattern facts.
- Add `SubscriptIndexBehavior` for `KSH_ARRAYS` and `KSH_ZERO_SUBSCRIPT`.
- Add `ArithmeticLiteralBehavior` for arithmetic input behavior. `OCTAL_ZEROES` changes literal interpretation; `C_BASES` remains tracked for arithmetic output formatting rather than widening input-literal facts.
- Update the zsh option support inventory as each option moves from tracked-only to behavior-backed.

## Alternatives Considered

### Keep Per-Option Semantic Queries

Each new option could add a query like `zsh_ksh_arrays_runtime_state_at`.

Rejected because it scales poorly. It spreads option interpretation across rules, encourages repeated runtime analyses, and does not force new rules to consider existing option-sensitive behavior.

### Add Rule Helpers Over Raw Option State

Rules could call helper functions such as `zsh_native_array_scalar_policy(semantic, offset)`.

Rejected because helpers are easier to ignore than facts. They reduce duplication but do not change the rule-writing workflow: a new rule can still inspect references or words directly and forget the helper.

### Store Raw `ZshOptionState` on Every Fact

Facts could carry the option state and let rules decide how to interpret it.

Rejected because it makes every rule an option interpreter. The problem is not access to state; the problem is inconsistent downstream handling. Facts should carry behavior, not raw mechanism.

### Change Facts Directly Without a Behavior Object

The fact builder could update existing facts in place with booleans such as `requires_array_selector`.

Partially accepted. Facts should change directly, but the intermediate `ShellBehaviorAt` object keeps behavior decisions centralized and avoids duplicating raw option interpretation across fact builders. The rule-facing surface should be facts; the fact-building surface should be behavior queries.

### Make Zsh Option State a Linter-Only Concern

The linter could own all option analysis and leave semantic unaware.

Rejected because option propagation needs semantic data: scopes, function bodies, dynamic call targets, command ordering, and function leak behavior. Keeping it in semantic avoids rebuilding semantic reachability in the linter.

## Security Considerations

This design does not execute shell code and does not add filesystem or network access. The main risk is analysis unsoundness: treating ambiguous option state as definitely safe could suppress diagnostics. Behavior enums must therefore include an `Ambiguous` variant where the analyzer cannot prove one behavior, and rule policy should default to conservative reporting for correctness and security rules.

Performance is also a denial-of-service consideration for large shell files. Whole-state runtime reanalysis must remain lazy and cached per function, and recursive function analysis must keep the existing active-function guard so dynamic dispatch cannot cause unbounded recursion.

## Verification

Implementation should be verified in layers:

- **Semantic option behavior:** `cargo test -p shuck-semantic zsh_option_analysis --lib`
- **Runtime function ambiguity:** `cargo test -p shuck-semantic semantic_runtime_ksh_arrays_state --lib`
- **C100 fact migration:** `cargo test -p shuck-linter quoted_bash_source --lib`
- **Expansion behavior migration:** `cargo test -p shuck-linter unquoted_expansion --lib`
- **Full linter tests:** `cargo test -p shuck-linter --lib`
- **Workspace tests:** `make test`
- **Compatibility guard:** `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C100`
- **Rule raw-option guard:** `rg "zsh_options_at|zsh_ksh_arrays_runtime_state_at|ZshOptionState|OptionValue" crates/shuck-linter/src/rules`

The raw-option guard should have no production rule hits after migration. If a rule intentionally needs a raw option during an intermediate phase, the spec should be updated with the reason and the planned removal point.

Expected observable behavior:

- Native zsh plain array scalar reads remain clean when facts prove native scalar behavior.
- `setopt ksh_arrays`, `emulate ksh`, dynamic option names, and ambiguous dynamic function calls report conservatively.
- `noglob` and `setopt no_glob` suppress pathname-expansion facts for affected commands.
- `SH_WORD_SPLIT` and `${=...}` produce field-splitting facts; native zsh scalar expansion without those controls does not.
- Adding a new variant to an option-sensitive behavior enum causes rule matches to fail until downstream policy is updated.
