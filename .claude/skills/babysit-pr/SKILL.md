---
name: babysit-pr
description: Continuously monitor and fix a GitHub PR until all review comments are addressed and CI is passing. Use when the user wants to hand off a PR for automated cleanup, says "babysit this PR", "fix up my PR", "get this PR ready to merge", or wants to walk away while Claude handles review feedback and CI failures.
---

# Babysit PR

Autonomously monitor a GitHub PR, address all review comments, and fix CI failures until the PR is ready to merge.

## Workflow

Loop until the PR is clean (no unresolved comments AND CI passing):

### 1) Check PR status
- Run `gh pr view --json number,url,state,reviewDecision,statusCheckRollup` to get current state
- Run `gh pr checks` to see CI status

### 2) Address review comments
- Use the `gh-address-comments` skill to fetch and fix all review comments
- Apply fixes immediately without asking for approval

### 3) Fix CI failures
- Use the `gh-fix-ci` skill to identify and fix any failing checks
- Apply fixes immediately without asking for approval

### 4) Push changes
- If any fixes were made, commit and push them
- Use a clear commit message describing what was addressed

### 5) Wait and re-check
- After pushing, sleep for 5 minutes to allow CI to start
- Then poll `gh pr checks` until all checks have completed (no "pending" or "in_progress" status)
- Once CI completes, loop back to step 1

### Exit conditions
- **Success**: No unresolved review comments AND all checks passing
- **Stuck**: Same failure persists after 3 fix attempts (report to user)
- **External blocker**: Waiting on reviewer action or external CI system

## Notes
- Prioritize review comments over CI fixes (reviewers may have feedback that affects CI)
- If a fix attempt breaks something else, revert and try a different approach
- Keep the user informed of progress with brief status updates between iterations
