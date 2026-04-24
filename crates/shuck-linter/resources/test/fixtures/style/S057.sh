#!/bin/sh
# shellcheck disable=2034
alias first='echo $1'
alias rest='printf "%s\n" "$@"'
alias conditional='${1+"$@"}'
alias foo=$BAR
alias bar='$(printf hi)'
alias baz='noglob gtl'
alias func='helper() { echo hi; }'
