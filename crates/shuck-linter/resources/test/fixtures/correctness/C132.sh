#!/bin/bash

# Should trigger: later argument expansion still sees the old shell value.
CFLAGS="${SLKCFLAGS}" ./configure --with-optmizer=${CFLAGS}

# Should trigger: command name expansion still uses the old PATH.
PATH=/tmp "$PATH"/bin/tool

# Should trigger: later prefix assignments still expand before earlier ones apply.
A=1 B="$A" C="$B" cmd

# Should trigger: builtin operands do not see the same-command prefix assignment.
foo=1 export "$foo"

# Should trigger: assignment subscripts are expanded before the command runs.
foo=1 bar[$foo]=x cmd

# Should trigger: later command arguments still see the old arithmetic value.
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo "$COUNTDOWN"

# Should not trigger: nested commands are out of scope.
foo=1 cmd "$(printf %s "$foo")"

# Should trigger: redirect targets are expanded before the command runs.
FOO=tmp cmd >"$FOO"

# Should not trigger: assignment-only commands do not create the temporary command environment.
foo=1 bar="$foo"
