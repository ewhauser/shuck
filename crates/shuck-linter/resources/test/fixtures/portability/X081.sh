#!/bin/sh

# Should trigger: longest suffix removal on $*
echo "${*%%dBm*}"

# Should trigger: longest prefix removal on $@
echo "${@##*.}"

# Should not trigger: trimming on a scalar is unrelated
echo "${name%%dBm*}"

# Should not trigger: replacement uses a different portability rule
echo "${*//dBm*/x}"
