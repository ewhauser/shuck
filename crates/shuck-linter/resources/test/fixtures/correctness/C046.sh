#!/bin/sh

# Invalid: piping into kill does not pass PIDs as arguments
printf '%s\n' 123 | kill -9

# Invalid: nested command substitutions should still be checked
result=$(printf '%s\n' 456 | kill -TERM)

# Valid: kill with explicit PID arguments
kill -9 123

# Valid: xargs bridges stdin into positional arguments
printf '%s\n' 789 | xargs kill -9
