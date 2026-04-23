#!/bin/bash

# Invalid: exporting parameter expansions instead of direct names.
export "$@"
export ${@}
export "$name"
export ${name}
export "${name}"
export $name
export "$1"
export ${1}
export "$*"
export ${*}
export "$#"
export -- "$name"

# Valid: export explicit names.
export HOME PATH
export HOME="$PWD"

# Valid: slices, operators, arrays, indirect refs, and mixed words are outside this rule.
export "${@:2}"
export "$@$@"
export "prefix$name"
export "${name:-fallback}"
export "${!name}"
export "${arr[@]}"
export "${arr[0]}"

# Valid: assignment value expansion is outside this rule.
export joined="$@"
