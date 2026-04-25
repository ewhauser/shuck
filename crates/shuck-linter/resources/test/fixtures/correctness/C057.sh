#!/bin/sh

# Invalid: redirecting the captured command's stdout leaves the substitution empty.
out=$(printf hi > out.txt)

# Invalid: redirecting stdout to stderr has the same capture issue.
out=$(printf hi >&2)

# Invalid: a compound wrapper redirect also controls the substitution output.
out=$({ printf hi; } >/dev/tty)

# Baseline direct capture.
out=$(printf hi)

# Valid: stderr is captured before stdout is redirected elsewhere.
choice=$(printf hi 2>&1 >/dev/tty)

# Valid: mixed output still leaves captured output available.
out=$(printf quiet >/dev/null; printf loud)
