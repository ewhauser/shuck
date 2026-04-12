#!/bin/sh

count=0
(count=1)
echo "$count"

value=outer
printf '%s\n' x | while read -r _; do value=inner; done
printf '%s\n' "$value"

(
  export path_prefix=/opt/demo
)
export path_prefix="$HOME/bin:$path_prefix"
