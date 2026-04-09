#!/bin/sh

# Invalid: a literal helper that is not available should report.
. ./missing.sh

# Invalid: a directive-pinned helper still reports when that file is missing.
# shellcheck source=missing-directed.sh
. "$generated_helper"

# Valid: a helper that exists next to the script is available to the analysis.
. ./c003_helper.sh

# Valid: runtime-built source paths belong to C002 instead.
. "$dynamic_helper"

# Valid: explicit opt-out directives should not produce C003.
# shellcheck source=/dev/null
. "$optional_plugin"
