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

# Invalid: general fallthrough chains are part of the same warning family.
[ "$x" = foo ] && printf '%s\n' yes || rm -f no
true && false || printf '%s\n' fallback

# Valid: common status-propagation and formatter idioms stay exempt.
cond && return 0 || return 1
ready && printf '%s\n' on || printf '%s\n' off

# Valid: assignment ternaries belong to other rules.
[ -n "$x" ] && out=foo || out=bar

# Valid: pure && and pure || chains are not this rule.
test -n "$x" && printf '%s\n' ok
test -n "$x" || printf '%s\n' missing
