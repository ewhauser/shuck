#!/bin/sh
# shellcheck disable=2154
while IFS='\n' read -r comp; do
  echo "$comp"
done < /dev/null
