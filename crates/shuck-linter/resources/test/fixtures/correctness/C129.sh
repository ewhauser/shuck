#!/bin/bash

value=foobar

# Should trigger: duplicate patterns make the later arm unreachable.
case "$value" in
  foo) : ;;
  foo) : ;;
esac

# Should trigger: a catch-all arm makes a later glob unreachable.
case "$value" in
  *) : ;;
  foo*) : ;;
esac

# Should trigger: earlier alternatives in the same arm can make later patterns unreachable.
case "$value" in
  *|foo*) : ;;
  foo) : ;;
esac

# Should not trigger: character classes are intentionally left out of this reachability check.
case "$value" in
  [ab]*) : ;;
  afoo) : ;;
esac

# Should not trigger: continue-matching terminators keep later arms reachable.
case "$value" in
  foo) : ;;&
  foo) : ;;
esac

# Should not trigger: escaped wildcards are literal characters, not catch-all patterns.
case "$value" in
  \?) : ;;
  :) : ;;
  *) : ;;
esac
