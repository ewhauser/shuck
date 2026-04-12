#!/bin/bash

# Invalid: mixing `&&` and `||` at the same logical level obscures intent.
[[ -n $a && -n $b || -n $c ]]

# Invalid: the same ambiguity applies when the operators appear in the opposite order.
[[ -n $a || -n $b && -n $c ]]

# Invalid: wrapping the whole mixed subexpression does not group the inner operators.
[[ ( -n $a && -n $b || -n $c ) && -n $d ]]

# Valid: explicit grouping keeps the `||` subexpression separate.
[[ -n $a && ( -n $b || -n $c ) ]]

# Valid: explicit grouping on the `&&` side is also clear.
[[ ( -n $a && -n $b ) || -n $c ]]

# Valid: a condition that only uses one logical operator does not need extra grouping.
[[ -n $a && -n $b && -n $c ]]
