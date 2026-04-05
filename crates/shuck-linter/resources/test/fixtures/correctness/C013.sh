#!/bin/sh

# Invalid: expanding find output in a `for` loop loses pathname boundaries
for file in $(find . -name '*.txt'); do
  printf '%s\n' "$file"
done

# Valid: streaming the output avoids word splitting
find . -name '*.txt' | while IFS= read -r file; do
  printf '%s\n' "$file"
done
