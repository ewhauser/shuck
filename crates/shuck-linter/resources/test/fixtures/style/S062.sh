#!/bin/bash

# Should trigger: unquoted assignment-default expansions passed to :
: ${HISTORY_FLAGS=''}
command : ${x=}
builtin : ${y:=fallback}
: prefix${z=word}suffix
: ${left=} ${right:=two}

# Should not trigger: quoted assignment-default expansions
: "${HISTORY_FLAGS=''}"
: "prefix${z=word}suffix"

# Should not trigger: non-assignment default operators
: ${x:-fallback}
: ${x-fallback}

# Should not trigger: other commands
echo ${x=}
printf '%s\n' ${y:=fallback}
env VAR=1 : ${x:=fallback}
