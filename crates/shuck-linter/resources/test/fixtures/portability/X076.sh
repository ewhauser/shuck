#!/bin/sh
# shellcheck disable=2154,2034,2082,3057,2299

# Should trigger: replacement applied directly to a command substitution.
x="${$(svn info):gs/%/%%}"

# Should trigger: slicing applied directly to a command substitution.
first_char="${$(svn info):0:1}"

# Should trigger: defaulting applied directly to a command substitution.
fallback="${$(svn info):-default}"

# Should not trigger here: nested parameter expansions are covered by X051.
dir="${${custom_datafile:-$HOME/.z}:A}"

# Should not trigger: POSIX defaulting or bash substring slices.
safe_default="${value:-default}"
prefix="${value:0:1}"

# Should not trigger: simple colon forms without a command substitution target.
branch="${branch:gs/%/%%}"
pwd_dir="${PWD:h}"
