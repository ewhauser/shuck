#!/bin/bash

set -- a 'b c'
arr=('a b' c)

printf '%s\n' $* ${*}
printf '%s\n' ${arr[*]} ${arr[*]:1}
for item in $*; do
  printf '%s\n' "$item"
done
$* --version

printf '%s\n' "$*" "${*}"
printf '%s\n' "${arr[*]}" "${arr[*]:1}"
