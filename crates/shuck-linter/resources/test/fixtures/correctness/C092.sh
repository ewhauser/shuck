#!/bin/bash

# Invalid: the command substitution output is executed as the condition command.
if $(python3 -c 'import sys' 2>/dev/null); then echo ok; fi

# Invalid: loop conditions have the same issue.
while $(false); do break; done
until $(false); do break; done

# Invalid: negation and short-circuit conditions still execute the `$(...)` output.
if ! $(false); then echo no; fi
if foo && $(false); then :; fi

# Valid: using the command directly tests its exit status.
if python3 -c 'import sys' 2>/dev/null; then echo ok; fi

# Valid: quoted substitutions in these condition positions fall under other checks, not C092.
if "$(printf '%s' maybe-command)"; then :; fi
if [[ "$pm" == apt ]] && "$(printf '%s' missing)" != installed; then :; fi

# Valid: wrapper commands are outside this condition-specific rule.
if command $(false); then :; fi
