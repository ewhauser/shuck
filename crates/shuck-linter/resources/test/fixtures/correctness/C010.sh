#!/bin/sh

x=foo

# Valid: this contradictory all-test chain belongs to a different warning family.
[ "$x" = foo ] && [ "$x" = bar ] || [ "$x" = baz ]

# Valid: `A || B && C` is not part of this warning family.
false || true && [ "$x" = baz ]

# Valid: an explicit branch keeps the control flow clear.
if [ "$x" = foo ]; then
  printf '%s\n' match
else
  printf '%s\n' miss
fi

# Invalid: general fallthrough chains are part of the same warning family.
[ "$x" = foo ] && printf '%s\n' yes || rm -f no
check_ready && log_ok || log_fail

# Valid: common status-propagation and formatter idioms stay exempt.
cond && return 0 || return 1
ready && printf '%s\n' on || printf '%s\n' off
test -d x && chmod 755 x || echo "chmod failed"

# Valid: assignment ternaries belong to other rules.
[ -n "$x" ] && out=foo || out=bar

# Valid: pure && and pure || chains are not this rule.
test -n "$x" && printf '%s\n' ok
test -n "$x" || printf '%s\n' missing
