#!/bin/sh

# Should trigger: positional substring expansion
printf '%s\n' "${1:1}"

# Should trigger: scalar substring expansion
printf '%s\n' "${name:2:3}"

# Should trigger: positional slicing on $@ and $*
printf '%s\n' "${@:1}" "${*:1:2}"

# Should not trigger: array slices are handled separately from scalar substring expansion
printf '%s\n' "${arr[@]:1}" "${arr[0]:1}"
