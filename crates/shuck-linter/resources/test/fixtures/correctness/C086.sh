#!/bin/bash

# Invalid: numeric `<`/`>` in test expressions should use `-lt`/`-gt`.
[ "$version" > "10" ]
[ "$version" < 10 ]

# Invalid: literal operands still use redirecting or lexical operators.
[ 1 > 2 ]
[[ $count > 10 ]]
[[ "$count" < 1 ]]

# Valid: redirects outside the test expression are unrelated.
[ "$version" ] > "$log"

# Valid: plain string ordering is outside this numeric-comparison rule.
[ "$version" > "$other" ]
[ "$version" < "$other" ]
[[ "$version" > "$other" ]]

# Valid: escaped or quoted operators stay test operands.
[ "$version" \> "$other" ]
[ "$version" \< "$other" ]
[ "$version" ">" "$other" ]
[ "$version" "<" "$other" ]

# Valid: decimal/version ordering is handled by C087 instead.
[[ "$version" > 1.2 ]]
[[ 1.2 < "$version" ]]

# Valid: `test` is out of scope for this rule.
test "$version" > 10
