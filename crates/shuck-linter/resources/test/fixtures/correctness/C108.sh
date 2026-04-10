#!/bin/bash

# Invalid: quoted keys in associative-array unset operands.
declare -A parts
parts[one]=1
parts[two]=2
unset parts["one"]
unset parts['two']
key=two
unset parts["$key"]

# Valid: unquoted key operand for associative arrays.
unset parts[$key]

# Valid: quote the entire operand to keep it literal.
unset 'parts[key]'
unset "parts[key]"

# Valid: indexed arrays are outside this rule.
declare -a nums
nums[1]=one
unset nums["1"]
