---
name: implement-rule
description: >
  Implement a shuck-rs lint rule from its YAML definition in docs/rules/.
  Use this skill whenever the user asks to implement a rule (e.g., "implement C003",
  "implement docs/rules/X001.yaml", "add the unused-assignment rule"), wire up a new
  lint check, or build out a rule from its definition. This is about writing the Rust
  code that makes a rule actually detect violations — not about importing rule
  definitions (that's the import-rules skill).
---

# Implement a Lint Rule

This skill turns a YAML rule definition in `docs/rules/` into a working lint rule with
violation struct, checker dispatch, test fixture, and snapshot test.

## Before you start

Read these files to understand the current state:

1. The YAML rule definition the user wants to implement (e.g., `docs/rules/C003.yaml`)
2. `crates/shuck-linter/src/registry.rs` — to see existing rules and find where to insert
3. `crates/shuck-linter/src/checker.rs` — to see current dispatch phases
4. `crates/shuck-linter/src/rules/correctness/unused_assignment.rs` — the reference implementation to follow

Also read the semantic model to understand what data is available for the rule:
- `crates/shuck-semantic/src/lib.rs` — `SemanticModel` public API

## Step 1: Understand the rule

Read the YAML definition. Key fields:

```yaml
new_category: Correctness    # → Category module: correctness, style, performance, portability, security
new_code: C001               # → Rule code used in registry
shellcheck_code: SC2034       # → ShellCheck mapping for suppression
description: ...              # → What the rule detects
examples:                     # → Test cases to base the fixture on
```

From the description and examples, determine:
- What semantic data the rule needs (bindings, references, scopes, declarations, source refs, AST, dataflow)
- Which checker phase is appropriate (see "Choosing the checker phase" below)
- What false positives to filter out

## Step 2: Add to the registry

**File:** `crates/shuck-linter/src/registry.rs`

Add an entry to the `declare_rules!` macro. Choose a PascalCase variant name that clearly describes the violation. Insert it in code-order (C001 before C002, etc.).

```rust
declare_rules! {
    ("C001", Category::Correctness, Severity::Warning, UnusedAssignment),
    // Add new rule here in code-sorted order
    ("C003", Category::Correctness, Severity::Warning, NewRuleName),
    ...
}
```

**Severity guidelines:**
- `Error` — the code is almost certainly wrong (undefined variable, unreachable code)
- `Warning` — likely a problem but could be intentional (unused assignment, unquoted expansion)
- `Hint` — suggestion, no real risk (style preferences)

If the YAML has a `legacy_code` (SH-NNN), add a legacy alias in `code_to_rule()`:
```rust
"SH-NNN" => Some(Rule::NewRuleName),
```

If there's a `shellcheck_code`, add it to `crates/shuck-linter/src/suppression/shellcheck_map.rs`:
```rust
(NNNN, Rule::NewRuleName),  // SCNNNN
```

## Step 3: Create the category module (if needed)

If this is the first rule in a category, create the module structure:

```
crates/shuck-linter/src/rules/{category}/
├── mod.rs
```

And register it in `crates/shuck-linter/src/rules/mod.rs`:
```rust
pub mod correctness;
pub mod style;  // new category
```

The `mod.rs` should contain a `#[cfg(test)]` block following the snapshot test pattern:

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::{LinterSettings, Rule, assert_diagnostics};
    use crate::test::test_path;

    #[test_case(Rule::NewRuleName, Path::new("C003.sh"))]
    fn rules(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("{category}").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }
}
```

If the category module already exists, just add a new `#[test_case]` line to the existing `rules` function.

## Step 4: Implement the rule

Create `crates/shuck-linter/src/rules/{category}/{rule_name}.rs`.

Follow this pattern:

```rust
use crate::{Checker, Rule, Violation};

pub struct RuleName {
    // Fields that provide context for the diagnostic message.
    // Common: `name: String` for variable names, `kind: String` for command types.
    // Use no fields if the message is always the same.
}

impl Violation for RuleName {
    fn rule() -> Rule {
        Rule::RuleName
    }

    fn message(&self) -> String {
        // Concise, lowercase message describing the violation.
        // Include context from fields: format!("variable `{}` is ...", self.name)
        format!("description of what's wrong")
    }
}

pub fn rule_name(checker: &mut Checker) {
    // Query the semantic model for the data this rule needs.
    // Filter out false positives.
    // Report violations via checker.report(violation, span).
}
```

