#!/bin/sh

# Should trigger: declared options are handled, but invalid flags still fall through.
while getopts "ab" opt; do
  case "$opt" in
    a) : ;;
    b) : ;;
  esac
done

# Should trigger: handling ':' in silent mode still leaves invalid flags uncovered.
while getopts ":a" opt; do
  case "$opt" in
    a) : ;;
    :) : ;;
  esac
done

# Should not trigger: an explicit invalid-option branch is present.
while getopts "a" opt; do
  case "$opt" in
    a) : ;;
    \?) : ;;
  esac
done

# Should not trigger: a catch-all branch covers unexpected flags.
while getopts "ab" opt; do
  case "$opt" in
    a) : ;;
    *) : ;;
  esac
done
