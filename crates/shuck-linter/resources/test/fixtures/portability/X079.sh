#!/bin/sh
# shellcheck disable=2034,2082,2299,2154,3057

# Should trigger: zsh parameter index flag on a command-substitution target.
if is-at-least 3.1 ${"$(rsync --version 2>&1)"[(w)3]}; then
  echo new
fi

# Should not trigger: reference targets with zsh-style subscripts belong to array portability rules.
value=${map[(I)needle]}
value="${precmd_functions[(r)_z_precmd]}"

# Should not trigger: ordinary braced subscript without a quoted target.
value=${map[1]}
