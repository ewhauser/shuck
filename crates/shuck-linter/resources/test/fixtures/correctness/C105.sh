#!/bin/bash

# Invalid: exporting positional-parameter splats.
export "$@"
export ${@}
export "${@:2}"
export -- "$@"

# Valid: export explicit names.
export HOME PATH
export HOME="$PWD"

# Valid: assignment value expansion is outside this rule.
export joined="$@"

# Valid: mixed strings are handled by a different rule family.
export "prefix$@suffix"
