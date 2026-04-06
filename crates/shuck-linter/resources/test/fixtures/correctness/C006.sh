#!/bin/bash

# Runtime prelude names should not be reported as undefined in Bash.
printf '%s %s %s %s %s %s %s %s %s\n' \
  "$IFS" \
  "$USER" \
  "$HOME" \
  "$SHELL" \
  "$PWD" \
  "$TERM" \
  "$LANG" \
  "$SUDO_USER" \
  "$DOAS_USER"

printf '%s %s %s %s %s %s %s %s %s %s %s %s %s %s\n' \
  "$LINENO" \
  "$FUNCNAME" \
  "${FUNCNAME[0]}" \
  "$BASH_SOURCE" \
  "${BASH_SOURCE[0]}" \
  "${BASH_LINENO[0]}" \
  "$RANDOM" \
  "${BASH_REMATCH[0]}" \
  "$READLINE_LINE" \
  "$BASH_VERSION" \
  "${BASH_VERSINFO[0]}" \
  "$OSTYPE" \
  "$HISTCONTROL" \
  "$HISTSIZE"

echo "$missing"

if true; then
  maybe=1
fi
echo "$maybe"

f() {
  local local_only
  printf '%s\n' "$local_only"
  readonly declared
  export exported
  printf '%s %s %s\n' "$1" "$@" "$#"
}
f
