#!/bin/bash

# shellcheck disable=2034,2154

# Invalid: all-elements expansions in scalar assignments collapse element boundaries.
params="$@"
params="${@}"
params=${@:5}
joined="${arr[@]}"
fallback="${arr[@]:-fallback}"
quoted="${arr[@]@Q}"
flags+=" ${add_flags[@]}"
targets[$key]="${items[@]}"
CFLAGS+=" ${add_flags[@]}" make
escaped="\\$@"
escaped_slice="\\${@:2}"
declare declared="$@"
readonly packed=${arr[@]}

# Invalid: quoted all-elements array slices also collapse in scalar assignments.
params="${@:5}"
joined="prefix${@:2}suffix"
declare declared="${arr[@]:1}"
readonly packed="${arr[@]:1:2}"

f() {
  local nested="${@:3}"
}

# Valid: star-selector slices are outside this rule.
joined="${arr[*]:1}"

# Valid: replacement forms substitute the replacement word, not the element list.
joined="${@:+fallback}"
joined="${arr[@]:+fallback}"
joined="\$@"
joined="\${@:2}"

# Valid: array compound assignments keep element boundaries.
arr=("${@:2}")
declare -a copied=("${arr[@]:1}")

# Valid: comparison contexts are handled by C112.
if [ "${arr[@]:1}" = foo ]; then :; fi
