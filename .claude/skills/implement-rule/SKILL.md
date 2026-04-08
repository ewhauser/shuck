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

**Critical design principle:** Rules must be cheap filters over pre-computed facts. Do NOT add
direct AST walks, traversal helpers, command normalization, test operand reconstruction, or
word/redirect/substitution classification in rule files. If a rule needs new structural data,
add it to `LinterFacts` in `crates/shuck-linter/src/facts.rs` and consume it from there.

### Two data sources

Rules access data through two paths:

1. **`checker.facts()`** — Pre-computed `LinterFacts`. This is the primary data source for most
   rules. Facts normalize commands, extract options, classify words, analyze tests, and index
   structural patterns (pipelines, loops, redirects, substitutions). Prefer facts for anything
   structural or command-oriented.

2. **`checker.semantic()`** — The `SemanticModel`. Use this for variable binding/reference
   analysis (unused assignments, undefined variables, scope queries, call graphs). These rules
   iterate semantic collections directly.

### Violation struct pattern

```rust
use crate::{Checker, Rule, Violation};

pub struct RuleName {
    // Fields that provide context for the diagnostic message.
    // Common: `name: String` for variable names.
    // Use no fields if the message is always the same (preferred).
}

impl Violation for RuleName {
    fn rule() -> Rule {
        Rule::RuleName
    }

    fn message(&self) -> String {
        // Concise, lowercase message describing the violation.
        "description of what's wrong".to_owned()
    }
}
```

Register the module in the category's `mod.rs`:
```rust
pub mod rule_name;
```

### Pattern A: Fact-based rules (most rules)

Filter pre-computed facts, collect spans, report. This is the standard pattern.

**Command facts** — for rules about specific commands (read, printf, exit, find, xargs, sudo, etc.):
```rust
pub fn read_without_raw(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()                              // iterator over CommandFact
        .filter(|fact| fact.effective_name_is("read"))      // normalized name (unwraps env/command/sudo)
        .filter(|fact| {
            fact.options()
                .read()                                     // pre-parsed ReadCommandFacts
                .is_some_and(|read| !read.uses_raw_input)
        })
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || ReadWithoutRaw);
}
```

**Pipeline facts** — for rules about piped command sequences:
```rust
pub fn find_output_to_xargs(checker: &mut Checker) {
    let spans = checker
        .facts()
        .pipelines()                                        // &[PipelineFact]
        .iter()
        .flat_map(|pipeline| unsafe_find_to_xargs_spans(checker, pipeline))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || FindOutputToXargs);
}

fn unsafe_find_to_xargs_spans(checker: &Checker<'_>, pipeline: &PipelineFact<'_>) -> Vec<Span> {
    pipeline.segments().windows(2).filter_map(|pair| {
        let left = checker.facts().command_for_stmt(pair[0].stmt())?;   // cross-reference
        let right = checker.facts().command_for_stmt(pair[1].stmt())?;

        if !pair[0].effective_name_is("find") || !pair[1].effective_name_is("xargs") {
            return None;
        }
        if left.options().find().is_some_and(|f| f.has_print0)
            && right.options().xargs().is_some_and(|x| x.uses_null_input) {
            return None;  // null-delimited pair is safe
        }
        Some(left.body_span())
    }).collect()
}
```

**Loop header facts** — for rules about for/select loop iteration lists:
```rust
pub fn find_output_loop(checker: &mut Checker) {
    let spans = checker
        .facts()
        .for_headers()                                      // &[ForHeaderFact]
        .iter()
        .flat_map(|header| header.words().iter())
        .filter(|word| word.contains_find_substitution())   // pre-computed
        .map(|word| word.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || FindOutputLoop);
}
```

**Test/conditional facts** — for rules about `[...]` and `[[...]]` tests:
```rust
pub fn constant_comparison_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.simple_test().is_some_and(simple_test_is_constant)     // SimpleTestFact
                || fact.conditional().is_some_and(conditional_is_constant) // ConditionalFact
        })
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || ConstantComparisonTest);
}
```

