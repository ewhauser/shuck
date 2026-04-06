#!/bin/sh

x=foo

# Invalid: the fallback depends on the middle command.
[ "$x" = foo ] && [ "$x" = bar ] || [ "$x" = baz ]

# Invalid: mixing both operators in one chain has the same problem.
true && false || printf '%s\n' fallback

# Valid: an explicit branch keeps the control flow clear.
if [ "$x" = foo ]; then
  printf '%s\n' match
else
  printf '%s\n' miss
fi

# Valid: pure && and pure || chains are not this rule.
test -n "$x" && printf '%s\n' ok
test -n "$x" || printf '%s\n' missing
