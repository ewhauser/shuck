# ShellCheck Mapping Audit

Audit date: 2026-04-17

This note tracks the follow-up work that still remains from the original audit pass.

Remaining work:

- 6 rules become clean once stale `shellcheck_code` metadata on already-mapped rules is refreshed.
- 7 look like duplicates and should be collapsed instead of getting a new mapping.
- 2 still need a cleaner example before a mapping decision is safe.

Important context:

- The drift is not limited to unmapped rules. In the same environment, 75 of 272 already-mapped rules had invalid examples that did not emit their declared `shellcheck_code`.
- The recommendations below are therefore based on current ShellCheck 0.11.0 behavior, not just the existing metadata in `docs/rules/*.yaml`.

## Remaining Metadata Refreshes

| Rule | Current `SC` | Blocker |
| --- | --- | --- |
| `C034` | `SC1046` | Keep paired with `C035` on `SC1047`; both examples currently emit both codes. |
| `C053` | `SC2283` | `C071` currently claims `SC2283`, but its example now emits `SC1105`. |
| `C089` | `SC2078` | `C020` currently claims `SC2078`, but its example now emits `SC2161`. |
| `C126` | `SC2104` | `C018` currently claims `SC2104`, but its example now emits `SC2105`. |
| `X052` | `SC2112` | `X004` currently claims `SC2112`, but its example now emits `SC2113`. |
| `X053` | `SC2277` | `X048` currently claims `SC2277`, but its example now emits `SC1085`. |

## Collapse Candidates

| Rule | Current `SC` | Recommendation |
| --- | --- | --- |
| `C024` | `SC1007` | Keep one canonical `SC1007` assignment-spacing rule. |
| `C026` | `SC1007` | Collapse into the same canonical `SC1007` assignment-spacing rule as `C024`. |
| `C028` | `SC1007` | Collapse into the same canonical `SC1007` assignment-spacing rule as `C024`. |
| `C027` | `SC1010` | Keep one canonical bare-`done` / missing-separator rule. |
| `C029` | `SC1010` | Collapse into the same canonical `SC1010` rule as `C027`. |
| `X034` | `SC1036` | Collapse into `X046`; both currently describe the same invalid `(` in test-pattern context. |
| `X064` | `SC3024` | Collapse into `X045`; both are the same POSIX `+=` portability warning. |

## Rewrite Or Manual Review

| Rule | Current `SC` | Why it still needs help |
| --- | --- | --- |
| `X033` | `SC1027`, `SC1036` | The current example does not isolate a unique portability code. It looks closer to the `X046` extglob-in-test case than to a distinct `elif` portability rule. |
| `X041` | `SC3010`, `SC1087`, `SC2203`, `SC2154` | `SC2203` is the only unique current code, but the example is too noisy to claim it confidently without a cleaner fixture. |

## Suggested Next Pass

1. Refresh the remaining stale mapped codes called out above so the uniqueness constraints are accurate again.
2. Collapse the duplicate families instead of assigning the same `SC` code to multiple Shuck rules.
3. Rewrite `X033` and `X041` examples to isolate the intended behavior before assigning a code.
