#!/bin/bash
set -- a b

# Invalid: positional @ expansion used as a [ ] or test operand.
if [ -z "$@" ]; then :; fi
if test -n "${@:-fallback}"; then :; fi
if [ -d "$@" ]; then :; fi
if [ "_$@" = "_--version" ]; then :; fi
if [ "$@" = "--version" ]; then :; fi
if [ -n foo -a "${@:-lhs}" = "${@:-rhs}" ]; then :; fi
if [ -d "$@" -o "${@:-fallback}" = "x" ]; then :; fi

# Valid: truthy tests, non-positional uses, and double-bracket comparisons.
if [ "$@" ]; then :; fi
if [ "_$*" = "_--version" ]; then :; fi
if [ -d "${arr[@]}" ]; then :; fi
if [ "_${arr[@]}" = "_x" ]; then :; fi
if [[ "_$@" == "_--version" ]]; then :; fi
if [[ "\$@" == "x" ]]; then :; fi
