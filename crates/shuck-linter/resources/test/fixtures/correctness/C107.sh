#!/bin/bash

# Invalid: test-style commands should use the earlier command directly.
run
if [ $? -ne 0 ]; then :; fi

run
[[ $? -ne 0 ]]

run && [ $? -eq 0 ]

run || [ $? -ne 0 ]

{ [ $? -ne 0 ]; }

check_status() {
  if [ $? -ne 0 ]; then :; fi
  [ $? -ne 0 ]
  run && [ $? -ne 0 ]
}

if [ "$x" = y ]; then
  [ $? -ne 0 ]
fi

# Valid: saving the status keeps the intended result.
run
saved=$?
if [ "$saved" -ne 0 ]; then :; fi

# Valid: non-test uses of `$?` stay out of C107.
case $? in
  0) : ;;
esac
test $? -ne 0
exit $?
[ $? -eq 1 ]
[[ "$name" = ok || $? -eq 1 ]]
