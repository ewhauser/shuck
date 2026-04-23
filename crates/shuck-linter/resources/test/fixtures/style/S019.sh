#!/bin/bash
[ "$(grep foo input.txt)" ]
[ -n "$(grep foo input.txt)" ]
[ -z "$(grep foo input.txt)" ]
[[ $(grep foo input.txt) ]]
[[ -n $(grep foo input.txt) ]]
[[ -z $(grep foo input.txt) ]]
[[ $(egrep foo input.txt) ]]
[[ -n $(fgrep foo input.txt) ]]
[[ -n "$1" && ! -f "$1" && -n "$(echo "$1" | GREP_OPTIONS="" \grep -v '^-')" ]]
if grep foo input.txt; then :; fi
[ "$(foo $(grep foo input.txt))" ]
[ "prefix$(grep foo input.txt)" ]
[[ $(grep foo input.txt) = bar ]]
