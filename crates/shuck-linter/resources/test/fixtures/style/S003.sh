#!/bin/sh

for entry in $(ls); do
  printf '%s\n' "$entry"
done

for entry in $(printf '%s\n' a b); do
  printf '%s\n' "$entry"
done

for entry in *.sh; do
  printf '%s\n' "$entry"
done
