#!/bin/sh

echo "$(date)"
echo $(date)
value=$(echo $(date))
command echo "$(date)"
exec echo "$(date)"

echo "prefix $(date)"
printf '%s\n' "$(date)"
