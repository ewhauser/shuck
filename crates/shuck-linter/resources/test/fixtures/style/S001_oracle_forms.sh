#!/bin/bash

set -- 'a b'
foo='a b'
unset maybe

printf '%s\n' $1
printf '%s\n' ${1}
printf '%s\n' $foo
printf '%s\n' ${foo}
printf '%s\n' ${maybe:-fallback}
printf '%s\n' ${maybe:=fallback}

foo=1
bash ${foo:+-x} script
bash ${foo:+"-x"} script
printf '%s\n' ${foo:+"a b"}

printf '%s\n' "$1" "${1}" "$foo" "${foo}" "${maybe:-fallback}" "${maybe:=fallback}"
