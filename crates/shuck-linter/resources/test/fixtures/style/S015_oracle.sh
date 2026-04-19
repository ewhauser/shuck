#!/bin/bash

set -- a 'b c'

for item in "$*"; do
  printf '%s\n' "$item"
done

for item in "$@"; do
  printf '%s\n' "$item"
done
for item; do
  printf '%s\n' "$item"
done
