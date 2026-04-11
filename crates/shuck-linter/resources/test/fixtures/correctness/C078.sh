#!/bin/bash
find . -exec echo *.txt {} +
find . -exec echo foo[ab]bar {} +
find . -exec echo $(basename "$dir") {} +
find . -exec echo $(basename "$dir")* {} +
find . -execdir echo "$prefix"*.tmp {} \;

find . -exec echo "$file" {} +
find . -exec echo "*.txt" {} +
find . -exec echo "$(basename "$dir")" {} +
find . -name *.txt -print
printf '*.txt'
