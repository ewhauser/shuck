#!/bin/sh

# Invalid: redirecting inside the substitution makes the capture empty.
out=$(printf hi >/dev/null 2>&1)

# Invalid: an explicit stdout redirect has the same issue.
out=$(printf hi 1>/dev/null)

# Valid: keep the redirect outside the substitution.
out=$(printf hi) >/dev/null 2>&1
