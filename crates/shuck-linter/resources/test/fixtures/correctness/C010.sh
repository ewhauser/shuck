#!/bin/sh

x=foo

# Invalid: every branch is a condition, so the mixed chain obscures the logic.
[ "$x" = foo ] && [ "$x" = bar ] || [ "$x" = baz ]

# Invalid: the same problem appears when the chain starts with `||`.
false || true && [ "$x" = baz ]

# Valid: an explicit branch keeps the control flow clear.
if [ "$x" = foo ]; then
  printf '%s\n' match
else
  printf '%s\n' miss
fi

# Valid: non-test fallthrough chains and assignment ternaries belong to other rules.
true && false || printf '%s\n' fallback
[ -n "$x" ] && out=foo || out=bar

# Valid: pure && and pure || chains are not this rule.
test -n "$x" && printf '%s\n' ok
test -n "$x" || printf '%s\n' missing
