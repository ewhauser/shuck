#!/bin/bash

# Invalid: set flags without a leading prefix
set euox pipefail

# Invalid: positional arguments should be explicit with --
set foo bar

# Valid: short options are prefixed
set -euo pipefail

# Valid: explicit positional separator
set -- foo bar

# Valid: single positional argument is unambiguous
set foo

# Valid: non-identifier positional values are not set flags
set n-aliases.conf n-env.conf
