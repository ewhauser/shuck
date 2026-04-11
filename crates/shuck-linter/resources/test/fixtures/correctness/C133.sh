#!/bin/bash

# Invalid: array flattened into same-name scalar, then reused as scalar text.
exts=("txt" "pdf" "doc")
exts="${exts[*]}"
exts+=" ${exts^^}"
echo "$exts"

# Valid: flattening into a different scalar variable.
joined="${exts[*]}"
echo "$joined"

# Valid: non all-elements subscript does not trigger this rule.
head="${exts[0]}"
echo "$head"
