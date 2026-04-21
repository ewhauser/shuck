#!/bin/bash

# Valid: mapfile can read from a process substitution in the current shell.
mapfile -t files < <(find . -name '*.pyc')

# Valid: readarray behaves the same way.
readarray -t logs < <(find . -name '*.log')

# Valid: explicit file-descriptor input is accepted too.
mapfile -u 3 -t files 3< <(find . -name '*.tmp')

# Valid: piping into mapfile is handled by subshell assignment rules instead.
find . -name '*.pyc' | mapfile -t files

# Valid: regular input redirect is fine.
mapfile -t files < input.txt

# Valid: process substitution used as an argument, not stdin source.
mapfile -t files >(wc -l)
