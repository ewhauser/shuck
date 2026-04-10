#!/bin/sh

# Should trigger: bash-only file-slurp command substitution
first=$(< input.txt)
second="$( < spaced.txt )"
third=$(0< fd.txt)
fourth=$(< quiet.txt 2>/dev/null)

# Should not trigger: portable command substitution with an explicit command
portable=$(cat < input.txt)

# Should not trigger: redirect-only substitution that does not read stdin
other=$(> out.txt)

# Should not trigger: assignment-only substitution
assigned=$(foo=bar)