Register the module in the category's `mod.rs`:
```rust
pub mod rule_name;
```

### Querying the semantic model

The checker holds `&SemanticModel`. Key query methods:

| Method | Returns | Use for |
|--------|---------|---------|
| `semantic.bindings()` | All variable bindings | Iterating assignments |
| `semantic.binding(id)` | Single binding | Looking up by ID |
| `semantic.references()` | All variable references | Iterating uses |
| `semantic.resolved_binding(ref_id)` | Binding a reference resolves to | Def-use chains |
| `semantic.unresolved_references()` | References with no binding | Undefined variables |
| `semantic.unused_assignments()` | Bindings never read | Dead assignments |
| `semantic.declarations()` | `declare`/`local`/`export` commands | Declaration analysis |
| `semantic.source_refs()` | `source`/`.` commands | Dynamic source paths |
| `semantic.call_sites_for(name)` | Where a function is called | Function call analysis |
| `semantic.call_graph()` | Reachable/uncalled functions | Dead function detection |
| `semantic.scope_kind(id)` | File/Function/Subshell/Pipeline | Scope-aware rules |

Binding has: `name`, `kind` (Assignment, FunctionDefinition, LoopVariable, etc.), `span`, `scope`, `references`, `attributes` (EXPORTED, READONLY, LOCAL, etc.)

Reference has: `name`, `kind` (Expansion, ParameterExpansion, ArithmeticRead, etc.), `span`, `scope`

### Common filtering patterns

```rust
// Skip exported variables (consumed by child processes)
if binding.attributes.contains(BindingAttributes::EXPORTED) { continue; }

// Skip function definitions (not regular assignments)
if matches!(binding.kind, BindingKind::FunctionDefinition) { continue; }

// Skip nameref bindings (used indirectly)
if matches!(binding.kind, BindingKind::Nameref) { continue; }

// Skip imported bindings
if matches!(binding.kind, BindingKind::Imported) { continue; }
```

## Step 5: Wire into checker dispatch

**File:** `crates/shuck-linter/src/checker.rs`

Add the rule invocation to the appropriate phase method:

```rust
fn check_bindings(&mut self) {
    // ... existing rules ...
    if self.is_rule_enabled(Rule::NewRuleName) {
        rules::category::rule_name::rule_name(self);
    }
}
```

### Choosing the checker phase

Match the rule to the phase based on what semantic data it primarily queries:

| Phase | Use when the rule... | Examples |
|-------|---------------------|----------|
| `check_bindings` | Iterates over variable bindings (assignments) | Unused assignment, overwritten variable |
| `check_references` | Iterates over variable references (uses) | Undefined variable, unquoted expansion |
| `check_scopes` | Checks scope-level properties | Scope leaks, variable shadowing |
| `check_declarations` | Checks `declare`/`local`/`export` commands | Bad declaration flags, conflicting attrs |
| `check_call_sites` | Checks function call patterns | Uncalled functions, wrong arg counts |
| `check_source_refs` | Checks `source`/`.` commands | Dynamic source paths |
| `check_commands` | Walks the AST structurally | Syntax-level patterns, command structure |
| `check_flow` | Needs CFG/dataflow analysis | Dead code, uninitialized reads |

When in doubt, `check_bindings` for rules about assignments, `check_references` for rules about variable uses, `check_commands` for structural/AST rules.

## Step 6: Create the test fixture

**File:** `crates/shuck-linter/resources/test/fixtures/{category}/{CODE}.sh`

Write a shell script that exercises both positive cases (should trigger) and negative cases (should not trigger). Use comments to label each section:

```bash
#!/bin/sh

# Should trigger: description of case
problematic_code_here

# Should not trigger: description of case
valid_code_here
```

Cover:
- The basic violation from the YAML examples
- Common false-positive scenarios that should NOT trigger
- Edge cases specific to the rule

## Step 7: Run tests and accept snapshots

```bash
cargo test -p shuck-linter
```

The snapshot test will fail the first time (new snapshot). Review the output to confirm it looks correct, then accept:

```bash
cargo insta accept --workspace
```

Run the full test suite to check for regressions:

```bash
cargo test --workspace
```

## Step 8: Summarize

After implementation, report:
- Rule code and name
- Which checker phase it's dispatched in
- How many test cases the fixture covers
- Whether all tests pass
