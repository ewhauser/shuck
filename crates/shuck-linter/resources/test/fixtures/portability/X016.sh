#!/bin/sh

# Should trigger: direct declare command in sh
declare portable=value
printf '%s\n' "$portable"

# Should trigger: direct typeset command in sh
typeset -i capacity=42
printf '%s\n' "$capacity"

# Should trigger: other bash-only builtins that ShellCheck groups under SC3044
shopt -s nullglob
complete -F _portable portable
compgen -A file
caller
dirs
disown
suspend
mapfile entries
readarray lines
pushd /tmp
popd

# Should trigger: wrapped declare command still resolves to declare
command declare wrapped=value
printf '%s\n' "$wrapped"

# Should trigger: wrapped typeset command still resolves to typeset
command typeset wrapped_typed=value
printf '%s\n' "$wrapped_typed"

# Should trigger: wrapped non-portable builtin still resolves to that builtin
command complete -F _wrapped wrapped

# Should trigger: nested declare command in substitutions still runs in sh
nested=$(declare inner=value)
printf '%s\n' "$nested"

# Should not trigger: plain portable assignment
plain=value
printf '%s\n' "$plain"
