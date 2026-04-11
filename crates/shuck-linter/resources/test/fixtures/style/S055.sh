#!/bin/sh
LOCS=*.oxt
LOCS="*.oxt"
LOCS=$dir/*.oxt
LOCS="$dir"/*.oxt
LOCS=${dir}/*.oxt
LOCS=$(pwd)/*.oxt
LOCS=\*.oxt
readonly LOCS=foo*bar
export LOCS=$dir/*.txt
LOCS=(*.oxt)
LOCS=("$dir"/*.txt)
LOCS=${name#*:}
