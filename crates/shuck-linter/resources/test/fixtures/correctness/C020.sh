#!/bin/bash

# Invalid: bracket and conditional tests on fixed literals are predetermined
[ 1 ]
[ "" ]
[[ x ]]

# Valid: runtime values belong in these tests, and `test` stays out of scope
test foo
[ "$value" ]
[[ $value ]]
