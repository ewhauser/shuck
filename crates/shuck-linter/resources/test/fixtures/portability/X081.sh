#!/bin/sh

# Should trigger: pattern trimming on $*
echo "${*%%dBm*}"
echo "${*%dBm*}"
echo "${*##dBm*}"
echo "${*#dBm*}"

# Should trigger: pattern trimming on $@
echo "${@%%dBm*}"
echo "${@%dBm*}"
echo "${@##*.}"
echo "${@#*.}"

# Should not trigger: named parameters are outside this rule
echo "${name%%dBm*}"
