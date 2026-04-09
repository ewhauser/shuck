#!/bin/sh

# Should trigger: direct let command in sh
let x=1
printf '%s\n' "$x"

# Should trigger: wrapped let command still resolves to let
command let y=2
printf '%s\n' "$y"

# Should not trigger: portable arithmetic expansion
z=$((1 + 2))
printf '%s\n' "$z"
