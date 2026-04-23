#!/bin/bash

# Invalid: the same variable name was array-like first, then reassigned as a scalar.
exts=("txt" "pdf" "doc")
exts="${exts[*]}"
echo "$exts"

# Invalid: even a single-element extraction still changes the same name to scalar.
items=("one" "two")
items="${items[0]}"
echo "$items"

# Invalid: later local declarations still reuse the array-typed name as scalar.
f() {
  local exts="archive"
  echo "$exts"
}

# Valid: flattening into a different scalar variable.
joined="${exts[*]}"
echo "$joined"

# Valid: an array-style reference alone does not establish prior array binding state.
echo "${unknown[@]}"
unknown="fallback"
echo "$unknown"
