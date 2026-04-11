#!/bin/bash

# shellcheck disable=2034,2154

# Invalid: quoted all-elements array slices in scalar assignments.
params="${@:5}"
joined="prefix${@:2}suffix"
declare declared="${arr[@]:1}"
readonly packed="${arr[@]:1:2}"

f() {
  local nested="${@:3}"
}

# Valid: unquoted slice forwarding.
params=${@:5}

# Valid: non-slice expansions.
joined="${@}"
joined="${@:-fallback}"

# Valid: star-selector slices are outside this rule.
joined="${arr[*]:1}"

# Valid: array compound assignments keep element boundaries.
arr=("${@:2}")
declare -a copied=("${arr[@]:1}")

# Valid: comparison contexts are handled by C112.
if [ "${arr[@]:1}" = foo ]; then :; fi
