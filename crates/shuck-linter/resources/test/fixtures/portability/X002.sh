#!/bin/sh
[ "$1" == foo ]
test "$1" == foo
if [[ "$1" == foo ]]; then :; fi
[ "$1" = foo ]
test "$1" = "=="
[ "$1" == foo -o "$2" = bar ]
