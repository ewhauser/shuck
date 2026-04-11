#!/bin/bash

# Invalid: bracket tests should not escape the negation token.
[ \! -f "$file" ]

# Invalid: the `test` builtin takes plain `!` too.
test \! -n "$value"

# Invalid: escaped negation still looks wrong in longer test expressions.
[ \! "$value" = ok ]

# Valid: plain negation is the intended spelling.
[ ! -f "$file" ]
test ! -n "$value"

# Valid: a literal bang used as data should not trigger.
test !
[ "$value" = \! ]
[[ \! -f "$file" ]]
