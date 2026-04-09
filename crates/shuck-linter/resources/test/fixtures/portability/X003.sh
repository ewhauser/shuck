#!/bin/sh

# Should trigger: local inside a POSIX sh function
inside_function() {
  local name=portable
  printf '%s\n' "$name"
}
inside_function

# Should trigger: wrapped local stays non-portable in sh
wrapped_function() {
  command local wrapped=value
  printf '%s\n' "$wrapped"
}
wrapped_function

# Should trigger: local at script scope in sh
local top_level=value
printf '%s\n' "$top_level"

# Should not trigger: plain assignments stay portable
portable_name=value
printf '%s\n' "$portable_name"
