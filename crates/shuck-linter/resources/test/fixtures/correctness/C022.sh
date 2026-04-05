#!/bin/sh

# Invalid: an empty test expression has no operands
[ ]
test

# Valid: populated tests are fine
[ "$value" ]
test "$value"
