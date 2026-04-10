#!/bin/sh

# Should trigger: array key expansion with [*]
printf '%s\n' "${!ARRAY[*]}"

# Should trigger: array key expansion with [@]
printf '%s\n' "${!ARRAY[@]}"

# Should not trigger: plain indirect expansion stays with X018
printf '%s\n' "${!name}"

# Should not trigger: prefix matching expansion stays with X018
printf '%s\n' "${!build_option_@}"
