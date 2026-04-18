#!/bin/sh

# Invalid: the subshell assignment does not update the outer shell binding.
count=0
(count=1)
echo "$count"

items=old
(items=new)
printf '%s\n' "$items"

# Invalid: pipeline-child updates stay in the child shell.
count=0
printf '%s\n' x | while read -r _; do count=1; done
echo "$count"

# Valid: a parent-shell reassignment after the subshell changes the visible value.
items=old
(items=new)
items=latest
echo "$items"
