#!/usr/bin/env bash
name=world
echo "$name"

shadowed() {
  local name=shadow
  echo "$name"
}

echo "$1"
echo "$?"
declare -n ref=name
echo "$ref"
