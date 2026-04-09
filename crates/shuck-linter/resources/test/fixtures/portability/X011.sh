#!/bin/sh

# Should trigger: direct here-string
cat <<< hi

# Should trigger: here-string in a nested command substitution
count="$(wc -c <<< "$value")"
printf '%s\n' "$count"

# Should not trigger: portable here-doc
cat <<EOF
hello
EOF
