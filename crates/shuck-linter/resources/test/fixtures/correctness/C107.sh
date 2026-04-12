#!/bin/bash

# Invalid: output commands overwrite the status that later `$?` reads.
run
echo status
if [ $? -ne 0 ]; then :; fi

run
printf '%s\n' status
case $? in
  0) : ;;
esac

check_status() {
  run
  printf '%s\n' status
  return $?
}

run
echo status
saved=$?

# Valid: immediate status checks are handled by other rules.
run
if [ $? -ne 0 ]; then :; fi

# Valid: saving the status before another command keeps the intended value.
run
saved=$?
echo status
if [ "$saved" -ne 0 ]; then :; fi

# Valid: non-output commands stay outside this narrower C107 check.
run
pwd >/dev/null
if [ $? -ne 0 ]; then :; fi
