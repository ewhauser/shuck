#!/bin/sh

# Invalid: an empty test expression has no operands
[ ]

# Valid: populated tests are fine
[ "$value" ]
test
test "$value"
