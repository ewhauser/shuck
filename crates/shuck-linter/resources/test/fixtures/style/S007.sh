#!/bin/sh

fmt='%s\n'
printf "$fmt" value
printf -v out "$fmt" value
nested_output="$(printf "$fmt" value)"
command printf "$fmt" value
exec printf "$fmt" value

printf '%s\n' "$fmt"
printf -- '%s\n' "$fmt"
