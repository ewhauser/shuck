#!/bin/bash

# Should trigger: an incomplete getopts handler uses bare single-letter labels.
while getopts "hb:c:" opt; do
  case "$opt" in
    h)
      echo help
      exit 0
      ;;
    b)
      bg="$OPTARG"
      ;;
  esac
done
echo "${bg:-}"

# Should not trigger: quoted labels avoid the bare-label style warning.
while getopts "hb:c:" opt; do
  case "$opt" in
    "h") : ;;
    "b") : ;;
  esac
done

# Should not trigger: complete or fallback-backed handlers are left alone.
while getopts "ab" opt; do
  case "$opt" in
    a) : ;;
    b) : ;;
  esac
done

while getopts "ab" opt; do
  case "$opt" in
    a) : ;;
    *) : ;;
  esac
done