**Redirect facts** — for rules about redirections on specific commands:
```rust
pub fn sudo_redirection_order(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.has_wrapper(WrapperKind::SudoFamily))
        .flat_map(|fact| {
            fact.redirect_facts().iter().filter_map(|redirect| {
                (redirects_output(redirect.redirect().kind)
                    && !redirect.analysis()
                        .is_some_and(|a| a.is_definitely_dev_null()))
                .then(|| redirect.target_span()).flatten()
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SudoRedirectionOrder);
}
```

**Surface fragment facts** — for lexer-level patterns (backticks, single quotes, etc.):
```rust
// Access via checker.facts().backtick_fragments(), .single_quoted_fragments(), etc.
```

**Word facts** — for rules about word expansion contexts:
```rust
// Access via checker.facts().word_facts(), .expansion_word_facts(context), .case_subject_facts()
```

### Key fact access methods

| Method | Returns | Use for |
|--------|---------|---------|
| `facts.commands()` | All command facts | General command matching |
| `facts.structural_commands()` | Top-level commands (iterator) | Commands not inside substitutions |
| `facts.pipelines()` | Pipeline facts | Multi-command pipe analysis |
| `facts.for_headers()` / `facts.select_headers()` | Loop list facts | Loop iteration patterns |
| `facts.lists()` | List operator facts | Mixed `&&`/`\|\|` detection |
| `facts.word_facts()` | Word expansion facts | Quoting/expansion analysis |
| `facts.expansion_word_facts(ctx)` | Words in specific context | Context-filtered word analysis |
| `facts.case_subject_facts()` | Case subject word facts | Case statement analysis |
| `facts.single_quoted_fragments()` | Single-quoted spans | Literal-in-quotes detection |
| `facts.backtick_fragments()` | Backtick substitution spans | Legacy syntax detection |
| `facts.command_for_stmt(stmt)` | Command fact by AST node | Cross-referencing pipelines/loops |
| `facts.command_for_command(cmd)` | Command fact by Command | Lookup from AST reference |
| `facts.word_fact(span, ctx)` | Word fact by span+context | Targeted word lookup |

### CommandFact key methods

| Method | Returns | Use for |
|--------|---------|---------|
| `fact.effective_name_is(name)` | bool | Normalized name check (unwraps wrappers) |
| `fact.has_wrapper(kind)` | bool | Check for sudo/env/command wrapping |
| `fact.options()` | CommandOptionFacts | Pre-parsed command-specific options |
| `fact.body_name_word()` | Option<&Word> | The body command's name word (for span) |
| `fact.body_span()` | Span | Span of the body command |
| `fact.span()` | Span | Full command span |
| `fact.redirect_facts()` | &[RedirectFact] | Pre-analyzed redirections |
| `fact.simple_test()` | Option<&SimpleTestFact> | `[...]` test structure |
| `fact.conditional()` | Option<&ConditionalFact> | `[[...]]` test structure |
| `fact.substitution_facts()` | &[SubstitutionFact] | Command substitution metadata |

### CommandOptionFacts (pre-parsed per-command data)

| Method | Returns | Use for |
|--------|---------|---------|
| `.read()` | Option<ReadCommandFacts> | `read -r` detection |
| `.printf()` | Option<PrintfCommandFacts> | Format word extraction |
| `.unset()` | Option<UnsetCommandFacts> | Function mode, operand words |
| `.find()` | Option<FindCommandFacts> | `-print0` detection |
| `.xargs()` | Option<XargsCommandFacts> | `-0`/`--null` detection |
| `.exit()` | Option<ExitCommandFacts> | Status word and staticness |
| `.sudo_family()` | Option<SudoFamilyCommandFacts> | Invoker type (sudo/doas/run0) |

### Pattern B: Semantic-based rules (binding/reference rules)

