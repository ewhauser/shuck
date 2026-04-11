#!/bin/bash

# shellcheck disable=2034

# Invalid: quoted unindexed BASH_SOURCE expansion.
x="$BASH_SOURCE"
y="${BASH_SOURCE}"
printf '%s\n' "$BASH_SOURCE" "${BASH_SOURCE}"
source "$(dirname "$BASH_SOURCE")/helper.bash"
if [[ "$BASH_SOURCE" == "main.bash" ]]; then :; fi
for item in "$BASH_SOURCE"; do
  :
done

# Valid: unquoted forms are outside C100 scope.
x=$BASH_SOURCE
y=${BASH_SOURCE}

# Valid: indexed and array-selector forms are explicit.
z="${BASH_SOURCE[0]}"
q="${BASH_SOURCE[@]}"
r="${BASH_SOURCE[*]}"

# Valid: operation forms are outside this rule.
s="${BASH_SOURCE%/*}"
t="${BASH_SOURCE:-fallback}"
