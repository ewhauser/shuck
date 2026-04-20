#!/bin/sh

# Invalid: the first definition is replaced before any direct call can reach it.
myfunc() { return 1; }
myfunc() { return 0; }
myfunc

# Valid: the first definition is exercised before the overwrite happens.
ok() { printf '%s\n' first; }
ok
ok() { printf '%s\n' second; }
ok

# Valid: unrelated functions are unaffected.
left() { :; }
right() { :; }
left
right
