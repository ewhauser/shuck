#!/bin/sh

# Invalid: command substitution turns lines into shell words.
for line in $(cat input.txt); do
  printf '%s\n' "$line"
done

# Invalid: backticks are the same pattern.
for line in `printf '%s\n' alpha beta`; do
  printf '%s\n' "$line"
done

# Valid: a read loop preserves each line.
while IFS= read -r line; do
  printf '%s\n' "$line"
done < input.txt
