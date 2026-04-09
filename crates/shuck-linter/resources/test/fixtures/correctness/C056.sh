#!/bin/sh

false

# Invalid: first branch command reads the condition status.
if [ "$x" = y ]; then saved=$?; fi

# Invalid: first loop-body command has the same issue.
while [ "$x" = y ]; do again=$?; break; done

# Valid: this branch status comes from a non-test condition command.
if false; then kept=$?; fi

# Valid: later commands in the branch read updated status.
if [ "$x" = y ]; then :; late=$?; fi

# Invalid: short-circuit checks have the same condition-status behavior.
[[ "$x" = y ]] || return $?

# Valid: non-test short-circuit checks are intentionally exempt.
foo || return $?
