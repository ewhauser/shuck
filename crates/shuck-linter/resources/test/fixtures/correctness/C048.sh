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

# Valid: real glob structure suppresses the warning even when the pattern is dynamic.
case $x in
  gm$MAMEVER*) printf '%s\n' glob ;;
  *${IDN_ITEM}*) printf '%s\n' glob ;;
  ${pat}*) printf '%s\n' glob ;;
  *${pat}) printf '%s\n' glob ;;
  x${pat}*) printf '%s\n' glob ;;
  [$hex]) printf '%s\n' glob ;;
  @($pat|bar)) printf '%s\n' glob ;;
  x$left@(foo|bar)) printf '%s\n' glob ;;
esac

# Valid: arithmetic-only case patterns are treated as stable enough for this rule.
case $x in
  $((error_code <= 125))) printf '%s\n' arithmetic ;;
  $((__git_cmd_idx+1))) printf '%s\n' arithmetic ;;
  x$((1))) printf '%s\n' arithmetic ;;
esac
