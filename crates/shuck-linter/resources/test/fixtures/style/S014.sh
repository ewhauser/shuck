#!/bin/bash

arr=(alpha "two words")
printf '%s\n' $*
printf '%s\n' ${arr[*]}
for item in ${*:1}; do
  :
done
$* --version

printf '%s\n' "$*"
printf '%s\n' "${arr[*]}"
printf '%s\n' ${arr[@]}
value=$*
