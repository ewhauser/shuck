#!/bin/bash

i=1

# Invalid: arithmetic expansion should not name the redirect target.
echo hi > "$((i++))"

# Invalid: append redirections have the same problem.
printf '%s\n' ok >> "$((i + 1))"

# Invalid: decrement updates are also not allowed here.
echo hi > "$((i--))"

# Valid: redirect to a normal filename.
echo hi > output.txt
