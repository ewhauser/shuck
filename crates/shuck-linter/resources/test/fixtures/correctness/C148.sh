#!/bin/bash

# Invalid: associative-array key is missing its closing bracket.
declare -A table=([left]=1 [right=2)

# Valid: properly closed associative-array keys.
declare -A table_ok=([left]=1 [right]=2)

# Valid: indexed arrays are outside this rule.
declare -a nums=([0]=1 [1=2)

# Valid: non-key words in associative compound assignments.
declare -A pairs=(left one right two)
