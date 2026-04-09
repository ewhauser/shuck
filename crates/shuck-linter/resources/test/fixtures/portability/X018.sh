#!/bin/sh

name=target

# Should trigger: plain indirect expansion
printf '%s\n' "${!name}"

# Should trigger: indirect expansion with an operator
printf '%s\n' "${!name:-fallback}"

# Should trigger: prefix-match expansion with a leading bang
printf '%s\n' "${!build_option_@}"

# Should trigger: indirect array-key expansion
printf '%s\n' "${!arr[*]}"

# Should not trigger: plain parameter expansion stays portable
printf '%s\n' "${name}"
