#!/bin/sh

# Should trigger: plain array assignment
items=(one two)

# Should trigger: declaration assignment operand using compound syntax
export visible=(left right)

# Should trigger: command-prefixed array assignment before a utility
temp=(first second) printf '%s\n' ok

# Should not trigger: scalar assignments stay portable
scalar=value
export other=still_scalar
