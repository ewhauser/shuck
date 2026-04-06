#!/bin/sh

x=foo
pat=foo

# Invalid: variable expansion builds the case pattern at runtime.
case $x in
  $pat) printf '%s\n' match ;;
esac

# Invalid: command substitution also makes the pattern dynamic.
case $x in
  $(printf '%s' foo)) printf '%s\n' match ;;
esac

# Valid: literal patterns stay stable.
case $x in
  foo|bar) printf '%s\n' literal ;;
esac
