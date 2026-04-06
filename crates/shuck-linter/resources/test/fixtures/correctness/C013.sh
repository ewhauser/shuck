#!/bin/sh

# Invalid: expanding find output in a `for` loop loses pathname boundaries
for file in $(find . -name '*.txt'); do
  printf '%s\n' "$file"
done

# Invalid: wrapper commands should still normalize to find
for file in $(command find . -name '*.txt'); do
  printf '%s\n' "$file"
done

# Valid: streaming the output avoids word splitting
find . -name '*.txt' | while IFS= read -r file; do
  printf '%s\n' "$file"
done

# Valid: post-processing the find output is outside this rule's narrowed shape
for file in $(find . -name '*.txt' | sort); do
  printf '%s\n' "$file"
done

# Valid: another post-processing pipeline should stay quiet
for file in $(find . -name '*.txt' | sed 's|^\./||'); do
  printf '%s\n' "$file"
done
