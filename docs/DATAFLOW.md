# Dataflow And C006 Probe Plan

This document tracks the remaining semantic-modeling work after the refreshed
full targeted corpus runs on 2026-04-08 plus the follow-on black-box `SC2154`
probe pass.

The probe pass changed the `C006` diagnosis materially:

- `C001` is still mostly blocked by helper/bootstrap modeling and a smaller
  join-liveness tail.
- `C006` is no longer best understood as a missing `rvm`/framework contract
  problem.
- For `C006`, the next highest-value work is first-occurrence parity,
  array-subscript parity, and inert generated-shell handling.

## Fresh Baseline

| Rule | ShellCheck-only | Shuck-only | Location-only | Biggest raw `Shuck-only` families |
|------|-----------------|------------|---------------|-----------------------------------|
| `C001` | 75 / 44 scripts | 1,239 / 471 scripts | 7 paired / 7 scripts | `termux` 366, `rvm` 322, `void-packages` 131, `bats` 107, `neofetch` 88 |
| `C006` | 98 / 39 scripts | 1,554 / 207 scripts | 6 paired / 5 scripts | `rvm` 1,027, `void-packages` 300, `powerlevel10k` 129 |

Notes:

- Both runs still had the same 11 corpus-noise fixtures and 23 zsh parse
  harness failures outside the rule signal.
- The raw `C006` `Shuck-only` location count is still useful for picking
  families to inspect, but the probe pass showed it should not be read as
  "ShellCheck is quiet at every listed location."
- The probe artifacts live under `/tmp/shuck-sc2154-probes/`, especially
  `/tmp/shuck-sc2154-probes/results.json`.

## Key C006 Probe Findings

### 1. Any in-file binding introduction suppresses `SC2154`

The generic binding suite produced `25/26` quiet cases. Only the pure
no-binding case still emitted `SC2154`.

Covered quiet cases included:

- plain assignment
- dead-branch assignment
- assignment in another function
- `local`, `declare`, `typeset`, `readonly`, `export`, `declare -g`
- name-only declarations and initialized declarations
- nameref
- `${name:=...}`
- `+=`
- array assignment
- arithmetic assignment
- loop variables
- `read`, `mapfile`, `printf -v`, `getopts`

This validates the current broad Shuck suppression policy for names introduced
anywhere in-file.

### 2. ShellCheck reports the first occurrence of a name, not every repeat

The repetition probes showed one report per distinct name per file:

- same-line repeats: one report
- later-line repeats: one report
- repeats across functions: one report
- two names: one report per name

This is now the best explanation for a large share of the raw `rvm` and
`void-packages` `Shuck-only` tail in `C006`.

### 3. Source following is narrow and cwd-sensitive

The source probes split into two groups:

- literal relative and literal repo-relative-like helpers can be followed only
  when ShellCheck runs from the target file's directory and the path is
  literal-followable
- the large-corpus harness does not run ShellCheck that way, so those helpers
  stay unresolved there

The following shapes stayed unresolved in every probe variant:

- `source "$rvm_path/scripts/rvm"`
- `source "${rvm_scripts_path:-$rvm_path/scripts}/hook"`
- `source "$rvm_scripts_path/functions/manage/${kind}"`

This means dynamic helper resolution is still interesting future work, but it
is not the first `C006` parity lever.

### 4. The remaining non-`rvm` tail is mixed

Representative direct-oracle checks showed:

- `powerlevel10k` is an inert quoted-template problem
- `termux docbook-xsl` is escaped literal shell text
- `termux awscli` is array-subscript / indirect-subscript handling
- `void-packages setup/install.sh` is quiet because it carries explicit
  `SC2154` disables
- other `void-packages` helpers still emit direct `SC2154`

So `C006` framework work should stay narrow and case-based, not broad
ambient-provider expansion.

## Workstream 1: Inert Generated Shell Text

### Shared impact

This is still the cleanest shared semantic problem:

- `C001`: `neofetch`
- `C006`: `powerlevel10k`, escaped literal maintainer-script bodies in
  `termux`

### Plan

- Extend `crates/shuck-semantic/src/builder.rs` so quoted heredoc bodies and
  other literal template collectors do not emit outer-file reads or bindings.
- Mirror the same guard in
  `crates/shuck-semantic/src/source_closure.rs`.
- Add regressions for:
  - `read -rd '' config <<'EOF'`
  - `var="$(command cat <<\END ... END)"`
  - escaped-dollar placeholders inside generated scripts

### Done when

- `powerlevel10k` drops out of `C006`
- `neofetch` drops out of `C001`
- escaped literal `termux` maintainer-script bodies stay quiet

## Workstream 2: C006 First-Occurrence And Subscript Parity

### Problem

Shuck currently reports too many `C006` sites because it does not mirror two
observed ShellCheck policies:

- one report per distinct undefined name per file
- no `SC2154` for names used only as array-subscript / indirect-subscript index
  operands

### Plan

- Add a `C006` post-filter so only the first reportable unresolved use of a
  given name is emitted per file.
- Exclude unresolved names that appear only in array-subscript index positions
  such as `__array_start` or `$target`.
- Keep this work generic. Do not encode `rvm`, `termux`, or `void-packages`
  knowledge into the rule.

### Done when

- repeated later uses of the same name in `rvm` stop showing up as extra
  `Shuck-only` locations
- `__array_start` / `$target`-style subscript-index cases disappear from
  `C006`

## Workstream 3: Source-Path Expectations, Not Source-Graph Guessing

### Problem

The probe pass showed that ShellCheck only follows literal helpers in fairly
specific conditions. Most of the current `rvm` source shapes are outside that
window.

### Plan

- Do not treat the current `rvm` tail as evidence for a repo-specific ambient
  contract table.
- After Workstreams 1 and 2, re-audit the smaller remaining source-path cases.
- If future source work is needed, start with literal helper-path parity and
  cwd-sensitive expectations before attempting more dynamic source expansion.

### Done when

- the remaining source-path tail is small enough to classify precisely
- any follow-on source work is justified by direct oracle probes, not by raw
  bucket size alone

## Workstream 4: C001 Helper/Bootstrap Modeling And Join Precision

The probe pass did not materially change the `C001` diagnosis.

### Still-active C001 work

- helper/bootstrap/source-closure modeling for `rvm`, `bats`, `acme.sh`,
  `makeself`, and `termux`
- ambient/framework modeling where helper state stays live through project
  closure
- join-liveness precision for cases like
  `void-packages/common/xbps-src/shutils/update_check.sh`

### Order inside C001

1. finish inert generated-shell handling
2. continue helper/bootstrap/source-closure work
3. revisit branch-join liveness once imported/helper reads are settled

## Verification Checklist

- Add targeted semantic/linter regressions before each implementation step.
- `cargo test -p shuck-semantic`
- `cargo test -p shuck-linter`
- `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C006`
- `make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001`

## Near-Term Order Of Operations

1. Finish inert generated-shell / escaped-dollar handling.
2. Add `C006` first-occurrence parity.
3. Add `C006` array-subscript parity.
4. Re-run targeted `C006` corpus comparisons and direct oracle spot checks.
5. Return to `C001` helper/bootstrap and join-liveness work.
6. Only after that, revisit any remaining `rvm`/framework contract expansion.