For rules that analyze variable assignments and references, iterate semantic collections directly:

```rust
pub fn unused_assignment(checker: &mut Checker) {
    for &binding_id in checker.semantic().unused_assignments() {
        let binding = checker.semantic().binding(binding_id);

        if !is_reportable(binding.kind, binding.attributes) { continue; }
        if binding.attributes.contains(BindingAttributes::EXPORTED) { continue; }
        if matches!(binding.kind, BindingKind::Nameref) { continue; }

        checker.report(
            UnusedAssignment { name: binding.name.to_string() },
            binding.span,
        );
    }
}
```

### Semantic model methods (for Pattern B only)

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

### Common semantic filtering

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

### Anti-patterns to avoid

- **No direct AST walks in rule files.** Don't call `walk_commands`, `iter_commands`, or
  recurse through child nodes. If you need structural data, add it to `LinterFacts`.
- **No command normalization in rules.** Use `fact.effective_name_is()` and
  `fact.has_wrapper()` — names are already normalized through env/command/sudo wrappers.
- **No option parsing in rules.** Use `fact.options().read()`, `.find()`, `.xargs()`, etc.
  If a new command needs option analysis, add it to `CommandOptionFacts` in `facts.rs`.
- **No word expansion analysis in rules.** Use pre-computed `WordFact` with its expansion
  analysis, operand class, and static text. Add new word analysis to `facts.rs` if needed.
- **No test operand reconstruction.** Use `SimpleTestFact` and `ConditionalFact` with their
  pre-computed shapes, operator families, and operand classes.
- **No imports from `crate::rules::common::*` in rule files.** Rule-facing shared types come
  from the crate root (re-exported from common modules).

## Step 5: Wire into checker dispatch

**File:** `crates/shuck-linter/src/checker.rs`

Add the rule invocation to the appropriate phase method:

```rust
fn check_command_facts(&mut self) {
    // ... existing rules ...
    if self.is_rule_enabled(Rule::NewRuleName) {
        rules::category::rule_name::rule_name(self);
    }
}
```

### Choosing the checker phase

Match the rule to the phase based on what data it primarily consumes:

| Phase | Use when the rule... | Examples |
|-------|---------------------|----------|
| `check_bindings` | Iterates semantic bindings (assignments) | UnusedAssignment |
| `check_references` | Iterates semantic references (variable uses) | UndefinedVariable |
| `check_scopes` | Checks scope-level properties | (reserved) |
| `check_declarations` | Checks `declare`/`local`/`export` | LocalTopLevel |
| `check_call_sites` | Checks function call patterns | OverwrittenFunction |
| `check_source_refs` | Checks `source`/`.` commands | DynamicSourcePath |
| `check_command_facts` | Filters command facts (name, options, structure) | ReadWithoutRaw, InvalidExitStatus, PrintfFormatVariable |
| `check_word_and_expansion_facts` | Filters word/expansion facts | UnquotedExpansion, TrapStringExpansion, CasePatternVar |
| `check_loop_list_and_pipeline_facts` | Filters pipeline/loop/list facts | FindOutputToXargs, FindOutputLoop, LoopFromCommandOutput |
| `check_redirect_and_substitution_facts` | Filters redirect/substitution facts | SudoRedirectionOrder, SubstWithRedirect |
| `check_surface_fragment_facts` | Filters lexer-level fragment facts | LegacyBackticks, SingleQuotedLiteral |
| `check_test_and_conditional_facts` | Filters test/conditional facts | ConstantComparisonTest, QuotedBashRegex |
| `check_flow` | Needs CFG/dataflow analysis | UnreachableAfterExit |

**Decision guide:** If the rule iterates `checker.semantic().*`, use one of the first six
phases. If it iterates `checker.facts().*`, pick the fact-category phase that matches.
Most new rules will use one of the fact-based phases (`check_command_facts` through
`check_test_and_conditional_facts`).

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
