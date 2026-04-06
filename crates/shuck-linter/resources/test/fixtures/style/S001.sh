#!/bin/bash

name=$1
printf '%s\n' $name
printf '%s\n' ${name:-fallback}

printf '%s\n' "$name"
printf '%s\n' "${name:-fallback}"

arr=(alpha "two words")
printf '%s\n' "${arr[@]}"
