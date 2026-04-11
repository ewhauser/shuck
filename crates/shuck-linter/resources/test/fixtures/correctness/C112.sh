#!/bin/bash

# shellcheck disable=2034,2154

# Invalid: all-elements array slices in [[ string comparisons ]].
if [[ "${sel[@]:0:4}" == "HELP" ]]; then :; fi
if [[ "x${@:2}y" == "x" ]]; then :; fi

# Valid: non-slice and star-selector forms.
if [[ "${sel[@]}" == "HELP" ]]; then :; fi
if [[ "${sel[*]:1}" == "HELP" ]]; then :; fi

# Valid: escaped slice marker.
if [[ "\${sel[@]:1}" == "HELP" ]]; then :; fi

# Valid: single-bracket comparisons are out of scope for C112.
if [ "${sel[@]:1}" = "HELP" ]; then :; fi
