#!/bin/bash

# Invalid at script scope
local top_level=bar
printf '%s\n' "$top_level"

# Valid inside a function
f() {
  local inside_function=baz
  printf '%s\n' "$inside_function"
}
f
