#!/bin/sh

fmt='%s\n'
printf "$fmt" value
printf -v out "$fmt" value

printf '%s\n' "$fmt"
printf -- '%s\n' "$fmt"
