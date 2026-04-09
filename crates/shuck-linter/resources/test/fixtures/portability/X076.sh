#!/bin/sh
# shellcheck disable=2154,2034,2082,3057,2299

# Should trigger: zsh-style global substitution flag.
x="${$(svn info):gs/%/%%}"

# Should also trigger: nested parameter target with a zsh path flag.
dir="${${custom_datafile:-$HOME/.z}:A}"

# Should not trigger: POSIX defaulting or bash substring slices.
fallback="${value:-default}"
prefix="${value:0:1}"

# Should not trigger: simple colon forms without a nested target.
branch="${branch:gs/%/%%}"
pwd_dir="${PWD:h}"
