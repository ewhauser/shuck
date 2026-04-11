#!/bin/sh

for item in a; do
  printf '%s\n' "$item"
done

for item in "a"; do
  printf '%s\n' "$item"
done

for item in "${TERMUX_SCRIPTDIR}"/packages/python/0009-fix-ctypes-util-find_library.patch; do
  printf '%s\n' "$item"
done

for item in "$(printf /tmp)"/x.patch; do
  printf '%s\n' "$item"
done

for item in ~; do
  printf '%s\n' "$item"
done

for item in a b; do
  printf '%s\n' "$item"
  break
done

for item in "$@"; do
  printf '%s\n' "$item"
done

for item in foo${bar}baz; do
  printf '%s\n' "$item"
done

for item in *.txt; do
  printf '%s\n' "$item"
done

for item in $(printf a); do
  printf '%s\n' "$item"
done
