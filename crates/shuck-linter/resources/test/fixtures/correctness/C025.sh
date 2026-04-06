#!/bin/sh

# Invalid: $10 is parsed as $1 followed by a literal 0.
printf '%s\n' "$10"

# Invalid: larger numbers have the same problem.
printf '%s\n' $123

# Valid: braces address the positional parameter directly.
printf '%s\n' "${10}"
printf '%s\n' "${123}"

# Valid: concatenating $1 with non-digits is fine.
printf '%s\n' "$1x"
