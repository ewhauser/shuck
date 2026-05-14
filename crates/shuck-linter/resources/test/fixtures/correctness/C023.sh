#!/bin/bash

: $((08))
: $((009 + 010 + 000))
declare -a values
values[018]=x

: $((0))
: $((10#08))
: $((0x10))
printf '%s\n' "${value:018:1}"
count=3
: $(( ${count}08 / 2 ))
declare -A checksums
checksums[008]=value
