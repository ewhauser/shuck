#!/bin/bash
# shellcheck disable=2034,2082,2299

# Should trigger: assigning to 0 is a zsh-specific idiom.
0="${${ZERO:-default}:-value}"

# Should also trigger when the value is simpler.
0="$PWD"

# Should not trigger in other assignments.
script_name="$0"
