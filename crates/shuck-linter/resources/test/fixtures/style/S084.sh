#!/bin/bash
# Builds an absolute output path.
build_path() {
  local suffix=$1
  echo "${BASE_DIR}/${suffix}"
  return 0
}

# Builds an absolute output path by joining BASE_DIR with a suffix.
#
# Globals:
#   BASE_DIR
# Arguments:
#   $1 - The path suffix to append.
# Outputs:
#   Writes the constructed path to stdout.
# Returns:
#   0 always.
build_more_path() {
  local suffix=$1
  echo "${BASE_DIR}/${suffix}"
  return 0
}
