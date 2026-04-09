#!/bin/sh
# shellcheck disable=2154,2034

# Should trigger: zsh prompt escape inside a quoted word.
X="%{$fg_bold[blue]%}text"

# Should also trigger on repeated prompt escapes in one word.
PS1="%{$fg[red]%}red%{$reset_color%}"

# Should not trigger: ordinary percent formatting without prompt escapes.
printf '%s\n' '%foo%'
