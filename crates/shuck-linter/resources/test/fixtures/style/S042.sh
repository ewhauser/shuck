#!/bin/bash
while IFS== read -r key val; do
  echo "$key=$val"
done < /dev/null
