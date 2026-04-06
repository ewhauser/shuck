#!/bin/sh

# Invalid: the command substitution writes its stdout to a file instead.
out=$(printf hi > out.txt)

# Invalid: sending stdout to stderr also leaves nothing to capture.
out=$(printf hi >&2)

# Valid: capture the command output directly.
out=$(printf hi)
