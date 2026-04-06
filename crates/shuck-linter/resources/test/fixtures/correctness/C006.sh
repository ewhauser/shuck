#!/bin/bash

# Bash runtime vars should not be reported as undefined.
printf '%s %s %s %s %s %s\n' \
  "$LINENO" \
  "$FUNCNAME" \
  "${FUNCNAME[0]}" \
  "$BASH_SOURCE" \
  "${BASH_SOURCE[0]}" \
  "${BASH_LINENO[0]}"

echo "$missing"

if true; then
  maybe=1
fi
echo "$maybe"

f() {
  local local_only
  printf '%s\n' "$local_only"
  readonly declared
  export exported
  printf '%s %s %s\n' "$1" "$@" "$#"
}
f
