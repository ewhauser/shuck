#!/bin/bash

# Invalid: mapfile reads from process substitution.
mapfile -t files < <(find . -name '*.pyc')

# Invalid: readarray is equivalent to mapfile.
readarray -t logs < <(find . -name '*.log')

# Valid: piping into mapfile is outside this rule.
find . -name '*.pyc' | mapfile -t files

# Valid: regular input redirect is fine.
mapfile -t files < input.txt

# Valid: process substitution used as an argument, not stdin source.
mapfile -t files >(wc -l)
