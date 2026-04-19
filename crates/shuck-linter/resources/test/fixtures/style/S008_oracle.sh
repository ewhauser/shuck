#!/bin/bash

set -- a 'b c'
arr=('a b' c)

printf '%s\n' $@ ${@:2}
printf '%s\n' ${arr[@]} ${arr[@]:1}
for item in $@; do
  printf '%s\n' "$item"
done

printf '%s\n' "$@" "${@:2}"
printf '%s\n' "${arr[@]}" "${arr[@]:1}"
for item in "$@"; do
  printf '%s\n' "$item"
done
for item; do
  printf '%s\n' "$item"
done
