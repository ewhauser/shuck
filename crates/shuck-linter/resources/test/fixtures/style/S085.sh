#!/bin/bash
# A longer script with helper functions and top-level dispatch.

readonly LOG_DIR="${LOG_DIR:-/tmp/shuck-demo}"

prepare_log_dir() {
  mkdir -p "${LOG_DIR}"
}

write_header() {
  printf '%s\n' "starting"
}

collect_inputs() {
  printf '%s\n' "$@"
}

write_footer() {
  printf '%s\n' "done"
}

# Padding keeps the fixture above the default non-trivial line threshold.
# The rule should still anchor on the last top-level statement.
# setup
# prepare
# collect
# validate
# write
# clean
# summarize

prepare_log_dir
write_header
collect_inputs "$@"
write_footer
