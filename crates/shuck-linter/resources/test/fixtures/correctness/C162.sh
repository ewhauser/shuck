#!/bin/bash

demo() {
  local -r result=$(get_value)
  typeset -r strict=$(strict_value)
  echo "$result"
}

multiple() {
  local -r combined=$(first)$(second)
  echo "$combined"
}

ok() {
  local result=$(get_value)
  declare result=$(get_value)
  readonly kept=$(get_value)
  echo "$(render_value)"
  echo "$result$kept"
}
