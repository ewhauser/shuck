#!/bin/bash
set -- a b
args=(a b)

# Invalid: all-elements splats folded into one stringy word.
printf '%s\n' "$@$@"
printf '%s\n' "$@""$@"
printf '%s\n' "items: $@"
printf '%s\n' x$@y
x$@y --version
if [ "_$@" = "_--version" ]; then :; fi
printf '%s\n' "items: ${args[@]}"
printf '%s\n' "items: ${!args[@]}"
printf '%s\n' "items: ${args[@]:1}"
printf '%s\n' "items: ${args[@]/a/b}"
printf '%s\n' "items: ${args[@]+foo}"
printf '%s\n' "items: ${args[@]+ ${args[*]}}"

# Valid: positional args passed with original boundaries.
printf '%s\n' "$@" "${@}" "${@:1}" ${@} ${@:1}
printf '%s\n' "${args[@]}" ${args[@]} "${args[@]:1}" "${!args[@]}"
printf '%s\n' "${args[@]+ ${args[*]}}"
printf '%s\n' "$*" "${@:-fallback}" "${args[*]}" "items: ${args[*]}"
value="items: $@"
