#!/bin/bash

# Invalid: `>` inside `[` redirects instead of comparing.
[ "$version" > "10" ]

# Invalid: literal operands still use a redirecting `>`.
[ 1 > 2 ]

# Valid: redirecting the test command after the closing bracket is unrelated.
[ "$version" ] > "$log"

# Valid: escaped or quoted `>` stays a test operand.
[ "$version" \> "$other" ]
[ "$version" ">" "$other" ]

# Valid: `test` and `[[` are out of scope for this rule.
test "$version" > "$other"
[[ "$version" > "$other" ]]
