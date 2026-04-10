#!/bin/sh

# Should trigger: longest suffix removal on $*
echo "${*%%dBm*}"

# Should not trigger: short suffix removal is a different form
echo "${*%dBm*}"

# Should not trigger: longest suffix removal on $@ is outside this rule
echo "${@%%dBm*}"
