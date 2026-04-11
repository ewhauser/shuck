#!/bin/sh

dir=vendor

# Invalid: the fallback depends on the command in the middle.
[ "$dir" = vendor ] && mv go-* "$dir" || mv pkg-* "$dir"

# Invalid: mixing both operators lets the fallback run after the middle command fails.
true && false || printf '%s\n' fallback

# Valid: an explicit branch makes the intent unambiguous.
if [ "$dir" = vendor ]; then
  mv go-* "$dir"
else
  mv pkg-* "$dir"
fi

# Valid for this rule: assignment ternaries are handled separately.
[ -n "$dir" ] && out=vendor || out=other
