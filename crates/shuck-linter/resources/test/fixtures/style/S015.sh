#!/bin/bash

arr=(alpha "two words")
for item in "$*"; do
  :
done

for item in "${arr[*]}"; do
  :
done

for item in "x$*y"; do
  :
done

for item in "$@" "${arr[@]}" ${arr[*]}; do
  :
done
