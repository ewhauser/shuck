#!/bin/bash

arr=(alpha "two words")
for item in "$*" "${arr[*]}" "x$*y"; do
  :
done

for item in "$@" "${arr[@]}" ${arr[*]}; do
  :
done
