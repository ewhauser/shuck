#!/bin/bash
set -- a b

# Invalid: positional args folded into one stringy word.
printf '%s\n' "$@$@"
printf '%s\n' "$@""$@"
printf '%s\n' "items: $@"
printf '%s\n' x$@y
x$@y --version
if [ "_$@" = "_--version" ]; then :; fi

# Valid: positional args passed with original boundaries.
printf '%s\n' "$@" "${@}" "${@:1}" ${@} ${@:1}
printf '%s\n' "$*" "${@:-fallback}" "${array[@]}"
value="items: $@"
