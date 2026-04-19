#!/bin/bash

# Invalid: simple test compares only fixed literals
[ 1 = 1 ]

# Invalid: `test` with literal operands is also constant
test foo != bar

# Invalid: bash conditionals can be constant too
[[ left == right ]]

# Invalid: nested constant comparisons inside larger conditions are still fixed
[[ "$value" = ok || left == right ]]

# Valid: runtime data makes the comparison meaningful
[ "$value" = ok ]
[[ $value == right ]]
