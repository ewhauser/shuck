#!/bin/sh

top_level() {
  source ./inside.sh
}

# Should not trigger: top-level source belongs to X031 instead
source ./top-level.sh
