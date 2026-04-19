# ShellCheck Mapping Audit

Audit date: 2026-04-17

This note tracks the follow-up work that still remains from the original audit pass.

Remaining work:

- 6 rules become clean once stale `shellcheck_code` metadata on already-mapped rules is refreshed.
- 2 still need a cleaner example before a mapping decision is safe.

Completed since the original audit:

- The 7 duplicate-rule candidates have been collapsed into `C024`, `C027`, `X045`, and `X046`.

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

## Rewrite Or Manual Review

| Rule | Current `SC` | Why it still needs help |
| --- | --- | --- |
| `X033` | `SC1027`, `SC1036` | The current example does not isolate a unique portability code. It looks closer to the `X046` extglob-in-test case than to a distinct `elif` portability rule. |
| `X041` | `SC3010`, `SC2203`, `SC2154` | `SC2203` is the only unique current code, but the example is too noisy to claim it confidently without a cleaner fixture. |

## Suggested Next Pass

1. Refresh the remaining stale mapped codes called out above so the uniqueness constraints are accurate again.
2. Rewrite `X033` and `X041` examples to isolate the intended behavior before assigning a code.
