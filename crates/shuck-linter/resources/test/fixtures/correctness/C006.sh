#!/bin/bash

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
