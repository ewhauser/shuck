#!/bin/bash
args=(echo hi)
eval "${args[@]}"
eval "$@"
command eval "${args[@]}"
builtin eval "${args[@]}"
sudo eval "${args[@]}"
env eval "${args[@]}"
eval "$*"
