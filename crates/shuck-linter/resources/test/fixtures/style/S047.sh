#!/bin/bash
LAYOUTS="$(ls layout.*.h | cut -d. -f2 | xargs echo)"
count="$(ls *.html | wc -l)"
ls /tmp | sort
while read -r file; do
  printf '%s\n' "$file"
done < <(ls | sed 1q)
