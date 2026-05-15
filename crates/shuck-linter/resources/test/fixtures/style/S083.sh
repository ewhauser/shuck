#!/bin/bash
do_work() {
  local input=$1
  echo "processing: $input"
  echo "validating: $input"
  echo "loading: $input"
  echo "normalizing: $input"
  echo "planning: $input"
  echo "executing: $input"
  echo "collecting: $input"
  echo "summarizing: $input"
  echo "finished: $input"
}

# Processes the given input string and writes the result to stdout.
do_more_work() {
  local input=$1
  echo "processing: $input"
}
