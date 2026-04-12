#!/bin/bash

value=foobar

# Should trigger: duplicate patterns shadow later case arms.
case "$value" in
  foo) : ;;
  foo) : ;;
esac

# Should trigger: a wildcard arm shadows a later literal.
case "$value" in
  foo*) : ;;
  foobar) : ;;
esac

# Should trigger: earlier alternatives in the same arm can shadow later patterns.
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

# Should not trigger: extglob reachability is not analyzed here.
shopt -s extglob
case "$value" in
  @(foo|bar)*) : ;;
  foobar) : ;;
esac
