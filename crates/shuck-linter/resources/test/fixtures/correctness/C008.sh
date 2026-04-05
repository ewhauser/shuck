#!/bin/sh

x=1
handler='echo ready'

# Invalid: variables inside a double-quoted trap body expand immediately
trap "echo $x" EXIT

# Invalid: the same issue applies with `trap --`
trap -- "printf '%s\n' $handler" INT

# Valid: single quotes defer expansion until the trap runs
trap 'echo $x' TERM

# Valid: plain literal trap bodies are fine
trap "echo ready" HUP
