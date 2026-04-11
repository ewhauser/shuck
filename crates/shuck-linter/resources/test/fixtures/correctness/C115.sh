#!/bin/sh

# Invalid: the `||` branch catches both a false condition and a failed assignment branch.
[ -z "$str" ] && domain=$domain || domain=$str

# Invalid: the same shape appears with simple literal assignments.
[ -n "$HOME" ] && mode=interactive || mode=batch

# Valid: an explicit branch avoids ternary fallthrough.
if [ -n "$HOME" ]; then
  mode=interactive
else
  mode=batch
fi

# Valid for other rules: test-only and command fallthrough chains are handled elsewhere.
[ "$x" = foo ] && [ "$x" = bar ] || [ "$x" = baz ]
cmd && first || second
