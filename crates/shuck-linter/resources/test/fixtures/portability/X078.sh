#!/bin/sh
# shellcheck disable=2154,1087

# Should trigger: bare zsh array subscript in case subject.
case "$words[1]" in
    install) echo installing ;;
esac

# Should also trigger when unquoted.
case $line[1] in
    x) : ;;
esac

# Should not trigger: braced expansions are handled separately.
case "${words[1]}" in
    remove) : ;;
esac
