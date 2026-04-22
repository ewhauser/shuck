#!/bin/sh

# Invalid: command substitution turns lines into shell words.
for line in $(cat input.txt); do
  printf '%s\n' "$line"
done

# Invalid: all-line-oriented pipelines behave the same way.
for line in $(grep foo input.txt | cut -d: -f1); do
  printf '%s\n' "$line"
done

# Valid: a read loop preserves each line.
while IFS= read -r line; do
  printf '%s\n' "$line"
done < input.txt

# Valid: safe generators and mixed pipelines do not match this rule.
for line in `printf '%s\n' alpha beta`; do
  printf '%s\n' "$line"
done
