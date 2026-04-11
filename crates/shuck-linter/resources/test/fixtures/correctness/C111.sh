#!/bin/bash
set -- a b

# Invalid: positional @ expansion in string comparisons.
if [ "_$@" = "_--version" ]; then :; fi
if [ "$@" = "--version" ]; then :; fi
if [ "${@:-fallback}" = "--version" ]; then :; fi
if [[ "_$@" == "_--version" ]]; then :; fi

# Valid: non-positional comparisons.
if [ "_$*" = "_--version" ]; then :; fi
if [ "_${arr[@]}" = "_x" ]; then :; fi
if [[ "\$@" == "x" ]]; then :; fi
