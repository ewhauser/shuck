#!/bin/sh
# shellcheck disable=2034,2082,2299,2154,3057

# Should trigger: zsh parameter index flag on a command-substitution target.
if is-at-least 3.1 ${"$(rsync --version 2>&1)"[(w)3]}; then
  echo new
fi

# Should also trigger for other index flags.
value=${map[(I)needle]}

# Should not trigger: ordinary braced subscript without a zsh flag.
value=${map[1]}
