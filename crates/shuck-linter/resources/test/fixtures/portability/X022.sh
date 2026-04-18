#!/bin/sh
read -p prompt name
printf -v out '%s\n' foo
export -fn greet
command export -fn greet
trap -p EXIT
wait -n
ulimit -n
type -P printf
