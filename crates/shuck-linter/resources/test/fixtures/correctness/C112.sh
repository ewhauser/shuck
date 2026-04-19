#!/bin/bash

# shellcheck disable=2034,2154

# Invalid: all-elements array expansions in [[ ... ]] tests.
set -- a b
arr=(x y)
if [[ "${sel[@]:0:4}" == "HELP" ]]; then :; fi
if [[ -n "$@" ]]; then :; fi
if [[ x == *${arr[@]}* ]]; then :; fi
if [[ "${@: -1}" == "mM" || "${@:-1}" == "Mm" ]]; then :; fi
if [[ " ${arr[@]} " =~ " x " ]]; then :; fi
if [[ "${arr[@]}" ]]; then :; fi

# Valid: star-selector forms, escaped text, and single-bracket tests.
if [[ "${sel[*]:1}" == "HELP" ]]; then :; fi
if [[ "\${sel[@]:1}" == "HELP" ]]; then :; fi
if [[ x == ${sel[*]}* ]]; then :; fi
if [[ "\$@" ]]; then :; fi
if [ "${sel[@]:1}" = "HELP" ]; then :; fi
