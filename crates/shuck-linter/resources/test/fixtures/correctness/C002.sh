#!/bin/sh

lib=./lib.sh

# Invalid: the sourced path is built at runtime.
. "$lib"

# Invalid: a variable plus a static suffix is still dynamic.
. "$lib".generated

# Valid: literal paths are analyzable.
. "./lib.sh"

# Valid: an explicit source directive pins the dependency.
# shellcheck source=generated.sh
. "$maybe_generated"
