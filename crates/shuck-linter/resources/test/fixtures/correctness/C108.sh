#!/bin/bash

# Invalid: array-subscript operands in `unset` should stay literal.
declare -A parts
parts[one]=1
parts[two]=2
unset parts["one"]
unset parts['two']
key=two
unset parts[$key]
unset parts["$key"]

# Valid: quote the entire operand to keep it literal.
unset 'parts[key]'
unset "parts[key]"

# Invalid: indexed arrays are also subject to this check.
declare -a nums
nums[1]=one
unset nums[1]
unset nums["1"]
