#!/bin/sh

top_level() {
  # shellcheck source=/dev/null
  source ./inside.sh
}

# Should not trigger: top-level source belongs to X031 instead
source ./top-level.sh
