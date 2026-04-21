#!/bin/bash

# Invalid: unary string tests on literals are always fixed
[ -z foo ]
test -n bar
[[ -z baz ]]
[ -z "${rootfs_path}_path" ]

# Valid: checking runtime values is meaningful
[ -z "$value" ]
[[ -n $value ]]
