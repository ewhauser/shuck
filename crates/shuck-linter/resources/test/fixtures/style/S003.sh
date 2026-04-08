#!/bin/sh

for entry in $(ls); do
  printf '%s\n' "$entry"
done

for entry in $(find . -type f); do
  printf '%s\n' "$entry"
done

for entry in $(tail -n +2 serverlist.csv | cut -d ',' -f1); do
  printf '%s\n' "$entry"
done

for entry in *.sh; do
  printf '%s\n' "$entry"
done
