#!/bin/sh
# shellcheck disable=2154,1087

# Should trigger: unbraced array-style syntax leaves a literal `[1]` suffix.
case "$words[1]" in
    install) echo installing ;;
esac

# Should also trigger: literal padding around the dynamic subject makes this arm impossible.
case " $oldobjs " in
    " ") : ;;
    "  ") : ;;
esac

# Should not trigger: this suffix-matching arm can still match.
case "prefix${value}suffix" in
    *suffix) : ;;
    prefix*suffix) : ;;
esac

# Should not trigger: braced expansions remain unconstrained here.
case "${words[1]}" in
    install) : ;;
esac
