#!/bin/bash

export greeting=$(printf '%s\n' hi)
export now="$(date)"

demo() {
  local temp_conf="$(mktemp)"
  declare current_soversion=$(sed -n '1p' VERSION)
  typeset build_root=$(pwd)
  readonly keep_me=$(date)
}

export plain=ok
greeting=$(printf '%s\n' hi)
export greeting
