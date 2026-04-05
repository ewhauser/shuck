#!/bin/bash

# Invalid: a bare literal in a test is predetermined
[ 1 ]
test foo
[[ bar ]]

# Valid: runtime values belong in these tests
[ "$value" ]
[[ $value ]]
