#!/bin/sh
# shellcheck disable=2121,1036

# Should trigger: csh-style set assignment with a list value.
set path = ( /usr/bin )

# Should also trigger: scalar csh-style assignment.
set foo = bar

# Should not trigger: regular set usage for positional parameters.
set -- path = /usr/bin
