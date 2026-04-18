#!/bin/bash
CFLAGS="${SLKCFLAGS}" ./configure --with-optmizer=${CFLAGS}
PATH=/tmp "$PATH"/bin/tool
A=1 B="$A" C="$B" cmd
foo="$foo" bar="$foo" cmd
foo=1 export "$foo"
foo=1 bar[$foo]=x cmd
FOO=tmp cmd >"$FOO"
foo=1 echo hi
foo="$foo" cmd
foo=1 cmd "$(printf %s "$foo")"
foo=1 foo=2 cmd
foo=1 bar="$foo"
COUNTDOWN=$[ $COUNTDOWN - 1 ]
